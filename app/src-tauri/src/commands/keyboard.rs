use crate::{injector, keyboard, MutexExt};
use std::sync::Mutex;
use tauri::Emitter;

#[derive(Default)]
struct ListenerRegistration(Option<(String, String)>);

impl ListenerRegistration {
    fn sync(&mut self, hotkey: &str, mode: &str, active: bool) -> bool {
        if active
            && self
                .0
                .as_ref()
                .is_some_and(|(key, active_mode)| key == hotkey && active_mode == mode)
        {
            return false;
        }
        self.0 = Some((hotkey.into(), mode.into()));
        true
    }
}

static REGISTRATION: Mutex<ListenerRegistration> = Mutex::new(ListenerRegistration(None));

fn validate_dictation_key(hotkey: &str) -> Result<(), String> {
    if keyboard::is_dictation_key_id(hotkey) {
        Ok(())
    } else {
        Err("Unsupported dictation key. Choose one from Settings.".to_string())
    }
}

fn validate_query_key(hotkey: &str) -> Result<(), String> {
    if keyboard::is_query_key_id(hotkey) {
        Ok(())
    } else {
        Err("Unsupported shortcut key. Choose one from Settings.".to_string())
    }
}

#[tauri::command]
pub fn start_keyboard_listener(
    app_handle: tauri::AppHandle,
    hotkey: String,
    mode: String,
) -> Result<(), String> {
    const VALID_MODES: &[&str] = &["double_tap", "hold_down", "both"];
    if !VALID_MODES.contains(&mode.as_str()) {
        tracing::error!(target: "keyboard", "Invalid keyboard listener mode: {}", mode);
        return Err(format!(
            "Invalid mode '{}'. Expected one of: {}",
            mode,
            VALID_MODES.join(", ")
        ));
    }
    validate_dictation_key(&hotkey)?;
    if !injector::is_accessibility_enabled() {
        return Err(
            "Accessibility permission is required. Please grant it in System Settings.".to_string(),
        );
    }
    let mut registration = REGISTRATION.lock_or_recover();
    if registration.sync(&hotkey, &mode, keyboard::dictation_listener_active()) {
        keyboard::start_listener(app_handle, &hotkey, &mode);
    } else {
        // Reattach after a webview remount without resetting an in-flight hold
        // or double-tap. This also retries a failed shared rdev thread.
        keyboard::ensure_listener_thread_spawned(app_handle);
    }
    tracing::info!(target: "keyboard", "Keyboard listener started: mode={}, key={}, accessibility={}", mode, hotkey, injector::is_accessibility_enabled());
    Ok(())
}

#[tauri::command]
pub fn stop_keyboard_listener() {
    let mut registration = REGISTRATION.lock_or_recover();
    registration.0 = None;
    keyboard::stop_listener();
    tracing::info!(target: "keyboard", "Keyboard listener stopped: accessibility={}", injector::is_accessibility_enabled());
}

#[tauri::command]
pub fn update_keyboard_key(app_handle: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    validate_dictation_key(&hotkey)?;
    let mut registration = REGISTRATION.lock_or_recover();
    let should_stop = keyboard::set_target_key(&hotkey);
    if let Some((key, _)) = registration.0.as_mut() {
        *key = hotkey.clone();
    }
    if should_stop {
        let _ = app_handle.emit("hold-down-stop", ());
        tracing::info!(target: "keyboard", "Keyboard key changed while held — emitted stop; updated to: {}", hotkey);
    } else {
        tracing::info!(target: "keyboard", "Keyboard key updated to: {}", hotkey);
    }
    Ok(())
}

#[tauri::command]
pub fn set_keyboard_recording(recording: bool) {
    keyboard::set_recording_state(recording);
}

