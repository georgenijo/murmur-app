use std::sync::Mutex;

/// Owns both the promotion decision and publication. A release cannot observe
/// an unpromoted press after its start has been accepted, or publish a stop
/// before that start. Detector locks must be released before `finish`/`begin`.
pub(super) struct HoldPromotion {
    state: Mutex<PromotionState>,
}

struct PromotionState {
    sequence: u64,
    promoted: Option<bool>,
}

impl HoldPromotion {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(PromotionState {
                sequence: 0,
                promoted: None,
            }),
        }
    }

    pub(super) fn begin(&self) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.sequence = state.sequence.wrapping_add(1);
        state.promoted = None;
        state.sequence
    }

    pub(super) fn invalidate(&self) {
        self.begin();
    }

    /// Read backend state before entering here: backend transitions may
    /// invalidate promotion while holding their own locks. `eligible` only
    /// inspects listener/detector state; `publish` only dispatches the event.
    pub(super) fn promote(
        &self,
        sequence: u64,
        over_recording: bool,
        eligible: impl FnOnce() -> bool,
        publish: impl FnOnce(),
    ) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.sequence != sequence || state.promoted.is_some() || !eligible() {
            return false;
        }
        state.promoted = Some(over_recording);
        publish();
        true
    }

    pub(super) fn finish<T>(&self, publish: impl FnOnce(Option<bool>) -> T) -> T {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.sequence = state.sequence.wrapping_add(1);
        let promoted = state.promoted.take();
        publish(promoted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn release_before_promotion_prevents_late_start() {
        let promotion = HoldPromotion::new();
        let press = promotion.begin();
        promotion.finish(|promoted| assert_eq!(promoted, None));
        assert!(!promotion.promote(press, false, || true, || panic!("late start")));
    }

    #[test]
    fn release_cannot_overtake_start_publication() {
        let promotion = Arc::new(HoldPromotion::new());
        let press = promotion.begin();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let timer_promotion = promotion.clone();
        let start_tx = event_tx.clone();
        let timer = std::thread::spawn(move || {
            timer_promotion.promote(
                press,
                false,
                || true,
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                    start_tx.send("start").unwrap();
                },
            )
        });
        entered_rx.recv().unwrap();
        let release_promotion = promotion.clone();
        let release = std::thread::spawn(move || {
            release_promotion.finish(|promoted| {
                assert_eq!(promoted, Some(false));
                event_tx.send("stop").unwrap();
            });
        });
        resume_tx.send(()).unwrap();
        assert!(timer.join().unwrap());
        release.join().unwrap();
        assert_eq!(event_rx.into_iter().collect::<Vec<_>>(), ["start", "stop"]);
    }

    #[test]
    fn obsolete_timer_cannot_promote_a_new_press() {
        let promotion = HoldPromotion::new();
        let old = promotion.begin();
        promotion.invalidate();
        let current = promotion.begin();
        assert!(!promotion.promote(old, false, || true, || panic!("old start")));
        assert!(promotion.promote(current, true, || true, || {}));
        promotion.finish(|promoted| assert_eq!(promoted, Some(true)));
    }

    #[test]
    fn invalid_context_and_duplicate_timer_never_publish() {
        let promotion = HoldPromotion::new();
        let press = promotion.begin();
        assert!(!promotion.promote(press, false, || false, || panic!("invalid start")));
        assert!(promotion.promote(press, false, || true, || {}));
        assert!(!promotion.promote(press, false, || true, || panic!("duplicate start")));
        promotion.finish(|promoted| assert_eq!(promoted, Some(false)));
        promotion.finish(|promoted| assert_eq!(promoted, None));
    }
}
