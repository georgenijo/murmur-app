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
pub fn log_frontend(
    level: String,
    message: String,
    transform_pass_id: Option<u64>,
    event_code: Option<String>,
) {
    let event_code = event_code
        .as_deref()
        .and_then(crate::telemetry::canonical_event_code);
    match (level.to_uppercase().as_str(), transform_pass_id, event_code) {
        ("WARN", Some(transform_pass_id), Some(event_code)) => {
            tracing::warn!(target: "system", source = "frontend", transform_pass_id, event_code, "{}", message)
        }
        ("ERROR", Some(transform_pass_id), Some(event_code)) => {
            tracing::error!(target: "system", source = "frontend", transform_pass_id, event_code, "{}", message)
        }
        (_, Some(transform_pass_id), Some(event_code)) => {
            tracing::info!(target: "system", source = "frontend", transform_pass_id, event_code, "{}", message)
        }
        ("WARN", Some(transform_pass_id), None) => {
            tracing::warn!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        ("ERROR", Some(transform_pass_id), None) => {
            tracing::error!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        (_, Some(transform_pass_id), None) => {
            tracing::info!(target: "system", source = "frontend", transform_pass_id, "{}", message)
        }
        ("WARN", None, Some(event_code)) => {
            tracing::warn!(target: "system", source = "frontend", event_code, "{}", message)
        }
        ("ERROR", None, Some(event_code)) => {
            tracing::error!(target: "system", source = "frontend", event_code, "{}", message)
        }
        (_, None, Some(event_code)) => {
            tracing::info!(target: "system", source = "frontend", event_code, "{}", message)
        }
        ("WARN", None, None) => {
            tracing::warn!(target: "system", source = "frontend", "{}", message)
        }
        ("ERROR", None, None) => {
            tracing::error!(target: "system", source = "frontend", "{}", message)
        }
        (_, None, None) => tracing::info!(target: "system", source = "frontend", "{}", message),
    }
}
