//! Bounded, content-free attribution of the post-transcription delivery tail (#1081).
//!
//! `PerformanceStageV1::ClipboardPaste` reports one number for everything between
//! the end of transcript transformation and the delivery outcome. This module
//! records where that time goes: main-thread callback queueing, ownership waits,
//! clipboard and target-verification work, the final live-focus proof, the
//! key-event block, and Smart-destination validation.
//!
//! Every value is a monotonic microsecond or count behind the existing
//! [`MeasurementV1`] wrapper, or a bounded enum. Text, titles, paths, window
//! identities and every other piece of target content are not representable.
//! A field is `notApplicable` when the path never reached the step,
//! `unavailable` when the step was dispatched but had not reported when the
//! record closed, and `measured` otherwise: zero is a measurement, never a
//! placeholder for "missing".
//!
//! Each delivery owns one [`DeliveryTimingCollector`]. The collector is closed
//! when the delivery outcome is decided; callbacks that run afterwards (a late
//! main-thread turn after a timeout, for example) find it closed and change
//! nothing, so they can neither alter the finished record nor reach a newer
//! recording's record.
use super::types::{MeasurementV1, UnavailableReasonV1};
use crate::injector::authority::AttemptStop;
use crate::injector::{ClipboardOnlyReason, InjectionOutcome};
use crate::smart_destination::{PrepareOutcome, Snapshot};
use serde::{Deserialize, Deserializer, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(crate) const DELIVERY_TIMING_SCHEMA_VERSION: u32 = 1;
/// Largest integer a JavaScript consumer can represent exactly.
const MAX_VALUE: u64 = 9_007_199_254_740_991;
/// Unfinished or unconsumed records kept in memory, mirroring the production
/// accumulator's bound so a quick next recording cannot grow the store.
const STORE_LIMIT: usize = 8;

fn version_one<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == DELIVERY_TIMING_SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(
            "unsupported delivery timing schema",
        ))
    }
}

/// How the delivery was routed. Mirrors `Snapshot::route_code()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryRouteV1 {
    Disabled,
    ClipboardOnly,
    NormalPaste,
    SmartTarget,
}

impl DeliveryRouteV1 {
    pub(crate) fn of(snapshot: &Snapshot) -> Self {
        match snapshot {
            Snapshot::Disabled => Self::Disabled,
            Snapshot::Unavailable => Self::ClipboardOnly,
            Snapshot::Unverified => Self::NormalPaste,
            Snapshot::Target { .. } => Self::SmartTarget,
        }
    }
}

/// How the delivery ended. The injector outcomes are flattened so a consumer
/// reads one bounded value; the receiver-side failures that never produced an
/// injector outcome have their own variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryOutcomeV1 {
    /// The one allowed key post occurred. It does not confirm insertion.
    AutoPasted,
    NoText,
    ClipboardOnlyAutoPasteDisabled,
    ClipboardOnlyAccessibilityDenied,
    ClipboardOnlyTargetChanged,
    ClipboardOnlyFocusNotEditable,
    ClipboardOnlyClipboardChanged,
    ClipboardOnlyPasteFailed,
    ClipboardOnlySmartTargetUnavailable,
    StoppedCancelled,
    StoppedExpired,
    StoppedAlreadyCommitted,
    StoppedOperationInFlight,
    ClipboardWriteFailed,
    /// The queued callback ran but the recording no longer owned delivery.
    CancelledBeforeInjection,
    /// The main-thread callback dropped its reply channel.
    SenderDropped,
    /// No outcome arrived within the injection callback's two-second bound.
    TimedOut,
    /// The callback could not be dispatched to the main thread.
    DispatchFailed,
    /// The record was closed without an explicit outcome (unexpected exit).
    Incomplete,
}

impl DeliveryOutcomeV1 {
    pub(crate) fn of_injection(outcome: InjectionOutcome) -> Self {
        match outcome {
            InjectionOutcome::NoText => Self::NoText,
            InjectionOutcome::AutoPasted => Self::AutoPasted,
            InjectionOutcome::ClipboardOnly(reason) => match reason {
                ClipboardOnlyReason::AutoPasteDisabled => Self::ClipboardOnlyAutoPasteDisabled,
                ClipboardOnlyReason::AccessibilityDenied => Self::ClipboardOnlyAccessibilityDenied,
                ClipboardOnlyReason::TargetChanged => Self::ClipboardOnlyTargetChanged,
                ClipboardOnlyReason::FocusNotEditable => Self::ClipboardOnlyFocusNotEditable,
                ClipboardOnlyReason::ClipboardChanged => Self::ClipboardOnlyClipboardChanged,
                ClipboardOnlyReason::PasteFailed => Self::ClipboardOnlyPasteFailed,
                ClipboardOnlyReason::SmartTargetUnavailable => {
                    Self::ClipboardOnlySmartTargetUnavailable
                }
            },
            InjectionOutcome::Stopped(stop) => match stop {
                AttemptStop::Cancelled => Self::StoppedCancelled,
                AttemptStop::Expired => Self::StoppedExpired,
                AttemptStop::AlreadyCommitted => Self::StoppedAlreadyCommitted,
                AttemptStop::OperationInFlight => Self::StoppedOperationInFlight,
            },
        }
    }
}

/// What the Smart-destination preparation turn reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SmartPrepareOutcomeV1 {
    /// No Smart preparation ran on this delivery.
    NotRun,
    Ready,
    WaitForActivation,
    Refused,
    /// Refused after the user switched away from a current-app destination:
    /// the delivery stays clipboard-only instead of falling back to paste.
    RefusedClipboardOnly,
    /// The callback ran but the recording no longer owned delivery.
    OwnerNotCurrent,
    SenderDropped,
    TimedOut,
    DispatchFailed,
}

impl SmartPrepareOutcomeV1 {
    pub(crate) fn of(outcome: PrepareOutcome) -> Self {
        match outcome {
            PrepareOutcome::Ready => Self::Ready,
            PrepareOutcome::WaitForActivation => Self::WaitForActivation,
            PrepareOutcome::Refused => Self::Refused,
            PrepareOutcome::RefusedClipboardOnly => Self::RefusedClipboardOnly,
        }
    }
}

/// One dispatched main-thread callback: time queued before it started, time
/// waiting for the dictation ownership lock, and the work done while owning it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryCallbackV1 {
    pub queue_wait_us: MeasurementV1<u64>,
    pub owner_wait_us: MeasurementV1<u64>,
    pub work_us: MeasurementV1<u64>,
}

/// One target verification: wall time and the number of lookup retries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryVerificationV1 {
    pub elapsed_us: MeasurementV1<u64>,
    pub lookup_retry_count: MeasurementV1<u64>,
}

/// Steps of the clipboard write and native paste, shared by the normal and
/// Smart routes where the same primitive runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryInjectV1 {
    pub focused_field_query_us: MeasurementV1<u64>,
    pub clipboard_write_us: MeasurementV1<u64>,
    pub before_focus_verification: DeliveryVerificationV1,
    pub after_focus_verification: DeliveryVerificationV1,
    pub after_clipboard_verification: DeliveryVerificationV1,
    pub event_construction_us: MeasurementV1<u64>,
    /// Live-focus proof and pasteboard-generation check made immediately
    /// before the key events are posted.
    pub final_focus_proof_us: MeasurementV1<u64>,
    /// Key-down, hold and key-up block.
    pub key_event_us: MeasurementV1<u64>,
    pub native_paste_retry_count: MeasurementV1<u64>,
    pub native_paste_retry_wait_us: MeasurementV1<u64>,
}