#[tauri::command]
pub fn set_app_disabled(app_handle: tauri::AppHandle, disabled: bool) -> Result<(), String> {
    if disabled {
        crate::smart_auto_probe::preempt();
        crate::audio_lifecycle::retire_standby();
    }
    keyboard::set_app_disabled(disabled);
    tracing::info!(target: "keyboard", "set_app_disabled: {}", disabled);
    sync_tray_disabled_item(disabled);
    app_handle
        .emit("app-disabled-changed", disabled)
        .map_err(|e| e.to_string())
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

#[tauri::command]
pub fn set_paste_last_shortcut(
    app_handle: tauri::AppHandle,
    shortcut: Option<String>,
) -> Result<(), String> {
    let Some(shortcut) = shortcut else {
        keyboard::stop_paste_last_listener();
        return Ok(());
    };

    if keyboard::is_dictation_key_id(&shortcut) {
        return Err("That shortcut is already assigned to Dictation.".to_string());
    }

    if keyboard::key_conflicts_with_query(&shortcut) {
        return Err("That shortcut is already assigned to Voice Query.".to_string());
    }
    if shortcut == "command_shift_p" {
        return Err("That shortcut opens Murmur's command palette.".to_string());
    }
    if shortcut == "command_comma" {
        return Err("That shortcut opens Murmur Settings.".to_string());
    }
    if !keyboard::is_paste_last_shortcut_id(&shortcut) {
        return Err("Unsupported Paste Last shortcut. Choose one from Settings.".to_string());
    }
    if !injector::is_accessibility_enabled() {
        return Err(
            "Accessibility permission is required for the Paste Last shortcut.".to_string(),
        );
    }
    // `start_paste_last_listener` validates before mutating its active target,
    // so an unsupported id or registration failure preserves the old binding.
    keyboard::start_paste_last_listener(app_handle, &shortcut)
}

// -- Voice-query hotkey (#538) --

#[tauri::command]
pub fn start_query_listener(app_handle: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    if keyboard::is_dictation_key_id(&hotkey) {
        return Err("That key is reserved for dictation.".to_string());
    }

    validate_query_key(&hotkey)?;
    if !injector::is_accessibility_enabled() {
        return Err(
            "Accessibility permission is required. Please grant it in System Settings.".to_string(),
        );
    }
    keyboard::start_query_listener(app_handle, &hotkey);
    tracing::info!(target: "keyboard", "Voice-query listener started");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn frontend_registration_is_idempotent_and_restarts_after_stop_or_change() {
        let mut registration = super::ListenerRegistration::default();
        assert!(registration.sync("shift_l", "hold_down", true));
        assert!(!registration.sync("shift_l", "hold_down", true));
        assert!(
            registration.sync("shift_l", "hold_down", false),
            "a failed listener must re-arm on retry"
        );
        assert!(registration.sync("alt_l", "hold_down", true));
        assert!(registration.sync("alt_l", "both", true));
        registration.0 = None;
        assert!(registration.sync("alt_l", "both", true));
    }
    use super::{validate_dictation_key, validate_query_key};

    #[test]
    fn dictation_ipc_accepts_only_picker_keys() {
        for key in ["shift_l", "alt_l", "ctrl_r", "f1", "f10", "f20"] {
            assert!(validate_dictation_key(key).is_ok(), "{key}");
        }
        for key in ["", "f0", "f21", "shift_r", "space", "mouse_4"] {
            assert!(validate_dictation_key(key).is_err(), "{key}");
        }
    }

    #[test]
    fn transform_and_query_keep_their_opposite_side_modifier_set() {
        for key in ["shift_r", "alt_r", "ctrl_l"] {
            assert!(validate_query_key(key).is_ok(), "{key}");
        }
        for key in ["shift_l", "alt_l", "ctrl_r", "f1", "f20", "mouse_4"] {
            assert!(validate_query_key(key).is_err(), "{key}");
        }
    }
}

#[tauri::command]
pub fn stop_query_listener() {
    keyboard::stop_query_listener();
    tracing::info!(target: "keyboard", "Voice-query listener stopped");
}
