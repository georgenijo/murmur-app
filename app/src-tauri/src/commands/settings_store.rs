//! Durable source-state transports. Rust schemas and migrations live in
//! murmur-core; settings/theme publication shares its full-sync publisher.
//! A host writer lock orders these commands with WebKit lifecycle imports.

use crate::events::EventsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::Manager;

use crate::MutexExt;

#[derive(Clone, Copy)]
enum JsonShape {
    Object,
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
pub(crate) static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn blob_path(dir: &Path, spec: BlobSpec) -> PathBuf {
    dir.join(spec.file_name)
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
    );
    if valid_shape {
        Ok(())
    } else {
        let expected = match spec.shape {
            JsonShape::Object => "object",
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
    let _guard = WRITE_LOCK.lock_or_recover();
    write_blob_locked(dir, spec, blob)
}

/// Caller holds WRITE_LOCK, including any read/merge preceding this publish.
fn write_blob_locked(dir: &Path, spec: BlobSpec, blob: &str) -> Result<(), String> {
    validate_blob(spec, blob)?;
    let key = if spec.file_name == SETTINGS.file_name {
        "dictation-settings"
    } else {
        "murmur-theme-library"
    };
    murmur_core::completion::save_source(dir, key, blob)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
fn delete_blob(dir: &Path, spec: BlobSpec) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    match std::fs::remove_file(blob_path(dir, spec)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to clear {}: {e}", spec.label)),
    }
}

/// The per-bundle app data directory that owns every durable host-written
/// file. Shared so non-command hosts (the capture backend memo) publish beside
/// `settings.json` instead of resolving their own directory.
pub(crate) fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
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
    let dir = data_dir(&app)?;
    let _guard = WRITE_LOCK.lock_or_recover();
    let blob = read_blob(&dir, SETTINGS)?;
    blob.map(|blob| {
        murmur_core::completion::save_source(&dir, "dictation-settings", &blob)
            .map_err(|e| e.to_string())
    })
    .transpose()
}

fn require_settings_owner(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("Full settings writes are owned by the main window".into())
    }
}

#[tauri::command]
pub fn save_settings_blob(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    blob: String,
) -> Result<(), String> {
    require_settings_owner(window.label())?;
    let value =
        murmur_core::state::settings(serde_json::from_str(&blob).map_err(|_| "Invalid settings")?)
            .map_err(|_| "Invalid settings")?;
    let blob = serde_json::to_string(&value).map_err(|_| "Invalid settings")?;
    // Conservatively revoke passive input resolution for every persisted
    // settings update, including device/Smart Auto edits that do not change HAL topology.
    crate::audio::prepare::configuration_changed();
    let dir = data_dir(&app)?;
    let (revision, blob) = {
        // Assign revisions in durable commit order across concurrent saves.
        // Native overrides can still supersede this intent during disk I/O.
        let _guard = WRITE_LOCK.lock_or_recover();
        let blob = merge_owned_fields(&dir, &blob)?;
        let revision = crate::dictation_control::accept_configuration(&app);
        write_blob_locked(&dir, SETTINGS, &blob)?;
        (revision, blob)
    };
    notify_settings_committed(&app, &blob);
    if let Ok(settings) = serde_json::from_str(&blob) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::dictation_control::configure_accepted(&handle, settings, revision);
        });
    }
    Ok(())
}

