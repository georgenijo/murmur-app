pub fn get_log_contents(lines: usize) -> String {
    crate::telemetry::read_pretty_log_tail(lines)
}

pub async fn clear_logs() -> Result<(), String> {
    // Invalidate scheduled restoration before waiting on its disk worker.
    crate::telemetry::clear_event_history();
    crate::host::runtime::spawn_blocking(crate::telemetry::clear_all_logs)
        .await
        .map_err(|_| "Log worker unavailable".to_string())?
}

pub fn log_frontend(
    level: String,
    message: String,
    transform_pass_id: Option<u64>,
    event_code: Option<String>,
    updater_version: Option<String>,
    updater_selection: Option<String>,
) {
    // Frontend messages may contain selected or dictated text. The structured
    // event boundary receives only a constant summary; stable event_code and
    // numeric correlation fields carry the operational meaning.
    drop(message);
    if event_code.as_deref() == Some("system.launch_frontend_hydrated") {
        crate::launch::frontend_hydrated();
        return;
    }
    let event_code = event_code
        .as_deref()
        .and_then(crate::telemetry::canonical_event_code);
    if let Some(event_code) = event_code.filter(|code| code.starts_with("updater.")) {
        if let Some(version) = updater_version
            .as_deref()
            .filter(|version| crate::telemetry::is_safe_updater_version(version))
        {
            let selection = match updater_selection.as_deref() {
                Some("retained") => "retained",
                Some("fresh_check") => "fresh_check",
                _ => "none",
            };
            match level.to_uppercase().as_str() {
                "WARN" => {
                    tracing::warn!(target: "system", source = "frontend", event_code, version, selection, "Frontend event")
                }
                "ERROR" => {
                    tracing::error!(target: "system", source = "frontend", event_code, version, selection, "Frontend event")
                }
                _ => {
                    tracing::info!(target: "system", source = "frontend", event_code, version, selection, "Frontend event")
                }
            }
            return;
        }
    }
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
