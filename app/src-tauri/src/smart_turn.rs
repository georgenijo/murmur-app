//! Smart Turn end-of-turn scoring for hands-free auto-stop (#834).
//!
//! The frontend's Stop on Silence detector decides when a pause after speech
//! is worth asking about; this module answers one question for it: given the
//! last 8 s of the recording at 16 kHz, how likely is it that the speaker has
//! finished their turn? Pipecat's Smart Turn v3.2 (BSD-2-Clause) scores Whisper
//! log-mel features of that window. The silence timer stays the hard ceiling,
//! so every failure here — model missing, load error, inference error, busy —
//! degrades to plain Stop on Silence.
//!
//! The feature extractor and the ONNX session setup are adapted from Hex
//! (`src/smart_turn.rs` at a07f994, MIT, Copyright (c) 2026 Christopher
//! Langton); see THIRD_PARTY_NOTICES.md.
//!
//! Runtime: the app already links ONNX Runtime 1.24 statically through
//! sherpa-onnx. `ort` is built with `alternative-backend`, which links nothing,
//! and is handed that same runtime's C API here, so there is one ONNX Runtime
//! in the process and no dylib to bundle.
//!
//! Threading: everything in this module is synchronous and runs on a Tokio
//! blocking worker. It never touches the capture thread; audio arrives as a
//! snapshot copy of the retained recording buffer.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use realfft::RealFftPlanner;

pub const SAMPLE_RATE: usize = 16_000;
/// Smart Turn scores the most recent 8 s, left-padded with silence.
pub const WINDOW_SAMPLES: usize = 8 * SAMPLE_RATE;
const MEL_BINS: usize = 80;
const N_FFT: usize = 400;
const HOP: usize = 160;
const MEL_FRAMES: usize = WINDOW_SAMPLES / HOP;

pub const MODEL_FILENAME: &str = "smart-turn-v3.2-cpu.onnx";
/// Pinned to an immutable Hugging Face revision.
pub const MODEL_URL: &str = "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/smart-turn-v3.2-cpu.onnx";
pub const MODEL_SIZE_BYTES: u64 = 8_679_182;
pub const MODEL_SHA256: &str = "2bb026316b14a660486a75b1733cd3fbab8c2fd0314dc9af7be49f8cca967e4f";

/// The ONNX Runtime C API version requested from the linked runtime. 24 is
/// ONNX Runtime 1.24, the version sherpa-onnx 1.13 ships.
const ORT_API_VERSION: u32 = 24;

/// Root of the hash-versioned Smart Turn store, beneath the app models dir.
pub fn models_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| {
        dir.join("local-dictation")
            .join("models")
            .join("smart-turn")
    })
}

/// Where the installed, verified model publishes to.
pub fn installed_model_path() -> Option<PathBuf> {
    Some(models_root()?.join(MODEL_SHA256).join(MODEL_FILENAME))
}

/// A published model of exactly the pinned size counts as installed. The full
/// SHA-256 is re-verified on every load.
pub fn model_installed() -> bool {
    installed_model_path()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == MODEL_SIZE_BYTES)
}

/// Why a check produced no probability. Every variant leaves the recording to
/// the silence timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckError {
    /// The model is not downloaded.
    NotInstalled,
    /// Another check is still running; this pause is not scored.
    Busy,
    /// The runtime or model failed to load.
    LoadFailed,
    /// Inference failed or returned a non-finite score.
    InferenceFailed,
}

impl CheckError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Busy => "busy",
            Self::LoadFailed => "load_failed",
            Self::InferenceFailed => "inference_failed",
        }
    }
}

/// One scored pause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub probability: f32,
    pub inference_ms: u64,
    /// True when this check also loaded the model.
    pub loaded: bool,
}

/// Scores whether `features` (from [`log_mel`]) end a turn.
pub trait Predictor: Send {
    fn predict(&mut self, features: &[f32]) -> Result<f32, String>;
}

/// The process-wide loaded model. `None` until the first check after the
/// model is installed; dropped by [`unload`] when the model is removed.
static PREDICTOR: Mutex<Option<Box<dyn Predictor>>> = Mutex::new(None);

