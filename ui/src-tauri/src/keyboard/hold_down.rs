use rdev::{EventType, Key};
use std::time::Instant;

use super::{is_same_modifier, is_modifier};

/// Cooldown after hold-down stop to prevent accidental re-trigger
const HOLD_DOWN_COOLDOWN_MS: u128 = 50;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoldDownEvent {
    None,
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoldState {
    Idle,
    Held,
}

pub(super) struct HoldDownDetector {
    pub(super) state: HoldState,
    pub(super) target_key: Option<Key>,
    pub(super) last_stopped_at: Option<Instant>,
}

impl HoldDownDetector {
    pub(super) fn new() -> Self {
        Self {
            state: HoldState::Idle,
            target_key: None,
            last_stopped_at: None,
        }
    }

    /// Set the target key. Returns `true` if the detector was in `Held` state
    /// (i.e. the caller should emit a stop event to the frontend).
    pub(super) fn set_target(&mut self, key: Option<Key>) -> bool {
        let was_held = self.state == HoldState::Held;
        if was_held {
            self.state = HoldState::Idle;
            self.last_stopped_at = Some(Instant::now());
        }
        self.target_key = key;
        was_held
    }

    pub(super) fn reset(&mut self) {
        self.state = HoldState::Idle;
    }

    fn in_cooldown(&self) -> bool {
        self.last_stopped_at
            .map(|t| t.elapsed().as_millis() < HOLD_DOWN_COOLDOWN_MS)
            .unwrap_or(false)
    }

    /// Process a keyboard event. Returns Start, Stop, or None.
    pub(super) fn handle_event(&mut self, event_type: &EventType) -> HoldDownEvent {
        let target = match self.target_key {
            Some(k) => k,
            None => return HoldDownEvent::None,
        };

        match self.state {
            HoldState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_same_modifier(*key, target) && !self.in_cooldown() {
                        self.state = HoldState::Held;
                        return HoldDownEvent::Start;
                    }
                }
                HoldDownEvent::None
            }

            HoldState::Held => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(Instant::now());
                        HoldDownEvent::Stop
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — ignore, stay held
                        HoldDownEvent::None
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A — cancel hold
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(Instant::now());
                        HoldDownEvent::Stop
                    }
                    _ => HoldDownEvent::None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn make_hold_detector(key: Key) -> HoldDownDetector {
        let mut d = HoldDownDetector::new();
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
    fn hold_basic_press_starts_release_stops() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_no_target_key_never_fires() {
        let mut d = HoldDownDetector::new();
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::None);
    }

    #[test]
    fn hold_wrong_key_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::Alt)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_key_repeat_ignored_while_held() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);

        // Key repeat events — should be ignored
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Held);

        // Release still works
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);
    }

    #[test]
    fn hold_modifier_plus_letter_cancels() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        // User types Shift+A — should cancel and stop
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::Stop);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_release_without_press_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Release while idle — nothing happens
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_stop() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Hold and release
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);

        // Immediately press again — should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_expires() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);

        // Wait for cooldown to expire
        sleep(Duration::from_millis(350));

        // Now press again — should work
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
    }

    #[test]
    fn hold_alt_key() {
        let mut d = make_hold_detector(Key::Alt);

        assert_eq!(d.handle_event(&press(Key::Alt)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::Alt)), HoldDownEvent::Stop);
    }

    #[test]
    fn hold_ctrl_key() {
        let mut d = make_hold_detector(Key::ControlRight);

        assert_eq!(d.handle_event(&press(Key::ControlRight)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ControlRight)), HoldDownEvent::Stop);
    }

    #[test]
    fn hold_set_target_while_held_stops() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        // Change target while held — resets to Idle, returns true (should emit stop)
        assert!(d.set_target(Some(Key::Alt)));
        assert_eq!(d.state, HoldState::Idle);

        // Changing target while idle — returns false
        assert!(!d.set_target(Some(Key::ControlRight)));
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_non_modifier_press_in_idle_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Random key presses while idle — nothing happens
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&press(Key::Space)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_letter_cancel() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        // Cancel with letter
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::Stop);

        // Immediate re-press should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
    }
}
