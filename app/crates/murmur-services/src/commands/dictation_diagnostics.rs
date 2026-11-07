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

pub(crate) fn arm_next_dictation_diagnostic_capture(
    window: crate::host::Window,
    state: crate::host::StateRef<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    state.dictation_diagnostics.arm_next()
}

pub(crate) fn disarm_next_dictation_diagnostic_capture(
    window: crate::host::Window,
    state: crate::host::StateRef<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_main_window(window.label())?;
    Ok(state.dictation_diagnostics.disarm())
}

pub(crate) fn get_dictation_diagnostic_capture_status(
    window: crate::host::Window,
    state: crate::host::StateRef<'_, State>,
) -> Result<DictationCaptureArmStatusV1, String> {
    require_diagnostics_window(window.label())?;
    Ok(state.dictation_diagnostics.arm_status())
}

pub(crate) fn list_dictation_diagnostic_captures(
    window: crate::host::Window,
    state: crate::host::StateRef<'_, State>,
) -> Result<Vec<DictationDiagnosticCaptureSummaryV1>, String> {
    require_diagnostics_window(window.label())?;
    state.dictation_diagnostics.list_captures()
}

pub(crate) fn get_dictation_diagnostic_capture(
    window: crate::host::Window,
    capture_id: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<Option<DictationDiagnosticCaptureV1>, String> {
    require_diagnostics_window(window.label())?;
    state.dictation_diagnostics.get_capture(capture_id.trim())
}

pub(crate) fn delete_dictation_diagnostic_capture(
    window: crate::host::Window,
    capture_id: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<(), String> {
    require_diagnostics_window(window.label())?;
    state
        .dictation_diagnostics
        .delete_capture(capture_id.trim())
}

pub(crate) async fn upload_dictation_diagnostic_capture(
    window: crate::host::Window,
    capture_id: String,
    state: crate::host::StateRef<'_, State>,
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
        for label in ["overlay", "transform-review", "unrecognized-window", ""] {
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
            "unrecognized-window",
            "",
        ] {
            assert!(require_main_window(label).is_err());
        }
    }
}
