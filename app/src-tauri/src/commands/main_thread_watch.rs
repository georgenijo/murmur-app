//! The Diagnostics switch for the main-thread watch.
//!
//! The watch is off unless this persisted setting or the
//! `MURMUR_MAIN_THREAD_WATCH` environment variable arms it, and it is armed
//! only at launch (see `main_thread_watch`). The switch therefore only saves
//! the setting: a change takes effect the next time Murmur starts, and the
//! status says so by reporting both the saved setting and whether the watch is
//! running in this launch, which never changes within a process.
//!
//! The setting lives in its own small file beside `settings.json`, owned by
//! Rust, so no window's cached settings snapshot can overwrite it and the
//! pop-out Diagnostics window can change it too.
//!
//! [`CHANGE_LOCK`] orders everything. A change holds it from the file write to
//! the broadcast, and a status read takes it too, so a read is one coherent
//! snapshot: it never sees a setting without its `seq`, and it never returns
//! after a newer event with an older state. Only the main and Diagnostics
//! windows may read or change the setting.

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

/// The saved setting as this process last read or wrote it, so a status
/// request never touches the disk. Written only under [`CHANGE_LOCK`].
static PERSISTED: AtomicBool = AtomicBool::new(false);
/// Grows with every saved change. A window that hears the same change twice
/// (event and read), or out of order, keeps the newest. Written only under
/// [`CHANGE_LOCK`].
static STATUS_SEQ: AtomicU64 = AtomicU64::new(0);
/// Held for a whole change and for every status read. An async lock, so
/// waiting for it never blocks a thread.
static CHANGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MainThreadWatchStatus {
    /// The saved setting. It applies at the next launch.
    pub setting: bool,
    /// The watch is running in this launch. Fixed for the life of the process.
    pub running_this_launch: bool,
    /// The environment arms it, so the setting cannot turn it off.
    pub forced_by_env: bool,
    /// Orders statuses; see [`STATUS_SEQ`].
    pub seq: u64,
}

