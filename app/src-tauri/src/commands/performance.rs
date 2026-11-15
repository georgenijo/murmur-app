use crate::performance_metrics::{
    PerformanceRunListV1, PerformanceRunV1, PerformanceStoreHealthV1, ResourceSampleV1,
};
use crate::{MutexExt, State};
use tauri::{Emitter, Manager};

const DIAGNOSTICS_TABS: [&str; 6] = [
    "events",
    "runs",
    "performance",
    "latency",
    "reports",
    "transforms",
];

const DIAGNOSTICS_WINDOW_LABELS: [&str; 2] = ["main", "diagnostics"];

fn require_diagnostics_window(label: &str) -> Result<(), String> {
    if DIAGNOSTICS_WINDOW_LABELS.contains(&label) {
        Ok(())
    } else {
        Err(
            "Diagnostics store health is only available in an authorized Diagnostics view."
                .to_string(),
        )
    }
}

const DIAGNOSTICS_LABEL: &str = "diagnostics";

/// One request to show a Diagnostics tab. `seq` grows with each request, so a
/// page that hears about the same request twice (event and read) or in the
/// wrong order keeps the newest.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DiagnosticsTabRequest {
    tab: String,
    seq: u64,
}

/// The latest tab asked for, retained in Rust: the page may not be listening
/// yet when a request is emitted (a new window is still loading, or a second
/// request arrives while the first page hydrates). A page subscribes to the
/// event first, then reads this.
#[derive(Default)]
struct TabRequests(std::sync::Mutex<Option<DiagnosticsTabRequest>>);

impl TabRequests {
    fn request(&self, tab: &str) -> DiagnosticsTabRequest {
        let mut latest = self.0.lock_or_recover();
        let seq = latest.as_ref().map_or(1, |request| request.seq + 1);
        let request = DiagnosticsTabRequest {
            tab: tab.to_string(),
            seq,
        };
        *latest = Some(request.clone());
        request
    }

    fn latest(&self) -> Option<DiagnosticsTabRequest> {
        self.0.lock_or_recover().clone()
    }
}

static TAB_REQUESTS: TabRequests = TabRequests(std::sync::Mutex::new(None));

/// Diagnostics is built on first open, not at launch, and destroyed on close,
/// so its webview costs nothing until someone uses it.
fn diagnostics_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(DIAGNOSTICS_LABEL) {
        return Ok(window);
    }
    let unavailable = || "The Diagnostics window is unavailable.".to_string();
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == DIAGNOSTICS_LABEL)
        .cloned()
        .ok_or_else(unavailable)?;
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|error| {
            tracing::warn!(target: "system", %error, "could not configure Diagnostics window");
            unavailable()
        })?
        .build()
        .map_err(|error| {
            tracing::warn!(target: "system", %error, "could not create Diagnostics window");
            unavailable()
        })?;
    crate::commands::native_window::hide_titlebar_separator(&window);
    crate::window_state::restore_created(&window);
    Ok(window)
}

// Async: building a webview must not run inside a main-thread command.
#[tauri::command]
pub async fn show_diagnostics_window(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    if !DIAGNOSTICS_TABS.contains(&tab.as_str()) {
        return Err("Unknown diagnostics tab.".to_string());
    }
    // Record the request before anything is built or shown, so a page that
    // loads late reads it and a later request always carries a higher `seq`.
    let request = TAB_REQUESTS.request(&tab);
    let window = diagnostics_window(&app)?;
    window.show().map_err(|error| {
        tracing::warn!(target: "system", %error, "could not show Diagnostics window");
        "The Diagnostics window could not be opened.".to_string()
    })?;
    let _ = window.unminimize();
    let _ = window.set_focus();
    window
        .emit("diagnostics-tab-requested", request)
        .map_err(|error| {
            tracing::warn!(target: "system", %error, "could not select Diagnostics tab");
            "The Diagnostics tab could not be selected.".to_string()
        })
}

/// The newest tab request, for a Diagnostics page that subscribed to
/// `diagnostics-tab-requested` after it was emitted.
#[tauri::command]
pub fn get_requested_diagnostics_tab(
    window: tauri::WebviewWindow,
) -> Option<DiagnosticsTabRequest> {
    if window.label() != DIAGNOSTICS_LABEL {
        return None;
    }
    TAB_REQUESTS.latest()
}

/// Run a diagnostics-store read on the blocking pool. These reads open and scan
/// SQLite, which took ~60 ms on the main thread when the commands were
/// synchronous. Nothing here touches AppKit.
async fn on_blocking_pool<T: Send + 'static>(
    app: tauri::AppHandle,
    work: impl FnOnce(&State) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || work(&app.state::<State>()))
        .await
        .map_err(|_| "The diagnostics store read was interrupted.".to_string())?
}

#[tauri::command]
pub async fn list_performance_runs(
    limit: Option<u32>,
    app: tauri::AppHandle,
) -> Result<PerformanceRunListV1, String> {
    on_blocking_pool(app, move |state| {
        state.performance.list(limit.unwrap_or(50))
    })
    .await
}

#[tauri::command]
pub fn get_performance_run(
    run_id: String,
    state: tauri::State<'_, State>,
) -> Result<Option<PerformanceRunV1>, String> {
    state.performance.get(run_id.trim())
}

#[tauri::command]
pub async fn get_performance_resource_window(
    app: tauri::AppHandle,
) -> Result<Vec<ResourceSampleV1>, String> {
    on_blocking_pool(app, |state| state.performance.resource_window()).await
}

#[tauri::command]
pub fn get_performance_store_health(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<PerformanceStoreHealthV1, String> {
    require_diagnostics_window(window.label())?;
    Ok(state.performance.health())
}

#[tauri::command]
pub fn recover_performance_store(
    window: tauri::WebviewWindow,
    allow_reinitialize: bool,
    state: tauri::State<'_, State>,
) -> Result<PerformanceStoreHealthV1, String> {
    require_diagnostics_window(window.label())?;
    state.performance.recover(allow_reinitialize)
}

#[tauri::command]
pub fn clear_performance_diagnostics(state: tauri::State<'_, State>) -> Result<(), String> {
    state.performance.clear()?;
    state.capture_health.clear()
}

#[cfg(test)]
mod tests {
    use super::{
        require_diagnostics_window, TabRequests, DIAGNOSTICS_TABS, DIAGNOSTICS_WINDOW_LABELS,
    };

    #[test]
    fn the_latest_tab_request_is_retained_with_a_growing_sequence() {
        let requests = TabRequests::default();
        assert_eq!(requests.latest(), None);
        let first = requests.request("events");
        let second = requests.request("runs");
        assert!(second.seq > first.seq);
        assert_eq!(requests.latest(), Some(second));
    }

    #[test]
    fn pop_out_tab_allowlist_covers_every_diagnostics_view() {
        assert_eq!(
            DIAGNOSTICS_TABS,
            [
                "events",
                "runs",
                "performance",
                "latency",
                "reports",
                "transforms"
            ]
        );
    }

    #[test]
    fn performance_store_health_is_strictly_scoped_to_diagnostics_surfaces() {
        for label in DIAGNOSTICS_WINDOW_LABELS {
            assert!(require_diagnostics_window(label).is_ok());
        }
        for label in ["overlay", "transform-review", "log-viewer", "", "main-copy"] {
            assert!(
                require_diagnostics_window(label).is_err(),
                "unexpected Diagnostics store access for {label:?}"
            );
        }
    }
}
