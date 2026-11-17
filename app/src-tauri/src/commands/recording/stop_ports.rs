//! The production [`Ports`] of the stop family (Stage 2a-2, sites 201-204b):
//! each effect is today's statement, run by the executor at the placement the
//! decision gave it.
//!
//! Three small port sets serve the four shells:
//!
//! * [`StopPorts`]: the stop command's own effects (201, 202, 203 and the
//!   204b tail). It owns the shell-local values an effect produces or consumes and
//!   the core never sees (X4): the preview-hide claim, the frozen delivery
//!   target, the gesture stamps, a failed capture cancel, the transition guard
//!   that R202.3 releases inside D, and the three tail hooks (`TERM` with the
//!   private capture, `PE` / `PGF` on the performance guard, `EV`).
//! * [`GuardPorts`]: the `IdleGuard` finish (204a). Its `EM(idle)` runs the
//!   guard's `on_idle` closure with the held `DictationState` (`emit_under_d`).
//! * [`IdleEmitPorts`]: the second D section after a pipeline (204b), shared by
//!   the stop command and the legacy `process_audio`.
//!
//! No port takes a lock of its own: the field effects (`ST`, `CX-`) go through
//! the `FieldAdapter` on the guard the shell passes the executor, and every other
//! effect is the statement the code ran inline before.

use super::{emit_dictation_terminal, DictationHost, State};
use crate::audio_lifecycle::AudioCancelReason;
use crate::dictation_context::DictationContextSnapshot;
use crate::dictation_executor::convert::{error_code_from_core, terminal_outcome_from_core};
use crate::dictation_executor::executor::{
    Executor, PhaseCtx, PhaseReport, PortFailure, PortResult, Ports,
};
use crate::dictation_telemetry::{DictationErrorCode, DictationTerminalOutcome};
use crate::performance_metrics::{PerformanceStageV1, RunOutcomeV1, StableRunErrorV1};
use crate::state::{AppState, DictationState};
use murmur_dictation_core::decision::Decision;
use murmur_dictation_core::effects::{
    Effect, EventKind, PerfCause, PerfOutcome, PerfStage, Placement,
};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::replies::{ErrText, Reply, TranscriptText};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// Wrap a decision in an executor. A stop decision has at most one awaited
/// effect (`PS`, last of its phase) and the crate tests validate every row, so a
/// failure here is a programming error.
pub(super) fn executor<'a>(ports: &'a dyn Ports, decision: Decision) -> Executor<'a> {
    Executor::new(ports, decision, false)
        .expect("a stop decision is structurally valid (crate row tests)")
}

/// Run a (pure) stop decide function under the re-entrancy guard: a nested call
/// is refused as `Invalid(Reentrant)` without calling `decide` (C4.2).
pub(super) fn decide(decide: impl FnOnce() -> Decision) -> Decision {
    crate::dictation_executor::executor::decide_guarded(decide)
}

/// Run one placement. `dictation` is `Some` only for `In`: the D guard's
/// contents the shell holds. Returns `true` when the executor ran the phase (or
/// stopped at the awaited `PS`); a refused or out-of-order run does nothing and is
/// logged. Today's code has no such path.
pub(super) fn run_phase(
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
    if let PhaseReport::Invalid(_) = report {
        tracing::error!(
            target: "pipeline",
            recording_id,
            phase = phase.label(),
            "stop effects were not run: the executor refused the phase"
        );
    }
    report
}

/// The awaited `PS` of an accepted pipeline start (R203.9/.10), or the refusal
/// text the stop returns when the executor did not reach it.
pub(super) fn pipeline_start(report: PhaseReport) -> Result<(), String> {
    match report {
        PhaseReport::Awaiting(awaited) if matches!(awaited.effect, Effect::PipelineSpawn(_)) => {
            Ok(())
        }
        other => {
            tracing::error!(
                target: "pipeline",
                report = ?other,
                "the stop's effects did not reach the awaited pipeline start"
            );
            Err("The dictation core did not start the pipeline".to_string())
        }
    }
}

// Test builds: the witness of the stop's executor after it resumed the awaited
// `PS` with the pipeline's actual result.
#[cfg(test)]
thread_local! {
    pub(super) static RESUMED: RefCell<Vec<crate::dictation_executor::executor::Witness>> =
        const { RefCell::new(Vec::new()) };
}

