//! Typed decide-function inputs (appendix C2.3, C2.4, C3).
//!
//! One input struct per decide function, one [`Input`] variant per C0.3 site.
//! Observations the shell reads (`[pre]`, `[in]` in C2.3) are plain fields; the
//! core reads nothing itself. Transcript, audio and clipboard content never
//! appear here: payloads are opaque [`Handle`]s or lengths.

use crate::admission::{Occupancy, RefusalCode};
use crate::decision::{Continuation, OpStatus, Refusal, SiteId};
use crate::ids::{Handle, Now, RecordingId};
use crate::status::{AudioCancelReason, ErrCode, StopTrigger};

/// How a start was initiated (5487).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Hold-to-talk.
    Hold,
    /// Toggle.
    Toggle,
}

/// The microphone selection of a start (5543-5562).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceSel {
    /// System default.
    SystemDefault,
    /// A fixed device.
    Explicit,
    /// Smart Auto.
    SmartAuto,
}

/// The key-down gesture claim (`gesture_claim`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureObs {
    /// A gesture token accompanied the request.
    pub has_token: bool,
    /// The gesture is a hold.
    pub hold: bool,
}

/// The speculative claim (`SpeculativeClaim`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpeculativeObs {
    /// A speculative capture was claimed.
    pub claimed: bool,
}

/// A resolved Smart Auto selection; the selection itself is a shell payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartAutoSel {
    /// Opaque handle of the resolved selection.
    pub handle: Handle,
}

/// C2.4 `StartInput` (site 101).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartInput {
    /// Hold or toggle.
    pub origin: Origin,
    /// Device selection.
    pub device: DeviceSel,
    /// Gesture claim.
    pub gesture: GestureObs,
    /// Speculative claim.
    pub speculative: SpeculativeObs,
    /// A mic test was kept for the supervisor to hand over (`[pre]`, 5482).
    pub kept_preview: bool,
    /// `keyboard::is_app_disabled()` (`[pre]`, 5483).
    pub app_disabled: bool,
    /// `smart_auto && device_name` (5491).
    pub fixed_and_smart: bool,
    /// Read under D+T exactly where 5347-5393 read it.
    pub occupancy: Occupancy,
    /// `is_dictation_cleanup_pending(current_id)` (`[in]`, 5523). `None` until the
    /// shell answers `NeedObservation(CleanupPending)` (D-S1-2: read only after
    /// the blockers and status step passed).
    pub cleanup_pending: Option<bool>,
    /// Answer to `NeedObservation(SmartAutoSelection)`.
    pub smart_auto_result: Option<Result<SmartAutoSel, String>>,
}

impl StartInput {
    /// Pre-D refusals derivable from the input alone (C2.1 `pre_verdict`):
    /// `app_disabled` (5483-5486), then Smart Auto with a fixed device
    /// (5491-5493). When `Some`, no D section is entered.
    pub fn pre_verdict(&self) -> Option<Refusal> {
        if self.app_disabled {
            Some(Refusal::Admission(RefusalCode::AppDisabled))
        } else if self.fixed_and_smart {
            Some(Refusal::Admission(RefusalCode::SmartAutoWithFixedDevice))
        } else {
            None
        }
    }
}

/// `Ok` or the mapped `AudioStartError` of the capture start (5675-5685).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureErr {
    /// `AlreadyStarting`.
    AlreadyStarting,
    /// `AudioRecovering`.
    AudioRecovering,
    /// Any other start error; the text is shell-owned.
    Other,
}

/// Capture start/stop/cancel result (C2.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureResult {
    /// Capture started.
    Ok,
    /// Capture failed.
    Err(CaptureErr),
}

/// C2.4 `CaptureResultInput` (site 102).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureResultInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status (observation).
    pub op_status: OpStatus,
    /// Result.
    pub result: CaptureResult,
}

/// Site 103: context install (R103.1-.6). Whether the capture is still current
/// (R103.0) is decided earlier, by [`crate::start::decide_start_destination`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartContextInput {
    /// The flight's token (`K:ResolveContext`).
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
}

