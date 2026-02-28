use rdev::{EventType, Key};
use std::time::Instant;

use super::{is_same_modifier, is_modifier, MAX_HOLD_DURATION_MS, DOUBLE_TAP_WINDOW_MS};

/// Cooldown after firing to prevent triple-tap spam
const COOLDOWN_MS: u128 = 50;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

pub(super) struct DoubleTapDetector {
    pub(super) state: DetectorState,
    pub(super) target_key: Option<Key>,
    pub(super) recording: bool,
    pub(super) state_entered_at: Instant,
    pub(super) last_fired_at: Option<Instant>,
}

impl DoubleTapDetector {
    pub(super) fn new() -> Self {
        Self {
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: Instant::now(),
            last_fired_at: None,
        }
    }

    pub(super) fn set_target(&mut self, key: Option<Key>) {
        self.target_key = key;
        self.reset();
    }

    pub(super) fn reset(&mut self) {
        self.state = DetectorState::Idle;
        self.state_entered_at = Instant::now();
    }

    fn transition(&mut self, new_state: DetectorState) {
        self.state = new_state;
        self.state_entered_at = Instant::now();
    }

    pub(super) fn elapsed_ms(&self) -> u128 {
        self.state_entered_at.elapsed().as_millis()
    }

    fn in_cooldown(&self) -> bool {
        self.last_fired_at
            .map(|t| t.elapsed().as_millis() < COOLDOWN_MS)
            .unwrap_or(false)
    }

    /// Process a keyboard event. Returns true if a double-tap was detected.
    pub(super) fn handle_event(&mut self, event_type: &EventType) -> bool {
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
