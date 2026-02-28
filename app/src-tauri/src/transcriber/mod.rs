pub mod whisper;
pub mod moonshine;

pub use moonshine::MoonshineBackend;
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
    fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<String, String>;

    /// Check if any model file exists in search paths.
    fn model_exists(&self) -> bool;

    /// Get the directory where models are stored (for downloads).
    fn models_dir(&self) -> Result<PathBuf, String>;

    /// Reset loaded model so next transcription triggers a reload.
    fn reset(&mut self);
}

/// Returns true if the model name refers to a Moonshine backend.
pub fn is_moonshine_model(model_name: &str) -> bool {
    model_name.starts_with("moonshine-")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid 16kHz mono 16-bit PCM WAV in memory.
    fn make_test_wav(samples: &[i16]) -> Vec<u8> {
        let num_samples = samples.len() as u32;
        let data_size = num_samples * 2;
        let file_size = 36 + data_size;

        let mut buf = Vec::with_capacity(file_size as usize + 8);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // mono
        buf.extend_from_slice(&16000u32.to_le_bytes()); // sample rate
        buf.extend_from_slice(&32000u32.to_le_bytes()); // byte rate
        buf.extend_from_slice(&2u16.to_le_bytes()); // block align
        buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        for &s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        buf
    }

    #[test]
    fn parse_wav_silence() {
        let wav = make_test_wav(&[0i16; 160]);
        let samples = parse_wav_to_samples(&wav).unwrap();
        assert_eq!(samples.len(), 160);
        assert!(samples.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn parse_wav_normalization() {
        let wav = make_test_wav(&[i16::MAX, i16::MIN]);
        let samples = parse_wav_to_samples(&wav).unwrap();
        assert_eq!(samples.len(), 2);
        assert!((samples[0] - 1.0).abs() < 1e-4);
        assert!((samples[1] - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn parse_wav_rejects_wrong_sample_rate() {
        let mut wav = make_test_wav(&[0i16; 10]);
        wav[24..28].copy_from_slice(&44100u32.to_le_bytes());
        wav[28..32].copy_from_slice(&88200u32.to_le_bytes());
        let result = parse_wav_to_samples(&wav);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("16000"));
    }

    #[test]
    fn parse_wav_rejects_stereo() {
        let mut wav = make_test_wav(&[0i16; 10]);
        // Update channels, block_align, and byte_rate for a consistent stereo header
        wav[22..24].copy_from_slice(&2u16.to_le_bytes()); // channels = 2
        wav[28..32].copy_from_slice(&64000u32.to_le_bytes()); // byte_rate = 16000 * 2 * 2
        wav[32..34].copy_from_slice(&4u16.to_le_bytes()); // block_align = 2 * 2
        let result = parse_wav_to_samples(&wav);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mono"));
    }

    #[test]
    fn parse_wav_rejects_garbage() {
        let result = parse_wav_to_samples(b"not a wav file");
        assert!(result.is_err());
    }

    #[test]
    fn is_moonshine_model_classification() {
        assert!(is_moonshine_model("moonshine-tiny"));
        assert!(is_moonshine_model("moonshine-base"));
        assert!(!is_moonshine_model("base.en"));
        assert!(!is_moonshine_model("large-v3-turbo"));
    }
}
