//! Integration tests for transcription backends.
//!
//! These tests require model files on disk and are automatically skipped
//! when models are not present. They are intended for local development,
//! not CI.
//!
//! Run: cd ui/src-tauri && cargo test --test transcription_integration -- --test-threads=1

use ui_lib::transcriber::{
    parse_wav_to_samples, MoonshineBackend, TranscriptionBackend, WhisperBackend,
};

/// Generate a 2-second 16kHz mono 16-bit PCM WAV containing a 440Hz sine tone.
fn make_sine_wav_bytes() -> Vec<u8> {
    let sample_rate = 16000u32;
    let duration_secs = 2.0f32;
    let freq = 440.0f32;
    let num_samples = (sample_rate as f32 * duration_secs) as usize;

    let samples: Vec<i16> = (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            (0.5 * (2.0 * std::f32::consts::PI * freq * t).sin() * i16::MAX as f32) as i16
        })
        .collect();

    let data_size = (num_samples * 2) as u32;
    let file_size = 36 + data_size;
    let mut buf = Vec::with_capacity(file_size as usize + 8);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in &samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

/// Find the first available whisper model name by scanning model directories.
fn find_whisper_model() -> Option<String> {
    let backend = WhisperBackend::new();
    let models_dir = backend.models_dir().ok()?;
    let entries = std::fs::read_dir(&models_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_str()?;
        if let Some(model) = name.strip_prefix("ggml-").and_then(|n| n.strip_suffix(".bin")) {
            return Some(model.to_string());
        }
    }
    None
}

#[test]
fn whisper_backend_roundtrip() {
    let mut backend = WhisperBackend::new();

    let model = match find_whisper_model() {
        Some(m) => m,
        None => {
            eprintln!("SKIPPED: no whisper model found on disk");
            return;
        }
    };

    backend
        .load_model(&model)
        .expect("failed to load whisper model");

    let wav_bytes = make_sine_wav_bytes();
    let samples = parse_wav_to_samples(&wav_bytes).expect("failed to parse WAV");

    let result = backend.transcribe(&samples, "en");
    assert!(result.is_ok(), "transcription failed: {:?}", result.err());
}

#[test]
fn moonshine_backend_roundtrip() {
    let mut backend = MoonshineBackend::new();

    if !backend.model_exists() {
        eprintln!("SKIPPED: no moonshine model found on disk");
        return;
    }

    backend
        .load_model("moonshine-tiny")
        .expect("failed to load moonshine model");

    let wav_bytes = make_sine_wav_bytes();
    let samples = parse_wav_to_samples(&wav_bytes).expect("failed to parse WAV");

    let result = backend.transcribe(&samples, "en");
    assert!(result.is_ok(), "transcription failed: {:?}", result.err());
}
