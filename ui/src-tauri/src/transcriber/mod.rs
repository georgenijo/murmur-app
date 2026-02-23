mod whisper;

pub use whisper::WhisperBackend;

use hound::{SampleFormat, WavReader};
use std::io::Cursor;
use std::path::PathBuf;

/// Sample rate required by transcription models (16kHz).
pub const WHISPER_SAMPLE_RATE: u32 = 16000;

/// Abstraction over transcription engines (whisper, moonshine, etc.)
pub trait TranscriptionBackend: Send + Sync {
    /// Human-readable backend name (e.g., "whisper", "moonshine")
    #[allow(dead_code)]
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
    if spec.sample_format != SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(format!(
            "Expected 16-bit integer PCM, got {:?} with {} bits per sample",
            spec.sample_format, spec.bits_per_sample
        ));
    }

    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to decode WAV samples: {}", e))?;

    Ok(samples)
}
