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
    // Frontend messages may contain selected or dictated text. The structured
    // event boundary receives only a constant summary; stable event_code and
    // numeric correlation fields carry the operational meaning.
    drop(message);
    let event_code = event_code
        .as_deref()
        .and_then(crate::telemetry::canonical_event_code);
    match (level.to_uppercase().as_str(), transform_pass_id, event_code) {
        ("WARN", Some(transform_pass_id), Some(event_code)) => {
            tracing::warn!(target: "system", source = "frontend", transform_pass_id, event_code, "Frontend event")
        }
        ("ERROR", Some(transform_pass_id), Some(event_code)) => {
            tracing::error!(target: "system", source = "frontend", transform_pass_id, event_code, "Frontend event")
        }
        (_, Some(transform_pass_id), Some(event_code)) => {
            tracing::info!(target: "system", source = "frontend", transform_pass_id, event_code, "Frontend event")
        }
        ("WARN", Some(transform_pass_id), None) => {
            tracing::warn!(target: "system", source = "frontend", transform_pass_id, "Frontend event")
        }
        ("ERROR", Some(transform_pass_id), None) => {
            tracing::error!(target: "system", source = "frontend", transform_pass_id, "Frontend event")
        }
        (_, Some(transform_pass_id), None) => {
            tracing::info!(target: "system", source = "frontend", transform_pass_id, "Frontend event")
        }
        ("WARN", None, Some(event_code)) => {
            tracing::warn!(target: "system", source = "frontend", event_code, "Frontend event")
        }
        ("ERROR", None, Some(event_code)) => {
            tracing::error!(target: "system", source = "frontend", event_code, "Frontend event")
        }
        (_, None, Some(event_code)) => {
            tracing::info!(target: "system", source = "frontend", event_code, "Frontend event")
        }
        ("WARN", None, None) => {
            tracing::warn!(target: "system", source = "frontend", "Frontend event")
        }
        ("ERROR", None, None) => {
            tracing::error!(target: "system", source = "frontend", "Frontend event")
        }
        (_, None, None) => tracing::info!(target: "system", source = "frontend", "Frontend event"),
    }
}
