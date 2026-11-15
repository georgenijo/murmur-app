//! The Diagnostics switch for the main-thread watch.
//!
//! The watch is off unless this persisted setting or the
//! `MURMUR_MAIN_THREAD_WATCH` environment variable arms it. The setting lives
//! in its own small file beside `settings.json`, owned by Rust, so no window's
//! cached settings snapshot can overwrite it and the pop-out Diagnostics window
//! can change it too. Changing it applies at once: the main thread adds or
//! removes the run-loop observers (see `main_thread_watch`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::main_thread_watch;

const FILE_NAME: &str = "main-thread-watch.json";

/// The persisted setting as this process last read or wrote it, so a status
/// request never touches the disk.
static PERSISTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainThreadWatchStatus {
    /// The persisted setting.
    pub setting: bool,
    /// The watch is running now.
    pub running: bool,
    /// The environment arms it, so the setting cannot turn it off.
    pub forced_by_env: bool,
}

fn status() -> MainThreadWatchStatus {
    MainThreadWatchStatus {
        setting: PERSISTED.load(Ordering::Acquire),
        running: main_thread_watch::is_armed(),
        forced_by_env: main_thread_watch::armed_by_env(),
    }
}

fn file_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// Missing, unreadable or malformed means off.
fn read_setting(dir: &Path) -> bool {
    std::fs::read_to_string(file_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("armed").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Publish atomically: a temp sibling, then a rename.
fn write_setting(dir: &Path, armed: bool) -> Result<(), String> {
    let path = file_path(dir);
    let temp = path.with_file_name(format!(".{FILE_NAME}.murmur-tmp"));
    let body = serde_json::json!({ "armed": armed }).to_string();
    std::fs::write(&temp, body).map_err(|e| format!("Failed to save the setting: {e}"))?;
    std::fs::rename(&temp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("Failed to save the setting: {e}")
    })
}

/// Launch: read the setting and start the watch only if it asks for it. Runs
/// on the main thread inside `setup`, after telemetry.
pub(crate) fn initialize(app: &tauri::AppHandle) {
    let persisted = super::settings_store::data_dir(app)
        .map(|dir| read_setting(&dir))
        .unwrap_or(false);
    PERSISTED.store(persisted, Ordering::Release);
    main_thread_watch::start_at_launch(persisted);
}

#[tauri::command]
pub async fn get_main_thread_watch_status() -> Result<MainThreadWatchStatus, String> {
    Ok(status())
}

/// Persist the setting, then have the main thread apply it.
#[tauri::command]
pub async fn set_main_thread_watch_armed(
    app: tauri::AppHandle,
    armed: bool,
) -> Result<MainThreadWatchStatus, String> {
    let dir = super::settings_store::data_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || write_setting(&dir, armed))
        .await
        .map_err(|_| "The setting could not be saved.".to_string())??;
    PERSISTED.store(armed, Ordering::Release);
    let (applied, done) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        main_thread_watch::apply_setting(armed);
        let _ = applied.send(());
    })
    .map_err(|e| format!("Failed to reach the main thread: {e}"))?;
    done.await
        .map_err(|_| "The main thread did not apply the setting.".to_string())?;
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur-main-thread-watch-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_setting_defaults_off_and_round_trips() {
        let dir = temp_dir("round-trip");
        assert!(!read_setting(&dir), "no file means off");
        write_setting(&dir, true).unwrap();
        assert!(read_setting(&dir));
        write_setting(&dir, false).unwrap();
        assert!(!read_setting(&dir));
        assert!(!dir.join(format!(".{FILE_NAME}.murmur-tmp")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_setting_file_means_off() {
        let dir = temp_dir("malformed");
        for body in [
            "",
            "not json",
            "[]",
            r#"{"armed":"yes"}"#,
            r#"{"other":true}"#,
        ] {
            std::fs::write(file_path(&dir), body).unwrap();
            assert!(!read_setting(&dir), "{body:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_serializes_with_camel_case_keys() {
        let value = serde_json::to_value(MainThreadWatchStatus {
            setting: true,
            running: false,
            forced_by_env: true,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "setting": true, "running": false, "forcedByEnv": true })
        );
    }
}
