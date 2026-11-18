//! The production [`Ports`] of the start command (Stage 2a-4a, sites 101 and
//! 102): each effect is today's statement, run by the executor at the placement
//! the decision gave it.
//!
//! [`StartSlots`] owns what the effects produce or consume and the core never
//! sees (X4): the gesture and speculative claims, the accepted gesture, the frozen
//! delivery target, the request timestamp, the microphone selection the start
//! resolved and the supervisor's answer. It is owned by the start future by value
//! and **lent** to a [`StartPorts`]; the ports (not `Send`) are dropped before the
//! kept-preview await and rebuilt over the same slots after it.
//!
//! No port takes a lock of its own: the field effects (`ST`, `ID+`, `CX-`) go
//! through the `FieldAdapter` on the guard the shell passes the executor, and every
//! other effect is the statement the start ran inline before. Every result a port
//! reports is the real one (an emit's `Result`, the supervisor's answer, the
//! terminal claim), never a constant.
//!
//! There is no `outside_core` span clearing (decision D-S2-2): `IN_CORE` stays set
//! across every port call.

use super::{
    emit_dictation_terminal, AudioStartError, DictationHost, State, DICTATION_SLO_CONTRACT,
};
use crate::audio_lifecycle::gesture::{GestureClaim, StartGesture};
use crate::audio_lifecycle::speculative::SpeculativeClaim;
use crate::dictation_context::DictationContextSnapshot;
use crate::dictation_executor::convert::{error_code_from_core, terminal_outcome_from_core};
use crate::dictation_executor::executor::{
    Executor, PhaseCtx, PhaseReport, PortFailure, PortResult, Ports,
};
use crate::performance_metrics::production::MicrophoneSelectionV1;
use crate::state::{AppState, DictationState};
use murmur_dictation_core::decision::{Decision, InvalidKind};
use murmur_dictation_core::effects::{AxMode, Effect, EventKind, Placement};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::inputs::CaptureErr;
use murmur_dictation_core::replies::{ErrText, Reply};
use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Wrap a decision in an executor. `kept_preview` selects the awaited form of `CS`
/// (`spawn_blocking(..).await`). The crate tests validate every start row, so a
/// failure here is a programming error.
pub(super) fn executor<'a>(
    ports: &'a dyn Ports,
    decision: Decision,
    kept_preview: bool,
) -> Executor<'a> {
    Executor::new(ports, decision, kept_preview)
        .expect("a start decision is structurally valid (crate row tests)")
}

/// Run a (pure) start decide function under the re-entrancy guard
/// (`decide_guarded`): a nested call returns `Invalid(Reentrant)`.
pub(super) fn decide(decide: impl FnOnce() -> Decision) -> Decision {
    crate::dictation_executor::executor::decide_guarded(decide)
}

/// The pre-D refusal check of site 101, under the same guard: a nested call is
/// refused before it reads anything.
pub(super) fn decide_pre<T>(decide: impl FnOnce() -> T) -> Result<T, InvalidKind> {
    let _entry = crate::dictation_executor::executor::CoreEntry::enter()?;
    Ok(decide())
}

/// Run a synchronous host read that runs under D and T (the Smart Auto resolver,
/// whose success emits `smart-auto-microphone-changed` inline) with the core span
/// set: a Rust listener of that notification that re-enters the start, stop or
/// cancel is refused before any lock, and an `IdleGuard` it drops queues its
/// finish instead of asking for D again (D-S2-2, D-S2-4). The lock placement of the
/// read is unchanged. Never held across an await.
pub(super) fn observe_in_core<T>(read: impl FnOnce() -> T) -> T {
    let _entry = crate::dictation_executor::executor::CoreEntry::enter()
        .unwrap_or_else(|kind| unreachable!("a start observation outside any core span: {kind:?}"));
    read()
}

