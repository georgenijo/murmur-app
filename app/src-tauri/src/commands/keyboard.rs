use crate::{injector, keyboard, MutexExt};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

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
        crate::dictation_control::send(&app_handle, murmur_core::dictation::Input::HoldStop);
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
    crate::dictation_control::set_disabled(&app_handle, disabled)
}

pub(crate) fn apply_app_disabled(
    app_handle: tauri::AppHandle,
    disabled: bool,
) -> Result<(), String> {
    if disabled {
        crate::smart_auto_probe::preempt();
        crate::audio_lifecycle::retire_standby();
    }
    let state = app_handle.state::<crate::State>();
    let reenabled = update_app_disabled(&state.app_state, disabled);
    if reenabled {
        crate::commands::recording::request_idle_model_preparation(&app_handle);
    }
    tracing::info!(target: "keyboard", "set_app_disabled: {}", disabled);
    sync_tray_disabled_item(&app_handle);
    app_handle
        .emit("app-disabled-changed", disabled)
        .map_err(|e| e.to_string())
}

// Shared with the worker regression; keep the epoch change synchronous with
// the authoritative disabled flag, before a re-enable can queue a replacement.
pub(crate) fn update_app_disabled(state: &crate::state::AppState, disabled: bool) -> bool {
    let was_disabled = keyboard::is_app_disabled();
    keyboard::set_app_disabled(disabled);
    if !was_disabled && disabled {
        state.idle_model_warmup.invalidate();
    }
    was_disabled && !disabled
}

static DISABLED_MENU_ITEM: std::sync::OnceLock<tauri::menu::CheckMenuItem<tauri::Wry>> =
    std::sync::OnceLock::new();

/// Called once from setup so the tray's "Disable Murmur" check item can be
/// kept in sync regardless of which surface flipped the state.
pub(crate) fn register_tray_disabled_item(item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    let _ = DISABLED_MENU_ITEM.set(item);
}

fn sync_tray_disabled_item(app: &tauri::AppHandle) {
    if DISABLED_MENU_ITEM.get().is_none() {
        return;
    }
    if let Some(context) = app.try_state::<murmur_core::context::AppContext>() {
        // Never synchronously wait for AppKit while holding configuration
        // ownership: a main-thread tray callback may need that same fence.
        let _ = context.main_thread.dispatch(Box::new(|| {
            if let Some(check) = DISABLED_MENU_ITEM.get() {
                let _ = check.set_checked(keyboard::is_app_disabled());
            }
        }));
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
    use super::validate_dictation_key;

    #[test]
    fn dictation_ipc_accepts_only_picker_keys() {
        for key in ["shift_l", "alt_l", "ctrl_r", "f1", "f10", "f20"] {
            assert!(validate_dictation_key(key).is_ok(), "{key}");
        }
        for key in ["", "f0", "f21", "shift_r", "space", "mouse_4"] {
            assert!(validate_dictation_key(key).is_err(), "{key}");
        }
    }
}
