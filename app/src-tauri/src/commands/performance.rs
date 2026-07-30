use crate::performance_metrics::{PerformanceRunListV1, PerformanceRunV1, ResourceSampleV1};
use crate::State;

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
    state.performance.clear()
}
