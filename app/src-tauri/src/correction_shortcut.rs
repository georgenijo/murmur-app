use crate::{MutexExt, State};
use rdev::{EventType, Key};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone)]
struct ShortcutConfig {
    device_name: Option<String>,
}
static CONFIG: Mutex<Option<ShortcutConfig>> = Mutex::new(None);
static CHORD: Mutex<Chord> = Mutex::new(Chord {
    modifiers: 0,
    latched: false,
});

struct Chord {
    modifiers: u8,
    latched: bool,
}

impl Chord {
    fn handle(&mut self, event: &EventType) -> bool {
        let (key, pressed) = match event {
            EventType::KeyPress(key) => (*key, true),
            EventType::KeyRelease(key) => (*key, false),
            _ => return false,
        };
        let bit = match key {
            Key::MetaLeft => 1,
            Key::MetaRight => 2,
            Key::ShiftLeft => 4,
            Key::ShiftRight => 8,
            Key::ControlLeft => 16,
            Key::ControlRight => 32,
            Key::Alt => 64,
            Key::AltGr => 128,
            _ => 0,
        };
        if pressed {
            self.modifiers |= bit;
        } else {
            self.modifiers &= !bit;
        }
        if key != Key::KeyE {
            return false;
        }
        if !pressed {
            self.latched = false;
            return false;
        }
        if self.latched {
            return false;
        }
        self.latched = true;
        const COMMAND_KEYS: u8 = 0b0000_0011;
        const SHIFT_KEYS: u8 = 0b0000_1100;
        const OTHER_MODIFIERS: u8 = 0b1111_0000;
        self.modifiers & COMMAND_KEYS != 0
            && self.modifiers & SHIFT_KEYS != 0
            && self.modifiers & OTHER_MODIFIERS == 0
    }
}

pub(crate) fn enabled() -> bool {
    CONFIG.lock_or_recover().is_some()
}

#[tauri::command]
pub(crate) fn set_correction_shortcut(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    enabled: bool,
    device_name: Option<String>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("main_window_required".into());
    }
    *CONFIG.lock_or_recover() = enabled.then_some(ShortcutConfig { device_name });
    *CHORD.lock_or_recover() = Chord {
        modifiers: 0,
        latched: false,
    };
    if enabled {
        crate::keyboard::ensure_listener_thread_spawned(app_handle);
    }
    Ok(())
}

pub(crate) fn handle(app: &tauri::AppHandle, event: &EventType) {
    if !enabled() || !CHORD.lock_or_recover().handle(event) {
        return;
    }
    let Some(ShortcutConfig { device_name }) = CONFIG.lock_or_recover().clone() else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<State>();
        if crate::keyboard::is_app_disabled() {
            return;
        }
        let session = crate::transform_apply::session_snapshot(&state.app_state);
        let result = if let Some(session) =
            session.filter(|session| session.purpose.is_correction())
        {
            if state.app_state.transform_status() == crate::state::TransformStatus::Listening {
                crate::transform_flow::finish_transform_instruction(
                    app.clone(),
                    state,
                    session.transform_pass_id,
                )
                .await
            } else {
                Ok(())
            }
        } else {
            crate::transform_flow::begin_dictation_correction(app.clone(), state, device_name).await
        };
        if let Err(error) = result {
            let _ = app.emit_to("main", "correction-start-failed", error);
            let _ = crate::commands::overlay::show_main_window(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chord_requires_both_modifiers_and_latches_key_repeat() {
        let mut chord = Chord {
            modifiers: 0,
            latched: false,
        };
        assert!(!chord.handle(&EventType::KeyPress(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyRelease(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyPress(Key::MetaLeft)));
        assert!(!chord.handle(&EventType::KeyPress(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyRelease(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyPress(Key::ShiftLeft)));
        assert!(chord.handle(&EventType::KeyPress(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyPress(Key::KeyE)));
        assert!(!chord.handle(&EventType::KeyRelease(Key::KeyE)));
        assert!(chord.handle(&EventType::KeyPress(Key::KeyE)));
    }
    #[test]
    fn extra_modifiers_do_not_activate_correction() {
        let mut chord = Chord {
            modifiers: 0,
            latched: false,
        };
        chord.handle(&EventType::KeyPress(Key::MetaLeft));
        chord.handle(&EventType::KeyPress(Key::ShiftLeft));
        chord.handle(&EventType::KeyPress(Key::Alt));
        assert!(!chord.handle(&EventType::KeyPress(Key::KeyE)));
        chord.handle(&EventType::KeyRelease(Key::Alt));
        assert!(!chord.handle(&EventType::KeyPress(Key::KeyE)));
        chord.handle(&EventType::KeyRelease(Key::KeyE));
        assert!(chord.handle(&EventType::KeyPress(Key::KeyE)));
    }
}
