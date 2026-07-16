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

/// Trigger the native macOS microphone permission prompt (TCC) in-flow.
///
/// Calls `AVCaptureDevice.requestAccessForMediaType:completionHandler:`. When the
/// status is `notDetermined` this shows the system dialog and registers the app in
/// the Microphone pane — without opening the device, so it cannot duck other apps'
/// audio (issue #177). When the status is already determined the completion fires
/// immediately and no dialog appears. Fire-and-forget: callers observe the outcome
/// by polling `check_microphone_permission_status`, so this never blocks on the
/// user's answer.
#[tauri::command]
pub fn request_microphone_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, Bool};
        use objc2_foundation::NSString;

        unsafe {
            let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
                return Err("AVCaptureDevice is unavailable".to_string());
            };
            // AVMediaTypeAudio == @"soun"
            let media = NSString::from_str("soun");
            // The completion handler runs on an arbitrary dispatch queue after the
            // user answers; the block only logs, all state reads go through the
            // polling commands.
            let handler = block2::RcBlock::new(|granted: Bool| {
                tracing::info!(
                    target: "system",
                    "microphone access request completed: granted={}",
                    granted.as_bool()
                );
            });
            let _: () = msg_send![
                cls,
                requestAccessForMediaType: &*media,
                completionHandler: &*handler
            ];
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// Reset this app's stale macOS Microphone TCC entry, then reopen the pane.
///
/// Mirrors `reset_accessibility_permission` for the case where the running build
/// still reports the microphone as denied/missing while System Settings lists the
/// app (common after rebuilding a dev bundle or moving the .app — the binary
/// signature changes and the TCC entry goes stale). Resets ONLY the current bundle
/// identifier via `tccutil reset Microphone <bundle-id>` — never all apps. macOS
/// re-prompts on next mic use afterward; this only clears the stale entry.
#[tauri::command]
pub fn reset_microphone_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle_id = current_bundle_identifier()
            .ok_or_else(|| "Could not determine the app's bundle identifier".to_string())?;

        tracing::info!(
            target: "system",
            "resetting Microphone TCC entry for bundle id {}",
            bundle_id
        );

        let output = std::process::Command::new("tccutil")
            .args(["reset", "Microphone", &bundle_id])
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

        return open_system_preference_pane("Privacy_Microphone");
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Microphone reset is only supported on macOS".to_string())
    }
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

/// Map a raw macOS `AVAuthorizationStatus` value to a banner-state string.
///
/// Pure helper so the status-enum -> banner-state mapping is unit-testable
/// without a live TCC database. The returned string is consumed by the frontend
/// permissions banner, which distinguishes a hard "denied" (show red, offer the
/// reset path) from a transient "notDetermined"/"unknown" state (do NOT show a
/// hard denied banner — see issue #190, false-negatives after a dev rebuild or
/// app move when TCC drops the entry and the status reads not-determined).
///
/// `AVAuthorizationStatus` (AVFoundation):
///   0 = notDetermined, 1 = restricted, 2 = denied, 3 = authorized.
/// Any unexpected value (e.g. a future/probe-failure sentinel) maps to "unknown"
/// rather than "denied" so a transient probe glitch never hard-fails the banner.
fn mic_status_to_banner_state(status: isize) -> &'static str {
    match status {
        3 => "granted",
        2 | 1 => "denied",
        0 => "notDetermined",
        _ => "unknown",
    }
}

/// Read the *current* microphone authorization status as a banner-state string.
///
/// Queries `AVCaptureDevice.authorizationStatus(for: .audio)` live at call-time
/// (never a cached value) and maps it via [`mic_status_to_banner_state`]. Unlike
/// the boolean [`check_microphone_permission`], this preserves the distinction
/// between a genuine "denied" and a transient "notDetermined"/"unknown" state so
/// the banner doesn't false-negative after a rebuild/move (issue #190).
///
/// Returns one of: "granted" | "denied" | "notDetermined" | "unknown".
/// Like the bool probe, this only reads TCC state — it never opens the device,
/// so it cannot duck other apps' audio (issue #177).
#[tauri::command]
pub fn check_microphone_permission_status() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyClass;
        use objc2_foundation::NSString;

        unsafe {
            let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
                // Class lookup failed: a probe glitch, not a real denial.
                return "unknown".to_string();
            };
            // AVMediaTypeAudio == @"soun"
            let media = NSString::from_str("soun");
            let status: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
            mic_status_to_banner_state(status).to_string()
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "granted".to_string()
    }
}

#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<String>, String> {
    audio::list_input_devices()
}

#[cfg(test)]
mod tests {
    use super::mic_status_to_banner_state;

    #[test]
    fn authorized_status_maps_to_granted() {
        // AVAuthorizationStatusAuthorized
        assert_eq!(mic_status_to_banner_state(3), "granted");
    }

    #[test]
    fn denied_and_restricted_map_to_denied() {
        // AVAuthorizationStatusDenied
        assert_eq!(mic_status_to_banner_state(2), "denied");
        // AVAuthorizationStatusRestricted (e.g. MDM/parental controls) — still a
        // genuine block, so it must read as denied, not a transient state.
        assert_eq!(mic_status_to_banner_state(1), "denied");
    }

    #[test]
    fn not_determined_is_not_a_hard_denial() {
        // AVAuthorizationStatusNotDetermined: TCC has no entry yet (common right
        // after a rebuild/move). Must NOT collapse to "denied" (issue #190).
        let state = mic_status_to_banner_state(0);
        assert_eq!(state, "notDetermined");
        assert_ne!(state, "denied");
    }

    #[test]
    fn unexpected_values_map_to_unknown_not_denied() {
        // Any future/sentinel value from a probe glitch must degrade to "unknown",
        // never a hard "denied" — we never want to false-negative the banner.
        for v in [-1isize, 4, 99, isize::MAX, isize::MIN] {
            let state = mic_status_to_banner_state(v);
            assert_eq!(state, "unknown", "value {v} should map to unknown");
            assert_ne!(state, "denied", "value {v} must not map to denied");
        }
    }
}
