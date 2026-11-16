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
    event_code: String,
    updater_version: Option<String>,
    updater_selection: Option<String>,
) {
    // The frontend sends only an allowlisted event code and, for updater
    // events, a validated version and selection. Free text never crosses this
    // boundary because it may contain selected or dictated text; any other
    // event is dropped.
    if event_code == "system.launch_frontend_hydrated" {
        crate::launch::frontend_hydrated();
        return;
    }
    let Some(event_code) = crate::telemetry::canonical_event_code(&event_code) else {
        return;
    };
    if event_code.starts_with("updater.") {
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
    match level.to_uppercase().as_str() {
        "WARN" => {
            tracing::warn!(target: "system", source = "frontend", event_code, "Frontend event")
        }
        "ERROR" => {
            tracing::error!(target: "system", source = "frontend", event_code, "Frontend event")
        }
        _ => tracing::info!(target: "system", source = "frontend", event_code, "Frontend event"),
    }
}