fn notify_settings_committed(app: &tauri::AppHandle, blob: &str) {
    // Hidden WKWebViews can observe settings-changed before their localStorage
    // cache updates. Notify only after the authoritative file was committed.
    let enabled = serde_json::from_str::<serde_json::Value>(blob)
        .ok()
        .and_then(|value| value.get("overlayLiveWords").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let _ = app.events().send_to(
        murmur_core::events::Consumer::DictationPreview,
        "live-words-consent-persisted",
        enabled,
    );
    let _ = app.events().send_to(
        murmur_core::events::Consumer::Overlay,
        "overlay-settings-persisted",
        (),
    );
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlaySettingField {
    AutoPaste,
    Disabled,
}

impl OverlaySettingField {
    fn key(self) -> &'static str {
        match self {
            Self::AutoPaste => "autoPaste",
            Self::Disabled => "disabled",
        }
    }
}

fn require_overlay_owner(label: &str) -> Result<(), String> {
    if label == "overlay" {
        Ok(())
    } else {
        Err("Quick settings are owned by the overlay window".into())
    }
}

fn patch_owned_setting(
    dir: &Path,
    field: OverlaySettingField,
    value: bool,
) -> Result<String, String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    let blob = read_blob(dir, SETTINGS)?.ok_or("No durable settings snapshot is available")?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&blob).map_err(|_| "Durable settings could not be decoded")?;
    let epoch = settings["ownedFieldsRevision"]
        .as_u64()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or("Settings revision exhausted")?;
    settings["ownedFieldsRevision"] = serde_json::json!(epoch);
    settings
        .as_object_mut()
        .ok_or("Durable settings must be an object")?
        .insert(field.key().into(), serde_json::Value::Bool(value));
    let committed =
        serde_json::to_string(&settings).map_err(|_| "Settings could not be encoded")?;
    write_blob_locked(dir, SETTINGS, &committed)?;
    Ok(committed)
}

/// Older full snapshots cannot undo an overlay commit accepted since their
/// capture. The epoch is durable and advances with each owned-field patch.
fn merge_owned_fields(dir: &Path, blob: &str) -> Result<String, String> {
    let mut incoming: serde_json::Value =
        serde_json::from_str(blob).map_err(|_| "Invalid settings")?;
    if let Some(current) = read_blob(dir, SETTINGS)? {
        let current: serde_json::Value =
            serde_json::from_str(&current).map_err(|_| "Invalid settings")?;
        let epoch = current["ownedFieldsRevision"].as_u64().unwrap_or(0);
        if incoming["ownedFieldsRevision"].as_u64().unwrap_or(0) < epoch {
            for field in ["autoPaste", "disabled", "ownedFieldsRevision"] {
                incoming[field] = current[field].clone();
            }
        }
    }
    serde_json::to_string(&incoming).map_err(|_| "Invalid settings".into())
}

