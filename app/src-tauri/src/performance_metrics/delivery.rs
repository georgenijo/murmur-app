//! Content-free delivery attribution. No API accepts arbitrary diagnostic text.
//! Each callback holds its recording's collector, never a global "latest run".
use super::{MeasurementV1 as M, UnavailableReasonV1 as U};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

fn missing<T>() -> M<T> {
    M::Unavailable {
        reason: U::NoSamples,
    }
}
pub(crate) fn us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}

macro_rules! bounded_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum $name { $($variant),+ }
    }
}
bounded_enum!(Route {
    Normal,
    SmartCurrent,
    SmartHandoff,
    ClipboardOnly
});
bounded_enum!(Outcome {
    NoText,
    AutoPasted,
    AutoPasteDisabled,
    AccessibilityDenied,
    TargetChanged,
    FocusNotEditable,
    ClipboardChanged,
    PasteFailed,
    SmartTargetUnavailable,
    Cancelled,
    Expired,
    AlreadyCommitted,
    OperationInFlight,
    StaleOwner,
    ClipboardWriteFailed,
    DispatchFailed,
    ReplyLost,
    Timeout,
    EarlyExit
});
bounded_enum!(FocusKind {
    Editable,
    NonEditable,
    Secure,
    Unknown
});
bounded_enum!(Transport { Native, Osascript });
bounded_enum!(Proof {
    LiveMatch,
    LiveMismatch,
    IdentityMismatch,
    FrontmostFallbackMatch,
    FrontmostFallbackMismatch
});
bounded_enum!(PostOutcome {
    Posted,
    Refused,
    Stopped,
    ConstructionFailed
});
// Names match the diagnostic contract's three event-construction failures.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FirstFailure {
    SourceCreate,
    KeyDownCreate,
    KeyUpCreate,
}
bounded_enum!(SmartOutcome {
    Ready,
    ActivationRequested,
    Refused,
    RefusedClipboardOnly,
    StructurallyConfirmed,
    Unconfirmed,
    NotObserved,
    Restored,
    Timeout,
    ReplyLost,
    StaleOwner
});
bounded_enum!(WindowDiagnostic { UnavailableOffPath });
bounded_enum!(VerificationPhase {
    BeforeFocus,
    AfterFocus,
    AfterClipboard
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusQuery {
    pub outcome: M<FocusKind>,
    pub transport: M<Transport>,
    pub ax_status: M<i32>,
    pub timed_out: bool,
}
impl Default for FocusQuery {
    fn default() -> Self {
        Self {
            outcome: missing(),
            transport: missing(),
            ax_status: missing(),
            timed_out: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetVerification {
    pub phase: VerificationPhase,
    pub elapsed_us: M<u64>,
    pub lookup_retry_count: M<u64>,
    pub outcome: M<crate::frontmost::VerificationOutcome>,
}
impl TargetVerification {
    fn new(phase: VerificationPhase) -> Self {
        Self {
            phase,
            elapsed_us: missing(),
            lookup_retry_count: missing(),
            outcome: missing(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativePost {
    pub event_construction_us: M<u64>,
    pub final_focus_proof_us: M<u64>,
    pub final_focus_proof: M<Proof>,
    pub final_focus_ax_status: M<i32>,
    pub key_down_post_us: M<u64>,
    pub key_hold_us: M<u64>,
    pub key_up_post_us: M<u64>,
    pub outcome: M<PostOutcome>,
}
impl Default for NativePost {
    fn default() -> Self {
        Self {
            event_construction_us: missing(),
            final_focus_proof_us: missing(),
            final_focus_proof: missing(),
            final_focus_ax_status: missing(),
            key_down_post_us: missing(),
            key_hold_us: missing(),
            key_up_post_us: missing(),
            outcome: missing(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryTimingV1 {
    #[serde(deserialize_with = "version_one")]
    pub schema_version: u32,
    pub accepted_route: M<Route>,
    pub actual_route: M<Route>,
    pub outcome: M<Outcome>,
    pub configured_delay_ms: u64,
    pub accessibility: M<bool>,
    pub stage_total_us: M<u64>,
    pub to_post_us: M<u64>,
    pub configured_delay_elapsed_us: M<u64>,
    pub prepare_queue_us: M<u64>,
    pub prepare_owner_wait_us: M<u64>,
    pub prepare_work_us: M<u64>,
    pub activation_wait_us: M<u64>,
    pub inject_queue_us: M<u64>,
    pub inject_owner_wait_us: M<u64>,
    pub inject_work_us: M<u64>,
    pub cleanup_us: M<u64>,
    pub receiver_and_outcome_us: M<u64>,
    pub clipboard_write_us: M<u64>,
    pub pasteboard_check_us: M<u64>,
    pub pasteboard_check_count: u64,
    pub focus_query_us: M<u64>,
    pub focus_query: FocusQuery,
    pub target_verification: [TargetVerification; 3],
    pub retry_target_verification: TargetVerification,
    pub window_diagnostic_us: M<u64>,
    pub window_diagnostic: WindowDiagnostic,
    pub native_post: NativePost,
    pub retry_post: NativePost,
    pub native_paste_retry_count: u8,
    pub first_failure: M<FirstFailure>,
    pub retry_wait_us: M<u64>,
    pub retry_focus_query_us: M<u64>,
    pub retry_focus_query: FocusQuery,
    pub smart_prepare_validation_us: M<u64>,
    pub smart_insert_validation_us: M<u64>,
    pub smart_post_settle_us: M<u64>,
    pub smart_readback_us: M<u64>,
    pub smart_restore_us: M<u64>,
    pub preparation: M<SmartOutcome>,
    pub confirmation: M<SmartOutcome>,
    pub rollback: M<SmartOutcome>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub expired: bool,
}
fn version_one<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let v = u32::deserialize(d)?;
    if v == 1 {
        Ok(v)
    } else {
        Err(serde::de::Error::custom(
            "unsupported delivery timing schema",
        ))
    }
}
impl Default for DeliveryTimingV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            accepted_route: missing(),
            actual_route: missing(),
            outcome: missing(),
            configured_delay_ms: 0,
            accessibility: missing(),
            stage_total_us: missing(),
            to_post_us: missing(),
            configured_delay_elapsed_us: M::NotApplicable,
            prepare_queue_us: M::NotApplicable,
            prepare_owner_wait_us: M::NotApplicable,
            prepare_work_us: M::NotApplicable,
            activation_wait_us: M::NotApplicable,
            inject_queue_us: missing(),
            inject_owner_wait_us: missing(),
            inject_work_us: missing(),
            cleanup_us: missing(),
            receiver_and_outcome_us: missing(),
            clipboard_write_us: missing(),
            pasteboard_check_us: missing(),
            pasteboard_check_count: 0,
            focus_query_us: missing(),
            focus_query: FocusQuery::default(),
            target_verification: [
                TargetVerification::new(VerificationPhase::BeforeFocus),
                TargetVerification::new(VerificationPhase::AfterFocus),
                TargetVerification::new(VerificationPhase::AfterClipboard),
            ],
            retry_target_verification: TargetVerification::new(VerificationPhase::BeforeFocus),
            window_diagnostic_us: missing(),
            window_diagnostic: WindowDiagnostic::UnavailableOffPath,
            native_post: NativePost::default(),
            retry_post: NativePost::default(),
            native_paste_retry_count: 0,
            first_failure: M::NotApplicable,
            retry_wait_us: M::NotApplicable,
            retry_focus_query_us: M::NotApplicable,
            retry_focus_query: FocusQuery::default(),
            smart_prepare_validation_us: M::NotApplicable,
            smart_insert_validation_us: M::NotApplicable,
            smart_post_settle_us: M::NotApplicable,
            smart_readback_us: M::NotApplicable,
            smart_restore_us: M::NotApplicable,
            preparation: M::NotApplicable,
            confirmation: M::NotApplicable,
            rollback: M::NotApplicable,
            timed_out: false,
            cancelled: false,
            expired: false,
        }
    }
}

pub(crate) fn accepted_route(snapshot: &crate::smart_destination::Snapshot) -> Route {
    use crate::smart_destination::Snapshot::*;
    match snapshot {
        Disabled | Unverified => Route::Normal,
        Unavailable => Route::ClipboardOnly,
        Target {
            current_app: true, ..
        } => Route::SmartCurrent,
        Target { .. } => Route::SmartHandoff,
    }
}

/// Actual Smart routing is established by preparation, not the recording-start
/// flag: a current-field recording can activate that field after a switch away.
pub(crate) fn actual_route(
    snapshot: &crate::smart_destination::Snapshot,
    auto_paste: bool,
    preparation: &M<SmartOutcome>,
) -> M<Route> {
    use crate::smart_destination::Snapshot;
    if !auto_paste {
        return M::measured(Route::ClipboardOnly);
    }
    match snapshot {
        Snapshot::Disabled | Snapshot::Unverified => M::measured(Route::Normal),
        Snapshot::Unavailable => M::measured(Route::ClipboardOnly),
        Snapshot::Target { .. } => match preparation.value() {
            Some(SmartOutcome::Ready) => M::measured(Route::SmartCurrent),
            Some(SmartOutcome::ActivationRequested) => M::measured(Route::SmartHandoff),
            // No confirmed preparation path (for example, permission was
            // unavailable before preparation): do not invent a Smart cohort.
            _ => missing(),
        },
    }
}

#[derive(Clone)]
pub(crate) struct DeliveryTiming(Arc<Mutex<Accumulator>>);
type TimerField = fn(&mut DeliveryTimingV1) -> &mut M<u64>;
// Internal sentinel only; persisted measurements never use a missing-data zero.
const TIMER_RUNNING: u64 = u64::MAX;

struct Accumulator {
    record: DeliveryTimingV1,
    started: Instant,
    closed: bool,
    next_timer: u64,
    timers: Vec<(u64, Instant, TimerField, Arc<AtomicU64>)>,
    posts: Vec<(bool, Arc<PostProgress>)>,
}
impl DeliveryTiming {
    pub(crate) fn new(started: Instant) -> Self {
        Self(Arc::new(Mutex::new(Accumulator {
            record: DeliveryTimingV1::default(),
            started,
            closed: false,
            next_timer: 0,
            timers: Vec::new(),
            posts: Vec::new(),
        })))
    }
    pub(crate) fn update(&self, f: impl FnOnce(&mut DeliveryTimingV1)) {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !a.closed {
            f(&mut a.record);
        }
    }
    pub(crate) fn snapshot(&self) -> DeliveryTimingV1 {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record
            .clone()
    }
    pub(crate) fn close(&self) -> DeliveryTimingV1 {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !a.closed {
            let sealed_at = Instant::now();
            // Acquiring observed posting progress also publishes the proof
            // freeze that preceded it, before we read the registered timers.
            for (retry, progress) in std::mem::take(&mut a.posts) {
                progress.checkpoint(&mut a.record, retry, sealed_at);
            }
            // Preserve elapsed-so-far for the exact in-flight substep when a
            // callback outlives its receiver. Subsequent timer drops are inert.
            for (_, start, field, frozen_us) in std::mem::take(&mut a.timers) {
                let frozen = frozen_us.load(Ordering::Acquire);
                let elapsed = if frozen == TIMER_RUNNING {
                    us(start)
                } else {
                    frozen
                };
                add(field(&mut a.record), elapsed);
            }
            a.record.stage_total_us = M::measured(us(a.started));
            if !matches!(a.record.outcome, M::Measured { .. }) {
                a.record.outcome = M::measured(Outcome::EarlyExit);
            }
            a.closed = true;
        }
        a.record.clone()
    }
    pub(crate) fn posted_at(&self, at: Instant) {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if !a.closed {
            a.record.to_post_us = M::measured(
                at.saturating_duration_since(a.started)
                    .as_micros()
                    .min(u64::MAX as u128) as u64,
            );
        }
    }
    pub(crate) fn bind(&self) -> ThreadScope {
        ThreadScope(CURRENT.with(|c| c.replace(Some(self.clone()))))
    }
}

/// Registered before the final proof. Only atomic stores occur between that
/// proof and key-up, so a receiver can seal observed posting progress without
/// making the delivery thread wait for the accumulator.
pub(crate) struct PostingProgress(Option<Arc<PostProgress>>);
#[derive(Clone, Copy)]
pub(crate) enum PostStep {
    Down = 0,
    Hold = 1,
    Up = 2,
}
struct PostProgress {
    origin: Instant,
    // Each step has an immutable start offset and then its completed duration.
    samples: [AtomicU64; 6],
}
impl PostProgress {
    fn offset(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.origin)
            .as_micros()
            .min((TIMER_RUNNING - 1) as u128) as u64
    }
    fn checkpoint(&self, record: &mut DeliveryTimingV1, retry: bool, at: Instant) {
        let cutoff = self.offset(at);
        let mut samples = [TIMER_RUNNING; 6];
        // Read latest first: an acquired later sample also publishes every
        // preceding sample. Ignore starts after the sealing instant.
        for i in (0..samples.len()).rev() {
            samples[i] = self.samples[i].load(Ordering::Acquire);
        }
        if samples[0] > cutoff {
            return;
        }
        if record.to_post_us.value().is_none() {
            record.to_post_us = M::measured(samples[0]);
        }
        let post = if retry {
            &mut record.retry_post
        } else {
            &mut record.native_post
        };
        for (i, field) in [
            &mut post.key_down_post_us,
            &mut post.key_hold_us,
            &mut post.key_up_post_us,
        ]
        .into_iter()
        .enumerate()
        {
            let start = samples[i * 2];
            if start <= cutoff {
                // An unfinished native call/hold is the blocked leaf. A
                // completion racing sealing cannot extend it past the cutoff.
                *field = M::measured(samples[i * 2 + 1].min(cutoff - start));
            }
        }
        if samples[4] <= cutoff && samples[5] <= cutoff - samples[4] {
            post.outcome = M::measured(PostOutcome::Posted);
        }
    }
}
impl PostingProgress {
    pub(crate) fn new(construction_us: u64) -> Self {
        CURRENT.with(|c| {
            let current = c.borrow();
            let Some(timing) = current.as_ref() else {
                return Self(None);
            };
            let mut a = timing.0.lock().unwrap_or_else(|p| p.into_inner());
            if a.closed {
                return Self(None);
            }
            let retry = a.record.native_paste_retry_count > 0;
            let post = if retry {
                &mut a.record.retry_post
            } else {
                &mut a.record.native_post
            };
            post.event_construction_us = M::measured(construction_us);
            let progress = Arc::new(PostProgress {
                origin: a.started,
                samples: std::array::from_fn(|_| AtomicU64::new(TIMER_RUNNING)),
            });
            a.posts.push((retry, progress.clone()));
            Self(Some(progress))
        })
    }
    pub(crate) fn started(&self, step: PostStep, at: Instant) {
        if let Some(progress) = &self.0 {
            progress.samples[step as usize * 2].store(progress.offset(at), Ordering::Release);
        }
    }
    pub(crate) fn finished(&self, step: PostStep, elapsed: u64) {
        if let Some(progress) = &self.0 {
            progress.samples[step as usize * 2 + 1]
                .store(elapsed.min(TIMER_RUNNING - 1), Ordering::Release);
        }
    }
}
/// Seals on every return, dispatch error, timeout and future cancellation.
pub(crate) struct CloseOnDrop(pub DeliveryTiming);
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}
thread_local! { static CURRENT: RefCell<Option<DeliveryTiming>> = const { RefCell::new(None) }; }
pub(crate) struct ThreadScope(Option<DeliveryTiming>);
impl Drop for ThreadScope {
    fn drop(&mut self) {
        CURRENT.with(|c| {
            c.replace(self.0.take());
        });
    }
}
pub(crate) fn record(f: impl FnOnce(&mut DeliveryTimingV1)) {
    CURRENT.with(|c| {
        if let Some(t) = c.borrow().as_ref() {
            t.update(f);
        }
    });
}
pub(crate) fn posted_at(at: Instant) {
    CURRENT.with(|c| {
        if let Some(t) = c.borrow().as_ref() {
            t.posted_at(at);
        }
    });
}
pub(crate) fn is_retry() -> bool {
    CURRENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|t| t.snapshot().native_paste_retry_count > 0)
    })
}
pub(crate) fn add(value: &mut M<u64>, duration: u64) {
    *value = M::measured(value.value().copied().unwrap_or(0).saturating_add(duration));
}
/// Child timers are inclusive; never add them to their parent phase. Open
/// timers are checkpointed at sealing, so a timeout identifies the blocked leaf.
pub(crate) struct Timer {
    owner: Option<(DeliveryTiming, u64)>,
    start: Instant,
    frozen_us: Option<Arc<AtomicU64>>,
}
impl DeliveryTiming {
    pub(crate) fn timer(&self, field: TimerField) -> Timer {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if a.closed {
            return Timer {
                owner: None,
                start: Instant::now(),
                frozen_us: None,
            };
        }
        a.next_timer += 1;
        let id = a.next_timer;
        let start = Instant::now();
        let frozen_us = Arc::new(AtomicU64::new(TIMER_RUNNING));
        a.timers.push((id, start, field, frozen_us.clone()));
        Timer {
            owner: Some((self.clone(), id)),
            start,
            frozen_us: Some(frozen_us),
        }
    }
}
impl Timer {
    pub(crate) fn new(field: TimerField) -> Self {
        CURRENT.with(|c| {
            c.borrow().as_ref().map(|t| t.timer(field)).unwrap_or(Self {
                owner: None,
                start: Instant::now(),
                frozen_us: None,
            })
        })
    }
}
pub(crate) struct StoppedTimer {
    timer: Timer,
    elapsed: u64,
}
impl Drop for StoppedTimer {
    fn drop(&mut self) {
        self.timer.finish(self.elapsed);
    }
}
impl Timer {
    /// Freeze at the proof boundary without locking the accumulator. Sealing
    /// can observe this duration even before key-up publishes the stopped timer.
    pub(crate) fn stopped(self) -> StoppedTimer {
        let elapsed = us(self.start).min(TIMER_RUNNING - 1);
        if let Some(frozen_us) = &self.frozen_us {
            frozen_us.store(elapsed, Ordering::Release);
        }
        StoppedTimer {
            timer: self,
            elapsed,
        }
    }
    fn finish(&mut self, elapsed: u64) {
        if let Some((owner, id)) = self.owner.take() {
            let mut a = owner.0.lock().unwrap_or_else(|p| p.into_inner());
            if a.closed {
                return;
            }
            if let Some(index) = a.timers.iter().position(|(i, _, _, _)| *i == id) {
                let (_, _, field, _) = a.timers.swap_remove(index);
                add(field(&mut a.record), elapsed);
            }
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.finish(us(self.start));
    }
}

pub(crate) fn injection_outcome(outcome: crate::injector::InjectionOutcome) -> Outcome {
    use crate::injector::{
        authority::AttemptStop as S, ClipboardOnlyReason as C, InjectionOutcome as I,
    };
    match outcome {
        I::NoText => Outcome::NoText,
        I::AutoPasted => Outcome::AutoPasted,
        I::ClipboardOnly(reason) => match reason {
            C::AutoPasteDisabled => Outcome::AutoPasteDisabled,
            C::AccessibilityDenied => Outcome::AccessibilityDenied,
            C::TargetChanged => Outcome::TargetChanged,
            C::FocusNotEditable => Outcome::FocusNotEditable,
            C::ClipboardChanged => Outcome::ClipboardChanged,
            C::PasteFailed => Outcome::PasteFailed,
            C::SmartTargetUnavailable => Outcome::SmartTargetUnavailable,
        },
        I::Stopped(reason) => match reason {
            S::Cancelled => Outcome::Cancelled,
            S::Expired => Outcome::Expired,
            S::AlreadyCommitted => Outcome::AlreadyCommitted,
            S::OperationInFlight => Outcome::OperationInFlight,
        },
    }
}
pub(crate) fn set_outcome(r: &mut DeliveryTimingV1, outcome: Outcome) {
    r.outcome = M::measured(outcome);
    r.timed_out |= outcome == Outcome::Timeout;
    r.cancelled |= outcome == Outcome::Cancelled;
    r.expired |= outcome == Outcome::Expired;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switched_away_current_field_records_the_prepared_handoff_route() {
        let snapshot = crate::smart_destination::Snapshot::Target {
            token: 1,
            revision: 1,
            identity: crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            current_app: true,
        };
        let timing = DeliveryTiming::new(Instant::now());
        timing.update(|r| {
            r.accepted_route = M::measured(accepted_route(&snapshot));
            // Native preparation returns this after activating the original
            // current field when the user switched away while speaking.
            r.preparation = M::measured(SmartOutcome::ActivationRequested);
            r.actual_route = actual_route(&snapshot, true, &r.preparation);
        });
        let record = timing.close();
        assert_eq!(record.accepted_route, M::measured(Route::SmartCurrent));
        assert_eq!(record.actual_route, M::measured(Route::SmartHandoff));
    }

    #[test]
    fn actual_route_uses_preparation_for_both_start_snapshot_shapes() {
        for current_app in [false, true] {
            let snapshot = crate::smart_destination::Snapshot::Target {
                token: 1,
                revision: 1,
                identity: crate::frontmost::DeliveryTargetSnapshot::Incomplete,
                current_app,
            };
            assert_eq!(
                actual_route(&snapshot, true, &M::measured(SmartOutcome::Ready)),
                M::measured(Route::SmartCurrent)
            );
            assert_eq!(
                actual_route(
                    &snapshot,
                    true,
                    &M::measured(SmartOutcome::ActivationRequested)
                ),
                M::measured(Route::SmartHandoff)
            );
            assert!(matches!(
                actual_route(&snapshot, true, &M::NotApplicable),
                M::Unavailable { .. }
            ));
            assert_eq!(
                actual_route(
                    &snapshot,
                    false,
                    &M::measured(SmartOutcome::ActivationRequested)
                ),
                M::measured(Route::ClipboardOnly)
            );
        }
        // Refused preparation may fall back to normal paste or clipboard only;
        // the final snapshot records that fallback, not a Smart timing cohort.
        assert_eq!(
            actual_route(
                &crate::smart_destination::Snapshot::Unverified,
                true,
                &M::measured(SmartOutcome::Refused)
            ),
            M::measured(Route::Normal)
        );
        assert_eq!(
            actual_route(
                &crate::smart_destination::Snapshot::Unavailable,
                true,
                &M::measured(SmartOutcome::Timeout)
            ),
            M::measured(Route::ClipboardOnly)
        );
    }

    #[test]
    fn shared_native_frontend_fixture_decodes() {
        let record: DeliveryTimingV1 = serde_json::from_str(include_str!(
            "../../../src/lib/fixtures/delivery-timing-v1.json"
        ))
        .unwrap();
        assert_eq!(record.schema_version, 1);
        assert_eq!(
            record.target_verification[2].phase,
            VerificationPhase::AfterClipboard
        );
    }
    #[test]
    fn timeout_checkpoints_in_flight_leaf_and_late_timer_cannot_reopen() {
        let timing = DeliveryTiming::new(Instant::now());
        let timer = timing.timer(|r| &mut r.smart_insert_validation_us);
        timing.update(|r| set_outcome(r, Outcome::Timeout));
        let sealed = timing.close();
        assert!(sealed.smart_insert_validation_us.value().is_some());
        drop(timer);
        assert_eq!(timing.snapshot(), sealed);
    }
    #[test]
    fn stopped_proof_timer_excludes_subsequent_key_work() {
        let timing = DeliveryTiming::new(Instant::now());
        let proof = timing
            .timer(|r| &mut r.native_post.final_focus_proof_us)
            .stopped();
        let expected = proof.elapsed;
        drop(proof);
        assert_eq!(
            timing.snapshot().native_post.final_focus_proof_us,
            M::measured(expected)
        );
    }
    #[test]
    fn sealing_before_key_up_preserves_frozen_proof_duration() {
        for retry in [false, true] {
            let timing = DeliveryTiming::new(Instant::now());
            let timer = timing.timer(if retry {
                |r| &mut r.retry_post.final_focus_proof_us
            } else {
                |r| &mut r.native_post.final_focus_proof_us
            });
            // Freezing must not acquire the accumulator mutex at the final
            // proof/post boundary. Keep it held while another thread freezes.
            let lock = timing.0.lock().unwrap();
            let (sender, receiver) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                sender.send(timer.stopped()).unwrap();
            });
            let frozen = receiver.recv_timeout(std::time::Duration::from_secs(1));
            drop(lock);
            let proof = frozen.expect("stopping must not wait on the accumulator");
            worker.join().unwrap();
            let expected = proof.elapsed;
            std::thread::sleep(std::time::Duration::from_millis(2));
            timing.update(|r| set_outcome(r, Outcome::Cancelled));
            let sealed = timing.close();
            let post = if retry {
                &sealed.retry_post
            } else {
                &sealed.native_post
            };
            assert_eq!(post.final_focus_proof_us, M::measured(expected));
            drop(proof);
            assert_eq!(timing.snapshot(), sealed);
        }
    }
    #[test]
    fn timeout_during_each_post_step_retains_observed_post_and_frozen_proof() {
        use std::time::Duration;
        for retry in [false, true] {
            // Pause in key-down, hold, key-up, or after key-up but before the
            // existing completion update. Exercise both native attempt slots.
            for completed_steps in 0..=3 {
                let timing = DeliveryTiming::new(Instant::now() - Duration::from_millis(1));
                let _scope = timing.bind();
                timing.update(|r| r.native_paste_retry_count = u8::from(retry));
                let progress = PostingProgress::new(5);
                let timer = timing.timer(if retry {
                    |r| &mut r.retry_post.final_focus_proof_us
                } else {
                    |r| &mut r.native_post.final_focus_proof_us
                });
                let lock = timing.0.lock().unwrap();
                let (sender, receiver) = std::sync::mpsc::channel();
                let worker = std::thread::spawn(move || {
                    let proof = timer.stopped();
                    for (i, step) in [PostStep::Down, PostStep::Hold, PostStep::Up]
                        .into_iter()
                        .enumerate()
                    {
                        progress.started(step, Instant::now());
                        if i == completed_steps {
                            break;
                        }
                        progress.finished(step, [7, 11, 13][i]);
                    }
                    sender.send((progress, proof)).unwrap();
                });
                let observed = receiver.recv_timeout(Duration::from_secs(1));
                drop(lock);
                let (progress, proof) =
                    observed.expect("proof and post progress must not wait for the accumulator");
                worker.join().unwrap();
                let frozen_proof = proof.elapsed;
                std::thread::sleep(Duration::from_millis(2));
                timing.update(|r| set_outcome(r, Outcome::Timeout));
                let sealed = timing.close();
                assert!(sealed.to_post_us.value().is_some());
                assert_eq!(sealed.outcome, M::measured(Outcome::Timeout));
                let post = if retry {
                    &sealed.retry_post
                } else {
                    &sealed.native_post
                };
                assert_eq!(post.event_construction_us, M::measured(5));
                assert_eq!(post.final_focus_proof_us, M::measured(frozen_proof));
                for (i, duration) in [
                    &post.key_down_post_us,
                    &post.key_hold_us,
                    &post.key_up_post_us,
                ]
                .into_iter()
                .enumerate()
                {
                    if i < completed_steps {
                        assert_eq!(*duration, M::measured([7, 11, 13][i]));
                    } else if i == completed_steps {
                        assert!(duration.value().is_some_and(|v| *v >= 2_000));
                    } else {
                        assert!(matches!(duration, M::Unavailable { .. }));
                    }
                }
                if completed_steps == 3 {
                    assert_eq!(post.outcome, M::measured(PostOutcome::Posted));
                } else {
                    assert!(matches!(post.outcome, M::Unavailable { .. }));
                }
                // Late completion can neither reopen this record nor change
                // the timeout outcome into successful insertion.
                progress.finished(PostStep::Up, 99);
                posted_at(Instant::now());
                drop(proof);
                record(|r| set_outcome(r, Outcome::AutoPasted));
                assert_eq!(timing.close(), sealed);
            }
        }
    }
    #[test]
    fn registered_but_refused_post_does_not_invent_posting_evidence() {
        let timing = DeliveryTiming::new(Instant::now());
        let _scope = timing.bind();
        let _progress = PostingProgress::new(5);
        record(|r| r.native_post.outcome = M::measured(PostOutcome::Refused));
        let sealed = timing.close();
        assert!(matches!(sealed.to_post_us, M::Unavailable { .. }));
        assert!(matches!(
            sealed.native_post.key_down_post_us,
            M::Unavailable { .. }
        ));
        assert_eq!(
            sealed.native_post.outcome,
            M::measured(PostOutcome::Refused)
        );
    }
    #[test]
    fn version_and_content_free_contract() {
        let record = DeliveryTimingV1::default();
        let mut json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["toPostUs"]["status"], "unavailable");
        assert_eq!(
            serde_json::from_value::<DeliveryTimingV1>(json.clone()).unwrap(),
            record
        );
        json["schemaVersion"] = 2.into();
        assert!(serde_json::from_value::<DeliveryTimingV1>(json.clone()).is_err());
        json["schemaVersion"] = 1.into();
        json["title"] = "forbidden".into();
        assert!(serde_json::from_value::<DeliveryTimingV1>(json).is_err());
    }
    #[test]
    fn closes_refusal_error_timeout_and_ignores_late_callback() {
        for outcome in [
            Outcome::TargetChanged,
            Outcome::ClipboardWriteFailed,
            Outcome::Timeout,
            Outcome::EarlyExit,
        ] {
            let old = DeliveryTiming::new(Instant::now());
            let newer = DeliveryTiming::new(Instant::now());
            {
                let _close = CloseOnDrop(old.clone());
                old.update(|r| set_outcome(r, outcome));
            }
            let closed = old.snapshot();
            {
                let _scope = old.bind();
                record(|r| set_outcome(r, Outcome::AutoPasted));
                posted_at(Instant::now());
            }
            assert_eq!(old.snapshot(), closed);
            assert_eq!(newer.snapshot(), DeliveryTimingV1::default());
            assert_eq!(closed.outcome, M::measured(outcome));
            assert!(closed.stage_total_us.value().is_some());
        }
    }
    #[test]
    fn missing_and_measured_zero_remain_distinct() {
        let t = DeliveryTiming::new(Instant::now());
        let _scope = t.bind();
        record(|r| add(&mut r.clipboard_write_us, 0));
        assert_eq!(t.snapshot().clipboard_write_us, M::measured(0));
        assert!(matches!(t.snapshot().inject_work_us, M::Unavailable { .. }));
    }
}
