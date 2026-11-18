//! The messages the live path sends and the vocabulary they are made of
//! (appendix C8.2).
//!
//! Everything here is a plain value the live path already holds: ids, enums and
//! effect kinds from the core. No transcript, audio, clipboard or selection
//! content exists in any of these types (CLAUDE.md privacy rule), so nothing a
//! message carries can reach a log or the summary file.
//!
//! **No type here owns heap memory.** A header's lists are [`Inline`] arrays with a
//! fixed capacity, so assembling a header and queueing a message never allocates
//! (D-S2c-1 a): the queue stores messages by value, and a list that would overflow
//! is counted and marks its header as not comparable instead of growing.

use murmur_dictation_core::decision::ContKind;
use murmur_dictation_core::effects::{Effect, Placement};
use murmur_dictation_core::ids::{Now, OperationId, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::replies::{Reply, ReplyType};
use murmur_dictation_core::state::LiveObs;
use murmur_dictation_core::status::Status;

/// Inline `Pre` capacity: the largest C3 `Pre` list is 4 today (R216.1).
pub(crate) const PRE_CAPACITY: usize = 8;
/// Inline `In` capacity: the largest C3 `In` list is 6 today (R101.15).
pub(crate) const IN_CAPACITY: usize = 16;
/// Shell-conditional witnesses per placement group.
pub(crate) const COND_CAPACITY: usize = 8;
/// Live-field mirrors per placement group.
pub(crate) const MIRROR_CAPACITY: usize = 8;
/// History operations a header carries (H1 and H2 registrations and retirements).
pub(crate) const OPS_CAPACITY: usize = 8;
/// Observed facts per header.
pub(crate) const OBSERVED_CAPACITY: usize = 4;
/// Ticker registrations a header carries (`SD` under D).
pub(crate) const TICKER_CAPACITY: usize = 2;

/// A fixed-capacity list stored inline: no heap, `push` fails when full.
#[derive(Clone, Debug)]
pub(crate) struct Inline<T, const N: usize> {
    items: [Option<T>; N],
    len: usize,
}

impl<T, const N: usize> Default for Inline<T, N> {
    fn default() -> Self {
        Self {
            items: std::array::from_fn(|_| None),
            len: 0,
        }
    }
}

impl<T, const N: usize> Inline<T, N> {
    /// Append `value`; `false` (and nothing stored) when the list is full.
    pub(crate) fn push(&mut self, value: T) -> bool {
        match self.items.get_mut(self.len) {
            Some(slot) => {
                *slot = Some(value);
                self.len += 1;
                true
            }
            None => false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.items[..self.len].iter().flatten()
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items[..self.len].iter_mut().flatten()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.items[..self.len].get(index).and_then(Option::as_ref)
    }

    /// The items as an owned vector (the comparison side may allocate).
    pub(crate) fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.iter().cloned().collect()
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Keep the items `keep` accepts (tests damage a header with it).
    #[cfg(test)]
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        let kept: Vec<T> = std::mem::take(self)
            .items
            .into_iter()
            .flatten()
            .filter(|item| keep(item))
            .collect();
        for item in kept {
            self.push(item);
        }
    }

    #[cfg(test)]
    pub(crate) fn remove(&mut self, index: usize) -> T {
        let mut all: Vec<T> = std::mem::take(self).items.into_iter().flatten().collect();
        let removed = all.remove(index);
        for item in all {
            self.push(item);
        }
        removed
    }

    #[cfg(test)]
    pub(crate) fn swap(&mut self, a: usize, b: usize) {
        self.items.swap(a, b);
    }

    #[cfg(test)]
    pub(crate) fn from_slice(values: &[T]) -> Self
    where
        T: Clone,
    {
        let mut list = Self::default();
        for value in values {
            assert!(list.push(value.clone()), "capacity {N}");
        }
        list
    }
}

impl<T: PartialEq, const N: usize> PartialEq for Inline<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

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
/// adapter's existing lock section (no new lock, C8.2). The *before* value is what
/// makes a stale-safe write checkable: `CX-(id)` must clear a slot holding `id` and
/// must leave a slot holding another recording's id alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MirrorWitness {
    pub field: MirrorField,
    pub before: MirrorValue,
    pub after: MirrorValue,
}

/// The inputs and the result of one ticker registration attempt (`SD`, query 404:
/// `id > 0`, the id is current, the status is `Recording`, the id is not cancelled
/// and `note_ticker_started` saw no later id). Captured where the live gate ran: under
/// the caller's D for `In:SD`, under the wrapper's own D for `PostDT:SD`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TickerGate {
    /// The recording the attempt was for.
    pub id: RecordingId,
    /// Live M1 status the gate read.
    pub status: Status,
    /// Live M2 current id the gate read.
    pub current_id: RecordingId,
    /// Live M3 `cancelled_id` the gate read.
    pub cancelled_through: RecordingId,
    /// The ticker high-water mark before `fetch_max` (`note_ticker_started`).
    pub ticker_before: RecordingId,
    /// Whether the live path registered a ticker.
    pub registered: bool,
}

