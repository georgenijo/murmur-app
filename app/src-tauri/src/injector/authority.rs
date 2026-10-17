//! Per-attempt authority shared by the async receiver and queued native work.
//! Blocking validation runs outside the fence. Only clipboard writes and the
//! final key-post boundary run inside it. Activation also uses the fence after
//! its AX queries; an already-dispatched native mutation cannot be undone.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Status {
    cancelled: bool,
    posted: bool,
    acknowledged: bool,
}

pub(crate) struct PasteAttempt {
    deadline: Instant,
    status: Mutex<Status>,
    finished: AtomicBool,
    delivery_id: u64,
}

impl PasteAttempt {
    pub(crate) fn new(timeout: Duration, delivery_id: u64) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            status: Mutex::new(Status::default()),
            finished: AtomicBool::new(false),
            delivery_id,
        }
    }

    pub(crate) fn is_current(&self) -> bool {
        let status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        !status.cancelled && !status.posted && Instant::now() < self.deadline
    }

    pub(crate) fn cancel(&self) {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancelled = true;
    }

    pub(crate) fn acknowledge(&self) {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .acknowledged = true;
    }

    fn ambiguous(&self) -> bool {
        let status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        status.posted && !status.acknowledged
    }

    pub(crate) fn write<T>(&self, write: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.cancelled || status.posted || Instant::now() >= self.deadline {
            return Err("Paste attempt expired or cancelled.".into());
        }
        // Keep cancellation and this synchronous mutation ordered. Blocking
        // validation must finish before entering this fence.
        write()
    }

    pub(crate) fn post(&self, check: impl FnOnce() -> bool, post: impl FnOnce()) -> bool {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.cancelled || status.posted || Instant::now() >= self.deadline || !check() {
            return false;
        }
        // Even a final native check may take time. Recheck the deadline before
        // committing; cancellation serializes with the actual key post.
        if Instant::now() >= self.deadline {
            return false;
        }
        status.posted = true;
        post();
        true
    }
}

/// Kept by the caller AND every queued callback. A timeout cannot release the
/// retry slot while old native work is still validating or cleaning up.
pub(crate) struct RetryWork(Arc<PasteAttempt>);

impl RetryWork {
    pub(crate) fn attempt(&self) -> &PasteAttempt {
        &self.0
    }
}

impl Drop for RetryWork {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub(crate) struct RetrySlot(Mutex<Option<Arc<PasteAttempt>>>);

impl RetrySlot {
    pub(crate) fn begin(&self, delivery_id: u64) -> Option<Arc<RetryWork>> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|previous| {
            !previous.finished.load(Ordering::SeqCst)
                || (previous.delivery_id == delivery_id && previous.ambiguous())
        }) {
            return None;
        }
        let attempt = Arc::new(PasteAttempt::new(Duration::from_secs(4), delivery_id));
        *slot = Some(attempt.clone());
        Some(Arc::new(RetryWork(attempt)))
    }
}

/// Cancels queued work on every exit, including dispatch errors and dropped
/// futures. A received result acknowledges posting before this guard drops.
pub(crate) struct CancelOnDrop<'a>(pub(crate) &'a PasteAttempt);
impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Validation may block while focus changes. Always put the cheap native
/// process/activation/clipboard checks AFTER it, at the side-effect boundary.
pub(crate) fn validated_side_effect(
    validate: impl FnOnce() -> bool,
    commit: impl FnOnce() -> bool,
) -> bool {
    validate() && commit()
}

