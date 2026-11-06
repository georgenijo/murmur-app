//! Durable source-state transports. Rust schemas and migrations live in
//! murmur-core; settings/theme publication shares its full-sync publisher.
//! A host writer lock orders these commands with WebKit lifecycle imports.

use crate::events::EventsExt;
use std::collections::HashMap;
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
// Native owns accepted patches even when reading/merging the current snapshot
// fails before the durable publisher can retain an exact merged document.
fn pending_owned() -> &'static Mutex<HashMap<PathBuf, OwnedEdits>> {
    static PENDING: std::sync::OnceLock<Mutex<HashMap<PathBuf, OwnedEdits>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(Default::default)
}

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
    let key = if spec.file_name == SETTINGS.file_name {
        "dictation-settings"
    } else {
        "murmur-theme-library"
    };
    match murmur_core::completion::quarantine_source(path.parent().expect("source parent"), key) {
        Ok(()) => {
            tracing::warn!(target: "system", store = spec.label, reason, "durable data file quarantined")
        }
        Err(_) => tracing::warn!(
            target: "system",
            store = spec.label,
            "failed to quarantine durable data file"
        ),
    }
}

/// Only never-created sources return None. Missing required or quarantined
/// destinations are persistent failures; hydration cannot silently seed defaults.
fn read_blob(dir: &Path, spec: BlobSpec) -> Result<Option<String>, String> {
    let path = blob_path(dir, spec);
    let key = if spec.file_name == SETTINGS.file_name {
        "dictation-settings"
    } else {
        "murmur-theme-library"
    };
    let required = murmur_core::completion::source_required(dir, key).map_err(|e| e.to_string())?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return if required {
                Err("Expected source destination missing".into())
            } else {
                Ok(None)
            }
        }
        Err(e) => return Err(format!("Failed to read {}: {e}", spec.label)),
    };
    if metadata.len() > spec.max_bytes as u64 {
        quarantine(&path, spec, "over the size ceiling");
        return Err("Source destination quarantined".into());
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return if required {
                Err("Expected source destination missing".into())
            } else {
                Ok(None)
            }
        }
        Err(e) => return Err(format!("Failed to read {}: {e}", spec.label)),
    };
    let contents = match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(_) => {
            quarantine(&path, spec, "not valid UTF-8");
            return Err("Source destination quarantined".into());
        }
    };
    if let Err(reason) = validate_blob(spec, &contents) {
        quarantine(&path, spec, &reason);
        return Err("Source destination quarantined".into());
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
fn source_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Durable data directory unavailable: {e}"))
}
pub(crate) fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Durable data directory unavailable: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create durable data directory: {e}"))?;
    Ok(dir)
}

pub(crate) fn load_settings_blob_worker(app: tauri::AppHandle) -> Result<Option<String>, String> {
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

pub(crate) fn save_settings_blob_worker(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    blob: String,
    owned_edits: Option<OwnedEdits>,
    intent_generation: Option<u64>,
    source_session: String,
    snapshot_current: bool,
) -> Result<String, String> {
    require_settings_owner(window.label())?;
    let value =
        murmur_core::state::settings(serde_json::from_str(&blob).map_err(|_| "Invalid settings")?)
            .map_err(|_| "Invalid settings")?;
    let blob = serde_json::to_string(&value).map_err(|_| "Invalid settings")?;
    // Conservatively revoke passive input resolution for every persisted
    // settings update, including device/Smart Auto edits that do not change HAL topology.
    crate::audio::prepare::configuration_changed();
    let dir = source_dir(&app)?;
    let (revision, blob) = {
        // Assign revisions in durable commit order across concurrent saves.
        // Native overrides can still supersede this intent during disk I/O.
        let _guard = WRITE_LOCK.lock_or_recover();
        let (blob, revision) =
            commit_settings_intent(&dir, &blob, owned_edits.as_ref(), snapshot_current, || {
                crate::dictation_control::accept_configuration(&app)
            })?;
        (revision, blob)
    };
    notify_settings_committed(&app, &blob);
    if let Ok(settings) = serde_json::from_str(&blob) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::dictation_control::configure_accepted(&handle, settings, revision);
        });
    }
    if owned_edits
        .as_ref()
        .is_some_and(|e| e.auto_paste.is_some() || e.disabled.is_some())
    {
        let _ = app.events().send_to(
            murmur_core::events::Consumer::Main,
            "overlay-owned-setting-committed",
            {
                let mut notice = owned_fields_notification(
                    OverlaySettingField::Disabled,
                    serde_json::from_str::<serde_json::Value>(&blob).unwrap()["disabled"] == true,
                    &blob,
                );
                notice["intentGeneration"] = serde_json::json!(intent_generation);
                notice["intentSession"] = serde_json::json!(source_session);
                notice["acknowledgedFields"] = serde_json::json!(owned_edits
                    .as_ref()
                    .map(|edits| {
                        let mut fields = Vec::new();
                        if edits.auto_paste.is_some() {
                            fields.push("autoPaste");
                        }
                        if edits.disabled.is_some() {
                            fields.push("disabled");
                        }
                        fields
                    })
                    .unwrap_or_default());
                notice
            },
        );
    }
    Ok(blob)
}

