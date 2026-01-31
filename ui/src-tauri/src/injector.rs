use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Delay after setting clipboard before simulating paste (ms)
/// This allows macOS clipboard to sync and window focus to settle
const PRE_PASTE_DELAY_MS: u64 = 150;
/// Delay between key events to let macOS process them
const KEY_EVENT_DELAY_MS: u64 = 20;

/// Copy text to clipboard and optionally simulate paste
pub fn inject_text(text: &str, auto_paste: bool) -> Result<(), String> {
    // Skip if text is empty
    if text.trim().is_empty() {
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Copy transcription to clipboard
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;

    // If auto-paste is disabled, we're done
    if !auto_paste {
        return Ok(());
    }

    // Check accessibility permission before attempting paste simulation
    if !is_accessibility_enabled() {
        // Don't error - text is in clipboard, user can paste manually
        eprintln!("[Injector] Accessibility permission not granted - text copied to clipboard only");
        return Ok(());
    }

    // Wait for clipboard to sync and window focus to settle
    thread::sleep(Duration::from_millis(PRE_PASTE_DELAY_MS));

    // Simulate Cmd+V paste
    simulate_paste()
}

/// Simulate Cmd+V keystroke using rdev
fn simulate_paste() -> Result<(), String> {
    use rdev::{simulate, EventType, Key};

    let delay = Duration::from_millis(KEY_EVENT_DELAY_MS);

    // Press Command (Meta) key
    simulate(&EventType::KeyPress(Key::MetaLeft))
        .map_err(|e| format!("Failed to press Command key: {:?}", e))?;
    thread::sleep(delay);

    // Press V
    simulate(&EventType::KeyPress(Key::KeyV))
        .map_err(|e| format!("Failed to press V key: {:?}", e))?;
    thread::sleep(delay);

    // Release V
    simulate(&EventType::KeyRelease(Key::KeyV))
        .map_err(|e| format!("Failed to release V key: {:?}", e))?;
    thread::sleep(delay);

    // Release Command
    simulate(&EventType::KeyRelease(Key::MetaLeft))
        .map_err(|e| format!("Failed to release Command key: {:?}", e))?;

    Ok(())
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
