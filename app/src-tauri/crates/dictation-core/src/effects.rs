//! Effects and placement classes (appendix C0.1, C0.2).
//!
//! A decision returns an ordered list of [`Placed`] effects. The core never
//! runs them: the shell executes each phase at its site's own boundaries
//! (`Pre` → acquire D → decide → `In` → release D → `PostDT` → release T →
//! `PostT`, C4.1) and the executor never takes D or T.
//!
//! `HIST` (C0.2) is deliberately **not** an [`Effect`]: it is the shell's
//! `core_history` bookkeeping right after `decide_*` returns.

use crate::ids::{Handle, RecordingId};
use crate::status::{AudioCancelReason, ErrCode, Status, StopTrigger, TerminalOutcome};

/// Where an effect runs relative to the locks (C0.1). Order is the execution
/// order: `Pre < In < PostDT < PostT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Placement {
    /// `Pre`: before this decision's D section, on the caller's thread.
    Pre,
    /// `In`: while D is held, in returned order.
    In,
    /// `PDT` (B's PostD_T): after D is released; T still held at command sites.
    PostDT,
    /// `PT` (B's PostT): after T is released.
    PostT,
}

impl Placement {
    /// The label used in appendix C tables.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pre => "Pre",
            Self::In => "In",
            Self::PostDT => "PDT",
            Self::PostT => "PT",
        }
    }
}

/// An effect with its placement class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    /// Placement class.
    pub at: Placement,
    /// The effect.
    pub effect: Effect,
}

impl Placed {
    /// Pair an effect with a placement.
    pub fn new(at: Placement, effect: Effect) -> Self {
        Self { at, effect }
    }
}

/// A shell predicate that may skip an effect (C0.2 `cond: PredId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    /// Always run.
    Always,
    /// Skipped when `speculative_flowing` (`P-spec-flowing`, `EM(starting)` 5618).
    UnlessSpeculativeFlowing,
    /// Runs only when the paired emit (the effect at this index of the same
    /// decision) returned `Ok` (`P-emit-ok(i)`, `PRES`).
    EmitOk(u8),
}

/// `EV(k)`: typed event emits (C0.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum EventKind {
    GenStarted,
    InitStalled,
    RecoveryStarted,
    Cancelled,
    InitFailed,
    CleanupFinished,
    RecoveryStalled,
    Interrupted { auto_transcribe: bool },
    TranscriptionComplete,
}

/// `PRES(k)`: `pipeline.dictation_presentation` trace kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PresentationKind {
    CleanupInProgress,
    InitFailed,
    RecoveryStalled,
    Interrupted,
}

/// `TERM(o/c)`: outcome and cause of a terminal claim (C0.2, C4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terminal {
    /// Terminal outcome.
    pub outcome: TerminalOutcome,
    /// Cause code.
    pub code: ErrCode,
}

impl Terminal {
    /// Build a terminal.
    pub const fn new(outcome: TerminalOutcome, code: ErrCode) -> Self {
        Self { outcome, code }
    }
}

/// Performance-ledger outcome of `PC(...)` / `PGF(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PerfOutcome {
    Failed,
    Cancelled,
    NoSpeech,
    Completed,
}

/// Performance stage named by `PC(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PerfStage {
    CaptureFinalization,
    InferenceDecode,
}

/// Performance cause named by `PC(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PerfCause {
    AudioCaptureFailed,
    InternalEarlyExit,
}

/// `PC(...)`: `complete_dictation_performance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfComplete {
    /// Outcome.
    pub outcome: PerfOutcome,
    /// Stage.
    pub stage: PerfStage,
    /// Cause, when the site names one.
    pub cause: Option<PerfCause>,
    /// Whether the site passes the interrupted audio duration (R215.1, R216.1).
    pub with_audio_duration: bool,
}

/// `AX(a|s)`: when the delivery-target snapshot is captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxMode {
    /// At start (5593).
    Accept,
    /// At stop (5952/5974).
    Stop,
}

/// `TI(d|u)`: `take_interrupted_dictation` as disposal or as use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeMode {
    /// Disposal.
    Dispose,
    /// Use (the observation answer of `NeedObservation(TakeInterrupted)`).
    Use,
}

/// `PE(v)`: `NativeDictationPerformanceGuard::enter` variants (6189).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineEnter {
    /// The VAD-gated pipeline.
    Vad,
}