/// Smart-destination steps. Absent from the normal route as `notApplicable`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverySmartV1 {
    pub prepare_outcome: SmartPrepareOutcomeV1,
    /// The fixed wait after a preparation that requested app activation.
    pub activation_wait_us: MeasurementV1<u64>,
    pub prepare_validation_us: MeasurementV1<u64>,
    pub insert_validation_us: MeasurementV1<u64>,
    pub settle_us: MeasurementV1<u64>,
    pub readback_us: MeasurementV1<u64>,
    pub restore_us: MeasurementV1<u64>,
    /// Guarded rollback of a prepared destination: the abort the injection
    /// callback runs before it replies, and the rollback an insertion refusal
    /// performs.
    pub cleanup_us: MeasurementV1<u64>,
    /// Focused-window lookups made while capturing Smart delivery-target
    /// snapshots. Already counted inside the preparation, insertion and abort
    /// steps above; reported separately because it is the one lookup there
    /// that no delivery decision reads.
    pub snapshot_window_lookup_us: MeasurementV1<u64>,
}

/// Content-free timing of one dictation delivery.
///
/// Optional on `PerformanceRunV1` and `ActiveRunV1`: historical rows and
/// dictations that never reached delivery omit it entirely. Existing
/// `inject timing` log fields and the `clipboardPaste` stage are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryTimingV1 {
    #[serde(deserialize_with = "version_one")]
    pub schema_version: u32,
    /// Route chosen when recording was accepted.
    pub accepted_route: DeliveryRouteV1,
    /// Route actually used; differs after a Smart preparation fallback.
    pub delivery_route: DeliveryRouteV1,
    pub outcome: DeliveryOutcomeV1,
    /// Delivery start to the moment the outcome was observed.
    pub outcome_us: MeasurementV1<u64>,
    /// Delivery start to the end of the delivery block. The difference from
    /// `outcomeUs` is post-outcome bookkeeping inside the `clipboardPaste` stage.
    pub total_us: MeasurementV1<u64>,
    /// The configured focus-settle sleep, which runs off the main thread.
    pub configured_delay_us: MeasurementV1<u64>,
    pub prepare_callback: DeliveryCallbackV1,
    pub injection_callback: DeliveryCallbackV1,
    pub inject: DeliveryInjectV1,
    /// The single diagnostic focused-window lookup, taken after the delivery
    /// decision. `unavailable`/`dependencyPending` means the record closed
    /// before the lookup reported.
    pub window_diagnostic_us: MeasurementV1<u64>,
    pub smart: DeliverySmartV1,
}

/// One accumulated measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Slot {
    /// The path never reached this step.
    #[default]
    Idle,
    /// Dispatched but not yet reported.
    Pending,
    Done(u64),
}

impl Slot {
    fn add(&mut self, value: u64) {
        let value = value.min(MAX_VALUE);
        *self = match *self {
            Self::Done(total) => Self::Done(total.saturating_add(value).min(MAX_VALUE)),
            Self::Idle | Self::Pending => Self::Done(value),
        };
    }

    fn pending(&mut self) {
        if *self == Self::Idle {
            *self = Self::Pending;
        }
    }

    fn measurement(self) -> MeasurementV1<u64> {
        match self {
            Self::Idle => MeasurementV1::NotApplicable,
            Self::Pending => MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::DependencyPending,
            },
            Self::Done(value) => MeasurementV1::measured(value),
        }
    }
}

/// Where a step is attributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    ConfiguredDelay,
    ActivationWait,
    PrepareQueueWait,
    PrepareOwnerWait,
    PrepareWork,
    InjectionQueueWait,
    InjectionOwnerWait,
    InjectionWork,
    FocusedFieldQuery,
    ClipboardWrite,
    BeforeFocusVerification,
    AfterFocusVerification,
    AfterClipboardVerification,
    EventConstruction,
    FinalFocusProof,
    KeyEvent,
    NativePasteRetryWait,
    PrepareValidation,
    InsertValidation,
    Settle,
    Readback,
    Restore,
    Cleanup,
    SnapshotWindowLookup,
}

/// Counts reported with a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Counter {
    BeforeFocusLookupRetries,
    AfterFocusLookupRetries,
    AfterClipboardLookupRetries,
    NativePasteRetries,
}

/// The three target verifications of the normal route, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationPoint {
    BeforeFocus,
    AfterFocus,
    AfterClipboard,
}

impl VerificationPoint {
    fn step(self) -> Step {
        match self {
            Self::BeforeFocus => Step::BeforeFocusVerification,
            Self::AfterFocus => Step::AfterFocusVerification,
            Self::AfterClipboard => Step::AfterClipboardVerification,
        }
    }

    fn counter(self) -> Counter {
        match self {
            Self::BeforeFocus => Counter::BeforeFocusLookupRetries,
            Self::AfterFocus => Counter::AfterFocusLookupRetries,
            Self::AfterClipboard => Counter::AfterClipboardLookupRetries,
        }
    }
}

/// A dispatched main-thread callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Callback {
    Prepare,
    Injection,
}

