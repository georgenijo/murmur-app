use tauri::Manager;

#[tauri::command]
pub fn get_log_contents(lines: usize) -> String {
    crate::telemetry::read_pretty_log_tail(lines)
}

#[tauri::command]
pub fn clear_logs() -> Result<(), String> {
    crate::telemetry::clear_all_logs()?;
    crate::telemetry::clear_event_history();
    Ok(())
}

#[tauri::command]
pub fn log_frontend(level: String, message: String) {
    match level.to_uppercase().as_str() {
        "WARN" => tracing::warn!(target: "system", source = "frontend", "{}", message),
        "ERROR" => tracing::error!(target: "system", source = "frontend", "{}", message),
        _ => tracing::info!(target: "system", source = "frontend", "{}", message),
    }
}

#[tauri::command]
pub fn open_log_viewer(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("log-viewer")
        .ok_or_else(|| "log-viewer window is not configured".to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}
