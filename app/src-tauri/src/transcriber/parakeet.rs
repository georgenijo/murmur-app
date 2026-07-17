//! Parakeet (NVIDIA, via sherpa-onnx) transcription backend — self-contained spike.
//!
//! This module owns everything it needs (a variant registry mapping each
//! selectable model to its on-disk bundle + decoding method, the `parakeet`
//! classifier, and its own punctuation stripping) so it can be torn out cleanly.
//!
//! Variants are a small table in `variant_for` below: quantization (int8 / fp16)
//! × decoding (greedy / beam). To add a combo, add a row here and a matching
//! `MODEL_OPTIONS` entry in `app/src/lib/settings.ts`.
//!
//! REMOVING THIS BACKEND (complete teardown — nothing else depends on it):
//!   1. Delete this file (`transcriber/parakeet.rs`) and the model bundle dirs
//!      (`<models>/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-*/`).
//!   2. `transcriber/mod.rs`: delete the `pub mod parakeet;` and
//!      `pub use parakeet::ParakeetBackend;` lines.
//!   3. `commands/recording.rs::configure_dictation`: delete the Parakeet
//!      backend-swap branch (revert to a plain `backend.reset()`).
//!   4. `commands/models.rs::check_specific_model_exists`: delete the
//!      `parakeet::specific_model_exists` branch.
//!   5. `app/src/lib/settings.ts`: remove the `parakeet-*` entries from
//!      `MODEL_OPTIONS` / `ModelOption`, and drop `'parakeet'` from
//!      `TranscriptionBackend`.
//!   6. `Cargo.toml`: remove the `sherpa-onnx` dependency.

use super::TranscriptionBackend;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::{Path, PathBuf};

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

/// CPU inference threads for the recognizer.
const NUM_THREADS: i32 = 4;

/// Bundle directory name (sherpa-onnx release folder name) for the fp16 model.
const FP16_DIR: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16";

/// Model values exposed in the frontend dropdown (for "any model present" checks).
const KNOWN_MODELS: &[&str] = &["parakeet-tdt-0.6b-v2-fp16"];

/// A selectable Parakeet configuration: which bundle + which decoding method.
struct ParakeetVariant {
    dir: &'static str,
    encoder: &'static str,
    decoder: &'static str,
    joiner: &'static str,
    /// sherpa-onnx decoding method: "greedy_search" or "modified_beam_search".
    decoding_method: &'static str,
}

impl ParakeetVariant {
    /// True if every required file exists with non-zero size under `models_dir`.
    fn is_complete(&self, models_dir: &Path) -> bool {
        let dir = models_dir.join(self.dir);
        [self.encoder, self.decoder, self.joiner, "tokens.txt"]
            .iter()
            .all(|f| {
                let p = dir.join(f);
                p.is_file() && p.metadata().map_or(false, |m| m.len() > 0)
            })
    }
}

/// Map a dropdown model value to its bundle + decoding method.
/// Returns None for non-Parakeet (or unknown Parakeet) names.
/// fp16 (non-quantized) + greedy was the accuracy/speed sweet spot in testing
/// (int8 lost accuracy, beam was a no-op). Add rows here — plus matching
/// `MODEL_OPTIONS` entries in settings.ts — to expose more variants.
fn variant_for(model_name: &str) -> Option<ParakeetVariant> {
    match model_name {
        "parakeet-tdt-0.6b-v2-fp16" => Some(ParakeetVariant {
            dir: FP16_DIR,
            encoder: "encoder.fp16.onnx",
            decoder: "decoder.fp16.onnx",
            joiner: "joiner.fp16.onnx",
            decoding_method: "greedy_search",
        }),
        _ => None,
    }
}

fn app_models_dir(data_dir: &Path) -> PathBuf {
    APP_MODELS_REL
        .iter()
        .fold(data_dir.to_path_buf(), |p, s| p.join(s))
}

fn data_models_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| app_models_dir(&d))
}

/// Returns true if `model_name` should be served by the Parakeet backend.
/// This is the dispatch sentinel used by `configure_dictation`.
pub fn is_parakeet_model(model_name: &str) -> bool {
    model_name.starts_with("parakeet")
}

/// Returns true if the specific Parakeet variant's bundle is present on disk.
/// Used by `check_specific_model_exists` so the UI knows per-variant availability.
pub fn specific_model_exists(model_name: &str) -> bool {
    match (variant_for(model_name), data_models_dir()) {
        (Some(v), Some(dir)) => v.is_complete(&dir),
        _ => false,
    }
}

/// Check a model bundle under an explicit models root. Download installation
/// uses this to validate a staging directory before publishing it atomically.
pub(crate) fn specific_model_exists_in(model_name: &str, models_dir: &Path) -> bool {
    variant_for(model_name).is_some_and(|variant| variant.is_complete(models_dir))
}

/// Download info for a Parakeet model: `(tarball_url, extracted_dir_name)`.
/// The sherpa-onnx release ships each bundle as `<dir>.tar.bz2` which unpacks
/// to a top-level `<dir>/` folder. Returns None for unknown models.
pub fn download_spec(model_name: &str) -> Option<(String, String)> {
    let v = variant_for(model_name)?;
    let url = format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{}.tar.bz2",
        v.dir
    );
    Some((url, v.dir.to_string()))
}

pub struct ParakeetBackend {
    recognizer: Option<OfflineRecognizer>,
    loaded_model_name: Option<String>,
}

impl ParakeetBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for ParakeetBackend {
    fn default() -> Self {
        Self {
            recognizer: None,
            loaded_model_name: None,
        }
    }
}

impl TranscriptionBackend for ParakeetBackend {
    fn name(&self) -> &str {
        "parakeet"
    }

