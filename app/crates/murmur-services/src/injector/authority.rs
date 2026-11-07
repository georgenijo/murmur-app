//! Per-attempt authority shared by the async receiver and queued native work.
//! Cancellation and side-effect commitment linearize through one atomic state.
//! Slow validation runs before commitment; native IPC already in flight cannot
//! be revoked. Its worker retains ownership and performs guarded cleanup.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CANCELLED: u8 = 1;
const POSTED: u8 = 2;
const MUTATING: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptStop {
    Cancelled,
    Expired,
    AlreadyCommitted,
    OperationInFlight,
}

impl AttemptStop {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::AlreadyCommitted => "already_committed",
            Self::OperationInFlight => "operation_in_flight",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostDecision {
    Posted,
    Refused,
    Stopped(AttemptStop),
}

impl PostDecision {
    pub(crate) fn is_posted(self) -> bool {
        self == Self::Posted
    }
}

pub(crate) struct PasteAttempt {
    deadline: Instant,
    operation: AtomicU8,
    acknowledged: AtomicBool,
    finished: AtomicBool,
    delivery_id: u64,
}

struct Mutation<'a>(&'a AtomicU8);
impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        self.0.fetch_and(!MUTATING, Ordering::SeqCst);
    }
}

impl PasteAttempt {
    pub(crate) fn new(timeout: Duration, delivery_id: u64) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            operation: AtomicU8::new(0),
            acknowledged: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            delivery_id,
        }
    }

    fn stopped(&self, state: u8) -> Option<AttemptStop> {
        if state & CANCELLED != 0 {
            Some(AttemptStop::Cancelled)
        } else if Instant::now() >= self.deadline {
            Some(AttemptStop::Expired)
        } else if state & POSTED != 0 {
            Some(AttemptStop::AlreadyCommitted)
        } else if state & MUTATING != 0 {
            Some(AttemptStop::OperationInFlight)
        } else {
            None
        }
    }

    pub(crate) fn stop_reason(&self) -> Option<AttemptStop> {
        self.stopped(self.operation.load(Ordering::SeqCst))
    }

    pub(crate) fn is_current(&self) -> bool {
        self.stop_reason().is_none()
    }

    /// Never waits for clipboard, AX, focus probing, or key-up. If commitment
    /// won the CAS first, that native operation remains explicitly in flight.
    pub(crate) fn cancel(&self) {
        self.operation.fetch_or(CANCELLED, Ordering::SeqCst);
    }

    pub(crate) fn acknowledge(&self) {
        self.acknowledged.store(true, Ordering::SeqCst);
    }

    fn ambiguous(&self) -> bool {
        self.operation.load(Ordering::SeqCst) & POSTED != 0
            && !self.acknowledged.load(Ordering::SeqCst)
    }

    fn claim(&self, operation: u8) -> Result<(), AttemptStop> {
        if let Some(reason) = self.stop_reason() {
            return Err(reason);
        }
        self.operation
            .compare_exchange(0, operation, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|state| {
                self.stopped(state)
                    .expect("nonzero operation has a stop reason")
            })
    }

    /// Clipboard writes and AX activation are synchronous native mutations.
    /// Claim before dispatch, without holding a mutex across IPC. A cancellation
    /// after this claim cannot undo the write/activation; the caller must keep
    /// its worker alive through completion and guarded focus rollback.
    pub(crate) fn mutate<T>(&self, mutate: impl FnOnce() -> T) -> Result<T, AttemptStop> {
        self.claim(MUTATING)?;
        let _in_flight = Mutation(&self.operation);
        Ok(mutate())
    }

    pub(crate) fn post(&self, check: impl FnOnce() -> bool, post: impl FnOnce()) -> PostDecision {
        if let Some(reason) = self.stop_reason() {
            return PostDecision::Stopped(reason);
        }
        // The bounded live AX focus query runs WITHOUT a held authority mutex.
        let safe = check();
        if let Some(reason) = self.stop_reason() {
            return PostDecision::Stopped(reason);
        }
        if !safe {
            return PostDecision::Refused;
        }
        match self.claim(POSTED) {
            Ok(()) => {
                post();
                PostDecision::Posted
            }
            Err(reason) => PostDecision::Stopped(reason),
        }
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

/// Target-field validation may block while focus changes. The final bounded
/// systemwide focus query and clipboard proof must follow it before commitment.
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
    fn cancellation_returns_while_native_activation_is_still_in_flight() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        let worker = owner.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let operation = std::thread::spawn(move || {
            worker.attempt().mutate(|| {
                entered_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                true
            })
        });
        entered_rx.recv().unwrap();
        let cancelling = owner.clone();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let canceller = std::thread::spawn(move || {
            cancelling.attempt().cancel();
            cancelled_tx.send(()).unwrap();
        });
        let immediate = cancelled_rx
            .recv_timeout(Duration::from_millis(500))
            .is_ok();
        assert!(slot.begin(1).is_none());
        resume_tx.send(()).unwrap();
        canceller.join().unwrap();
        assert_eq!(
            operation.join().unwrap(),
            Ok(true),
            "already committed IPC completes honestly"
        );
        assert!(immediate, "cancel must not wait for native AXFrontmost");
        assert_eq!(owner.attempt().stop_reason(), Some(AttemptStop::Cancelled));
        assert_eq!(
            owner
                .attempt()
                .post(|| true, || panic!("post after cancelled activation")),
            PostDecision::Stopped(AttemptStop::Cancelled)
        );
    }

    #[test]
    fn cancellation_during_final_live_focus_query_prevents_commit_without_waiting() {
        let attempt = Arc::new(PasteAttempt::new(Duration::from_secs(1), 1));
        let worker = attempt.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let operation = std::thread::spawn(move || {
            worker.post(
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                    true
                },
                || panic!("cancelled focus query posted"),
            )
        });
        entered_rx.recv().unwrap();
        let cancelling = attempt.clone();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let canceller = std::thread::spawn(move || {
            cancelling.cancel();
            cancelled_tx.send(()).unwrap();
        });
        let immediate = cancelled_rx
            .recv_timeout(Duration::from_millis(500))
            .is_ok();
        resume_tx.send(()).unwrap();
        canceller.join().unwrap();
        assert!(immediate, "cancel must not wait for live focus IPC");
        assert_eq!(
            operation.join().unwrap(),
            PostDecision::Stopped(AttemptStop::Cancelled)
        );
    }

    #[test]
    fn cancellation_after_post_commit_keeps_ambiguous_retry_latched() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        let worker = owner.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let operation = std::thread::spawn(move || {
            worker.attempt().post(
                || true,
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                },
            )
        });
        entered_rx.recv().unwrap();
        let cancelling = owner.clone();
        let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
        let canceller = std::thread::spawn(move || {
            cancelling.attempt().cancel();
            cancelled_tx.send(()).unwrap();
        });
        let immediate = cancelled_rx
            .recv_timeout(Duration::from_millis(500))
            .is_ok();
        assert!(slot.begin(1).is_none());
        resume_tx.send(()).unwrap();
        canceller.join().unwrap();
        assert!(
            immediate,
            "cancel must not wait for key-up after commitment"
        );
        assert_eq!(operation.join().unwrap(), PostDecision::Posted);
        drop(owner);
        assert!(slot.begin(1).is_none());
        assert!(slot.begin(2).is_some());
    }

    #[test]
    fn stop_reasons_are_typed_and_mutation_does_not_clear_cancellation() {
        let expired = PasteAttempt::new(Duration::ZERO, 1);
        assert_eq!(
            expired.mutate::<()>(|| panic!("expired activation")),
            Err(AttemptStop::Expired)
        );
        assert_eq!(
            expired.post(|| true, || panic!("expired post")),
            PostDecision::Stopped(AttemptStop::Expired)
        );
        let cancelled = PasteAttempt::new(Duration::from_secs(1), 1);
        cancelled.cancel();
        assert_eq!(
            cancelled.mutate::<()>(|| panic!("cancelled activation")),
            Err(AttemptStop::Cancelled)
        );
    }

    #[test]
    fn timeout_before_queued_execution_prevents_clipboard_and_post() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        attempt.cancel();
        assert!(attempt
            .mutate::<()>(|| panic!("stale clipboard write"))
            .is_err());
        assert!(!attempt.post(|| true, || panic!("stale post")).is_posted());
    }

    #[test]
    fn deadline_before_execution_needs_no_receiver_flag() {
        let attempt = PasteAttempt::new(Duration::ZERO, 1);
        assert!(!attempt.is_current());
        assert!(attempt.mutate(|| ()).is_err());
        assert!(!attempt.post(|| true, || panic!("expired post")).is_posted());
    }

    #[test]
    fn timeout_during_validation_prevents_commit() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(!validated_side_effect(
            || {
                attempt.cancel();
                true
            },
            || attempt.post(|| true, || panic!("late post")).is_posted()
        ));
    }

    #[test]
    fn cancellation_before_clipboard_is_checked_after_validation() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        attempt.cancel();
        assert!(attempt.mutate(|| ()).is_err());
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
            || attempt
                .post(|| clipboard.get() == 1, || panic!("changed clipboard"))
                .is_posted()
        ));
    }

    #[test]
    fn cancellation_after_clipboard_prevents_post() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(attempt.mutate(|| ()).is_ok());
        attempt.cancel();
        assert!(!attempt
            .post(|| true, || panic!("cancelled post"))
            .is_posted());
    }

    #[test]
    fn a_post_is_one_shot() {
        let attempt = PasteAttempt::new(Duration::from_secs(1), 1);
        assert!(attempt.post(|| true, || {}).is_posted());
        assert!(!attempt
            .post(|| true, || panic!("duplicate post"))
            .is_posted());
        assert!(attempt.mutate(|| ()).is_err());
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
                        .is_posted()
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
        assert!(attempt.mutate(|| ()).is_err());
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
        assert!(owner.attempt().post(|| true, || {}).is_posted());
        owner.attempt().cancel();
        drop(owner);
        assert!(slot.begin(1).is_none());
        assert!(slot.begin(2).is_some());
    }

    #[test]
    fn acknowledged_post_allows_a_later_explicit_retry() {
        let slot = RetrySlot::default();
        let owner = slot.begin(1).unwrap();
        assert!(owner.attempt().post(|| true, || {}).is_posted());
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