impl Callback {
    fn steps(self) -> [Step; 3] {
        match self {
            Self::Prepare => [
                Step::PrepareQueueWait,
                Step::PrepareOwnerWait,
                Step::PrepareWork,
            ],
            Self::Injection => [
                Step::InjectionQueueWait,
                Step::InjectionOwnerWait,
                Step::InjectionWork,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Steps are accepted.
    Open,
    /// The outcome is decided; only the explicitly pending window diagnostic
    /// may still report.
    Closed,
    /// The record was built; nothing is accepted.
    Finished,
}

struct State {
    phase: Phase,
    delivery_route: DeliveryRouteV1,
    outcome: Option<DeliveryOutcomeV1>,
    prepare_outcome: SmartPrepareOutcomeV1,
    outcome_us: Slot,
    window_diagnostic: Slot,
    configured_delay: Slot,
    activation_wait: Slot,
    prepare_queue_wait: Slot,
    prepare_owner_wait: Slot,
    prepare_work: Slot,
    injection_queue_wait: Slot,
    injection_owner_wait: Slot,
    injection_work: Slot,
    focused_field_query: Slot,
    clipboard_write: Slot,
    before_focus_verification: Slot,
    after_focus_verification: Slot,
    after_clipboard_verification: Slot,
    event_construction: Slot,
    final_focus_proof: Slot,
    key_event: Slot,
    native_paste_retry_wait: Slot,
    prepare_validation: Slot,
    insert_validation: Slot,
    settle: Slot,
    readback: Slot,
    restore: Slot,
    cleanup: Slot,
    snapshot_window_lookup: Slot,
    before_focus_lookup_retries: Slot,
    after_focus_lookup_retries: Slot,
    after_clipboard_lookup_retries: Slot,
    native_paste_retries: Slot,
}

impl State {
    fn new(route: DeliveryRouteV1) -> Self {
        Self {
            phase: Phase::Open,
            delivery_route: route,
            outcome: None,
            prepare_outcome: SmartPrepareOutcomeV1::NotRun,
            outcome_us: Slot::Idle,
            window_diagnostic: Slot::Idle,
            configured_delay: Slot::Idle,
            activation_wait: Slot::Idle,
            prepare_queue_wait: Slot::Idle,
            prepare_owner_wait: Slot::Idle,
            prepare_work: Slot::Idle,
            injection_queue_wait: Slot::Idle,
            injection_owner_wait: Slot::Idle,
            injection_work: Slot::Idle,
            focused_field_query: Slot::Idle,
            clipboard_write: Slot::Idle,
            before_focus_verification: Slot::Idle,
            after_focus_verification: Slot::Idle,
            after_clipboard_verification: Slot::Idle,
            event_construction: Slot::Idle,
            final_focus_proof: Slot::Idle,
            key_event: Slot::Idle,
            native_paste_retry_wait: Slot::Idle,
            prepare_validation: Slot::Idle,
            insert_validation: Slot::Idle,
            settle: Slot::Idle,
            readback: Slot::Idle,
            restore: Slot::Idle,
            cleanup: Slot::Idle,
            snapshot_window_lookup: Slot::Idle,
            before_focus_lookup_retries: Slot::Idle,
            after_focus_lookup_retries: Slot::Idle,
            after_clipboard_lookup_retries: Slot::Idle,
            native_paste_retries: Slot::Idle,
        }
    }

    fn step(&mut self, step: Step) -> &mut Slot {
        match step {
            Step::ConfiguredDelay => &mut self.configured_delay,
            Step::ActivationWait => &mut self.activation_wait,
            Step::PrepareQueueWait => &mut self.prepare_queue_wait,
            Step::PrepareOwnerWait => &mut self.prepare_owner_wait,
            Step::PrepareWork => &mut self.prepare_work,
            Step::InjectionQueueWait => &mut self.injection_queue_wait,
            Step::InjectionOwnerWait => &mut self.injection_owner_wait,
            Step::InjectionWork => &mut self.injection_work,
            Step::FocusedFieldQuery => &mut self.focused_field_query,
            Step::ClipboardWrite => &mut self.clipboard_write,
            Step::BeforeFocusVerification => &mut self.before_focus_verification,
            Step::AfterFocusVerification => &mut self.after_focus_verification,
            Step::AfterClipboardVerification => &mut self.after_clipboard_verification,
            Step::EventConstruction => &mut self.event_construction,
            Step::FinalFocusProof => &mut self.final_focus_proof,
            Step::KeyEvent => &mut self.key_event,
            Step::NativePasteRetryWait => &mut self.native_paste_retry_wait,
            Step::PrepareValidation => &mut self.prepare_validation,
            Step::InsertValidation => &mut self.insert_validation,
            Step::Settle => &mut self.settle,
            Step::Readback => &mut self.readback,
            Step::Restore => &mut self.restore,
            Step::Cleanup => &mut self.cleanup,
            Step::SnapshotWindowLookup => &mut self.snapshot_window_lookup,
        }
    }

    fn counter(&mut self, counter: Counter) -> &mut Slot {
        match counter {
            Counter::BeforeFocusLookupRetries => &mut self.before_focus_lookup_retries,
            Counter::AfterFocusLookupRetries => &mut self.after_focus_lookup_retries,
            Counter::AfterClipboardLookupRetries => &mut self.after_clipboard_lookup_retries,
            Counter::NativePasteRetries => &mut self.native_paste_retries,
        }
    }

    fn callback(&self, queue: Slot, owner: Slot, work: Slot) -> DeliveryCallbackV1 {
        DeliveryCallbackV1 {
            queue_wait_us: queue.measurement(),
            owner_wait_us: owner.measurement(),
            work_us: work.measurement(),
        }
    }

    fn verification(&self, elapsed: Slot, retries: Slot) -> DeliveryVerificationV1 {
        DeliveryVerificationV1 {
            elapsed_us: elapsed.measurement(),
            lookup_retry_count: retries.measurement(),
        }
    }

    fn build(&self, accepted_route: DeliveryRouteV1, total_us: u64) -> DeliveryTimingV1 {
        // A native paste that reached event construction without retrying
        // measured zero retries; one that never got there did not apply.
        let pasted = matches!(self.event_construction, Slot::Done(_));
        let retry_default = if pasted { Slot::Done(0) } else { Slot::Idle };
        let retries = if self.native_paste_retries == Slot::Idle {
            retry_default
        } else {
            self.native_paste_retries
        };
        let retry_wait = if self.native_paste_retry_wait == Slot::Idle {
            retry_default
        } else {
            self.native_paste_retry_wait
        };
        DeliveryTimingV1 {
            schema_version: DELIVERY_TIMING_SCHEMA_VERSION,
            accepted_route,
            delivery_route: self.delivery_route,
            outcome: self.outcome.unwrap_or(DeliveryOutcomeV1::Incomplete),
            outcome_us: self.outcome_us.measurement(),
            total_us: MeasurementV1::measured(total_us.min(MAX_VALUE)),
            configured_delay_us: self.configured_delay.measurement(),
            prepare_callback: self.callback(
                self.prepare_queue_wait,
                self.prepare_owner_wait,
                self.prepare_work,
            ),
            injection_callback: self.callback(
                self.injection_queue_wait,
                self.injection_owner_wait,
                self.injection_work,
            ),
            inject: DeliveryInjectV1 {
                focused_field_query_us: self.focused_field_query.measurement(),
                clipboard_write_us: self.clipboard_write.measurement(),
                before_focus_verification: self.verification(
                    self.before_focus_verification,
                    self.before_focus_lookup_retries,
                ),
                after_focus_verification: self.verification(
                    self.after_focus_verification,
                    self.after_focus_lookup_retries,
                ),
                after_clipboard_verification: self.verification(
                    self.after_clipboard_verification,
                    self.after_clipboard_lookup_retries,
                ),
                event_construction_us: self.event_construction.measurement(),
                final_focus_proof_us: self.final_focus_proof.measurement(),
                key_event_us: self.key_event.measurement(),
                native_paste_retry_count: retries.measurement(),
                native_paste_retry_wait_us: retry_wait.measurement(),
            },
            window_diagnostic_us: self.window_diagnostic.measurement(),
            smart: DeliverySmartV1 {
                prepare_outcome: self.prepare_outcome,
                activation_wait_us: self.activation_wait.measurement(),
                prepare_validation_us: self.prepare_validation.measurement(),
                insert_validation_us: self.insert_validation.measurement(),
                settle_us: self.settle.measurement(),
                readback_us: self.readback.measurement(),
                restore_us: self.restore.measurement(),
                cleanup_us: self.cleanup.measurement(),
                snapshot_window_lookup_us: self.snapshot_window_lookup.measurement(),
            },
        }
    }
}

fn micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros())
        .unwrap_or(u64::MAX)
        .min(MAX_VALUE)
}

/// Per-delivery accumulator shared by the async delivery flow and the
/// main-thread callbacks it dispatches.
pub(crate) struct DeliveryTimingCollector {
    started: Instant,
    accepted_route: DeliveryRouteV1,
    state: Mutex<State>,
}

impl DeliveryTimingCollector {
    pub(crate) fn new(accepted_route: DeliveryRouteV1) -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            accepted_route,
            state: Mutex::new(State::new(accepted_route)),
        })
    }

    fn locked(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Add an elapsed span to a step. Ignored once the outcome is decided.
    pub(crate) fn record(&self, step: Step, elapsed: Duration) {
        let mut state = self.locked();
        if state.phase == Phase::Open {
            state.step(step).add(micros(elapsed));
        }
    }

    /// Add to a count. Ignored once the outcome is decided.
    pub(crate) fn record_count(&self, counter: Counter, value: u64) {
        let mut state = self.locked();
        if state.phase == Phase::Open {
            state.counter(counter).add(value);
        }
    }

    pub(crate) fn set_delivery_route(&self, route: DeliveryRouteV1) {
        let mut state = self.locked();
        if state.phase == Phase::Open {
            state.delivery_route = route;
        }
    }

    pub(crate) fn set_prepare_outcome(&self, outcome: SmartPrepareOutcomeV1) {
        let mut state = self.locked();
        if state.phase == Phase::Open {
            state.prepare_outcome = outcome;
        }
    }

    /// Mark a dispatched callback as expected so a callback that never starts
    /// reads as pending instead of as a step the path never reached.
    pub(crate) fn dispatch_callback(&self, callback: Callback) {
        let mut state = self.locked();
        if state.phase == Phase::Open {
            for step in callback.steps() {
                state.step(step).pending();
            }
        }
    }

    /// Enter on the thread that runs a dispatched callback. Records the queue
    /// wait and routes this thread's step recordings to the collector until
    /// the returned scope drops.
    pub(crate) fn enter_callback(
        self: &Arc<Self>,
        callback: Callback,
        dispatched_at: Instant,
    ) -> CallbackScope {
        let [queue_wait, _, _] = callback.steps();
        self.record(queue_wait, dispatched_at.elapsed());
        let previous = SCOPE.with(|scope| {
            scope.borrow_mut().replace(Scope {
                collector: Arc::clone(self),
                callback,
            })
        });
        CallbackScope { previous }
    }

    /// A callback that finished running reports nothing more: owner wait or
    /// work it never reached did not apply, rather than being pending.
    fn callback_finished(&self, callback: Callback) {
        let mut state = self.locked();
        if state.phase != Phase::Open {
            return;
        }
        let [_, owner_wait, work] = callback.steps();
        for step in [owner_wait, work] {
            let slot = state.step(step);
            if *slot == Slot::Pending {
                *slot = Slot::Idle;
            }
        }
    }

    /// The window diagnostic is the one measurement taken after the delivery
    /// decision. Marking it pending lets a record that closes first say so.
    pub(crate) fn begin_window_diagnostic(&self) {
        let mut state = self.locked();
        if state.phase != Phase::Finished {
            state.window_diagnostic.pending();
        }
    }

    /// Report the diagnostic. Accepted only after `begin_window_diagnostic`,
    /// even once the outcome is decided, so no other late work can write here.
    pub(crate) fn end_window_diagnostic(&self, elapsed: Duration) {
        let mut state = self.locked();
        if state.phase != Phase::Finished && state.window_diagnostic == Slot::Pending {
            state.window_diagnostic = Slot::Done(micros(elapsed));
        }
    }

    /// Decide the outcome and stop accepting steps. Only the first call wins.
    pub(crate) fn close(&self, outcome: DeliveryOutcomeV1) {
        let mut state = self.locked();
        if state.phase != Phase::Open {
            return;
        }
        state.phase = Phase::Closed;
        state.outcome = Some(outcome);
        state.outcome_us = Slot::Done(micros(self.started.elapsed()));
    }

    /// Build the record exactly once. A record never closed explicitly is
    /// finished as `incomplete`.
    pub(crate) fn finish(&self) -> Option<DeliveryTimingV1> {
        let mut state = self.locked();
        if state.phase == Phase::Finished {
            return None;
        }
        state.phase = Phase::Finished;
        Some(state.build(self.accepted_route, micros(self.started.elapsed())))
    }
}

