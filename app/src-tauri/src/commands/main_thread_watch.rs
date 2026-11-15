//! The Diagnostics switch for the main-thread watch.
//!
//! The watch is off unless this persisted setting or the
//! `MURMUR_MAIN_THREAD_WATCH` environment variable arms it. The setting lives
//! in its own small file beside `settings.json`, owned by Rust, so no window's
//! cached settings snapshot can overwrite it and the pop-out Diagnostics window
//! can change it too. Changing it applies at once: the main thread adds or
//! removes the run-loop observers (see `main_thread_watch`).
//!
//! A change is one transaction, serialized by [`CHANGE_LOCK`]: write the file,
//! have the main thread apply it, read the resulting status, and broadcast it,
//! all before the next change starts. Disk, the in-memory setting and the
//! running watch therefore cannot disagree, and `seq` orders what windows hear.
//! Only the main and Diagnostics windows may read or change it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tauri::Emitter;

use crate::main_thread_watch;

const FILE_NAME: &str = "main-thread-watch.json";
/// Windows that show the Diagnostics control, and the only ones allowed to use
/// these commands.
const OWNER_WINDOWS: [&str; 2] = ["main", "diagnostics"];
/// Emitted to each owner window after every change, with the new status.
const STATUS_EVENT: &str = "main-thread-watch-status";

/// The persisted setting as this process last read or wrote it, so a status
/// request never touches the disk. Written only under [`CHANGE_LOCK`].
static PERSISTED: AtomicBool = AtomicBool::new(false);
/// Grows with every applied change. A window that hears the same change twice
/// (event and read), or out of order, keeps the newest.
static STATUS_SEQ: AtomicU64 = AtomicU64::new(0);
/// Held for a whole change, through the main thread's acknowledgement and the
/// broadcast. An async lock, so waiting for it never blocks a thread.
static CHANGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainThreadWatchStatus {
    /// The persisted setting.
    pub setting: bool,
    /// The watch is running now.
    pub running: bool,
    /// The environment arms it, so the setting cannot turn it off.
    pub forced_by_env: bool,
    /// Orders statuses; see [`STATUS_SEQ`].
    pub seq: u64,
}

fn status() -> MainThreadWatchStatus {
    MainThreadWatchStatus {
        setting: PERSISTED.load(Ordering::Acquire),
        running: main_thread_watch::is_armed(),
        forced_by_env: main_thread_watch::armed_by_env(),
        seq: STATUS_SEQ.load(Ordering::Acquire),
    }
}

fn require_owner_window(label: &str) -> Result<(), String> {
    if OWNER_WINDOWS.contains(&label) {
        Ok(())
    } else {
        Err("The main-thread watch is only available in Diagnostics.".to_string())
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

async fn write_off_thread(dir: PathBuf, armed: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || write_setting(&dir, armed))
        .await
        .map_err(|_| "The setting could not be saved.".to_string())?
}

/// One serialized change: save, apply, then publish the resulting status.
/// `apply` runs the change on the main thread and returns once it has; if it
/// fails the previous setting is put back, so disk and memory still match
/// the watch. `publish` runs before the next change may start.
async fn change_setting<A, Fut>(
    dir: PathBuf,
    armed: bool,
    apply: A,
    publish: impl FnOnce(&MainThreadWatchStatus),
) -> Result<MainThreadWatchStatus, String>
where
    A: FnOnce(bool) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let _change = CHANGE_LOCK.lock().await;
    let previous = PERSISTED.load(Ordering::Acquire);
    write_off_thread(dir.clone(), armed).await?;
    PERSISTED.store(armed, Ordering::Release);
    if let Err(error) = apply(armed).await {
        PERSISTED.store(previous, Ordering::Release);
        let _ = write_off_thread(dir, previous).await;
        return Err(error);
    }
    STATUS_SEQ.fetch_add(1, Ordering::AcqRel);
    let status = status();
    publish(&status);
    Ok(status)
}

