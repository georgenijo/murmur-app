//! Private accuracy corpus for the internal benchmark flavor (issue #827).
//!
//! An accuracy corpus is a directory of 16 kHz WAV clips plus a manifest whose
//! entries use the same field names as the guided personal corpus
//! (`promptId`, `label`, `reference`, `fileName`, `sha256`, `selected`). It is
//! produced locally by `scripts/murmur_accuracy.py export` (opt-in, from the
//! user's own saved recordings and corrected history text) or by
//! `scripts/murmur_accuracy.py synth` (macOS `say` fixtures with known text).
//!
//! Unlike the guided corpus it has no fixed prompt count. The loader only runs
//! in builds compiled with the `internal-benchmark` feature, validates every
//! path and digest before reading audio, and never logs reference text.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) const ACCURACY_CORPUS_ID: &str = "accuracy-v1";
const ACCURACY_SCHEMA_VERSION: u32 = 1;
const ACCURACY_SOURCES: &[&str] = &["corrected-history-export", "synthetic-say"];
const MAX_RECORDINGS: usize = 500;
const MAX_ID_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 120;
const MAX_REFERENCE_BYTES: usize = 4_000;
const MAX_WAV_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccuracyManifest {
    schema_version: u32,
    corpus_id: String,
    contains_real_user_data: bool,
    source: String,
    sample_rate: u32,
    #[serde(default)]
    recordings: Vec<AccuracyRecording>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccuracyRecording {
    prompt_id: String,
    label: String,
    reference: String,
    file_name: String,
    sha256: String,
    #[serde(default = "default_selected")]
    selected: bool,
}

fn default_selected() -> bool {
    true
}

pub(crate) struct AccuracyFixture {
    pub id: String,
    pub label: String,
    pub reference: String,
    pub wav: Vec<u8>,
}

/// The loaded corpus. `contains_real_user_data` selects the content-free
/// provenance label recorded in the benchmark report.
pub(crate) struct AccuracyCorpus {
    pub contains_real_user_data: bool,
    pub fixtures: Vec<AccuracyFixture>,
}

fn corpus_root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("MURMUR_ACCURACY_CORPUS_DIR") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return Ok(root);
        }
        return Err("MURMUR_ACCURACY_CORPUS_DIR must be an absolute path".to_string());
    }
    dirs::data_dir()
        .map(|root| root.join("Murmur Accuracy Corpus").join("v1"))
        .ok_or_else(|| "Could not locate the local Application Support directory".to_string())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn single_component(file_name: &str) -> bool {
    let relative = Path::new(file_name);
    relative.components().count() == 1
        && relative.file_name().and_then(|name| name.to_str()) == Some(file_name)
        && file_name.ends_with(".wav")
}

/// Load the selected recordings from the accuracy corpus, in manifest order.
pub(crate) fn load() -> Result<AccuracyCorpus, String> {
    load_from(&corpus_root()?)
}