/// The effect vocabulary of appendix C0.2, one variant per legend code.
///
/// Payloads carry only what the legend names. Opaque shell objects (contexts,
/// AX snapshots, capture handles) are bound by the executor, never carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `ST→x`: `SetStatus(x)`. Always `In`. Carries the status write, the
    /// `dictation_state_changed` trace and `output_quieting::observe_transition`.
    SetStatus(Status),
    /// `ID+`: `AllocId` (`next_recording_id`, commits M2). `In`, first effect.
    AllocId,
    /// `EM(s)`: emit `recording-status-changed`.
    EmitStatus {
        /// Emitted status string.
        status: Status,
        /// Shell predicate that may skip it.
        cond: Cond,
    },
    /// `EV(k)`: typed event emit.
    EmitEvent(EventKind),
    /// `PRES(k)`: presentation trace, conditional on its paired emit.
    Presentation {
        /// Kind.
        kind: PresentationKind,
        /// Normally [`Cond::EmitOk`].
        cond: Cond,
    },
    /// `TERM(o/c)`: terminal claim; the claim happens when this executes (C4.5).
    Terminal(Terminal),
    /// `PA`: performance `abort`.
    PerfAbort,
    /// `PR`: `register_production` + `begin_ledger.register`.
    PerfRegister,
    /// `PB`: `spawn_native_dictation_performance_begin`.
    PerfBegin,
    /// `PC(...)`: `complete_dictation_performance`.
    PerfComplete(PerfComplete),
    /// `CX-`: `clear_active_context(id)`; writes only the live slot (M4).
    ClearContext(RecordingId),
    /// `CX+`: `set_active_context`; writes only the live slot (M4).
    SetContext(RecordingId),
    /// `KB(b)`: `keyboard::set_processing(b)`.
    KeyboardProcessing(bool),
    /// `MC`: `cancel_recording(id)` (`cancelled_id.fetch_max`); live only (M3).
    MarkCancelled(RecordingId),
    /// `TL`: `last_transcription_at = now`, read by the executor at execution.
    TouchLastTranscription,
    /// `AX(a|s)`: delivery-target snapshot; blocking.
    AxSnapshot(AxMode),
    /// `GA`: `gesture_claim.accepted` (5564).
    GestureAccepted,
    /// `PCC`: `audio::prepare::capture_claimed` (5565-5575).
    PrepareCaptureClaimed,
    /// `PH`: `PreviewHideOwner::claim`; atomics only.
    PreviewHideClaim,
    /// `TI(d|u)`: `take_interrupted_dictation(id)`.
    TakeInterrupted {
        /// The recording whose capture is taken.
        id: RecordingId,
        /// Disposal or use.
        mode: TakeMode,
    },
    /// `TS`: `hotkey_release::take_for` + `stop_gesture::take_for`.
    TakeStopStamps,
    /// `SD`: `StartSideTasks` attempt (VAD stream, partial ticker), self-gating.
    StartSideTasks,
    /// `PM`: `spawn_model_preparation`.
    ModelPreparation,
    /// `CS`: capture start (blocking 2 s, or awaited `spawn_blocking` ≤ 17 s
    /// when a preview was kept; always last of its phase).
    CaptureStart(RecordingId),
    /// `CP`: capture stop.
    CaptureStop(RecordingId),
    /// `CC`: capture cancel.
    CaptureCancel {
        /// Recording to cancel.
        id: RecordingId,
        /// Why.
        reason: AudioCancelReason,
    },
    /// `RS`: self-dispatched `tokio::spawn(stop_native_recording_for(Some(id), trigger))`.
    SelfStop {
        /// Recording to stop.
        id: RecordingId,
        /// Trigger.
        trigger: StopTrigger,
    },
    /// `PS`: `run_transcription_pipeline`; awaited, always last of its phase.
    PipelineSpawn(RecordingId),
    /// `REL`: drop of `gesture_claim` / `SpeculativeClaim` at command return.
    ReleaseClaims,
    /// `TRQ`: `pipeline.dictation_requested` trace, before the status write.
    TraceRequested,
    /// `OBS(k)`: shell observation read.
    Observe(crate::decision::ObsKind),
    /// `ACC`: `dictation_telemetry.accepted(id)` + `dictation_diagnostics.claim(id)`.
    AcceptTelemetry,
    /// `PCF(k)`: `performance.production_capture_failure`.
    ProductionCaptureFailure(Handle),
    /// `SPEC`: `speculative.is_flowing()/consume()`.
    SpeculativeConsume,
    /// `PE(v)`: performance guard enter.
    PipelineEnter(PipelineEnter),
    /// `PGF(o)`: performance guard finish.
    PipelineGuardFinish(PerfOutcome),
    /// `SDE`: `smart-destination-selected` emit under D.
    SmartDestinationEmit,
}

