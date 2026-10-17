use std::cell::RefCell;
use std::path::PathBuf;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

thread_local! {
    /// Whisper's VAD context is not Send/Sync, so keep one cache per blocking
    /// worker. Tokio normally reuses those workers; a different worker simply
    /// pays the small initialization cost once for its own context.
    static VAD_CONTEXT: RefCell<Option<(String, WhisperVadContext)>> = const { RefCell::new(None) };
}

pub const VAD_MODEL_FILENAME: &str = "ggml-silero-v5.1.2.bin";
pub const VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";
const SPEECH_PAD_MS: i32 = 120;

/// Expected path for the VAD model under the app's models directory.
pub fn vad_model_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("local-dictation")
            .join("models")
            .join(VAD_MODEL_FILENAME)
    })
}

/// Check whether the VAD model file exists on disk.
pub fn vad_model_exists() -> bool {
    vad_model_path().is_some_and(|path| {
        crate::model_artifact::binary_model_is_valid(
            &path,
            crate::model_artifact::MIN_VAD_MODEL_BYTES,
        )
    })
}

pub enum VadResult {
    /// No speech detected in the audio.
    NoSpeech,
    /// Speech detected — contains trimmed samples (speech segments only).
    Speech(Vec<f32>),
}

/// Zero is the explicit lowest-latency setting; non-zero values run VAD.
pub fn is_enabled(sensitivity: u32) -> bool {
    sensitivity > 0
}

/// Map the user-facing sensitivity scale to whisper.cpp's VAD threshold.
/// Higher sensitivity keeps more audio, so it uses a lower threshold.
pub fn threshold_for_sensitivity(sensitivity: u32) -> f32 {
    1.0 - (sensitivity.clamp(0, 100) as f32 / 100.0)
}

/// Run Silero VAD on the given 16kHz mono samples and return only speech segments.
///
/// `model_path` must point to a valid Silero VAD GGML model file.
/// This function creates a `WhisperVadContext` which is `!Send`, so it must
/// run entirely within a single thread (use `spawn_blocking`).
pub fn filter_speech(
    model_path: &str,
    samples: &[f32],
    threshold: f32,
) -> Result<VadResult, String> {
    VAD_CONTEXT.with(|cache| {
        let mut cached = cache.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|(cached_path, _)| cached_path != model_path)
        {
            let mut ctx_params = WhisperVadContextParams::default();
            ctx_params.set_n_threads(1);
            ctx_params.set_use_gpu(false);
            let context = WhisperVadContext::new(model_path, ctx_params)
                .map_err(|e| format!("Failed to create VAD context: {}", e))?;
            *cached = Some((model_path.to_string(), context));
        }

        let context = &mut cached
            .as_mut()
            .expect("VAD context was initialized above")
            .1;
        filter_speech_with_context(context, samples, threshold)
    })
}