struct Scope {
    collector: Arc<DeliveryTimingCollector>,
    callback: Callback,
}

thread_local! {
    static SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// Routes step recordings made on this thread to one delivery's collector.
#[must_use = "recordings are routed only while the scope is alive"]
pub(crate) struct CallbackScope {
    previous: Option<Scope>,
}

impl Drop for CallbackScope {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let current = SCOPE
            .try_with(|scope| scope.borrow_mut().take())
            .ok()
            .flatten();
        let _ = SCOPE.try_with(|scope| *scope.borrow_mut() = previous);
        if let Some(finished) = current {
            finished.collector.callback_finished(finished.callback);
        }
    }
}

fn current() -> Option<(Arc<DeliveryTimingCollector>, Callback)> {
    SCOPE
        .try_with(|scope| {
            scope
                .borrow()
                .as_ref()
                .map(|scope| (Arc::clone(&scope.collector), scope.callback))
        })
        .ok()
        .flatten()
}

/// Add an elapsed span to a step of the delivery running on this thread.
/// Inert outside a callback scope (Paste Last, tests, other callers).
pub(crate) fn record(step: Step, elapsed: Duration) {
    if let Some((collector, _)) = current() {
        collector.record(step, elapsed);
    }
}

/// Run `operation` and attribute its wall time to `step`.
pub(crate) fn time<T>(step: Step, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    record(step, started.elapsed());
    result
}

pub(crate) fn record_verification(
    point: VerificationPoint,
    elapsed: Duration,
    lookup_retries: u64,
) {
    if let Some((collector, _)) = current() {
        collector.record(point.step(), elapsed);
        collector.record_count(point.counter(), lookup_retries);
    }
}

/// The wait for the dictation ownership lock inside the current callback.
pub(crate) fn record_owner_wait(elapsed: Duration) {
    if let Some((collector, callback)) = current() {
        let [_, owner_wait, _] = callback.steps();
        collector.record(owner_wait, elapsed);
    }
}

/// The work done while owning delivery inside the current callback.
pub(crate) fn record_callback_work(elapsed: Duration) {
    if let Some((collector, callback)) = current() {
        let [_, _, work] = callback.steps();
        collector.record(work, elapsed);
    }
}

/// One native paste retry and the wait before it.
pub(crate) fn record_native_paste_retry(wait: Duration) {
    if let Some((collector, _)) = current() {
        collector.record_count(Counter::NativePasteRetries, 1);
        collector.record(Step::NativePasteRetryWait, wait);
    }
}

/// Owns one delivery's collector for the async flow and guarantees its record
/// is published exactly once on every exit, including `?` returns and unwinds.
pub(crate) struct DeliveryTimingRecorder {
    collector: Arc<DeliveryTimingCollector>,
    performance: super::PerformanceMetrics,
    recording_id: u64,
}

impl DeliveryTimingRecorder {
    pub(crate) fn begin(
        performance: super::PerformanceMetrics,
        recording_id: u64,
        accepted_route: DeliveryRouteV1,
    ) -> Self {
        Self {
            collector: DeliveryTimingCollector::new(accepted_route),
            performance,
            recording_id,
        }
    }

    pub(crate) fn collector(&self) -> Arc<DeliveryTimingCollector> {
        Arc::clone(&self.collector)
    }

    /// Publish the record. Later calls, including the drop, do nothing. A
    /// record whose collector was never closed is published as `incomplete`.
    pub(crate) fn finish(&self) {
        if let Some(timing) = self.collector.finish() {
            self.performance
                .record_delivery_timing(self.recording_id, timing);
        }
    }
}

impl Drop for DeliveryTimingRecorder {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Finished records awaiting their run's completion, keyed by recording id.
/// A recording id never repeats, so a record can only attach to its own run.
#[derive(Default)]
pub(crate) struct DeliveryTimingStore {
    records: BTreeMap<u64, DeliveryTimingV1>,
}

impl DeliveryTimingStore {
    /// First record for a recording wins; a duplicate is ignored.
    pub(crate) fn insert(&mut self, recording_id: u64, timing: DeliveryTimingV1) -> bool {
        if self.records.contains_key(&recording_id) {
            return false;
        }
        while self.records.len() >= STORE_LIMIT {
            if self.records.pop_first().is_none() {
                break;
            }
        }
        self.records.insert(recording_id, timing);
        true
    }