/// Site 104: ready publish (R104.1-.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyPublishInput {
    /// The flight's token (`K:ReadyPublish`).
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// Recording id.
    pub rid: RecordingId,
    /// Supervisor phase/owner read before D (5306).
    pub audio_ready: bool,
    /// `active_context(rid).is_some()`: one leaf-mutex read, taken only after the
    /// id and status guards passed (decision D-S1-2), so it is an `Option`:
    /// `None` makes the decision answer `NeedObservation(ContextPresence)`.
    pub context_present: Option<bool>,
    /// Stamp taken at 5816.
    pub context_ready_at: Now,
}

/// Metadata of a taken interrupted capture; the audio bytes stay in the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptedMeta {
    /// Preserved samples.
    pub samples: usize,
    /// Reason code.
    pub reason_code: ErrCode,
    /// Duration.
    pub duration_ms: u64,
}

/// C2.4 `StopInput` (site 201).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopInput {
    /// The owner the caller expects.
    pub expected: Option<RecordingId>,
    /// Trigger.
    pub trigger: StopTrigger,
    /// `recording_id` read under T (5899; cannot change under T).
    pub observed_current_id: RecordingId,
    /// Answer to `NeedObservation(TakeInterrupted)`:
    /// `Some(None)` nothing preserved, `Some(Some(meta))` taken.
    pub interrupted: Option<Option<InterruptedMeta>>,
    /// Stamp at 5897.
    pub entered: Now,
}

impl StopInput {
    /// `expected` names a recording that is not the one observed under T
    /// (5900): the stale path of R201.1, decided before any D section.
    pub fn is_stale(&self) -> bool {
        self.expected.is_some_and(|e| e != self.observed_current_id)
    }
}

/// Sites 202 / 202n: stop continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopContextInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// Recording id.
    pub rid: RecordingId,
    /// `[pre]` context read (6011); selects 202 (false) or 202n (true).
    pub context_present: bool,
    /// Interrupted error code, if any.
    pub interrupted_error: Option<ErrCode>,
}

/// Result of the capture stop (6064-6091).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCaptureResult {
    /// The stop failed.
    Err,
    /// The stop returned samples.
    Samples {
        /// Sample count.
        n: usize,
        /// Interrupted error code, if any.
        interrupted: Option<ErrCode>,
    },
}

/// Site 203.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StopCaptureResultInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// Recording id.
    pub rid: RecordingId,
    /// Result.
    pub result: StopCaptureResult,
}

/// Who finishes the `IdleGuard` (C2.3 row 204a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardOrigin {
    /// `IdleGuard::finish` after the capture stop.
    StopCapture,
    /// The pipeline guard (2885).
    PipelineExit,
    /// Legacy `process_audio` parse failure: site 302 (`ProcessAudioFinish`,
    /// R302.1); `handle` routes it there.
    LegacyParse,
    /// `IdleGuard::drop` (panic or early return).
    Dropped,
}

/// Site 204a.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardFinishInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Origin.
    pub origin: GuardOrigin,
    /// The `on_idle` closure emits `idle` (6076, 6135, 6169, 3781, 3787).
    pub closure_emit: bool,
}

/// Which post-pipeline tail (site 204b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostKind {
    /// Stop command (6209-6213).
    StopPost,
    /// Legacy `process_audio` (3817-3821).
    LegacyPost,
}

/// A pipeline's successful terminal (R204b.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineTerminal {
    /// Success.
    Success,
    /// No speech, with its code.
    NoSpeech(ErrCode),
    /// Cancelled at some stage.
    Cancelled,
}

/// The D-less tail result of a stop (R204b.5-.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineResult {
    /// The pipeline returned.
    Ok(PipelineTerminal),
    /// The pipeline failed.
    Err,
}

/// Site 204b.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineIdleEmitInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Stop or legacy tail. `LegacyPost` is decided by `decide_process_audio_finish`
    /// (site 302, R302.2 with its reply); `handle` routes it there.
    pub kind: PostKind,
    /// Tail result (`StopPost` only).
    pub result: Option<PipelineResult>,
    /// Interrupted error code, if any.
    pub interrupted: Option<ErrCode>,
    /// Character count of the delivered text (a length, never content).
    pub char_count: usize,
}

