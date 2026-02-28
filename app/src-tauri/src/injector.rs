use crate::{log_info, log_warn};
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Copy text to clipboard and optionally simulate paste.
/// `delay_ms` controls the pause before simulating Cmd+V (window focus settling).
/// On paste failure, retries once after 100ms.
pub fn inject_text(text: &str, auto_paste: bool, delay_ms: u64) -> Result<(), String> {
    log_info!("inject_text called with auto_paste={}, delay_ms={}, text_len={}", auto_paste, delay_ms, text.len());

    // Skip if text is empty
    if text.trim().is_empty() {
        log_info!("inject_text: text is empty, skipping");
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Copy transcription to clipboard
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
    log_info!("inject_text: text copied to clipboard");

    // If auto-paste is disabled, we're done
    if !auto_paste {
        return Ok(());
    }

    // Check accessibility permission before attempting paste simulation
    if !is_accessibility_enabled() {
        // Don't error - text is in clipboard, user can paste manually
        log_warn!("inject_text: accessibility permission not granted — text in clipboard only");
        return Ok(());
    }

    // Wait for window focus to settle (clipboard write via NSPasteboard is synchronous)
    thread::sleep(Duration::from_millis(delay_ms));

    // Simulate Cmd+V paste, retry once on failure
    match simulate_paste() {
        Ok(()) => Ok(()),
        Err(first_err) => {
            log_warn!("inject_text: first paste attempt failed: {}, retrying in 100ms", first_err);
            thread::sleep(Duration::from_millis(100));
            simulate_paste().map_err(|retry_err| {
                format!("Auto-paste failed after retry: {}", retry_err)
            })
        }
    }
}

/// Simulate Cmd+V keystroke using osascript (most reliable on macOS Sonoma/Sequoia)
fn simulate_paste() -> Result<(), String> {
    use std::process::Command;

    log_info!("simulate_paste: using osascript to simulate Cmd+V");

    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        log_info!("simulate_paste: completed successfully");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("osascript failed: {}", stderr))
    }
}

/// Check if accessibility permission is granted (macOS)
pub fn is_accessibility_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Trigger the macOS accessibility permission prompt.
/// Registers the app in System Settings > Privacy & Security > Accessibility
/// and shows the system dialog. Returns current trust status.
#[cfg(target_os = "macos")]
pub fn request_accessibility_prompt() -> bool {
    use std::ffi::c_void;

    #[repr(C)]
    struct Opaque([u8; 0]);

    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: Opaque;
        static kCFTypeDictionaryValueCallBacks: Opaque;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const Opaque as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const Opaque as *const c_void,
        );
        if dict.is_null() {
            return false;
        }
        let trusted = AXIsProcessTrustedWithOptions(dict);
        CFRelease(dict);
        trusted
    }
}
