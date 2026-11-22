//! The agent-command key: an opt-in hold-to-talk key, separate from the
//! dictation key. Holding it records a spoken command for the user's coding
//! agents; releasing it finishes the recording.
//!
//! It shares the one rdev thread with dictation and Paste Last and publishes
//! three events to the main window:
//!
//! * `agent-command-start` once the key has been held alone for
//!   [`ARMING_MS`];
//! * `agent-command-stop` when it is released after that;
//! * `agent-command-cancel` when the hold turned out to be a shortcut (another
//!   key inside [`CHORD_WINDOW_MS`]), or the key was unbound or the app
//!   disabled mid-hold. The recording is discarded, never dispatched.
//!
//! A hold that ends inside the arming window publishes nothing.

use super::hold_promotion::HoldPromotion;
use super::is_modifier;
use crate::MutexExt;
use rdev::{EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::Emitter;

/// The keys the agent command may be bound to. Disjoint from
/// `DICTATION_KEY_IDS` by construction, so the two keys can never be the same
/// physical key, and none is a Paste Last chord or an app shortcut.
pub(super) const AGENT_COMMAND_KEY_IDS: &[&str] = &["alt_r", "shift_r", "ctrl_l"];

/// The key must be held alone this long before the recording starts. Longer
/// than the dictation key's 150 ms: Right Option types accented characters
/// and Right Shift types capitals, and a start here sends speech to an agent.
pub(super) const ARMING_MS: u128 = 250;

/// After the start, another non-modifier key inside this window means the
/// hold was a slow shortcut (Option held, then an arrow), not a command.
pub(super) const CHORD_WINDOW_MS: u128 = 1_000;

const _: () = assert!(ARMING_MS < CHORD_WINDOW_MS);

pub fn is_agent_command_key_id(id: &str) -> bool {
    AGENT_COMMAND_KEY_IDS.contains(&id)
}

/// What one key event means for the agent-command hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Signal {
    None,
    /// The key went down alone: start the arming timer.
    Press,
    /// The key was released while held.
    Release,
    /// The hold ended on another input: it was typing or a shortcut.
    Abort,
}

/// Pure hold detector. Time is passed in so tests need no sleeps.
#[derive(Debug, Default)]
pub(super) struct Detector {
    target: Option<Key>,
    held_since: Option<Instant>,
}

impl Detector {
    pub(super) fn set_target(&mut self, target: Option<Key>) {
        self.target = target;
        self.held_since = None;
    }

    pub(super) fn reset(&mut self) {
        self.held_since = None;
    }

    pub(super) fn is_held(&self) -> bool {
        self.held_since.is_some()
    }

    /// `other_modifier_held`: a modifier other than the target is down. The
    /// key pressed into a held modifier is part of a shortcut, never a start.
    pub(super) fn handle(
        &mut self,
        event_type: &EventType,
        other_modifier_held: bool,
        now: Instant,
    ) -> Signal {
        let Some(target) = self.target else {
            return Signal::None;
        };
        let Some(held_since) = self.held_since else {
            return match event_type {
                EventType::KeyPress(key) if *key == target && !other_modifier_held => {
                    self.held_since = Some(now);
                    Signal::Press
                }
                _ => Signal::None,
            };
        };
        let held_ms = now.saturating_duration_since(held_since).as_millis();
        match event_type {
            EventType::KeyRelease(key) if *key == target => {
                self.held_since = None;
                Signal::Release
            }
            // Key repeat of the target itself.
            EventType::KeyPress(key) if *key == target => Signal::None,
            // Before the start: any other input makes this ordinary typing.
            EventType::KeyPress(_) | EventType::ButtonPress(_) | EventType::Wheel { .. }
                if held_ms < ARMING_MS =>
            {
                self.held_since = None;
                Signal::Abort
            }
            // After the start: a key inside the chord window is a slow
            // shortcut. Space is typed mid-command and never one.
            EventType::KeyPress(key)
                if !is_modifier(*key) && *key != Key::Space && held_ms < CHORD_WINDOW_MS =>
            {
                self.held_since = None;
                Signal::Abort
            }
            _ => Signal::None,
        }
    }
}

/// Whether the agent-command key keeps the shared rdev thread working.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static DETECTOR: Mutex<Detector> = Mutex::new(Detector {
    target: None,
    held_since: None,
});
static PROMOTION: HoldPromotion = HoldPromotion::new();
/// For the events published from outside the rdev callback.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

fn log_hold(phase: &'static str) {
    // Content-free: the phase only, never a key. Debug level, so it is off
    // unless debug logging is switched on.
    tracing::debug!(target: "keyboard", phase, "agent command hold");
}

