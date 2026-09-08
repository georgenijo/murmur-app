use crate::{MutexExt, State};
use rdev::{EventType, Key};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone)]
struct ShortcutConfig {
    device_name: Option<String>,
    smart_auto: Option<crate::microphone_auto::SmartAutoRequest>,
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
    smart_auto: Option<crate::microphone_auto::SmartAutoRequest>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("main_window_required".into());
    }
    *CONFIG.lock_or_recover() = enabled.then_some(ShortcutConfig {
        device_name,
        smart_auto,
    });
    *CHORD.lock_or_recover() = Chord {
        modifiers: 0,
        latched: false,
    };
    if enabled {
        crate::keyboard::ensure_listener_thread_spawned(app_handle);
    }
    Ok(())
}

/// What a correction-shortcut press should do, given the pass state the app is
/// in when the chord fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutAction {
    /// A correction pass is listening: this press is the "done speaking" half.
    Finish,
    /// Nothing owns the transform pipeline: start a new correction pass.
    Begin,
    /// Something else owns the pipeline — an ordinary transform pass, or a
    /// correction pass still arming (audio start plus the AX identity probe),
    /// thinking, or awaiting review. Swallow the press.
    ///
    /// Falling through to `begin_dictation_correction` here would fail its own
    /// `Finish or cancel the current transform first.` guard, and `handle`
    /// would answer a stray keystroke by popping the main window — stealing
    /// focus from the app the user is actually correcting text in. The main
    /// window is reserved for genuine "nothing to correct" / model-missing
    /// errors.
    Ignore,
}

fn decide(
    correction_session: bool,
    session_applied: bool,
    transform_status: crate::state::TransformStatus,
    transform_pass_active: bool,
) -> ShortcutAction {
    if correction_session && transform_status == crate::state::TransformStatus::Listening {
        return ShortcutAction::Finish;
    }
    // Approve/Copy drops the status straight back to Idle but keeps the pass ID
    // claimed and the session alive for the APPLIED_LINGER_MS Undo window.
    // Chaining a second correction onto the one just approved is an advertised
    // flow, so this press must reach `begin_dictation_correction`, which
    // supersedes the lingering pass the same way the hold key does. Only the
    // applied session distinguishes this from the startup window below, where a
    // pass has claimed its ID but not yet left Idle.
    if transform_status == crate::state::TransformStatus::Idle && session_applied {
        return ShortcutAction::Begin;
    }
    if transform_pass_active || transform_status != crate::state::TransformStatus::Idle {
        return ShortcutAction::Ignore;
    }
    ShortcutAction::Begin
}

pub(crate) fn handle(app: &tauri::AppHandle, event: &EventType) {
    if !enabled() || !CHORD.lock_or_recover().handle(event) {
        return;
    }
    let Some(ShortcutConfig {
        device_name,
        smart_auto,
    }) = CONFIG.lock_or_recover().clone()
    else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<State>();
        if crate::keyboard::is_app_disabled() {
            return;
        }
        let session = crate::transform_apply::session_snapshot(&state.app_state);
        // `purpose` is stamped as the session is installed (see
        // `transform_flow::start_capture`), so this read is accurate from the
        // first `Listening` emit onwards rather than only after the AX probe.
        let action = decide(
            session
                .as_ref()
                .is_some_and(|session| session.purpose.is_correction()),
            session.as_ref().is_some_and(|session| session.applied),
            state.app_state.transform_status(),
            state.app_state.active_transform_pass_id().is_some(),
        );
        let result = match action {
            ShortcutAction::Ignore => return,
            ShortcutAction::Finish => match session {
                Some(session) => {
                    crate::transform_flow::finish_transform_instruction(
                        app.clone(),
                        state,
                        session.transform_pass_id,
                    )
                    .await
                }
                None => return,
            },
            ShortcutAction::Begin => {
                crate::transform_flow::begin_dictation_correction(
                    app.clone(),
                    state,
                    device_name,
                    smart_auto,
                )
                .await
            }
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

    use crate::state::TransformStatus;

    #[test]
    fn listening_correction_press_finishes_the_instruction() {
        assert_eq!(
            decide(true, false, TransformStatus::Listening, true),
            ShortcutAction::Finish
        );
    }

    #[test]
    fn idle_press_begins_a_correction() {
        assert_eq!(
            decide(false, false, TransformStatus::Idle, false),
            ShortcutAction::Begin
        );
        // A leftover snapshot with no live pass still starts a fresh pass;
        // `begin_dictation_correction` re-checks the same two conditions.
        assert_eq!(
            decide(true, false, TransformStatus::Idle, false),
            ShortcutAction::Begin
        );
    }

    #[test]
    fn press_during_correction_startup_is_swallowed_not_surfaced() {
        // Between activate_transform_pass and the first Listening emit the
        // pass is Capturing/Connecting. A second chord here must not fall
        // through to begin_dictation_correction, whose refusal would pop the
        // main window over the app being corrected.
        for status in [
            TransformStatus::Capturing,
            TransformStatus::Connecting,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
        ] {
            assert_eq!(
                decide(true, false, status, true),
                ShortcutAction::Ignore,
                "{status:?}"
            );
        }
    }

    #[test]
    fn press_during_the_applied_linger_window_starts_a_chained_correction() {
        // Approve/Copy leaves status Idle with the pass ID still claimed for
        // APPLIED_LINGER_MS. begin_dictation_correction supersedes that pass,
        // so the press must not be swallowed — chained correction is the
        // advertised flow.
        assert_eq!(
            decide(true, true, TransformStatus::Idle, true),
            ShortcutAction::Begin
        );
        // The same applies after an ordinary selected-text transform was
        // approved: an applied review never blocks starting a correction.
        assert_eq!(
            decide(false, true, TransformStatus::Idle, true),
            ShortcutAction::Begin
        );
    }

    #[test]
    fn an_applied_session_does_not_unblock_a_mid_flight_pass() {
        // `applied` only means "linger" while the status is Idle. A live pass
        // still owns the pipeline whatever a stale session says.
        for status in [
            TransformStatus::Capturing,
            TransformStatus::Connecting,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
            TransformStatus::Applying,
        ] {
            assert_eq!(
                decide(true, true, status, true),
                ShortcutAction::Ignore,
                "{status:?}"
            );
        }
    }

    #[test]
    fn press_during_an_ordinary_transform_pass_is_swallowed() {
        for status in [
            TransformStatus::Capturing,
            TransformStatus::Connecting,
            TransformStatus::Listening,
            TransformStatus::Thinking,
            TransformStatus::ReviewPending,
        ] {
            assert_eq!(
                decide(false, false, status, true),
                ShortcutAction::Ignore,
                "{status:?}"
            );
        }
    }

    #[test]
    fn a_claimed_pass_id_alone_blocks_a_new_correction() {
        // `activate_transform_pass` runs before the status leaves Idle.
        assert_eq!(
            decide(false, false, TransformStatus::Idle, true),
            ShortcutAction::Ignore
        );
    }
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