/// Run one placement and return the executor's report. `dictation` is `Some` only
/// for `In`: the D guard's contents the shell holds. The caller handles the
/// report: `Awaiting` for the kept-preview `CS`; `Invalid` (nothing ran) is logged.
pub(super) fn try_run_phase(
    executor: &mut Executor<'_>,
    phase: Placement,
    dictation: Option<&mut DictationState>,
    recording_id: u64,
) -> PhaseReport {
    let mut ctx = PhaseCtx {
        dictation,
        recording_id,
    };
    let report = executor.run(phase, &mut ctx);
    if let PhaseReport::Invalid(kind) = &report {
        tracing::error!(
            target: "pipeline",
            recording_id,
            phase = phase.label(),
            refusal = ?kind,
            "start effects were not run: the executor refused the phase"
        );
    }
    report
}

/// Run one placement where the shell is outside any core span and runs the phases
/// in order (every start call after its re-entrancy wait): a refused or
/// out-of-order run cannot happen there, so it is `unreachable!`, never ignored.
pub(super) fn run_phase(
    executor: &mut Executor<'_>,
    phase: Placement,
    dictation: Option<&mut DictationState>,
    recording_id: u64,
) {
    if let PhaseReport::Invalid(kind) = try_run_phase(executor, phase, dictation, recording_id) {
        unreachable!("the start's {} phase was refused: {kind:?}", phase.label());
    }
}

/// A decision made outside any core span is never `Invalid`: the only refusal is a
/// nested decide.
pub(super) fn expect_valid(decision: &Decision) {
    if let murmur_dictation_core::decision::Verdict::Invalid(kind) = &decision.verdict {
        unreachable!("a start decision outside any core span was refused: {kind:?}");
    }
}

// Test builds: the witness of the smart-destination section's executor (R103.0).
#[cfg(test)]
thread_local! {
    pub(super) static DESTINATION: RefCell<Vec<crate::dictation_executor::executor::Witness>> =
        const { RefCell::new(Vec::new()) };
}

// Test builds: the witness of the start's executor after it resumed the awaited
// `CS` with the supervisor's actual result.
#[cfg(test)]
thread_local! {
    pub(super) static RESUMED: RefCell<Vec<crate::dictation_executor::executor::Witness>> =
        const { RefCell::new(Vec::new()) };
}

/// The JSON of a start reply, with the shell-owned slot filled in: `shell_error`
/// for `Err(Shell)` (the supervisor's own text).
pub(super) fn render_reply(
    reply: &Reply,
    shell_error: Option<String>,
) -> Result<serde_json::Value, String> {
    match reply {
        Reply::Typed {
            ty,
            state,
            recording_id: None,
        } => Ok(serde_json::json!({
            "type": ty.as_str(),
            "state": state.as_str(),
        })),
        Reply::Typed {
            ty,
            state,
            recording_id: Some(id),
        } => Ok(serde_json::json!({
            "type": ty.as_str(),
            "state": state.as_str(),
            "recordingId": id.0,
        })),
        Reply::Transcription { .. } | Reply::Ok => {
            unreachable!("a start never replies with a transcription or a bare Ok")
        }
        Reply::Err(ErrText::Shell | ErrText::SmartAuto) => {
            Err(shell_error.expect("a shell error reply carries its text"))
        }
        Reply::Err(other) => Err(other
            .message()
            .expect("every non-shell error text is literal")),
    }
}

/// The reply of a start decision: a refused entry (`Invalid`, nothing ran) is an
/// `Err`, anything else renders its typed reply.
pub(super) fn reply_of(
    decision: &Decision,
    shell_error: Option<String>,
) -> Result<serde_json::Value, String> {
    expect_valid(decision);
    let reply = decision
        .reply
        .as_ref()
        .expect("a start decision that ran to a reply carries its row's reply");
    render_reply(reply, shell_error)
}

pub(super) fn start_refused(kind: InvalidKind) -> String {
    format!("The dictation core refused the start: {kind:?}")
}

