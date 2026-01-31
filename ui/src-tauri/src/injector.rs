use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Delay after setting clipboard before simulating paste (ms)
/// This allows macOS clipboard to sync and window focus to settle
const PRE_PASTE_DELAY_MS: u64 = 150;

/// Copy text to clipboard and optionally simulate paste
pub fn inject_text(text: &str, auto_paste: bool) -> Result<(), String> {
    eprintln!("[Injector] inject_text called with auto_paste={}, text_len={}", auto_paste, text.len());
    
    // Skip if text is empty
    if text.trim().is_empty() {
        eprintln!("[Injector] Text is empty, skipping");
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Copy transcription to clipboard
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
    eprintln!("[Injector] Text copied to clipboard successfully");

    // If auto-paste is disabled, we're done
    if !auto_paste {
        eprintln!("[Injector] Auto-paste disabled, returning");
        return Ok(());
    }

    // Check accessibility permission before attempting paste simulation
    let accessibility_enabled = is_accessibility_enabled();
    eprintln!("[Injector] Accessibility permission check: {}", accessibility_enabled);
    
    if !accessibility_enabled {
        // Don't error - text is in clipboard, user can paste manually
        eprintln!("[Injector] Accessibility permission not granted - text copied to clipboard only");
        return Ok(());
    }

    // Wait for clipboard to sync and window focus to settle
    eprintln!("[Injector] Waiting {}ms before paste simulation", PRE_PASTE_DELAY_MS);
    thread::sleep(Duration::from_millis(PRE_PASTE_DELAY_MS));

    // Simulate Cmd+V paste
    eprintln!("[Injector] Starting paste simulation...");
    let result = simulate_paste();
    eprintln!("[Injector] Paste simulation result: {:?}", result);
    result
}

/// Simulate Cmd+V keystroke using osascript (most reliable on macOS Sonoma/Sequoia)
fn simulate_paste() -> Result<(), String> {
    use std::process::Command;

    eprintln!("[Injector] Using osascript to simulate Cmd+V...");

    let output = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to keystroke "v" using command down"#)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        eprintln!("[Injector] Paste simulation completed successfully");
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
        let result = unsafe { AXIsProcessTrusted() };
        eprintln!("[Injector] AXIsProcessTrusted() returned: {}", result);
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