fn filter_speech_with_context(
    ctx: &mut WhisperVadContext,
    samples: &[f32],
    threshold: f32,
) -> Result<VadResult, String> {
    let mut vad_params = WhisperVadParams::default();
    vad_params.set_threshold(threshold);
    // Preserve the initial consonant around VAD boundaries. The upstream
    // default is only 30 ms, which is easy to clip when speech begins as the
    // hotkey is pressed.
    vad_params.set_speech_pad(SPEECH_PAD_MS);

    let segments = ctx
        .segments_from_samples(vad_params, samples)
        .map_err(|e| format!("VAD inference failed: {}", e))?;

    let num_segments = segments.num_segments();
    if num_segments == 0 {
        return Ok(VadResult::NoSpeech);
    }

    let sample_rate = 16_000.0_f32;
    let total_samples = samples.len();
    let mut speech_samples = Vec::new();

    for seg in segments {
        // Timestamps are in centiseconds (cs); convert to sample indices
        let start_idx = ((seg.start / 100.0) * sample_rate) as usize;
        let end_idx = ((seg.end / 100.0) * sample_rate).ceil() as usize;

        let start_idx = start_idx.min(total_samples);
        let end_idx = end_idx.min(total_samples);

        if start_idx < end_idx {
            speech_samples.extend_from_slice(&samples[start_idx..end_idx]);
        }
    }

    if speech_samples.is_empty() {
        Ok(VadResult::NoSpeech)
    } else {
        Ok(VadResult::Speech(speech_samples))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Public synthetic speech only; never accepts a private corpus path.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "loads installed Core ML and VAD models; synthetic fixture diagnostic"]
    fn synthetic_attenuated_speech_and_silence_diagnostic() {
        use crate::transcriber::{
            parse_wav_to_samples, CoreMlBackend, TranscriptionBackend, COREML_MODEL_NAME,
        };
        let path = vad_model_path()
            .filter(|path| path.exists())
            .expect("installed VAD required");
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/audio/short.wav");
        let clean = parse_wav_to_samples(&std::fs::read(fixture).unwrap()).unwrap();
        assert!(!clean.is_empty());
        let mut backend = CoreMlBackend::new();
        backend
            .load_model(COREML_MODEL_NAME)
            .expect("installed Core ML required");
        for gain_db in [0, -12, -24, -30, -36, -48, -96] {
            let gain = if gain_db == -96 {
                0.0
            } else {
                10.0f32.powf(gain_db as f32 / 20.0)
            };
            let mut samples: Vec<f32> = clean.iter().map(|sample| sample * gain).collect();
            samples.resize(samples.len() + 24_000, 0.0); // 1.5 seconds trailing silence.
            let mut nonempty_previews = 0;
            for end in (12_800..samples.len()).step_by(11_200) {
                let window = &samples[end.saturating_sub(320_000)..end];
                let preview = backend
                    .transcribe_preview(
                        window,
                        "auto",
                        None,
                        true,
                        crate::transcriber::PreviewPass::SyntheticDiagnostic,
                    )
                    .unwrap();
                nonempty_previews += usize::from(!preview.trim().is_empty());
            }
            let (vad_speech, kept_samples, final_nonempty) = match filter_speech(
                path.to_str().unwrap(),
                &samples,
                threshold_for_sensitivity(50),
            )
            .unwrap()
            {
                VadResult::NoSpeech => (false, 0, false),
                VadResult::Speech(kept) => {
                    let final_text = backend.transcribe(&kept, "auto", None, true).unwrap();
                    (true, kept.len(), !final_text.trim().is_empty())
                }
            };
            println!(
                "{}",
                serde_json::json!({
                    "fixture": if gain == 0.0 { "synthetic_silence" } else { "synthetic_short" },
                    "gain_db": gain_db, "input_samples": samples.len(), "kept_samples": kept_samples,
                    "rms": crate::audio::compute_rms(&samples), "peak": crate::audio::compute_peak(&samples),
                    "nonempty_previews": nonempty_previews, "vad_speech": vad_speech,
                    "final_nonempty": final_nonempty, "disagreement": nonempty_previews > 0 && !vad_speech,
                })
            );
            if gain_db == 0 {
                assert!(nonempty_previews > 0 && vad_speech && final_nonempty);
            }
            if gain == 0.0 {
                assert!(!vad_speech);
            }
        }
    }

    #[test]
    fn zero_sensitivity_disables_vad() {
        assert!(!is_enabled(0));
        assert!(is_enabled(1));
        assert!(is_enabled(100));
    }

    #[test]
    fn sensitivity_threshold_mapping_is_clamped_and_shared() {
        assert_eq!(threshold_for_sensitivity(0), 1.0);
        assert_eq!(threshold_for_sensitivity(50), 0.5);
        assert_eq!(threshold_for_sensitivity(100), 0.0);
        assert_eq!(threshold_for_sensitivity(150), 0.0);
    }

    #[test]
    fn missing_model_returns_an_error_without_populating_cache() {
        let missing = std::env::temp_dir().join("murmur-missing-vad-model.bin");
        let missing = missing.to_string_lossy().into_owned();
        let result = filter_speech(&missing, &[0.0; 16_000], 0.5);
        assert!(result.is_err());
        VAD_CONTEXT.with(|cache| {
            assert!(
                cache
                    .borrow()
                    .as_ref()
                    .is_none_or(|(path, _)| path != &missing),
                "a failed context must not be cached"
            );
        });
    }

    #[test]
    fn installed_model_context_is_reused_on_the_same_worker() {
        let Some(path) = vad_model_path().filter(|path| path.exists()) else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let samples = vec![0.0; 16_000];
        filter_speech(&path, &samples, 0.5).expect("first VAD pass");
        let first_context = VAD_CONTEXT.with(|cache| {
            let cache = cache.borrow();
            let (_, context) = cache.as_ref().expect("context should be cached");
            context as *const WhisperVadContext as usize
        });
        filter_speech(&path, &samples, 0.5).expect("second VAD pass");
        let second_context = VAD_CONTEXT.with(|cache| {
            let cache = cache.borrow();
            let (_, context) = cache.as_ref().expect("context should be cached");
            context as *const WhisperVadContext as usize
        });
        assert_eq!(first_context, second_context);
    }
}