/// The JSON of a stop reply, with the shell-owned slots filled in: `shell_error`
/// for `Err(Shell)` and `text` for a `FromPipeline` transcription.
pub(super) fn render_reply(
    reply: &Reply,
    shell_error: Option<String>,
    text: &str,
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
            recording_id: Some(_),
            ..
        } => unreachable!("only a start reply carries a recording id"),
        Reply::Transcription { text: slot, state } => Ok(serde_json::json!({
            "type": "transcription",
            "text": match slot {
                TranscriptText::Empty => "",
                TranscriptText::FromPipeline => text,
            },
            "state": state.as_str(),
        })),
        Reply::Ok => unreachable!("a stop never replies with a bare Ok"),
        Reply::Err(ErrText::Shell) => {
            Err(shell_error.expect("a shell error reply carries its text"))
        }
        Reply::Err(other) => Err(other
            .message()
            .expect("every non-shell error text is literal")),
    }
}

/// The reply of a stop decision: a refused entry (`Invalid`, nothing ran) is an
/// `Err`, anything else renders its typed reply.
pub(super) fn reply_of(
    decision: &Decision,
    shell_error: Option<String>,
    text: &str,
) -> Result<serde_json::Value, String> {
    use murmur_dictation_core::decision::Verdict;
    if let Verdict::Invalid(kind) = &decision.verdict {
        return Err(format!("The dictation core refused the stop: {kind:?}"));
    }
    let reply = decision
        .reply
        .as_ref()
        .expect("a stop decision that ran carries its row's reply");
    render_reply(reply, shell_error, text)
}

// --------------------------------------------------------------- stop command

/// The values the stop command's effects produce for the shell.
#[derive(Default)]
pub(super) struct StopSlots {
    /// `PH`: the live preview's hide claim.
    pub(super) preview_hide: Option<crate::commands::dictation_preview::PreviewHideOwner>,
    /// `CC`: the capture-cancel error (R206 is still inline in the shell).
    pub(super) cancel_error: Option<String>,
    /// `AX`: the delivery target frozen at the accepted stop.
    pub(super) delivery_target: Option<crate::frontmost::DeliveryTargetSnapshot>,
    /// `TS`: the matching rdev release.
    pub(super) hotkey_release: Option<crate::hotkey_release::HotkeyRelease>,
    /// `TS`: the pending stop-key stamp.
    pub(super) stop_stamp: Option<crate::stop_gesture::StopStamp>,
}

/// Hooks for the effects whose payload is a shell local (X4): they run the
/// statement the stop command ran at that point.
#[derive(Default)]
pub(super) struct StopHooks<'a> {
    /// `TERM` in the tail: `emit_dictation_terminal_with_capture`, then the
    /// metrics and stage-gap trace lines that followed it.
    pub(super) terminal: Option<Box<dyn Fn(DictationTerminalOutcome, DictationErrorCode) + 'a>>,
    /// The log line that followed the capture-result `TERM` claim (R203): run
    /// right after the default terminal emit, before `CX-`.
    pub(super) after_terminal: Option<Box<dyn FnOnce() + 'a>>,
    /// `PE`: `performance_guard.enter(Vad)`.
    pub(super) perf_enter: Option<Box<dyn FnOnce() + 'a>>,
    /// `PGF`: `performance_guard.finish(..)`.
    pub(super) perf_finish: Option<Box<dyn FnOnce() + 'a>>,
    /// `EV(TranscriptionComplete)`: the `transcription-complete` broadcast.
    /// Returns whether the broadcast was delivered, which the witness reports.
    pub(super) event: Option<Box<dyn FnOnce() -> bool + 'a>>,
}

pub(super) struct StopPorts<'a> {
    host: &'a dyn DictationHost,
    state: &'a State,
    recording_id: Cell<u64>,
    stop_entered_at: std::time::Instant,
    /// Site 203's `KB(false)` is the `IdleGuard`'s D section (204a), which clears
    /// the gate through the keyboard module directly, not through the host.
    guard_keyboard: Cell<bool>,
    slots: RefCell<StopSlots>,
    hooks: RefCell<StopHooks<'a>>,
    /// R202.3 releases T inside D, between `EM(idle)` and `PC` (B0 E1): the shell
    /// hands the guard over and the `PC` port drops it first.
    transition: RefCell<Option<tokio::sync::MutexGuard<'a, ()>>>,
    /// `CP`: how the capture stop is driven, and what it returned.
    capture: RefCell<Option<CaptureStop>>,
}

