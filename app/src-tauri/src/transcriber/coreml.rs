//! FluidAudio Core ML transcription backend for Apple Silicon Macs.
//!
//! FluidAudio owns its model download and compilation cache under
//! `~/Library/Application Support/FluidAudio/Models`. Murmur deliberately keeps
//! this the new-install default while keeping the existing Whisper and
//! sherpa-onnx paths selectable.

use super::TranscriptionBackend;
use fluidaudio_rs::FluidAudio;
use std::io::Write;
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
const REPAIR_MARKER: &str = ".murmur-coreml-repair-pending-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelPreparationPhase {
    Repairing { repeated: bool },
    Initializing,
    Validating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelPreparationError {
    UnknownModel,
    CacheUnavailable,
    RepairStateUnavailable,
    RepairFailed,
    NativeInitializationFailed,
    ValidationFailed,
}

impl std::fmt::Display for ModelPreparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::UnknownModel => "Unknown Core ML model",
            Self::CacheUnavailable => "Could not locate the Core ML model cache",
            Self::RepairStateUnavailable => "Could not record Core ML repair state",
            Self::RepairFailed => "Could not safely repair the incomplete Core ML cache",
            Self::NativeInitializationFailed => "Core ML model initialization failed",
            Self::ValidationFailed => "Core ML setup completed with an incomplete cache",
        };
        formatter.write_str(message)
    }
}

fn cache_root() -> Option<PathBuf> {
    dirs::data_dir().map(|path| path.join("FluidAudio").join("Models"))
}

fn model_dir() -> Option<PathBuf> {
    cache_root().map(|path| path.join(CACHE_DIR_NAME))
}

fn nonempty_file(path: &Path) -> bool {
    path.is_file() && path.metadata().is_ok_and(|metadata| metadata.len() > 0)
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

fn marker_is_pending(path: &Path) -> Result<bool, ModelPreparationError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(ModelPreparationError::RepairStateUnavailable),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ModelPreparationError::RepairStateUnavailable),
    }
}

fn write_repair_marker(path: &Path) -> Result<(), ModelPreparationError> {
    let parent = path
        .parent()
        .ok_or(ModelPreparationError::RepairStateUnavailable)?;
    std::fs::create_dir_all(parent).map_err(|_| ModelPreparationError::RepairStateUnavailable)?;
    let temporary = parent.join(format!("{REPAIR_MARKER}.{}.tmp", std::process::id()));
    let write_result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| ModelPreparationError::RepairStateUnavailable)?;
        file.write_all(b"1\n")
            .map_err(|_| ModelPreparationError::RepairStateUnavailable)?;
        file.sync_all()
            .map_err(|_| ModelPreparationError::RepairStateUnavailable)?;
        std::fs::rename(&temporary, path).map_err(|_| ModelPreparationError::RepairStateUnavailable)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}

fn prepare_model_at<F, I>(
    model_name: &str,
    model_path: &Path,
    repair_marker: &Path,
    mut observe: F,
    initialize: I,
) -> Result<(), ModelPreparationError>
where
    F: FnMut(ModelPreparationPhase),
    I: FnOnce() -> Result<(), ModelPreparationError>,
{
    if !is_coreml_model(model_name) {
        return Err(ModelPreparationError::UnknownModel);
    }

    if cache_requires_repair(model_path) {
        let repeated = marker_is_pending(repair_marker)?;
        write_repair_marker(repair_marker)?;
        observe(ModelPreparationPhase::Repairing { repeated });
        remove_incomplete_cache(model_path).map_err(|_| ModelPreparationError::RepairFailed)?;
    }

    observe(ModelPreparationPhase::Initializing);
    initialize()?;
    observe(ModelPreparationPhase::Validating);
    if !model_exists_at(model_path) {
        return Err(ModelPreparationError::ValidationFailed);
    }

    let _ = std::fs::remove_file(repair_marker);
    Ok(())
}

pub fn specific_model_exists(model_name: &str) -> bool {
    is_coreml_model(model_name)
        && cfg!(target_arch = "aarch64")
        && model_dir().as_deref().is_some_and(model_exists_at)
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
    prepare_model_with_observer(model_name, |_| {}).map_err(|error| error.to_string())
}

pub(crate) fn prepare_model_with_observer<F>(
    model_name: &str,
    mut observe: F,
) -> Result<(), ModelPreparationError>
where
    F: FnMut(ModelPreparationPhase),
{
    let model_path = model_dir().ok_or(ModelPreparationError::CacheUnavailable)?;
    let repair_marker = model_path
        .parent()
        .ok_or(ModelPreparationError::CacheUnavailable)?
        .join(REPAIR_MARKER);
    prepare_model_at(
        model_name,
        &model_path,
        &repair_marker,
        |phase| {
            if matches!(phase, ModelPreparationPhase::Repairing { .. }) {
                tracing::warn!(target: "pipeline", "coreml_repairing_incomplete_cache");
            }
            observe(phase);
        },
        || {
            let engine =
                new_engine().map_err(|_| ModelPreparationError::NativeInitializationFailed)?;
            engine
                .init_asr()
                .map_err(|_| ModelPreparationError::NativeInitializationFailed)
        },
    )
}

