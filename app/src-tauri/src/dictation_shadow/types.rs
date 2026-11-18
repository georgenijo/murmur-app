//! The messages the live path sends and the vocabulary they are made of
//! (appendix C8.2).
//!
//! Everything here is a plain value the live path already holds: ids, enums and
//! effect kinds from the core. No transcript, audio, clipboard or selection
//! content exists in any of these types (CLAUDE.md privacy rule), so nothing a
//! message carries can reach a log or the summary file.

use murmur_dictation_core::decision::{ContKind, ObsKind, OpStatus};
use murmur_dictation_core::effects::{Effect, Placement};
use murmur_dictation_core::ids::{Now, OperationId, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::replies::{Reply, ReplyType};
use murmur_dictation_core::state::LiveObs;
use murmur_dictation_core::status::Status;

/// Names a flight-local recorder. Drawn from the recorder's own counter, created
/// **before** `run(Pre)`, and never equal to a header's `seq` (the counters live in
/// different ranges, see `transport::REC_BASE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct RecorderId(pub u64);

/// A shell predicate that may skip a conditional effect (C0.2 `cond: PredId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PredId {
    /// `P-spec-flowing`: `EM(starting)` is skipped when a speculative capture is
    /// already flowing.
    SpecFlowing,
    /// `P-emit-ok(i)`: runs only when the paired emit (effect `i` of the decision)
    /// returned `Ok`.
    EmitOk(u8),
}

/// A shell-conditional effect as the live path evaluated it: the predicate and
/// whether the effect was taken. A taken conditional is also an effect; a skipped
/// one is not (C8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CondWitness {
    pub pred: PredId,
    pub taken: bool,
}

/// What an effect returned when it ran: the part of a result a comparison can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectResult {
    /// Ran; nothing to report.
    Ran,
    /// An emit (`EM`, `EV`, `SDE`): whether the `emit` returned `Ok`. This is what
    /// `P-emit-ok` reads.
    Emitted { ok: bool },
    /// A terminal claim (`TERM`): whether this claim won the ledger (decision
    /// D-S2-9: the real result, never assumed).
    Claimed { won: bool },
}

/// An effect the live path actually executed, with its result. The placement is
/// the list it travels in (`pre_effects`, `in_d_effects`) or the `Witness`
/// message's own `placement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WitnessedEffect {
    pub effect: Effect,
    pub result: EffectResult,
}

impl WitnessedEffect {
    pub(crate) fn ran(effect: Effect) -> Self {
        Self {
            effect,
            result: EffectResult::Ran,
        }
    }

    pub(crate) fn emitted(effect: Effect, ok: bool) -> Self {
        Self {
            effect,
            result: EffectResult::Emitted { ok },
        }
    }

    pub(crate) fn claimed(effect: Effect, won: bool) -> Self {
        Self {
            effect,
            result: EffectResult::Claimed { won },
        }
    }
}

/// A live field an effect writes that the core does not store (C8.3 CP3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MirrorField {
    /// M3: `AppState.cancelled_id`, written by `MC`.
    CancelledThrough,
    /// M4: `AppState.active_context`'s id, written by `CX±`.
    ActiveContext,
    /// `keyboard::IS_PROCESSING`, written by `KB(b)`.
    KeyboardProcessing,
    /// The partial ticker registration, written by `SD`. Recorded for the summary
    /// and the soak; no decided commit exists for it, so it is never compared.
    TickerRegistered,
}

/// A live field value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MirrorValue {
    /// The `cancelled_id` high-water mark.
    Id(RecordingId),
    /// The id of the installed context, if any.
    Slot(Option<RecordingId>),
    /// A flag.
    Flag(bool),
}

/// A live M-field value before and after an effect wrote it, read inside the
/// adapter's existing lock section (no new lock, C8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MirrorWitness {
    pub field: MirrorField,
    pub before: MirrorValue,
    pub after: MirrorValue,
}

/// Persisted M1/M2 as the live path read them, plus the M3/M4 observations
/// (`obs`, C1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Projection {
    pub status: Status,
    pub current_id: RecordingId,
    pub obs: LiveObs,
}

/// What the shell did to the history for this decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveOpKind {
    /// `reg`: the shell drew `op` from `NEXT_OP` and registered the continuation.
    Registered,
    /// `ret` / `ret_if_matches`: the shell retired `op`.
    Retired,
}

/// An op id the live shell drew from `NEXT_OP` (or retired) for a decision. The
/// shadow **replays** these ids and never draws its own (C8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveOp {
    pub kind: LiveOpKind,
    pub cont: ContKind,
    pub recording_id: RecordingId,
    pub op: OperationId,
}

/// A value the live path read for this decision (C8.2 `observed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservedFact {
    /// `speculative_flowing`: what `P-spec-flowing` reads.
    SpeculativeFlowing(bool),
    /// The live path answered this `NeedObservation` (its answer is a field of the
    /// header's `input`).
    Answered(ObsKind),
    /// The validated status of a consumed op token.
    Op { op: OperationId, status: OpStatus },
}

