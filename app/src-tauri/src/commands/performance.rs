use crate::performance_metrics::{PerformanceRunListV1, PerformanceRunV1, ResourceSampleV1};
use crate::State;
use tauri::{Emitter, Manager};

const DIAGNOSTICS_TABS: [&str; 6] = [
    "events",
    "runs",
    "performance",
    "latency",
    "reports",
    "transforms",
];

#[tauri::command]
pub fn show_diagnostics_window(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    if !DIAGNOSTICS_TABS.contains(&tab.as_str()) {
        return Err("Unknown diagnostics tab.".to_string());
    }
    let Some(window) = app.get_webview_window("diagnostics") else {
        return Err("The Diagnostics window is unavailable.".to_string());
    };
    window.show().map_err(|error| {
        tracing::warn!(target: "system", %error, "could not show Diagnostics window");
        "The Diagnostics window could not be opened.".to_string()
    })?;
    let _ = window.unminimize();
    let _ = window.set_focus();
    window
        .emit("diagnostics-tab-requested", tab)
        .map_err(|error| {
            tracing::warn!(target: "system", %error, "could not select Diagnostics tab");
            "The Diagnostics tab could not be selected.".to_string()
        })
}

#[tauri::command]
pub fn list_performance_runs(
    limit: Option<u32>,
    state: tauri::State<'_, State>,
) -> Result<PerformanceRunListV1, String> {
    state.performance.list(limit.unwrap_or(50))
}

#[tauri::command]
pub fn get_performance_run(
    run_id: String,
    state: tauri::State<'_, State>,
) -> Result<Option<PerformanceRunV1>, String> {
    state.performance.get(run_id.trim())
}

#[tauri::command]
pub fn get_performance_resource_window(
    state: tauri::State<'_, State>,
) -> Result<Vec<ResourceSampleV1>, String> {
    state.performance.resource_window()
}

#[tauri::command]
pub fn clear_performance_diagnostics(state: tauri::State<'_, State>) -> Result<(), String> {
    state.performance.clear()?;
    state.capture_health.clear()
}

#[cfg(test)]
mod tests {
    use super::DIAGNOSTICS_TABS;

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
}
