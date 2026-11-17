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
    /// `is_dictation_cleanup_pending(current_id)` (`[in]`, 5523).
    pub cleanup_pending: bool,
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

/// Site 103: context install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartContextInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// No speculative capture is involved.
    pub speculative_none: bool,
    /// `q_capture_current` result (`[in]`, 5699).
    pub capture_is_current: bool,
}

/// Site 104: ready publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyPublishInput {
    /// The flight's token.
    pub token: Continuation,
    /// Validated token status.
    pub op_status: OpStatus,
    /// Recording id.
    pub rid: RecordingId,
    /// Supervisor phase/owner read before D (5306).
    pub audio_ready: bool,
    /// One leaf-mutex read under D (5314).
    pub context_present: bool,
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
    /// Legacy `process_audio` parse failure.
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
    /// Stop or legacy tail.
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
        /// Context installed (`[in]`, 4947).
        context_present: bool,
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
        /// Opaque failure kind (`from_audio_failure`).
        kind: Handle,
        /// Recovery reason, if the failure was a recovery.
        recovery_reason: Option<AudioCancelReason>,
    },
    /// Site 214.
    RecoveryStalled {
        /// Recording id.
        rid: RecordingId,
    },
    /// Sites 215 / 216.
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
}

/// Site 301.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessAudioStartInput {
    /// Occupancy read at 3684-3706.
    pub occupancy: Occupancy,
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
}

impl Input {
    /// The shadow `site_id` this input enters through. Where one input maps to
    /// two sites the choice follows C2.3: `StopContext` is 202 without a context
    /// (D) and 202n with one (no D); `Interrupted` is 215 (the D-less half; the
    /// short-idle second half 216 is reached from it).
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
            Self::GuardFinish(_) => SiteId::GuardFinish,
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
            },
            Self::ProcessAudioStart(_) => SiteId::ProcessAudioStart,
            Self::ProcessAudioFinish(_) => SiteId::ProcessAudioFinish,
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
            cleanup_pending: false,
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
}