/// Score the tail of a 16 kHz recording. Loads the model on first use.
/// Never waits behind a running check: overlapping calls return `Busy`.
pub fn score(samples: &[f32]) -> Result<Score, CheckError> {
    score_with(&PREDICTOR, samples, || {
        let path = installed_model_path().ok_or(CheckError::NotInstalled)?;
        if !model_installed() {
            return Err(CheckError::NotInstalled);
        }
        load_onnx(&path).map_err(|error| {
            tracing::warn!(target: "recording", error = %error, "smart turn model load failed");
            CheckError::LoadFailed
        })
    })
}

fn score_with(
    slot: &Mutex<Option<Box<dyn Predictor>>>,
    samples: &[f32],
    load: impl FnOnce() -> Result<Box<dyn Predictor>, CheckError>,
) -> Result<Score, CheckError> {
    let mut guard = match slot.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Err(CheckError::Busy),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
    };
    let loaded = guard.is_none();
    if loaded {
        *guard = Some(load()?);
    }
    let predictor = guard.as_mut().expect("predictor was just loaded");
    let started = Instant::now();
    let probability = predictor
        .predict(&log_mel(samples))
        .ok()
        .filter(|probability| probability.is_finite())
        .ok_or(CheckError::InferenceFailed)?;
    Ok(Score {
        probability,
        inference_ms: started.elapsed().as_millis() as u64,
        loaded,
    })
}

/// Drop the loaded model so its file is no longer open. Waits for a running
/// check, which is bounded by one inference.
pub fn unload() {
    *PREDICTOR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Whisper log-mel features as Hugging Face's
/// `WhisperFeatureExtractor(chunk_length=8)` computes them with
/// `do_normalize=True`: the last 8 s, left-padded with zeros, normalized to
/// zero mean and unit variance, then an 80-bin Slaney mel spectrogram of a
/// 400-point periodic-Hann STFT with 160-sample hops. Row-major `[80][800]`.
pub fn log_mel(audio: &[f32]) -> Vec<f32> {
    static FILTERS: OnceLock<MelFilters> = OnceLock::new();
    let filters = FILTERS.get_or_init(MelFilters::new);

    let tail = &audio[audio.len().saturating_sub(WINDOW_SAMPLES)..];
    let mut signal = vec![0.0_f64; WINDOW_SAMPLES];
    for (target, sample) in signal[WINDOW_SAMPLES - tail.len()..].iter_mut().zip(tail) {
        *target = if sample.is_finite() {
            f64::from(*sample)
        } else {
            0.0
        };
    }
    let mean = signal.iter().sum::<f64>() / WINDOW_SAMPLES as f64;
    let variance = signal.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / WINDOW_SAMPLES as f64;
    let scale = (variance + 1e-7).sqrt();
    for x in &mut signal {
        // The reference normalizes in float32.
        *x = f64::from(((*x - mean) / scale) as f32);
    }

    // Centered frames with reflect padding.
    let pad = N_FFT / 2;
    let mut padded = Vec::with_capacity(WINDOW_SAMPLES + 2 * pad);
    padded.extend((1..=pad).rev().map(|index| signal[index]));
    padded.extend_from_slice(&signal);
    padded.extend((1..=pad).map(|offset| signal[WINDOW_SAMPLES - 1 - offset]));

    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(N_FFT);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut power = vec![0.0_f64; N_FFT / 2 + 1];
    let mut mel = vec![0.0_f64; MEL_BINS * MEL_FRAMES];
    // The reference drops the STFT's last frame, so 800 of 801 remain.
    for frame in 0..MEL_FRAMES {
        let start = frame * HOP;
        for ((target, sample), weight) in input
            .iter_mut()
            .zip(&padded[start..start + N_FFT])
            .zip(&filters.window)
        {
            *target = sample * weight;
        }
        fft.process(&mut input, &mut spectrum)
            .expect("FFT buffers have the planned lengths");
        for (target, bin) in power.iter_mut().zip(&spectrum) {
            *target = bin.norm_sqr();
        }
        for (bin, (first, weights)) in filters.bins.iter().enumerate() {
            let energy = weights
                .iter()
                .zip(&power[*first..])
                .map(|(weight, power)| weight * power)
                .sum::<f64>();
            mel[bin * MEL_FRAMES + frame] = energy.max(1e-10).log10();
        }
    }
    let ceiling = mel.iter().copied().fold(f64::NEG_INFINITY, f64::max) - 8.0;
    mel.into_iter()
        .map(|value| ((value.max(ceiling) + 4.0) / 4.0) as f32)
        .collect()
}

struct MelFilters {
    window: Vec<f64>,
    /// Each mel bin's first nonzero FFT bin and its weights from there.
    bins: Vec<(usize, Vec<f64>)>,
}

impl MelFilters {
    fn new() -> Self {
        let window = (0..N_FFT)
            .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos())
            .collect();
        let fft_bins = N_FFT / 2 + 1;
        let nyquist = SAMPLE_RATE as f64 / 2.0;
        let fft_frequencies = (0..fft_bins)
            .map(|bin| nyquist * bin as f64 / (fft_bins - 1) as f64)
            .collect::<Vec<_>>();
        let low = hertz_to_mel(0.0);
        let high = hertz_to_mel(nyquist);
        let edges = (0..MEL_BINS + 2)
            .map(|index| mel_to_hertz(low + (high - low) * index as f64 / (MEL_BINS + 1) as f64))
            .collect::<Vec<_>>();
        let bins = (0..MEL_BINS)
            .map(|bin| {
                let (left, center, right) = (edges[bin], edges[bin + 1], edges[bin + 2]);
                let slaney = 2.0 / (right - left);
                let weights = fft_frequencies
                    .iter()
                    .map(|frequency| {
                        let rising = (frequency - left) / (center - left);
                        let falling = (right - frequency) / (right - center);
                        rising.min(falling).max(0.0) * slaney
                    })
                    .collect::<Vec<_>>();
                let first = weights.iter().position(|w| *w > 0.0).unwrap_or(0);
                let last = weights.iter().rposition(|w| *w > 0.0).unwrap_or(first);
                (first, weights[first..=last].to_vec())
            })
            .collect();
        Self { window, bins }
    }
}

