use crate::{keyboard, injector};
use tauri::Emitter;

#[tauri::command]
pub fn start_keyboard_listener(app_handle: tauri::AppHandle, hotkey: String, mode: String) -> Result<(), String> {
    const VALID_MODES: &[&str] = &["double_tap", "hold_down", "both"];
    if !VALID_MODES.contains(&mode.as_str()) {
        tracing::error!(target: "keyboard", "Invalid keyboard listener mode: {}", mode);
        return Err(format!("Invalid mode '{}'. Expected one of: {}", mode, VALID_MODES.join(", ")));
    }
    if !injector::is_accessibility_enabled() {
        return Err("Accessibility permission is required. Please grant it in System Settings.".to_string());
    }
    keyboard::start_listener(app_handle, &hotkey, &mode);
    tracing::info!(target: "keyboard", "Keyboard listener started: mode={}, key={}, accessibility={}", mode, hotkey, injector::is_accessibility_enabled());
    Ok(())
}

#[tauri::command]
pub fn stop_keyboard_listener() {
    keyboard::stop_listener();
    tracing::info!(target: "keyboard", "Keyboard listener stopped: accessibility={}", injector::is_accessibility_enabled());
}

#[tauri::command]
pub fn update_keyboard_key(app_handle: tauri::AppHandle, hotkey: String) {
    let should_stop = keyboard::set_target_key(&hotkey);
    if should_stop {
        let _ = app_handle.emit("hold-down-stop", ());
        tracing::info!(target: "keyboard", "Keyboard key changed while held — emitted stop; updated to: {}", hotkey);
    } else {
        tracing::info!(target: "keyboard", "Keyboard key updated to: {}", hotkey);
    }
}

#[tauri::command]
pub fn set_keyboard_recording(recording: bool) {
    keyboard::set_recording_state(recording);
}

#[tauri::command]
pub fn set_app_disabled(app_handle: tauri::AppHandle, disabled: bool) -> Result<(), String> {
    keyboard::set_app_disabled(disabled);
    tracing::info!(target: "keyboard", "set_app_disabled: {}", disabled);
    sync_tray_disabled_item(disabled);
    app_handle.emit("app-disabled-changed", disabled).map_err(|e| e.to_string())
}

static DISABLED_MENU_ITEM: std::sync::OnceLock<tauri::menu::CheckMenuItem<tauri::Wry>> =
    std::sync::OnceLock::new();

/// Called once from setup so the tray's "Disable Murmur" check item can be
/// kept in sync regardless of which surface flipped the state.
pub(crate) fn register_tray_disabled_item(item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    let _ = DISABLED_MENU_ITEM.set(item);
}

fn sync_tray_disabled_item(disabled: bool) {
    if let Some(check) = DISABLED_MENU_ITEM.get() {
        let _ = check.set_checked(disabled);
    }
}

#[tauri::command]
pub fn get_app_disabled() -> bool {
    keyboard::is_app_disabled()
}

// -- Transform hotkey (issue #312, PR-B1) --
//
// A second, independent hold-down shortcut coexisting with the dictation
// listener on the same shared rdev thread (see `keyboard::TRANSFORM_DETECTOR`
// / `keyboard::ensure_listener_thread_spawned`). No mode parameter — the
// transform hotkey is always hold-down.

#[tauri::command]
pub fn start_transform_listener(app_handle: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    if keyboard::is_dictation_key_id(&hotkey) {
        tracing::error!(target: "keyboard", "start_transform_listener: rejected dictation key '{}'", hotkey);
        return Err(format!(
            "'{}' is reserved for the dictation hotkey and cannot be used as the transform hotkey.",
            hotkey
        ));
    }
    if !injector::is_accessibility_enabled() {
        return Err("Accessibility permission is required. Please grant it in System Settings.".to_string());
    }
    keyboard::start_transform_listener(app_handle, &hotkey);
    tracing::info!(target: "keyboard", "Transform listener started: key={}", hotkey);
    Ok(())
}

#[tauri::command]
pub fn stop_transform_listener() {
    keyboard::stop_transform_listener();
    tracing::info!(target: "keyboard", "Transform listener stopped");
}

#[tauri::command]
pub fn set_transform_key(app_handle: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    if keyboard::is_dictation_key_id(&hotkey) {
        tracing::error!(target: "keyboard", "set_transform_key: rejected dictation key '{}'", hotkey);
        return Err(format!(
            "'{}' is reserved for the dictation hotkey and cannot be used as the transform hotkey.",
            hotkey
        ));
    }
    let should_release = keyboard::set_transform_key(&hotkey);
    if should_release {
        if let Some((pass_id, elapsed_ms)) = keyboard::take_transform_hold_context() {
            crate::transform_trace::key_stop(pass_id, elapsed_ms, "key_reconfigured");
            let _ = app_handle.emit(
                "transform-key-released",
                serde_json::json!({ "transformPassId": pass_id }),
            );
        }
        tracing::info!(target: "keyboard", "Transform key changed while held — emitted released; updated to: {}", hotkey);
    } else {
        tracing::info!(target: "keyboard", "Transform key updated to: {}", hotkey);
    }
    Ok(())
}