    fn load_model(&mut self, model_name: &str) -> Result<(), String> {
        if let Some(ref loaded) = self.loaded_model_name {
            if loaded == model_name {
                let rss = crate::resource_monitor::get_process_rss_mb();
                tracing::info!(target: "pipeline", rss_mb = rss, "parakeet_cache_hit");
                return Ok(());
            }
            self.reset();
        }

        let variant = variant_for(model_name)
            .ok_or_else(|| format!("Unknown Parakeet model '{}'", model_name))?;
        let models_dir = self.models_dir()?;
        let model_dir = models_dir.join(variant.dir);
        if !variant.is_complete(&models_dir) {
            return Err(format!(
                "Parakeet model bundle '{}' not found or incomplete at {}",
                variant.dir,
                model_dir.display()
            ));
        }

        let to_str = |p: PathBuf| -> Result<String, String> {
            p.to_str()
                .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())
                .map(|s| s.to_string())
        };

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(to_str(model_dir.join(variant.encoder))?),
            decoder: Some(to_str(model_dir.join(variant.decoder))?),
            joiner: Some(to_str(model_dir.join(variant.joiner))?),
        };
        config.model_config.tokens = Some(to_str(model_dir.join("tokens.txt"))?);
        config.model_config.model_type = Some("nemo_transducer".to_string());
        config.model_config.num_threads = NUM_THREADS;
        config.model_config.provider = Some("cpu".to_string());
        config.decoding_method = Some(variant.decoding_method.to_string());

        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
            "Failed to create Parakeet recognizer (sherpa-onnx returned null)".to_string()
        })?;

        self.recognizer = Some(recognizer);
        self.loaded_model_name = Some(model_name.to_string());
        let rss = crate::resource_monitor::get_process_rss_mb();
        tracing::info!(
            target: "pipeline",
            rss_mb = rss,
            bundle = variant.dir,
            decoding = variant.decoding_method,
            "parakeet_cache_miss"
        );
        Ok(())
    }

    fn is_model_loaded(&self, model_name: &str) -> bool {
        self.loaded_model_name.as_deref() == Some(model_name) && self.recognizer.is_some()
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        _language: &str,
        _initial_prompt: Option<&str>,
        smart_punctuation: bool,
    ) -> Result<String, String> {
        // Parakeet v2 is English-only and ignores prompts; language/initial_prompt unused.
        let recognizer = self
            .recognizer
            .as_ref()
            .ok_or_else(|| "Parakeet model not loaded. Call load_model() first.".to_string())?;

        let stream = recognizer.create_stream();
        stream.accept_waveform(super::WHISPER_SAMPLE_RATE as i32, samples);
        recognizer.decode(&stream);
        let text = stream.get_result().map(|r| r.text).unwrap_or_default();

        // Parakeet emits native casing/punctuation; strip when smart punctuation is off.
        let trimmed = text.trim().to_string();
        if smart_punctuation {
            Ok(trimmed)
        } else {
            Ok(strip_punctuation(&trimmed))
        }
    }

    fn token_count(&self, _text: &str) -> Option<usize> {
        // sherpa-onnx does not expose a tokenizer; stats fall back to an estimate.
        None
    }

    fn model_exists(&self) -> bool {
        let models_dir = match self.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        KNOWN_MODELS
            .iter()
            .any(|m| variant_for(m).map_or(false, |v| v.is_complete(&models_dir)))
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        data_models_dir().ok_or_else(|| "Could not find application data directory".to_string())
    }

    fn reset(&mut self) {
        tracing::info!(target: "pipeline", "parakeet: releasing recognizer");
        self.recognizer = None;
        self.loaded_model_name = None;
    }
}

/// Strip sentence and quotation punctuation while preserving contractions
/// (apostrophes between letters) and compound-word hyphens. Kept local so this
/// backend stays self-contained and removable.
fn strip_punctuation(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len());

    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\'' | '\u{2019}' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            '-' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\u{201C}' | '\u{201D}'
            | '\u{2018}' | '\u{2014}' | '\u{2013}' | '\u{2026}'
            | '\u{AB}' | '\u{BB}' | '\u{BF}' | '\u{A1}'
            | '\u{3002}' | '\u{3001}' | '\u{FF01}' | '\u{FF1F}'
            | '\u{30FB}' | '\u{300C}' | '\u{300D}' | '\u{300E}' | '\u{300F}' => result.push(' '),
            _ => result.push(c),
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_parakeet_model_classification() {
        assert!(is_parakeet_model("parakeet-tdt-0.6b-v2"));
        assert!(is_parakeet_model("parakeet-tdt-0.6b-v2-fp16-beam"));
        assert!(!is_parakeet_model("base.en"));
        assert!(!is_parakeet_model("large-v3-turbo"));
    }

    #[test]
    fn variant_registry_and_known_models_in_sync() {
        let v = variant_for("parakeet-tdt-0.6b-v2-fp16").unwrap();
        assert_eq!(v.dir, FP16_DIR);
        assert_eq!(v.encoder, "encoder.fp16.onnx");
        assert_eq!(v.decoding_method, "greedy_search");

        assert!(variant_for("base.en").is_none());
        assert!(variant_for("parakeet-tdt-0.6b-v2-int8").is_none()); // trimmed
        // KNOWN_MODELS and variant_for must stay in sync.
        assert!(KNOWN_MODELS.iter().all(|m| variant_for(m).is_some()));
    }

    #[test]
    fn download_spec_builds_url_for_fp16() {
        let (url, dir) = download_spec("parakeet-tdt-0.6b-v2-fp16").unwrap();
        assert!(url.ends_with("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16.tar.bz2"));
        assert_eq!(dir, FP16_DIR);
        assert!(download_spec("base.en").is_none());
    }

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
    fn strip_empty_string() {
        assert_eq!(strip_punctuation(""), "");
    }
}