/// One coherent snapshot. The caller holds [`CHANGE_LOCK`].
fn snapshot() -> MainThreadWatchStatus {
    MainThreadWatchStatus {
        setting: PERSISTED.load(Ordering::Acquire),
        running_this_launch: main_thread_watch::is_armed(),
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
/// on the main thread inside `setup`, after telemetry. This is the only place
/// the watch is ever armed.
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

/// A status read: one snapshot, taken under the same lock a change holds.
async fn read_status() -> MainThreadWatchStatus {
    let _read = CHANGE_LOCK.lock().await;
    snapshot()
}

/// One serialized change: save the file, then publish the new status. The file
/// is written first, so a failed write leaves the setting, the sequence and
/// every window exactly as they were. `publish` runs before the next change or
/// read may start.
async fn change_setting(
    dir: PathBuf,
    armed: bool,
    publish: impl FnOnce(&MainThreadWatchStatus),
) -> Result<MainThreadWatchStatus, String> {
    let _change = CHANGE_LOCK.lock().await;
    write_off_thread(dir, armed).await?;
    PERSISTED.store(armed, Ordering::Release);
    STATUS_SEQ.fetch_add(1, Ordering::AcqRel);
    let status = snapshot();
    publish(&status);
    Ok(status)
}

#[tauri::command]
pub async fn get_main_thread_watch_status(
    window: tauri::WebviewWindow,
) -> Result<MainThreadWatchStatus, String> {
    require_owner_window(window.label())?;
    Ok(read_status().await)
}

/// Save the setting and tell the windows. It applies at the next launch.
#[tauri::command]
pub async fn set_main_thread_watch_armed(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    armed: bool,
) -> Result<MainThreadWatchStatus, String> {
    require_owner_window(window.label())?;
    let dir = super::settings_store::data_dir(&app)?;
    change_setting(dir, armed, |status| {
        for label in OWNER_WINDOWS {
            let _ = app.emit_to(label, STATUS_EVENT, *status);
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

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
            running_this_launch: false,
            forced_by_env: true,
            seq: 7,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "setting": true, "runningThisLaunch": false, "forcedByEnv": true, "seq": 7
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

    /// Two opposing changes at once, many times over. Each is one transaction,
    /// so disk and the in-memory setting always name the same value, and the
    /// statuses go out in `seq` order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_opposing_changes_leave_disk_and_memory_in_agreement() {
        let dir = temp_dir("concurrent");
        let published = Arc::new(Mutex::new(Vec::<MainThreadWatchStatus>::new()));
        for _ in 0..40 {
            let change = |armed: bool| {
                let (dir, published) = (dir.clone(), published.clone());
                tokio::spawn(async move {
                    change_setting(dir, armed, move |status| {
                        published.lock().unwrap().push(*status)
                    })
                    .await
                })
            };
            let (on, off) = (change(true), change(false));
            on.await.unwrap().unwrap();
            off.await.unwrap().unwrap();
            assert_eq!(
                read_setting(&dir),
                PERSISTED.load(Ordering::Acquire),
                "disk vs memory"
            );
        }
        let published = published.lock().unwrap();
        assert_eq!(published.len(), 80);
        assert!(
            published.windows(2).all(|pair| pair[0].seq < pair[1].seq),
            "statuses are published in order"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads that run while changes are in flight. Every read must return a
    /// `(seq, setting)` pair some change actually committed, never a setting
    /// with another change's `seq`, and the `runningThisLaunch` value never
    /// moves.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_read_spanning_changes_is_one_committed_snapshot() {
        let dir = temp_dir("spanning-read");
        let committed = Arc::new(Mutex::new(HashMap::<u64, bool>::new()));
        let baseline = read_status().await;
        committed
            .lock()
            .unwrap()
            .insert(baseline.seq, baseline.setting);

        let writer = {
            let (dir, committed) = (dir.clone(), committed.clone());
            tokio::spawn(async move {
                for i in 0..60 {
                    let committed = committed.clone();
                    change_setting(dir.clone(), i % 2 == 0, move |status| {
                        committed.lock().unwrap().insert(status.seq, status.setting);
                    })
                    .await
                    .unwrap();
                    tokio::task::yield_now().await;
                }
            })
        };
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let committed = committed.clone();
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    for _ in 0..120 {
                        let status = read_status().await;
                        let known = committed.lock().unwrap().get(&status.seq).copied();
                        assert_eq!(known, Some(status.setting), "seq {}", status.seq);
                        assert_eq!(status.running_this_launch, baseline.running_this_launch);
                        seen.push(status.seq);
                        tokio::task::yield_now().await;
                    }
                    assert!(
                        seen.windows(2).all(|pair| pair[0] <= pair[1]),
                        "a later read never goes back to an older state"
                    );
                })
            })
            .collect();
        writer.await.unwrap();
        for reader in readers {
            reader.await.unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The race above is too narrow to catch every time, so this pins the rule
    /// itself: while a change holds the lock, a read waits for it and then
    /// sees the finished change, not the state from before it.
    #[tokio::test]
    async fn a_read_waits_for_the_change_in_flight() {
        let before = read_status().await;
        let change = CHANGE_LOCK.lock().await;
        let read = tokio::spawn(read_status());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!read.is_finished(), "a read must wait for the change");
        PERSISTED.store(!before.setting, Ordering::Release);
        STATUS_SEQ.fetch_add(1, Ordering::AcqRel);
        drop(change);
        let after = read.await.unwrap();
        assert_eq!(after.seq, before.seq + 1);
        assert_eq!(after.setting, !before.setting);
    }

    #[tokio::test]
    async fn a_failed_save_changes_nothing_and_publishes_nothing() {
        let dir = temp_dir("failed-save");
        change_setting(dir.clone(), true, |_| {}).await.unwrap();
        let before = read_status().await;
        // The directory is gone, so the write fails.
        let _ = std::fs::remove_dir_all(&dir);
        let error = change_setting(dir.clone(), false, |_| {
            panic!("nothing is published for a failed change")
        })
        .await
        .unwrap_err();
        assert!(error.contains("Failed to save the setting"), "{error}");
        assert_eq!(read_status().await, before);
    }
}
