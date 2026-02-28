use std::path::PathBuf;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

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
/// `model_path` must point to a valid `ggml-silero-vad.bin` file.
/// This function creates a `WhisperVadContext` which is `!Send`, so it must
/// run entirely within a single thread (use `spawn_blocking`).
pub fn filter_speech(model_path: &str, samples: &[f32]) -> Result<VadResult, String> {
    let mut ctx_params = WhisperVadContextParams::default();
    ctx_params.set_n_threads(1);
    ctx_params.set_use_gpu(false);

    let mut ctx = WhisperVadContext::new(model_path, ctx_params)
        .map_err(|e| format!("Failed to create VAD context: {}", e))?;

    let vad_params = WhisperVadParams::default();
    // Default params already match our desired values:
    //   threshold: 0.5, min_speech_duration: 250ms,
    //   min_silence_duration: 100ms, speech_pad: 30ms

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
