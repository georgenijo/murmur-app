//! Content-free delivery attribution. No API accepts arbitrary diagnostic text.
//! Each callback holds its recording's collector, never a global "latest run".
use super::{MeasurementV1 as M, UnavailableReasonV1 as U};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
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

pub(crate) fn route(snapshot: &crate::smart_destination::Snapshot) -> Route {
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

#[derive(Clone)]
pub(crate) struct DeliveryTiming(Arc<Mutex<Accumulator>>);
type TimerField = fn(&mut DeliveryTimingV1) -> &mut M<u64>;

struct Accumulator {
    record: DeliveryTimingV1,
    started: Instant,
    closed: bool,
    next_timer: u64,
    timers: Vec<(u64, Instant, TimerField)>,
}
impl DeliveryTiming {
    pub(crate) fn new(started: Instant) -> Self {
        Self(Arc::new(Mutex::new(Accumulator {
            record: DeliveryTimingV1::default(),
            started,
            closed: false,
            next_timer: 0,
            timers: Vec::new(),
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
            // Preserve elapsed-so-far for the exact in-flight substep when a
            // callback outlives its receiver. Subsequent timer drops are inert.
            for (_, start, field) in std::mem::take(&mut a.timers) {
                add(field(&mut a.record), us(start));
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
}
impl DeliveryTiming {
    pub(crate) fn timer(&self, field: TimerField) -> Timer {
        let mut a = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if a.closed {
            return Timer {
                owner: None,
                start: Instant::now(),
            };
        }
        a.next_timer += 1;
        let id = a.next_timer;
        a.timers.push((id, Instant::now(), field));
        Timer {
            owner: Some((self.clone(), id)),
            start: Instant::now(),
        }
    }
}
impl Timer {
    pub(crate) fn new(field: TimerField) -> Self {
        CURRENT.with(|c| {
            c.borrow().as_ref().map(|t| t.timer(field)).unwrap_or(Self {
                owner: None,
                start: Instant::now(),
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
    /// Freeze locally at the proof boundary; publish only after key-up.
    pub(crate) fn stopped(self) -> StoppedTimer {
        let elapsed = us(self.start);
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
            if let Some(index) = a.timers.iter().position(|(i, _, _)| *i == id) {
                let (_, _, field) = a.timers.swap_remove(index);
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
