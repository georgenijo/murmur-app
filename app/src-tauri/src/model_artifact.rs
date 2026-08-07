//! Shared, cheap validation for downloaded binary model artifacts.
//!
//! Hugging Face proxies and captive portals can return a successful HTTP
//! response whose body is an HTML, JSON, or Git LFS error document. Treating
//! mere file existence as installation success defers that failure until model
//! load, where the resulting error is much less actionable.

use std::io::Read;
use std::path::Path;

pub const MIN_WHISPER_MODEL_BYTES: u64 = 10 * 1024 * 1024;
pub const MIN_VAD_MODEL_BYTES: u64 = 512 * 1024;

const PREFIX_BYTES: usize = 512;

pub fn validate_binary_model(path: &Path, minimum_bytes: u64, label: &str) -> Result<(), String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("Could not inspect {label}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{label} is not a regular file"));
    }
    if metadata.len() < minimum_bytes {
        return Err(format!(
            "{label} is incomplete ({} bytes; expected at least {minimum_bytes})",
            metadata.len()
        ));
    }

    let mut file =
        std::fs::File::open(path).map_err(|error| format!("Could not open {label}: {error}"))?;
    let mut prefix = [0_u8; PREFIX_BYTES];
    let read = file
        .read(&mut prefix)
        .map_err(|error| format!("Could not inspect {label}: {error}"))?;
    if response_document_prefix(&prefix[..read]) {
        return Err(format!(
            "{label} contains a web error document instead of model data"
        ));
    }
    Ok(())
}

pub fn binary_model_is_valid(path: &Path, minimum_bytes: u64) -> bool {
    validate_binary_model(path, minimum_bytes, "Model file").is_ok()
}

fn response_document_prefix(prefix: &[u8]) -> bool {
    let text = String::from_utf8_lossy(prefix);
    let normalized = text.trim_start_matches(|character: char| {
        character.is_ascii_whitespace() || character == '\u{feff}'
    });
    let lower = normalized.to_ascii_lowercase();
    lower.starts_with('<')
        || lower.starts_with('{')
        || lower.starts_with('[')
        || lower.starts_with("version https://git-lfs.github.com/spec/")
        || lower.starts_with("accessdenied")
        || lower.starts_with("error:")
}

#[cfg(test)]
mod tests {
    use super::response_document_prefix;

    #[test]
    fn rejects_common_successful_error_payloads() {
        assert!(response_document_prefix(
            b"<!doctype html><title>Proxy</title>"
        ));
        assert!(response_document_prefix(b"  {\"error\":\"unauthorized\"}"));
        assert!(response_document_prefix(
            b"version https://git-lfs.github.com/spec/v1\n"
        ));
        assert!(!response_document_prefix(b"lmgg\0\0\0\0binary model"));
    }
}
