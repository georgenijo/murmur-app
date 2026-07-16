//! FluidAudio Core ML transcription backend for Apple Silicon Macs.
//!
//! FluidAudio owns its model download and compilation cache under
//! `~/Library/Application Support/FluidAudio/Models`. Murmur deliberately keeps
//! this the new-install default while keeping the existing Whisper and
//! sherpa-onnx paths selectable.

use super::TranscriptionBackend;
use fluidaudio_rs::FluidAudio;
use std::path::{Path, PathBuf};

pub use super::{is_coreml_model, COREML_MODEL_NAME};
const CACHE_DIR_NAME: &str = "parakeet-tdt-0.6b-v3";
const REQUIRED_MODELS: &[&str] = &[
    "Preprocessor.mlmodelc",
    "Encoder.mlmodelc",
    "Decoder.mlmodelc",
    "JointDecisionv3.mlmodelc",
];
const VOCAB_FILE: &str = "parakeet_vocab.json";

fn cache_root() -> Option<PathBuf> {
    dirs::data_dir().map(|path| path.join("FluidAudio").join("Models"))
}

fn model_dir() -> Option<PathBuf> {
    cache_root().map(|path| path.join(CACHE_DIR_NAME))
}

fn nonempty_file(path: &Path) -> bool {
    path.is_file() && path.metadata().map_or(false, |metadata| metadata.len() > 0)
}

fn model_exists_at(path: &Path) -> bool {
    REQUIRED_MODELS.iter().all(|model| {
        let compiled = path.join(model);
        compiled.is_dir()
            && nonempty_file(&compiled.join("coremldata.bin"))
            && nonempty_file(&compiled.join("weights").join("weight.bin"))
    }) && nonempty_file(&path.join(VOCAB_FILE))
}

fn cache_requires_repair(path: &Path) -> bool {
    path.exists() && !model_exists_at(path)
}

fn remove_incomplete_cache(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect incomplete Core ML cache: {error}"))?;
    let result = if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.map_err(|error| format!("Could not remove incomplete Core ML cache: {error}"))
}

pub fn specific_model_exists(model_name: &str) -> bool {
    is_coreml_model(model_name)
        && cfg!(target_arch = "aarch64")
        && model_dir().as_deref().map_or(false, model_exists_at)
}

fn new_engine() -> Result<FluidAudio, String> {
    let engine = FluidAudio::new().map_err(|error| format!("FluidAudio setup failed: {error}"))?;
    if !engine.is_apple_silicon() {
        return Err("FluidAudio Core ML transcription requires Apple Silicon".to_string());
    }
    Ok(engine)
}

/// Download, compile, and validate the FluidAudio model cache.
///
/// This is synchronous because the upstream Rust bridge exposes a synchronous
/// initializer. Callers must run it on a blocking worker.
pub fn prepare_model(model_name: &str) -> Result<(), String> {
    if !is_coreml_model(model_name) {
        return Err(format!("Unknown Core ML model '{model_name}'"));
    }

    let model_path =
        model_dir().ok_or_else(|| "Could not find FluidAudio model directory".to_string())?;
    if cache_requires_repair(&model_path) {
        tracing::warn!(target: "pipeline", "coreml_repairing_incomplete_cache");
        remove_incomplete_cache(&model_path)?;
    }

    let engine = new_engine()?;
    engine
        .init_asr()
        .map_err(|error| format!("Core ML model setup failed: {error}"))?;

    if !specific_model_exists(model_name) {
        return Err("Core ML setup completed but the model cache is incomplete".to_string());
    }
    Ok(())
}

pub struct CoreMlBackend {
    engine: Option<FluidAudio>,
    loaded_model_name: Option<String>,
}

impl CoreMlBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for CoreMlBackend {
    fn default() -> Self {
        Self {
            engine: None,
            loaded_model_name: None,
        }
    }
}

impl TranscriptionBackend for CoreMlBackend {
    fn name(&self) -> &str {
        "coreml"
    }

    fn load_model(&mut self, model_name: &str) -> Result<(), String> {
        if self.is_model_loaded(model_name) {
            tracing::info!(
                target: "pipeline",
                rss_mb = crate::resource_monitor::get_process_rss_mb(),
                "coreml_cache_hit"
            );
            return Ok(());
        }
        if !is_coreml_model(model_name) {
            return Err(format!("Unknown Core ML model '{model_name}'"));
        }
        if !specific_model_exists(model_name) {
            return Err(
                "Core ML model is not downloaded. Open Settings to download it.".to_string(),
            );
        }

        self.reset();
        let engine = new_engine()?;
        engine
            .init_asr()
            .map_err(|error| format!("Failed to load Core ML model: {error}"))?;
        if !engine.is_asr_available() {
            return Err("FluidAudio initialized without an available ASR model".to_string());
        }

        self.engine = Some(engine);
        self.loaded_model_name = Some(model_name.to_string());
        tracing::info!(
            target: "pipeline",
            rss_mb = crate::resource_monitor::get_process_rss_mb(),
            model = model_name,
            "coreml_cache_miss"
        );
        Ok(())
    }