/// The core's name for a supervisor start error (5675-5685). Exhaustive: a new
/// `AudioStartError` fails to compile here until its row is decided.
fn capture_err(error: &AudioStartError) -> CaptureErr {
    match error {
        AudioStartError::AlreadyStarting => CaptureErr::AlreadyStarting,
        AudioStartError::AudioRecovering => CaptureErr::AudioRecovering,
        AudioStartError::AlreadyRecording
        | AudioStartError::SpawnFailed(_)
        | AudioStartError::InitializationFailed(_)
        | AudioStartError::Cancelled
        | AudioStartError::SupervisorUnavailable => CaptureErr::Other,
    }
}

/// The immutable facts of one accepted start the effects read.
pub(super) struct StartPlan {
    /// `"hold"` or `"toggle"`.
    pub(super) origin: &'static str,
    /// The request's gesture is a hold.
    pub(super) hold: bool,
    /// `"smart_auto"`, `"explicit"` or `"system_default"` (the request log).
    pub(super) device_selection: &'static str,
    /// The performance ledger's selection kind.
    pub(super) microphone_selection: MicrophoneSelectionV1,
    /// The microphone the supervisor is asked for.
    pub(super) selected_device_name: Option<String>,
}

/// The values the start command's effects produce or consume (X4).
pub(super) struct StartSlots {
    // Declared in the order today's locals drop: the speculative claim, then the
    // gesture claim.
    speculative: RefCell<Option<SpeculativeClaim>>,
    gesture_claim: RefCell<Option<GestureClaim>>,
    plan: RefCell<Option<StartPlan>>,
    /// `GA`: the accepted gesture, `SPEC` adds the speculative capture.
    gesture: RefCell<Option<StartGesture>>,
    /// `AX`: the delivery target frozen at the accepted start.
    delivery_target: RefCell<Option<crate::frontmost::DeliveryTargetSnapshot>>,
    /// `TRQ`: the request's wall-clock stamp.
    started_at_ms: Cell<Option<i64>>,
    /// `SPEC`: whether the speculative capture was already flowing before it was
    /// handed over (skips `EM(starting)`).
    speculative_flowing: Cell<bool>,
    /// `CS`: the supervisor's answer.
    audio_result: RefCell<Option<Result<(), AudioStartError>>>,
    /// The log line that followed the superseded start's `TERM` claim (R103.4-.6):
    /// run by the terminal port right after the claim.
    after_terminal: RefCell<Option<Box<dyn FnOnce() + Send>>>,
}

impl StartSlots {
    pub(super) fn new(gesture_claim: GestureClaim, speculative: SpeculativeClaim) -> Self {
        Self {
            speculative: RefCell::new(Some(speculative)),
            gesture_claim: RefCell::new(Some(gesture_claim)),
            plan: RefCell::new(None),
            gesture: RefCell::new(None),
            delivery_target: RefCell::new(None),
            started_at_ms: Cell::new(None),
            speculative_flowing: Cell::new(false),
            audio_result: RefCell::new(None),
            after_terminal: RefCell::new(None),
        }
    }

    /// Arm the log line that follows the next `TERM` claim.
    pub(super) fn after_terminal(&self, log: Box<dyn FnOnce() + Send>) {
        *self.after_terminal.borrow_mut() = Some(log);
    }

    /// Whether the still-unconsumed speculative capture is already flowing: the value
    /// `SPEC` will read when it hands the capture over, read once more under D for the
    /// shadow's header (appendix C8.2). One atomic load, no lock.
    #[cfg(feature = "dictation-shadow")]
    pub(super) fn speculative_flowing_now(&self) -> bool {
        self.speculative
            .borrow()
            .as_ref()
            .is_some_and(SpeculativeClaim::is_flowing)
    }

    /// Whether the request carried a speculative capture (an observation only).
    pub(super) fn speculative_claimed(&self) -> bool {
        self.speculative
            .borrow()
            .as_ref()
            .is_some_and(SpeculativeClaim::is_claimed)
    }

    pub(super) fn set_plan(&self, plan: StartPlan) {
        *self.plan.borrow_mut() = Some(plan);
    }