#[derive(Default)]
pub struct CoreMlBackend {
    engine: Option<FluidAudio>,
    loaded_model_name: Option<String>,
}

impl CoreMlBackend {
    pub fn new() -> Self {
        Self::default()
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
        let output = if smart_punctuation {
            text.clone()
        } else {
            strip_punctuation(&text)
        };

        tracing::info!(
            target: "pipeline",
            confidence = result.confidence as f64,
            model_processing_ms = (result.processing_time * 1000.0) as u64,
            "coreml_transcription_complete"
        );

        if output.trim().is_empty() {
            let diagnostics = empty_output_diagnostics(samples.len(), &result.text, &text);
            tracing::warn!(
                target: "pipeline",
                input_sample_count = diagnostics.input_sample_count,
                raw_output_length = diagnostics.raw_output_length,
                normalized_length = diagnostics.normalized_length,
                period_only = diagnostics.period_only,
                "coreml_empty_output"
            );
        }

        Ok(output)
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

#[derive(Debug, PartialEq)]
struct EmptyOutputDiagnostics {
    input_sample_count: usize,
    raw_output_length: usize,
    normalized_length: usize,
    period_only: bool,
}

fn empty_output_diagnostics(
    input_sample_count: usize,
    raw_output: &str,
    normalized_output: &str,
) -> EmptyOutputDiagnostics {
    let raw_trimmed = raw_output.trim();
    EmptyOutputDiagnostics {
        input_sample_count,
        raw_output_length: raw_output.chars().count(),
        normalized_length: normalized_output.chars().count(),
        period_only: !raw_trimmed.is_empty()
            && raw_trimmed.chars().all(|character| character == '.'),
    }
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
    fn partial_cache_is_repaired_and_marker_is_cleared_after_success() {
        let root = test_dir();
        let model = root.join(CACHE_DIR_NAME);
        let marker = root.join(REPAIR_MARKER);
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join(VOCAB_FILE), b"partial").unwrap();
        let mut phases = Vec::new();

        prepare_model_at(
            COREML_MODEL_NAME,
            &model,
            &marker,
            |phase| phases.push(phase),
            || {
                write_complete_model(&model);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            phases,
            vec![
                ModelPreparationPhase::Repairing { repeated: false },
                ModelPreparationPhase::Initializing,
                ModelPreparationPhase::Validating,
            ]
        );
        assert!(model_exists_at(&model));
        assert!(!marker.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_incomplete_cache_repair_is_detected_across_attempts() {
        let root = test_dir();
        let model = root.join(CACHE_DIR_NAME);
        let marker = root.join(REPAIR_MARKER);
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join(VOCAB_FILE), b"partial").unwrap();
        fs::write(&marker, b"1\n").unwrap();
        let mut phases = Vec::new();

        let result = prepare_model_at(
            COREML_MODEL_NAME,
            &model,
            &marker,
            |phase| phases.push(phase),
            || Err(ModelPreparationError::NativeInitializationFailed),
        );

        assert_eq!(
            result,
            Err(ModelPreparationError::NativeInitializationFailed)
        );
        assert_eq!(
            phases,
            vec![
                ModelPreparationPhase::Repairing { repeated: true },
                ModelPreparationPhase::Initializing,
            ]
        );
        assert!(marker.is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initializer_success_without_complete_artifacts_fails_validation() {
        let root = test_dir();
        let model = root.join(CACHE_DIR_NAME);
        let marker = root.join(REPAIR_MARKER);
        let mut phases = Vec::new();

        let result = prepare_model_at(
            COREML_MODEL_NAME,
            &model,
            &marker,
            |phase| phases.push(phase),
            || Ok(()),
        );

        assert_eq!(result, Err(ModelPreparationError::ValidationFailed));
        assert_eq!(
            phases,
            vec![
                ModelPreparationPhase::Initializing,
                ModelPreparationPhase::Validating,
            ]
        );
        assert!(!model.exists());
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
    fn empty_output_diagnostics_are_privacy_safe_and_complete() {
        let diagnostics = empty_output_diagnostics(12_345, " ... ", "");
        assert_eq!(
            diagnostics,
            EmptyOutputDiagnostics {
                input_sample_count: 12_345,
                raw_output_length: 5,
                normalized_length: 0,
                period_only: true,
            }
        );

        let diagnostics = empty_output_diagnostics(80, "", "");
        assert_eq!(diagnostics.raw_output_length, 0);
        assert!(!diagnostics.period_only);
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
