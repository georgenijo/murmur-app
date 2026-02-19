//! Double-tap modifier key detection using rdev for low-level keyboard events.
//!
//! To start recording (double-tap): Idle → WaitingFirstUp → WaitingSecondDown → WaitingSecondUp → FIRE
//! To stop recording (single tap):  Idle → WaitingFirstUp → FIRE on release
//! Rejects held keys, modifier+letter combos, slow taps, and triple-tap spam.

use rdev::{listen, set_is_main_thread, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;

/// Max duration a single tap can be held before it's rejected
const MAX_HOLD_DURATION_MS: u128 = 300;

/// Max gap between first key-up and second key-down
const DOUBLE_TAP_WINDOW_MS: u128 = 400;

/// Cooldown after firing to prevent triple-tap spam
const COOLDOWN_MS: u128 = 500;

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

struct DoubleTapDetector {
    state: DetectorState,
    target_key: Option<Key>,
    recording: bool,
    state_entered_at: Instant,
    last_fired_at: Option<Instant>,
}

impl DoubleTapDetector {
    fn new() -> Self {
        Self {
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: Instant::now(),
            last_fired_at: None,
        }
    }

    fn set_target(&mut self, key: Option<Key>) {
        self.target_key = key;
        self.reset();
    }

    fn reset(&mut self) {
        self.state = DetectorState::Idle;
        self.state_entered_at = Instant::now();
    }

    fn transition(&mut self, new_state: DetectorState) {
        self.state = new_state;
        self.state_entered_at = Instant::now();
    }

    fn elapsed_ms(&self) -> u128 {
        self.state_entered_at.elapsed().as_millis()
    }

    fn in_cooldown(&self) -> bool {
        self.last_fired_at
            .map(|t| t.elapsed().as_millis() < COOLDOWN_MS)
            .unwrap_or(false)
    }

    /// Process a keyboard event. Returns true if a double-tap was detected.
    fn handle_event(&mut self, event_type: &EventType) -> bool {
        let target = match self.target_key {
            Some(k) => k,
            None => return false,
        };

        if self.in_cooldown() {
            return false;
        }

        match self.state {
            DetectorState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_same_modifier(*key, target) {
                        self.transition(DetectorState::WaitingFirstUp);
                    }
                }
                false
            }

            DetectorState::WaitingFirstUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            if self.recording {
                                // Single tap to stop — fire immediately
                                self.last_fired_at = Some(Instant::now());
                                self.reset();
                                return true;
                            }
                            self.transition(DetectorState::WaitingSecondDown);
                        } else {
                            // Held too long — not a tap
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat event — ignore, stay in same state
                        // But check if we've been held too long
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                    _ => {
                        // Check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                }
                false
            }

            DetectorState::WaitingSecondDown => {
                if self.elapsed_ms() > DOUBLE_TAP_WINDOW_MS {
                    self.reset();
                    return false;
                }
                match event_type {
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        self.transition(DetectorState::WaitingSecondUp);
                    }
                    EventType::KeyPress(_) => {
                        // Any other key press — abort
                        self.reset();
                    }
                    _ => {}
                }
                false
            }

            DetectorState::WaitingSecondUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            // Double-tap detected!
                            self.last_fired_at = Some(Instant::now());
                            self.reset();
                            return true;
                        } else {
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // Combo like Shift+A on second press
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                    _ => {
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                }
                false
            }
        }
    }
}

/// Check if a key is any modifier key
fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::Alt
            | Key::AltGr
            | Key::ControlLeft
            | Key::ControlRight
            | Key::MetaLeft
            | Key::MetaRight
    )
}

/// Check if two keys are the same modifier (treating left/right variants as equivalent for Shift)
fn is_same_modifier(a: Key, b: Key) -> bool {
    a == b
}

/// Map hotkey string from settings to rdev Key
fn hotkey_to_rdev_key(hotkey: &str) -> Option<Key> {
    match hotkey {
        "shift_l" => Some(Key::ShiftLeft),
        "alt_l" => Some(Key::Alt),
        "ctrl_r" => Some(Key::ControlRight),
        _ => None,
    }
}

// -- Global listener state --

static LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static LISTENER_THREAD_SPAWNED: AtomicBool = AtomicBool::new(false);

static DETECTOR: Mutex<Option<DoubleTapDetector>> = Mutex::new(None);

/// Start the double-tap listener. Spawns the rdev listener thread if not already running.
/// If already running, just updates the target key and re-enables.
pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str) {
    let target = hotkey_to_rdev_key(hotkey);

    // Initialize or update the detector
    {
        let mut det = DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        match det.as_mut() {
            Some(d) => d.set_target(target),
            None => {
                let mut d = DoubleTapDetector::new();
                d.set_target(target);
                *det = Some(d);
            }
        }
    }

    LISTENER_ACTIVE.store(true, Ordering::SeqCst);

    // Only spawn the thread once
    if LISTENER_THREAD_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let handle = app_handle.clone();
        std::thread::spawn(move || {
            // CRITICAL: rdev's keyboard translation calls TIS/TSM APIs that must
            // run on the main thread on macOS. This flag tells rdev to dispatch
            // those calls to the main queue via dispatch_sync instead of calling
            // them directly from this background thread.
            set_is_main_thread(false);

            let callback = move |event: Event| {
                if !LISTENER_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }

                let fired = {
                    let mut det = DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(d) = det.as_mut() {
                        d.handle_event(&event.event_type)
                    } else {
                        false
                    }
                };

                if fired {
                    let _ = handle.emit("double-tap-toggle", ());
                }
            };

            if let Err(e) = listen(callback) {
                eprintln!("[Keyboard] rdev listen error: {:?}", e);
            }
        });
    }
}