/// Site 205.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelInput {
    /// The owner the caller expects.
    pub expected: Option<RecordingId>,
}

/// Which command asked for the capture cancel whose result site 206 decides
/// (R201.4 through the stop command, R205.4 through the cancel command).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOrigin {
    /// The stop command's `Starting` arm (R201.4).
    Stop,
    /// The cancel command's `Starting` / `Recording` arm (R205.4).
    Cancel,
}

/// Site 206.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelCaptureResultInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// Recording id.
    pub rid: RecordingId,
    /// `Ok(bool)` or `Err`.
    pub result: Result<bool, ()>,
    /// The caller: only the success reply differs (stop replies
    /// `recording_cancelled`, cancel replies `Ok(())`).
    pub origin: CancelOrigin,
}

/// Supervisor-thread events (C3.6; never under T).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorEvent {
    /// Site 212.
    Accepted {
        /// Recording id.
        rid: RecordingId,
    },
    /// Site 207.
    Ready {
        /// Recording id.
        rid: RecordingId,
        /// Context installed (`[in]`, 4947). `None` until the decision answers
        /// `NeedObservation(ContextPresent)`: the shell passes `None` first and
        /// reads the slot only when asked (D-S1-2), so this carries both calls.
        context_present: Option<bool>,
        /// Stamp at 4927.
        ready_received: Now,
    },
    /// Site 208.
    StillConnecting {
        /// Recording id.
        rid: RecordingId,
    },
    /// Site 209.
    Recovering {
        /// Recording id.
        rid: RecordingId,
        /// Why.
        reason: AudioCancelReason,
    },
    /// Site 210.
    CleanupFinished {
        /// Recording id.
        rid: RecordingId,
    },
    /// Site 211.
    Idle {
        /// Recording id.
        rid: RecordingId,
    },
    /// Site 213.
    InitializationFailed {
        /// Recording id.
        rid: RecordingId,
        /// Opaque failure kind (`from_audio_failure`), for effects that carry it.
        kind: Handle,
        /// The terminal error code the shell computed with
        /// `from_audio_failure(kind)` **before** the decision (decision
        /// D-S1-4), so R213.2's terminal code is a pure function of the input
        /// and the core never needs the opaque kind.
        error_code: ErrCode,
        /// Recovery reason, if the failure was a recovery.
        recovery_reason: Option<AudioCancelReason>,
    },
    /// Site 214.
    RecoveryStalled {
        /// Recording id.
        rid: RecordingId,
    },
    /// Site 215: the D-less half of `Interrupted` (`is_current()` read without
    /// D, 5159): the stale and auto-transcribe arms, and the first half of the
    /// short-capture arm.
    Interrupted {
        /// Recording id.
        rid: RecordingId,
        /// Delivered samples (`≥ 8000` auto-transcribes, R215.2).
        delivered_samples: usize,
        /// Duration.
        duration_ms: u64,
        /// Reason.
        reason: AudioCancelReason,
    },
    /// Site 216: the second half of a short `Interrupted` (`< 8000` samples),
    /// a separate D section (5228-5230) with **no id or status guard**. The
    /// status it acts on is read from the core under that D hold (R216.1-.5),
    /// so the input carries only the recording id (C2.3 row 216).
    InterruptedShortIdle {
        /// Recording id.
        rid: RecordingId,
    },
}

/// Site 301.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessAudioStartInput {
    /// The facts read so far (3684-3706): `meeting_active` /
    /// `meeting_inference_active` before D, then `file_transcribing` under D. Unread
    /// facts keep their quiet defaults (decision D-S1-2).
    pub occupancy: Occupancy,
    /// `benchmark.is_running()` (3701): a coordinator-mutex read that today's code
    /// makes only after the file check passed, so it is `None` until the decision
    /// answers `NeedObservation(BenchmarkRunning)` (decision D-S1-2).
    pub benchmark_running: Option<bool>,
}

/// Which legacy finish (site 302).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessAudioFinishKind {
    /// Parse failure (R302.1, through 204a).
    ParseFailed,
    /// Pipeline done (R302.2, through 204b).
    PipelineDone,
}

