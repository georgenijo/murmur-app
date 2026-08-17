//! Durable, host-owned home for frontend persistence blobs.
//!
//! The frontend owns the settings, history, statistics, and theme-library schemas and their
//! migration rules. This module validates only each bounded JSON container,
//! then publishes it atomically in the per-bundle app data directory. The
//! durable files survive WKWebView storage eviction and a manual reinstall;
//! localStorage remains a synchronous frontend cache.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::Manager;

use crate::MutexExt;

#[derive(Clone, Copy)]
enum JsonShape {
    Object,
    Array,
}

#[derive(Clone, Copy)]
struct BlobSpec {
    label: &'static str,
    file_name: &'static str,
    max_bytes: usize,
    shape: JsonShape,
}

const SETTINGS: BlobSpec = BlobSpec {
    label: "Settings",
    file_name: "settings.json",
    max_bytes: 1024 * 1024,
    shape: JsonShape::Object,
};

// History is capped at 200 entries in the frontend, but imported-file
// transcripts can be substantially larger than the settings object.
const HISTORY: BlobSpec = BlobSpec {
    label: "History",
    file_name: "history.json",
    max_bytes: 8 * 1024 * 1024,
    shape: JsonShape::Array,
};

const STATS: BlobSpec = BlobSpec {
    label: "Statistics",
    file_name: "stats.json",
    max_bytes: 1024 * 1024,
    shape: JsonShape::Object,
};

const THEME_LIBRARY: BlobSpec = BlobSpec {
    label: "Theme library",
    file_name: "theme-library.json",
    max_bytes: 1024 * 1024,
    shape: JsonShape::Object,
};

const MAIN_WINDOW_LABEL: &str = "main";

fn require_main_window(label: &str) -> Result<(), String> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("Theme library access is only available from the main window.".to_string())
    }
}

/// Serializes writers across every window and blob. Each file has its own temp
/// sibling, while one lock keeps publish/delete ordering deterministic.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn blob_path(dir: &Path, spec: BlobSpec) -> PathBuf {
    dir.join(spec.file_name)
}

/// Temp sibling used for atomic publish. It stays in the destination directory
/// so the rename cannot cross filesystems.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.json");
    path.with_file_name(format!(".{name}.murmur-tmp"))
}

fn validate_blob(spec: BlobSpec, blob: &str) -> Result<(), String> {
    if blob.len() > spec.max_bytes {
        return Err(format!(
            "{} blob is too large ({} bytes, limit {})",
            spec.label,
            blob.len(),
            spec.max_bytes
        ));
    }
    let value = serde_json::from_str::<serde_json::Value>(blob)
        .map_err(|e| format!("{} blob is not valid JSON: {e}", spec.label))?;
    let valid_shape = matches!(
        (spec.shape, value),
        (JsonShape::Object, serde_json::Value::Object(_))
            | (JsonShape::Array, serde_json::Value::Array(_))
    );
    if valid_shape {
        Ok(())
    } else {
        let expected = match spec.shape {
            JsonShape::Object => "object",
            JsonShape::Array => "array",
        };
        Err(format!("{} blob must be a JSON {expected}", spec.label))
    }
}

/// Preserve rejected content as local evidence rather than deleting it. Log
/// only the file kind and rejection reason, never any user content.
fn quarantine(path: &Path, spec: BlobSpec, reason: &str) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let target = path.with_file_name(format!("{}.corrupt-{seconds}", spec.file_name));
    match std::fs::rename(path, &target) {
        Ok(()) => tracing::warn!(
            target: "system",
            store = spec.label,
            reason,
            "durable data file quarantined"
        ),
        Err(e) => tracing::warn!(
            target: "system",
            store = spec.label,
            reason,
            "failed to quarantine durable data file: {e}"
        ),
    }
}

/// `Ok(None)` means no usable durable copy exists, so the frontend may migrate
/// its localStorage cache. `Err` is reserved for filesystem failures.
fn read_blob(dir: &Path, spec: BlobSpec) -> Result<Option<String>, String> {
    let path = blob_path(dir, spec);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read {}: {e}", spec.label)),
    };
    if metadata.len() > spec.max_bytes as u64 {
        quarantine(&path, spec, "over the size ceiling");
        return Ok(None);
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read {}: {e}", spec.label)),
    };
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(_) => {
            quarantine(&path, spec, "not valid UTF-8");
            return Ok(None);
        }
    };
    if let Err(reason) = validate_blob(spec, &contents) {
        quarantine(&path, spec, &reason);
        return Ok(None);
    }
    Ok(Some(contents))
}

