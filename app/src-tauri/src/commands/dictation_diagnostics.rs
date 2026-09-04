use crate::dictation_diagnostics::{
    DictationCaptureArmStatusV1, DictationDiagnosticCaptureSummaryV1, DictationDiagnosticCaptureV1,
};
use crate::State;

fn require_diagnostics_window(label: &str) -> Result<(), String> {
    if matches!(label, "main" | "diagnostics") {
        Ok(())
    } else {
        Err("dictation diagnostic captures are only available in Diagnostics".to_string())
    }
}

fn require_main_window(label: &str) -> Result<(), String> {
    (label == "main")
        .then_some(())
        .ok_or_else(|| "arm the next private capture from the main Murmur window".to_string())
}

#[tauri::command]
pub(crate) fn arm_next_dictation_diagnostic_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    state.dictation_diagnostics.arm_next()
}

#[tauri::command]
pub(crate) fn disarm_next_dictation_diagnostic_capture(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    Ok(state.dictation_diagnostics.disarm())
}

#[tauri::command]
pub(crate) fn get_dictation_diagnostic_capture_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_diagnostics_window(window.label())?;
    Ok(state.dictation_diagnostics.arm_status())
}

#[tauri::command]
pub(crate) fn list_dictation_diagnostic_captures(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<Vec<DictationDiagnosticCaptureSummaryV1>, String> {
    require_diagnostics_window(window.label())?;
    state.dictation_diagnostics.list_captures()
}

#[tauri::command]
pub(crate) fn get_dictation_diagnostic_capture(
    window: tauri::WebviewWindow,
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<Option<DictationDiagnosticCaptureV1>, String> {
    require_diagnostics_window(window.label())?;
    state.dictation_diagnostics.get_capture(capture_id.trim())
}

#[tauri::command]
pub(crate) fn delete_dictation_diagnostic_capture(
    window: tauri::WebviewWindow,
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_diagnostics_window(window.label())?;
    state
        .dictation_diagnostics
        .delete_capture(capture_id.trim())
}

#[tauri::command]
pub(crate) async fn upload_dictation_diagnostic_capture(
    window: tauri::WebviewWindow,
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_diagnostics_window(window.label())?;
    state
        .dictation_diagnostics
        .upload_capture(capture_id.trim())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_diagnostics_windows_can_access_private_captures() {
        assert!(require_diagnostics_window("main").is_ok());
        assert!(require_diagnostics_window("diagnostics").is_ok());
        for label in ["overlay", "transform-review", "query-review", ""] {
            assert!(require_diagnostics_window(label).is_err());
        }
    }

    #[test]
    fn only_main_window_can_arm_private_capture() {
        assert!(require_main_window("main").is_ok());
        for label in [
            "diagnostics",
            "overlay",
            "transform-review",
            "query-review",
            "",
        ] {
            assert!(require_main_window(label).is_err());
        }
    }
}
