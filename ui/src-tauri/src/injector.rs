use arboard::Clipboard;
use enigo::{Direction::{Click, Press, Release}, Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

/// Check if accessibility permission is granted (macOS)
pub fn is_accessibility_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        // SAFETY: AXIsProcessTrusted is a stable macOS API that queries
        // accessibility permission status without requiring preconditions
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Inject text by copying to clipboard and simulating Cmd+V
pub fn inject_text(text: &str) -> Result<(), String> {
    // Skip if text is empty
    if text.trim().is_empty() {
        return Ok(());
    }

    // Copy text to clipboard first (always works)
    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;

    // Check accessibility permission BEFORE calling enigo to avoid the popup
    if !is_accessibility_enabled() {
        println!("[Injector] Accessibility permission not granted - text copied to clipboard only");
        return Err("Accessibility permission required for auto-paste. Text has been copied to clipboard - press Cmd+V to paste manually.".to_string());
    }

    // Small delay for clipboard to be ready
    thread::sleep(Duration::from_millis(50));

    // Simulate Cmd+V
    simulate_paste()
}

/// Simulate Cmd+V paste keystroke
fn simulate_paste() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("Failed to initialize keyboard simulation (check Accessibility permissions): {}", e))?;

    // Press Meta (Cmd), tap V, release Meta
    enigo.key(Key::Meta, Press)
        .map_err(|e| format!("Failed to press Cmd key: {}", e))?;

    enigo.key(Key::Unicode('v'), Click)
        .map_err(|e| format!("Failed to press V key: {}", e))?;

    enigo.key(Key::Meta, Release)
        .map_err(|e| format!("Failed to release Cmd key: {}", e))?;

    // Small delay to ensure paste completes
    thread::sleep(Duration::from_millis(50));

    Ok(())
}