impl Effect {
    /// The legend code of C0.2 (`ST`, `ID+`, `EM`, ...), used by traces, shadow
    /// diffs and tests.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SetStatus(_) => "ST",
            Self::AllocId => "ID+",
            Self::EmitStatus { .. } => "EM",
            Self::EmitEvent(_) => "EV",
            Self::Presentation { .. } => "PRES",
            Self::Terminal(_) => "TERM",
            Self::PerfAbort => "PA",
            Self::PerfRegister => "PR",
            Self::PerfBegin => "PB",
            Self::PerfComplete(_) => "PC",
            Self::ClearContext(_) => "CX-",
            Self::SetContext(_) => "CX+",
            Self::KeyboardProcessing(_) => "KB",
            Self::MarkCancelled(_) => "MC",
            Self::TouchLastTranscription => "TL",
            Self::AxSnapshot(_) => "AX",
            Self::GestureAccepted => "GA",
            Self::PrepareCaptureClaimed => "PCC",
            Self::PreviewHideClaim => "PH",
            Self::TakeInterrupted { .. } => "TI",
            Self::TakeStopStamps => "TS",
            Self::StartSideTasks => "SD",
            Self::ModelPreparation => "PM",
            Self::CaptureStart(_) => "CS",
            Self::CaptureStop(_) => "CP",
            Self::CaptureCancel { .. } => "CC",
            Self::SelfStop { .. } => "RS",
            Self::PipelineSpawn(_) => "PS",
            Self::ReleaseClaims => "REL",
            Self::TraceRequested => "TRQ",
            Self::Observe(_) => "OBS",
            Self::AcceptTelemetry => "ACC",
            Self::ProductionCaptureFailure(_) => "PCF",
            Self::SpeculativeConsume => "SPEC",
            Self::PipelineEnter(_) => "PE",
            Self::PipelineGuardFinish(_) => "PGF",
            Self::SmartDestinationEmit => "SDE",
        }
    }

    /// `ST` is always `In` (C0.2).
    pub const fn is_set_status(&self) -> bool {
        matches!(self, Self::SetStatus(_))
    }
}

/// A defect in a decision's effect list (see [`validate_effects`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectListError {
    /// Placements decreased at this index (`Pre ≤ In ≤ PDT ≤ PT`, C0.1).
    PlacementDecreases {
        /// Index of the offending effect.
        index: usize,
    },
    /// An `ST` was placed outside `In` (C0.2).
    SetStatusNotIn {
        /// Index of the offending effect.
        index: usize,
    },
    /// A `Cond::EmitOk(i)` does not name an earlier effect of the list.
    DanglingPairedEmit {
        /// Index of the conditional effect.
        index: usize,
    },
}