/// The shell values `CP` (R202.1, `PT`) needs and produces (X4). An interrupted
/// capture's samples bypass the supervisor: the supervisor already preserved them.
pub(super) struct CaptureStop {
    trigger: crate::audio_lifecycle::DictationStopTrigger,
    tail_anchor: Option<std::time::Instant>,
    interrupted: Option<Vec<f32>>,
    result: Option<Result<Vec<f32>, String>>,
}

impl<'a> StopPorts<'a> {
    pub(super) fn new(
        host: &'a dyn DictationHost,
        state: &'a State,
        recording_id: u64,
        stop_entered_at: std::time::Instant,
    ) -> Self {
        StopPorts {
            host,
            state,
            recording_id: Cell::new(recording_id),
            stop_entered_at,
            guard_keyboard: Cell::new(false),
            slots: RefCell::default(),
            hooks: RefCell::default(),
            transition: RefCell::new(None),
            capture: RefCell::new(None),
        }
    }

    /// Route `KB` through the keyboard module, as the `IdleGuard` does.
    pub(super) fn use_guard_keyboard(&self) {
        self.guard_keyboard.set(true);
    }

    pub(super) fn set_hooks(&self, hooks: StopHooks<'a>) {
        *self.hooks.borrow_mut() = hooks;
    }

    /// Arm `CP`: the resolved trigger, the tail anchor and, for an interrupted
    /// capture, the preserved samples.
    pub(super) fn arm_capture_stop(
        &self,
        trigger: crate::audio_lifecycle::DictationStopTrigger,
        tail_anchor: Option<std::time::Instant>,
        interrupted: Option<Vec<f32>>,
    ) {
        *self.capture.borrow_mut() = Some(CaptureStop {
            trigger,
            tail_anchor,
            interrupted,
            result: None,
        });
    }

    /// What `CP` returned: the samples, or the supervisor's error. `None` if the
    /// capture stop never ran.
    pub(super) fn take_capture_result(&self) -> Option<Result<Vec<f32>, String>> {
        self.capture
            .borrow_mut()
            .as_mut()
            .and_then(|capture| capture.result.take())
    }

    pub(super) fn hold_transition(&self, guard: tokio::sync::MutexGuard<'a, ()>) {
        *self.transition.borrow_mut() = Some(guard);
    }

    /// The values the effects produced.
    pub(super) fn take_slots(&self) -> StopSlots {
        std::mem::take(&mut *self.slots.borrow_mut())
    }
}