/// End the current hold from outside the callback. A started recording is
/// cancelled: nobody is left to release the key into a stop.
fn interrupt() {
    DETECTOR.lock_or_recover().reset();
    let started = PROMOTION.finish(|promoted| promoted.is_some());
    if started {
        log_hold("interrupted");
        if let Some(app) = APP.get() {
            let _ = app.emit("agent-command-cancel", ());
        }
    }
}

/// Bind the key (`None` unbinds). An unknown id leaves the working binding.
pub(super) fn configure(app: Option<tauri::AppHandle>, id: Option<&str>) -> Result<(), String> {
    let target = id
        .map(|id| {
            is_agent_command_key_id(id)
                .then(|| super::hotkey_to_rdev_key(id))
                .flatten()
                .ok_or_else(|| {
                    "Unsupported agent command key. Choose one from Settings.".to_string()
                })
        })
        .transpose()?;
    if let Some(app) = app {
        let _ = APP.set(app);
    }
    let unchanged = DETECTOR.lock_or_recover().target == target;
    if unchanged && active() == target.is_some() {
        // A repeated registration must not drop a hold in progress.
        return Ok(());
    }
    interrupt();
    DETECTOR.lock_or_recover().set_target(target);
    ACTIVE.store(target.is_some(), Ordering::SeqCst);
    Ok(())
}

/// The app was disabled or a cancel key was pressed: drop the hold.
pub(super) fn abandon(cancel_started_recording: bool) {
    if cancel_started_recording {
        interrupt();
    } else {
        DETECTOR.lock_or_recover().reset();
        PROMOTION.invalidate();
    }
}

/// Feed one rdev event. Called on the listener thread after the app-disabled
/// gate; `other_modifier_held` excludes the agent-command key itself.
pub(super) fn handle_event(
    handle: &tauri::AppHandle,
    event_type: &EventType,
    other_modifier_held: impl FnOnce(Key) -> bool,
) {
    // The detector lock is released before the promotion lock is taken:
    // promotion locks before detector locks (see `HoldPromotion`).
    let signal = {
        let mut detector = DETECTOR.lock_or_recover();
        let Some(target) = detector.target else {
            return;
        };
        detector.handle(event_type, other_modifier_held(target), Instant::now())
    };
    match signal {
        Signal::None => {}
        Signal::Press => schedule_start(handle.clone(), PROMOTION.begin()),
        Signal::Release => PROMOTION.finish(|promoted| {
            if promoted.is_some() {
                log_hold("stop");
                let _ = handle.emit("agent-command-stop", ());
            }
        }),
        Signal::Abort => PROMOTION.finish(|promoted| {
            if promoted.is_some() {
                log_hold("cancel");
                let _ = handle.emit("agent-command-cancel", ());
            }
        }),
    }
}