    /// `AX`'s frozen target, moved out for the later sites.
    pub(super) fn take_delivery_target(&self) -> Option<crate::frontmost::DeliveryTargetSnapshot> {
        self.delivery_target.borrow_mut().take()
    }

    /// `TRQ`'s stamp.
    pub(super) fn started_at_ms(&self) -> Option<i64> {
        self.started_at_ms.get()
    }

    /// The supervisor's answer, once `CS` ran: `Ok`, or the mapped error kind with
    /// its text (the reply of `Err(Shell)`). Peeked, not taken: the `PA` port logs
    /// the same error.
    pub(super) fn audio_outcome(&self) -> Option<Result<(), (CaptureErr, String)>> {
        let result = self.audio_result.borrow();
        Some(match result.as_ref()? {
            Ok(()) => Ok(()),
            Err(error) => Err((capture_err(error), error.to_string())),
        })
    }

    /// Record the answer of the awaited `CS`.
    pub(super) fn store_audio_result(&self, result: Result<(), AudioStartError>) {
        *self.audio_result.borrow_mut() = Some(result);
    }

    /// What the supervisor start needs: the microphone, the origin and the
    /// gesture (moved out; it is spent by the start).
    pub(super) fn start_args(&self) -> Option<(Option<String>, &'static str, StartGesture)> {
        let plan = self.plan.borrow();
        let plan = plan.as_ref()?;
        let gesture = self.gesture.borrow_mut().take()?;
        Some((plan.selected_device_name.clone(), plan.origin, gesture))
    }
}

/// What the effects of sites 103 and 104 read (stage 2a-4b): the frozen delivery
/// target, the resolved context (the payload of `CX+`, the model of `PM` and the
/// runtime identity of `PB`) and the request's origin stamp.
pub(super) struct TailFacts<'a> {
    pub(super) delivery_target: &'a crate::frontmost::DeliveryTargetSnapshot,
    pub(super) context: Arc<DictationContextSnapshot>,
    pub(super) performance_started_at_ms: i64,
}

pub(super) struct StartPorts<'a> {
    host: &'a dyn DictationHost,
    state: &'a State,
    slots: &'a StartSlots,
    tail: Option<&'a TailFacts<'a>>,
    /// The flight's recording id once known; before `ID+` the live id is read.
    recording_id: Cell<Option<u64>>,
}

impl<'a> StartPorts<'a> {
    pub(super) fn new(
        host: &'a dyn DictationHost,
        state: &'a State,
        slots: &'a StartSlots,
        recording_id: Option<u64>,
    ) -> Self {
        Self {
            host,
            state,
            slots,
            tail: None,
            recording_id: Cell::new(recording_id),
        }
    }

    /// The ports of sites 103 and 104.
    pub(super) fn with_tail(mut self, tail: &'a TailFacts<'a>) -> Self {
        self.tail = Some(tail);
        self
    }

    /// The recording this start flight owns. In the `In` phase of R101.15 the id
    /// was just allocated under D, so the live id is the flight's.
    fn rid(&self) -> u64 {
        self.recording_id
            .get()
            .unwrap_or_else(|| self.state.app_state.recording_id.load(Ordering::SeqCst))
    }
}