    pub(crate) fn get(&self, recording_id: u64) -> Option<DeliveryTimingV1> {
        self.records.get(&recording_id).cloned()
    }

    pub(crate) fn remove(&mut self, recording_id: u64) -> Option<DeliveryTimingV1> {
        self.records.remove(&recording_id)
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn us(value: u64) -> Duration {
        Duration::from_micros(value)
    }

    fn measured(value: u64) -> MeasurementV1<u64> {
        MeasurementV1::measured(value)
    }

    fn pending() -> MeasurementV1<u64> {
        MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::DependencyPending,
        }
    }

    fn finished(
        collector: &Arc<DeliveryTimingCollector>,
        outcome: DeliveryOutcomeV1,
    ) -> DeliveryTimingV1 {
        collector.close(outcome);
        collector.finish().expect("first finish builds the record")
    }

    #[test]
    fn untouched_steps_are_not_applicable_and_zero_is_a_measurement() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        collector.record(Step::ClipboardWrite, us(0));
        let timing = finished(&collector, DeliveryOutcomeV1::AutoPasted);
        assert_eq!(timing.inject.clipboard_write_us, measured(0));
        assert_eq!(
            timing.inject.focused_field_query_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            timing.smart.prepare_validation_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(timing.configured_delay_us, MeasurementV1::NotApplicable);
        assert_eq!(timing.window_diagnostic_us, MeasurementV1::NotApplicable);
    }

    #[test]
    fn repeated_steps_accumulate_and_saturate_at_the_json_safe_bound() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        collector.record(Step::FocusedFieldQuery, us(40));
        collector.record(Step::FocusedFieldQuery, us(2));
        collector.record(Step::KeyEvent, Duration::from_secs(u64::MAX / 2));
        collector.record(Step::KeyEvent, Duration::from_secs(u64::MAX / 2));
        let timing = finished(&collector, DeliveryOutcomeV1::AutoPasted);
        assert_eq!(timing.inject.focused_field_query_us, measured(42));
        assert_eq!(timing.inject.key_event_us, measured(MAX_VALUE));
    }

