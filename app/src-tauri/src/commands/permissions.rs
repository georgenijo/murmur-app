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

/// Read the running process's bundle identifier (macOS).
///
/// Returns the *runtime* bundle id (e.g. the dev bundle `Local Dictation Dev`),
/// which is what TCC actually keys Accessibility entries on — not the static
/// identifier from `tauri.conf.json`, which can be stale for rebuilt/dev bundles.
#[cfg(target_os = "macos")]
fn current_bundle_identifier() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    unsafe {
        let cls = AnyClass::get(c"NSBundle")?;
        let bundle: *mut AnyObject = msg_send![cls, mainBundle];
        if bundle.is_null() {
            return None;
        }
        let ident: *const NSString = msg_send![bundle, bundleIdentifier];
        if ident.is_null() {
            return None;
        }
        let s = (*ident).to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Reset this app's stale macOS Accessibility TCC entry, then reopen the pane.
///
/// Troubleshooting action for the case where System Settings lists the app under
/// Accessibility but the running build still reports access missing (common after
/// rebuilding a dev bundle). Resets ONLY the current bundle identifier via
/// `tccutil reset Accessibility <bundle-id>` — never all apps. macOS still requires
/// the user to re-enable the app manually afterward; this only clears the stale entry.
#[tauri::command]
pub fn reset_accessibility_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle_id = current_bundle_identifier()
            .ok_or_else(|| "Could not determine the app's bundle identifier".to_string())?;

        tracing::info!(
            target: "system",
            "resetting Accessibility TCC entry for bundle id {}",
            bundle_id
        );

        let output = std::process::Command::new("tccutil")
            .args(["reset", "Accessibility", &bundle_id])
            .output()
            .map_err(|e| format!("Failed to run tccutil: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                target: "system",
                "tccutil reset failed for {}: {}",
                bundle_id,
                stderr.trim()
            );
            return Err(format!("tccutil reset failed: {}", stderr.trim()));
        }

        return open_system_preference_pane("Privacy_Accessibility");
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Accessibility reset is only supported on macOS".to_string())
    }
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