impl TickerGate {
    /// What the gate must decide from its own inputs.
    pub(crate) fn expected(&self) -> bool {
        self.id.0 > 0
            && self.status == Status::Recording
            && self.id == self.current_id
            && self.cancelled_through < self.id
            && self.ticker_before <= self.id
    }
}

/// Why a `StartSideTasks` that ran spawned no ticker and so produced no gate: the
/// spawner returned before it asked the gate (D-S2c-6). Closed and numeric: no
/// content, no id. The reason is recorded per row and never compared with the core,
/// which does not model the spawner's preconditions (they are live-only
/// observations).
///
/// A spawner that reaches the gate and is refused is **not** a skip: that attempt is
/// a [`TickerGate`] with `registered: false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TickerSkipReason {
    /// No recording context was recorded for the recording.
    NoContext = 1,
    /// The model has no live preview (neither Whisper nor Core ML).
    NoLivePreview = 2,
}

impl TickerSkipReason {
    /// Every reason, by code: the summary has one counter per entry.
    pub(crate) const ALL: [Self; 2] = [Self::NoContext, Self::NoLivePreview];

    /// The wire and summary code: 1-based, so `0` is never a reason.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }

    /// Index into a per-reason counter array.
    pub(crate) const fn index(self) -> usize {
        self as usize - 1
    }
}

/// Persisted M1/M2 as the live path read them, plus the M3/M4 observations
/// (`obs`, C1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Projection {
    pub status: Status,
    pub current_id: RecordingId,
    pub obs: LiveObs,
}

/// Which history table an operation touched. H1 is `pending_ops` (continuations),
/// H2 is `pipelines` (the registered pipeline of a recording).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryTable {
    H1,
    H2,
}

/// What the shell did to a history table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryAction {
    /// `reg` / `reg_pipeline`: the shell drew `op` from `NEXT_OP` and registered it.
    Registered,
    /// `ret` / `ret_if_matches` / an owner's `Drop`: the shell retired `op`;
    /// `matched` is whether the table held it.
    Retired { matched: bool },
}

/// One history operation of a decision. The shadow **replays** these ids and never
/// draws its own (C8.1), and it copies no history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveOp {
    pub table: HistoryTable,
    pub action: HistoryAction,
    pub cont: ContKind,
    pub recording_id: RecordingId,
    pub op: OperationId,
}

/// A value the live path read for this decision (C8.2 `observed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservedFact {
    /// `speculative_flowing`: what `P-spec-flowing` reads.
    SpeculativeFlowing(bool),
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
/// released and before any blocking effect (C8.2). Owns no heap memory.
#[derive(Clone, Debug)]
pub(crate) struct ShadowHeader {
    /// Taken at the existing decision boundary; **is** D order. Not the recorder id.
    pub seq: u64,
    /// The transport's `lost` count when the ticket was taken, set by `submit`. After
    /// a reset the comparator admits only a header whose epoch is at least the count
    /// it acknowledged: an older flight may have lost a message the reset could not
    /// see (Sol round 5, major 1).
    pub epoch: u64,
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
    pub observed: Inline<ObservedFact, OBSERVED_CAPACITY>,
    /// The H1 / H2 operations of this decision before the header was sent.
    pub live_ops: Inline<LiveOp, OPS_CAPACITY>,
    /// The shell's stamp; excluded from comparison.
    pub now: Now,
    /// Effects that ran at `Pre`, in order, with their results.
    pub pre_effects: Inline<WitnessedEffect, PRE_CAPACITY>,
    /// Shell-conditional `Pre` effects.
    pub pre_conditionals: Inline<CondWitness, COND_CAPACITY>,
    /// Effects that ran at `In`, in order.
    pub in_d_effects: Inline<WitnessedEffect, IN_CAPACITY>,
    /// Shell-conditional effects evaluated under D.
    pub in_d_conditionals: Inline<CondWitness, COND_CAPACITY>,
    /// The live fields each `Pre` effect wrote (`CX-`, `MC`, `KB`), before and after.
    pub pre_mirrors: Inline<MirrorWitness, MIRROR_CAPACITY>,
    /// The live fields each `In` effect wrote, before and after.
    pub in_d_mirrors: Inline<MirrorWitness, MIRROR_CAPACITY>,
    /// Ticker registrations attempted by an `In:SD`.
    pub in_d_tickers: Inline<TickerGate, TICKER_CAPACITY>,
    /// Persisted M1/M2 as the `In` effects committed them, read under D just
    /// before release.
    pub live_post_d: Projection,
    /// A capture list overflowed while this header was assembled: it is incomplete
    /// and is counted, never compared.
    pub overflowed: bool,
}