    #[test]
    fn a_dispatched_callback_that_never_reports_is_pending_not_missing() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        collector.dispatch_callback(Callback::Injection);
        let timing = finished(&collector, DeliveryOutcomeV1::TimedOut);
        assert_eq!(timing.outcome, DeliveryOutcomeV1::TimedOut);
        assert_eq!(timing.injection_callback.queue_wait_us, pending());
        assert_eq!(timing.injection_callback.owner_wait_us, pending());
        assert_eq!(timing.injection_callback.work_us, pending());
        // The prepare callback was never dispatched on this route.
        assert_eq!(
            timing.prepare_callback.queue_wait_us,
            MeasurementV1::NotApplicable
        );
    }

    #[test]
    fn callback_scope_attributes_queue_owner_and_work_then_restores_the_thread() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::SmartTarget);
        collector.dispatch_callback(Callback::Prepare);
        collector.dispatch_callback(Callback::Injection);
        {
            let dispatched = Instant::now() - Duration::from_millis(7);
            let _scope = collector.enter_callback(Callback::Prepare, dispatched);
            record_owner_wait(us(150));
            record_callback_work(us(900));
            record(Step::PrepareValidation, us(800));
        }
        // Nothing records once the scope has dropped.
        record(Step::PrepareValidation, us(5));
        {
            let _scope = collector.enter_callback(Callback::Injection, Instant::now());
            record_owner_wait(us(20));
            time(Step::ClipboardWrite, || ());
        }
        let timing = finished(&collector, DeliveryOutcomeV1::AutoPasted);
        let queue = timing.prepare_callback.queue_wait_us.value().copied();
        assert!(queue.is_some_and(|queue| queue >= 7_000), "{queue:?}");
        assert_eq!(timing.prepare_callback.owner_wait_us, measured(150));
        assert_eq!(timing.prepare_callback.work_us, measured(900));
        assert_eq!(timing.smart.prepare_validation_us, measured(800));
        assert_eq!(timing.injection_callback.owner_wait_us, measured(20));
        // The injection callback finished without reaching owned work: it did
        // not apply, which differs from a callback that never ran.
        assert_eq!(
            timing.injection_callback.work_us,
            MeasurementV1::NotApplicable
        );
        assert!(timing.inject.clipboard_write_us.value().is_some());
    }

    #[test]
    fn recordings_are_inert_without_a_scope_and_nested_scopes_restore() {
        record(Step::ClipboardWrite, us(10));
        record_native_paste_retry(us(10));
        record_verification(VerificationPoint::BeforeFocus, us(10), 1);
        record_owner_wait(us(10));
        record_callback_work(us(10));
        assert!(current().is_none());

        let outer = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        let inner = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        {
            let _outer = outer.enter_callback(Callback::Injection, Instant::now());
            {
                let _inner = inner.enter_callback(Callback::Injection, Instant::now());
                record(Step::KeyEvent, us(3));
            }
            record(Step::KeyEvent, us(4));
        }
        assert!(current().is_none());
        assert_eq!(
            finished(&inner, DeliveryOutcomeV1::AutoPasted)
                .inject
                .key_event_us,
            measured(3)
        );
        assert_eq!(
            finished(&outer, DeliveryOutcomeV1::AutoPasted)
                .inject
                .key_event_us,
            measured(4)
        );
    }

    #[test]
    fn verification_points_report_elapsed_and_lookup_retries_separately() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        {
            let _scope = collector.enter_callback(Callback::Injection, Instant::now());
            record_verification(VerificationPoint::BeforeFocus, us(300), 0);
            record_verification(VerificationPoint::AfterFocus, us(12_000), 2);
            record_verification(VerificationPoint::AfterClipboard, us(450), 1);
        }
        let timing = finished(&collector, DeliveryOutcomeV1::AutoPasted);
        assert_eq!(
            timing.inject.before_focus_verification,
            DeliveryVerificationV1 {
                elapsed_us: measured(300),
                lookup_retry_count: measured(0),
            }
        );
        assert_eq!(
            timing.inject.after_focus_verification,
            DeliveryVerificationV1 {
                elapsed_us: measured(12_000),
                lookup_retry_count: measured(2),
            }
        );
        assert_eq!(
            timing.inject.after_clipboard_verification,
            DeliveryVerificationV1 {
                elapsed_us: measured(450),
                lookup_retry_count: measured(1),
            }
        );
    }

    #[test]
    fn native_paste_retry_count_and_wait_are_zero_only_when_a_paste_was_built() {
        let never_pasted = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        let timing = finished(&never_pasted, DeliveryOutcomeV1::ClipboardOnlyTargetChanged);
        assert_eq!(
            timing.inject.native_paste_retry_count,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            timing.inject.native_paste_retry_wait_us,
            MeasurementV1::NotApplicable
        );

        let clean = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        clean.record(Step::EventConstruction, us(30));
        let timing = finished(&clean, DeliveryOutcomeV1::AutoPasted);
        assert_eq!(timing.inject.native_paste_retry_count, measured(0));
        assert_eq!(timing.inject.native_paste_retry_wait_us, measured(0));

        let retried = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        {
            let _scope = retried.enter_callback(Callback::Injection, Instant::now());
            record(Step::EventConstruction, us(30));
            record_native_paste_retry(Duration::from_millis(100));
        }
        let timing = finished(&retried, DeliveryOutcomeV1::AutoPasted);
        assert_eq!(timing.inject.native_paste_retry_count, measured(1));
        assert_eq!(timing.inject.native_paste_retry_wait_us, measured(100_000));
    }

    #[test]
    fn a_late_callback_cannot_change_a_closed_record() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::SmartTarget);
        collector.dispatch_callback(Callback::Injection);
        collector.close(DeliveryOutcomeV1::TimedOut);
        let before = {
            let state = collector.locked();
            state.outcome_us
        };
        {
            // The main-thread turn finally runs, after the receiver gave up.
            let _late = collector.enter_callback(Callback::Injection, Instant::now());
            record_owner_wait(us(1));
            record_callback_work(us(2));
            record_verification(VerificationPoint::BeforeFocus, us(3), 2);
            record(Step::ClipboardWrite, us(4));
            record_native_paste_retry(us(5));
            collector.set_delivery_route(DeliveryRouteV1::ClipboardOnly);
            collector.set_prepare_outcome(SmartPrepareOutcomeV1::Ready);
            collector.close(DeliveryOutcomeV1::AutoPasted);
        }
        let timing = collector.finish().unwrap();
        assert_eq!(timing.outcome, DeliveryOutcomeV1::TimedOut);
        assert_eq!(timing.delivery_route, DeliveryRouteV1::SmartTarget);
        assert_eq!(timing.smart.prepare_outcome, SmartPrepareOutcomeV1::NotRun);
        assert_eq!(timing.outcome_us, before.measurement());
        assert_eq!(timing.injection_callback.queue_wait_us, pending());
        assert_eq!(timing.injection_callback.owner_wait_us, pending());
        assert_eq!(timing.injection_callback.work_us, pending());
        assert_eq!(
            timing.inject.clipboard_write_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            timing.inject.before_focus_verification.elapsed_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            timing.inject.native_paste_retry_count,
            MeasurementV1::NotApplicable
        );
        // Nothing can build a second record from the same collector.
        assert!(collector.finish().is_none());
    }

    #[test]
    fn the_window_diagnostic_reports_after_close_only_when_pending() {
        let strays = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        strays.close(DeliveryOutcomeV1::AutoPasted);
        strays.end_window_diagnostic(us(900));
        let timing = strays.finish().unwrap();
        assert_eq!(timing.window_diagnostic_us, MeasurementV1::NotApplicable);

        let on_time = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        on_time.begin_window_diagnostic();
        on_time.close(DeliveryOutcomeV1::AutoPasted);
        on_time.end_window_diagnostic(us(900));
        assert_eq!(
            on_time.finish().unwrap().window_diagnostic_us,
            measured(900)
        );

        let too_late = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        too_late.begin_window_diagnostic();
        too_late.close(DeliveryOutcomeV1::AutoPasted);
        let timing = too_late.finish().unwrap();
        too_late.end_window_diagnostic(us(900));
        assert_eq!(timing.window_diagnostic_us, pending());
        assert!(too_late.finish().is_none());
    }

    #[test]
    fn finishing_without_an_outcome_closes_as_incomplete() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::Disabled);
        let timing = collector.finish().unwrap();
        assert_eq!(timing.outcome, DeliveryOutcomeV1::Incomplete);
        assert_eq!(timing.outcome_us, MeasurementV1::NotApplicable);
        assert_eq!(timing.accepted_route, DeliveryRouteV1::Disabled);
        assert!(timing.total_us.value().is_some());
    }

    #[test]
    fn the_first_outcome_wins() {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::NormalPaste);
        collector.close(DeliveryOutcomeV1::ClipboardOnlyTargetChanged);
        collector.close(DeliveryOutcomeV1::AutoPasted);
        assert_eq!(
            collector.finish().unwrap().outcome,
            DeliveryOutcomeV1::ClipboardOnlyTargetChanged
        );
    }

    #[test]
    fn every_injection_outcome_maps_to_one_distinct_bounded_outcome() {
        let mut seen = vec![
            DeliveryOutcomeV1::of_injection(InjectionOutcome::NoText),
            DeliveryOutcomeV1::of_injection(InjectionOutcome::AutoPasted),
        ];
        for reason in [
            ClipboardOnlyReason::AutoPasteDisabled,
            ClipboardOnlyReason::AccessibilityDenied,
            ClipboardOnlyReason::TargetChanged,
            ClipboardOnlyReason::FocusNotEditable,
            ClipboardOnlyReason::ClipboardChanged,
            ClipboardOnlyReason::PasteFailed,
            ClipboardOnlyReason::SmartTargetUnavailable,
        ] {
            seen.push(DeliveryOutcomeV1::of_injection(
                InjectionOutcome::ClipboardOnly(reason),
            ));
        }
        for stop in [
            AttemptStop::Cancelled,
            AttemptStop::Expired,
            AttemptStop::AlreadyCommitted,
            AttemptStop::OperationInFlight,
        ] {
            seen.push(DeliveryOutcomeV1::of_injection(InjectionOutcome::Stopped(
                stop,
            )));
        }
        for (index, outcome) in seen.iter().enumerate() {
            assert!(!seen[..index].contains(outcome), "{outcome:?} repeats");
            assert_ne!(*outcome, DeliveryOutcomeV1::Incomplete);
        }
        assert_eq!(seen.len(), 13);
    }

    #[test]
    fn routes_mirror_the_smart_destination_route_codes() {
        for (snapshot, route, code) in [
            (Snapshot::Disabled, DeliveryRouteV1::Disabled, "disabled"),
            (
                Snapshot::Unavailable,
                DeliveryRouteV1::ClipboardOnly,
                "clipboardOnly",
            ),
            (
                Snapshot::Unverified,
                DeliveryRouteV1::NormalPaste,
                "normalPaste",
            ),
        ] {
            assert_eq!(DeliveryRouteV1::of(&snapshot), route);
            assert_eq!(serde_json::to_value(route).unwrap(), json!(code));
        }
        assert_eq!(
            serde_json::to_value(DeliveryRouteV1::SmartTarget).unwrap(),
            json!("smartTarget")
        );
    }

    fn sample_timing() -> DeliveryTimingV1 {
        let collector = DeliveryTimingCollector::new(DeliveryRouteV1::SmartTarget);
        collector.set_delivery_route(DeliveryRouteV1::NormalPaste);
        collector.set_prepare_outcome(SmartPrepareOutcomeV1::Refused);
        collector.record(Step::ConfiguredDelay, us(120_000));
        collector.dispatch_callback(Callback::Prepare);
        collector.dispatch_callback(Callback::Injection);
        {
            let _scope = collector.enter_callback(Callback::Injection, Instant::now());
            record_owner_wait(us(10));
            record_callback_work(us(5_000));
            record_verification(VerificationPoint::AfterClipboard, us(1_500), 1);
            record(Step::ClipboardWrite, us(2_000));
            record(Step::FinalFocusProof, us(700));
            record(Step::KeyEvent, us(3_100));
        }
        collector.begin_window_diagnostic();
        collector.close(DeliveryOutcomeV1::AutoPasted);
        collector.end_window_diagnostic(us(450));
        collector.finish().unwrap()
    }

    #[test]
    fn serializes_schema_one_with_camel_case_measurement_wrappers() {
        let timing = sample_timing();
        let value = serde_json::to_value(&timing).unwrap();
        assert_eq!(value["schemaVersion"], json!(1));
        assert_eq!(value["acceptedRoute"], json!("smartTarget"));
        assert_eq!(value["deliveryRoute"], json!("normalPaste"));
        assert_eq!(value["outcome"], json!("autoPasted"));
        assert_eq!(
            value["configuredDelayUs"],
            json!({"status": "measured", "value": 120_000})
        );
        assert_eq!(
            value["injectionCallback"]["workUs"],
            json!({"status": "measured", "value": 5_000})
        );
        assert_eq!(
            value["inject"]["afterClipboardVerification"],
            json!({
                "elapsedUs": {"status": "measured", "value": 1_500},
                "lookupRetryCount": {"status": "measured", "value": 1}
            })
        );
        assert_eq!(
            value["prepareCallback"]["queueWaitUs"],
            json!({"status": "unavailable", "reason": "dependencyPending"})
        );
        assert_eq!(
            value["inject"]["focusedFieldQueryUs"],
            json!({"status": "notApplicable"})
        );
        assert_eq!(value["smart"]["prepareOutcome"], json!("refused"));
        assert_eq!(
            value["windowDiagnosticUs"],
            json!({"status": "measured", "value": 450})
        );
        let keys = |value: &Value| {
            let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(
            keys(&value),
            [
                "acceptedRoute",
                "configuredDelayUs",
                "deliveryRoute",
                "inject",
                "injectionCallback",
                "outcome",
                "outcomeUs",
                "prepareCallback",
                "schemaVersion",
                "smart",
                "totalUs",
                "windowDiagnosticUs",
            ]
        );
        assert_eq!(
            keys(&value["inject"]),
            [
                "afterClipboardVerification",
                "afterFocusVerification",
                "beforeFocusVerification",
                "clipboardWriteUs",
                "eventConstructionUs",
                "finalFocusProofUs",
                "focusedFieldQueryUs",
                "keyEventUs",
                "nativePasteRetryCount",
                "nativePasteRetryWaitUs",
            ]
        );
        assert_eq!(
            keys(&value["smart"]),
            [
                "activationWaitUs",
                "cleanupUs",
                "insertValidationUs",
                "prepareOutcome",
                "prepareValidationUs",
                "readbackUs",
                "restoreUs",
                "settleUs",
                "snapshotWindowLookupUs",
            ]
        );
        // The frontend guard in `lib/performance.ts` rejects any other key set,
        // so the nested callback and verification shapes are pinned too.
        for callback in ["prepareCallback", "injectionCallback"] {
            assert_eq!(
                keys(&value[callback]),
                ["ownerWaitUs", "queueWaitUs", "workUs"],
                "{callback}"
            );
        }
        for point in [
            "beforeFocusVerification",
            "afterFocusVerification",
            "afterClipboardVerification",
        ] {
            assert_eq!(
                keys(&value["inject"][point]),
                ["elapsedUs", "lookupRetryCount"],
                "{point}"
            );
        }
    }

    #[test]
    fn round_trips_and_rejects_unknown_fields_or_other_schema_versions() {
        let timing = sample_timing();
        let value = serde_json::to_value(&timing).unwrap();
        assert_eq!(
            serde_json::from_value::<DeliveryTimingV1>(value.clone()).unwrap(),
            timing
        );

        for (path, replacement) in [
            // A field that could carry content is rejected, not ignored.
            ("text", json!("dictated words")),
            ("schemaVersion", json!(2)),
        ] {
            let mut changed = value.clone();
            changed[path] = replacement;
            assert!(
                serde_json::from_value::<DeliveryTimingV1>(changed).is_err(),
                "{path} must be rejected"
            );
        }
        let mut nested = value;
        nested["inject"]["windowTitle"] = json!("Inbox");
        assert!(serde_json::from_value::<DeliveryTimingV1>(nested).is_err());
    }

    #[test]
    fn the_store_keeps_the_first_record_and_stays_bounded() {
        let mut store = DeliveryTimingStore::default();
        let first = sample_timing();
        let mut second = sample_timing();
        second.outcome = DeliveryOutcomeV1::TimedOut;
        assert!(store.insert(1, first.clone()));
        assert!(!store.insert(1, second));
        assert_eq!(store.get(1), Some(first));

        for id in 2..=20 {
            store.insert(id, sample_timing());
        }
        assert!(store.get(1).is_none(), "oldest record is evicted");
        assert!(store.get(20).is_some());
        assert_eq!(store.records.len(), STORE_LIMIT);

        assert!(store.remove(20).is_some());
        assert!(store.get(20).is_none());
        store.clear();
        assert!(store.records.is_empty());
    }

    fn published(
        metrics: &crate::performance_metrics::PerformanceMetrics,
        recording_id: u64,
    ) -> Option<DeliveryTimingV1> {
        metrics
            .delivery_timing
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(recording_id)
    }

    #[test]
    fn outcome_codes_are_stable_bounded_camel_case_strings() {
        for (outcome, code) in [
            (DeliveryOutcomeV1::AutoPasted, "autoPasted"),
            (DeliveryOutcomeV1::NoText, "noText"),
            (
                DeliveryOutcomeV1::ClipboardOnlyAutoPasteDisabled,
                "clipboardOnlyAutoPasteDisabled",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlyAccessibilityDenied,
                "clipboardOnlyAccessibilityDenied",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlyTargetChanged,
                "clipboardOnlyTargetChanged",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlyFocusNotEditable,
                "clipboardOnlyFocusNotEditable",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlyClipboardChanged,
                "clipboardOnlyClipboardChanged",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlyPasteFailed,
                "clipboardOnlyPasteFailed",
            ),
            (
                DeliveryOutcomeV1::ClipboardOnlySmartTargetUnavailable,
                "clipboardOnlySmartTargetUnavailable",
            ),
            (DeliveryOutcomeV1::StoppedCancelled, "stoppedCancelled"),
            (DeliveryOutcomeV1::StoppedExpired, "stoppedExpired"),
            (
                DeliveryOutcomeV1::StoppedAlreadyCommitted,
                "stoppedAlreadyCommitted",
            ),
            (
                DeliveryOutcomeV1::StoppedOperationInFlight,
                "stoppedOperationInFlight",
            ),
            (
                DeliveryOutcomeV1::ClipboardWriteFailed,
                "clipboardWriteFailed",
            ),
            (
                DeliveryOutcomeV1::CancelledBeforeInjection,
                "cancelledBeforeInjection",
            ),
            (DeliveryOutcomeV1::SenderDropped, "senderDropped"),
            (DeliveryOutcomeV1::TimedOut, "timedOut"),
            (DeliveryOutcomeV1::DispatchFailed, "dispatchFailed"),
            (DeliveryOutcomeV1::Incomplete, "incomplete"),
        ] {
            assert_eq!(serde_json::to_value(outcome).unwrap(), json!(code));
            assert_eq!(
                serde_json::from_value::<DeliveryOutcomeV1>(json!(code)).unwrap(),
                outcome
            );
        }
    }

    #[test]
    fn smart_prepare_outcomes_map_one_to_one_and_keep_the_clipboard_only_refusal_distinct() {
        for (prepared, outcome, code) in [
            (PrepareOutcome::Ready, SmartPrepareOutcomeV1::Ready, "ready"),
            (
                PrepareOutcome::WaitForActivation,
                SmartPrepareOutcomeV1::WaitForActivation,
                "waitForActivation",
            ),
            (
                PrepareOutcome::Refused,
                SmartPrepareOutcomeV1::Refused,
                "refused",
            ),
            (
                PrepareOutcome::RefusedClipboardOnly,
                SmartPrepareOutcomeV1::RefusedClipboardOnly,
                "refusedClipboardOnly",
            ),
        ] {
            assert_eq!(SmartPrepareOutcomeV1::of(prepared), outcome);
            assert_eq!(serde_json::to_value(outcome).unwrap(), json!(code));
        }
        for (outcome, code) in [
            (SmartPrepareOutcomeV1::NotRun, "notRun"),
            (SmartPrepareOutcomeV1::OwnerNotCurrent, "ownerNotCurrent"),
            (SmartPrepareOutcomeV1::SenderDropped, "senderDropped"),
            (SmartPrepareOutcomeV1::TimedOut, "timedOut"),
            (SmartPrepareOutcomeV1::DispatchFailed, "dispatchFailed"),
        ] {
            assert_eq!(serde_json::to_value(outcome).unwrap(), json!(code));
        }
    }

    #[test]
    fn every_refusal_error_and_timeout_path_closes_the_record_with_its_own_outcome() {
        let paths = [
            // Refusals the injector decided.
            DeliveryOutcomeV1::ClipboardOnlyAutoPasteDisabled,
            DeliveryOutcomeV1::ClipboardOnlyAccessibilityDenied,
            DeliveryOutcomeV1::ClipboardOnlyTargetChanged,
            DeliveryOutcomeV1::ClipboardOnlyFocusNotEditable,
            DeliveryOutcomeV1::ClipboardOnlyClipboardChanged,
            DeliveryOutcomeV1::ClipboardOnlyPasteFailed,
            DeliveryOutcomeV1::ClipboardOnlySmartTargetUnavailable,
            DeliveryOutcomeV1::StoppedCancelled,
            DeliveryOutcomeV1::StoppedExpired,
            DeliveryOutcomeV1::StoppedAlreadyCommitted,
            DeliveryOutcomeV1::StoppedOperationInFlight,
            // Errors and receiver-side failures that never produced an outcome.
            DeliveryOutcomeV1::ClipboardWriteFailed,
            DeliveryOutcomeV1::CancelledBeforeInjection,
            DeliveryOutcomeV1::SenderDropped,
            DeliveryOutcomeV1::TimedOut,
            DeliveryOutcomeV1::DispatchFailed,
            // Success and the no-text early exit close the same way.
            DeliveryOutcomeV1::AutoPasted,
            DeliveryOutcomeV1::NoText,
        ];
        for outcome in paths {
            let collector = DeliveryTimingCollector::new(DeliveryRouteV1::SmartTarget);
            collector.dispatch_callback(Callback::Prepare);
            collector.dispatch_callback(Callback::Injection);
            collector.close(outcome);
            // A late success from a callback that outlived the receiver cannot
            // rewrite the decided outcome.
            collector.close(DeliveryOutcomeV1::AutoPasted);
            let timing = collector.finish().expect("first finish builds the record");
            assert_eq!(timing.outcome, outcome);
            assert!(timing.outcome_us.value().is_some(), "{outcome:?}");
            assert!(timing.total_us.value().is_some(), "{outcome:?}");
            // Callbacks that never reported say so instead of reading as zero.
            assert_eq!(timing.prepare_callback.work_us, pending(), "{outcome:?}");
            assert_eq!(timing.injection_callback.work_us, pending(), "{outcome:?}");
        }
    }

    #[test]
    fn the_recorder_publishes_once_and_an_unclosed_drop_is_incomplete() {
        let metrics = crate::performance_metrics::PerformanceMetrics::default();
        let recorder =
            DeliveryTimingRecorder::begin(metrics.clone(), 7, DeliveryRouteV1::NormalPaste);
        assert!(
            published(&metrics, 7).is_none(),
            "nothing is published early"
        );
        recorder.collector().close(DeliveryOutcomeV1::AutoPasted);
        recorder.finish();
        let record = published(&metrics, 7).expect("finish publishes the record");
        assert_eq!(record.outcome, DeliveryOutcomeV1::AutoPasted);
        // Neither a second finish nor the drop publishes anything further.
        recorder.finish();
        drop(recorder);
        assert_eq!(published(&metrics, 7), Some(record));

        // An early `?` return leaves the recorder to its drop.
        {
            let _abandoned =
                DeliveryTimingRecorder::begin(metrics.clone(), 8, DeliveryRouteV1::SmartTarget);
        }
        let incomplete = published(&metrics, 8).expect("drop publishes the record");
        assert_eq!(incomplete.outcome, DeliveryOutcomeV1::Incomplete);
        assert_eq!(incomplete.accepted_route, DeliveryRouteV1::SmartTarget);
        assert_eq!(incomplete.outcome_us, MeasurementV1::NotApplicable);

        // So does an unwind.
        let unwinding = metrics.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _recorder =
                DeliveryTimingRecorder::begin(unwinding, 9, DeliveryRouteV1::NormalPaste);
            panic!("test unwind");
        }));
        assert!(result.is_err());
        assert_eq!(
            published(&metrics, 9).map(|record| record.outcome),
            Some(DeliveryOutcomeV1::Incomplete)
        );
    }

    #[test]
    fn a_late_callback_of_an_older_recording_cannot_reach_a_newer_recordings_record() {
        let metrics = crate::performance_metrics::PerformanceMetrics::default();
        let older = DeliveryTimingRecorder::begin(metrics.clone(), 1, DeliveryRouteV1::SmartTarget);
        let older_collector = older.collector();
        older_collector.dispatch_callback(Callback::Injection);
        // The older delivery gave up waiting and published its record.
        older_collector.close(DeliveryOutcomeV1::TimedOut);
        older.finish();

        let newer = DeliveryTimingRecorder::begin(metrics.clone(), 2, DeliveryRouteV1::NormalPaste);
        let newer_collector = newer.collector();
        newer_collector.dispatch_callback(Callback::Injection);
        {
            // The older recording's main-thread turn finally runs while the
            // newer recording is open. It holds only its own collector.
            let _late = older_collector.enter_callback(Callback::Injection, Instant::now());
            record_owner_wait(us(11));
            record_callback_work(us(22));
            record(Step::ClipboardWrite, us(33));
            record_native_paste_retry(us(44));
            older_collector.close(DeliveryOutcomeV1::AutoPasted);
        }
        newer_collector.close(DeliveryOutcomeV1::ClipboardOnlyFocusNotEditable);
        newer.finish();

        let older_record = published(&metrics, 1).expect("older record");
        let newer_record = published(&metrics, 2).expect("newer record");
        assert_eq!(older_record.outcome, DeliveryOutcomeV1::TimedOut);
        assert_eq!(
            older_record.inject.clipboard_write_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            newer_record.outcome,
            DeliveryOutcomeV1::ClipboardOnlyFocusNotEditable
        );
        // Nothing the late callback measured landed in the newer record.
        assert_eq!(
            newer_record.inject.clipboard_write_us,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            newer_record.inject.native_paste_retry_count,
            MeasurementV1::NotApplicable
        );
        assert_eq!(newer_record.injection_callback.owner_wait_us, pending());
        assert_eq!(newer_record.injection_callback.work_us, pending());
    }
}