/// Caller holds WRITE_LOCK. A superseded document contributes only its owned
/// patch; every other field comes from the authoritative durable snapshot.
fn commit_settings_intent<T>(
    dir: &Path,
    blob: &str,
    edits: Option<&OwnedEdits>,
    snapshot: bool,
    before_publish: impl FnOnce() -> T,
) -> Result<(String, T), String> {
    let edits = {
        let mut pending = pending_owned().lock_or_recover();
        let retained = pending.entry(dir.into()).or_default();
        if let Some(edits) = edits {
            if edits.auto_paste.is_some() {
                retained.auto_paste = edits.auto_paste;
            }
            if edits.disabled.is_some() {
                retained.disabled = edits.disabled;
            }
        }
        *retained
    };
    let base = if snapshot {
        blob.to_owned()
    } else {
        read_blob(dir, SETTINGS)?.ok_or("No durable settings snapshot is available")?
    };
    let committed = merge_owned_fields_with_edits(dir, &base, Some(&edits))?;
    let revision = before_publish();
    write_blob_locked(dir, SETTINGS, &committed)?;
    pending_owned().lock_or_recover().remove(dir);
    Ok((committed, revision))
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

#[cfg(test)]
fn patch_owned_setting(
    dir: &Path,
    field: OverlaySettingField,
    value: bool,
) -> Result<String, String> {
    patch_owned_setting_with(dir, field, value, || ()).map(|(blob, ())| blob)
}

fn patch_owned_setting_with<T>(
    dir: &Path,
    field: OverlaySettingField,
    value: bool,
    before_publish: impl FnOnce() -> T,
) -> Result<(String, T), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    let edits = match field {
        OverlaySettingField::AutoPaste => OwnedEdits {
            auto_paste: Some(value),
            disabled: None,
        },
        OverlaySettingField::Disabled => OwnedEdits {
            auto_paste: None,
            disabled: Some(value),
        },
    };
    commit_settings_intent(dir, "{}", Some(&edits), false, before_publish)
}

/// A tray action is a native producer, not a renderer snapshot. Keep admission,
/// publication and the runtime override in the same FIFO as renderer patches.
/// Acquire the permit synchronously in the tray callback, before spawning.
pub(crate) fn toggle_tray_disabled(app: tauri::AppHandle) -> Result<(), String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    tauri::async_runtime::spawn(async move {
        let result = crate::shutdown::source_call(move || {
            let _admission = admission;
            let dir = source_dir(&app)?;
            let (blob, disabled) = commit_tray_disabled(&dir, |disabled| {
                crate::dictation_control::set_disabled(&app, disabled)
            })?;
            notify_settings_committed(&app, &blob);
            let _ = app.events().send_to(
                murmur_core::events::Consumer::Main,
                "overlay-owned-setting-committed",
                owned_fields_notification(OverlaySettingField::Disabled, disabled, &blob),
            );
            Ok::<_, String>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            tracing::warn!(target: "keyboard", "tray disable publication failed; native source recovery retains accepted edits");
        }
    });
    Ok(())
}

fn commit_tray_disabled(
    dir: &Path,
    apply_runtime: impl FnOnce(bool) -> Result<(), String>,
) -> Result<(String, bool), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    // Resolve the toggle inside the FIFO, so two rapid tray clicks are two
    // transitions even when neither callback has yet updated runtime state.
    let current = read_blob(dir, SETTINGS)?.ok_or("No durable settings snapshot is available")?;
    let current: serde_json::Value =
        serde_json::from_str(&current).map_err(|_| "Invalid settings")?;
    let disabled = !pending_owned()
        .lock_or_recover()
        .get(dir)
        .and_then(|edits| edits.disabled)
        .unwrap_or_else(|| current["disabled"].as_bool().unwrap_or(true));
    let edits = OwnedEdits {
        disabled: Some(disabled),
        auto_paste: None,
    };
    let (blob, runtime) = commit_settings_intent(dir, "{}", Some(&edits), false, || {
        // Reserve the native override before publication, as other settings
        // commits do, fencing configurations already waiting to apply.
        apply_runtime(disabled)
    })?;
    runtime?;
    Ok((blob, disabled))
}

fn owned_fields_notification(
    field: OverlaySettingField,
    value: bool,
    committed: &str,
) -> serde_json::Value {
    let settings: serde_json::Value =
        serde_json::from_str(committed).expect("committed settings are valid JSON");
    serde_json::json!({ "field": field.key(), "value": value,
        "ownedFieldsRevision": settings["ownedFieldsRevision"],
        "autoPaste": settings["autoPaste"], "disabled": settings["disabled"] })
}

/// Older full snapshots cannot undo an overlay commit accepted since their
/// capture. The epoch is durable and advances with each owned-field patch.
#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedEdits {
    auto_paste: Option<bool>,
    disabled: Option<bool>,
}
#[cfg(test)]
fn merge_owned_fields(dir: &Path, blob: &str) -> Result<String, String> {
    merge_owned_fields_with_edits(dir, blob, None)
}
fn merge_owned_fields_with_edits(
    dir: &Path,
    blob: &str,
    edits: Option<&OwnedEdits>,
) -> Result<String, String> {
    let mut incoming: serde_json::Value =
        serde_json::from_str(blob).map_err(|_| "Invalid settings")?;
    if let Some(current) = read_blob(dir, SETTINGS)? {
        let current: serde_json::Value =
            serde_json::from_str(&current).map_err(|_| "Invalid settings")?;
        // Onboarding has its own deliberate source-patch owner. A stale or
        // unreadable cache in an incidental full snapshot cannot undo its gate.
        if current["launchOnboardingComplete"].is_boolean() {
            incoming["launchOnboardingComplete"] = current["launchOnboardingComplete"].clone();
        }
        let epoch = current["ownedFieldsRevision"].as_u64().unwrap_or(0);
        // Full documents never own these fields once native has a snapshot.
        // Only deliberate, independently ordered patches may change them.
        for field in ["autoPaste", "disabled"] {
            incoming[field] = current[field].clone();
        }
        incoming["ownedFieldsRevision"] = serde_json::json!(epoch);
    }
    if let Some(edits) = edits {
        if edits.auto_paste.is_some() || edits.disabled.is_some() {
            if let Some(v) = edits.auto_paste {
                incoming["autoPaste"] = serde_json::json!(v);
            }
            if let Some(v) = edits.disabled {
                incoming["disabled"] = serde_json::json!(v);
            }
            let epoch = incoming["ownedFieldsRevision"]
                .as_u64()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|v| *v <= 9_007_199_254_740_991)
                .ok_or("Settings revision exhausted")?;
            incoming["ownedFieldsRevision"] = serde_json::json!(epoch);
        }
    }
    serde_json::to_string(&incoming).map_err(|_| "Invalid settings".into())
}

