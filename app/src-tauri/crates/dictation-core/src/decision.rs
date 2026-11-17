//! Decision result types (appendix C2.1, C0.3, C1.4.1).
//!
//! Stage 1a defines the shapes only. The `decide_*` functions arrive with the
//! Stage 2a family PRs; they will return a [`Decision`].

use crate::admission::RefusalCode;
use crate::effects::{validate_effects, Effect, EffectListError, Placed};
use crate::ids::{OperationId, RecordingId};
use crate::replies::Reply;
use crate::status::Status;

/// A C3 row id such as `R101.15`; coverage counting and shadow diffs key on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RowId(pub &'static str);

/// The shadow `site_id` of a decision entry (C0.3). `202n`, `204a` and `204b`
/// are encoded as 2021, 2041 and 2042.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum SiteId {
    StartRequest,
    StartCaptureResult,
    StartContext,
    ReadyPublish,
    StopRequest,
    StopContext,
    StopContextNoLock,
    StopCaptureResult,
    GuardFinish,
    PipelineIdleEmit,
    CancelRequest,
    CancelCaptureResult,
    SupReady,
    SupStillConnecting,
    SupRecovering,
    SupCleanupFinished,
    SupIdle,
    SupAccepted,
    SupInitFailed,
    SupStalled,
    SupInterrupted,
    SupInterruptedShortIdle,
    ProcessAudioStart,
    ProcessAudioFinish,
}

impl SiteId {
    /// Every site, in C0.3 order.
    pub const ALL: [SiteId; 24] = [
        Self::StartRequest,
        Self::StartCaptureResult,
        Self::StartContext,
        Self::ReadyPublish,
        Self::StopRequest,
        Self::StopContext,
        Self::StopContextNoLock,
        Self::StopCaptureResult,
        Self::GuardFinish,
        Self::PipelineIdleEmit,
        Self::CancelRequest,
        Self::CancelCaptureResult,
        Self::SupReady,
        Self::SupStillConnecting,
        Self::SupRecovering,
        Self::SupCleanupFinished,
        Self::SupIdle,
        Self::SupAccepted,
        Self::SupInitFailed,
        Self::SupStalled,
        Self::SupInterrupted,
        Self::SupInterruptedShortIdle,
        Self::ProcessAudioStart,
        Self::ProcessAudioFinish,
    ];

    /// The `u16` `site_id` of the shadow event (C0.3 "Shadow encoding").
    pub const fn code(self) -> u16 {
        match self {
            Self::StartRequest => 101,
            Self::StartCaptureResult => 102,
            Self::StartContext => 103,
            Self::ReadyPublish => 104,
            Self::StopRequest => 201,
            Self::StopContext => 202,
            Self::StopContextNoLock => 2021,
            Self::StopCaptureResult => 203,
            Self::GuardFinish => 2041,
            Self::PipelineIdleEmit => 2042,
            Self::CancelRequest => 205,
            Self::CancelCaptureResult => 206,
            Self::SupReady => 207,
            Self::SupStillConnecting => 208,
            Self::SupRecovering => 209,
            Self::SupCleanupFinished => 210,
            Self::SupIdle => 211,
            Self::SupAccepted => 212,
            Self::SupInitFailed => 213,
            Self::SupStalled => 214,
            Self::SupInterrupted => 215,
            Self::SupInterruptedShortIdle => 216,
            Self::ProcessAudioStart => 301,
            Self::ProcessAudioFinish => 302,
        }
    }
}

/// A read a decide function may ask the shell to perform before deciding
/// (C2.2). Returned *before* any mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObsKind {
    /// `resolve_smart_auto(request)` (5546).
    SmartAutoSelection,
    /// `take_interrupted_dictation(rid)` (5939), destructive.
    TakeInterrupted {
        /// Recording whose preserved capture is taken.
        id: RecordingId,
    },
    /// `active_context(rid).is_some()` (4947), a leaf-mutex read today's `Ready`
    /// arm performs only after the stale check passed (decision D-S1-2: the
    /// decide function asks for it instead of the shell reading it eagerly).
    ContextPresence {
        /// Recording whose installed context is looked up.
        id: RecordingId,
    },
}

/// Why a stale input (stale recording id, never an op status) takes the site's
/// own disposal path (C1.4.1 rule 3, C3.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleKind {
    /// The input's recording id is not the current one.
    StaleRecording,
}

/// Inputs that cannot occur today (C3.0). Fail-closed: no mutation, no emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidKind {
    /// Nested decide or `run` (C4.2): `Invalid(Reentrant)` with no effects.
    Reentrant,
    /// An `X`-marked row's literal behaviour, kept so the core is total.
    Impossible(RowId),
}

/// A pure refusal (no mutation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A start-admission refusal (`admit()`'s code).
    Admission(RefusalCode),
    /// Stop while Idle (R201.3): `not_recording`.
    NotRecording,
    /// Stop while Processing (R201.2): `already_processing`.
    AlreadyProcessing,
    /// Stop while Recovering with the capture already taken (R201.6).
    AudioRecovering,
}

/// The decision's disposition (C2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Decision taken, effects returned.
    Applied,
    /// No change; the caller's straight-line code proceeds.
    Continue,
    /// Pure refusal; no mutation.
    Refused(Refusal),
    /// Stale recording id: only the site's disposal/bookkeeping effects.
    Stale(StaleKind),
    /// Impossible input; fail-closed.
    Invalid(InvalidKind),
    /// Returned before any mutation; the shell fills the observation and calls again.
    NeedObservation(ObsKind),
}

