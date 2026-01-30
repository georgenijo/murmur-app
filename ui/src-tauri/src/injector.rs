use arboard::Clipboard;
use enigo::{Direction::{Click, Press, Release}, Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

/// Inject text by copying to clipboard and simulating Cmd+V
pub fn inject_text(text: &str) -> Result<(), String> {
    // Skip if text is empty
    if text.trim().is_empty() {
        return Ok(());
    }

    // Copy text to clipboard
    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    clipboard.set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;

    // Small delay for clipboard to be ready
    thread::sleep(Duration::from_millis(50));

    // Try to simulate Cmd+V with panic recovery
    let paste_result = std::panic::catch_unwind(|| {
        simulate_paste()
    });

    match paste_result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Text injection failed - please grant Accessibility permission in System Settings → Privacy & Security → Accessibility".to_string()),
    }
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
