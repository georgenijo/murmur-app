use crate::transform_diagnostics::{
    CaptureArmStatusV1, DiagnosticCaptureSummaryV1, DiagnosticCaptureV1, TransformAttemptListV1,
};
use crate::State;

const MAIN_WINDOW_LABEL: &str = "main";

fn require_main_window(label: &str) -> Result<(), String> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("transform diagnostics are only available in the main window".to_string())
    }
}

#[tauri::command]
pub fn arm_next_transform_diagnostic_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<CaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    state.transform_diagnostics.arm_next()
}

#[tauri::command]
pub fn get_transform_diagnostic_capture_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<CaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    Ok(state.transform_diagnostics.arm_status())
}

#[tauri::command]
pub fn list_transform_attempts(
    window: tauri::WebviewWindow,
    limit: Option<usize>,
    state: tauri::State<'_, State>,
) -> Result<TransformAttemptListV1, String> {
    require_main_window(window.label())?;
    Ok(state
        .transform_diagnostics
        .list_attempts(limit.unwrap_or(50)))
}

#[tauri::command]
pub fn list_transform_diagnostic_captures(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<Vec<DiagnosticCaptureSummaryV1>, String> {
    require_main_window(window.label())?;
    state.transform_diagnostics.list_captures()
}

#[tauri::command]
pub fn get_transform_diagnostic_capture(
    window: tauri::WebviewWindow,
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<Option<DiagnosticCaptureV1>, String> {
    require_main_window(window.label())?;
    state.transform_diagnostics.get_capture(capture_id.trim())
}

#[tauri::command]
pub fn delete_transform_diagnostic_capture(
    window: tauri::WebviewWindow,
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_main_window(window.label())?;
    state
        .transform_diagnostics
        .delete_capture(capture_id.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_diagnostics_are_strictly_scoped_to_main_window() {
        assert!(require_main_window(MAIN_WINDOW_LABEL).is_ok());
        for label in ["log-viewer", "transform-review", "overlay", "", "main-copy"] {
            assert!(
                require_main_window(label).is_err(),
                "unexpected diagnostics access for {label:?}"
            );
        }
    }
}