/// Runtime synchronization is not a deliberate edit. Resolve owned fields
/// under the source writer lock before reserving/applying a configuration.
fn rebase_runtime_owned_fields(
    dir: &Path,
    mut settings: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if !settings.is_object() {
        return Err("Invalid control settings".into());
    }
    if let Some(blob) = read_blob(dir, SETTINGS)? {
        let current: serde_json::Value =
            serde_json::from_str(&blob).map_err(|_| "Invalid settings")?;
        for field in ["disabled", "autoPaste", "ownedFieldsRevision"] {
            if let Some(value) = current.get(field) {
                settings[field] = value.clone();
            }
        }
    }
    // Publication failure does not surrender an already accepted native patch
    // to an older, still-readable disk value. Shutdown owns its retry.
    if let Some(edits) = pending_owned().lock_or_recover().get(dir) {
        if let Some(disabled) = edits.disabled {
            settings["disabled"] = serde_json::json!(disabled);
        }
        if let Some(auto_paste) = edits.auto_paste {
            settings["autoPaste"] = serde_json::json!(auto_paste);
        }
    }
    Ok(settings)
}

pub(crate) fn sync_app_disabled_worker(
    app: tauri::AppHandle,
    disabled: bool,
) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    let settings = rebase_runtime_owned_fields(
        &source_dir(&app)?,
        serde_json::json!({"disabled": disabled}),
    )?;
    crate::dictation_control::set_disabled(&app, settings["disabled"].as_bool().unwrap_or(true))
}

pub(crate) fn configure_renderer_control_worker(
    app: tauri::AppHandle,
    settings: serde_json::Value,
) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock_or_recover();
    let settings = rebase_runtime_owned_fields(&source_dir(&app)?, settings)?;
    crate::dictation_control::configure_accepted(
        &app,
        settings,
        crate::dictation_control::accept_configuration(&app),
    )
    .map(|_| ())
}

/// Never accept a retained full snapshot from a quick control. Merge its one
/// owned field into the latest document under the same lock as full writers.
pub(crate) fn patch_overlay_setting_worker(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    field: OverlaySettingField,
    value: bool,
) -> Result<String, String> {
    require_overlay_owner(window.label())?;
    let (committed, runtime) =
        patch_owned_setting_with(&source_dir(&app)?, field, value, || match field {
            OverlaySettingField::Disabled => crate::dictation_control::set_disabled(&app, value),
            OverlaySettingField::AutoPaste => Ok(()),
        })?;
    runtime?;
    notify_settings_committed(&app, &committed);
    let _ = app.events().send_to(
        murmur_core::events::Consumer::Main,
        "overlay-owned-setting-committed",
        owned_fields_notification(field, value, &committed),
    );
    Ok(committed)
}

pub(crate) fn load_theme_library_blob_worker(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    require_main_window(window.label())?;
    let _guard = WRITE_LOCK.lock_or_recover();
    read_blob(&data_dir(&app)?, THEME_LIBRARY)
}

pub(crate) fn save_theme_library_blob_worker(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    blob: String,
) -> Result<(), String> {
    require_main_window(window.label())?;
    write_blob(&data_dir(&app)?, THEME_LIBRARY, &blob)
}