    fn is_model_loaded(&self, model_name: &str) -> bool {
        self.loaded_model_name.as_deref() == Some(model_name)
            && self
                .engine
                .as_ref()
                .is_some_and(FluidAudio::is_asr_available)
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        _language: &str,
        _initial_prompt: Option<&str>,
        smart_punctuation: bool,
    ) -> Result<String, String> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| "Core ML model not loaded. Call load_model() first.".to_string())?;
        let result = engine
            .transcribe_samples(samples)
            .map_err(|error| format!("Core ML transcription failed: {error}"))?;
        let text = normalize_result_text(&result.text);

        tracing::info!(
            target: "pipeline",
            confidence = result.confidence as f64,
            model_processing_ms = (result.processing_time * 1000.0) as u64,
            "coreml_transcription_complete"
        );

        if smart_punctuation {
            Ok(text)
        } else {
            Ok(strip_punctuation(&text))
        }
    }

    fn token_count(&self, _text: &str) -> Option<usize> {
        None
    }

    fn model_exists(&self) -> bool {
        specific_model_exists(COREML_MODEL_NAME)
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        cache_root().ok_or_else(|| "Could not find FluidAudio model directory".to_string())
    }

    fn reset(&mut self) {
        if self.engine.is_some() {
            tracing::info!(target: "pipeline", "coreml: releasing FluidAudio engine");
        }
        self.engine = None;
        self.loaded_model_name = None;
    }
}

fn strip_punctuation(input: &str) -> String {
    input
        .chars()
        .map(|character| match character {
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\u{201c}' | '\u{201d}' | '\u{2014}'
            | '\u{2013}' | '\u{2026}' => ' ',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// FluidAudio can occasionally emit a standalone sentence-boundary token at
/// the beginning of an otherwise valid transcript. Remove only that exact
/// artifact so meaningful leading punctuation such as `.NET` and `...` stays
/// untouched.
fn normalize_result_text(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|character| character == '.') {
        return String::new();
    }

    let Some(after_period) = trimmed.strip_prefix('.') else {
        return trimmed.to_string();
    };

    if after_period.starts_with(char::is_whitespace) {
        let transcript = after_period.trim_start();
        if !transcript.is_empty() && !transcript.starts_with('.') {
            return transcript.to_string();
        }
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("murmur-coreml-test-{}-{nonce}", std::process::id()))
    }

    fn write_complete_model(path: &Path) {
        fs::create_dir_all(path).unwrap();
        for model in REQUIRED_MODELS {
            let compiled = path.join(model);
            fs::create_dir_all(compiled.join("weights")).unwrap();
            fs::write(compiled.join("coremldata.bin"), b"compiled").unwrap();
            fs::write(compiled.join("weights/weight.bin"), b"weights").unwrap();
        }
        fs::write(path.join(VOCAB_FILE), b"{}").unwrap();
    }

    #[test]
    fn classifies_only_the_explicit_coreml_model() {
        assert!(is_coreml_model(COREML_MODEL_NAME));
        assert!(!is_coreml_model("parakeet-tdt-0.6b-v2-fp16"));
        assert!(!is_coreml_model("base.en"));
    }

    #[test]
    fn removes_isolated_leading_period_artifact() {
        assert_eq!(normalize_result_text(". Hello there."), "Hello there.");
        assert_eq!(normalize_result_text("  .\nHello there.  "), "Hello there.");
        assert_eq!(normalize_result_text("."), "");
        assert_eq!(normalize_result_text(" ... "), "");
    }

    #[test]
    fn preserves_meaningful_leading_punctuation() {
        assert_eq!(normalize_result_text(".NET is fast."), ".NET is fast.");
        assert_eq!(normalize_result_text("...and then."), "...and then.");
        assert_eq!(normalize_result_text("Hello there."), "Hello there.");
    }

    #[test]
    fn complete_cache_requires_every_nonempty_component() {
        let path = test_dir();
        write_complete_model(&path);
        assert!(model_exists_at(&path));

        fs::write(path.join("Encoder.mlmodelc/coremldata.bin"), b"").unwrap();
        assert!(!model_exists_at(&path));

        fs::write(path.join("Encoder.mlmodelc/coremldata.bin"), b"compiled").unwrap();
        fs::write(path.join("Encoder.mlmodelc/weights/weight.bin"), b"").unwrap();
        assert!(!model_exists_at(&path));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn partial_cache_is_not_ready() {
        let path = test_dir();
        fs::create_dir_all(path.join("Preprocessor.mlmodelc")).unwrap();
        fs::write(
            path.join("Preprocessor.mlmodelc/coremldata.bin"),
            b"compiled",
        )
        .unwrap();
        assert!(!model_exists_at(&path));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn repair_is_requested_only_for_an_existing_incomplete_cache() {
        let path = test_dir();
        assert!(!cache_requires_repair(&path));

        fs::create_dir_all(path.join("Preprocessor.mlmodelc")).unwrap();
        assert!(cache_requires_repair(&path));

        write_complete_model(&path);
        assert!(!cache_requires_repair(&path));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn punctuation_setting_preserves_words() {
        assert_eq!(strip_punctuation("Hello, Core ML!"), "Hello Core ML");
    }
}