/// Check the structural rules every effect list must satisfy: non-decreasing
/// placements (C0.1), `ST` only `In` (C0.2) and `EmitOk` naming an earlier effect.
///
/// One placement is off the chain by construction: at S7 T is released *inside*
/// D by the shell, between two `In` effects (R202.3). That is a shell action,
/// not an effect, so the list itself still satisfies these rules.
pub fn validate_effects(effects: &[Placed]) -> Result<(), EffectListError> {
    let mut last = Placement::Pre;
    for (index, placed) in effects.iter().enumerate() {
        if placed.at < last {
            return Err(EffectListError::PlacementDecreases { index });
        }
        last = placed.at;
        if placed.effect.is_set_status() && placed.at != Placement::In {
            return Err(EffectListError::SetStatusNotIn { index });
        }
        let cond = match &placed.effect {
            Effect::EmitStatus { cond, .. } | Effect::Presentation { cond, .. } => Some(*cond),
            _ => None,
        };
        if let Some(Cond::EmitOk(paired)) = cond {
            if usize::from(paired) >= index {
                return Err(EffectListError::DanglingPairedEmit { index });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Placement::*;

    fn st(at: Placement, s: Status) -> Placed {
        Placed::new(at, Effect::SetStatus(s))
    }

    #[test]
    fn placement_order_is_execution_order() {
        assert!(Pre < In && In < PostDT && PostDT < PostT);
        assert_eq!(
            [Pre, In, PostDT, PostT].map(Placement::label),
            ["Pre", "In", "PDT", "PT"]
        );
    }

    #[test]
    fn r101_15_shaped_list_is_valid() {
        let list = vec![
            Placed::new(In, Effect::AllocId),
            Placed::new(In, Effect::GestureAccepted),
            Placed::new(In, Effect::PrepareCaptureClaimed),
            Placed::new(In, Effect::TraceRequested),
            st(In, Status::Starting),
            Placed::new(In, Effect::AxSnapshot(AxMode::Accept)),
            Placed::new(PostDT, Effect::SpeculativeConsume),
            Placed::new(PostDT, Effect::EmitEvent(EventKind::GenStarted)),
            Placed::new(
                PostDT,
                Effect::EmitStatus {
                    status: Status::Starting,
                    cond: Cond::UnlessSpeculativeFlowing,
                },
            ),
            Placed::new(PostDT, Effect::PerfRegister),
            Placed::new(PostDT, Effect::CaptureStart(RecordingId(1))),
        ];
        assert_eq!(validate_effects(&list), Ok(()));
    }

    #[test]
    fn decreasing_placement_is_rejected() {
        let list = vec![
            Placed::new(PostDT, Effect::PerfAbort),
            Placed::new(In, Effect::KeyboardProcessing(false)),
        ];
        assert_eq!(
            validate_effects(&list),
            Err(EffectListError::PlacementDecreases { index: 1 })
        );
    }

    #[test]
    fn set_status_outside_in_is_rejected() {
        for at in [Pre, PostDT, PostT] {
            assert_eq!(
                validate_effects(&[st(at, Status::Idle)]),
                Err(EffectListError::SetStatusNotIn { index: 0 })
            );
        }
    }

    #[test]
    fn presentation_must_pair_with_an_earlier_emit() {
        let pres = |cond| {
            Placed::new(
                PostDT,
                Effect::Presentation {
                    kind: PresentationKind::CleanupInProgress,
                    cond,
                },
            )
        };
        let emit = Placed::new(
            PostDT,
            Effect::EmitStatus {
                status: Status::Recovering,
                cond: Cond::Always,
            },
        );
        assert_eq!(
            validate_effects(&[emit.clone(), pres(Cond::EmitOk(0))]),
            Ok(())
        );
        assert_eq!(
            validate_effects(&[pres(Cond::EmitOk(0)), emit]),
            Err(EffectListError::DanglingPairedEmit { index: 0 })
        );
    }

    #[test]
    fn legend_codes_are_distinct_per_variant_family() {
        let samples = [
            Effect::SetStatus(Status::Idle),
            Effect::AllocId,
            Effect::EmitStatus {
                status: Status::Idle,
                cond: Cond::Always,
            },
            Effect::EmitEvent(EventKind::Cancelled),
            Effect::Terminal(Terminal::new(TerminalOutcome::Success, ErrCode::None)),
            Effect::PerfAbort,
            Effect::PerfRegister,
            Effect::PerfBegin,
            Effect::ClearContext(RecordingId(1)),
            Effect::SetContext(RecordingId(1)),
            Effect::KeyboardProcessing(true),
            Effect::MarkCancelled(RecordingId(1)),
            Effect::TouchLastTranscription,
            Effect::AxSnapshot(AxMode::Stop),
            Effect::GestureAccepted,
            Effect::PrepareCaptureClaimed,
            Effect::PreviewHideClaim,
            Effect::TakeStopStamps,
            Effect::StartSideTasks,
            Effect::ModelPreparation,
            Effect::CaptureStart(RecordingId(1)),
            Effect::CaptureStop(RecordingId(1)),
            Effect::PipelineSpawn(RecordingId(1)),
            Effect::ReleaseClaims,
            Effect::TraceRequested,
            Effect::AcceptTelemetry,
            Effect::SpeculativeConsume,
            Effect::SmartDestinationEmit,
        ];
        let mut codes: Vec<_> = samples.iter().map(Effect::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total);
    }
}