/// Stop processing double-tap events (the thread stays alive but idle).
pub fn stop_listener() {
    LISTENER_ACTIVE.store(false, Ordering::SeqCst);

    // Reset detector state
    let mut det = DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        d.reset();
    }
}

/// Update the target key without stopping/restarting the listener.
pub fn set_target_key(hotkey: &str) {
    let target = hotkey_to_rdev_key(hotkey);
    let mut det = DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        d.set_target(target);
    }
}

/// Tell the detector whether we're currently recording.
/// When recording, a single tap fires (to stop). When idle, double-tap fires (to start).
pub fn set_recording_state(recording: bool) {
    let mut det = DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        d.recording = recording;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn make_detector(key: Key) -> DoubleTapDetector {
        let mut d = DoubleTapDetector::new();
        d.set_target(Some(key));
        d
    }

    fn press(key: Key) -> EventType {
        EventType::KeyPress(key)
    }

    fn release(key: Key) -> EventType {
        EventType::KeyRelease(key)
    }

    #[test]
    fn basic_double_tap_fires() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Second tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn no_target_key_never_fires() {
        let mut d = DoubleTapDetector::new();
        // target_key is None
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn wrong_key_ignored() {
        let mut d = make_detector(Key::ShiftLeft);

        // Press Alt instead of Shift — should stay idle
        assert!(!d.handle_event(&press(Key::Alt)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn modifier_plus_letter_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // Shift down
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Then 'A' while Shift held — user is typing Shift+A
        assert!(!d.handle_event(&press(Key::KeyA)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn held_too_long_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Wait longer than MAX_HOLD_DURATION_MS
        sleep(Duration::from_millis(350));

        // Release after too long
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn slow_gap_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap — quick
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait longer than DOUBLE_TAP_WINDOW_MS
        sleep(Duration::from_millis(450));

        // Second press after too long a gap — timeout resets to Idle,
        // the press event itself is consumed by the timeout check
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn cooldown_prevents_triple_tap() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Immediately try another double-tap — should be blocked by cooldown
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        // in_cooldown() returns true, so handle_event returns false early
    }

    #[test]
    fn cooldown_expires() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Wait for cooldown to expire
        sleep(Duration::from_millis(550));

        // Now another double-tap should work
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn second_tap_held_too_long_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap — quick
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — press quick but hold too long before release
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        sleep(Duration::from_millis(350));

        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn letter_during_second_tap_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — Shift down then letter
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        d.handle_event(&press(Key::KeyB));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn other_key_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Press a different key in the gap
        d.handle_event(&press(Key::KeyA));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn key_repeat_during_first_tap_within_hold_duration() {
        let mut d = make_detector(Key::ShiftLeft);

        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Key repeat (same key press again) — should stay in state
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Release quickly
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
    }

    #[test]
    fn alt_key_double_tap() {
        let mut d = make_detector(Key::Alt);

        d.handle_event(&press(Key::Alt));
        d.handle_event(&release(Key::Alt));
        d.handle_event(&press(Key::Alt));
        assert!(d.handle_event(&release(Key::Alt)));
    }

    #[test]
    fn ctrl_key_double_tap() {
        let mut d = make_detector(Key::ControlRight);

        d.handle_event(&press(Key::ControlRight));
        d.handle_event(&release(Key::ControlRight));
        d.handle_event(&press(Key::ControlRight));
        assert!(d.handle_event(&release(Key::ControlRight)));
    }

    #[test]
    fn single_tap_does_not_fire() {
        let mut d = make_detector(Key::ShiftLeft);

        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
        // No second tap — never fires
    }

    #[test]
    fn set_target_resets_state() {
        let mut d = make_detector(Key::ShiftLeft);

        // Start a first tap
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Change target — should reset
        d.set_target(Some(Key::Alt));
        assert_eq!(d.state, DetectorState::Idle);
        assert_eq!(d.target_key, Some(Key::Alt));
    }

    #[test]
    fn hotkey_string_mapping() {
        assert_eq!(hotkey_to_rdev_key("shift_l"), Some(Key::ShiftLeft));
        assert_eq!(hotkey_to_rdev_key("alt_l"), Some(Key::Alt));
        assert_eq!(hotkey_to_rdev_key("ctrl_r"), Some(Key::ControlRight));
        assert_eq!(hotkey_to_rdev_key("unknown"), None);
    }

    #[test]
    fn is_modifier_classification() {
        assert!(is_modifier(Key::ShiftLeft));
        assert!(is_modifier(Key::ShiftRight));
        assert!(is_modifier(Key::Alt));
        assert!(is_modifier(Key::ControlLeft));
        assert!(is_modifier(Key::ControlRight));
        assert!(is_modifier(Key::MetaLeft));
        assert!(!is_modifier(Key::KeyA));
        assert!(!is_modifier(Key::Space));
        assert!(!is_modifier(Key::Return));
    }

    // -- Single-tap-to-stop tests (recording=true) --

    #[test]
    fn single_tap_stops_when_recording() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        // Single tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_held_too_long_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        sleep(Duration::from_millis(350));

        // Held too long — not a tap, should not fire
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_with_letter_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        // User types Shift+A — should not stop recording
        assert!(!d.handle_event(&press(Key::KeyA)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn double_tap_still_required_when_not_recording() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = false;

        // Single tap should NOT fire
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
        // Needs second tap to fire
    }

    #[test]
    fn full_cycle_double_tap_start_single_tap_stop() {
        let mut d = make_detector(Key::ShiftLeft);

        // Not recording — double tap to start
        d.recording = false;
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Wait for cooldown
        sleep(Duration::from_millis(550));

        // Now recording — single tap to stop
        d.recording = true;
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }
}