fn load_from(root: &Path) -> Result<AccuracyCorpus, String> {
    let bytes = std::fs::read(root.join("manifest.json"))
        .map_err(|_| "The accuracy corpus manifest could not be read".to_string())?;
    let manifest: AccuracyManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The accuracy corpus manifest is invalid: {error}"))?;
    if manifest.schema_version != ACCURACY_SCHEMA_VERSION
        || manifest.corpus_id != ACCURACY_CORPUS_ID
    {
        return Err("The accuracy corpus manifest uses an unsupported format".to_string());
    }
    if !ACCURACY_SOURCES.contains(&manifest.source.as_str())
        || manifest.contains_real_user_data != (manifest.source == "corrected-history-export")
    {
        return Err("The accuracy corpus manifest has unexpected provenance".to_string());
    }
    if manifest.sample_rate != crate::state::WHISPER_SAMPLE_RATE {
        return Err("The accuracy corpus must contain 16 kHz recordings".to_string());
    }

    let selected = manifest
        .recordings
        .into_iter()
        .filter(|recording| recording.selected)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err("The accuracy corpus has no selected recordings".to_string());
    }
    if selected.len() > MAX_RECORDINGS {
        return Err(format!(
            "The accuracy corpus has more than {MAX_RECORDINGS} selected recordings"
        ));
    }
    let mut ids = HashSet::new();
    for recording in &selected {
        if !valid_id(&recording.prompt_id) {
            return Err(
                "Accuracy corpus IDs must contain only lowercase letters, numbers, and hyphens"
                    .to_string(),
            );
        }
        if !ids.insert(recording.prompt_id.as_str()) {
            return Err("The accuracy corpus contains duplicate selected IDs".to_string());
        }
        if recording.label.trim().is_empty() || recording.label.len() > MAX_LABEL_BYTES {
            return Err("An accuracy corpus label is invalid".to_string());
        }
        if recording.reference.trim().is_empty() || recording.reference.len() > MAX_REFERENCE_BYTES
        {
            return Err("An accuracy corpus reference is invalid".to_string());
        }
        if !single_component(&recording.file_name) {
            return Err("The accuracy corpus contains an invalid audio filename".to_string());
        }
    }

    let audio_dir = root.join("audio");
    let fixtures = selected
        .into_iter()
        .map(|recording| {
            let path = audio_dir.join(&recording.file_name);
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| "A selected accuracy corpus WAV is missing".to_string())?;
            if !metadata.is_file() || metadata.len() > MAX_WAV_BYTES {
                return Err("A selected accuracy corpus WAV is invalid or too large".to_string());
            }
            let wav = std::fs::read(&path)
                .map_err(|_| "A selected accuracy corpus WAV could not be read".to_string())?;
            let actual = format!("{:x}", Sha256::digest(&wav));
            if !actual.eq_ignore_ascii_case(&recording.sha256) {
                return Err(
                    "A selected accuracy corpus WAV failed integrity validation".to_string()
                );
            }
            Ok(AccuracyFixture {
                id: recording.prompt_id,
                label: recording.label,
                reference: recording.reference,
                wav,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(AccuracyCorpus {
        contains_real_user_data: manifest.contains_real_user_data,
        fixtures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_clip(root: &Path, file_name: &str, bytes: &[u8]) -> String {
        let audio = root.join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(audio.join(file_name), bytes).unwrap();
        format!("{:x}", Sha256::digest(bytes))
    }

    fn write_manifest(root: &Path, source: &str, real: bool, recordings: serde_json::Value) {
        let manifest = json!({
            "schemaVersion": 1,
            "corpusId": ACCURACY_CORPUS_ID,
            "containsRealUserData": real,
            "source": source,
            "sampleRate": 16_000,
            "recordings": recordings,
        });
        std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    }

    fn entry(id: &str, file_name: &str, sha256: &str) -> serde_json::Value {
        json!({
            "promptId": id,
            "label": "Synthetic",
            "reference": "Queue an update for Claude.",
            "fileName": file_name,
            "sha256": sha256,
            "terms": ["queue", "Claude"],
        })
    }

    #[test]
    fn loads_selected_recordings_in_manifest_order_and_ignores_extra_fields() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_clip(dir.path(), "b.wav", b"bbbb");
        let second = write_clip(dir.path(), "a.wav", b"aaaa");
        let mut skipped = entry("skipped", "a.wav", &second);
        skipped["selected"] = json!(false);
        write_manifest(
            dir.path(),
            "synthetic-say",
            false,
            json!([entry("syn-b", "b.wav", &first), skipped, entry("syn-a", "a.wav", &second)]),
        );
        let corpus = load_from(dir.path()).expect("valid corpus loads");
        assert!(!corpus.contains_real_user_data);
        let ids = corpus.fixtures.iter().map(|f| f.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, ["syn-b", "syn-a"]);
        assert_eq!(corpus.fixtures[0].wav, b"bbbb");
    }

    #[test]
    fn rejects_provenance_that_does_not_match_the_real_data_flag() {
        let dir = tempfile::tempdir().unwrap();
        let sha = write_clip(dir.path(), "a.wav", b"aaaa");
        write_manifest(dir.path(), "synthetic-say", true, json!([entry("a", "a.wav", &sha)]));
        assert!(load_from(dir.path()).is_err());
        write_manifest(
            dir.path(),
            "corrected-history-export",
            false,
            json!([entry("a", "a.wav", &sha)]),
        );
        assert!(load_from(dir.path()).is_err());
        write_manifest(dir.path(), "guided-local-recording", true, json!([entry("a", "a.wav", &sha)]));
        assert!(load_from(dir.path()).is_err());
    }

    #[test]
    fn rejects_unsafe_names_duplicates_digest_mismatch_and_empty_sets() {
        let dir = tempfile::tempdir().unwrap();
        let sha = write_clip(dir.path(), "a.wav", b"aaaa");
        for recordings in [
            json!([entry("a", "../a.wav", &sha)]),
            json!([entry("a", "a.txt", &sha)]),
            json!([entry("Upper", "a.wav", &sha)]),
            json!([entry("a", "a.wav", &sha), entry("a", "a.wav", &sha)]),
            json!([entry("a", "a.wav", &"0".repeat(64))]),
            json!([entry("a", "missing.wav", &sha)]),
            json!([]),
        ] {
            write_manifest(dir.path(), "synthetic-say", false, recordings);
            assert!(load_from(dir.path()).is_err());
        }
    }

    #[test]
    fn rejects_an_unsupported_corpus_id_or_sample_rate() {
        let dir = tempfile::tempdir().unwrap();
        let sha = write_clip(dir.path(), "a.wav", b"aaaa");
        let manifest = json!({
            "schemaVersion": 1,
            "corpusId": "personal-v1",
            "containsRealUserData": false,
            "source": "synthetic-say",
            "sampleRate": 16_000,
            "recordings": [entry("a", "a.wav", &sha)],
        });
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        assert!(load_from(dir.path()).is_err());
        let mut manifest = manifest;
        manifest["corpusId"] = json!(ACCURACY_CORPUS_ID);
        manifest["sampleRate"] = json!(44_100);
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        assert!(load_from(dir.path()).is_err());
    }
}