fn write_blob(dir: &Path, spec: BlobSpec, blob: &str) -> Result<(), String> {
    validate_blob(spec, blob)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create durable data directory: {e}"))?;

    let path = blob_path(dir, spec);
    let temp = temp_path_for(&path);
    let _guard = WRITE_LOCK.lock_or_recover();
    if let Err(e) = std::fs::write(&temp, blob) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to write {}: {e}", spec.label));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("Failed to protect {}: {e}", spec.label));
        }
    }
    if let Err(e) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to publish {}: {e}", spec.label));
    }
    Ok(())
}

fn delete_blob(dir: &Path, spec: BlobSpec) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    match std::fs::remove_file(blob_path(dir, spec)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to clear {}: {e}", spec.label)),
    }
}

fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Durable data directory unavailable: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create durable data directory: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn load_settings_blob(app: tauri::AppHandle) -> Result<Option<String>, String> {
    read_blob(&data_dir(&app)?, SETTINGS)
}

#[tauri::command]
pub fn save_settings_blob(app: tauri::AppHandle, blob: String) -> Result<(), String> {
    write_blob(&data_dir(&app)?, SETTINGS, &blob)
}

#[tauri::command]
pub fn load_history_blob(app: tauri::AppHandle) -> Result<Option<String>, String> {
    read_blob(&data_dir(&app)?, HISTORY)
}

#[tauri::command]
pub fn save_history_blob(app: tauri::AppHandle, blob: String) -> Result<(), String> {
    write_blob(&data_dir(&app)?, HISTORY, &blob)
}

#[tauri::command]
pub fn clear_history_blob(app: tauri::AppHandle) -> Result<(), String> {
    delete_blob(&data_dir(&app)?, HISTORY)
}

#[tauri::command]
pub fn load_stats_blob(app: tauri::AppHandle) -> Result<Option<String>, String> {
    read_blob(&data_dir(&app)?, STATS)
}

#[tauri::command]
pub fn save_stats_blob(app: tauri::AppHandle, blob: String) -> Result<(), String> {
    write_blob(&data_dir(&app)?, STATS, &blob)
}

#[tauri::command]
pub fn clear_stats_blob(app: tauri::AppHandle) -> Result<(), String> {
    delete_blob(&data_dir(&app)?, STATS)
}

#[tauri::command]
pub fn load_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    require_main_window(window.label())?;
    read_blob(&data_dir(&app)?, THEME_LIBRARY)
}

#[tauri::command]
pub fn save_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    blob: String,
) -> Result<(), String> {
    require_main_window(window.label())?;
    write_blob(&data_dir(&app)?, THEME_LIBRARY, &blob)
}

