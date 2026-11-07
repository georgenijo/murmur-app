//! Transport only. Network, trust, policy, state and recovery live in murmur-core.
use crate::events::EventsExt;
use crate::host::ServiceAccess;
use murmur_core::events::{EventKind, EventSink};
use murmur_core::updater::{self, Action, Snapshot, Updater};
use std::sync::Arc;
#[cfg(feature = "tauri-host")]
use tauri::Manager;

pub(crate) fn initialize(
    app: &crate::host::Context,
    store: murmur_core::completion::CompletionService,
) -> Result<(), String> {
    let root = super::settings_store::data_dir(app)?;
    let changed = app.clone();
    let notification = app.clone();
    let focused = std::sync::atomic::AtomicBool::new(false);
    let updater = Updater::production(
        root,
        std::env::current_exe().map_err(|_| "Updater executable unavailable")?,
        app.config().identifier.clone(),
        app.package_info().version.to_string(),
        store,
        move |snapshot| {
            let version = match &snapshot.update_status {
                updater::Status::Available { version, .. }
                | updater::Status::Ready { version, .. }
                | updater::Status::Preparing { version }
                | updater::Status::Downloading { version, .. }
                | updater::Status::Restarting { version } => Some(version.clone()),
                _ => None,
            };
            let _ = super::tray::set_tray_update_available(version);
            activate_completed(&snapshot, &focused, || {
                let app = changed.clone();
                let _ = changed.run_on_main_thread(move || {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                });
            });
            let _ = changed
                .events()
                .publish(EventKind::UpdaterStateChanged, snapshot);
        },
        move |version| {
            #[cfg(feature = "tauri-host")]
            {
                use tauri_plugin_notification::NotificationExt;
                let _ = notification
                    .notification()
                    .builder()
                    .title("Update Available")
                    .body(format!("Murmur v{version} is ready to install."))
                    .show();
            }
            #[cfg(not(feature = "tauri-host"))]
            notification.notify_update(&version);
        },
    );
    #[cfg(any(feature = "shutdown-test-support", feature = "native-host-qa"))]
    let updater = fixture::configure(app, updater)?;
    app.install_updater_owner(Arc::new(updater));
    let timer = app.clone();
    crate::host::runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if crate::shutdown::closing() {
                break;
            }
            let app = timer.clone();
            let _ = crate::host::runtime::spawn_blocking(move || {
                run(&app, Action::Check { background: true })
            })
            .await;
        }
    });
    Ok(())
}
// A failed activation never consumes durable notes or repeats focus on each
// Configure/snapshot. The window operations remain on the native main thread.
fn activate_completed(
    snapshot: &Snapshot,
    focused: &std::sync::atomic::AtomicBool,
    activate: impl FnOnce(),
) {
    if snapshot.completed_update.is_some()
        && !focused.swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        activate();
    }
}
fn main_only(window: &crate::host::Window) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Updater belongs to the main window".into());
    }
    Ok(())
}
fn owner(app: &crate::host::Context) -> Result<Arc<Updater>, String> {
    app.try_updater_owner()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Updater is still initializing".into())
}
fn run(app: &crate::host::Context, action: Action) -> Result<Snapshot, String> {
    crate::shutdown::require_running()?;
    // Other internal builds must never consume the production feed or replace
    // their unsigned developer bundle. The QA fixture alone opts in explicitly.
    if crate::identity::internal() {
        #[cfg(any(feature = "shutdown-test-support", feature = "native-host-qa"))]
        if !fixture::enabled(app) {
            return Err("Updater unavailable for this internal identity".into());
        }
        #[cfg(not(any(feature = "shutdown-test-support", feature = "native-host-qa")))]
        return Err("Updater unavailable for this internal identity".into());
    }
    let updater = owner(app)?;
    let relaunch = updater.act(action)?;
    if relaunch {
        crate::shutdown::require_running()?;
        crate::shutdown::request(app, murmur_core::shutdown::Request::Restart);
    }
    Ok(updater.snapshot())
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn updater_snapshot(
    app: crate::host::Context,
    window: crate::host::Window,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    Ok(owner(&app)?.snapshot())
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub async fn updater_action(
    app: crate::host::Context,
    window: crate::host::Window,
    action: Action,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    crate::host::runtime::spawn_blocking(move || run(&app, action))
        .await
        .map_err(|_| "Updater worker unavailable".to_string())?
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallEnvironment {
    app_translocated: bool,
}
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_update_install_environment() -> Result<UpdateInstallEnvironment, String> {
    Ok(UpdateInstallEnvironment {
        app_translocated: updater::translocated(
            &std::env::current_exe().map_err(|_| "Could not verify installation location")?,
        ),
    })
}
#[derive(serde::Deserialize)]
pub struct UpdaterCanaryRequest {
    action: String,
    result: Option<serde_json::Value>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterCanaryState {
    path: Option<String>,
    result: Option<serde_json::Value>,
    dry_run: bool,
}
/// Existing OTA canary command contract is retained; Rust runs the canary.
#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn updater_canary(
    window: crate::host::Window,
    request: UpdaterCanaryRequest,
) -> Result<UpdaterCanaryState, String> {
    main_only(&window)?;
    match request.action.as_str() {
        "read" => {}
        "write" => updater::canary_write(&request.result.ok_or("Canary result required")?)?,
        _ => return Err("Unknown updater canary action".into()),
    }
    Ok(UpdaterCanaryState {
        path: updater::canary_path().map(|p| p.to_string_lossy().into_owned()),
        result: updater::canary_read(),
        dry_run: updater::canary_dry_run(),
    })
}

#[cfg(any(feature = "shutdown-test-support", feature = "native-host-qa"))]
mod fixture {
    use super::*;
    /// This override is compiled only with the QA feature. Production identity
    /// ignores it, even in a feature-enabled developer build.
    pub(super) fn enabled(app: &crate::host::Context) -> bool {
        ((cfg!(feature = "shutdown-test-support")
            && app.config().identifier == "com.localdictation.shutdown-test")
            || (cfg!(feature = "native-host-qa")
                && app.config().identifier == "com.localdictation.gpui-host-qa110"))
            && std::env::var_os("MURMUR_UPDATER_FIXTURE").is_some()
    }
    pub(super) fn configure(
        app: &crate::host::Context,
        updater: Updater,
    ) -> Result<Updater, String> {
        if !enabled(app) {
            return Ok(updater);
        }
        let path = std::path::PathBuf::from(
            std::env::var_os("MURMUR_UPDATER_FIXTURE").ok_or("Updater fixture missing")?,
        );
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "Updater fixture unreadable")?
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "Updater fixture unreadable")?;
        if bytes.len() > 65536 {
            return Err("Updater fixture too large".into());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid updater fixture")?;
        updater.isolated_fixture(
            value["feed"].as_str().ok_or("Fixture feed missing")?.into(),
            value["publicKey"]
                .as_str()
                .ok_or("Fixture key missing")?
                .into(),
        )
    }
}

#[cfg(feature = "shutdown-test-support")]
pub(crate) fn fixture_plugin() -> tauri::plugin::TauriPlugin<crate::host::DefaultRuntime> {
    tauri::plugin::Builder::new("updater-test")
        .invoke_handler(tauri::generate_handler![updater_test_run])
        .build()
}
#[cfg(feature = "shutdown-test-support")]
#[cfg_attr(feature = "tauri-host", tauri::command)]
async fn updater_test_run(
    app: crate::host::Context,
    window: crate::host::Window,
    action: Action,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    if !fixture::enabled(&app) {
        return Err("Isolated updater fixture required".into());
    }
    updater_action(app, window, action).await
}

pub(crate) fn close_admission(app: &crate::host::Context) {
    if let Ok(updater) = owner(app) {
        updater.stop();
    }
}
pub(crate) fn shutdown_flush(app: &crate::host::Context, deadline: std::time::Instant) -> bool {
    let Ok(updater) = owner(app) else {
        return true;
    };
    updater.stop();
    while std::time::Instant::now() < deadline {
        if updater.drained() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn completed_notes_activate_once_and_survive_activation_failure() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let focused = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let mut snapshot = murmur_core::updater::Snapshot::default();
        super::activate_completed(&snapshot, &focused, || {
            calls.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        snapshot.completed_update = Some(murmur_core::updater::CompletedUpdate {
            version: "1.2.0".into(),
            notes: "Notes".into(),
        });
        super::activate_completed(&snapshot, &focused, || {
            calls.fetch_add(1, Ordering::Relaxed);
            let _ = Err::<(), _>("Synthetic focus unavailable");
        });
        super::activate_completed(&snapshot, &focused, || {
            calls.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(snapshot.completed_update.unwrap().notes, "Notes");
    }
    #[test]
    fn canary_command_preserves_read_write_wire_schema() {
        let read: super::UpdaterCanaryRequest =
            serde_json::from_value(serde_json::json!({"action":"read"})).unwrap();
        assert_eq!(read.action, "read");
        assert!(read.result.is_none());
        let write: super::UpdaterCanaryRequest = serde_json::from_value(
            serde_json::json!({"action":"write","result":{"schemaVersion":1,"status":"dry-run"}}),
        )
        .unwrap();
        assert_eq!(write.result.unwrap()["status"], "dry-run");
        let state = super::UpdaterCanaryState {
            path: Some("synthetic/canary.json".into()),
            result: None,
            dry_run: true,
        };
        assert_eq!(
            serde_json::to_value(state).unwrap(),
            serde_json::json!({"path":"synthetic/canary.json","result":null,"dryRun":true})
        );
    }
    #[test]
    fn generated_updater_fixture_acl_is_qa_only_and_main_only() {
        let mut context = crate::generated_context();
        let authority = context.runtime_authority_mut();
        for window in ["main", "diagnostics", "overlay", "dictation-preview"] {
            let allowed = authority
                .resolve_access(
                    "plugin:updater-test|updater_test_run",
                    window,
                    window,
                    &tauri::ipc::Origin::Local,
                )
                .is_some();
            assert_eq!(
                allowed,
                cfg!(feature = "shutdown-test-support") && window == "main"
            );
        }
        // Neither the removed updater plugin nor a process relaunch capability
        // can bypass the native owner in either generated configuration.
        for command in ["plugin:updater|download", "plugin:process|restart"] {
            assert!(authority
                .resolve_access(command, "main", "main", &tauri::ipc::Origin::Local)
                .is_none());
        }
    }
}