/// Never accept a retained full snapshot from a quick control. Merge its one
/// owned field into the latest document under the same lock as full writers.
#[tauri::command]
pub fn patch_overlay_setting(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    field: OverlaySettingField,
    value: bool,
) -> Result<String, String> {
    require_overlay_owner(window.label())?;
    let committed = patch_owned_setting(&data_dir(&app)?, field, value)?;
    notify_settings_committed(&app, &committed);
    let _ = app.events().send_to(
        murmur_core::events::Consumer::Main,
        "overlay-owned-setting-committed",
        serde_json::json!({ "field": field.key(), "value": value, "ownedFieldsRevision": serde_json::from_str::<serde_json::Value>(&committed).ok().and_then(|v| v["ownedFieldsRevision"].as_u64()).unwrap_or(0) }),
    );
    Ok(committed)
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
    write_blob(
        &data_dir(&app)?,
        THEME_LIBRARY,
        r#"{"version":1,"revision":0,"themes":[]}"#,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn full_settings_writes_cannot_bypass_probe_consent_from_companions() {
        assert!(super::require_settings_owner("main").is_ok());
        for label in ["overlay", "dictation-preview", "diagnostics", ""] {
            assert!(super::require_settings_owner(label).is_err());
        }
    }
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
    fn owned_patches_preserve_revoked_consent_and_concurrent_fields() {
        let dir = temp_dir("owned_patch");
        write_blob(&dir, SETTINGS, r#"{"settingsVersion":7,"overlayLiveWords":false,"overlayProcessingStyle":"pulse","autoPaste":false,"disabled":false}"#).unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| patch_owned_setting(&dir, OverlaySettingField::AutoPaste, true).unwrap());
            scope.spawn(|| patch_owned_setting(&dir, OverlaySettingField::Disabled, true).unwrap());
        });
        let value: serde_json::Value =
            serde_json::from_str(&read_blob(&dir, SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(value["overlayLiveWords"], false);
        assert_eq!(value["autoPaste"], true);
        assert_eq!(value["disabled"], true);
        assert!(value.get("overlayProcessingStyle").is_none());
        assert!(require_overlay_owner("overlay").is_ok());
        for owner in ["main", "dictation-preview", "diagnostics", ""] {
            assert!(require_overlay_owner(owner).is_err());
        }
        for field in ["overlayLiveWords", "overlayProcessingStyle", "__proto__"] {
            assert!(
                serde_json::from_value::<OverlaySettingField>(serde_json::json!(field)).is_err()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trips_each_blob_with_its_required_shape() {
        let dir = temp_dir("round_trip");
        for (spec, blob) in [
            (SETTINGS, r#"{"model":"tiny.en","settingsVersion":1}"#),
            (THEME_LIBRARY, r#"{"version":1,"revision":1,"themes":[]}"#),
        ] {
            write_blob(&dir, spec, blob).unwrap();
            let key = if spec.file_name == SETTINGS.file_name {
                "dictation-settings"
            } else {
                "murmur-theme-library"
            };
            let expected =
                murmur_core::state::normalize(key, serde_json::from_str(blob).unwrap()).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&read_blob(&dir, spec).unwrap().unwrap())
                    .unwrap(),
                expected
            );
        }
        assert_eq!(entries(&dir), vec!["settings.json", "theme-library.json"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_none_for_missing_files() {
        let dir = temp_dir("missing");
        assert_eq!(read_blob(&dir, SETTINGS).unwrap(), None);
        assert_eq!(read_blob(&dir, THEME_LIBRARY).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_the_data_directory_on_first_write() {
        let dir = temp_dir("create_dir").join("nested");
        write_blob(
            &dir,
            THEME_LIBRARY,
            r#"{"version":1,"revision":0,"themes":[]}"#,
        )
        .unwrap();
        assert_eq!(
            read_blob(&dir, THEME_LIBRARY).unwrap().as_deref(),
            Some(r#"{"revision":0,"themes":[],"version":1}"#)
        );
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn overwrites_atomically_and_leaves_no_temp_file() {
        let dir = temp_dir("overwrite");
        write_blob(&dir, SETTINGS, r#"{"totalWords":1,"padding":"old"}"#).unwrap();
        write_blob(&dir, SETTINGS, r#"{"totalWords":2}"#).unwrap();
        assert_eq!(
            read_blob(&dir, SETTINGS).unwrap().as_deref(),
            Some(
                murmur_core::completion::save_source(
                    &dir,
                    "dictation-settings",
                    r#"{"totalWords":2}"#
                )
                .unwrap()
                .as_str()
            )
        );
        assert_eq!(entries(&dir), vec![SETTINGS.file_name.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn durable_files_are_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("permissions");
        write_blob(
            &dir,
            THEME_LIBRARY,
            r#"{"version":1,"revision":0,"themes":[]}"#,
        )
        .unwrap();
        let mode = std::fs::metadata(blob_path(&dir, THEME_LIBRARY))
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
        std::fs::write(blob_path(&dir, THEME_LIBRARY), "private transcript{{{").unwrap();
        assert_eq!(read_blob(&dir, THEME_LIBRARY).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(
            names[0].starts_with("theme-library.json.corrupt-"),
            "{names:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_invalid_utf8() {
        let dir = temp_dir("invalid_utf8");
        std::fs::write(blob_path(&dir, SETTINGS), [0xff, 0xfe]).unwrap();
        assert_eq!(read_blob(&dir, SETTINGS).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("settings.json.corrupt-"), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn enforces_each_container_shape() {
        let dir = temp_dir("shape");
        assert!(write_blob(&dir, SETTINGS, "[]").is_err());
        assert!(write_blob(&dir, THEME_LIBRARY, "[]").is_err());
        assert!(write_blob(&dir, SETTINGS, "[]").is_err());
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_an_oversized_file() {
        let dir = temp_dir("oversized");
        let padding = "a".repeat(SETTINGS.max_bytes);
        std::fs::write(
            blob_path(&dir, SETTINGS),
            format!(r#"{{"pad":"{padding}"}}"#),
        )
        .unwrap();
        assert_eq!(read_blob(&dir, SETTINGS).unwrap(), None);
        let names = entries(&dir);
        assert_eq!(names.len(), 1, "unexpected entries: {names:?}");
        assert!(names[0].starts_with("settings.json.corrupt-"), "{names:?}");
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
        write_blob(
            &dir,
            THEME_LIBRARY,
            r#"{"version":1,"revision":0,"themes":[]}"#,
        )
        .unwrap();
        assert!(write_blob(&dir, THEME_LIBRARY, "[]").is_err());
        assert_eq!(
            read_blob(&dir, THEME_LIBRARY).unwrap().as_deref(),
            Some(r#"{"revision":0,"themes":[],"version":1}"#)
        );
        assert_eq!(entries(&dir), vec![THEME_LIBRARY.file_name.to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_is_idempotent_and_affects_only_the_selected_blob() {
        let dir = temp_dir("clear");
        write_blob(
            &dir,
            THEME_LIBRARY,
            r#"{"version":1,"revision":0,"themes":[]}"#,
        )
        .unwrap();
        write_blob(&dir, SETTINGS, "{}").unwrap();
        delete_blob(&dir, THEME_LIBRARY).unwrap();
        delete_blob(&dir, THEME_LIBRARY).unwrap();
        assert_eq!(read_blob(&dir, THEME_LIBRARY).unwrap(), None);
        assert!(read_blob(&dir, SETTINGS).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_write_leaves_no_temp_file_behind() {
        let dir = temp_dir("write_failure");
        // The shared publisher uses exclusive random temp siblings. Force the
        // actual rename to fail while still exercising temp-file cleanup.
        std::fs::create_dir_all(blob_path(&dir, SETTINGS)).unwrap();
        assert!(write_blob(&dir, SETTINGS, "{}").is_err());
        assert!(!blob_path(&dir, SETTINGS).is_file());
        assert_eq!(entries(&dir).len(), 1, "unexpected leftovers");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn theme_library_access_is_strictly_scoped_to_main() {
        assert!(require_main_window(MAIN_WINDOW_LABEL).is_ok());
        for label in [
            "diagnostics",
            "overlay",
            "transform-review",
            "unrecognized-window",
            "",
        ] {
            assert!(
                require_main_window(label).is_err(),
                "unexpected access for {label:?}"
            );
        }
    }
}

#[cfg(test)]
mod lifecycle_ordering_tests {
    use super::*;
    #[test]
    fn delayed_or_retried_full_settings_cannot_undo_a_newer_overlay_disable() {
        let dir = tempfile::tempdir().unwrap();
        let original=serde_json::json!({"settingsVersion":7,"autoPaste":false,"disabled":false,"language":"es"}).to_string();
        write_blob(dir.path(), SETTINGS, &original).unwrap();
        patch_owned_setting(dir.path(), OverlaySettingField::Disabled, true).unwrap();
        patch_owned_setting(dir.path(), OverlaySettingField::AutoPaste, true).unwrap();
        let _guard = WRITE_LOCK.lock_or_recover();
        let committed = merge_owned_fields(dir.path(), &original).unwrap();
        write_blob_locked(dir.path(), SETTINGS, &committed).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&read_blob(dir.path(), SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(value["disabled"], true);
        assert_eq!(value["autoPaste"], true);
        assert_eq!(value["language"], "es");
        assert_eq!(value["ownedFieldsRevision"], 2);
    }
}