pub(crate) fn clear_theme_library_blob_worker(
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

/// Explicit restore from a user-selected backup; never synthesize defaults or
/// replace a readable destination. Quarantines remain untouched for recovery.
pub(crate) fn restore_source_backup_worker(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    key: String,
    blob: String,
) -> Result<(), String> {
    require_settings_owner(window.label())?;
    restore_source_backup_in(&data_dir(&app)?, &key, &blob)
}

fn restore_source_backup_in(dir: &Path, key: &str, blob: &str) -> Result<(), String> {
    let spec = match key {
        "dictation-settings" => SETTINGS,
        "murmur-theme-library" => THEME_LIBRARY,
        _ => return Err("Unsupported recovery destination".into()),
    };
    validate_blob(spec, blob)?;
    let value: serde_json::Value = serde_json::from_str(blob).map_err(|_| "Invalid backup")?;
    // Full exported source only: a partial object must not silently seed defaults.
    if !murmur_core::completion::complete_source_document(key, &value) {
        return Err("Select a complete current-version source backup".into());
    }
    let _guard = WRITE_LOCK.lock_or_recover();
    if read_blob(dir, spec).is_ok_and(|v| v.is_some()) {
        return Err("Destination is readable; restore is unnecessary".into());
    }
    if blob_path(dir, spec).exists() {
        return Err("Destination must be missing or quarantined before recovery".into());
    }
    murmur_core::completion::restore_source(dir, key, &value).map_err(|e| e.to_string())
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
        assert_eq!(
            entries(&dir),
            vec![
                "settings.json",
                "settings.json.identity",
                "settings.json.required",
                "settings.json.required-history",
                "theme-library.json",
                "theme-library.json.identity",
                "theme-library.json.required",
                "theme-library.json.required-history"
            ]
        );
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
        assert_eq!(
            entries(&dir),
            vec![
                SETTINGS.file_name.to_string(),
                "settings.json.identity".into(),
                "settings.json.required".into(),
                "settings.json.required-history".into()
            ]
        );
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
        assert!(read_blob(&dir, THEME_LIBRARY).is_err());
        let names = entries(&dir);
        assert_eq!(names.len(), 3, "unexpected entries: {names:?}");
        assert!(
            names[0].starts_with("theme-library.json.quarantine-"),
            "{names:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quarantines_invalid_utf8() {
        let dir = temp_dir("invalid_utf8");
        std::fs::write(blob_path(&dir, SETTINGS), [0xff, 0xfe]).unwrap();
        assert!(read_blob(&dir, SETTINGS).is_err());
        let names = entries(&dir);
        assert_eq!(names.len(), 3, "unexpected entries: {names:?}");
        assert!(
            names[0].starts_with("settings.json.quarantine-"),
            "{names:?}"
        );
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
        assert!(read_blob(&dir, SETTINGS).is_err());
        let names = entries(&dir);
        assert_eq!(names.len(), 3, "unexpected entries: {names:?}");
        assert!(
            names[0].starts_with("settings.json.quarantine-"),
            "{names:?}"
        );
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
        assert_eq!(
            entries(&dir),
            vec![
                THEME_LIBRARY.file_name.to_string(),
                "theme-library.json.identity".into(),
                "theme-library.json.required".into(),
                "theme-library.json.required-history".into()
            ]
        );
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
        assert!(read_blob(&dir, THEME_LIBRARY).is_err());
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
        assert_eq!(
            entries(&dir).len(),
            1,
            "future-schema read refuses a destination directory before publishing a receipt"
        );
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
    fn generation_fence_and_native_quick_control_preserve_both_edits() {
        use murmur_core::source_order::SourceWrite;
        let dir = tempfile::tempdir().unwrap();
        let original = serde_json::json!({"settingsVersion":7,"autoPaste":false,"disabled":false,"language":"en","ownedFieldsRevision":0}).to_string();
        write_blob(dir.path(), SETTINGS, &original).unwrap();
        let session = uuid::Uuid::new_v4().to_string();
        crate::shutdown::register_test_source("main", &session);
        let older_dir = dir.path().to_path_buf();
        let older = crate::shutdown::versioned_source_call(
            "main".into(),
            session.clone(),
            "dictation-settings".into(),
            1,
            move || write_blob(&older_dir, SETTINGS, &original),
        );
        patch_owned_setting(dir.path(), OverlaySettingField::AutoPaste, true).unwrap();
        let newer_dir = dir.path().to_path_buf();
        let newer = crate::shutdown::versioned_source_call(
            "main".into(),
            session,
            "dictation-settings".into(),
            2,
            move || {
                let _guard = WRITE_LOCK.lock_or_recover();
                let blob = merge_owned_fields_with_edits(
                    &newer_dir,
                    r#"{"settingsVersion":7,"autoPaste":false,"disabled":true,"language":"es","ownedFieldsRevision":0}"#,
                    Some(&OwnedEdits {
                        auto_paste: None,
                        disabled: Some(true),
                    }),
                )?;
                write_blob_locked(&newer_dir, SETTINGS, &blob)
            },
        );
        assert_eq!(
            tauri::async_runtime::block_on(newer).unwrap(),
            SourceWrite::Applied { value: () }
        );
        assert_eq!(
            tauri::async_runtime::block_on(older).unwrap(),
            SourceWrite::Superseded
        );
        let value: serde_json::Value =
            serde_json::from_str(&read_blob(dir.path(), SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(value["language"], "es");
        assert_eq!(value["autoPaste"], true);
        assert_eq!(value["disabled"], true);
    }

    #[test]
    fn delayed_or_retried_full_settings_cannot_undo_a_newer_overlay_disable() {
        let dir = tempfile::tempdir().unwrap();
        let original=serde_json::json!({"settingsVersion":7,"autoPaste":false,"disabled":false,"language":"es"}).to_string();
        write_blob(dir.path(), SETTINGS, &original).unwrap();
        patch_owned_setting(dir.path(), OverlaySettingField::Disabled, true).unwrap();
        patch_owned_setting(dir.path(), OverlaySettingField::AutoPaste, true).unwrap();
        let latest = read_blob(dir.path(), SETTINGS).unwrap().unwrap();
        let notification = owned_fields_notification(OverlaySettingField::AutoPaste, true, &latest);
        // Even if the earlier Disable event was missed, this next event carries
        // the complete pair represented by revision 2.
        assert_eq!(notification["disabled"], true);
        assert_eq!(notification["autoPaste"], true);
        assert_eq!(notification["ownedFieldsRevision"], 2);
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

#[cfg(test)]
mod explicit_owned_edit_tests {
    use super::*;
    #[test]
    fn missed_revision_event_does_not_undo_explicit_main_reenable() {
        let dir = tempfile::tempdir().unwrap();
        let stale=serde_json::json!({"settingsVersion":7,"disabled":false,"autoPaste":false,"ownedFieldsRevision":0}).to_string();
        write_blob(dir.path(), SETTINGS, &stale).unwrap();
        patch_owned_setting(dir.path(), OverlaySettingField::Disabled, true).unwrap();
        // Main saw app-disabled-changed but missed the revision event. Its
        // deliberate re-enable is still revision zero, and must patch rev one.
        let _guard = WRITE_LOCK.lock_or_recover();
        let committed = merge_owned_fields_with_edits(
            dir.path(),
            &stale,
            Some(&OwnedEdits {
                disabled: Some(false),
                auto_paste: None,
            }),
        )
        .unwrap();
        write_blob_locked(dir.path(), SETTINGS, &committed).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&read_blob(dir.path(), SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(value["disabled"], false);
        assert_eq!(value["ownedFieldsRevision"], 2);
    }
    #[test]
    fn explicit_backup_restore_requires_complete_supplied_data_and_preserves_quarantine() {
        for key in ["dictation-settings", "murmur-theme-library"] {
            let spec = if key == "dictation-settings" {
                SETTINGS
            } else {
                THEME_LIBRARY
            };
            for corrupt in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let value = if key == "dictation-settings" {
                    serde_json::json!({"language":"es","autoPasteDelayMs":23,"microphone":"system_default"})
                } else {
                    serde_json::json!({"version":1,"revision":3,"themes":[]})
                };
                let backup =
                    murmur_core::completion::save_source(dir.path(), key, &value.to_string())
                        .unwrap();
                assert!(restore_source_backup_in(dir.path(), key, &backup).is_err());
                if corrupt {
                    std::fs::write(blob_path(dir.path(), spec), b"{bad").unwrap();
                } else {
                    std::fs::remove_file(blob_path(dir.path(), spec)).unwrap();
                }
                std::fs::write(
                    dir.path().join(format!("{}.pending", spec.file_name)),
                    b"broken receipt",
                )
                .unwrap();
                assert!(read_blob(dir.path(), spec).is_err());
                assert!(restore_source_backup_in(dir.path(), key, "{}").is_err());
                restore_source_backup_in(dir.path(), key, &backup).unwrap();
                assert_eq!(read_blob(dir.path(), spec).unwrap().unwrap(), backup);
                if corrupt {
                    assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(&format!("{}.quarantine-", spec.file_name))));
                }
            }
        }
    }
    #[test]
    fn settings_merge_resumes_first_save_after_marker_before_destination_crash() {
        let dir = tempfile::tempdir().unwrap();
        let exact = murmur_core::state::settings(
            serde_json::json!({"language":"es","autoPasteDelayMs":23,"microphone":"system_default"}),
        )
        .unwrap();
        use sha2::{Digest, Sha256};
        // Stage the current receipt format against an absent destination.
        let exact: serde_json::Value = serde_json::from_str(
            &murmur_core::completion::save_source(
                dir.path(),
                "dictation-settings",
                &exact.to_string(),
            )
            .unwrap(),
        )
        .unwrap();
        let identity: String = serde_json::from_slice(
            &std::fs::read(dir.path().join("settings.json.identity")).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("settings.json")).unwrap();
        let marker: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("settings.json.required")).unwrap(),
        )
        .unwrap();
        let base_marker_digest =
            format!("{:x}", Sha256::digest(serde_json::to_vec(&marker).unwrap()));
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&exact).unwrap()));
        std::fs::write(dir.path().join("settings.json.pending"), serde_json::json!({
            "schema":2,"key":"dictation-settings","storeId":identity,"generation":uuid::Uuid::new_v4().to_string(),
            "baseDigest":null,"baseMarkerDigest":base_marker_digest,"targetDigest":digest,"targetRevision":0,"value":exact
        }).to_string()).unwrap();
        // Production read/merge on the next launch must replay before reading.
        let committed = merge_owned_fields(dir.path(), &exact.to_string()).unwrap();
        write_blob(dir.path(), SETTINGS, &committed).unwrap();
        let disk: serde_json::Value =
            serde_json::from_str(&read_blob(dir.path(), SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(disk["language"], "es");
        assert_eq!(disk["autoPasteDelayMs"], 23);
    }
    #[test]
    fn incidental_settings_snapshot_cannot_undo_onboarding_reset() {
        let dir = tempfile::tempdir().unwrap();
        write_blob(
            dir.path(),
            SETTINGS,
            r#"{"launchOnboardingComplete":false,"language":"ja"}"#,
        )
        .unwrap();
        let merged = merge_owned_fields(
            dir.path(),
            r#"{"launchOnboardingComplete":true,"language":"es"}"#,
        )
        .unwrap();
        write_blob(dir.path(), SETTINGS, &merged).unwrap();
        let disk: serde_json::Value =
            serde_json::from_str(&read_blob(dir.path(), SETTINGS).unwrap().unwrap()).unwrap();
        assert_eq!(disk["launchOnboardingComplete"], false);
        assert_eq!(disk["language"], "es");
    }
}

/// Worker-only source fence, serialized with accepted source publications and
/// quick-control merges. Recovery preserves their owned-field revision rules.
pub(crate) fn flush(app: &tauri::AppHandle) -> bool {
    let _guard = WRITE_LOCK.lock_or_recover();
    let Ok(root) = data_dir(app) else {
        return false;
    };
    if !flush_owned(&root) {
        return false;
    }
    ["dictation-settings", "murmur-theme-library"]
        .iter()
        .all(|key| murmur_core::completion::recover_source(&root, key).is_ok())
}

/// Caller holds WRITE_LOCK, after the accepted producer drain.
fn flush_owned(root: &Path) -> bool {
    let has_pending = pending_owned()
        .lock_or_recover()
        .get(root)
        .is_some_and(|e| e.auto_paste.is_some() || e.disabled.is_some());
    !has_pending || commit_settings_intent(root, "{}", None, false, || ()).is_ok()
}

#[tauri::command]
pub async fn load_settings_blob(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        load_settings_blob_worker(app)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedEditGenerations {
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_paste: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disabled: Option<u64>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsWrite {
    #[serde(flatten)]
    snapshot: murmur_core::source_order::SourceWrite<String>,
    owned_patch_generations: OwnedEditGenerations,
}

/// Runs inside the real source queue. Snapshot and field fences are independent;
/// accepted fields survive a superseded document. Cross-window fields resolve
/// by queue acceptance order, while retries retain their original field IDs.
fn select_settings_intent(
    order: &mut murmur_core::source_order::SourceOrder,
    owner: &str,
    session: &str,
    generation: u64,
    edits: OwnedEdits,
    fields: OwnedEditGenerations,
) -> Result<(bool, OwnedEdits, OwnedEditGenerations), String> {
    let valid = |g: u64| g > 0 && g <= generation && g <= 9_007_199_254_740_991;
    if !valid(generation) {
        return Err("Invalid settings generation".into());
    }
    for (value, field_generation) in [
        (edits.auto_paste, fields.auto_paste),
        (edits.disabled, fields.disabled),
    ] {
        if value.is_some() && !field_generation.is_some_and(valid) {
            return Err("Invalid owned field generation".into());
        }
    }
    let snapshot = order.take(owner, session, "dictation-settings", generation);
    let mut accepted = OwnedEdits::default();
    let mut acknowledgements = OwnedEditGenerations::default();
    for (key, value, field_generation, target, ack) in [
        (
            "settings:autoPaste",
            edits.auto_paste,
            fields.auto_paste,
            &mut accepted.auto_paste,
            &mut acknowledgements.auto_paste,
        ),
        (
            "settings:disabled",
            edits.disabled,
            fields.disabled,
            &mut accepted.disabled,
            &mut acknowledgements.disabled,
        ),
    ] {
        if let (Some(value), Some(g)) = (value, field_generation) {
            // Even a duplicate ID is owned by native already; its original
            // publication was committed or retained by the durable owner.
            if order.take(owner, session, key, g) {
                *target = Some(value);
            }
            *ack = Some(g);
        }
    }
    Ok((snapshot, accepted, acknowledgements))
}

#[tauri::command]
pub async fn save_settings_blob(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    blob: String,
    owned_edits: Option<OwnedEdits>,
    owned_edit_generations: Option<OwnedEditGenerations>,
    intent_generation: u64,
    source_session: String,
) -> Result<SettingsWrite, String> {
    require_settings_owner(window.label())?;
    let admission = crate::shutdown::WriterPermit::acquire()?;
    match crate::shutdown::ordered_source_call(
        window.label().into(),
        source_session.clone(),
        move |order, owner, session| {
            let _admission = admission;
            // Validate before reserving any field identity.
            murmur_core::state::settings(
                serde_json::from_str(&blob).map_err(|_| "Invalid settings")?,
            )
            .map_err(|_| "Invalid settings")?;
            let (snapshot, edits, acknowledged) = select_settings_intent(
                order,
                owner,
                session,
                intent_generation,
                owned_edits.unwrap_or_default(),
                owned_edit_generations.unwrap_or_default(),
            )?;
            let published = if snapshot || edits.auto_paste.is_some() || edits.disabled.is_some() {
                Some(save_settings_blob_worker(
                    app,
                    window,
                    blob,
                    Some(edits),
                    Some(intent_generation),
                    source_session,
                    snapshot,
                )?)
            } else {
                None
            };
            Ok(SettingsWrite {
                snapshot: if snapshot {
                    murmur_core::source_order::SourceWrite::Applied {
                        value: published.unwrap(),
                    }
                } else {
                    murmur_core::source_order::SourceWrite::Superseded
                },
                owned_patch_generations: acknowledged,
            })
        },
    )
    .await?
    {
        murmur_core::source_order::SourceWrite::Applied { value } => Ok(value),
        murmur_core::source_order::SourceWrite::Superseded => Ok(SettingsWrite {
            snapshot: murmur_core::source_order::SourceWrite::Superseded,
            owned_patch_generations: OwnedEditGenerations::default(),
        }),
    }
}

#[tauri::command]
pub async fn patch_overlay_setting(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    field: OverlaySettingField,
    value: bool,
    source_session: String,
) -> Result<String, String> {
    require_overlay_owner(window.label())?;
    let admission = crate::shutdown::WriterPermit::acquire()?;
    match crate::shutdown::ordered_source_call(
        window.label().into(),
        source_session,
        move |_, _, _| {
            let _admission = admission;
            patch_overlay_setting_worker(app, window, field, value)
        },
    )
    .await?
    {
        murmur_core::source_order::SourceWrite::Applied { value } => Ok(value),
        murmur_core::source_order::SourceWrite::Superseded => Err("Source session retired".into()),
    }
}

#[tauri::command]
pub async fn load_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        load_theme_library_blob_worker(window, app)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[tauri::command]
pub async fn save_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    blob: String,
    intent_generation: u64,
    source_session: String,
) -> Result<murmur_core::source_order::SourceWrite<()>, String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::versioned_source_call(
        window.label().into(),
        source_session,
        "murmur-theme-library".into(),
        intent_generation,
        move || {
            let _admission = admission;
            save_theme_library_blob_worker(window, app, blob)
        },
    )
    .await
}

#[tauri::command]
pub async fn clear_theme_library_blob(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        clear_theme_library_blob_worker(window, app)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[tauri::command]
pub async fn restore_source_backup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    key: String,
    blob: String,
) -> Result<(), String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        restore_source_backup_worker(app, window, key, blob)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[cfg(test)]
mod round_three_ordering_tests {
    use super::*;
    use murmur_core::source_order::SourceWrite;

    // Same selection, merge and full-sync publisher used by the IPC worker,
    // through the real native source FIFO, without creating an app or webview.
    async fn save(
        root: PathBuf,
        session: String,
        generation: u64,
        language: &str,
        edits: OwnedEdits,
        fields: OwnedEditGenerations,
    ) -> SourceWrite<String> {
        let blob = serde_json::json!({"language":language,"autoPaste":false,"disabled":false,"ownedFieldsRevision":0}).to_string();
        let admission = crate::shutdown::WriterPermit::accepted_producer();
        match crate::shutdown::ordered_source_call(
            "main".into(),
            session,
            move |order, owner, session| {
                let _admission = admission;
                let (snapshot, edits, _) =
                    select_settings_intent(order, owner, session, generation, edits, fields)?;
                let _guard = WRITE_LOCK.lock_or_recover();
                let (blob, ()) =
                    commit_settings_intent(&root, &blob, Some(&edits), snapshot, || ())?;
                Ok(if snapshot {
                    SourceWrite::Applied { value: blob }
                } else {
                    SourceWrite::Superseded
                })
            },
        )
        .await
        .unwrap()
        {
            SourceWrite::Applied { value } => value,
            SourceWrite::Superseded => SourceWrite::Superseded,
        }
    }
    async fn quick(
        root: PathBuf,
        session: String,
        field: OverlaySettingField,
        value: bool,
    ) -> SourceWrite<String> {
        let admission = crate::shutdown::WriterPermit::accepted_producer();
        crate::shutdown::ordered_source_call("overlay".into(), session, move |_, _, _| {
            let _admission = admission;
            patch_owned_setting(&root, field, value)
        })
        .await
        .unwrap()
    }
    fn setup() -> (tempfile::TempDir, String, String) {
        let root = tempfile::tempdir().unwrap();
        write_blob(
            root.path(),
            SETTINGS,
            r#"{"autoPaste":false,"disabled":false,"language":"en"}"#,
        )
        .unwrap();
        let main = uuid::Uuid::new_v4().to_string();
        let overlay = uuid::Uuid::new_v4().to_string();
        crate::shutdown::register_test_source("main", &main);
        crate::shutdown::register_test_source("overlay", &overlay);
        (root, main, overlay)
    }
    fn recovered(root: &Path) -> serde_json::Value {
        // The real source barrier and completion owner's shutdown receipt. No
        // renderer flush runs. Reopen/recover exactly as the next launch does.
        let path = root.to_path_buf();
        tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
            let _guard = WRITE_LOCK.lock_or_recover();
            assert!(flush_owned(&path));
            murmur_core::completion::recover_source(&path, "dictation-settings").unwrap();
        }))
        .unwrap();
        let service =
            murmur_core::completion::CompletionService::start(root.into(), false).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(service.shutdown_receipt()),
            murmur_core::shutdown::WriteOutcome::Committed
        );
        murmur_core::completion::recover_source(root, "dictation-settings").unwrap();
        serde_json::from_str(&read_blob(root, SETTINGS).unwrap().unwrap()).unwrap()
    }
    #[test]
    fn tray_disable_survives_shutdown_without_renderer_echo() {
        let (root, _, _) = setup();
        let path = root.path().to_path_buf();
        tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
            let (_, disabled) = commit_tray_disabled(&path, |value| {
                assert!(value);
                Ok(())
            })
            .unwrap();
            assert!(disabled);
        }))
        .unwrap();
        let disk = recovered(root.path());
        assert_eq!(disk["disabled"], true);
        assert_eq!(disk["autoPaste"], false);
        assert_eq!(disk["language"], "en");
        assert_eq!(disk["ownedFieldsRevision"], 1);
    }

    #[test]
    fn tray_disable_survives_renderer_echo_stale_snapshot_and_owned_patch_retry() {
        let (root, main, _) = setup();
        // A real main-owned enable accepted before the native tray disable.
        tauri::async_runtime::block_on(save(
            root.path().into(),
            main.clone(),
            1,
            "en",
            OwnedEdits {
                disabled: Some(false),
                auto_paste: None,
            },
            OwnedEditGenerations {
                disabled: Some(1),
                auto_paste: None,
            },
        ));
        let path = root.path().to_path_buf();
        tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
            commit_tray_disabled(&path, |_| Ok(())).unwrap();
            // Both runtime adapters use this rebase under WRITE_LOCK before
            // accepting a configuration. Even a later-arriving false snapshot
            // cannot re-enable the native tray-owned disable.
            let _guard = WRITE_LOCK.lock_or_recover();
            let rebased = rebase_runtime_owned_fields(
                &path,
                serde_json::json!({
                    "disabled":false, "autoPaste":true, "ownedFieldsRevision":0, "language":"es",
                }),
            )
            .unwrap();
            assert_eq!(rebased["disabled"], true);
            assert_eq!(rebased["autoPaste"], false);
            assert_eq!(rebased["ownedFieldsRevision"], 2);
            assert_eq!(rebased["language"], "es");
        }))
        .unwrap();
        // The old hook echoed disabled=true with no owned patch. Exercise the
        // actual merge, not a notification mock, then send disabled=false in a
        // stale document with a fresh snapshot generation and the old field ID.
        let path = root.path().to_path_buf();
        tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
            let _guard = WRITE_LOCK.lock_or_recover();
            commit_settings_intent(
                &path,
                r#"{"disabled":true,"autoPaste":false,"language":"en"}"#,
                None,
                true,
                || (),
            )
            .unwrap();
        }))
        .unwrap();
        tauri::async_runtime::block_on(save(
            root.path().into(),
            main,
            2,
            "es",
            OwnedEdits {
                disabled: Some(false),
                auto_paste: None,
            },
            OwnedEditGenerations {
                disabled: Some(1),
                auto_paste: None,
            },
        ));
        let disk = recovered(root.path());
        assert_eq!(disk["disabled"], true);
        assert_eq!(disk["autoPaste"], false);
        assert_eq!(disk["language"], "es");
        assert_eq!(disk["ownedFieldsRevision"], 2);
    }

    #[test]
    fn rapid_tray_toggles_resolve_inside_native_fifo() {
        let (root, _, _) = setup();
        for expected in [true, false] {
            let path = root.path().to_path_buf();
            tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
                commit_tray_disabled(&path, |value| {
                    assert_eq!(value, expected);
                    Ok(())
                })
                .unwrap();
            }))
            .unwrap();
        }
        let disk = recovered(root.path());
        assert_eq!(disk["disabled"], false);
        assert_eq!(disk["ownedFieldsRevision"], 2);
    }

    #[test]
    fn tray_disable_retains_ownership_after_publication_failure() {
        let (root, _, _) = setup();
        let dir = root.path().to_path_buf();
        tauri::async_runtime::block_on(crate::shutdown::source_call(move || {
            let path = dir.join("settings.json");
            let held = dir.join("synthetic-settings-held");
            let result = commit_tray_disabled(&dir, |disabled| {
                assert!(disabled);
                // Fail the real publisher after merge/runtime acceptance.
                std::fs::rename(&path, &held).unwrap();
                std::fs::create_dir(&path).unwrap();
                Ok(())
            });
            assert!(result.is_err());
            std::fs::remove_dir(&path).unwrap();
            std::fs::rename(&held, &path).unwrap();
            let _guard = WRITE_LOCK.lock_or_recover();
            let rebased =
                rebase_runtime_owned_fields(&dir, serde_json::json!({"disabled":false})).unwrap();
            assert_eq!(rebased["disabled"], true);
        }))
        .unwrap();
        let disk = recovered(root.path());
        assert_eq!(disk["disabled"], true);
        assert_eq!(disk["autoPaste"], false);
    }

    #[test]
    fn runtime_owned_rebase_rejects_malformed_snapshots_and_preserves_initialization() {
        let root = tempfile::tempdir().unwrap();
        let initial = serde_json::json!({"disabled":true,"autoPaste":false});
        assert_eq!(
            rebase_runtime_owned_fields(root.path(), initial.clone()).unwrap(),
            initial
        );
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!([]),
            serde_json::json!(false),
        ] {
            assert!(rebase_runtime_owned_fields(root.path(), invalid).is_err());
        }
    }

    #[test]
    fn superseded_auto_paste_patch_survives_native_shutdown_without_renderer_flush() {
        let (root, main, overlay) = setup();
        tauri::async_runtime::block_on(quick(
            root.path().into(),
            overlay,
            OverlaySettingField::Disabled,
            true,
        ));
        // A is constructed first, but B is accepted/executed first, with no
        // owned fields and a stale cache (the overlay event has not arrived).
        let a = save(
            root.path().into(),
            main.clone(),
            1,
            "en",
            OwnedEdits {
                auto_paste: Some(true),
                disabled: None,
            },
            OwnedEditGenerations {
                auto_paste: Some(1),
                disabled: None,
            },
        );
        let b = save(
            root.path().into(),
            main,
            2,
            "es",
            OwnedEdits::default(),
            OwnedEditGenerations::default(),
        );
        assert!(matches!(
            tauri::async_runtime::block_on(b),
            SourceWrite::Applied { .. }
        ));
        assert_eq!(tauri::async_runtime::block_on(a), SourceWrite::Superseded);
        let disk = recovered(root.path());
        assert_eq!(disk["autoPaste"], true);
        assert_eq!(disk["disabled"], true);
        assert_eq!(disk["language"], "es");
    }
    #[test]
    fn accepted_patch_is_native_owned_even_when_merge_read_fails() {
        let (root, main, _) = setup();
        let path = root.path().join("settings.json");
        let held = root.path().join("synthetic-settings-held");
        std::fs::rename(&path, &held).unwrap();
        std::fs::create_dir(&path).unwrap();
        let dir = root.path().to_path_buf();
        let result = tauri::async_runtime::block_on(crate::shutdown::ordered_source_call(
            "main".into(),
            main,
            move |order, owner, session| {
                let (_, edits, _) = select_settings_intent(
                    order,
                    owner,
                    session,
                    1,
                    OwnedEdits {
                        auto_paste: Some(true),
                        disabled: None,
                    },
                    OwnedEditGenerations {
                        auto_paste: Some(1),
                        disabled: None,
                    },
                )?;
                let _guard = WRITE_LOCK.lock_or_recover();
                commit_settings_intent(&dir, "{}", Some(&edits), false, || ())
            },
        ));
        assert!(result.is_err());
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(held, path).unwrap();
        // No renderer retry: the shutdown owner publishes the retained field.
        assert_eq!(recovered(root.path())["autoPaste"], true);
    }
    #[test]
    fn superseded_settings_receipt_separates_document_and_exact_field_ownership() {
        let receipt = SettingsWrite {
            snapshot: SourceWrite::Superseded,
            owned_patch_generations: OwnedEditGenerations {
                auto_paste: Some(1),
                disabled: None,
            },
        };
        assert_eq!(
            serde_json::to_value(receipt).unwrap(),
            serde_json::json!({
                "status":"superseded", "ownedPatchGenerations":{"autoPaste":1}
            })
        );
    }

    #[test]
    fn concurrent_windows_order_same_field_and_never_clobber_other_fields() {
        let (root, main, overlay) = setup();
        let save_auto = |g, value| {
            save(
                root.path().into(),
                main.clone(),
                g,
                "es",
                OwnedEdits {
                    auto_paste: Some(value),
                    disabled: None,
                },
                OwnedEditGenerations {
                    auto_paste: Some(g),
                    disabled: None,
                },
            )
        };
        tauri::async_runtime::block_on(save_auto(1, true));
        tauri::async_runtime::block_on(quick(
            root.path().into(),
            overlay.clone(),
            OverlaySettingField::Disabled,
            true,
        ));
        tauri::async_runtime::block_on(quick(
            root.path().into(),
            overlay.clone(),
            OverlaySettingField::AutoPaste,
            false,
        ));
        // Identical retry retains its original field ID, so cannot undo the
        // later overlay edit even though its snapshot gets a fresh generation.
        tauri::async_runtime::block_on(save(
            root.path().into(),
            main.clone(),
            2,
            "es",
            OwnedEdits {
                auto_paste: Some(true),
                disabled: None,
            },
            OwnedEditGenerations {
                auto_paste: Some(1),
                disabled: None,
            },
        ));
        assert_eq!(recovered(root.path())["autoPaste"], false);
        tauri::async_runtime::block_on(save_auto(3, true));
        let disk = recovered(root.path());
        assert_eq!(disk["autoPaste"], true);
        assert_eq!(disk["disabled"], true);
        // Reloading overlay retires only overlay, not main.
        let reloaded = uuid::Uuid::new_v4().to_string();
        crate::shutdown::register_test_source("overlay", &reloaded);
        tauri::async_runtime::block_on(quick(
            root.path().into(),
            reloaded,
            OverlaySettingField::Disabled,
            false,
        ));
        assert_eq!(
            tauri::async_runtime::block_on(quick(
                root.path().into(),
                overlay,
                OverlaySettingField::Disabled,
                true
            )),
            SourceWrite::Superseded
        );
        tauri::async_runtime::block_on(save_auto(4, false));
        let disk = recovered(root.path());
        assert_eq!(disk["autoPaste"], false);
        assert_eq!(disk["disabled"], false);
    }
}
