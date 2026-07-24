use crate::transform_diagnostics::{
    CaptureArmStatusV1, DiagnosticCaptureSummaryV1, DiagnosticCaptureV1, TransformAttemptListV1,
};
use crate::State;

#[tauri::command]
pub fn arm_next_transform_diagnostic_capture(
    state: tauri::State<'_, State>,
) -> Result<CaptureArmStatusV1, String> {
    state.transform_diagnostics.arm_next()
}

#[tauri::command]
pub fn get_transform_diagnostic_capture_status(
    state: tauri::State<'_, State>,
) -> CaptureArmStatusV1 {
    state.transform_diagnostics.arm_status()
}

#[tauri::command]
pub fn list_transform_attempts(
    limit: Option<usize>,
    state: tauri::State<'_, State>,
) -> TransformAttemptListV1 {
    state
        .transform_diagnostics
        .list_attempts(limit.unwrap_or(50))
}

#[tauri::command]
pub fn list_transform_diagnostic_captures(
    state: tauri::State<'_, State>,
) -> Result<Vec<DiagnosticCaptureSummaryV1>, String> {
    state.transform_diagnostics.list_captures()
}

#[tauri::command]
pub fn get_transform_diagnostic_capture(
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<Option<DiagnosticCaptureV1>, String> {
    state.transform_diagnostics.get_capture(capture_id.trim())
}

#[tauri::command]
pub fn delete_transform_diagnostic_capture(
    capture_id: String,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    state
        .transform_diagnostics
        .delete_capture(capture_id.trim())
}
