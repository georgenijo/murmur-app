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
const BALANCED_ACCURACY_WINDOW: f64 = 0.02;
const BALANCED_SPEED_TIE_RATIO: f64 = 1.10;

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

fn word_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let reference = words(reference);
    let hypothesis = words(hypothesis);
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
    (previous[hypothesis.len()], reference.len())
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

fn recommendations(results: &[ModelResult]) -> Recommendations {
    let successful = results
        .iter()
        .filter(|result| result.error.is_none())
        .collect::<Vec<_>>();
    let fastest = successful
        .iter()
        .filter_map(|result| result.realtime_factor.map(|value| (*result, value)))
        .min_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.model_name.cmp(&right.0.model_name))
        })
        .map(|(result, _)| result.model_name.clone());
    let most_accurate = successful
        .iter()
        .filter_map(|result| result.word_error_rate.map(|value| (*result, value)))
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(result, _)| result.model_name.clone());
    let best_wer = successful
        .iter()
        .filter_map(|result| result.word_error_rate)
        .min_by(f64::total_cmp);
    let balanced = best_wer.and_then(|best| {
        let candidates = successful
            .iter()
            .filter(|result| {
                result
                    .word_error_rate
                    .is_some_and(|wer| wer <= best + BALANCED_ACCURACY_WINDOW)
            })
            .filter_map(|result| result.realtime_factor.map(|value| (*result, value)))
            .collect::<Vec<_>>();
        let fastest_realtime_factor = candidates
            .iter()
            .map(|(_, realtime_factor)| *realtime_factor)
            .min_by(f64::total_cmp)?;
        candidates
            .into_iter()
            .filter(|(_, realtime_factor)| {
                *realtime_factor <= fastest_realtime_factor * BALANCED_SPEED_TIE_RATIO
            })
            .min_by(|left, right| {
                left.0
                    .memory_delta_mb
                    .cmp(&right.0.memory_delta_mb)
                    .then_with(|| left.1.total_cmp(&right.1))
                    .then_with(|| left.0.model_name.cmp(&right.0.model_name))
            })
            .map(|(result, _)| result.model_name.clone())
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
    let total_steps = selected.len() * steps_per_model;
    let mut completed = 0;
    let mut results = Vec::with_capacity(selected.len());

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
            let mut scored_transcripts = transcripts
                .into_iter()
                .map(|transcript| {
                    let (errors, reference_words) = word_errors(fixture.reference, &transcript);
                    (errors, reference_words, transcript)
                })
                .collect::<Vec<_>>();
            scored_transcripts.sort_by_key(|(errors, _, _)| *errors);
            let (errors, reference_words, transcript) = scored_transcripts
                .into_iter()
                .nth((iterations - 1) / 2)
                .expect("benchmark presets include measured iterations");
            total_errors += errors;
            total_reference_words += reference_words;
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
        results,
        recommendations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_result(
        name: &str,
        warm_median_ms: f64,
        realtime_factor: f64,
        word_error_rate: f64,
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
            word_error_rate: Some(word_error_rate),
            memory_delta_mb,
            fixtures: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn word_error_rate_counts_substitutions_insertions_and_deletions() {
        assert_eq!(word_errors("one two three", "one four three"), (1, 3));
        assert_eq!(word_errors("one two three", "one two three four"), (1, 3));
        assert_eq!(word_errors("one two three", "one three"), (1, 3));
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

    #[test]
    fn fastest_uses_realtime_factor_instead_of_pooled_latency() {
        let recommendations = recommendations(&[
            model_result("tiny.en", 79.1, 0.0086, 0.08, 46),
            model_result("parakeet-v3", 108.6, 0.0080, 0.05, 545),
        ]);

        assert_eq!(recommendations.fastest.as_deref(), Some("parakeet-v3"));
    }

    #[test]
    fn balanced_prefers_lower_memory_within_ten_percent_speed_band() {
        let recommendations = recommendations(&[
            model_result("accurate", 200.0, 0.0120, 0.05, 500),
            model_result("high-memory", 80.0, 0.0080, 0.06, 2_925),
            model_result("balanced", 87.0, 0.0087, 0.07, 545),
            model_result("fast-outside-window", 50.0, 0.0050, 0.10, 46),
        ]);

        assert_eq!(
            recommendations.fastest.as_deref(),
            Some("fast-outside-window")
        );
        assert_eq!(recommendations.most_accurate.as_deref(), Some("accurate"));
        assert_eq!(recommendations.balanced.as_deref(), Some("balanced"));
    }

    #[test]
    fn balanced_keeps_a_clear_speed_winner_despite_higher_memory() {
        let recommendations = recommendations(&[
            model_result("accurate", 200.0, 0.0200, 0.05, 500),
            model_result("clear-speed-winner", 80.0, 0.0080, 0.06, 2_925),
            model_result("low-memory", 90.0, 0.0090, 0.06, 46),
        ]);

        assert_eq!(
            recommendations.balanced.as_deref(),
            Some("clear-speed-winner")
        );
    }

    #[test]
    fn recommendations_ignore_failed_and_incomplete_results() {
        let valid = model_result("valid", 100.0, 0.0100, 0.05, 100);
        let mut failed = model_result("failed", 10.0, 0.0010, 0.01, 1);
        failed.error = Some("inference failed".to_string());
        let mut incomplete = model_result("incomplete", 20.0, 0.0020, 0.02, 2);
        incomplete.realtime_factor = None;
        incomplete.word_error_rate = None;

        let recommendations = recommendations(&[failed, incomplete, valid]);

        assert_eq!(recommendations.fastest.as_deref(), Some("valid"));
        assert_eq!(recommendations.most_accurate.as_deref(), Some("valid"));
        assert_eq!(recommendations.balanced.as_deref(), Some("valid"));
    }

    #[test]
    fn speed_and_memory_ties_use_model_name_deterministically() {
        let recommendations = recommendations(&[
            model_result("zeta", 100.0, 0.0100, 0.05, 100),
            model_result("alpha", 100.0, 0.0100, 0.05, 100),
        ]);

        assert_eq!(recommendations.fastest.as_deref(), Some("alpha"));
        assert_eq!(recommendations.balanced.as_deref(), Some("alpha"));
    }
}
