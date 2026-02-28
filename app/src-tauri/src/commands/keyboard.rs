use crate::{keyboard, injector};
use crate::{log_info, log_error};
use tauri::Emitter;

#[tauri::command]
pub fn start_keyboard_listener(app_handle: tauri::AppHandle, hotkey: String, mode: String) -> Result<(), String> {
    const VALID_MODES: &[&str] = &["double_tap", "hold_down", "both"];
    if !VALID_MODES.contains(&mode.as_str()) {
        log_error!("Invalid keyboard listener mode: {}", mode);
        return Err(format!("Invalid mode '{}'. Expected one of: {}", mode, VALID_MODES.join(", ")));
    }
    if !injector::is_accessibility_enabled() {
        return Err("Accessibility permission is required. Please grant it in System Settings.".to_string());
    }
    keyboard::start_listener(app_handle, &hotkey, &mode);
    log_info!("Keyboard listener started: mode={}, key={}, accessibility={}", mode, hotkey, injector::is_accessibility_enabled());
    Ok(())
}

#[tauri::command]
pub fn stop_keyboard_listener() {
    keyboard::stop_listener();
    log_info!("Keyboard listener stopped: accessibility={}", injector::is_accessibility_enabled());
}

#[tauri::command]
pub fn update_keyboard_key(app_handle: tauri::AppHandle, hotkey: String) {
    let should_stop = keyboard::set_target_key(&hotkey);
    if should_stop {
        let _ = app_handle.emit("hold-down-stop", ());
        log_info!("Keyboard key changed while held — emitted stop; updated to: {}", hotkey);
    } else {
        log_info!("Keyboard key updated to: {}", hotkey);
    }
}

#[tauri::command]
pub fn set_keyboard_recording(recording: bool) {
    keyboard::set_recording_state(recording);
}