/// Site 302.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessAudioFinishInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Which finish.
    pub kind: ProcessAudioFinishKind,
    /// `ParseFailed` only: the guard's `on_idle` closure emits `idle` (3780, 3786).
    /// `false` for a finish deferred re-entrantly (D-S2-4: the closure borrows the
    /// caller's frame and cannot be queued). Ignored by `PipelineDone`.
    pub closure_emit: bool,
}

/// Every decide-function input, one variant per C0.3 site.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Input {
    StartRequest(StartInput),
    StartCaptureResult(CaptureResultInput),
    StartContext(StartContextInput),
    ReadyPublish(ReadyPublishInput),
    StopRequest(StopInput),
    StopContext(StopContextInput),
    StopCaptureResult(StopCaptureResultInput),
    GuardFinish(GuardFinishInput),
    PipelineIdleEmit(PipelineIdleEmitInput),
    CancelRequest(CancelInput),
    CancelCaptureResult(CancelCaptureResultInput),
    Supervisor(SupervisorEvent),
    ProcessAudioStart(ProcessAudioStartInput),
    ProcessAudioFinish(ProcessAudioFinishInput),
    /// R103.0: the start's Smart Destination section (its own D hold, a separate
    /// decision of site 103).
    StartDestination(DestinationInput),
    /// R104.7: the end of the start flight (a pure decision of site 104).
    StartFlightEnd(StartFlightEndInput),
    /// R301.6: the import's Smart Destination section (a separate decision of
    /// site 301, the start's R103.0 under the import's own row id).
    ProcessAudioDestination(DestinationInput),
}

/// R103.0 / R301.6: the Smart Destination section's ownership recheck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DestinationInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Smart destination is enabled (the emit is made only then).
    pub smart_enabled: bool,
}

/// R104.7: the end of the start flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartFlightEndInput {
    /// Recording id.
    pub rid: RecordingId,
}

impl Input {
    /// The recording id the input names, if it names one: what the shadow taints
    /// when a record it could have touched is lost (C8.3 cascade control). A start
    /// request names none (its id is allocated by the decision, `ID+`); a stop,
    /// cancel or supervisor input that carries only an optional expectation names
    /// that expectation.
    pub fn recording_id(&self) -> Option<RecordingId> {
        match self {
            Self::StartRequest(_) | Self::ProcessAudioStart(_) => None,
            Self::StartCaptureResult(i) => Some(i.token.recording_id),
            Self::StartContext(i) => Some(i.token.recording_id),
            Self::ReadyPublish(i) => Some(i.rid),
            Self::StopRequest(i) => i.expected.or(Some(i.observed_current_id)),
            Self::StopContext(i) => Some(i.rid),
            Self::StopCaptureResult(i) => Some(i.rid),
            Self::GuardFinish(i) => Some(i.rid),
            Self::PipelineIdleEmit(i) => Some(i.rid),
            Self::CancelRequest(i) => i.expected,
            Self::CancelCaptureResult(i) => Some(i.rid),
            Self::Supervisor(event) => Some(match event {
                SupervisorEvent::Accepted { rid }
                | SupervisorEvent::Ready { rid, .. }
                | SupervisorEvent::StillConnecting { rid }
                | SupervisorEvent::Recovering { rid, .. }
                | SupervisorEvent::CleanupFinished { rid }
                | SupervisorEvent::Idle { rid }
                | SupervisorEvent::InitializationFailed { rid, .. }
                | SupervisorEvent::RecoveryStalled { rid }
                | SupervisorEvent::Interrupted { rid, .. }
                | SupervisorEvent::InterruptedShortIdle { rid } => *rid,
            }),
            Self::ProcessAudioFinish(i) => Some(i.rid),
            Self::StartDestination(i) | Self::ProcessAudioDestination(i) => Some(i.rid),
            Self::StartFlightEnd(i) => Some(i.rid),
        }
    }
}