#[tauri::command]
pub async fn get_main_thread_watch_status(
    window: tauri::WebviewWindow,
) -> Result<MainThreadWatchStatus, String> {
    require_owner_window(window.label())?;
    Ok(status())
}

/// Persist the setting, have the main thread apply it, and tell the windows.
#[tauri::command]
pub async fn set_main_thread_watch_armed(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    armed: bool,
) -> Result<MainThreadWatchStatus, String> {
    require_owner_window(window.label())?;
    let dir = super::settings_store::data_dir(&app)?;
    let main_thread = app.clone();
    change_setting(
        dir,
        armed,
        move |armed| async move {
            let (applied, done) = tokio::sync::oneshot::channel();
            main_thread
                .run_on_main_thread(move || {
                    main_thread_watch::apply_setting(armed);
                    let _ = applied.send(());
                })
                .map_err(|e| format!("Failed to reach the main thread: {e}"))?;
            done.await
                .map_err(|_| "The main thread did not apply the setting.".to_string())
        },
        |status| {
            for label in OWNER_WINDOWS {
                let _ = app.emit_to(label, STATUS_EVENT, *status);
            }
        },
    )
    .await
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
            seq: 7,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "setting": true, "running": false, "forcedByEnv": true, "seq": 7
            })
        );
    }

    #[test]
    fn only_the_main_and_diagnostics_windows_may_use_the_commands() {
        assert!(require_owner_window("main").is_ok());
        assert!(require_owner_window("diagnostics").is_ok());
        for label in ["overlay", "dictation-preview", "settings-editors", ""] {
            assert!(require_owner_window(label).is_err(), "{label:?}");
        }
    }

    use std::sync::{Arc, Mutex};

    /// Two opposing changes at once, many times over. Each is one transaction,
    /// so disk, the in-memory setting and what the main thread applied last
    /// always name the same value, and the statuses go out in `seq` order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_opposing_changes_leave_disk_memory_and_the_watch_in_agreement() {
        let dir = temp_dir("concurrent");
        let applied = Arc::new(Mutex::new(false));
        let published = Arc::new(Mutex::new(Vec::<MainThreadWatchStatus>::new()));
        for _ in 0..40 {
            let change = |armed: bool| {
                let (dir, applied, published) = (dir.clone(), applied.clone(), published.clone());
                tokio::spawn(async move {
                    change_setting(
                        dir,
                        armed,
                        move |armed| async move {
                            // Give the other change every chance to interleave.
                            tokio::task::yield_now().await;
                            tokio::time::sleep(std::time::Duration::from_micros(300)).await;
                            *applied.lock().unwrap() = armed;
                            Ok(())
                        },
                        move |status| published.lock().unwrap().push(*status),
                    )
                    .await
                })
            };
            let (on, off) = (change(true), change(false));
            on.await.unwrap().unwrap();
            off.await.unwrap().unwrap();
            let on_disk = read_setting(&dir);
            assert_eq!(on_disk, *applied.lock().unwrap(), "disk vs the watch");
            assert_eq!(on_disk, PERSISTED.load(Ordering::Acquire), "disk vs memory");
        }
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 80);
        assert!(
            published.windows(2).all(|pair| pair[0].seq < pair[1].seq),
            "statuses are published in order"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_failed_apply_puts_the_previous_setting_back() {
        let dir = temp_dir("failed-apply");
        change_setting(dir.clone(), true, |_| async { Ok(()) }, |_| {})
            .await
            .unwrap();
        let seq = STATUS_SEQ.load(Ordering::Acquire);
        let error = change_setting(
            dir.clone(),
            false,
            |_| async { Err("main thread unreachable".to_string()) },
            |_| panic!("nothing is published for a failed change"),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "main thread unreachable");
        assert!(read_setting(&dir), "the file is back to the previous value");
        assert!(PERSISTED.load(Ordering::Acquire));
        assert_eq!(STATUS_SEQ.load(Ordering::Acquire), seq);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