/// A handoff owns exactly its one activation, with no intervening app/Space
/// transitions. Moving away and back also revokes rollback ownership.
pub(crate) fn rollback_transition_matches(before: (u64, u64), current: (u64, u64)) -> bool {
    before.0.checked_add(1) == Some(current.0) && before.1 == current.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn timeout_before_queued_execution_prevents_clipboard_and_post() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        attempt.cancel();
        assert!(attempt
            .write::<()>(|| panic!("stale clipboard write"))
            .is_err());
        assert!(!attempt.post(|| true, || panic!("stale post")));
    }

    #[test]
    fn deadline_before_execution_needs_no_receiver_flag() {
        let attempt = PasteAttempt::new(Duration::ZERO, 1);
        assert!(!attempt.is_current());
        assert!(attempt.write(|| Ok(())).is_err());
        assert!(!attempt.post(|| true, || panic!("expired post")));
    }

    #[test]
    fn timeout_during_validation_prevents_commit() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(!validated_side_effect(
            || {
                attempt.cancel();
                true
            },
            || attempt.post(|| true, || panic!("late post"))
        ));
    }

    #[test]
    fn cancellation_before_clipboard_is_checked_after_validation() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        attempt.cancel();
        assert!(attempt.write(|| Ok(())).is_err());
    }

    #[test]
    fn focus_switch_during_ax_is_rechecked_before_post() {
        let active = Cell::new(true);
        assert!(!validated_side_effect(
            || {
                active.set(false);
                true
            },
            || active.get()
        ));
    }

    #[test]
    fn clipboard_switch_during_validation_is_rechecked_before_post() {
        let clipboard = Cell::new(1);
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(!validated_side_effect(
            || {
                clipboard.set(2);
                true
            },
            || attempt.post(|| clipboard.get() == 1, || panic!("changed clipboard"))
        ));
    }

    #[test]
    fn cancellation_after_clipboard_prevents_post() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(attempt.write(|| Ok(())).is_ok());
        attempt.cancel();
        assert!(!attempt.post(|| true, || panic!("cancelled post")));
    }

    #[test]
    fn a_post_is_one_shot() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(attempt.post(|| true, || {}));
        assert!(!attempt.post(|| true, || panic!("duplicate post")));
        assert!(attempt.write(|| Ok(())).is_err());
    }

    #[test]
    fn timeout_while_worker_is_in_validation_cancels_post_and_keeps_slot() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        let worker = owner.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            validated_side_effect(
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                    true
                },
                || {
                    worker
                        .attempt()
                        .post(|| true, || panic!("timed-out worker posted"))
                },
            )
        });
        entered_rx.recv().unwrap();
        owner.attempt().cancel();
        drop(owner);
        assert!(slot.begin(1).is_none());
        resume_tx.send(()).unwrap();
        assert!(!handle.join().unwrap());
        assert!(slot.begin(1).is_some());
    }

    #[test]
    fn dropping_receiver_owner_cancels_callback_side_effects() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        {
            let _cancel = CancelOnDrop(&attempt);
        }
        assert!(!attempt.is_current());
        assert!(attempt.write(|| Ok(())).is_err());
    }

    #[test]
    fn queued_worker_keeps_retry_locked_after_receiver_timeout() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        let worker = owner.clone();
        owner.attempt().cancel();
        drop(owner);
        assert!(slot.begin(1).is_none());
        assert!(slot.begin(2).is_none());
        drop(worker);
        assert!(slot.begin(1).is_some());
    }

    #[test]
    fn ambiguous_post_blocks_duplicate_retry_until_a_new_delivery() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        assert!(owner.attempt().post(|| true, || {}));
        owner.attempt().cancel();
        drop(owner);
        assert!(slot.begin(1).is_none());
        assert!(slot.begin(2).is_some());
    }

    #[test]
    fn acknowledged_post_allows_a_later_explicit_retry() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        assert!(owner.attempt().post(|| true, || {}));
        owner.attempt().acknowledge();
        drop(owner);
        assert!(slot.begin(1).is_some());
    }

    #[test]
    fn rollback_never_steals_focus_after_user_moves_on_or_changes_spaces() {
        assert!(rollback_transition_matches((10, 2), (11, 2)));
        assert!(!rollback_transition_matches((10, 2), (12, 2)));
        assert!(!rollback_transition_matches((10, 2), (13, 2)));
        assert!(!rollback_transition_matches((10, 2), (11, 3)));
        assert!(!rollback_transition_matches((u64::MAX, 2), (0, 2)));
    }
}
