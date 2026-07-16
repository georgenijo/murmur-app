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

/// Expected path for the VAD model under the app's models directory.
pub fn vad_model_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("local-dictation").join("models").join(VAD_MODEL_FILENAME))
}

/// Check whether the VAD model file exists on disk.
pub fn vad_model_exists() -> bool {
    vad_model_path().is_some_and(|p| p.exists())
}

pub enum VadResult {
    /// No speech detected in the audio.
    NoSpeech,
    /// Speech detected — contains trimmed samples (speech segments only).
    Speech(Vec<f32>),
}

/// Run Silero VAD on the given 16kHz mono samples and return only speech segments.
///
/// `model_path` must point to a valid Silero VAD GGML model file.
/// This function creates a `WhisperVadContext` which is `!Send`, so it must
/// run entirely within a single thread (use `spawn_blocking`).
pub fn filter_speech(model_path: &str, samples: &[f32], threshold: f32) -> Result<VadResult, String> {
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

    #[test]
    fn missing_model_returns_an_error_without_populating_cache() {
        let missing = std::env::temp_dir().join("murmur-missing-vad-model.bin");
        let missing = missing.to_string_lossy().into_owned();
        let result = filter_speech(&missing, &[0.0; 16_000], 0.5);
        assert!(result.is_err());
        VAD_CONTEXT.with(|cache| {
            assert!(
                cache.borrow().as_ref().is_none_or(|(path, _)| path != &missing),
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
