//! Durable, host-owned home for the frontend's settings blob.
//!
//! The frontend owns the entire settings schema and every migration rule, so
//! this module treats the payload as an opaque JSON object: it checks the
//! container (bounded, parses as an object) and never inspects a field. That
//! keeps a settings change from ever needing a Rust change, while moving the
//! durable copy out of WKWebView localStorage — which a reinstall or a WebKit
//! storage eviction can drop — into the per-bundle app data directory.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::Manager;

use crate::MutexExt;

const SETTINGS_FILE_NAME: &str = "settings.json";

/// Hard ceiling on one settings blob. The real object is a few KiB; the bound
/// exists so a tampered or runaway writer cannot park an unbounded file in the
/// app data directory, and so a load never reads an arbitrarily large file.
const MAX_SETTINGS_BYTES: usize = 1024 * 1024;

/// Serializes writers. The main and overlay windows can both persist settings,
/// and two concurrent publishes share one temp sibling — without this, one
/// writer's rename can consume the other's half-written temp file.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE_NAME)
}

/// Temp sibling used for the atomic publish. Kept in the settings directory so
/// the rename stays on one filesystem.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SETTINGS_FILE_NAME);
    path.with_file_name(format!(".{name}.murmur-tmp"))
}

/// The only shape check the host makes: a bounded JSON object. Refused on save
/// and treated as corruption on load.
fn validate_blob(blob: &str) -> Result<(), String> {
    if blob.len() > MAX_SETTINGS_BYTES {
        return Err(format!(
            "Settings blob is too large ({} bytes, limit {MAX_SETTINGS_BYTES})",
            blob.len()
        ));
    }
    match serde_json::from_str::<serde_json::Value>(blob) {
        Ok(serde_json::Value::Object(_)) => Ok(()),
        Ok(_) => Err("Settings blob must be a JSON object".to_string()),
        Err(e) => Err(format!("Settings blob is not valid JSON: {e}")),
    }
}

/// Rename a file we cannot trust aside instead of deleting it — settings a user
/// spent time on are unrecoverable, so corruption becomes evidence, never loss.
/// Best-effort: if the rename fails the load still falls back to localStorage.
fn quarantine(path: &Path, reason: &str) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(SETTINGS_FILE_NAME);
    let target = path.with_file_name(format!("{name}.corrupt-{seconds}"));
    match std::fs::rename(path, &target) {
        // Content-free: why it was rejected, never what it contained.
        Ok(()) => tracing::warn!(target: "system", reason, "settings file quarantined"),
        Err(e) => {
            tracing::warn!(target: "system", reason, "failed to quarantine settings file: {e}")
        }
    }
}

/// Read the stored blob from `dir`. `Ok(None)` means "no usable settings on
/// disk" — either the file has never been written or it was just quarantined —
/// so the caller falls back to its localStorage cache. `Err` is reserved for
/// filesystem failures, where retrying later may succeed.
pub(crate) fn read_settings_blob(dir: &Path) -> Result<Option<String>, String> {
    let path = settings_path(dir);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read settings: {e}")),
    };
    // Bound before reading, so an oversized file is never pulled into memory.
    if metadata.len() > MAX_SETTINGS_BYTES as u64 {
        quarantine(&path, "over the size ceiling");
        return Ok(None);
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read settings: {e}")),
    };
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(_) => {
            quarantine(&path, "not valid UTF-8");
            return Ok(None);
        }
    };
    if let Err(reason) = validate_blob(&contents) {
        quarantine(&path, &reason);
        return Ok(None);
    }
    Ok(Some(contents))
}

/// Validate, then publish `blob` into `dir` atomically.
pub(crate) fn write_settings_blob(dir: &Path, blob: &str) -> Result<(), String> {
    validate_blob(blob)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create settings directory: {e}"))?;

    let path = settings_path(dir);
    let temp = temp_path_for(&path);
    let _guard = WRITE_LOCK.lock_or_recover();
    if let Err(e) = std::fs::write(&temp, blob) {
        // A partial write (ENOSPC, permissions) must not leave a temp sibling
        // behind, and must never replace the last good settings file.
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to write settings: {e}"));
    }
    if let Err(e) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to publish settings: {e}"));
    }
    Ok(())
}

fn settings_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Settings directory unavailable: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create settings directory: {e}"))?;
    Ok(dir)
}