/// Publish the start for `press_id` after the arming window unless the press
/// ended, the key was rebound, or the app was disabled meanwhile.
fn schedule_start(handle: tauri::AppHandle, press_id: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ARMING_MS as u64));
        PROMOTION.promote(
            press_id,
            false,
            || active() && !super::is_app_disabled() && DETECTOR.lock_or_recover().is_held(),
            || {
                log_hold("start");
                let _ = handle.emit("agent-command-start", ());
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn press(key: Key) -> EventType {
        EventType::KeyPress(key)
    }

    fn release(key: Key) -> EventType {
        EventType::KeyRelease(key)
    }

    fn detector() -> (Detector, Instant) {
        let mut detector = Detector::default();
        detector.set_target(Some(Key::AltGr));
        (detector, Instant::now())
    }

    fn ms(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn hold_and_release_is_a_press_then_a_release() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        assert!(detector.is_held());
        // The target's own repeat changes nothing.
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, ms(t0, 100)),
            Signal::None
        );
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, ms(t0, 2_000)),
            Signal::Release
        );
        assert!(!detector.is_held());
        // A stray second release is not another stop.
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, ms(t0, 2_010)),
            Signal::None
        );
    }

    #[test]
    fn an_unbound_detector_ignores_every_key() {
        let mut detector = Detector::default();
        let now = Instant::now();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, now),
            Signal::None
        );
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, now),
            Signal::None
        );
        assert!(!detector.is_held());
    }

    #[test]
    fn only_the_bound_key_starts_a_hold() {
        let (mut detector, t0) = detector();
        // The dictation keys and the left Option key are other keys.
        for key in [Key::Alt, Key::ShiftLeft, Key::ControlRight, Key::F5] {
            assert_eq!(detector.handle(&press(key), false, t0), Signal::None);
            assert_eq!(detector.handle(&release(key), false, t0), Signal::None);
        }
        assert!(!detector.is_held());
    }

    #[test]
    fn a_key_pressed_into_another_modifier_is_a_shortcut_not_a_start() {
        let (mut detector, t0) = detector();
        // Command already down, then Right Option: Command+Option+something.
        assert_eq!(detector.handle(&press(Key::AltGr), true, t0), Signal::None);
        assert!(!detector.is_held());
        // Its release is then not a stop.
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, ms(t0, 400)),
            Signal::None
        );
    }

    #[test]
    fn any_input_inside_the_arming_window_aborts_the_hold() {
        let inputs = [
            press(Key::KeyE),
            press(Key::Space),
            press(Key::ShiftLeft),
            press(Key::MetaLeft),
            EventType::ButtonPress(rdev::Button::Left),
            EventType::Wheel {
                delta_x: 0,
                delta_y: 1,
            },
        ];
        for input in inputs {
            let (mut detector, t0) = detector();
            assert_eq!(
                detector.handle(&press(Key::AltGr), false, t0),
                Signal::Press
            );
            assert_eq!(
                detector.handle(&input, false, ms(t0, ARMING_MS as u64 - 1)),
                Signal::Abort,
                "{input:?}"
            );
            // The key is still physically down; its release is a no-op.
            assert_eq!(
                detector.handle(&release(Key::AltGr), false, ms(t0, 500)),
                Signal::None
            );
        }
    }

    #[test]
    fn pointer_movement_never_aborts_a_hold() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 1.0, y: 2.0 }, false, ms(t0, 10)),
            Signal::None
        );
        assert!(detector.is_held());
    }

    #[test]
    fn a_key_inside_the_chord_window_aborts_a_started_hold() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        // Option held, then an arrow 600 ms later: word navigation.
        assert_eq!(
            detector.handle(&press(Key::LeftArrow), false, ms(t0, 600)),
            Signal::Abort
        );
        assert!(!detector.is_held());
    }

    #[test]
    fn modifiers_and_space_after_the_start_keep_the_hold() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        for key in [Key::ShiftLeft, Key::MetaLeft, Key::Space] {
            assert_eq!(
                detector.handle(&press(key), false, ms(t0, ARMING_MS as u64)),
                Signal::None,
                "{key:?}"
            );
        }
        // Mouse input after the start does not end a command either.
        assert_eq!(
            detector.handle(
                &EventType::ButtonPress(rdev::Button::Left),
                false,
                ms(t0, 400)
            ),
            Signal::None
        );
        assert!(detector.is_held());
    }

    #[test]
    fn a_key_after_the_chord_window_keeps_the_hold() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        assert_eq!(
            detector.handle(&press(Key::KeyA), false, ms(t0, CHORD_WINDOW_MS as u64)),
            Signal::None
        );
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, ms(t0, 3_000)),
            Signal::Release
        );
    }

    #[test]
    fn rebinding_or_resetting_drops_the_hold() {
        let (mut detector, t0) = detector();
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, t0),
            Signal::Press
        );
        detector.reset();
        assert_eq!(
            detector.handle(&release(Key::AltGr), false, ms(t0, 500)),
            Signal::None
        );

        assert_eq!(
            detector.handle(&press(Key::AltGr), false, ms(t0, 600)),
            Signal::Press
        );
        detector.set_target(Some(Key::ShiftRight));
        assert!(!detector.is_held());
        // The old key no longer starts anything; the new one does.
        assert_eq!(
            detector.handle(&press(Key::AltGr), false, ms(t0, 700)),
            Signal::None
        );
        assert_eq!(
            detector.handle(&press(Key::ShiftRight), false, ms(t0, 800)),
            Signal::Press
        );
    }

    #[test]
    fn agent_command_keys_are_allow_listed_and_disjoint_from_dictation_keys() {
        for id in AGENT_COMMAND_KEY_IDS {
            assert!(is_agent_command_key_id(id), "{id}");
            assert!(!super::super::is_dictation_key_id(id), "{id}");
            assert!(!super::super::is_paste_last_shortcut_id(id), "{id}");
            assert!(super::super::hotkey_to_rdev_key(id).is_some(), "{id}");
        }
        for id in [
            "",
            "alt_l",
            "shift_l",
            "fn",
            "f5",
            "command_shift_v",
            "space",
        ] {
            assert!(!is_agent_command_key_id(id), "{id}");
        }
    }
}
