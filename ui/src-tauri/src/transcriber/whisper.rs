use super::TranscriptionBackend;
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    install_whisper_log_trampoline,
};

static INIT_LOGGING: Once = Once::new();

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere).
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        install_whisper_log_trampoline();
    });
}

/// Build the app's primary models directory from a data dir root.
fn app_models_dir(data_dir: &Path) -> PathBuf {
    APP_MODELS_REL.iter().fold(data_dir.to_path_buf(), |p, s| p.join(s))
}

/// Get all potential model directories to search.
fn get_model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(custom_path) = std::env::var("WHISPER_MODEL_DIR") {
        paths.push(PathBuf::from(custom_path));
    }

    if let Some(data_dir) = dirs::data_dir() {
        paths.push(app_models_dir(&data_dir));
        paths.push(data_dir.join("pywhispercpp").join("models"));
    }

    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".cache").join("whisper.cpp"));
        paths.push(home.join(".cache").join("whisper"));
        paths.push(home.join(".whisper").join("models"));
    }

    paths
}

/// Get the path to a specific model file, searching multiple locations.
fn get_model_path(model_name: &str) -> Result<PathBuf, String> {
    let filename = format!("ggml-{}.bin", model_name);
    let search_paths = get_model_search_paths();

    for dir in &search_paths {
        let path = dir.join(&filename);
        if path.exists() {
            return Ok(path);
        }
    }

    let searched_locations = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(format!(
        "Model '{}' not found. Searched locations:\n{}\n\nDownload from: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename, searched_locations, filename
    ))
}

pub struct WhisperBackend {
    context: Option<WhisperContext>,
    loaded_model_name: Option<String>,
}

impl WhisperBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for WhisperBackend {
    fn default() -> Self {
        Self {
            context: None,
            loaded_model_name: None,
        }
    }
}

impl TranscriptionBackend for WhisperBackend {
    fn name(&self) -> &str {
        "whisper"
    }

    fn load_model(&mut self, model_name: &str) -> Result<(), String> {
        if let Some(ref loaded) = self.loaded_model_name {
            if loaded == model_name {
                return Ok(());
            }
            self.reset();
        }

        suppress_whisper_logs();

        let model_path = get_model_path(model_name)?;
        let path_str = model_path
            .to_str()
            .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())?;

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| format!("Failed to load whisper model: {}", e))?;

        self.context = Some(ctx);
        self.loaded_model_name = Some(model_name.to_string());
        Ok(())
    }

    fn transcribe(&self, samples: &[f32], language: &str) -> Result<String, String> {
        let ctx = self
            .context
            .as_ref()
            .ok_or_else(|| "Whisper model not loaded. Call load_model() first.".to_string())?;

        let mut state = ctx
            .create_state()
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

        state
            .full(params, samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state
            .full_n_segments()
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

    fn model_exists(&self) -> bool {
        let search_paths = get_model_search_paths();
        for dir in &search_paths {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().and_then(|e| e.to_str()) == Some("bin") {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| "Could not find application data directory".to_string())?;
        Ok(app_models_dir(&data_dir))
    }

    fn reset(&mut self) {
        self.context = None;
        self.loaded_model_name = None;
    }
}
