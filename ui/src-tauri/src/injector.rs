use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Delay after setting clipboard before simulating paste (ms)
/// This allows macOS clipboard to sync and window focus to settle
const PRE_PASTE_DELAY_MS: u64 = 150;

/// Copy text to clipboard and optionally simulate paste
pub fn inject_text(text: &str, auto_paste: bool) -> Result<(), String> {
    log_info!("inject_text called with auto_paste={}, text_len={}", auto_paste, text.len());

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

    // Wait for clipboard to sync and window focus to settle
    thread::sleep(Duration::from_millis(PRE_PASTE_DELAY_MS));

    // Simulate Cmd+V paste
    simulate_paste()
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