impl ShadowHeader {
    /// A header for `seq` with every list empty; the live path fills it in.
    pub(crate) fn new(
        seq: u64,
        rec: RecorderId,
        site_id: u16,
        row: u16,
        pre: Projection,
        input: Input,
        now: Now,
    ) -> Self {
        Self {
            seq,
            epoch: 0,
            rec,
            site_id,
            row,
            pre,
            input,
            observed: Inline::default(),
            live_ops: Inline::default(),
            now,
            pre_effects: Inline::default(),
            pre_conditionals: Inline::default(),
            in_d_effects: Inline::default(),
            in_d_conditionals: Inline::default(),
            pre_mirrors: Inline::default(),
            in_d_mirrors: Inline::default(),
            in_d_tickers: Inline::default(),
            live_post_d: pre,
            overflowed: false,
        }
    }

    fn note(&mut self, stored: bool) {
        self.overflowed |= !stored;
    }

    pub(crate) fn add_observed(&mut self, fact: ObservedFact) {
        let stored = self.observed.push(fact);
        self.note(stored);
    }

    pub(crate) fn add_op(&mut self, op: LiveOp) {
        let stored = self.live_ops.push(op);
        self.note(stored);
    }

    pub(crate) fn add_in_effect(&mut self, effect: WitnessedEffect) {
        let stored = self.in_d_effects.push(effect);
        self.note(stored);
    }

    pub(crate) fn add_in_conditional(&mut self, witness: CondWitness) {
        let stored = self.in_d_conditionals.push(witness);
        self.note(stored);
    }

    pub(crate) fn add_in_mirror(&mut self, mirror: MirrorWitness) {
        let stored = self.in_d_mirrors.push(mirror);
        self.note(stored);
    }

    pub(crate) fn add_in_ticker(&mut self, gate: TickerGate) {
        let stored = self.in_d_tickers.push(gate);
        self.note(stored);
    }

    /// Move the recorder's buffered `Pre` witnesses into the header.
    pub(crate) fn adopt_pre(&mut self, pre: super::transport::PreCapture) {
        self.pre_effects = pre.effects;
        self.pre_conditionals = pre.conditionals;
        self.pre_mirrors = pre.mirrors;
        self.overflowed |= pre.overflowed;
    }
}

/// One message of the single bounded queue, stored by value.
#[derive(Clone, Debug)]
// The header is the large variant on purpose: the queue stores messages by value in
// preallocated slots, so nothing is boxed or allocated on submit (D-S2c-1 a).
#[allow(clippy::large_enum_variant)]
pub(crate) enum ShadowMsg {
    Header(ShadowHeader),
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
    /// A live M-field written at effect time after D, with its before and after.
    Mirror {
        rec: RecorderId,
        mirror: MirrorWitness,
    },
    /// A ticker registration attempt after D (`PostDT:SD`).
    Ticker {
        rec: RecorderId,
        gate: TickerGate,
    },
    /// A `PostDT:SD` whose spawner returned before the gate (D-S2c-6): the
    /// alternative to a [`ShadowMsg::Ticker`], never both.
    TickerSkipped {
        rec: RecorderId,
        /// The recording the spawner was asked for, checked against the decision's
        /// target exactly as a gate's id is.
        target: RecordingId,
        reason: TickerSkipReason,
    },
    /// An H1 / H2 operation after D (an owner's `Drop`, a deferred retirement).
    History {
        rec: RecorderId,
        op: LiveOp,
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
    },
}

impl ShadowMsg {
    /// The recorder a message belongs to.
    pub(crate) fn rec(&self) -> RecorderId {
        match self {
            Self::Header(header) => header.rec,
            Self::Witness { rec, .. }
            | Self::Cond { rec, .. }
            | Self::Mirror { rec, .. }
            | Self::Ticker { rec, .. }
            | Self::TickerSkipped { rec, .. }
            | Self::History { rec, .. }
            | Self::Reply { rec, .. }
            | Self::Close { rec, .. }
            | Self::PreAbandoned { rec } => *rec,
        }
    }
}

/// `diff_kind` of the mismatch event (C8.3), in the documented order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub(crate) enum DiffKind {
    /// The multiset of effects differs (including a decided effect, conditional or
    /// mirror witness never seen, or seen twice).
    EffectSet = 1,
    /// Same set, different order within a placement.
    EffectOrder = 2,
    /// An effect ran in a different placement class than decided.
    Placement = 3,
    /// The reply kind differs.
    Reply = 4,
    /// Persisted M1/M2, or an M3/M4 mirror, differs from the decided commit.
    PostState = 5,
    /// The registered continuation or a history operation differs.
    Continuation = 6,
    /// A decision or a conditional needs an input the live path did not record.
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