impl Ports for StartPorts<'_> {
    fn app_state(&self) -> &AppState {
        &self.state.app_state
    }

    fn speculative_flowing(&self) -> bool {
        self.slots.speculative_flowing.get()
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
        self.tail.map(|tail| Arc::clone(&tail.context))
    }

    fn emit(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::EmitStatus { status, .. } => self
                .host
                .emit_event("recording-status-changed", status.as_str())
                .map_err(|_| PortFailure),
            // Publish Starting before the worker can report Ready, otherwise a
            // very fast device open could emit Recording first and this command
            // would then overwrite the frontend with a stale Starting event.
            Effect::EmitEvent(EventKind::GenStarted) => self
                .host
                .emit_event(
                    "dictation-generation-started",
                    serde_json::json!({ "recordingId": self.rid() }),
                )
                .map_err(|_| PortFailure),
            // R103.0: the smart-destination selection, under the shell's D hold.
            Effect::SmartDestinationEmit => {
                let tail = self.tail.ok_or(PortFailure)?;
                // The real result of the emit, never a constant.
                self.host
                    .emit_smart_destination(self.rid(), &tail.context, tail.delivery_target)
                    .map_err(|_| PortFailure)
            }
            _ => Err(PortFailure),
        }
    }

    fn trace(&self, effect: &Effect) -> PortResult {
        let Effect::TraceRequested = effect else {
            return Err(PortFailure);
        };
        let plan = self.slots.plan.borrow();
        let plan = plan.as_ref().ok_or(PortFailure)?;
        // The performance origin is stamped, and the denominator published, before
        // the state-transition trace (`accept_live_dictation_request`).
        self.slots
            .started_at_ms
            .set(Some(chrono::Utc::now().timestamp_millis()));
        tracing::info!(
            target: "pipeline",
            event_code = "pipeline.dictation_requested",
            slo_contract = DICTATION_SLO_CONTRACT,
            device_selection = plan.device_selection,
            recording_id = self.rid(),
            origin = plan.origin,
            "start_native_recording"
        );
        Ok(())
    }

    fn terminal(&self, effect: &Effect) -> PortResult {
        let Effect::Terminal(terminal) = effect else {
            return Err(PortFailure);
        };
        let claimed = emit_dictation_terminal(
            self.state,
            self.rid(),
            terminal_outcome_from_core(terminal.outcome),
            error_code_from_core(terminal.code),
        );
        // The log line that followed the claim stays right after it.
        if let Some(log) = self.slots.after_terminal.borrow_mut().take() {
            log();
        }
        // The ledger's answer is the witness: `Lost` (another sink won, or the
        // flight was never accepted: PD-10) is a failure of this claim.
        if claimed {
            Ok(())
        } else {
            Err(PortFailure)
        }
    }

    fn perf(&self, effect: &Effect) -> PortResult {
        match effect {
            // Reserve the generation before the worker is allowed to emit
            // lifecycle events. Terminal persistence can then wait for the
            // asynchronous begin without holding capture ownership or the
            // recording transition.
            Effect::PerfRegister => {
                let plan = self.slots.plan.borrow();
                let plan = plan.as_ref().ok_or(PortFailure)?;
                let rid = self.rid();
                self.host.performance().register_production(
                    rid,
                    plan.microphone_selection,
                    crate::audio_inventory::production_device_kind(
                        plan.selected_device_name.as_deref(),
                    ),
                );
                self.host.performance().begin_register(rid);
                Ok(())
            }
            Effect::PerfAbort => {
                #[cfg(test)]
                crate::barrier_hook::at("start.perf_abort");
                let rid = self.rid();
                self.host.performance().begin_abort(rid);
                // The supervisor's own text, logged where it always was: right
                // after the begin is aborted.
                if let Some(Err(error)) = self.slots.audio_result.borrow().as_ref() {
                    tracing::error!(target: "audio", "start_native_recording: audio failed: {}", error);
                }
                Ok(())
            }
            // Diagnostics begin once ownership is fully installed (T released).
            Effect::PerfBegin => {
                #[cfg(test)]
                crate::barrier_hook::at("start.perf_begin");
                let tail = self.tail.ok_or(PortFailure)?;
                let runtimes = super::runtime_identity(
                    &tail.context.transcription.model_name,
                    crate::performance_metrics::ModelWarmStateV1::Unknown,
                );
                self.host.performance().spawn_begin(
                    self.rid(),
                    runtimes,
                    tail.performance_started_at_ms,
                );
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn keyboard(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn claims(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::GestureAccepted => {
                let hold = self
                    .slots
                    .plan
                    .borrow()
                    .as_ref()
                    .map(|plan| plan.hold)
                    .ok_or(PortFailure)?;
                let claim = self
                    .slots
                    .gesture_claim
                    .borrow_mut()
                    .take()
                    .ok_or(PortFailure)?;
                *self.slots.gesture.borrow_mut() = Some(claim.accepted(hold));
                Ok(())
            }
            Effect::PrepareCaptureClaimed => {
                let gesture = self.slots.gesture.borrow();
                let gesture = gesture.as_ref().ok_or(PortFailure)?;
                crate::audio::prepare::capture_claimed(
                    crate::audio::prepare::CaptureClaim::Dictation(gesture.timing.map(|g| {
                        crate::audio::prepare::PrepareIdentity {
                            token: g.candidate.token,
                            listener_generation: self.host.keyboard().listener_generation(),
                            topology_generation: crate::audio_lifecycle::input_topology_generation(
                            ),
                            configuration_generation:
                                crate::audio::prepare::configuration_generation(),
                        }
                    })),
                );
                Ok(())
            }
            // A Fn key-down capture that already retains PCM is adopted with no
            // connecting phase: remember whether it was flowing before the hand-over.
            Effect::SpeculativeConsume => {
                let mut gesture = self.slots.gesture.borrow_mut();
                let gesture = gesture.as_mut().ok_or(PortFailure)?;
                let claim = self
                    .slots
                    .speculative
                    .borrow_mut()
                    .take()
                    .ok_or(PortFailure)?;
                self.slots.speculative_flowing.set(claim.is_flowing());
                gesture.speculative_capture = claim.consume();
                Ok(())
            }
            // The command's return: the speculative claim, then the gesture claim
            // (today's reverse-declaration drop order), after T was released.
            Effect::ReleaseClaims => {
                #[cfg(test)]
                crate::barrier_hook::at("start.release_claims");
                drop(self.slots.speculative.borrow_mut().take());
                // The gesture claim is a plain stamp: taking it is its release.
                self.slots.gesture_claim.borrow_mut().take();
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn blocking(&self, effect: &Effect) -> PortResult {
        match effect {
            // Freeze the native delivery target immediately after the accepted
            // Idle -> Starting transition, under the same ownership lock.
            Effect::AxSnapshot(AxMode::Accept) => {
                *self.slots.delivery_target.borrow_mut() =
                    Some(self.host.ax().capture_delivery_target_snapshot());
                Ok(())
            }
            // The direct (<= 2 s) form; the kept-preview form is awaited by the shell.
            Effect::CaptureStart(id) => {
                let (device, origin, gesture) = self.slots.start_args().ok_or(PortFailure)?;
                let result = self
                    .host
                    .supervisor()
                    .start_dictation_recording(device, id.0, origin, gesture);
                let ok = result.is_ok();
                self.slots.store_audio_result(result);
                if ok {
                    Ok(())
                } else {
                    Err(PortFailure)
                }
            }
            _ => Err(PortFailure),
        }
    }

    fn detached(&self, effect: &Effect) -> PortResult {
        match effect {
            // R104.7: the model-preparation attempt, self-gating in the host.
            Effect::ModelPreparation => {
                #[cfg(test)]
                crate::barrier_hook::at("start.model_preparation");
                let tail = self.tail.ok_or(PortFailure)?;
                self.host.spawn_model_preparation(
                    tail.context.transcription.model_name.clone(),
                    self.rid(),
                );
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }
}

/// The command's return after a refusal: T was released by the caller, so the
/// `PT:REL` effect now drops the claims (today's reverse-declaration drop order),
/// then the reply.
#[cfg_attr(feature = "dictation-shadow", allow(dead_code))]
pub(super) fn release_and_reply(
    host: &dyn DictationHost,
    state: &State,
    slots: &StartSlots,
    decision: Decision,
    recording_id: u64,
    smart_auto_error: Option<String>,
) -> Result<serde_json::Value, String> {
    let ports = StartPorts::new(host, state, slots, Some(recording_id));
    run_phase(
        &mut executor(&ports, decision.clone(), false),
        Placement::PostT,
        None,
        recording_id,
    );
    reply_of(&decision, smart_auto_error)
}
