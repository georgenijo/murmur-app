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
pub fn log_frontend(level: String, message: String, transform_pass_id: Option<u64>) {
    match (level.to_uppercase().as_str(), transform_pass_id) {
        ("WARN", Some(transform_pass_id)) => {
            tracing::warn!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        ("ERROR", Some(transform_pass_id)) => {
            tracing::error!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        (_, Some(transform_pass_id)) => {
            tracing::info!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        ("WARN", None) => tracing::warn!(target: "system", source = "frontend", "{}", message),
        ("ERROR", None) => tracing::error!(target: "system", source = "frontend", "{}", message),
        (_, None) => tracing::info!(target: "system", source = "frontend", "{}", message),
    }
}
