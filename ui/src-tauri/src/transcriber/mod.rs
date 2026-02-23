mod whisper;

pub use whisper::WhisperBackend;

use crate::state::WHISPER_SAMPLE_RATE;
use hound::WavReader;
use std::io::Cursor;
use std::path::PathBuf;

/// Abstraction over transcription engines (whisper, moonshine, etc.)
#[allow(dead_code)]
pub trait TranscriptionBackend: Send + Sync {
    /// Human-readable backend name (e.g., "whisper", "moonshine")
    fn name(&self) -> &str;

    /// Load model by name. Called lazily on first transcription.
    fn load_model(&mut self, model_name: &str) -> Result<(), String>;

    /// Run inference on 16kHz mono f32 samples.
    fn transcribe(&self, samples: &[f32], language: &str) -> Result<String, String>;

    /// Check if any model file exists in search paths.
    fn model_exists(&self) -> bool;

    /// Get the directory where models are stored (for downloads).
    fn models_dir(&self) -> Result<PathBuf, String>;

    /// Reset loaded model so next transcription triggers a reload.
    fn reset(&mut self);
}

/// Parse WAV audio bytes and convert to f32 samples for transcription.
pub fn parse_wav_to_samples(wav_bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(wav_bytes);
    let reader =
        WavReader::new(cursor).map_err(|e| format!("Failed to parse WAV: {}", e))?;

    let spec = reader.spec();

    if spec.sample_rate != WHISPER_SAMPLE_RATE {
        return Err(format!(
            "Expected {}Hz sample rate, got {}",
            WHISPER_SAMPLE_RATE, spec.sample_rate
        ));
    }
    if spec.channels != 1 {
        return Err(format!(
            "Expected mono audio, got {} channels",
            spec.channels
        ));
    }

    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to decode WAV samples: {}", e))?;

    Ok(samples)
}

/// Check if any model file exists for the default backend.
pub fn check_model_exists() -> bool {
    WhisperBackend::new().model_exists()
}

/// Get the primary models directory (for downloads) from the default backend.
pub fn get_models_dir() -> Result<PathBuf, String> {
    WhisperBackend::new().models_dir()
}
