use murmur_core::keyboard::*;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct FakeClock(Mutex<Instant>);
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}
impl FakeClock {
    fn advance(&self, ms: u64) {
        *self.0.lock().unwrap() += Duration::from_millis(ms);
    }
}
fn double() -> (DoubleTapDetector, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock(Mutex::new(Instant::now())));
    let mut detector = DoubleTapDetector::with_clock(clock.clone());
    detector.set_target(Some(Key::ShiftLeft));
    (detector, clock)
}
#[test]
fn double_tap_exact_edges_expiry_hold_and_cooldown() {
    for (held, gap, fire) in [
        (200, 400, true),
        (201, 400, false),
        (200, 401, false),
        (0, 0, true),
    ] {
        let (mut d, clock) = double();
        assert!(!d.handle_event(&EventType::KeyPress(Key::ShiftLeft)));
        clock.advance(held);
        assert!(!d.handle_event(&EventType::KeyRelease(Key::ShiftLeft)));
        clock.advance(gap);
        assert!(!d.handle_event(&EventType::KeyPress(Key::ShiftLeft)));
        assert_eq!(d.handle_event(&EventType::KeyRelease(Key::ShiftLeft)), fire);
        if fire {
            assert!(!d.handle_event(&EventType::KeyPress(Key::ShiftLeft)));
            clock.advance(50);
            assert!(!d.handle_event(&EventType::KeyPress(Key::ShiftLeft)));
            assert_eq!(d.state, DetectorState::WaitingFirstUp);
        }
    }
}
#[test]
fn combo_rejection_preparation_and_single_tap_stop() {
    let (mut d, clock) = double();
    d.handle_event(&EventType::KeyPress(Key::ShiftLeft));
    clock.advance(50);
    d.handle_event(&EventType::KeyRelease(Key::ShiftLeft));
    assert!(matches!(
        d.take_prepare_signal(false),
        PrepareSignal::Request(_)
    ));
    d.handle_event(&EventType::KeyPress(Key::KeyA));
    assert!(matches!(
        d.take_prepare_signal(false),
        PrepareSignal::Abandon
    ));
    d.recording = true;
    d.handle_event(&EventType::KeyPress(Key::ShiftLeft));
    clock.advance(50);
    assert!(d.handle_event(&EventType::KeyRelease(Key::ShiftLeft)));
    let mut f = DoubleTapDetector::new();
    f.set_target(Some(Key::F8));
    assert!(!f.handle_event_with_context(&EventType::KeyPress(Key::F8), true, false));
    assert_eq!(f.state, DetectorState::Idle);
}
#[test]
fn hold_and_combined_arming_boundaries_combo_release_and_stale_promotion() {
    use murmur_core::hold_promotion::HoldPromotion;
    for (mode, expected) in [(DetectorMode::HoldDown, 150), (DetectorMode::Both, 200)] {
        let clock = Arc::new(FakeClock(Mutex::new(Instant::now())));
        let mut d = HoldDownDetector::with_clock(clock.clone());
        d.set_target(Some(Key::ShiftLeft));
        d.arming_ms = hold_arming_ms(mode, Some(Key::ShiftLeft));
        assert_eq!(d.arming_ms, expected);
        assert_eq!(
            d.handle_event(&EventType::KeyPress(Key::ShiftLeft)),
            HoldDownEvent::Start
        );
        clock.advance(expected as u64 - 1);
        assert!(d.is_arming());
        clock.advance(1);
        assert!(!d.is_arming());
        assert_eq!(
            d.handle_event(&EventType::KeyRelease(Key::ShiftLeft)),
            HoldDownEvent::Stop
        );
        clock.advance(49);
        assert_eq!(
            d.handle_event(&EventType::KeyPress(Key::ShiftLeft)),
            HoldDownEvent::None
        );
        clock.advance(1);
        assert_eq!(
            d.handle_event(&EventType::KeyPress(Key::ShiftLeft)),
            HoldDownEvent::Start
        );
        assert_eq!(
            d.handle_event(&EventType::KeyPress(Key::KeyA)),
            HoldDownEvent::Stop
        );
    }
    let promotion = HoldPromotion::new();
    let old = promotion.begin();
    promotion.invalidate();
    assert!(!promotion.promote(old, false, || true, || panic!("stale promotion")));
}
