//! Transport only. Network, trust, policy, state and recovery live in murmur-core.
use crate::events::EventsExt;
use murmur_core::events::{EventKind, EventSink};
use murmur_core::updater::{self, Action, Snapshot, Updater};
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn initialize(
    app: &tauri::AppHandle,
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
            if snapshot.completed_update.is_some()
                && !focused.swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                let app = changed.clone();
                let _ = changed.run_on_main_thread(move || {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                });
            }
            let _ = changed
                .events()
                .publish(EventKind::UpdaterStateChanged, snapshot);
        },
        move |version| {
            use tauri_plugin_notification::NotificationExt;
            let _ = notification
                .notification()
                .builder()
                .title("Murmur update available")
                .body(format!("Murmur v{version} is ready to install."))
                .show();
        },
    );
    #[cfg(feature = "shutdown-test-support")]
    let updater = fixture::configure(app, updater)?;
    app.manage(Arc::new(updater));
    let timer = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if crate::shutdown::closing() {
                break;
            }
            let app = timer.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                run(&app, Action::Check { background: true })
            })
            .await;
        }
    });
    Ok(())
}
fn main_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Updater belongs to the main window".into());
    }
    Ok(())
}
fn owner(app: &tauri::AppHandle) -> Result<Arc<Updater>, String> {
    app.try_state::<Arc<Updater>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "Updater is still initializing".into())
}
fn run(app: &tauri::AppHandle, action: Action) -> Result<Snapshot, String> {
    crate::shutdown::require_running()?;
    // Other internal builds must never consume the production feed or replace
    // their unsigned developer bundle. The QA fixture alone opts in explicitly.
    if crate::identity::internal() {
        #[cfg(feature = "shutdown-test-support")]
        if !fixture::enabled(app) {
            return Err("Updater unavailable for this internal identity".into());
        }
        #[cfg(not(feature = "shutdown-test-support"))]
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
#[tauri::command]
pub fn updater_snapshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    Ok(owner(&app)?.snapshot())
}
#[tauri::command]
pub async fn updater_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: Action,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(move || run(&app, action))
        .await
        .map_err(|_| "Updater worker unavailable".to_string())?
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstallEnvironment {
    app_translocated: bool,
}
#[tauri::command]
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
#[tauri::command]
pub fn updater_canary(
    window: tauri::WebviewWindow,
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

#[cfg(feature = "shutdown-test-support")]
mod fixture {
    use super::*;
    /// This override is compiled only with the QA feature. Production identity
    /// ignores it, even in a feature-enabled developer build.
    pub(super) fn enabled(app: &tauri::AppHandle) -> bool {
        app.config().identifier == "com.localdictation.shutdown-test"
            && std::env::var_os("MURMUR_UPDATER_FIXTURE").is_some()
    }
    pub(super) fn configure(app: &tauri::AppHandle, updater: Updater) -> Result<Updater, String> {
        if !enabled(app) {
            return Ok(updater);
        }
        let path = std::path::PathBuf::from(
            std::env::var_os("MURMUR_UPDATER_FIXTURE").ok_or("Updater fixture missing")?,
        );
        let bytes = std::fs::read(path).map_err(|_| "Updater fixture unreadable")?;
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
pub(crate) fn fixture_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("updater-test")
        .invoke_handler(tauri::generate_handler![updater_test_run])
        .build()
}
#[cfg(feature = "shutdown-test-support")]
#[tauri::command]
async fn updater_test_run(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: Action,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    if !fixture::enabled(&app) {
        return Err("Isolated updater fixture required".into());
    }
    updater_action(app, window, action).await
}

pub(crate) fn close_admission(app: &tauri::AppHandle) {
    if let Ok(updater) = owner(app) {
        updater.stop();
    }
}
pub(crate) fn shutdown_flush(app: &tauri::AppHandle, deadline: std::time::Instant) -> bool {
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
