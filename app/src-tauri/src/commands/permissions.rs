use crate::{audio, injector};

#[cfg(target_os = "macos")]
fn open_system_preference_pane(pane: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{}",
            pane
        ))
        .spawn()
        .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn open_system_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { return open_system_preference_pane("Privacy_Microphone"); }
    #[cfg(not(target_os = "macos"))]
    { Err("System preferences shortcut not supported on this platform".to_string()) }
}

/// Check if accessibility permission is granted (macOS)
#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    injector::is_accessibility_enabled()
}

/// Request accessibility permission (triggers system prompt + opens System Settings on macOS)
#[tauri::command]
pub fn request_accessibility_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Trigger the system dialog and register the app in the Accessibility list.
        // Return value is the current trust status — we proceed to open System Settings
        // regardless, so the result is intentionally discarded here.
        let _ = injector::request_accessibility_prompt();
        return open_system_preference_pane("Privacy_Accessibility");
    }
    #[cfg(not(target_os = "macos"))]
    { Ok(()) }
}

/// Request microphone permission (opens System Settings on macOS)
#[tauri::command]
pub fn request_microphone_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { return open_system_preference_pane("Privacy_Microphone"); }
    #[cfg(not(target_os = "macos"))]
    { Ok(()) }
}

/// Check microphone authorization status WITHOUT opening the device.
///
/// Reads `AVCaptureDevice.authorizationStatus(for: .audio)`. This only queries
/// TCC state — it never instantiates a capture/voice-processing audio unit, so
/// it cannot duck other apps' audio. (Issue #177: the previous getUserMedia probe
/// opened VPIO on every window focus, ducking other system audio each time.)
#[tauri::command]
pub fn check_microphone_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyClass;
        use objc2_foundation::NSString;

        // AVAuthorizationStatusAuthorized == 3
        const AUTHORIZED: isize = 3;
        unsafe {
            let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
                return false;
            };
            // AVMediaTypeAudio == @"soun"
            let media = NSString::from_str("soun");
            let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
            status == AUTHORIZED
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<String>, String> {
    audio::list_input_devices()
}
