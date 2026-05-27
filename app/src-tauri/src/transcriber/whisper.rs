use super::TranscriptionBackend;
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    install_logging_hooks,
};

static INIT_LOGGING: Once = Once::new();

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere).
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        install_logging_hooks();
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
    state: Option<WhisperState>,
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
            state: None,
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
                let rss = crate::resource_monitor::get_process_rss_mb();
                tracing::info!(target: "pipeline", rss_mb = rss, "whisper_cache_hit");
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

        let gpu_backend = if cfg!(target_os = "macos") {
            "metal"
        } else if cfg!(target_os = "linux") && std::path::Path::new("/dev/nvidia0").exists() {
            "cuda"
        } else {
            "cpu"
        };
        tracing::info!(target: "pipeline", model = model_name, gpu = gpu_backend, "whisper_model_loading");

        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| format!("Failed to load whisper model: {}", e))?;

        let state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;
        self.context = Some(ctx);
        self.state = Some(state);
        self.loaded_model_name = Some(model_name.to_string());
        let rss = crate::resource_monitor::get_process_rss_mb();
        tracing::info!(target: "pipeline", rss_mb = rss, gpu = gpu_backend, "whisper_cache_miss");
        Ok(())
    }

    fn transcribe(&mut self, samples: &[f32], language: &str, initial_prompt: Option<&str>, smart_punctuation: bool) -> Result<String, String> {
        let state = self
            .state
            .as_mut()
            .ok_or_else(|| "Whisper state not initialized. Call load_model() first.".to_string())?;
        tracing::info!(target: "pipeline", "whisper: reusing cached state for transcription");

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let lang_opt: Option<&str> = if language == "auto" { None } else { Some(language) };
        params.set_language(lang_opt);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_single_segment(true);
        if let Some(prompt) = initial_prompt {
            params.set_initial_prompt(prompt);
        }
        params.set_debug_mode(false);

        state
            .full(params, samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state.full_n_segments();

        let mut text = String::new();
        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| format!("Failed to get segment {}", i))?;
            let segment_text = segment
                .to_str()
                .map_err(|e| format!("Failed to get text for segment {}: {}", i, e))?;
            text.push_str(segment_text);
        }

        let trimmed = text.trim().to_string();
        if smart_punctuation {
            Ok(trimmed)
        } else {
            Ok(strip_punctuation(&trimmed))
        }
    }

    fn token_count(&self, text: &str) -> Option<usize> {
        let ctx = self.context.as_ref()?;
        ctx.tokenize(text, 1024).ok().map(|tokens| tokens.len())
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
        tracing::info!(target: "pipeline", "whisper: releasing cached state and model");
        drop(self.state.take());
        drop(self.context.take());
        self.loaded_model_name = None;
    }
}

fn strip_punctuation(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len());

    for (i, &c) in chars.iter().enumerate() {
        match c {
            // Apostrophe: keep if between two alphanumeric chars (contractions)
            '\'' | '\u{2019}' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            // Hyphen: keep if between two alphanumeric chars (compound words)
            '-' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            // Strip sentence and quotation punctuation
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\u{201C}' | '\u{201D}'
            | '\u{2018}' | '\u{2014}' | '\u{2013}' | '\u{2026}'
            | '\u{AB}' | '\u{BB}' | '\u{BF}' | '\u{A1}'
            | '\u{3002}' | '\u{3001}' | '\u{FF01}' | '\u{FF1F}'
            | '\u{30FB}' | '\u{300C}' | '\u{300D}' | '\u{300E}' | '\u{300F}' => {}
            _ => result.push(c),
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::strip_punctuation;

    #[test]
    fn strip_basic_sentence_punctuation() {
        assert_eq!(strip_punctuation("Hello, world!"), "Hello world");
    }

    #[test]
    fn strip_preserves_apostrophe_in_contraction() {
        assert_eq!(strip_punctuation("Don't do that."), "Don't do that");
    }

    #[test]
    fn strip_preserves_hyphen_in_compound() {
        assert_eq!(strip_punctuation("It's state-of-the-art!"), "It's state-of-the-art");
    }

    #[test]
    fn strip_unicode_dashes_and_ellipsis() {
        assert_eq!(strip_punctuation("Hello\u{2026} world\u{2014}really?"), "Hello world really");
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(strip_punctuation(""), "");
    }

    #[test]
    fn strip_whitespace_only_with_punctuation() {
        assert_eq!(strip_punctuation("   .   "), "");
    }

    #[test]
    fn strip_preserves_french_contraction() {
        assert_eq!(strip_punctuation("c'est la vie!"), "c'est la vie");
    }
}
