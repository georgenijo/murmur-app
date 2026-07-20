use crate::commands::models::check_specific_model_exists;
use crate::resource_monitor::get_process_rss_mb;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::transcriber::CoreMlBackend;
use crate::transcriber::{
    ParakeetBackend, TranscriptionBackend, WhisperBackend, COREML_MODEL_NAME, WHISPER_SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;

const PARAKEET_CPU_MODEL: &str = "parakeet-tdt-0.6b-v2-fp16";

/// Whisper model names ordered smallest-to-largest. Used to pick the
/// cheapest selected whisper model for the untimed shared-init warm-up.
const WHISPER_SIZE_ORDER: &[&str] =
    &["tiny.en", "base.en", "small.en", "medium.en", "large-v3-turbo"];

struct Fixture {
    id: &'static str,
    label: &'static str,
    wav: &'static [u8],
    reference: &'static str,
}

struct PreparedFixture<'a> {
    fixture: &'a Fixture,
    samples: Vec<f32>,
    audio_seconds: f64,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "short",
        label: "Short",
        wav: include_bytes!("../../../bench/audio/short.wav"),
        reference: include_str!("../../../bench/audio/short.txt"),
    },
    Fixture {
        id: "medium",
        label: "Medium",
        wav: include_bytes!("../../../bench/audio/medium.wav"),
        reference: include_str!("../../../bench/audio/medium.txt"),
    },
    Fixture {
        id: "long",
        label: "Long",
        wav: include_bytes!("../../../bench/audio/long.wav"),
        reference: include_str!("../../../bench/audio/long.txt"),
    },
    Fixture {
        id: "xlong",
        label: "Extra long",
        wav: include_bytes!("../../../bench/audio/xlong.wav"),
        reference: include_str!("../../../bench/audio/xlong.txt"),
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkModel {
    pub model_name: String,
    pub label: String,
    pub backend: String,
    pub accelerator: String,
    pub size: String,
    pub supported: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BenchmarkPreset {
    Quick,
    Standard,
    Thorough,
}

impl BenchmarkPreset {
    fn iterations(self) -> usize {
        match self {
            Self::Quick => 3,
            Self::Standard => 5,
            Self::Thorough => 10,
        }
    }

    fn fixtures(self) -> &'static [Fixture] {
        match self {
            Self::Quick => &FIXTURES[..2],
            Self::Standard | Self::Thorough => FIXTURES,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRequest {
    pub model_names: Vec<String>,
    pub preset: BenchmarkPreset,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureResult {
    pub fixture_id: String,
    pub label: String,
    pub audio_seconds: f64,
    pub warm_median_ms: f64,
    pub warm_p95_ms: f64,
    pub realtime_factor: f64,
    pub word_error_rate: f64,
    pub word_errors: usize,
    pub reference_words: usize,
    pub normalized_word_error_rate: f64,
    pub normalized_word_errors: usize,
    pub normalized_reference_words: usize,
    pub reference: String,
    pub transcript: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResult {
    pub model_name: String,
    pub label: String,
    pub backend: String,
    pub accelerator: String,
    pub model_load_ms: Option<f64>,
    pub first_inference_ms: Option<f64>,
    pub warm_median_ms: Option<f64>,
    pub warm_p95_ms: Option<f64>,
    pub realtime_factor: Option<f64>,
    pub word_error_rate: Option<f64>,
    pub normalized_word_error_rate: Option<f64>,
    /// Process-RSS delta measured around this model's run. Models are
    /// benchmarked sequentially in one process, so allocator retention from
    /// an earlier model can inflate a later model's baseline; treat this as
    /// a rough signal, not an isolated per-model measurement.
    pub memory_delta_mb: u64,
    pub fixtures: Vec<FixtureResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendations {
    pub fastest: Option<String>,
    pub most_accurate: Option<String>,
    pub balanced: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReport {
    pub created_at: String,
    pub app_version: String,
    pub platform: String,
    pub preset: BenchmarkPreset,
    pub iterations: usize,
    /// Duration of the untimed warm-up pass run once before any per-model
    /// timing, in milliseconds. This absorbs one-time shared backend init
    /// (Metal shader compilation, ANE compile cache, etc.) that would
    /// otherwise be misattributed to whichever model happens to load first.
    /// It IS representative of real first-launch latency, just not a
    /// per-model attribute.
    pub shared_init_ms: f64,
    pub results: Vec<ModelResult>,
    pub recommendations: Recommendations,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkProgress {
    completed: usize,
    total: usize,
    model_name: String,
    model_label: String,
    fixture: Option<String>,
    phase: &'static str,
}

pub struct BenchmarkCoordinator {
    activity: Mutex<CoordinatorActivity>,
    cancelled: AtomicBool,
}

#[derive(Clone, Copy, PartialEq)]
enum CoordinatorActivity {
    Idle,
    Benchmark,
    SharedBackendChange,
}

impl BenchmarkCoordinator {
    pub fn new() -> Self {
        Self {
            activity: Mutex::new(CoordinatorActivity::Idle),
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn is_running(&self) -> bool {
        *self
            .activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == CoordinatorActivity::Benchmark
    }

    pub fn try_start(&self) -> bool {
        let mut activity = self
            .activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *activity != CoordinatorActivity::Idle {
            return false;
        }
        *activity = CoordinatorActivity::Benchmark;
        self.cancelled.store(false, Ordering::SeqCst);
        true
    }

    pub fn try_start_shared_backend_change(&self) -> bool {
        let mut activity = self
            .activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *activity != CoordinatorActivity::Idle {
            return false;
        }
        *activity = CoordinatorActivity::SharedBackendChange;
        true
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn finish(&self) {
        *self
            .activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = CoordinatorActivity::Idle;
    }

    pub fn finish_shared_backend_change(&self) {
        let mut activity = self
            .activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *activity == CoordinatorActivity::SharedBackendChange {
            *activity = CoordinatorActivity::Idle;
        }
    }
}

impl Default for BenchmarkCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

pub fn benchmark_models() -> Vec<BenchmarkModel> {
    let coreml_supported = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let whisper_accelerator = if cfg!(target_os = "macos") {
        "Metal GPU"
    } else {
        "GPU / CPU"
    };
    let definitions = [
        (
            COREML_MODEL_NAME,
            "Parakeet v3",
            "Core ML",
            "Apple Neural Engine",
            "~470 MB",
            coreml_supported,
        ),
        (
            PARAKEET_CPU_MODEL,
            "Parakeet v2",
            "sherpa-onnx",
            "CPU",
            "~1.2 GB",
            true,
        ),
        (
            "tiny.en",
            "Whisper Tiny",
            "whisper.cpp",
            whisper_accelerator,
            "~75 MB",
            true,
        ),
        (
            "base.en",
            "Whisper Base",
            "whisper.cpp",
            whisper_accelerator,
            "~150 MB",
            true,
        ),
        (
            "small.en",
            "Whisper Small",
            "whisper.cpp",
            whisper_accelerator,
            "~500 MB",
            true,
        ),
        (
            "medium.en",
            "Whisper Medium",
            "whisper.cpp",
            whisper_accelerator,
            "~1.5 GB",
            true,
        ),
        (
            "large-v3-turbo",
            "Whisper Large Turbo",
            "whisper.cpp",
            whisper_accelerator,
            "~3 GB",
            true,
        ),
    ];

    definitions
        .into_iter()
        .map(
            |(model_name, label, backend, accelerator, size, supported)| BenchmarkModel {
                model_name: model_name.to_string(),
                label: label.to_string(),
                backend: backend.to_string(),
                accelerator: accelerator.to_string(),
                size: size.to_string(),
                supported,
                installed: supported && check_specific_model_exists(model_name.to_string()),
            },
        )
        .collect()
}

fn backend_for(model_name: &str) -> Result<Box<dyn TranscriptionBackend>, String> {
    match model_name {
        COREML_MODEL_NAME => {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                Ok(Box::new(CoreMlBackend::new()))
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                Err("Core ML requires an Apple Silicon Mac".to_string())
            }
        }
        PARAKEET_CPU_MODEL => Ok(Box::new(ParakeetBackend::new())),
        "tiny.en" | "base.en" | "small.en" | "medium.en" | "large-v3-turbo" => {
            Ok(Box::new(WhisperBackend::new()))
        }
        _ => Err(format!("Unknown benchmark model '{model_name}'")),
    }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|word| !word.is_empty())
        .map(ToString::to_string)
        .collect()
}

// --- Text normalization for WER scoring -------------------------------------
//
// Raw WER punishes formatting and inverse-text-normalization (ITN) differences
// that are not recognition errors: "16 kHz" vs "sixteen kilohertz", "Mac OS" vs
// "macOS", "front end" vs "frontend". `normalized_words` maps both the reference
// and the hypothesis to a common canonical token stream (on top of the existing
// lowercase + punctuation-split behaviour) so those differences stop counting.
//
// The normalization is intentionally conservative: every rule maps genuinely
// equivalent spellings of the SAME thing to one token. It never collapses two
// different spoken words, so real misrecognitions (e.g. "Tauri" -> "Tori") still
// score as errors. The compound-word and unit tables below are small, curated,
// and hand-maintained for exactly this reason.

/// Curated technical compounds frequently written as either one word or two.
/// Both spellings collapse to the joined single-word form. Adding an entry that
/// joins two genuinely distinct words would hide real errors, so keep this list
/// limited to true one-word/two-word spelling variants.
const COMPOUND_JOINS: &[(&str, &str)] = &[
    ("front", "end"),
    ("back", "end"),
    ("mac", "os"),
    ("web", "site"),
    ("data", "base"),
    ("run", "time"),
    ("code", "base"),
    ("name", "space"),
    ("white", "space"),
    ("life", "cycle"),
    ("key", "board"),
    ("check", "box"),
    ("drop", "down"),
    ("time", "stamp"),
];

/// Canonicalize a common unit token. Both the abbreviation and the spelled-out
/// form map to the same canonical abbreviation. Curated; only true synonyms.
fn normalize_unit(word: &str) -> Option<&'static str> {
    Some(match word {
        "hz" | "hertz" => "hz",
        "khz" | "kilohertz" => "khz",
        "mhz" | "megahertz" => "mhz",
        "ghz" | "gigahertz" => "ghz",
        "kb" | "kilobyte" | "kilobytes" => "kb",
        "mb" | "megabyte" | "megabytes" => "mb",
        "gb" | "gigabyte" | "gigabytes" => "gb",
        "tb" | "terabyte" | "terabytes" => "tb",
        "ms" | "millisecond" | "milliseconds" => "ms",
        _ => return None,
    })
}

fn cardinal_small(word: &str) -> Option<u32> {
    Some(match word {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    })
}

fn tens_word(word: &str) -> Option<u32> {
    Some(match word {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn ones_word(word: &str) -> Option<u32> {
    cardinal_small(word).filter(|value| (1..=9).contains(value))
}

fn ordinal_word(word: &str) -> Option<u32> {
    Some(match word {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        "fortieth" => 40,
        "fiftieth" => 50,
        "sixtieth" => 60,
        "seventieth" => 70,
        "eightieth" => 80,
        "ninetieth" => 90,
        _ => return None,
    })
}

/// Parse a digit ordinal such as "1st", "2nd", "16th" into its numeric value.
fn digit_ordinal(word: &str) -> Option<u32> {
    for suffix in ["st", "nd", "rd", "th"] {
        if let Some(digits) = word.strip_suffix(suffix) {
            if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
                return digits.parse().ok();
            }
        }
    }
    None
}

/// Collapse number words to digits so "sixteen" and "16" score identically.
/// Cardinals and ordinals map to distinct canonical tokens ("16" vs "16ord") so
/// a cardinal/ordinal mismatch is still counted. Handles 0-100, common ordinals,
/// two-word tens ("twenty one" -> "21"), and "hundred".
fn fold_numbers(tokens: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();

        if let Some(value) = digit_ordinal(token).or_else(|| ordinal_word(token)) {
            out.push(format!("{value}ord"));
            index += 1;
            continue;
        }

        if let Some(tens) = tens_word(token) {
            if let Some(ones) = tokens.get(index + 1).and_then(|word| ones_word(word)) {
                out.push((tens + ones).to_string());
                index += 2;
                continue;
            }
            out.push(tens.to_string());
            index += 1;
            continue;
        }

        if token == "hundred" {
            if let Some(previous) = out.last().and_then(|word| word.parse::<u32>().ok()) {
                if (1..=9).contains(&previous) {
                    *out.last_mut().expect("previous digit present") = (previous * 100).to_string();
                    index += 1;
                    continue;
                }
            }
            out.push("100".to_string());
            index += 1;
            continue;
        }

        if let Some(value) = cardinal_small(token) {
            out.push(value.to_string());
            index += 1;
            continue;
        }

        out.push(tokens[index].clone());
        index += 1;
    }
    out
}

/// Join curated two-word technical compounds into their single-word form.
fn join_compounds(tokens: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if let Some(next) = tokens.get(index + 1) {
            let matches = COMPOUND_JOINS
                .iter()
                .any(|(left, right)| *left == tokens[index] && *right == next);
            if matches {
                out.push(format!("{}{}", tokens[index], next));
                index += 2;
                continue;
            }
        }
        out.push(tokens[index].clone());
        index += 1;
    }
    out
}

/// Whisper-`EnglishTextNormalizer`-style canonicalization on top of `words`:
/// digit/word number equivalence, common unit abbreviations, and curated tech
/// compounds. Deterministic and local. Applied to both reference and hypothesis
/// before the edit-distance pass.
fn normalized_words(text: &str) -> Vec<String> {
    let joined = join_compounds(fold_numbers(words(text)));
    joined
        .into_iter()
        .map(|word| normalize_unit(&word).map(str::to_string).unwrap_or(word))
        .collect()
}

fn edit_distance(reference: &[String], hypothesis: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    for (row, reference_word) in reference.iter().enumerate() {
        let mut current = vec![row + 1; hypothesis.len() + 1];
        for (column, hypothesis_word) in hypothesis.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(reference_word != hypothesis_word));
        }
        previous = current;
    }
    previous[hypothesis.len()]
}

/// Raw WER: lowercase + punctuation split only. Returns (errors, reference words).
fn word_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let reference = words(reference);
    let hypothesis = words(hypothesis);
    (edit_distance(&reference, &hypothesis), reference.len())
}

/// Normalized WER: applies `normalized_words` before scoring so formatting/ITN
/// differences do not count. Returns (errors, normalized reference words).
fn normalized_word_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let reference = normalized_words(reference);
    let hypothesis = normalized_words(hypothesis);
    (edit_distance(&reference, &hypothesis), reference.len())
}

fn percentile(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((percentile * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    Some(sorted[index])
}

fn emit_progress(
    app: &tauri::AppHandle,
    completed: usize,
    total: usize,
    model: &BenchmarkModel,
    fixture: Option<&str>,
    phase: &'static str,
) {
    let _ = app.emit(
        "benchmark-progress",
        BenchmarkProgress {
            completed,
            total,
            model_name: model.model_name.clone(),
            model_label: model.label.clone(),
            fixture: fixture.map(ToString::to_string),
            phase,
        },
    );
}

fn error_result(model: &BenchmarkModel, error: String) -> ModelResult {
    ModelResult {
        model_name: model.model_name.clone(),
        label: model.label.clone(),
        backend: model.backend.clone(),
        accelerator: model.accelerator.clone(),
        model_load_ms: None,
        first_inference_ms: None,
        warm_median_ms: None,
        warm_p95_ms: None,
        realtime_factor: None,
        word_error_rate: None,
        normalized_word_error_rate: None,
        memory_delta_mb: 0,
        fixtures: Vec::new(),
        error: Some(error),
    }
}

fn prepare_fixtures(
    fixtures: &[Fixture],
    vad_threshold: f32,
) -> Result<Vec<PreparedFixture<'_>>, String> {
    let vad_path = crate::vad::vad_model_path()
        .filter(|path| path.exists())
        .ok_or_else(|| "Silero VAD model is not installed".to_string())?;
    let vad_path = vad_path.to_string_lossy();

    fixtures
        .iter()
        .map(|fixture| {
            let samples = crate::transcriber::parse_wav_to_samples(fixture.wav)
                .map_err(|error| format!("Could not decode {} fixture: {error}", fixture.label))?;
            let samples = match crate::vad::filter_speech(&vad_path, &samples, vad_threshold)
                .map_err(|error| format!("VAD failed for {} fixture: {error}", fixture.label))?
            {
                crate::vad::VadResult::Speech(samples) => samples,
                crate::vad::VadResult::NoSpeech => {
                    return Err(format!(
                        "VAD detected no speech in the {} benchmark fixture",
                        fixture.label
                    ));
                }
            };
            let audio_seconds = samples.len() as f64 / WHISPER_SAMPLE_RATE as f64;
            Ok(PreparedFixture {
                fixture,
                samples,
                audio_seconds,
            })
        })
        .collect()
}

/// Realtime-factor deltas below this fraction are treated as statistical
/// noise rather than a genuine performance difference, so run-to-run jitter
/// never flips which model looks "fastest" or "balanced". See issue #272.
const REALTIME_FACTOR_TIE_BAND: f64 = 0.10;

/// True when `a` and `b` are within `REALTIME_FACTOR_TIE_BAND` of the smaller
/// of the two, i.e. indistinguishable for recommendation purposes.
fn within_tie_band(a: f64, b: f64) -> bool {
    let base = a.min(b);
    if base <= 0.0 {
        return (a - b).abs() <= f64::EPSILON;
    }
    // A tiny epsilon keeps the boundary inclusive of an intended-exact 10%
    // delta despite binary floating-point representation error (e.g.
    // 1.10 - 1.00 != 0.10 bit-for-bit).
    (a - b).abs() / base <= REALTIME_FACTOR_TIE_BAND + 1e-9
}

/// Picks the model with the lowest `metric` value, then deterministically
/// breaks ties among every candidate within `REALTIME_FACTOR_TIE_BAND` of the
/// best value: lower peak memory delta wins, then alphabetical model name as
/// a final stable tie-break. Used for both "fastest" and "balanced" so noise
/// in the underlying metric is never presented as signal.
fn pick_by_metric_with_tiebreak<'a>(
    candidates: &[(&'a ModelResult, f64)],
) -> Option<&'a ModelResult> {
    let best = candidates
        .iter()
        .map(|(_, value)| *value)
        .min_by(f64::total_cmp)?;
    candidates
        .iter()
        .filter(|(_, value)| within_tie_band(*value, best))
        .map(|(result, _)| *result)
        .min_by(|left, right| {
            left.memory_delta_mb
                .cmp(&right.memory_delta_mb)
                .then_with(|| left.model_name.cmp(&right.model_name))
        })
}

/// Decide which model(s) to load-and-drop during the untimed shared-init
/// warm-up pass. Picks the smallest selected whisper model (whisper models
/// share one Metal/shader init cost, so only one needs warming) plus one
/// entry per non-whisper backend family that is selected, since each of
/// those (Core ML ANE compile cache, sherpa-onnx) has its own separate
/// shared init cost. Returns an empty plan when nothing is selected.
fn warmup_plan(selected: &[BenchmarkModel]) -> Vec<String> {
    let mut plan = Vec::new();

    let whisper_pick = WHISPER_SIZE_ORDER.iter().find_map(|candidate| {
        selected
            .iter()
            .find(|model| model.model_name == *candidate)
            .map(|model| model.model_name.clone())
    });
    if let Some(model_name) = whisper_pick {
        plan.push(model_name);
    }

    for model_name in [COREML_MODEL_NAME, PARAKEET_CPU_MODEL] {
        if selected.iter().any(|model| model.model_name == model_name) {
            plan.push(model_name.to_string());
        }
    }

    plan
}

fn recommendations(results: &[ModelResult]) -> Recommendations {
    let successful = results
        .iter()
        .filter(|result| result.error.is_none())
        .collect::<Vec<_>>();

    // "Fastest" ranks by realtime_factor (compute-seconds per audio-second),
    // not warm_median_ms. warm_median_ms on ModelResult is a percentile
    // pooled across every fixture's warm samples -- short and long audio in
    // one distribution -- so its rank position jitters between fixture-length
    // strata across otherwise-identical runs. realtime_factor is a
    // corpus-weighted ratio computed per fixture then combined, so it stays
    // stable and comparable across different fixture mixes. warm_median_ms is
    // left untouched elsewhere in the report; only this ranking input
    // changes. See issue #272.
    let fastest_candidates = successful
        .iter()
        .filter_map(|result| result.realtime_factor.map(|value| (*result, value)))
        .collect::<Vec<_>>();
    let fastest =
        pick_by_metric_with_tiebreak(&fastest_candidates).map(|result| result.model_name.clone());

    // Accuracy recommendations rank on the normalized WER so formatting/ITN
    // differences do not distort the ranking (see `normalized_words`).
    let most_accurate = successful
        .iter()
        .filter_map(|result| result.normalized_word_error_rate.map(|value| (*result, value)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(result, _)| result.model_name.clone());

    let best_wer = successful
        .iter()
        .filter_map(|result| result.normalized_word_error_rate)
        .min_by(f64::total_cmp);

    // "Balanced" first narrows to models within 2 accuracy points of the best
    // WER, then within that accuracy window ranks by realtime_factor (same
    // stability fix as "fastest") and, among models whose realtime_factor is
    // tied within the 10% noise band, prefers lower peak memory delta. This
    // is the rule that stops a model like Parakeet v2 (~2.9GB RSS delta) from
    // winning over alternatives at 46-545MB purely because pooled timing
    // noise nudged it ahead when their speed is otherwise indistinguishable.
    // See issue #272.
    let balanced = best_wer.and_then(|best| {
        let balanced_candidates = successful
            .iter()
            .filter(|result| {
                result
                    .normalized_word_error_rate
                    .is_some_and(|wer| wer <= best + 0.02)
            })
            .filter_map(|result| result.realtime_factor.map(|value| (*result, value)))
            .collect::<Vec<_>>();
        pick_by_metric_with_tiebreak(&balanced_candidates)
            .map(|result| result.model_name.clone())
    });

    Recommendations {
        fastest,
        most_accurate,
        balanced,
    }
}

pub fn run(
    app: &tauri::AppHandle,
    coordinator: &BenchmarkCoordinator,
    request: BenchmarkRequest,
) -> Result<BenchmarkReport, String> {
    if request.model_names.is_empty() {
        return Err("Select at least one installed model".to_string());
    }
    let catalog = benchmark_models();
    let mut seen = HashSet::new();
    let model_names = request
        .model_names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect::<Vec<_>>();
    let selected = model_names
        .iter()
        .map(|name| {
            catalog
                .iter()
                .find(|model| model.model_name == *name)
                .cloned()
                .ok_or_else(|| format!("Unknown benchmark model '{name}'"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(model) = selected
        .iter()
        .find(|model| !model.supported || !model.installed)
    {
        return Err(format!("{} is not installed on this machine", model.label));
    }

    let fixtures = prepare_fixtures(request.preset.fixtures(), 0.5)?;
    let iterations = request.preset.iterations();
    let steps_per_model = 1 + fixtures.len() * (1 + iterations);
    let warmup_targets = warmup_plan(&selected);
    let total_steps = selected.len() * steps_per_model + warmup_targets.len();
    let mut completed = 0;
    let mut results = Vec::with_capacity(selected.len());

    // Untimed warm-up pass: absorb one-time shared backend init (Metal
    // shader compilation, ANE compile cache, ...) before any per-model
    // timing starts, so it isn't misattributed to whichever model happens
    // to load first. See BenchmarkReport::shared_init_ms.
    let mut shared_init_ms = 0.0;
    for target in &warmup_targets {
        if coordinator.is_cancelled() {
            return Err("Benchmark cancelled".to_string());
        }
        if let Some(model) = selected
            .iter()
            .find(|candidate| candidate.model_name == *target)
        {
            emit_progress(app, completed, total_steps, model, None, "priming");
        }
        if let Ok(mut backend) = backend_for(target) {
            let warmup_started = Instant::now();
            if backend.load_model(target).is_ok() {
                shared_init_ms += warmup_started.elapsed().as_secs_f64() * 1000.0;
            }
            backend.reset();
        }
        completed += 1;
    }

    for model in selected {
        let model_start = completed;
        if coordinator.is_cancelled() {
            return Err("Benchmark cancelled".to_string());
        }
        emit_progress(app, completed, total_steps, &model, None, "loading");
        let mut backend = match backend_for(&model.model_name) {
            Ok(backend) => backend,
            Err(error) => {
                results.push(error_result(&model, error));
                completed = model_start + steps_per_model;
                emit_progress(app, completed, total_steps, &model, None, "complete");
                continue;
            }
        };
        let baseline_rss = get_process_rss_mb();
        let load_started = Instant::now();
        if let Err(error) = backend.load_model(&model.model_name) {
            results.push(error_result(&model, error));
            completed = model_start + steps_per_model;
            emit_progress(app, completed, total_steps, &model, None, "complete");
            continue;
        }
        let model_load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
        completed += 1;
        let mut peak_rss = get_process_rss_mb();
        let mut first_inference_ms = None;
        let mut all_warm = Vec::new();
        let mut fixture_results = Vec::with_capacity(fixtures.len());
        let mut total_errors = 0;
        let mut total_reference_words = 0;
        let mut total_normalized_errors = 0;
        let mut total_normalized_reference_words = 0;
        let mut corpus_warm_seconds = 0.0;
        let mut corpus_audio_seconds = 0.0;
        let mut failed = None;

        for prepared in &fixtures {
            let fixture = prepared.fixture;
            if coordinator.is_cancelled() {
                backend.reset();
                return Err("Benchmark cancelled".to_string());
            }
            let samples = &prepared.samples;
            let audio_seconds = prepared.audio_seconds;
            emit_progress(
                app,
                completed,
                total_steps,
                &model,
                Some(fixture.label),
                "warming",
            );
            let warmup_started = Instant::now();
            match backend.transcribe(samples, "en", None, true) {
                Ok(_) => {}
                Err(error) => {
                    failed = Some(error);
                    break;
                }
            };
            if first_inference_ms.is_none() {
                first_inference_ms = Some(warmup_started.elapsed().as_secs_f64() * 1000.0);
            }
            completed += 1;
            peak_rss = peak_rss.max(get_process_rss_mb());

            let mut warm = Vec::with_capacity(iterations);
            let mut transcripts = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                if coordinator.is_cancelled() {
                    backend.reset();
                    return Err("Benchmark cancelled".to_string());
                }
                emit_progress(
                    app,
                    completed,
                    total_steps,
                    &model,
                    Some(fixture.label),
                    "measuring",
                );
                let started = Instant::now();
                match backend.transcribe(samples, "en", None, true) {
                    Ok(output) => transcripts.push(output),
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                }
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                warm.push(elapsed_ms);
                completed += 1;
                peak_rss = peak_rss.max(get_process_rss_mb());
            }
            if failed.is_some() {
                break;
            }
            let warm_median_ms = percentile(&warm, 0.5).unwrap_or(0.0);
            let warm_p95_ms = percentile(&warm, 0.95).unwrap_or(0.0);
            all_warm.extend_from_slice(&warm);
            corpus_warm_seconds += warm_median_ms / 1000.0;
            corpus_audio_seconds += audio_seconds;
            // Score each transcript on both raw and normalized WER. Pick the
            // median transcript by normalized errors (the reported ranking
            // metric), breaking ties by raw errors for determinism.
            let mut scored_transcripts = transcripts
                .into_iter()
                .map(|transcript| {
                    let (errors, reference_words) = word_errors(fixture.reference, &transcript);
                    let (normalized_errors, normalized_reference_words) =
                        normalized_word_errors(fixture.reference, &transcript);
                    (
                        normalized_errors,
                        errors,
                        reference_words,
                        normalized_reference_words,
                        transcript,
                    )
                })
                .collect::<Vec<_>>();
            scored_transcripts
                .sort_by_key(|(normalized_errors, errors, ..)| (*normalized_errors, *errors));
            let (
                normalized_errors,
                errors,
                reference_words,
                normalized_reference_words,
                transcript,
            ) = scored_transcripts
                .into_iter()
                .nth((iterations - 1) / 2)
                .expect("benchmark presets include measured iterations");
            total_errors += errors;
            total_reference_words += reference_words;
            total_normalized_errors += normalized_errors;
            total_normalized_reference_words += normalized_reference_words;
            fixture_results.push(FixtureResult {
                fixture_id: fixture.id.to_string(),
                label: fixture.label.to_string(),
                audio_seconds,
                warm_median_ms,
                warm_p95_ms,
                realtime_factor: warm_median_ms / 1000.0 / audio_seconds,
                word_error_rate: errors as f64 / reference_words as f64,
                word_errors: errors,
                reference_words,
                normalized_word_error_rate: normalized_errors as f64
                    / normalized_reference_words as f64,
                normalized_word_errors: normalized_errors,
                normalized_reference_words,
                reference: fixture.reference.trim().to_string(),
                transcript,
            });
        }

        backend.reset();
        if let Some(error) = failed {
            results.push(error_result(&model, error));
            completed = model_start + steps_per_model;
            emit_progress(app, completed, total_steps, &model, None, "complete");
            continue;
        }
        results.push(ModelResult {
            model_name: model.model_name,
            label: model.label,
            backend: model.backend,
            accelerator: model.accelerator,
            model_load_ms: Some(model_load_ms),
            first_inference_ms,
            warm_median_ms: percentile(&all_warm, 0.5),
            warm_p95_ms: percentile(&all_warm, 0.95),
            realtime_factor: Some(corpus_warm_seconds / corpus_audio_seconds),
            word_error_rate: Some(total_errors as f64 / total_reference_words as f64),
            normalized_word_error_rate: Some(
                total_normalized_errors as f64 / total_normalized_reference_words as f64,
            ),
            memory_delta_mb: peak_rss.saturating_sub(baseline_rss),
            fixtures: fixture_results,
            error: None,
        });
    }

    emit_progress(
        app,
        total_steps,
        total_steps,
        results
            .last()
            .and_then(|result| {
                catalog
                    .iter()
                    .find(|model| model.model_name == result.model_name)
            })
            .unwrap_or(&catalog[0]),
        None,
        "complete",
    );
    let recommendations = recommendations(&results);
    Ok(BenchmarkReport {
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        preset: request.preset,
        iterations,
        shared_init_ms,
        results,
        recommendations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_error_rate_counts_substitutions_insertions_and_deletions() {
        assert_eq!(word_errors("one two three", "one four three"), (1, 3));
        assert_eq!(word_errors("one two three", "one two three four"), (1, 3));
        assert_eq!(word_errors("one two three", "one three"), (1, 3));
    }

    #[test]
    fn normalizer_collapses_formatting_and_itn_differences() {
        // The concrete pairs from the issue must normalize to identical tokens.
        assert_eq!(normalized_words("16 kHz"), normalized_words("sixteen kilohertz"));
        assert_eq!(normalized_words("Mac OS"), normalized_words("macOS"));
        assert_eq!(normalized_words("front end"), normalized_words("frontend"));
        // A few more equivalences the tables promise.
        assert_eq!(normalized_words("500 MB"), normalized_words("five hundred megabytes"));
        assert_eq!(normalized_words("2 ms"), normalized_words("two milliseconds"));
        assert_eq!(normalized_words("twenty one"), normalized_words("21"));
        assert_eq!(normalized_words("the 1st run"), normalized_words("the first run"));
    }

    #[test]
    fn normalized_word_errors_ignores_formatting_but_keeps_recognition_errors() {
        // Formatting/ITN differences score zero under normalization.
        assert_eq!(normalized_word_errors("16 kHz", "sixteen kilohertz"), (0, 2));
        assert_eq!(normalized_word_errors("front end", "frontend"), (0, 1));
        assert_eq!(normalized_word_errors("Mac OS", "macOS"), (0, 1));

        // Real misrecognitions still count (this is #271's territory, not ours).
        assert!(normalized_word_errors("Tauri", "Tori").0 > 0);
        assert!(word_errors("Tauri", "Tori").0 > 0);

        // Different numbers/units must not collapse to the same token.
        assert_eq!(normalized_word_errors("16 kHz", "32 kHz"), (1, 2));
        assert_eq!(normalized_word_errors("500 MB", "500 GB"), (1, 2));
        // Cardinal vs ordinal is a genuine difference and is preserved.
        assert!(normalized_word_errors("one", "first").0 > 0);
    }

    #[test]
    fn normalization_shrinks_a_known_raw_wer_delta() {
        // Same sentence, formatted two ways: raw scoring counts several errors,
        // normalized scoring counts none. Locks the raw-vs-normalized delta.
        let reference = "The front end uses 16 kHz audio";
        let hypothesis = "The frontend uses sixteen kilohertz audio";
        assert_eq!(word_errors(reference, hypothesis), (4, 7));
        assert_eq!(normalized_word_errors(reference, hypothesis), (0, 6));
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        assert_eq!(percentile(&[4.0, 1.0, 3.0, 2.0], 0.5), Some(2.0));
        assert_eq!(percentile(&[4.0, 1.0, 3.0, 2.0], 0.95), Some(4.0));
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn presets_have_bounded_workloads() {
        assert_eq!(BenchmarkPreset::Quick.fixtures().len(), 2);
        assert_eq!(BenchmarkPreset::Quick.iterations(), 3);
        assert_eq!(BenchmarkPreset::Standard.fixtures().len(), 4);
        assert_eq!(BenchmarkPreset::Thorough.iterations(), 10);
    }

    #[test]
    fn bundled_fixtures_retain_speech_after_vad() {
        if !crate::vad::vad_model_exists() {
            return;
        }
        let prepared = prepare_fixtures(FIXTURES, 0.5).expect("prepare benchmark fixtures");
        assert_eq!(prepared.len(), FIXTURES.len());
        assert!(prepared
            .iter()
            .all(|fixture| !fixture.samples.is_empty() && fixture.audio_seconds > 0.0));
    }

    #[test]
    fn benchmark_and_shared_backend_changes_are_mutually_exclusive() {
        let coordinator = BenchmarkCoordinator::new();
        assert!(coordinator.try_start_shared_backend_change());
        assert!(!coordinator.try_start());
        coordinator.finish_shared_backend_change();

        assert!(coordinator.try_start());
        assert!(!coordinator.try_start_shared_backend_change());
        coordinator.finish();
        assert!(coordinator.try_start_shared_backend_change());
    }

    fn model(name: &'static str) -> BenchmarkModel {
        benchmark_models()
            .into_iter()
            .find(|model| model.model_name == name)
            .unwrap_or_else(|| panic!("{name} missing from benchmark catalog"))
    }

    #[test]
    fn warmup_plan_is_empty_when_nothing_is_selected() {
        assert!(warmup_plan(&[]).is_empty());
    }

    #[test]
    fn warmup_plan_picks_the_smallest_selected_whisper_model() {
        let selected = [model("medium.en"), model("tiny.en"), model("base.en")];
        assert_eq!(warmup_plan(&selected), vec!["tiny.en".to_string()]);
    }

    #[test]
    fn warmup_plan_skips_whisper_when_no_whisper_model_is_selected() {
        let selected = [model(PARAKEET_CPU_MODEL)];
        assert_eq!(warmup_plan(&selected), vec![PARAKEET_CPU_MODEL.to_string()]);
    }

    #[test]
    fn warmup_plan_warms_each_selected_backend_family_once() {
        let selected = [
            model("base.en"),
            model("large-v3-turbo"),
            model(PARAKEET_CPU_MODEL),
            model(COREML_MODEL_NAME),
        ];
        assert_eq!(
            warmup_plan(&selected),
            vec![
                "base.en".to_string(),
                COREML_MODEL_NAME.to_string(),
                PARAKEET_CPU_MODEL.to_string(),
            ]
        );
    }

    #[test]
    fn balanced_prefers_fastest_model_within_two_accuracy_points() {
        let result = |name: &str, latency: f64, wer: f64| ModelResult {
            model_name: name.to_string(),
            label: name.to_string(),
            backend: String::new(),
            accelerator: String::new(),
            model_load_ms: Some(1.0),
            first_inference_ms: Some(1.0),
            warm_median_ms: Some(latency),
            warm_p95_ms: Some(latency),
            realtime_factor: Some(latency / 1000.0),
            word_error_rate: Some(wer),
            normalized_word_error_rate: Some(wer),
            memory_delta_mb: 0,
            fixtures: Vec::new(),
            error: None,
        };
        let recommendations = recommendations(&[
            result("accurate", 300.0, 0.05),
            result("balanced", 100.0, 0.06),
            result("fast", 50.0, 0.10),
        ]);
        assert_eq!(recommendations.fastest.as_deref(), Some("fast"));
        assert_eq!(recommendations.most_accurate.as_deref(), Some("accurate"));
        assert_eq!(recommendations.balanced.as_deref(), Some("balanced"));
    }

    fn full_result(
        name: &str,
        warm_median_ms: f64,
        realtime_factor: f64,
        wer: f64,
        memory_delta_mb: u64,
    ) -> ModelResult {
        ModelResult {
            model_name: name.to_string(),
            label: name.to_string(),
            backend: String::new(),
            accelerator: String::new(),
            model_load_ms: Some(1.0),
            first_inference_ms: Some(1.0),
            warm_median_ms: Some(warm_median_ms),
            warm_p95_ms: Some(warm_median_ms),
            realtime_factor: Some(realtime_factor),
            word_error_rate: Some(wer),
            memory_delta_mb,
            fixtures: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn fastest_ignores_pooled_median_flip_and_follows_stable_realtime_factor() {
        // Reproduces the issue #272 scenario: two runs of the same models
        // where the pooled warm_median_ms straddles and flips which model
        // looks faster, but realtime_factor (computed per-fixture, stable
        // across fixture mixes) consistently favors the same model.
        let run_a = recommendations(&[
            full_result("tiny.en", 108.6, 0.09, 0.04, 75),
            full_result("parakeet-v3", 95.0, 0.05, 0.03, 470),
        ]);
        let run_b = recommendations(&[
            full_result("tiny.en", 70.0, 0.09, 0.04, 75),
            full_result("parakeet-v3", 77.5, 0.05, 0.03, 470),
        ]);
        assert_eq!(run_a.fastest.as_deref(), Some("parakeet-v3"));
        assert_eq!(run_b.fastest.as_deref(), Some("parakeet-v3"));
    }

    #[test]
    fn fastest_breaks_ties_within_ten_percent_by_lower_memory_then_name() {
        // Realtime factors within 10% of each other are a tie; the model
        // with lower memory delta should win regardless of pooled timing
        // noise or input order.
        let first_order = recommendations(&[
            full_result("heavy", 100.0, 1.00, 0.05, 3000),
            full_result("light", 100.0, 1.05, 0.05, 200),
        ]);
        assert_eq!(first_order.fastest.as_deref(), Some("light"));

        // Order should not matter.
        let second_order = recommendations(&[
            full_result("light", 100.0, 1.05, 0.05, 200),
            full_result("heavy", 100.0, 1.00, 0.05, 3000),
        ]);
        assert_eq!(second_order.fastest.as_deref(), Some("light"));
    }

    #[test]
    fn fastest_tie_band_boundary_is_exactly_ten_percent() {
        // At exactly 10% relative delta the two models are still tied, so
        // the lower-memory model wins even though it is nominally slower.
        let at_boundary = recommendations(&[
            full_result("slower-lighter", 100.0, 1.10, 0.05, 100),
            full_result("faster-heavier", 100.0, 1.00, 0.05, 3000),
        ]);
        assert_eq!(at_boundary.fastest.as_deref(), Some("slower-lighter"));

        // Just past 10% the delta is a real difference, so the faster model
        // wins outright regardless of memory.
        let past_boundary = recommendations(&[
            full_result("slower-lighter", 100.0, 1.1001, 0.05, 100),
            full_result("faster-heavier", 100.0, 1.00, 0.05, 3000),
        ]);
        assert_eq!(past_boundary.fastest.as_deref(), Some("faster-heavier"));
    }

    #[test]
    fn balanced_breaks_ties_within_accuracy_window_by_lower_memory() {
        // Both models are within the 2-point accuracy window and their
        // realtime factors are tied within 10%, so the lower-memory model
        // (matching the Parakeet v2 ~2.9GB-vs-alternatives scenario from the
        // issue) should be recommended as "balanced".
        let recommendations = recommendations(&[
            full_result("parakeet-v2", 100.0, 1.00, 0.05, 2925),
            full_result("small.en", 100.0, 1.03, 0.06, 500),
        ]);
        assert_eq!(recommendations.balanced.as_deref(), Some("small.en"));
    }

    #[test]
    fn balanced_excludes_models_outside_the_accuracy_window() {
        let recommendations = recommendations(&[
            full_result("accurate", 300.0, 0.30, 0.05, 500),
            full_result("inaccurate-but-fast", 10.0, 0.01, 0.20, 50),
        ]);
        assert_eq!(recommendations.balanced.as_deref(), Some("accurate"));
    }
}