#[tauri::command]
pub fn clear_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), String> {
    require_main_window(window.label())?;
    delete_blob(&data_dir(&app)?, THEME_LIBRARY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur_durable_store_test_{}_{}",
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
    fn round_trips_each_blob_with_its_required_shape() {
        let dir = temp_dir("round_trip");
        for (spec, blob) in [
            (SETTINGS, r#"{"model":"tiny.en","settingsVersion":1}"#),
            (HISTORY, r#"[{"id":"one","text":"private transcript"}]"#),
            (STATS, r#"{"totalWords":42,"totalRecordings":2}"#),
            (THEME_LIBRARY, r#"{"version":1,"revision":1,"themes":[]}"#),
        ] {
            write_blob(&dir, spec, blob).unwrap();
            assert_eq!(read_blob(&dir, spec).unwrap().as_deref(), Some(blob));
        }
        assert_eq!(
            entries(&dir),
            vec![
                "history.json",
                "settings.json",
                "stats.json",
                "theme-library.json"
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_none_for_missing_files() {
        let dir = temp_dir("missing");
        assert_eq!(read_blob(&dir, SETTINGS).unwrap(), None);
        assert_eq!(read_blob(&dir, HISTORY).unwrap(), None);
        assert_eq!(read_blob(&dir, STATS).unwrap(), None);
        assert_eq!(read_blob(&dir, THEME_LIBRARY).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_the_data_directory_on_first_write() {
        let dir = temp_dir("create_dir").join("nested");
        write_blob(&dir, HISTORY, "[]").unwrap();
        assert_eq!(read_blob(&dir, HISTORY).unwrap().as_deref(), Some("[]"));
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn overwrites_atomically_and_leaves_no_temp_file() {
        let dir = temp_dir("overwrite");
        write_blob(&dir, STATS, r#"{"totalWords":1,"padding":"old"}"#).unwrap();
        write_blob(&dir, STATS, r#"{"totalWords":2}"#).unwrap();
        assert_eq!(
            read_blob(&dir, STATS).unwrap().as_deref(),
            Some(r#"{"totalWords":2}"#)
        );
        assert_eq!(entries(&dir), vec![STATS.file_name.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn durable_files_are_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("permissions");
        write_blob(&dir, HISTORY, r#"[{"id":"private"}]"#).unwrap();
        let mode = std::fs::metadata(blob_path(&dir, HISTORY))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_invalid_json_without_logging_content() {
        let dir = temp_dir("not_json");
        std::fs::write(blob_path(&dir, HISTORY), "private transcript{{{").unwrap();
        assert_eq!(read_blob(&dir, HISTORY).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("history.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_invalid_utf8() {
        let dir = temp_dir("invalid_utf8");
        std::fs::write(blob_path(&dir, STATS), [0xff, 0xfe]).unwrap();
        assert_eq!(read_blob(&dir, STATS).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("stats.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn enforces_each_container_shape() {
        let dir = temp_dir("shape");
        assert!(write_blob(&dir, SETTINGS, "[]").is_err());
        assert!(write_blob(&dir, HISTORY, "{}").is_err());
        assert!(write_blob(&dir, STATS, "[]").is_err());
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_an_oversized_file() {
        let dir = temp_dir("oversized");
        let padding = "a".repeat(STATS.max_bytes);
        std::fs::write(blob_path(&dir, STATS), format!(r#"{{"pad":"{padding}"}}"#)).unwrap();
        assert_eq!(read_blob(&dir, STATS).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("stats.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_an_oversized_save_without_touching_disk() {
        let dir = temp_dir("reject_oversized");
        let padding = "a".repeat(SETTINGS.max_bytes);
        let error = write_blob(&dir, SETTINGS, &format!(r#"{{"pad":"{padding}"}}"#)).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejected_save_leaves_the_previous_blob_intact() {
        let dir = temp_dir("reject_keeps_previous");
        write_blob(&dir, HISTORY, r#"[{"id":"kept"}]"#).unwrap();
        assert!(write_blob(&dir, HISTORY, "{}").is_err());
        assert_eq!(
            read_blob(&dir, HISTORY).unwrap().as_deref(),
            Some(r#"[{"id":"kept"}]"#)
        );
        assert_eq!(entries(&dir), vec![HISTORY.file_name.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_is_idempotent_and_affects_only_the_selected_blob() {
        let dir = temp_dir("clear");
        write_blob(&dir, HISTORY, "[]").unwrap();
        write_blob(&dir, STATS, "{}").unwrap();
        delete_blob(&dir, HISTORY).unwrap();
        delete_blob(&dir, HISTORY).unwrap();
        assert_eq!(read_blob(&dir, HISTORY).unwrap(), None);
        assert_eq!(read_blob(&dir, STATS).unwrap().as_deref(), Some("{}"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_write_leaves_no_temp_file_behind() {
        let dir = temp_dir("write_failure");
        std::fs::create_dir_all(temp_path_for(&blob_path(&dir, SETTINGS))).unwrap();
        assert!(write_blob(&dir, SETTINGS, "{}").is_err());
        assert!(!blob_path(&dir, SETTINGS).exists());
        assert_eq!(entries(&dir).len(), 1, "unexpected leftovers");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn temp_path_stays_in_the_data_directory() {
        let path = blob_path(Path::new("/tmp/murmur"), HISTORY);
        let temp = temp_path_for(&path);
        assert_eq!(temp.parent(), path.parent());
        assert_ne!(temp.file_name(), path.file_name());
    }

    #[test]
    fn theme_library_access_is_strictly_scoped_to_main() {
        assert!(require_main_window(MAIN_WINDOW_LABEL).is_ok());
        for label in [
            "diagnostics",
            "overlay",
            "transform-review",
            "query-review",
            "",
        ] {
            assert!(
                require_main_window(label).is_err(),
                "unexpected access for {label:?}"
            );
        }
    }
}