/// What a continuation continues (the `K:` column of C3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContKind {
    /// `K:CaptureStart` (R101.15).
    CaptureStart,
    /// `K:ResolveContext` (R102.1).
    ResolveContext,
    /// `K:ReadyPublish` (R103.1).
    ReadyPublish,
    /// `K:CaptureCancel` (R201.4, R205.4).
    CaptureCancel,
    /// `K:StopContext` (R201.7, R201.8).
    StopContext,
    /// `K:CaptureStop` (R202.1).
    CaptureStop,
    /// `K:Pipeline` (R203.9, R301.5).
    Pipeline,
    /// `K:AutoStop` (R215.2).
    AutoStop,
}

/// What a decision returns for the shell to register (C1.4.1 rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContinuationSpec {
    /// Recording the flight belongs to.
    pub recording_id: RecordingId,
    /// Flight kind.
    pub kind: ContKind,
}

/// The shell-held token of a registered continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Continuation {
    /// Recording the flight belongs to.
    pub recording_id: RecordingId,
    /// Allocated by the shell from `NEXT_OP`.
    pub op: OperationId,
    /// Flight kind.
    pub kind: ContKind,
}

/// A validated token status, an observation the shell fills. No decide
/// function branches on it (C1.4.1 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpStatus {
    /// `pending_ops[op]` exists with the same recording id and kind.
    Outstanding,
    /// Absent, retired, forged or wrong kind.
    Unknown,
}

/// A decision (C2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Disposition.
    pub verdict: Verdict,
    /// `Some` iff an `ST` effect is present.
    pub transition: Option<(Status, Status)>,
    /// Ordered effects.
    pub effects: Vec<Placed>,
    /// Continuation to register, if the decision continues a flight.
    pub continuation: Option<ContinuationSpec>,
    /// Typed reply.
    pub reply: Option<Reply>,
    /// The C3 row that produced this decision.
    pub row: RowId,
}

/// A structural defect in a [`Decision`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionError {
    /// The effect list breaks a C0.1/C0.2 rule.
    Effects(EffectListError),
    /// `transition` is `Some` but no `ST` is present, or the reverse.
    TransitionMismatch,
    /// The `ST` target differs from `transition.1`.
    TransitionTargetMismatch,
}

impl Decision {
    /// A decision that holds only `pre`, the input-derived effects that run
    /// before the D section (C2.1 `pre_effects()`). The shell runs it through its
    /// own executor **before** it acquires D and decides.
    pub fn pre_only(pre: Vec<Placed>, row: RowId) -> Self {
        Self {
            verdict: Verdict::Applied,
            transition: None,
            effects: pre,
            continuation: None,
            reply: None,
            row,
        }
    }

    /// This decision without its leading `Pre` effects, which the shell already
    /// ran from `pre_effects()` before the decide (`decide_*` returns them
    /// prepended, for tracing and shadow, C2.1).
    #[must_use]
    pub fn after_pre(mut self) -> Self {
        let pre = self
            .effects
            .iter()
            .take_while(|placed| placed.at == crate::effects::Placement::Pre)
            .count();
        self.effects.drain(..pre);
        self
    }

    /// Check the structural rules of C2.1: valid effect list, and `transition`
    /// is `Some` iff an `ST` effect is present (with a matching target).
    pub fn validate(&self) -> Result<(), DecisionError> {
        validate_effects(&self.effects).map_err(DecisionError::Effects)?;
        let set_status = self.effects.iter().find_map(|p| match p.effect {
            Effect::SetStatus(to) => Some(to),
            _ => None,
        });
        match (self.transition, set_status) {
            (None, None) => Ok(()),
            (Some((_, to)), Some(st)) if to == st => Ok(()),
            (Some(_), Some(_)) => Err(DecisionError::TransitionTargetMismatch),
            _ => Err(DecisionError::TransitionMismatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::Placement;

    fn decision(transition: Option<(Status, Status)>, effects: Vec<Placed>) -> Decision {
        Decision {
            verdict: Verdict::Applied,
            transition,
            effects,
            continuation: None,
            reply: None,
            row: RowId("R000.0"),
        }
    }

    #[test]
    fn site_codes_follow_c0_3_and_are_unique() {
        let mut codes: Vec<u16> = SiteId::ALL.iter().map(|s| s.code()).collect();
        assert_eq!(SiteId::StopContextNoLock.code(), 2021);
        assert_eq!(SiteId::GuardFinish.code(), 2041);
        assert_eq!(SiteId::PipelineIdleEmit.code(), 2042);
        assert_eq!(SiteId::StopCaptureResult.code(), 203);
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total);
    }

    #[test]
    fn transition_is_some_iff_st_present() {
        let st = Placed::new(Placement::In, Effect::SetStatus(Status::Idle));
        assert_eq!(decision(None, vec![]).validate(), Ok(()));
        assert_eq!(
            decision(Some((Status::Processing, Status::Idle)), vec![st.clone()]).validate(),
            Ok(())
        );
        assert_eq!(
            decision(None, vec![st.clone()]).validate(),
            Err(DecisionError::TransitionMismatch)
        );
        assert_eq!(
            decision(Some((Status::Processing, Status::Idle)), vec![]).validate(),
            Err(DecisionError::TransitionMismatch)
        );
        assert_eq!(
            decision(Some((Status::Processing, Status::Recording)), vec![st]).validate(),
            Err(DecisionError::TransitionTargetMismatch)
        );
    }

    #[test]
    fn effect_list_errors_surface() {
        let bad = vec![
            Placed::new(Placement::PostT, Effect::PerfAbort),
            Placed::new(Placement::Pre, Effect::PerfAbort),
        ];
        assert_eq!(
            decision(None, bad).validate(),
            Err(DecisionError::Effects(
                EffectListError::PlacementDecreases { index: 1 }
            ))
        );
    }
}