impl Input {
    /// The shadow `site_id` this input enters through. Where one input maps to
    /// two sites the choice follows C2.3: `StopContext` is 202 without a context
    /// (D) and 202n with one (no D); `Interrupted` is 215 (the D-less half) and
    /// `InterruptedShortIdle` is 216 (the later D section). Every [`SiteId`] is
    /// reachable from some input (test `every_decision_site_has_an_input`).
    pub fn site(&self) -> SiteId {
        match self {
            Self::StartRequest(_) => SiteId::StartRequest,
            Self::StartCaptureResult(_) => SiteId::StartCaptureResult,
            Self::StartContext(_) => SiteId::StartContext,
            Self::ReadyPublish(_) => SiteId::ReadyPublish,
            Self::StopRequest(_) => SiteId::StopRequest,
            Self::StopContext(i) if i.context_present => SiteId::StopContextNoLock,
            Self::StopContext(_) => SiteId::StopContext,
            Self::StopCaptureResult(_) => SiteId::StopCaptureResult,
            // The legacy import's two finishes are site 302 (one representation,
            // `ProcessAudioFinish`): `handle` routes them through it.
            Self::GuardFinish(i) if i.origin == GuardOrigin::LegacyParse => {
                SiteId::ProcessAudioFinish
            }
            Self::GuardFinish(_) => SiteId::GuardFinish,
            Self::PipelineIdleEmit(i) if i.kind == PostKind::LegacyPost => {
                SiteId::ProcessAudioFinish
            }
            Self::PipelineIdleEmit(_) => SiteId::PipelineIdleEmit,
            Self::CancelRequest(_) => SiteId::CancelRequest,
            Self::CancelCaptureResult(_) => SiteId::CancelCaptureResult,
            Self::Supervisor(event) => match event {
                SupervisorEvent::Accepted { .. } => SiteId::SupAccepted,
                SupervisorEvent::Ready { .. } => SiteId::SupReady,
                SupervisorEvent::StillConnecting { .. } => SiteId::SupStillConnecting,
                SupervisorEvent::Recovering { .. } => SiteId::SupRecovering,
                SupervisorEvent::CleanupFinished { .. } => SiteId::SupCleanupFinished,
                SupervisorEvent::Idle { .. } => SiteId::SupIdle,
                SupervisorEvent::InitializationFailed { .. } => SiteId::SupInitFailed,
                SupervisorEvent::RecoveryStalled { .. } => SiteId::SupStalled,
                SupervisorEvent::Interrupted { .. } => SiteId::SupInterrupted,
                SupervisorEvent::InterruptedShortIdle { .. } => SiteId::SupInterruptedShortIdle,
            },
            Self::ProcessAudioStart(_) => SiteId::ProcessAudioStart,
            Self::ProcessAudioFinish(_) => SiteId::ProcessAudioFinish,
            Self::StartDestination(_) => SiteId::StartContext,
            Self::StartFlightEnd(_) => SiteId::ReadyPublish,
            Self::ProcessAudioDestination(_) => SiteId::ProcessAudioStart,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> StartInput {
        StartInput {
            origin: Origin::Hold,
            device: DeviceSel::SystemDefault,
            gesture: GestureObs::default(),
            speculative: SpeculativeObs::default(),
            kept_preview: false,
            app_disabled: false,
            fixed_and_smart: false,
            occupancy: Occupancy::default(),
            cleanup_pending: None,
            smart_auto_result: None,
        }
    }

    #[test]
    fn pre_verdict_orders_app_disabled_before_fixed_smart_auto() {
        assert_eq!(start().pre_verdict(), None);
        let mut both = start();
        both.app_disabled = true;
        both.fixed_and_smart = true;
        assert_eq!(
            both.pre_verdict(),
            Some(Refusal::Admission(RefusalCode::AppDisabled))
        );
        both.app_disabled = false;
        assert_eq!(
            both.pre_verdict(),
            Some(Refusal::Admission(RefusalCode::SmartAutoWithFixedDevice))
        );
    }

    #[test]
    fn stop_context_site_follows_the_context_read() {
        let token = Continuation {
            recording_id: RecordingId(1),
            op: crate::ids::OperationId(1),
            kind: crate::decision::ContKind::StopContext,
        };
        let mk = |context_present| {
            Input::StopContext(StopContextInput {
                token,
                op_status: OpStatus::Outstanding,
                rid: RecordingId(1),
                context_present,
                interrupted_error: None,
            })
        };
        assert_eq!(mk(false).site(), SiteId::StopContext);
        assert_eq!(mk(true).site(), SiteId::StopContextNoLock);
    }

    #[test]
    fn supervisor_events_map_to_their_sites() {
        let rid = RecordingId(3);
        let cases = [
            (SupervisorEvent::Accepted { rid }, 212),
            (SupervisorEvent::StillConnecting { rid }, 208),
            (SupervisorEvent::CleanupFinished { rid }, 210),
            (SupervisorEvent::Idle { rid }, 211),
            (SupervisorEvent::RecoveryStalled { rid }, 214),
        ];
        for (event, code) in cases {
            assert_eq!(Input::Supervisor(event).site().code(), code);
        }
    }

    fn every_input() -> Vec<Input> {
        let rid = RecordingId(1);
        let token = Continuation {
            recording_id: rid,
            op: crate::ids::OperationId(1),
            kind: crate::decision::ContKind::CaptureStart,
        };
        let op_status = OpStatus::Outstanding;
        let stop_context = |context_present| {
            Input::StopContext(StopContextInput {
                token,
                op_status,
                rid,
                context_present,
                interrupted_error: None,
            })
        };
        vec![
            Input::StartRequest(start()),
            Input::StartCaptureResult(CaptureResultInput {
                token,
                op_status,
                result: CaptureResult::Ok,
            }),
            Input::StartContext(StartContextInput { token, op_status }),
            Input::ReadyPublish(ReadyPublishInput {
                token,
                op_status,
                rid,
                audio_ready: true,
                context_present: Some(true),
                context_ready_at: Now(0),
            }),
            Input::StopRequest(StopInput {
                expected: None,
                trigger: StopTrigger::User,
                observed_current_id: rid,
                interrupted: None,
                entered: Now(0),
            }),
            stop_context(false),
            stop_context(true),
            Input::StopCaptureResult(StopCaptureResultInput {
                token,
                op_status,
                rid,
                result: StopCaptureResult::Err,
            }),
            Input::GuardFinish(GuardFinishInput {
                rid,
                origin: GuardOrigin::StopCapture,
                closure_emit: true,
            }),
            Input::PipelineIdleEmit(PipelineIdleEmitInput {
                rid,
                kind: PostKind::StopPost,
                result: None,
                interrupted: None,
                char_count: 0,
            }),
            Input::CancelRequest(CancelInput { expected: None }),
            Input::CancelCaptureResult(CancelCaptureResultInput {
                token,
                op_status,
                rid,
                result: Ok(true),
                origin: CancelOrigin::Cancel,
            }),
            Input::Supervisor(SupervisorEvent::Ready {
                rid,
                context_present: Some(true),
                ready_received: Now(0),
            }),
            Input::Supervisor(SupervisorEvent::StillConnecting { rid }),
            Input::Supervisor(SupervisorEvent::Recovering {
                rid,
                reason: AudioCancelReason::User,
            }),
            Input::Supervisor(SupervisorEvent::CleanupFinished { rid }),
            Input::Supervisor(SupervisorEvent::Idle { rid }),
            Input::Supervisor(SupervisorEvent::Accepted { rid }),
            Input::Supervisor(SupervisorEvent::InitializationFailed {
                rid,
                kind: Handle(0),
                error_code: ErrCode::InitializationTimeout,
                recovery_reason: None,
            }),
            Input::Supervisor(SupervisorEvent::RecoveryStalled { rid }),
            Input::Supervisor(SupervisorEvent::Interrupted {
                rid,
                delivered_samples: 0,
                duration_ms: 0,
                reason: AudioCancelReason::RuntimeFailure,
            }),
            Input::Supervisor(SupervisorEvent::InterruptedShortIdle { rid }),
            Input::ProcessAudioStart(ProcessAudioStartInput {
                occupancy: Occupancy::default(),
                benchmark_running: None,
            }),
            Input::ProcessAudioFinish(ProcessAudioFinishInput {
                rid,
                kind: ProcessAudioFinishKind::ParseFailed,
                closure_emit: true,
            }),
        ]
    }

    #[test]
    fn every_decision_site_has_an_input() {
        let mut sites: Vec<u16> = every_input().iter().map(|i| i.site().code()).collect();
        sites.sort_unstable();
        let mut expected: Vec<u16> = SiteId::ALL.iter().map(|s| s.code()).collect();
        expected.sort_unstable();
        assert_eq!(sites, expected, "each site is reached by exactly one input");
    }

    #[test]
    fn sub_section_inputs_belong_to_their_parent_site() {
        // R103.0, R104.7 and R301.6 are decisions of their own that the C0.3
        // site table folds into a parent site; none adds a 25th site.
        let rid = RecordingId(1);
        let dest = DestinationInput {
            rid,
            smart_enabled: true,
        };
        assert_eq!(Input::StartDestination(dest).site(), SiteId::StartContext);
        assert_eq!(
            Input::StartFlightEnd(StartFlightEndInput { rid }).site(),
            SiteId::ReadyPublish
        );
        assert_eq!(
            Input::ProcessAudioDestination(dest).site(),
            SiteId::ProcessAudioStart
        );
        for input in [
            Input::StartDestination(dest),
            Input::StartFlightEnd(StartFlightEndInput { rid }),
            Input::ProcessAudioDestination(dest),
        ] {
            assert_eq!(input.recording_id(), Some(rid));
        }
    }

    #[test]
    fn recording_id_names_what_a_lost_record_could_have_touched() {
        let rid = RecordingId(4);
        assert_eq!(Input::StartRequest(start()).recording_id(), None);
        assert_eq!(
            Input::StopRequest(StopInput {
                expected: Some(RecordingId(2)),
                trigger: StopTrigger::User,
                observed_current_id: rid,
                interrupted: None,
                entered: Now(0),
            })
            .recording_id(),
            Some(RecordingId(2)),
            "a stale stop names the recording it expected"
        );
        assert_eq!(
            Input::CancelRequest(CancelInput { expected: None }).recording_id(),
            None
        );
        for input in every_input() {
            if !matches!(
                input,
                Input::StartRequest(_) | Input::ProcessAudioStart(_) | Input::CancelRequest(_)
            ) {
                assert!(input.recording_id().is_some(), "{input:?}");
            }
        }
    }

    #[test]
    fn initialization_failed_carries_the_observed_error_code() {
        // R213.2's terminal code comes from the input, not from the opaque kind:
        // the same kind with a different observed code is a different input,
        // and every code is carried verbatim.
        let rid = RecordingId(4);
        let event = |error_code| {
            Input::Supervisor(SupervisorEvent::InitializationFailed {
                rid,
                kind: Handle(7),
                error_code,
                recovery_reason: None,
            })
        };
        for code in [
            ErrCode::InitializationTimeout,
            ErrCode::PermissionDenied,
            ErrCode::DeviceUnavailable,
            ErrCode::FirstBufferTimeout,
        ] {
            let Input::Supervisor(SupervisorEvent::InitializationFailed {
                error_code, kind, ..
            }) = event(code)
            else {
                unreachable!("built as InitializationFailed");
            };
            assert_eq!(error_code, code);
            assert_eq!(kind, Handle(7));
            assert_eq!(event(code).site(), SiteId::SupInitFailed);
            assert_eq!(event(code).site().code(), 213);
        }
        assert_ne!(
            event(ErrCode::PermissionDenied),
            event(ErrCode::DeviceUnavailable)
        );
    }

    #[test]
    fn short_interrupted_second_half_is_site_216() {
        let rid = RecordingId(9);
        let first = Input::Supervisor(SupervisorEvent::Interrupted {
            rid,
            delivered_samples: 100,
            duration_ms: 10,
            reason: AudioCancelReason::RuntimeFailure,
        });
        let second = Input::Supervisor(SupervisorEvent::InterruptedShortIdle { rid });
        assert_eq!(first.site(), SiteId::SupInterrupted);
        assert_eq!(second.site(), SiteId::SupInterruptedShortIdle);
        assert_eq!(second.site().code(), 216);
    }
}