const MIN_LOG_HERTZ: f64 = 1_000.0;
const MIN_LOG_MEL: f64 = 15.0;

fn hertz_to_mel(hertz: f64) -> f64 {
    if hertz >= MIN_LOG_HERTZ {
        MIN_LOG_MEL + (hertz / MIN_LOG_HERTZ).ln() * (27.0 / 6.4_f64.ln())
    } else {
        3.0 * hertz / 200.0
    }
}

fn mel_to_hertz(mel: f64) -> f64 {
    if mel >= MIN_LOG_MEL {
        MIN_LOG_HERTZ * ((6.4_f64.ln() / 27.0) * (mel - MIN_LOG_MEL)).exp()
    } else {
        200.0 * mel / 3.0
    }
}

struct OnnxPredictor {
    session: ort::session::Session,
}

impl Predictor for OnnxPredictor {
    fn predict(&mut self, features: &[f32]) -> Result<f32, String> {
        let input = ort::value::Tensor::from_array((
            [1_usize, MEL_BINS, MEL_FRAMES],
            features.to_vec().into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let outputs = self
            .session
            .run(ort::inputs!["input_features" => input])
            .map_err(|error| error.to_string())?;
        let (_, probabilities) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| error.to_string())?;
        probabilities
            .first()
            .copied()
            .ok_or_else(|| "Smart Turn returned no probability".to_string())
    }
}

/// Verify the pinned digest, then open a single-threaded session: one
/// inference is ~10 ms of CPU and must not compete with transcription.
fn load_onnx(path: &Path) -> Result<Box<dyn Predictor>, String> {
    let (size, sha) =
        crate::llm_sidecar::model_file_digest(path).map_err(|error| error.to_string())?;
    if size != MODEL_SIZE_BYTES || sha != MODEL_SHA256 {
        return Err("Smart Turn model failed verification".to_string());
    }
    init_runtime()?;
    let builder_error =
        |error: ort::Error<ort::session::builder::SessionBuilder>| error.to_string();
    let session = ort::session::Session::builder()
        .map_err(|error| error.to_string())?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
        .map_err(builder_error)?
        .with_parallel_execution(false)
        .map_err(builder_error)?
        .with_intra_threads(1)
        .map_err(builder_error)?
        .with_inter_threads(1)
        .map_err(builder_error)?
        .with_intra_op_spinning(false)
        .map_err(builder_error)?
        .commit_from_file(path)
        .map_err(|error| error.to_string())?;
    Ok(Box::new(OnnxPredictor { session }))
}

/// Point `ort` at the ONNX Runtime sherpa-onnx links statically, once.
fn init_runtime() -> Result<(), String> {
    static RUNTIME: OnceLock<Result<(), String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            // SAFETY: `OrtGetApiBase` is the runtime's C entry point; it takes
            // no arguments and returns a static table (or null).
            let base = unsafe { ort::sys::OrtGetApiBase() };
            if base.is_null() {
                return Err("ONNX Runtime returned no API base".to_string());
            }
            // SAFETY: `base` is non-null and points at the runtime's static
            // table; `GetApi` returns null for an unsupported version.
            let api = unsafe { ((*base).GetApi)(ORT_API_VERSION) };
            if api.is_null() {
                return Err(format!(
                    "linked ONNX Runtime does not support API version {ORT_API_VERSION}"
                ));
            }
            // SAFETY: a non-null `GetApi` result is a valid `OrtApi` table of
            // plain function pointers that lives for the process; reading it
            // makes the bitwise copy `set_api` keeps.
            ort::set_api(unsafe { std::ptr::read(api) });
            Ok(())
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference signal Hex fed `WhisperFeatureExtractor`: 2.5 s of a
    /// modulated 220 Hz tone plus 1250 Hz, then 0.5 s of a faint 50 Hz hum.
    fn reference_signal() -> Vec<f32> {
        (0..3 * SAMPLE_RATE)
            .map(|n| {
                let t = n as f64 / SAMPLE_RATE as f64;
                let tau = 2.0 * std::f64::consts::PI;
                let value = if t < 2.5 {
                    0.3 * (tau * 220.0 * t).sin() * (0.5 + 0.5 * (tau * 3.0 * t).sin())
                        + 0.1 * (tau * 1250.0 * t).sin()
                } else {
                    0.001 * (tau * 50.0 * t).sin()
                };
                value as f32
            })
            .collect()
    }

    #[test]
    fn log_mel_matches_whisper_feature_extractor() {
        let features = log_mel(&reference_signal());
        assert_eq!(features.len(), MEL_BINS * MEL_FRAMES);
        // transformers' WhisperFeatureExtractor(chunk_length=8) on the same
        // signal, as recorded in Hex's test of the same extractor.
        let picks = [
            (2, 520, 0.712_121_6),
            (5, 600, 1.688_267),
            (10, 640, 0.636_762_7),
            (15, 700, 0.017_422_2),
            (30, 745, 1.278_610_7),
            (60, 600, -0.161_787_63),
            (79, 700, -0.161_787_63),
            (1, 780, 0.560_602_3),
            (0, 0, -0.161_787_63),
            (0, 799, 0.632_712_4),
        ];
        for (bin, frame, expected) in picks {
            let actual = features[bin * MEL_FRAMES + frame];
            assert!(
                (actual - expected).abs() < 1e-3,
                "bin {bin} frame {frame}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn non_finite_samples_read_as_silence() {
        let mut audio = vec![0.0_f32; SAMPLE_RATE];
        audio[100] = f32::NAN;
        audio[200] = f32::INFINITY;
        assert!(log_mel(&audio).iter().all(|value| value.is_finite()));
    }

    #[test]
    fn longer_audio_scores_only_the_last_window() {
        let tail = reference_signal();
        let mut long = vec![0.25_f32; 5 * SAMPLE_RATE];
        long.extend(vec![0.0_f32; WINDOW_SAMPLES - tail.len()]);
        long.extend(&tail);
        assert_eq!(log_mel(&long), log_mel(&tail));
    }

    struct Scripted(Result<f32, String>);
    impl Predictor for Scripted {
        fn predict(&mut self, features: &[f32]) -> Result<f32, String> {
            assert_eq!(features.len(), MEL_BINS * MEL_FRAMES);
            self.0.clone()
        }
    }

    fn slot() -> Mutex<Option<Box<dyn Predictor>>> {
        Mutex::new(None)
    }

    #[test]
    fn loads_once_then_reuses_the_session() {
        let slot = slot();
        let first = score_with(&slot, &[0.0; 1600], || Ok(Box::new(Scripted(Ok(0.9))))).unwrap();
        assert!(first.loaded);
        assert_eq!(first.probability, 0.9);
        let second = score_with(&slot, &[0.0; 1600], || panic!("must not reload")).unwrap();
        assert!(!second.loaded);
    }

    #[test]
    fn a_missing_model_or_failed_load_reports_and_stays_unloaded() {
        let slot = slot();
        assert_eq!(
            score_with(&slot, &[], || Err(CheckError::NotInstalled)),
            Err(CheckError::NotInstalled)
        );
        assert!(slot.lock().unwrap().is_none());
        assert_eq!(
            score_with(&slot, &[], || Err(CheckError::LoadFailed)),
            Err(CheckError::LoadFailed)
        );
    }

    #[test]
    fn inference_errors_and_non_finite_scores_fail_the_check() {
        let slot = slot();
        assert_eq!(
            score_with(&slot, &[], || Ok(Box::new(Scripted(Err("boom".into()))))),
            Err(CheckError::InferenceFailed)
        );
        let slot = self::slot();
        assert_eq!(
            score_with(&slot, &[], || Ok(Box::new(Scripted(Ok(f32::NAN))))),
            Err(CheckError::InferenceFailed)
        );
    }

    #[test]
    fn an_overlapping_check_is_busy_instead_of_waiting() {
        let slot = slot();
        let _held = slot.lock().unwrap();
        assert_eq!(
            score_with(&slot, &[], || panic!("must not load")),
            Err(CheckError::Busy)
        );
    }

    #[test]
    fn model_pins_are_well_formed() {
        assert_eq!(MODEL_SHA256.len(), 64);
        assert!(MODEL_URL.ends_with(MODEL_FILENAME));
        assert!(MODEL_URL.contains("/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/"));
        let path = installed_model_path().unwrap();
        assert!(path.ends_with(Path::new(MODEL_SHA256).join(MODEL_FILENAME)));
    }

    /// Real-model check and synthetic end-of-turn measurement. Needs the
    /// downloaded model (Settings → Recording → Smart Auto-Stop, or
    /// `SMART_TURN_MODEL=/path/to/smart-turn-v3.2-cpu.onnx`) and macOS `say`.
    /// Prints only content-free scores and timings.
    ///
    /// `cargo test --lib smart_turn::tests::real_model -- --ignored --nocapture`
    ///
    /// It asserts runtime parity, not accuracy: Smart Turn is trained on human
    /// speech and `say` voices are far out of its distribution, so their
    /// scores are a smoke measurement (reported in the PR), not a quality bar.
    #[test]
    #[ignore = "needs the downloaded Smart Turn model and macOS `say`"]
    fn real_model_scores_reference_and_synthetic_turns() {
        const THRESHOLD: f32 = 0.7;
        let model = std::env::var_os("SMART_TURN_MODEL")
            .map(PathBuf::from)
            .or_else(installed_model_path)
            .expect("model path");
        let mut predictor = load_onnx(&model).expect("load model");

        // Hex: Python onnxruntime 1.23.2 scores these features 0.8642676; the
        // quantized model moves slightly with the runtime version.
        let reference = predictor.predict(&log_mel(&reference_signal())).unwrap();
        println!("reference_signal probability={reference:.4}");
        assert!((reference - 0.864_267_6).abs() < 5e-3, "P {reference}");

        // Each sentence includes an embedded one-second pause. Score its
        // prefix plus 300 ms silence, then the complete sentence with the
        // same trailing silence. Synthetic scores do not establish human
        // accuracy; report false stops and true ends without tuning to them.
        let sentences = [
            "I think we should move the meeting to [[slnc 1000]] Friday afternoon.",
            "The main reason this keeps failing is that the [[slnc 1000]] network drops.",
            "Can you send me the report and also [[slnc 1000]] the slides?",
            "So what I wanted to ask you about was [[slnc 1000]] the budget.",
            "Let's start with the budget and then we can [[slnc 1000]] talk about hiring.",
            "My plan for the weekend is to [[slnc 1000]] go hiking with friends.",
            "We need three more people for the [[slnc 1000]] design review.",
            "If the build passes tonight we could [[slnc 1000]] ship it tomorrow.",
        ];
        let check = SAMPLE_RATE * 300 / 1000;
        let dir = std::env::temp_dir().join(format!("murmur-smart-turn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("fixture.wav");
        let say = |text: &str| {
            let mut child = std::process::Command::new("/usr/bin/say")
                .args([
                    "-v",
                    "Samantha",
                    "--file-format=WAVE",
                    "--data-format=LEI16@16000",
                    "-o",
                ])
                .arg(&wav)
                .arg(text)
                .spawn()
                .expect("run say");
            let deadline = Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(status) = child.try_wait().expect("wait for say") {
                    assert!(status.success(), "say failed");
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("say exceeded its synthetic-fixture deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            read_wav_f32(&wav)
        };
        let (mut mid, mut end, mut inference_ms) = (Vec::new(), Vec::new(), Vec::new());
        let mut scored = |audio: &[f32]| {
            let started = Instant::now();
            let probability = predictor.predict(&log_mel(audio)).unwrap();
            inference_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            assert!(probability.is_finite());
            probability
        };
        for sentence in sentences {
            let paused = say(sentence);
            let pause = embedded_pause_start(&paused).expect("embedded pause");
            let mut before = paused[..pause].to_vec();
            before.extend(std::iter::repeat_n(0.0, check));
            mid.push(scored(&before));
            let mut whole = say(&sentence.replace(" [[slnc 1000]]", ""));
            whole.extend(std::iter::repeat_n(0.0, check));
            end.push(scored(&whole));
        }
        let _ = std::fs::remove_dir_all(&dir);

        let false_stops = mid.iter().filter(|p| **p >= THRESHOLD).count();
        let stops = end.iter().filter(|p| **p >= THRESHOLD).count();
        let mean_ms = inference_ms.iter().sum::<f64>() / inference_ms.len() as f64;
        let max_ms = inference_ms.iter().copied().fold(0.0, f64::max);
        println!(
            "mid_sentence false_stops={false_stops}/{} probabilities={mid:.3?}",
            mid.len()
        );
        println!(
            "turn_end stops={stops}/{} probabilities={end:.3?}",
            end.len()
        );
        println!("features+inference ms mean={mean_ms:.1} max={max_ms:.1}");
    }

    /// First 500 ms near-silent run after speech, locating the embedded pause.
    fn embedded_pause_start(audio: &[f32]) -> Option<usize> {
        let run = SAMPLE_RATE / 2;
        let speech = audio.iter().position(|x| x.abs() > 0.01)?;
        let mut quiet = 0;
        for (index, sample) in audio.iter().enumerate().skip(speech) {
            if sample.abs() < 1e-3 {
                quiet += 1;
                if quiet == run {
                    return Some(index + 1 - run);
                }
            } else {
                quiet = 0;
            }
        }
        None
    }

    fn read_wav_f32(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("open wav");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate as usize, SAMPLE_RATE);
        assert_eq!(spec.channels, 1);
        match spec.sample_format {
            hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|sample| sample.unwrap() as f32 / (1_u32 << (spec.bits_per_sample - 1)) as f32)
                .collect(),
        }
    }
}