/// The kind of a reply: the JSON `type` string, not the body (C8.3 `diff_kind 4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyKind {
    /// `Ok(())`.
    Ok,
    /// `Err(..)`.
    Err,
    /// A typed reply, by its `type` literal.
    Typed(ReplyType),
    /// `{"type":"transcription",..}`.
    Transcription,
}

impl ReplyKind {
    /// The kind of a decided reply.
    pub(crate) fn of(reply: &Reply) -> Self {
        match reply {
            Reply::Ok => Self::Ok,
            Reply::Err(_) => Self::Err,
            Reply::Typed { ty, .. } => Self::Typed(*ty),
            Reply::Transcription { .. } => Self::Transcription,
        }
    }
}

/// The header of one decision: assembled under D, submitted right after D is
/// released and before any blocking effect (C8.2).
#[derive(Clone, Debug)]
pub(crate) struct ShadowHeader {
    /// Taken at the existing decision boundary; **is** D order. Not the recorder id.
    pub seq: u64,
    /// The flight's recorder, created before `run(Pre)`.
    pub rec: RecorderId,
    /// The C0.3 site number the live path took (`2041` for 204a).
    pub site_id: u16,
    /// The C3 row ordinal the live path took (`RowId::ordinal`).
    pub row: u16,
    /// Persisted M1/M2 at section entry plus the M3/M4 observations.
    pub pre: Projection,
    /// The typed input, observations answered.
    pub input: Input,
    /// Every value the live path read for this decision.
    pub observed: Vec<ObservedFact>,
    /// Op ids the shell drew and registered or retired.
    pub live_ops: Vec<LiveOp>,
    /// The shell's stamp; excluded from comparison.
    pub now: Now,
    /// Effects that ran at `Pre`, in order, with their results.
    pub pre_effects: Vec<WitnessedEffect>,
    /// Shell-conditional `Pre` effects.
    pub pre_conditionals: Vec<CondWitness>,
    /// Effects that ran at `In`, in order.
    pub in_d_effects: Vec<WitnessedEffect>,
    /// Shell-conditional effects evaluated under D.
    pub in_d_conditionals: Vec<CondWitness>,
    /// M4 before/after each `Pre:CX-`.
    pub pre_mirrors: Vec<MirrorWitness>,
    /// M4 before/after each `In:CX±`.
    pub in_d_mirrors: Vec<MirrorWitness>,
    /// Persisted M1/M2 as the `In` effects committed them, read under D just
    /// before release.
    pub live_post_d: Projection,
}

/// One message of the single bounded channel.
#[derive(Debug)]
pub(crate) enum ShadowMsg {
    Header(Box<ShadowHeader>),
    /// An effect that ran after D.
    Witness {
        rec: RecorderId,
        /// The effect's index in the **full** decided list, kept even when
        /// `after_pre` / `after_prefix` dropped a prefix before the executor ran it.
        idx: u16,
        placement: Placement,
        effect: WitnessedEffect,
    },
    /// A shell-conditional effect after D.
    Cond {
        rec: RecorderId,
        pred: PredId,
        taken: bool,
    },
    /// A live M-field written at effect time after D.
    Mirror {
        rec: RecorderId,
        field: MirrorField,
        value: MirrorValue,
    },
    Reply {
        rec: RecorderId,
        kind: ReplyKind,
    },
    /// `completed`: the flight ran to its end.
    Close {
        rec: RecorderId,
        completed: bool,
    },
    /// The flight was dropped after `run(Pre)` began and before D entry: no
    /// decision, no `seq`. Reported, not lost.
    PreAbandoned {
        rec: RecorderId,
        pre_effects: Vec<WitnessedEffect>,
        pre_conditionals: Vec<CondWitness>,
    },
    /// A header that will never arrive (its ticket was dropped).
    Lost {
        seq: u64,
    },
}

/// `diff_kind` of the mismatch event (C8.3), in the documented order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub(crate) enum DiffKind {
    /// The multiset of effects differs (including a decided effect never witnessed).
    EffectSet = 1,
    /// Same set, different order within a placement.
    EffectOrder = 2,
    /// An effect ran in a different placement class than decided.
    Placement = 3,
    /// The reply kind differs.
    Reply = 4,
    /// Persisted M1/M2, or an M3/M4 mirror, differs from the decided commit.
    PostState = 5,
    /// The registered continuation differs in kind, recording id or op id.
    Continuation = 6,
    /// The decision asked for an observation the live path did not record.
    Observation = 7,
    /// The shadow's own decide panicked.
    ShadowPanic = 8,
    /// A shell-conditional's predicate id or taken/skipped differs.
    Conditional = 9,
}

impl DiffKind {
    /// The numeric `diff_kind` field.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }
}