impl Ports for StopPorts<'_> {
    fn app_state(&self) -> &AppState {
        &self.state.app_state
    }

    fn speculative_flowing(&self) -> bool {
        false
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
        None
    }

    fn emit(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::EmitStatus { status, .. } => self
                .host
                .emit("recording-status-changed", status.as_str())
                .map_err(|_| PortFailure),
            Effect::EmitEvent(EventKind::TranscriptionComplete) => {
                let hook = self.hooks.borrow_mut().event.take().ok_or(PortFailure)?;
                hook().then_some(()).ok_or(PortFailure)
            }
            _ => Err(PortFailure),
        }
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, effect: &Effect) -> PortResult {
        let Effect::Terminal(terminal) = effect else {
            return Err(PortFailure);
        };
        let outcome = terminal_outcome_from_core(terminal.outcome);
        let code = error_code_from_core(terminal.code);
        let hooks = self.hooks.borrow();
        match hooks.terminal.as_ref() {
            Some(hook) => hook(outcome, code),
            None => {
                emit_dictation_terminal(self.state, self.recording_id.get(), outcome, code);
            }
        }
        drop(hooks);
        if let Some(log) = self.hooks.borrow_mut().after_terminal.take() {
            log();
        }
        Ok(())
    }

    fn perf(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::PerfComplete(pc) => {
                let outcome = match (pc.outcome, pc.stage, pc.cause) {
                    (
                        PerfOutcome::Failed,
                        PerfStage::CaptureFinalization,
                        Some(PerfCause::AudioCaptureFailed),
                    ) => RunOutcomeV1::Failed {
                        stage: PerformanceStageV1::CaptureFinalization,
                        error_code: StableRunErrorV1::AudioCaptureFailed,
                    },
                    (
                        PerfOutcome::Failed,
                        PerfStage::CaptureFinalization,
                        Some(PerfCause::InternalEarlyExit),
                    ) => {
                        // R202.3: T is released inside D, before the completion
                        // is recorded (B0 E1).
                        self.transition.borrow_mut().take();
                        RunOutcomeV1::Failed {
                            stage: PerformanceStageV1::CaptureFinalization,
                            error_code: StableRunErrorV1::InternalEarlyExit,
                        }
                    }
                    _ => return Err(PortFailure),
                };
                self.host.performance().complete(
                    self.recording_id.get(),
                    outcome,
                    Vec::new(),
                    None,
                    None,
                );
                Ok(())
            }
            Effect::PipelineEnter(_) => {
                let hook = self
                    .hooks
                    .borrow_mut()
                    .perf_enter
                    .take()
                    .ok_or(PortFailure)?;
                hook();
                Ok(())
            }
            Effect::PipelineGuardFinish(_) => {
                let hook = self
                    .hooks
                    .borrow_mut()
                    .perf_finish
                    .take()
                    .ok_or(PortFailure)?;
                hook();
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        if self.guard_keyboard.get() {
            crate::keyboard::set_processing(*processing);
        } else {
            self.host.keyboard().set_processing(*processing);
        }
        Ok(())
    }

    fn claims(&self, effect: &Effect) -> PortResult {
        let recording_id = self.recording_id.get();
        match effect {
            Effect::PreviewHideClaim => {
                self.slots.borrow_mut().preview_hide = self.host.claim_preview_hide(recording_id);
                Ok(())
            }
            Effect::TakeStopStamps => {
                // The release is claimed at the accepted transition, before
                // teardown, so a later release queued behind this stop cannot
                // replace it.
                let hotkey_release = self
                    .host
                    .keyboard()
                    .hotkey_release_take_for(recording_id, self.stop_entered_at);
                let stop_stamp = self
                    .host
                    .keyboard()
                    .stop_gesture_take_for(recording_id, self.stop_entered_at);
                let mut slots = self.slots.borrow_mut();
                slots.hotkey_release = hotkey_release;
                slots.stop_stamp = stop_stamp;
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn blocking(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::AxSnapshot(_) => {
                // Freeze the native delivery target at the accepted stop
                // transition, under the same ownership lock that commits
                // Processing.
                self.slots.borrow_mut().delivery_target =
                    Some(self.host.ax().capture_delivery_target_snapshot());
                Ok(())
            }
            Effect::CaptureStop(id) => {
                let (trigger, tail_anchor, interrupted) = {
                    let mut capture = self.capture.borrow_mut();
                    let capture = capture.as_mut().ok_or(PortFailure)?;
                    (
                        capture.trigger,
                        capture.tail_anchor,
                        capture.interrupted.take(),
                    )
                };
                let result = match interrupted {
                    // An interrupted capture already holds its samples.
                    Some(samples) => Ok(samples),
                    None => {
                        self.host
                            .supervisor()
                            .stop_dictation_recording(id.0, trigger, tail_anchor)
                    }
                };
                let outcome = if result.is_ok() {
                    Ok(())
                } else {
                    Err(PortFailure)
                };
                if let Some(capture) = self.capture.borrow_mut().as_mut() {
                    capture.result = Some(result);
                }
                outcome
            }
            Effect::CaptureCancel { id, .. } => {
                // Cancel the exact owner even if readiness races with this
                // command after the status snapshot. A starting-only cancellation
                // could otherwise no-op while the command falsely reports
                // cancellation and leaves a newly live stream running.
                match self
                    .host
                    .supervisor()
                    .cancel_dictation_capture(id.0, AudioCancelReason::User)
                {
                    Ok(_) => Ok(()),
                    Err(error) => {
                        self.slots.borrow_mut().cancel_error = Some(error.to_string());
                        Err(PortFailure)
                    }
                }
            }
            _ => Err(PortFailure),
        }
    }

    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }
}

// -------------------------------------------------------------- 204a: IdleGuard

/// The guard's `on_idle` closure: it runs as `EM(idle)` with D's contents and
/// reports whether the emit it made was delivered.
pub(super) type OnIdle<'a> = Box<dyn FnOnce(&DictationState) -> bool + 'a>;

/// What an emit statement reports to the executor's witness: the closure bodies
/// that predate the executor return `()` (nothing to report) or a `Result`.
pub(super) trait EmitOutcome {
    fn delivered(&self) -> bool;
}

impl EmitOutcome for () {
    fn delivered(&self) -> bool {
        true
    }
}

impl EmitOutcome for bool {
    fn delivered(&self) -> bool {
        *self
    }
}

impl<T, E> EmitOutcome for Result<T, E> {
    fn delivered(&self) -> bool {
        self.is_ok()
    }
}

/// The ports of `IdleGuard::finish` / `Drop`. `EM(idle)` runs the guard's
/// `on_idle` closure with the held `DictationState`.
pub(super) struct GuardPorts<'a> {
    app_state: &'a AppState,
    on_idle: RefCell<Option<OnIdle<'a>>>,
}

