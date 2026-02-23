use super::TranscriptionBackend;
use sherpa_rs::moonshine::{MoonshineConfig, MoonshineRecognizer};
use std::path::{Path, PathBuf};

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

fn app_models_dir(data_dir: &Path) -> PathBuf {
    APP_MODELS_REL
        .iter()
        .fold(data_dir.to_path_buf(), |p, s| p.join(s))
}

/// Map user-facing model name to the directory name on disk.
/// e.g. "moonshine-tiny" -> "sherpa-onnx-moonshine-tiny-en-int8"
pub fn model_dir_name(model_name: &str) -> String {
    let variant = model_name.strip_prefix("moonshine-").unwrap_or(model_name);
    format!("sherpa-onnx-moonshine-{}-en-int8", variant)
}

/// Archive filename for downloading.
pub fn archive_filename(model_name: &str) -> String {
    format!("{}.tar.bz2", model_dir_name(model_name))
}

/// Download URL for a moonshine model archive.
pub fn download_url(model_name: &str) -> String {
    format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{}",
        archive_filename(model_name)
    )
}

pub struct MoonshineBackend {
    recognizer: Option<MoonshineRecognizer>,
    loaded_model_name: Option<String>,
}

impl MoonshineBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for MoonshineBackend {
    fn default() -> Self {
        Self {
            recognizer: None,
            loaded_model_name: None,
        }
    }
}

impl TranscriptionBackend for MoonshineBackend {
    fn name(&self) -> &str {
        "moonshine"
    }

    fn load_model(&mut self, model_name: &str) -> Result<(), String> {
        if let Some(ref loaded) = self.loaded_model_name {
            if loaded == model_name {
                return Ok(());
            }
            self.reset();
        }

        let models_dir = self.models_dir()?;
        let dir_name = model_dir_name(model_name);
        let model_dir = models_dir.join(&dir_name);

        if !model_dir.exists() {
            return Err(format!(
                "Moonshine model directory '{}' not found at {}",
                dir_name,
                model_dir.display()
            ));
        }

        let to_str = |p: PathBuf| -> Result<String, String> {
            p.to_str()
                .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())
                .map(|s| s.to_string())
        };

        let config = MoonshineConfig {
            preprocessor: to_str(model_dir.join("preprocess.onnx"))?,
            encoder: to_str(model_dir.join("encode.int8.onnx"))?,
            uncached_decoder: to_str(model_dir.join("uncached_decode.int8.onnx"))?,
            cached_decoder: to_str(model_dir.join("cached_decode.int8.onnx"))?,
            tokens: to_str(model_dir.join("tokens.txt"))?,
            provider: Some("cpu".to_string()),
            num_threads: None,
            ..Default::default()
        };

        let recognizer = MoonshineRecognizer::new(config)
            .map_err(|e| format!("Failed to load Moonshine model: {}", e))?;

        self.recognizer = Some(recognizer);
        self.loaded_model_name = Some(model_name.to_string());
        Ok(())
    }

    fn transcribe(&mut self, samples: &[f32], _language: &str) -> Result<String, String> {
        let recognizer = self
            .recognizer
            .as_mut()
            .ok_or_else(|| "Moonshine model not loaded. Call load_model() first.".to_string())?;

        let result = recognizer.transcribe(16000, samples);
        Ok(result.text.trim().to_string())
    }

    fn model_exists(&self) -> bool {
        let models_dir = match self.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("sherpa-onnx-moonshine-") && entry.path().is_dir() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| "Could not find application data directory".to_string())?;
        Ok(app_models_dir(&data_dir))
    }

    fn reset(&mut self) {
        self.recognizer = None;
        self.loaded_model_name = None;
    }
}
