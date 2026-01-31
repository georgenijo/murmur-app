use crate::state::WHISPER_SAMPLE_RATE;
use hound::WavReader;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Once;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, install_whisper_log_trampoline};

static INIT_LOGGING: Once = Once::new();

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere)
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        // This routes whisper.cpp logs through Rust's log crate
        // Since we don't have a logger configured, they get discarded
        install_whisper_log_trampoline();
    });
}

/// Get all potential model directories to search
fn get_model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Check environment variable first
    if let Ok(custom_path) = std::env::var("WHISPER_MODEL_DIR") {
        paths.push(PathBuf::from(custom_path));
    }

    // App's own data directory
    if let Some(data_dir) = dirs::data_dir() {
        paths.push(data_dir.join("local-dictation").join("models"));
        paths.push(data_dir.join("pywhispercpp").join("models"));
    }

    // Home directory locations
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".cache").join("whisper.cpp"));
        paths.push(home.join(".cache").join("whisper"));
        paths.push(home.join(".whisper").join("models"));
    }

    paths
}

/// Get the path to a specific model file, searching multiple locations
pub fn get_model_path(model_name: &str) -> Result<PathBuf, String> {
    let filename = format!("ggml-{}.bin", model_name);
    let search_paths = get_model_search_paths();

    for dir in &search_paths {
        let path = dir.join(&filename);
        if path.exists() {
            return Ok(path);
        }
    }

    // Model not found - provide helpful error message
    let searched_locations = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(format!(
        "Model '{}' not found. Searched locations:\n{}\n\nDownload from: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename,
        searched_locations,
        filename
    ))
}

/// Get the primary models directory (for downloads)
#[allow(dead_code)]
pub fn get_models_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Could not find application data directory".to_string())?;
    Ok(data_dir.join("local-dictation").join("models"))
}

/// Initialize a WhisperContext for the given model
pub fn init_whisper_context(model_name: &str) -> Result<WhisperContext, String> {
    // Suppress verbose whisper.cpp logging
    suppress_whisper_logs();

    let model_path = get_model_path(model_name)?;
    let path_str = model_path.to_str()
        .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())?;

    let params = WhisperContextParameters::default();
    WhisperContext::new_with_params(path_str, params)
        .map_err(|e| format!("Failed to load whisper model: {}", e))
}

/// Parse WAV audio bytes and convert to f32 samples for whisper
pub fn parse_wav_to_samples(wav_bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(wav_bytes);
    let reader = WavReader::new(cursor)
        .map_err(|e| format!("Failed to parse WAV: {}", e))?;

    let spec = reader.spec();

    // Whisper expects 16kHz mono audio
    if spec.sample_rate != WHISPER_SAMPLE_RATE {
        return Err(format!("Expected {}Hz sample rate, got {}", WHISPER_SAMPLE_RATE, spec.sample_rate));
    }
    if spec.channels != 1 {
        return Err(format!("Expected mono audio, got {} channels", spec.channels));
    }

    // Convert i16 samples to f32 (normalized to -1.0 to 1.0)
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to decode WAV samples: {}", e))?;

    Ok(samples)
}

/// Transcribe audio samples using the given WhisperContext
pub fn transcribe(ctx: &WhisperContext, samples: &[f32], language: &str) -> Result<String, String> {
    let mut state = ctx.create_state()
        .map_err(|e| format!("Failed to create whisper state: {}", e))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_single_segment(true);
    params.set_debug_mode(false);

    state.full(params, samples)
        .map_err(|e| format!("Transcription failed: {}", e))?;

    let num_segments = state.full_n_segments()
        .map_err(|e| format!("Failed to get segments: {}", e))?;

    let mut text = String::new();
    for i in 0..num_segments {
        let segment = state
            .full_get_segment_text(i)
            .map_err(|e| format!("Failed to get segment {}: {}", i, e))?;
        text.push_str(&segment);
    }

    Ok(text.trim().to_string())
}