/// Load the durable settings blob, or `None` when there is nothing usable on
/// disk. The frontend re-runs its own validation over whatever comes back.
#[tauri::command]
pub fn load_settings_blob(app: tauri::AppHandle) -> Result<Option<String>, String> {
    read_settings_blob(&settings_dir(&app)?)
}

/// Persist the frontend's serialized settings object.
#[tauri::command]
pub fn save_settings_blob(app: tauri::AppHandle, blob: String) -> Result<(), String> {
    write_settings_blob(&settings_dir(&app)?, &blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur_settings_store_test_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn round_trips_a_settings_blob() {
        let dir = temp_dir("round_trip");
        let blob = r#"{"model":"tiny.en","settingsVersion":1}"#;
        write_settings_blob(&dir, blob).unwrap();
        assert_eq!(read_settings_blob(&dir).unwrap().as_deref(), Some(blob));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_no_settings_when_the_file_is_missing() {
        let dir = temp_dir("missing");
        assert_eq!(read_settings_blob(&dir).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_the_settings_directory_on_first_write() {
        let dir = temp_dir("create_dir").join("nested");
        write_settings_blob(&dir, "{}").unwrap();
        assert_eq!(read_settings_blob(&dir).unwrap().as_deref(), Some("{}"));
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn overwrites_atomically_and_leaves_no_temp_file() {
        let dir = temp_dir("overwrite");
        write_settings_blob(&dir, r#"{"autoPaste":false,"stale":"padding"}"#).unwrap();
        write_settings_blob(&dir, r#"{"autoPaste":true}"#).unwrap();
        assert_eq!(
            read_settings_blob(&dir).unwrap().as_deref(),
            Some(r#"{"autoPaste":true}"#)
        );
        assert_eq!(entries(&dir), vec![SETTINGS_FILE_NAME.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_a_file_that_is_not_json() {
        let dir = temp_dir("not_json");
        std::fs::write(settings_path(&dir), "not json{{{").unwrap();
        assert_eq!(read_settings_blob(&dir).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("settings.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_json_that_is_not_an_object() {
        let dir = temp_dir("not_object");
        std::fs::write(settings_path(&dir), "[1, 2, 3]").unwrap();
        assert_eq!(read_settings_blob(&dir).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("settings.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_an_oversized_file() {
        let dir = temp_dir("oversized");
        let padding = "a".repeat(MAX_SETTINGS_BYTES);
        std::fs::write(settings_path(&dir), format!(r#"{{"pad":"{padding}"}}"#)).unwrap();
        assert_eq!(read_settings_blob(&dir).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("settings.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_a_save_that_is_not_a_json_object() {
        let dir = temp_dir("reject_shape");
        for blob in ["[1,2,3]", "\"text\"", "17", "not json{{{"] {
            assert!(write_settings_blob(&dir, blob).is_err(), "{blob}");
        }
        assert!(entries(&dir).is_empty(), "nothing should be written");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_a_save_over_the_size_ceiling() {
        let dir = temp_dir("reject_size");
        let padding = "a".repeat(MAX_SETTINGS_BYTES);
        let error = write_settings_blob(&dir, &format!(r#"{{"pad":"{padding}"}}"#)).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        assert!(entries(&dir).is_empty(), "nothing should be written");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rejected_save_leaves_the_previous_settings_intact() {
        let dir = temp_dir("reject_keeps_previous");
        write_settings_blob(&dir, r#"{"autoPaste":true}"#).unwrap();
        assert!(write_settings_blob(&dir, "[]").is_err());
        assert_eq!(
            read_settings_blob(&dir).unwrap().as_deref(),
            Some(r#"{"autoPaste":true}"#)
        );
        assert_eq!(entries(&dir), vec![SETTINGS_FILE_NAME.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file_behind() {
        // The temp sibling path is occupied by a directory, so `fs::write`
        // itself fails rather than the rename.
        let dir = temp_dir("write_failure");
        std::fs::create_dir_all(temp_path_for(&settings_path(&dir))).unwrap();
        assert!(write_settings_blob(&dir, "{}").is_err());
        assert!(!settings_path(&dir).exists());
        assert_eq!(entries(&dir).len(), 1, "unexpected leftovers");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn temp_path_stays_in_the_settings_directory() {
        let path = settings_path(Path::new("/tmp/murmur"));
        let temp = temp_path_for(&path);
        assert_eq!(temp.parent(), path.parent());
        assert_ne!(temp.file_name(), path.file_name());
    }
}