impl<'a> GuardPorts<'a> {
    pub(super) fn new(app_state: &'a AppState, on_idle: Option<OnIdle<'a>>) -> Self {
        GuardPorts {
            app_state,
            on_idle: RefCell::new(on_idle),
        }
    }
}

impl Ports for GuardPorts<'_> {
    fn app_state(&self) -> &AppState {
        self.app_state
    }

    fn speculative_flowing(&self) -> bool {
        false
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
        None
    }

    fn emit(&self, _effect: &Effect) -> PortResult {
        // `EM(idle)` is only ever decided in `In`, where `emit_under_d` runs.
        Err(PortFailure)
    }

    fn emit_under_d(&self, effect: &Effect, dictation: &DictationState) -> PortResult {
        let Effect::EmitStatus { .. } = effect else {
            return Err(PortFailure);
        };
        let on_idle = self.on_idle.borrow_mut().take().ok_or(PortFailure)?;
        on_idle(dictation).then_some(()).ok_or(PortFailure)
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn perf(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        crate::keyboard::set_processing(*processing);
        Ok(())
    }

    fn claims(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn blocking(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }
}

// ------------------------------------------------------- 204b: pipeline idle emit

/// The ports of the post-pipeline D section: `KB(false)` and `EM(idle)`. The two
/// callers differ only in how they reach the keyboard gate and the webview (the
/// stop command through its host, the legacy `process_audio` directly), so both
/// are closures.
pub(super) struct IdleEmitPorts<'a> {
    app_state: &'a AppState,
    set_processing: &'a dyn Fn(bool),
    emit_idle: &'a dyn Fn() -> bool,
}

impl<'a> IdleEmitPorts<'a> {
    pub(super) fn new(
        app_state: &'a AppState,
        set_processing: &'a dyn Fn(bool),
        emit_idle: &'a dyn Fn() -> bool,
    ) -> Self {
        IdleEmitPorts {
            app_state,
            set_processing,
            emit_idle,
        }
    }
}

impl Ports for IdleEmitPorts<'_> {
    fn app_state(&self) -> &AppState {
        self.app_state
    }

    fn speculative_flowing(&self) -> bool {
        false
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
        None
    }

    fn emit(&self, effect: &Effect) -> PortResult {
        let Effect::EmitStatus { .. } = effect else {
            return Err(PortFailure);
        };
        (self.emit_idle)().then_some(()).ok_or(PortFailure)
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn perf(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        (self.set_processing)(*processing);
        Ok(())
    }

    fn claims(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn blocking(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_dictation_core::effects::Cond;
    use murmur_dictation_core::status::Status;

    fn idle_emit() -> Effect {
        Effect::EmitStatus {
            status: Status::Idle,
            cond: Cond::Always,
        }
    }

    /// The witness reports the emit the statement made, not a constant `Ok`: a
    /// failed `idle` emit through the guard's closure, the shared 204b port or the
    /// `transcription-complete` hook is a failed port call (the command's reply is
    /// unchanged by it).
    #[test]
    fn the_guard_and_the_idle_emit_ports_report_the_real_emit_result() {
        let app_state = AppState::default();
        for delivered in [true, false] {
            let guard_ports = GuardPorts::new(&app_state, Some(Box::new(move |_| delivered)));
            let dictation = app_state.dictation.lock().unwrap();
            assert_eq!(
                guard_ports.emit_under_d(&idle_emit(), &dictation).is_ok(),
                delivered
            );
            drop(dictation);
            let set_processing = |_: bool| {};
            let emit_idle = move || delivered;
            let idle_ports = IdleEmitPorts::new(&app_state, &set_processing, &emit_idle);
            assert_eq!(idle_ports.emit(&idle_emit()).is_ok(), delivered);
        }
    }

    #[test]
    fn emit_outcomes_of_the_closure_bodies_that_predate_the_executor() {
        assert!(().delivered());
        assert!(true.delivered());
        assert!(!false.delivered());
        assert!(Ok::<(), ()>(()).delivered());
        assert!(!Err::<(), _>("failed").delivered());
    }
}
