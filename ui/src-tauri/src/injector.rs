use arboard::Clipboard;

/// Copy text to clipboard for user to paste manually
pub fn inject_text(text: &str) -> Result<(), String> {
    // Skip if text is empty
    if text.trim().is_empty() {
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Copy transcription to clipboard
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;

    Ok(())
}

/// Check if accessibility permission is granted (macOS)
/// Kept for potential future use (e.g., if user wants auto-paste option)
#[allow(dead_code)]
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
