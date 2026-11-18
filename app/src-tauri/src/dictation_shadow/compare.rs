//! The comparison, run on the `murmur-dictation-shadow` thread (appendix C8.3), or
//! on the calling thread in the synchronous test mode.
//!
//! **It never executes an effect and never touches a live field, lock, port or
//! window.** It holds no port at all: `DictationCore::handle` is pure, the live
//! values it compares against arrive as plain data in the messages, and the only
//! state it owns is [`ShadowCore`] (persisted M1 and M2) and its own bookkeeping.
//! It never draws an op id: the header carries the ids the live shell drew
//! (`live_ops`) and the shadow replays them (`replayed_token`). The comparator is
//! owned by exactly one thread and needs no lock.
//!
//! * **Ordering is by `seq`, never by time.** A header is applied when every smaller
//!   `seq` was applied or lost. Early headers wait in a buffer of at most 64
//!   ([`MAX_PENDING_HEADERS`]); a gap still open when more than 64 later headers
//!   wait is declared lost. A flight lasts 2 to 17 s, so a time window could not tell
//!   a slow flight from a lost record, and none exists.
//! * **Loss is attributed, never guessed** (D-S2c-1 c). A message the queue could not
//!   take arrives as a [`LossEntry`] naming its recorder and `seq`. The affected
//!   record is invalidated and closed unchecked (never compared as clean); a lost
//!   header's position taints, **in `seq` order**, the neighbours at that position
//!   (the last applied header and the next one); a header that arrives after its
//!   position was declared lost, or for an invalidated flight, is discarded. A header
//!   that allocated a new id (a fresh generation) is never tainted by a neighbour.
//! * **At most 64 records are open** ([`MAX_OPEN_RECORDS`]); the oldest beyond that
//!   is closed as `lost_open`. A witness that arrives before its header is held (at
//!   most 32 per recorder).
//! * **Comparison points** (CP1-CP4), each reporting the first failing level in
//!   `diff_kind` order, with `Pre` compared before `In`. Effects are matched across
//!   placements, so an effect that ran in another placement than decided is
//!   `Placement` (3), not a missing and an extra effect. Every conditional, mirror
//!   and ticker attempt needs exactly one witness at a completed close.
//! * **Cascade control**: a mismatch taints its recording id, and a later mismatch
//!   with the same `(site, row, diff_kind)`, or any later level of a record that
//!   already mismatched, is a `followup`, not a new root.
//! * **Resync, not cascade**: every header resets M1/M2 from its own `pre`.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};

use murmur_dictation_core::decision::{ContKind, Continuation, Decision, OpStatus, Verdict};
use murmur_dictation_core::effects::{Cond, Effect, PerfOutcome, Placement};
use murmur_dictation_core::ids::{Now, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::state::{DictationCore, LiveObs, PersistedCore};
use murmur_dictation_core::status::Status;

use super::pd::{awaiting_outcome, credited, PdEvidence};
use super::summary::Stats;
use super::transport::Transport;
use super::types::{
    CondWitness, DiffKind, EffectResult, HistoryAction, HistoryTable, LiveOp, LossEntry,
    MirrorField, MirrorValue, MirrorWitness, ObservedFact, PredId, RecorderId, ReplyKind,
    ShadowHeader, ShadowMsg, TickerGate, WitnessedEffect,
};

/// Early headers held while a gap is open (C8.3).
pub(crate) const MAX_PENDING_HEADERS: usize = 64;
/// Records open at once; the oldest beyond this is closed as `lost_open`.
pub(crate) const MAX_OPEN_RECORDS: usize = 64;
/// Messages held per recorder while its header has not arrived.
pub(crate) const MAX_HELD_PER_REC: usize = 32;
/// Closed recorders remembered, so a late message for one is dropped, not held.
const CLOSED_MEMORY: usize = 512;
/// Recording ids remembered for taint and cascade control.
const MAX_REMEMBERED_IDS: usize = 1024;
/// Rows remembered per recording id for the two-flight preserved defects.
const MAX_ROWS_PER_ID: usize = 8;

/// Rate limit of the mismatch event (C8.4 step 4): one per `(site, input_kind,
/// diff_kind)` per 60 s, and at most 50 per session.
pub(crate) const EVENT_WINDOW_MS: u64 = 60_000;
pub(crate) const MAX_EVENTS_PER_SESSION: u64 = 50;

/// The numeric content of `pipeline.dictation_shadow_mismatch` (C8.4): three
/// numbers and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MismatchEvent {
    pub site_id: u16,
    pub input_kind: u16,
    pub diff_kind: u8,
}

/// Decides one input against one view. Production: `DictationCore::handle`. A test
/// injects a panicking one to exercise `diff_kind 8`.
pub(crate) type Decider = Box<dyn Fn(&DictationCore, Input, Now) -> Decision + Send>;
/// Receives each mismatch event the rate limit lets through. Production: the
/// numeric file sink (`summary::EventFile`), never `tracing`.
pub(crate) type EventSink = Box<dyn FnMut(MismatchEvent) + Send>;
/// Milliseconds on a monotonic clock, for the rate limit only.
pub(crate) type Clock = Box<dyn Fn() -> u64 + Send>;

/// Persisted M1 and M2 only (C8.3). M3 and M4 are never stored here: they are
/// replayed from each header's `pre.obs` as observations.
pub(crate) struct ShadowCore {
    persisted: PersistedCore,
}

impl ShadowCore {
    pub(crate) fn new() -> Self {
        Self {
            persisted: PersistedCore::default(),
        }
    }

    /// `resync_m`: set M1 and M2 from the live values the header recorded.
    pub(crate) fn resync_m(&mut self, pre: &super::types::Projection) {
        // Through the core's own projection, not `PersistedCore::new`: that
        // constructor has exactly one caller in `src/` (the live adapter's seed).
        self.persisted =
            DictationCore::project(pre.status, pre.current_id, LiveObs::default()).persisted();
    }

    /// The decision view: the persisted M1/M2 plus this decision's observations.
    pub(crate) fn view(&self, obs: LiveObs) -> DictationCore {
        DictationCore::new(self.persisted, obs)
    }

    /// Commit what the decision's `ST` and `ID+` write (the only M1/M2 writers).
    fn commit(&mut self, decision: &Decision) {
        for placed in &decision.effects {
            match placed.effect {
                Effect::SetStatus(to) => self.persisted.set_status(to),
                Effect::AllocId => {
                    self.persisted.alloc_id();
                }
                _ => {}
            }
        }
    }

    fn post(&self) -> (Status, RecordingId) {
        (self.persisted.status(), self.persisted.current_id())
    }
}

/// The token a continuation input will carry, replayed from the id the live shell
/// drew. The shadow never draws from `NEXT_OP` (C8.1): this is the only place an op
/// id enters its view of a decision.
pub(crate) fn replayed_token(decision: &Decision, live_ops: &[LiveOp]) -> Option<Continuation> {
    let spec = decision.continuation?;
    live_ops
        .iter()
        .find(|op| {
            op.table == HistoryTable::H1
                && op.action == HistoryAction::Registered
                && op.cont == spec.kind
                && op.recording_id == spec.recording_id
        })
        .map(|op| Continuation {
            recording_id: op.recording_id,
            op: op.op,
            kind: op.cont,
        })
}

// ------------------------------------------------------------ record helpers

fn cond_of(effect: &Effect) -> Cond {
    match effect {
        Effect::EmitStatus { cond, .. } | Effect::Presentation { cond, .. } => *cond,
        _ => Cond::Always,
    }
}

fn pred_of(cond: Cond) -> Option<PredId> {
    match cond {
        Cond::Always => None,
        Cond::UnlessSpeculativeFlowing => Some(PredId::SpecFlowing),
        Cond::EmitOk(paired) => Some(PredId::EmitOk(paired)),
    }
}

fn mirror_field_of(effect: &Effect) -> Option<MirrorField> {
    match effect {
        Effect::ClearContext(_) | Effect::SetContext(_) => Some(MirrorField::ActiveContext),
        Effect::MarkCancelled(_) => Some(MirrorField::CancelledThrough),
        Effect::KeyboardProcessing(_) => Some(MirrorField::KeyboardProcessing),
        _ => None,
    }
}

/// Effects whose live-field write is captured in the header's mirror lists (the
/// context slot, read inside the adapter's existing lock). `MC` and `KB` write after
/// D and arrive as `Mirror` messages.
fn is_header_mirrored(effect: &Effect) -> bool {
    matches!(effect, Effect::ClearContext(_) | Effect::SetContext(_))
}

/// Whether the live field's value before and after an effect is the decided commit:
/// `CX-(id)` clears a slot holding `id` and leaves a slot holding another recording
/// alone (stale-safe); `CX+(id)` installs `id`; `MC(id)` leaves the high-water mark
/// at `max(before, id)`; `KB(b)` leaves `b`.
pub(crate) fn mirror_ok(effect: &Effect, mirror: &MirrorWitness) -> bool {
    if Some(mirror.field) != mirror_field_of(effect) {
        return false;
    }
    match (effect, mirror.before, mirror.after) {
        (Effect::ClearContext(id), MirrorValue::Slot(before), MirrorValue::Slot(after)) => {
            if before == Some(*id) {
                after.is_none()
            } else {
                after == before
            }
        }
        (Effect::SetContext(id), MirrorValue::Slot(_), MirrorValue::Slot(after)) => {
            after == Some(*id)
        }
        (Effect::MarkCancelled(id), MirrorValue::Id(before), MirrorValue::Id(after)) => {
            after == before.max(*id)
        }
        (Effect::KeyboardProcessing(flag), MirrorValue::Flag(_), MirrorValue::Flag(after)) => {
            after == *flag
        }
        _ => false,
    }
}

fn spec_flowing(observed: &[ObservedFact]) -> Option<bool> {
    observed.first().map(|fact| match fact {
        ObservedFact::SpeculativeFlowing(flowing) => *flowing,
    })
}

fn placement_index(placement: Placement) -> usize {
    match placement {
        Placement::Pre => 0,
        Placement::In => 1,
        Placement::PostDT => 2,
        Placement::PostT => 3,
    }
}

fn push_unique(ids: &mut Vec<RecordingId>, id: RecordingId) {
    if !id.is_none() && !ids.contains(&id) {
        ids.push(id);
    }
}

/// Every recording id a header names, for cascade signatures.
fn header_ids(header: &ShadowHeader) -> Vec<RecordingId> {
    let mut ids = Vec::with_capacity(3);
    for id in [
        header.input.recording_id(),
        Some(header.pre.current_id),
        Some(header.live_post_d.current_id),
    ]
    .into_iter()
    .flatten()
    {
        push_unique(&mut ids, id);
    }
    ids
}

/// The decision allocated a new id: a fresh generation.
fn is_fresh_generation(header: &ShadowHeader) -> bool {
    header.live_post_d.current_id > header.pre.current_id
}

/// The ids a header's decision is *about*, for loss taint. A fresh generation is
/// about its new id only, so a neighbour's loss never taints it; anything else is
/// about the id it names and the id that was current.
fn subject_ids(header: &ShadowHeader) -> Vec<RecordingId> {
    let mut ids = Vec::with_capacity(2);
    if is_fresh_generation(header) {
        push_unique(&mut ids, header.live_post_d.current_id);
    } else {
        if let Some(id) = header.input.recording_id() {
            push_unique(&mut ids, id);
        }
        push_unique(&mut ids, header.pre.current_id);
    }
    ids
}

/// The token and validated status an input carries, if it carries one.
pub(crate) fn input_token(input: &Input) -> Option<(Continuation, OpStatus)> {
    match input {
        Input::StartCaptureResult(i) => Some((i.token, i.op_status)),
        Input::StartContext(i) => Some((i.token, i.op_status)),
        Input::ReadyPublish(i) => Some((i.token, i.op_status)),
        Input::StopContext(i) => Some((i.token, i.op_status)),
        Input::StopCaptureResult(i) => Some((i.token, i.op_status)),
        Input::CancelCaptureResult(i) => Some((i.token, i.op_status)),
        _ => None,
    }
}

/// The recording whose pipeline a decision registers in H2: the rows that return
/// `K:Pipeline` (R203.9, R203.10, R301.5). The same continuation is the H1 operation
/// and the H2 registration, so the row declares both (D-S2c-2 d).
fn decision_pipeline(decision: &Decision) -> Option<RecordingId> {
    decision
        .continuation
        .filter(|spec| spec.kind == ContKind::Pipeline)
        .map(|spec| spec.recording_id)
}

/// The retirement names the very token the input carried: recording, kind and op.
fn same_token(op: &LiveOp, token: &Continuation) -> bool {
    op.recording_id == token.recording_id && op.cont == token.kind && op.op == token.op
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Decided, not yet witnessed.
    Pending,
    Ran,
    /// A conditional the live path reported skipped.
    Skipped,
}

/// What a conditional's predicate must have been.
enum Expect {
    Known(bool),
    /// The input that decides it was not recorded (`diff_kind 7`).
    Missing,
}

/// One open decision: what was decided and what has been witnessed so far.
struct Record {
    site_id: u16,
    row: u16,
    row_name: &'static str,
    /// The recording that was current when the decision was taken (the key of the
    /// two-flight preserved defects).
    pre_id: RecordingId,
    /// Every id the header named, for cascade signatures.
    ids: Vec<RecordingId>,
    /// What the decision is about, for loss taint.
    subject: Vec<RecordingId>,
    decision: Decision,
    observed: Vec<ObservedFact>,
    slots: Vec<Slot>,
    /// `Emitted{ok}` results by decided index: what `P-emit-ok(i)` reads.
    emit_ok: HashMap<usize, bool>,
    /// The shadow's M1/M2 immediately before each decided effect.
    state_before: Vec<(Status, RecordingId)>,
    /// What ran, by legend code, for the preserved-defect evidence.
    ran: Vec<(&'static str, EffectResult)>,
    tickers: Vec<TickerGate>,
    /// Decided indices of `PostDT` / `PostT` conditionals, in order.
    later_conds: Vec<usize>,
    later_cond_cursor: usize,
    /// Decided `PostDT` / `PostT` effects that write a mirrored live field.
    later_mirrors: Vec<(usize, bool)>,
    /// Decided `PostDT` / `PostT` `SD` effects.
    later_tickers: Vec<(usize, bool)>,
    last_idx: [Option<usize>; 4],
    h1_registered: u8,
    /// The token the input carried and its validated status: the full H1 token whose
    /// retirement the row requires (D-S2c-2 d).
    token: Option<(Continuation, OpStatus)>,
    /// The input's token was retired, exactly once, with the outcome its status says.
    h1_retired: bool,
    /// The recording whose pipeline the row registers in H2 (`K:Pipeline`: R203.9,
    /// R203.10, R301.5), if it does.
    h2_needed: Option<RecordingId>,
    h2_registered: bool,
    /// The recording the decision's `SD` targets: the input's recording id, else the
    /// current id at the effect (the start's freshly allocated one).
    side_target: Option<RecordingId>,
    /// A `PipelineGuardFinish(Completed)` ran: the successful performance outcome.
    perf_success: bool,
    reply: Option<ReplyKind>,
    /// Some comparison reported a mismatch for this record.
    mismatched: bool,
    /// An attributed loss could have touched it: its messages are consumed unchecked.
    skipped: bool,
}

impl Record {
    fn new(header: &ShadowHeader, decision: Decision, skipped: bool) -> Self {
        let later = |placed: &murmur_dictation_core::effects::Placed| {
            matches!(placed.at, Placement::PostDT | Placement::PostT)
        };
        let later_conds = decision
            .effects
            .iter()
            .enumerate()
            .filter(|(_, p)| later(p) && pred_of(cond_of(&p.effect)).is_some())
            .map(|(i, _)| i)
            .collect();
        let later_mirrors = decision
            .effects
            .iter()
            .enumerate()
            .filter(|(_, p)| later(p) && mirror_field_of(&p.effect).is_some())
            .map(|(i, _)| (i, false))
            .collect();
        let later_tickers = decision
            .effects
            .iter()
            .enumerate()
            .filter(|(_, p)| later(p) && p.effect == Effect::StartSideTasks)
            .map(|(i, _)| (i, false))
            .collect();
        let mut state = (header.pre.status, header.pre.current_id);
        let state_before = decision
            .effects
            .iter()
            .map(|placed| {
                let before = state;
                match placed.effect {
                    Effect::SetStatus(to) => state.0 = to,
                    Effect::AllocId => state.1 = RecordingId(state.1 .0 + 1),
                    _ => {}
                }
                before
            })
            .collect();
        let h2_needed = decision_pipeline(&decision);
        Self {
            site_id: header.site_id,
            row: header.row,
            row_name: decision.row.0,
            pre_id: header.pre.current_id,
            ids: header_ids(header),
            subject: subject_ids(header),
            slots: vec![Slot::Pending; decision.effects.len()],
            decision,
            observed: header.observed.to_vec(),
            emit_ok: HashMap::new(),
            state_before,
            ran: Vec::new(),
            tickers: Vec::new(),
            later_conds,
            later_cond_cursor: 0,
            later_mirrors,
            later_tickers,
            last_idx: [None; 4],
            h1_registered: 0,
            h1_retired: false,
            h2_needed,
            h2_registered: false,
            side_target: header.input.recording_id(),
            perf_success: false,
            token: input_token(&header.input),
            reply: None,
            mismatched: false,
            skipped,
        }
    }

    fn decided_indices(&self, group: Placement) -> Vec<usize> {
        self.decision
            .effects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.at == group)
            .map(|(i, _)| i)
            .collect()
    }

    fn note_ran(&mut self, idx: usize) {
        if matches!(
            self.decision.effects[idx].effect,
            Effect::PipelineGuardFinish(PerfOutcome::Completed)
        ) {
            self.perf_success = true;
        }
    }

    fn mark_ran(&mut self, idx: usize, result: EffectResult) {
        self.note_ran(idx);
        self.slots[idx] = Slot::Ran;
        self.ran
            .push((self.decision.effects[idx].effect.code(), result));
        if let EffectResult::Emitted { ok } = result {
            self.emit_ok.insert(idx, ok);
        }
    }

    /// What a conditional's predicate must have been, from the observation or the
    /// paired emit's own result.
    fn expected_taken(&self, cond: Cond) -> Expect {
        match cond {
            Cond::Always => Expect::Known(true),
            Cond::UnlessSpeculativeFlowing => match spec_flowing(&self.observed) {
                Some(flowing) => Expect::Known(!flowing),
                None => Expect::Missing,
            },
            Cond::EmitOk(paired) => {
                let paired = usize::from(paired);
                match self.slots.get(paired) {
                    Some(Slot::Skipped) => Expect::Known(false),
                    Some(Slot::Ran) => match self.emit_ok.get(&paired) {
                        Some(ok) => Expect::Known(*ok),
                        None => Expect::Missing,
                    },
                    _ => Expect::Missing,
                }
            }
        }
    }

    /// The ticker attempt was for the recording the decision's `SD` (decided effect
    /// `j`) targets: the input's recording id, else the id current at that effect.
    fn target_matches(&self, j: usize, gate: &TickerGate) -> bool {
        gate.id == self.side_target.unwrap_or(self.state_before[j].1)
    }

    /// CP1 over the header's `Pre` and `In` lists. Effects are matched across
    /// placements (same placement first, then any), so an effect that ran in another
    /// placement than decided is `Placement` and is not also a missing and an extra
    /// effect. Returns the failing levels of the `Pre` group and of the `In` group;
    /// the caller reports `Pre` first, each at its lowest `diff_kind`.
    fn check_header_effects(&mut self, header: &ShadowHeader) -> [Vec<DiffKind>; 2] {
        let groups = [Placement::Pre, Placement::In];
        let mut diffs: [Vec<DiffKind>; 2] = [Vec::new(), Vec::new()];
        let actual_lists: [Vec<&WitnessedEffect>; 2] = [
            header.pre_effects.iter().collect(),
            header.in_d_effects.iter().collect(),
        ];
        let cond_lists: [Vec<&CondWitness>; 2] = [
            header.pre_conditionals.iter().collect(),
            header.in_d_conditionals.iter().collect(),
        ];

        // 1. Conditionals: which decided effects the live path skipped.
        let mut cond_pairs: Vec<(usize, usize, bool)> = Vec::new();
        for (g, group) in groups.iter().enumerate() {
            let cond_idxs: Vec<usize> = self
                .decided_indices(*group)
                .into_iter()
                .filter(|&i| pred_of(cond_of(&self.decision.effects[i].effect)).is_some())
                .collect();
            let witnesses = &cond_lists[g];
            if witnesses.len() < cond_idxs.len() {
                diffs[g].push(DiffKind::EffectSet);
            } else if witnesses.len() > cond_idxs.len() {
                diffs[g].push(DiffKind::Conditional);
            }
            for (k, &i) in cond_idxs.iter().enumerate() {
                let Some(witness) = witnesses.get(k) else {
                    continue;
                };
                if Some(witness.pred) != pred_of(cond_of(&self.decision.effects[i].effect)) {
                    diffs[g].push(DiffKind::Conditional);
                    continue;
                }
                if !witness.taken {
                    self.slots[i] = Slot::Skipped;
                }
                cond_pairs.push((g, i, witness.taken));
            }
        }

        // 2. Effects: same placement first, then any placement (a move).
        let flat: Vec<(usize, &WitnessedEffect)> = actual_lists
            .iter()
            .enumerate()
            .flat_map(|(g, list)| list.iter().map(move |w| (g, *w)))
            .collect();
        let mut matched: Vec<Option<usize>> = vec![None; flat.len()];
        for (a, (g, witnessed)) in flat.iter().enumerate() {
            matched[a] = self.decision.effects.iter().enumerate().position(|(j, p)| {
                self.slots[j] == Slot::Pending && p.at == groups[*g] && p.effect == witnessed.effect
            });
            if let Some(j) = matched[a] {
                self.slots[j] = Slot::Ran;
            }
        }
        for (a, (_, witnessed)) in flat.iter().enumerate() {
            if matched[a].is_some() {
                continue;
            }
            matched[a] =
                self.decision.effects.iter().enumerate().position(|(j, p)| {
                    self.slots[j] == Slot::Pending && p.effect == witnessed.effect
                });
            if let Some(j) = matched[a] {
                self.slots[j] = Slot::Ran;
            }
        }
        let mut in_order: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for (a, (g, witnessed)) in flat.iter().enumerate() {
            match matched[a] {
                Some(j) => {
                    self.note_ran(j);
                    self.ran
                        .push((self.decision.effects[j].effect.code(), witnessed.result));
                    if let EffectResult::Emitted { ok } = witnessed.result {
                        self.emit_ok.insert(j, ok);
                    }
                    let decided_at = self.decision.effects[j].at;
                    if decided_at == groups[*g] {
                        in_order[*g].push(j);
                    } else {
                        diffs[*g].push(DiffKind::Placement);
                        if let Some(origin) = groups.iter().position(|at| *at == decided_at) {
                            diffs[origin].push(DiffKind::Placement);
                        }
                    }
                }
                None => diffs[*g].push(DiffKind::EffectSet),
            }
        }
        // A decided `Pre` / `In` effect that nothing in the header matched is **not**
        // classified here (D-S2c-2 g): the live path may have run it later, and a
        // later witness then reports it as `Placement`. It stays pending, and is a
        // missing effect (`EffectSet`) only when the record closes completed.
        for g in 0..groups.len() {
            if in_order[g].windows(2).any(|pair| pair[1] < pair[0]) {
                diffs[g].push(DiffKind::EffectOrder);
            }
        }

        // 3. Conditionals: taken or skipped against what the input says it must be.
        for (g, i, taken) in cond_pairs {
            match self.expected_taken(cond_of(&self.decision.effects[i].effect)) {
                Expect::Known(want) if want != taken => diffs[g].push(DiffKind::Conditional),
                Expect::Missing => diffs[g].push(DiffKind::Observation),
                Expect::Known(_) => {}
            }
        }

        // 4. The context slot (M4) before and after each `CX±` that ran in the group.
        for (g, list) in [&header.pre_mirrors, &header.in_d_mirrors]
            .iter()
            .enumerate()
        {
            let mirrored: Vec<usize> = in_order[g]
                .iter()
                .copied()
                .filter(|&j| is_header_mirrored(&self.decision.effects[j].effect))
                .collect();
            let mirrors: Vec<&MirrorWitness> = list.iter().collect();
            if mirrors.len() != mirrored.len() {
                diffs[g].push(DiffKind::EffectSet);
            } else if mirrored
                .iter()
                .zip(&mirrors)
                .any(|(&j, mirror)| !mirror_ok(&self.decision.effects[j].effect, mirror))
            {
                diffs[g].push(DiffKind::PostState);
            }
        }

        // 5. Ticker registrations attempted under D (`In:SD`).
        let in_sd: Vec<usize> = in_order[1]
            .iter()
            .copied()
            .filter(|&j| self.decision.effects[j].effect == Effect::StartSideTasks)
            .collect();
        let gates: Vec<&TickerGate> = header.in_d_tickers.iter().collect();
        if gates.len() != in_sd.len() {
            diffs[1].push(DiffKind::EffectSet);
        } else {
            for (&j, gate) in in_sd.iter().zip(&gates) {
                // Under D the gate reads the state the decision itself just wrote.
                // The attempt must be for the recording the decision's `SD` targets
                // before its outcome means anything (D-S2c-2 f).
                let (status, current) = self.state_before[j];
                if !self.target_matches(j, gate)
                    || !gate_consistent(gate)
                    || gate.status != status
                    || gate.current_id != current
                {
                    diffs[1].push(DiffKind::PostState);
                }
            }
        }
        self.tickers.extend(gates.into_iter().copied());
        diffs
    }

    /// The H1 / H2 operations of the decision, against what the row declares
    /// (D-S2c-2 d). Returns every failing level (always `Continuation`).
    ///
    /// * H1 registration: exactly one, of the decided continuation (kind and
    ///   recording), whose token the shadow replays;
    /// * H1 retirement: the input's own token, in full (recording, kind and op),
    ///   exactly once, and found exactly when the input says it was outstanding;
    /// * H2 registration: only for a `K:Pipeline` row, once, for the decided recording;
    /// * H2 retirement: must have found the pipeline, and names a recording the
    ///   decision is about.
    ///
    /// Presence (a missing operation) is judged by [`Record::missing_history`] at the
    /// boundary where the operation is due.
    fn check_history<'a>(&mut self, ops: impl Iterator<Item = &'a LiveOp>) -> Vec<DiffKind> {
        let mut diffs = Vec::new();
        for op in ops {
            let ok = match (op.table, op.action) {
                (HistoryTable::H1, HistoryAction::Registered) => {
                    let wanted = self.decision.continuation.is_some_and(|spec| {
                        spec.kind == op.cont && spec.recording_id == op.recording_id
                    });
                    let first = self.h1_registered == 0;
                    self.h1_registered = self.h1_registered.saturating_add(1);
                    wanted && first
                }
                (HistoryTable::H1, HistoryAction::Retired { matched }) => match self.token {
                    Some((token, status)) if same_token(op, &token) => {
                        let first = !self.h1_retired;
                        self.h1_retired = true;
                        first && matched == (status == OpStatus::Outstanding)
                    }
                    // Another operation's retirement (the pipeline's own, an
                    // owner's drop): not the row's token, nothing to compare.
                    _ => true,
                },
                (HistoryTable::H2, HistoryAction::Registered) => {
                    let wanted = self.h2_needed == Some(op.recording_id);
                    let first = !self.h2_registered;
                    self.h2_registered = true;
                    wanted && first
                }
                (HistoryTable::H2, HistoryAction::Retired { matched }) => {
                    matched && self.ids.contains(&op.recording_id)
                }
            };
            if !ok {
                diffs.push(DiffKind::Continuation);
            }
        }
        diffs
    }

    /// An operation the row requires that nobody reported. At the header: the
    /// input's H1 retirement (it happens before the decision is taken, so the header
    /// carries it). At a completed close: the H2 registration (a `K:Pipeline` row
    /// registers it right after the decision or when it spawns).
    fn missing_history(&self, completed_close: bool) -> bool {
        if completed_close {
            self.h2_needed.is_some() && !self.h2_registered
        } else {
            self.token.is_some() && !self.h1_retired
        }
    }

    /// CP2: one effect that ran after D, against the decided effect at its index.
    fn on_witness(
        &mut self,
        idx: u16,
        placement: Placement,
        witnessed: &WitnessedEffect,
    ) -> Option<DiffKind> {
        let idx = usize::from(idx);
        let Some(decided) = self.decision.effects.get(idx) else {
            return Some(DiffKind::EffectSet);
        };
        let (decided_effect, decided_at) = (decided.effect.clone(), decided.at);
        let slot = self.slots[idx];
        let mut diff = None;
        if slot == Slot::Skipped {
            // The live path said it skipped this conditional and then ran it.
            diff = Some(DiffKind::Conditional);
        } else if slot == Slot::Ran {
            diff = Some(DiffKind::EffectSet);
        } else if decided_effect != witnessed.effect {
            // Swapped with another pending effect of the same placement: an order
            // difference. Anything else is a different set.
            let partner = self.decision.effects.iter().enumerate().position(|(j, p)| {
                j != idx
                    && self.slots[j] == Slot::Pending
                    && p.at == placement
                    && p.effect == witnessed.effect
            });
            match partner {
                Some(j) => {
                    self.mark_ran(j, witnessed.result);
                    diff = Some(DiffKind::EffectOrder);
                }
                None => diff = Some(DiffKind::EffectSet),
            }
        } else if decided_at != placement {
            diff = Some(DiffKind::Placement);
        } else if self.last_idx[placement_index(placement)].is_some_and(|last| idx < last) {
            diff = Some(DiffKind::EffectOrder);
        }
        if self.slots[idx] == Slot::Pending && decided_effect == witnessed.effect {
            self.mark_ran(idx, witnessed.result);
        }
        let last = &mut self.last_idx[placement_index(placement)];
        *last = Some(last.map_or(idx, |l| l.max(idx)));
        diff
    }

    /// CP2: a shell-conditional after D, as (predicate id, taken).
    fn on_cond(&mut self, pred: PredId, taken: bool) -> Option<DiffKind> {
        let Some(&idx) = self.later_conds.get(self.later_cond_cursor) else {
            // More conditionals than the decision has.
            return Some(DiffKind::Conditional);
        };
        self.later_cond_cursor += 1;
        let cond = cond_of(&self.decision.effects[idx].effect);
        if pred_of(cond) != Some(pred) {
            return Some(DiffKind::Conditional);
        }
        if !taken {
            self.slots[idx] = Slot::Skipped;
        }
        match self.expected_taken(cond) {
            Expect::Known(want) => (want != taken).then_some(DiffKind::Conditional),
            Expect::Missing => Some(DiffKind::Observation),
        }
    }

    /// CP3: a live M-field written at effect time, before and after, against the
    /// decided commit.
    fn on_mirror(&mut self, mirror: &MirrorWitness) -> Option<DiffKind> {
        let entry = self.later_mirrors.iter_mut().find(|(i, consumed)| {
            !*consumed && mirror_field_of(&self.decision.effects[*i].effect) == Some(mirror.field)
        });
        let Some((idx, consumed)) = entry else {
            // A write the decision does not have, or a second one.
            return Some(DiffKind::EffectSet);
        };
        *consumed = true;
        (!mirror_ok(&self.decision.effects[*idx].effect, mirror)).then_some(DiffKind::PostState)
    }

    /// CP3: a ticker attempt after D (`PostDT:SD`), against the gate's own inputs.
    fn on_ticker(&mut self, gate: &TickerGate) -> Option<DiffKind> {
        let Some(&mut (j, ref mut consumed)) = self
            .later_tickers
            .iter_mut()
            .find(|(_, consumed)| !*consumed)
        else {
            return Some(DiffKind::EffectSet);
        };
        *consumed = true;
        self.tickers.push(*gate);
        // After release too, the attempt must be for the decision's target first.
        (!self.target_matches(j, gate) || !gate_consistent(gate)).then_some(DiffKind::PostState)
    }

    /// CP4: the flight closed.
    fn on_close(&mut self, completed: bool) -> Option<DiffKind> {
        let mut diffs = Vec::new();
        let expected_reply = self.decision.reply.as_ref().map(ReplyKind::of);
        if completed {
            // Every decided effect, conditional, mirror and ticker attempt was
            // witnessed exactly once; a missing one is `EffectSet`. A record that
            // already mismatched left its unmatched effects pending on purpose:
            // reporting them again would be the same fault twice.
            if !self.mismatched
                && (self.slots.contains(&Slot::Pending)
                    || self.later_cond_cursor < self.later_conds.len()
                    || self
                        .later_mirrors
                        .iter()
                        .any(|(i, consumed)| !consumed && self.slots[*i] == Slot::Ran)
                    || self
                        .later_tickers
                        .iter()
                        .any(|(i, consumed)| !consumed && self.slots[*i] == Slot::Ran))
            {
                diffs.push(DiffKind::EffectSet);
            }
            if !self.mismatched && self.missing_history(true) {
                diffs.push(DiffKind::Continuation);
            }
            if expected_reply != self.reply {
                diffs.push(DiffKind::Reply);
            }
        } else if self.reply.is_some() && expected_reply != self.reply {
            // An abandoned flight may never have replied; one that did must agree.
            diffs.push(DiffKind::Reply);
        }
        diffs.into_iter().min()
    }
}

/// A ticker gate's recorded result against its own recorded inputs (query 404).
fn gate_consistent(gate: &TickerGate) -> bool {
    gate.registered == gate.expected()
}

// ----------------------------------------------------------------- comparator

/// What the comparison does with each message. Owned by the shadow thread, or
/// driven on the calling thread in the synchronous test mode.
pub(crate) struct Comparator {
    core: ShadowCore,
    decider: Decider,
    sink: EventSink,
    clock: Clock,
    next_seq: u64,
    pending: BTreeMap<u64, ShadowHeader>,
    /// Positions a loss entry named: skipped, in `seq` order, when reached.
    lost_seqs: BTreeSet<u64>,
    records: HashMap<RecorderId, Record>,
    held: HashMap<RecorderId, Vec<ShadowMsg>>,
    /// Open recorders (records and held ones), oldest first.
    open: VecDeque<RecorderId>,
    /// Recorders finished with: closed, abandoned, evicted or invalidated.
    closed: VecDeque<RecorderId>,
    /// Ids an attributed loss could have touched: their decisions are skipped.
    tainted: BTreeSet<u64>,
    /// The next applied header is the neighbour of a lost position.
    taint_next: bool,
    /// What the last applied header was about.
    last_subject: Vec<RecordingId>,
    /// Root mismatch signatures by recording id (cascade control).
    signatures: BTreeMap<u64, Vec<(u16, u16, u8)>>,
    /// Rows that closed clean, by the recording that was current (PD windows).
    clean_rows: BTreeMap<u64, Vec<&'static str>>,
    /// Preserved defects witnessed except for the recording's later successful
    /// performance outcome (PD-4): `(pd, site, row)` by recording id.
    pd_pending: BTreeMap<u64, Vec<(u8, u16, u16)>>,
    event_last_ms: HashMap<(u16, u16, u8), u64>,
    events_sent: u64,
    pub(crate) stats: Stats,
}

impl Comparator {
    pub(crate) fn new(decider: Decider, sink: EventSink, clock: Clock) -> Self {
        Self {
            core: ShadowCore::new(),
            decider,
            sink,
            clock,
            next_seq: 1,
            pending: BTreeMap::new(),
            lost_seqs: BTreeSet::new(),
            records: HashMap::new(),
            held: HashMap::new(),
            open: VecDeque::new(),
            closed: VecDeque::new(),
            tainted: BTreeSet::new(),
            taint_next: false,
            last_subject: Vec::new(),
            signatures: BTreeMap::new(),
            clean_rows: BTreeMap::new(),
            pd_pending: BTreeMap::new(),
            event_last_ms: HashMap::new(),
            events_sent: 0,
            stats: Stats::default(),
        }
    }

    /// The production comparator: `DictationCore::handle`, the given event sink and a
    /// monotonic clock.
    pub(crate) fn production(sink: EventSink) -> Self {
        let origin = std::time::Instant::now();
        Self::new(
            Box::new(|core, input, now| core.handle(input, now)),
            sink,
            Box::new(move || u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)),
        )
    }

    /// Take everything the live side has queued: each message is applied **after**
    /// the loss log was read, so a loss recorded before a message was pushed is always
    /// seen first. Returns whether anything was applied.
    pub(crate) fn drain(&mut self, transport: &Transport) -> bool {
        let mut progressed = false;
        while let Some(msg) = transport.pop() {
            self.pump_losses(transport);
            self.apply(msg);
            progressed = true;
        }
        self.pump_losses(transport) | progressed
    }

    /// Fold the live side's counters and the loss log into the comparison. Returns
    /// whether it saw anything.
    pub(crate) fn pump_losses(&mut self, transport: &Transport) -> bool {
        let mut seen = false;
        let dropped = transport.take_dropped();
        let overflow = transport.take_overflow();
        self.stats.dropped += dropped;
        self.stats.dropped_contended += transport.take_contended();
        self.stats.witness_overflow += overflow;
        seen |= dropped > 0 || overflow > 0;
        let unattributed = transport.take_loss_overflow();
        if unattributed > 0 {
            seen = true;
            self.on_unattributed_loss(unattributed);
        }
        while let Some(loss) = transport.pop_loss() {
            seen = true;
            self.on_loss(loss);
        }
        seen
    }

    /// Apply one message. The shadow's own panics are caught per header.
    fn apply(&mut self, msg: ShadowMsg) {
        let outcome = catch_unwind(AssertUnwindSafe(|| self.handle_msg(msg)));
        if outcome.is_err() {
            self.stats.shadow_panic += 1;
        }
    }

    pub(crate) fn handle_msg(&mut self, msg: ShadowMsg) {
        match msg {
            ShadowMsg::Header(header) => self.on_header(header),
            ShadowMsg::PreAbandoned { rec } => {
                // `run(Pre)` began and D was never entered: no decision, no seq, no
                // gap, no taint. Reported, not a mismatch.
                self.stats.abandoned_pre_d += 1;
                self.forget(rec);
            }
            other => {
                let rec = other.rec();
                self.route(rec, other);
            }
        }
    }

    // ------------------------------------------------------------- ordering

    fn on_header(&mut self, header: ShadowHeader) {
        let seq = header.seq;
        let rec = header.rec;
        if seq < self.next_seq {
            // Its position was already declared lost: applying it would compare
            // evidence that was already given up on.
            self.stats.late_header_discarded += 1;
            self.forget(rec);
            return;
        }
        if self.closed.contains(&rec) {
            // Its flight was invalidated or evicted: discard the header, and skip
            // its position in `seq` order so the headers behind it are not stuck.
            self.stats.late_header_discarded += 1;
            self.lost_seqs.insert(seq);
            self.drain_ready();
            return;
        }
        if seq == self.next_seq {
            self.apply_header(header);
            self.next_seq += 1;
            self.drain_ready();
            return;
        }
        self.pending.insert(seq, header);
        while self.pending.len() > MAX_PENDING_HEADERS {
            // More than 64 later headers wait on one open gap: it is lost. A gap is
            // measured in headers, never in time.
            self.lose_position();
            self.drain_ready();
        }
    }

    /// The header at `next_seq` will never be applied: count it, taint its
    /// neighbours at this position, and move on.
    fn lose_position(&mut self) {
        self.stats.lost_gap += 1;
        let before = self.last_subject.clone();
        // The record the neighbour before the gap opened may still be open: it is
        // invalidated here, where the taint is applied, so it cannot close clean or
        // earn preserved-defect credit (D-S2c-2 e).
        self.taint_and_invalidate(&before);
        self.taint_next = true;
        self.next_seq += 1;
    }

    fn drain_ready(&mut self) {
        loop {
            if self.lost_seqs.remove(&self.next_seq) {
                self.lose_position();
            } else if let Some(header) = self.pending.remove(&self.next_seq) {
                self.apply_header(header);
                self.next_seq += 1;
            } else {
                break;
            }
        }
    }

    // ----------------------------------------------------------------- loss

    fn taint(&mut self, ids: &[RecordingId]) {
        for id in ids {
            if !id.is_none() {
                self.tainted.insert(id.0);
            }
        }
        while self.tainted.len() > MAX_REMEMBERED_IDS {
            self.tainted.pop_first();
        }
    }

    /// Taint `ids` and invalidate every open record about one of them: it is
    /// closed unchecked, never clean, and earns no preserved-defect credit.
    fn taint_and_invalidate(&mut self, ids: &[RecordingId]) {
        self.taint(ids);
        let mut invalidated = 0;
        for record in self.records.values_mut() {
            if !record.skipped && record.subject.iter().any(|id| ids.contains(id)) {
                record.skipped = true;
                invalidated += 1;
            }
        }
        self.stats.invalidated += invalidated;
    }

    /// An attributed loss: invalidate what it affects, taint the ids it names, and,
    /// when the lost thing was a header, leave its position to be skipped in `seq`
    /// order. The loss is counted under the site the *recorder* kept (a lost witness
    /// is attributed like a lost header); a loss with no site is unattributed, never
    /// counted under site `0`.
    pub(crate) fn on_loss(&mut self, loss: LossEntry) {
        let rec = RecorderId(loss.rec);
        match u16::try_from(loss.site) {
            Ok(site) if site != 0 => *self.stats.lost_by_site.entry(site).or_default() += 1,
            _ => self.stats.loss_unattributed += 1,
        }
        let mut ids: Vec<RecordingId> = loss
            .ids
            .iter()
            .filter(|id| **id != 0)
            .map(|id| RecordingId(*id))
            .collect();
        // The lost flight's own record is affected whatever the loss named.
        if let Some(record) = self.records.get(&rec) {
            ids.extend(record.subject.iter().copied());
        }
        self.taint_and_invalidate(&ids);
        // Its header may still be waiting behind a gap: it can never be compared, so
        // its position is given up here (in `seq` order, like a lost header's).
        let waiting = self
            .pending
            .iter()
            .find(|(_, header)| header.rec == rec)
            .map(|(seq, _)| *seq);
        if let Some(seq) = waiting {
            self.pending.remove(&seq);
            self.stats.late_header_discarded += 1;
            self.lost_seqs.insert(seq);
        }
        self.forget(rec);
        if loss.header && loss.seq != 0 && loss.seq >= self.next_seq {
            self.lost_seqs.insert(loss.seq);
        }
        self.drain_ready();
    }

    /// The loss log itself overflowed: nothing says which records were affected, so
    /// every open one is invalidated and the next header is treated as a neighbour.
    pub(crate) fn on_unattributed_loss(&mut self, count: u64) {
        self.stats.loss_unattributed += count;
        for record in self.records.values_mut() {
            if !record.skipped {
                record.skipped = true;
                self.stats.invalidated += 1;
            }
        }
        for header in self.pending.values_mut() {
            header.overflowed = true;
        }
        self.held.clear();
        let before = self.last_subject.clone();
        self.taint(&before);
        self.taint_next = true;
    }

    // -------------------------------------------------------------- headers

    fn apply_header(&mut self, header: ShadowHeader) {
        let subject = subject_ids(&header);
        let fresh = is_fresh_generation(&header);
        if std::mem::take(&mut self.taint_next) && !fresh {
            // The neighbour after a lost position: records still open about it are
            // invalidated here too.
            self.taint_and_invalidate(&subject);
        }
        let skipped = subject.iter().any(|id| self.tainted.contains(&id.0));
        let ids = header_ids(&header);
        self.last_subject.clone_from(&subject);
        let (site, row) = (header.site_id, header.row);

        if header.overflowed {
            // A capture list overflowed: the header is incomplete, so there is
            // nothing complete to compare. Counted, never reported as a mismatch.
            self.stats.skipped_incomplete += 1;
            self.forget(header.rec);
            return;
        }

        // The shadow's own decision, from the live pre-state. A panic is the
        // shadow's own bug: caught, counted, never the live path's problem.
        self.core.resync_m(&header.pre);
        let view = self.core.view(header.pre.obs);
        let decided = catch_unwind(AssertUnwindSafe(|| {
            (self.decider)(&view, header.input.clone(), header.now)
        }));
        let decision = match decided {
            Ok(decision) => decision,
            Err(_) => {
                self.stats.compared += 1;
                self.stats.row(site, row).compared += 1;
                self.stats.shadow_panic += 1;
                self.report(site, row, DiffKind::ShadowPanic, &ids);
                self.forget(header.rec);
                return;
            }
        };
        self.stats.compared += 1;
        self.stats.row(site, row).compared += 1;

        if skipped {
            self.stats.skipped_tainted += 1;
        }
        if let Verdict::NeedObservation(_) = decision.verdict {
            // The decision asked for a read the live path did not record: there is
            // nothing complete to compare (`diff_kind 7`).
            if !skipped {
                self.report(site, row, DiffKind::Observation, &ids);
            }
            self.forget(header.rec);
            return;
        }

        self.core.commit(&decision);
        let post = self.core.post();
        let mut record = Record::new(&header, decision, skipped);
        if !skipped {
            if let Some(diff) = Self::cp1(&mut record, &header, post) {
                record.mismatched = true;
                self.report(site, row, diff, &ids);
            }
        }

        // Messages that arrived before the header attach now.
        let held = self.held.remove(&header.rec).unwrap_or_default();
        self.open.retain(|id| *id != header.rec);
        self.open.push_back(header.rec);
        self.records.insert(header.rec, record);
        self.evict_over_cap();
        for msg in held {
            self.route(header.rec, msg);
        }
    }

    /// CP1: the header against the shadow's decision. `Pre` is compared before `In`,
    /// and within each the lowest `diff_kind` is reported.
    fn cp1(
        record: &mut Record,
        header: &ShadowHeader,
        post: (Status, RecordingId),
    ) -> Option<DiffKind> {
        let [pre, mut in_d] = record.check_header_effects(header);
        if post != (header.live_post_d.status, header.live_post_d.current_id) {
            in_d.push(DiffKind::PostState);
        }
        in_d.extend(record.check_history(header.live_ops.iter()));
        if record.missing_history(false) {
            in_d.push(DiffKind::Continuation);
        }
        // A decided continuation needs exactly one registration, whose token the
        // shadow replays (it never draws an id).
        if record.decision.continuation.is_some()
            && (record.h1_registered != 1
                || replayed_token(&record.decision, &header.live_ops.to_vec()).is_none())
        {
            in_d.push(DiffKind::Continuation);
        }
        pre.into_iter().min().or_else(|| in_d.into_iter().min())
    }

    // ------------------------------------------------------------- messages

    fn route(&mut self, rec: RecorderId, msg: ShadowMsg) {
        if self.closed.contains(&rec) {
            // A late message of a flight already closed, abandoned or invalidated:
            // the comparison is over.
            return;
        }
        let Some(mut record) = self.records.remove(&rec) else {
            self.hold(rec, msg);
            return;
        };
        let (site, row) = (record.site_id, record.row);
        let ids = record.ids.clone();
        let mut closing = false;
        let mut completed = false;
        let diff = if record.skipped {
            closing = matches!(msg, ShadowMsg::Close { .. });
            None
        } else {
            match msg {
                ShadowMsg::Witness {
                    idx,
                    placement,
                    effect,
                    ..
                } => record.on_witness(idx, placement, &effect),
                ShadowMsg::Cond { pred, taken, .. } => record.on_cond(pred, taken),
                ShadowMsg::Mirror { mirror, .. } => record.on_mirror(&mirror),
                ShadowMsg::Ticker { gate, .. } => record.on_ticker(&gate),
                ShadowMsg::History { op, .. } => {
                    record.check_history(std::iter::once(&op)).into_iter().min()
                }
                ShadowMsg::Reply { kind, .. } => {
                    record.reply = Some(kind);
                    None
                }
                ShadowMsg::Close {
                    completed: done, ..
                } => {
                    closing = true;
                    completed = done;
                    if !done {
                        self.stats.abandoned += 1;
                    }
                    record.on_close(done)
                }
                _ => None,
            }
        };
        if let Some(diff) = diff {
            if record.mismatched {
                // A later level of a fault already reported for this record.
                self.stats.followup += 1;
                self.stats.row(site, row).mismatched += 1;
            } else {
                record.mismatched = true;
                self.report(site, row, diff, &ids);
            }
        }
        if closing {
            if !record.mismatched && !record.skipped {
                self.stats.matched += 1;
                self.credit_pd(&record, completed);
            }
            self.forget(rec);
        } else {
            self.records.insert(rec, record);
        }
    }

    /// Credit the preserved defects a clean, closed record witnessed, and remember
    /// the row for the two-flight defects.
    fn credit_pd(&mut self, record: &Record, completed: bool) {
        let prior: Vec<&'static str> = self
            .clean_rows
            .get(&record.pre_id.0)
            .cloned()
            .unwrap_or_default();
        let evidence = PdEvidence {
            row: record.row_name,
            ran: &record.ran,
            tickers: &record.tickers,
            recording: record.side_target.unwrap_or(record.pre_id),
            completed,
            prior_rows: &prior,
        };
        for pd in credited(&evidence) {
            self.credit(pd, record.site_id, record.row);
        }
        // A defect that also needs the recording's later successful performance
        // outcome waits for it.
        if completed {
            for pd in awaiting_outcome(&evidence) {
                let waiting = self.pd_pending.entry(evidence.recording.0).or_default();
                waiting.push((pd, record.site_id, record.row));
            }
            while self.pd_pending.len() > MAX_REMEMBERED_IDS {
                self.pd_pending.pop_first();
            }
        }
        // This record's own successful outcome settles what waited on its recording.
        if completed && record.perf_success {
            for id in &record.ids {
                for (pd, site, row) in self.pd_pending.remove(&id.0).unwrap_or_default() {
                    self.credit(pd, site, row);
                }
            }
        }
        if completed && !record.pre_id.is_none() {
            let rows = self.clean_rows.entry(record.pre_id.0).or_default();
            rows.push(record.row_name);
            if rows.len() > MAX_ROWS_PER_ID {
                rows.remove(0);
            }
            while self.clean_rows.len() > MAX_REMEMBERED_IDS {
                self.clean_rows.pop_first();
            }
        }
    }

    fn credit(&mut self, pd: u8, site: u16, row: u16) {
        *self.stats.pd_witness.entry(pd).or_default() += 1;
        self.stats.row(site, row).pd_witness += 1;
    }

    /// A message whose header has not arrived: held, at most 32 per recorder.
    fn hold(&mut self, rec: RecorderId, msg: ShadowMsg) {
        let is_new = !self.held.contains_key(&rec);
        let queue = self.held.entry(rec).or_default();
        if queue.len() >= MAX_HELD_PER_REC {
            self.stats.witness_overflow += 1;
            return;
        }
        queue.push(msg);
        if is_new {
            self.open.push_back(rec);
            self.evict_over_cap();
        }
    }

    /// At most 64 recorders are open: the oldest beyond that is `lost_open`, and
    /// taints what it was about.
    fn evict_over_cap(&mut self) {
        while self.open.len() > MAX_OPEN_RECORDS {
            let Some(oldest) = self.open.pop_front() else {
                break;
            };
            self.stats.lost_open += 1;
            match self.records.remove(&oldest) {
                Some(record) => self.taint(&record.subject),
                None => {
                    let before = self.last_subject.clone();
                    self.taint(&before);
                    self.taint_next = true;
                }
            }
            self.held.remove(&oldest);
            self.remember_closed(oldest);
        }
    }

    /// The recorder is finished with: forget its state and remember the id so a late
    /// message or header is dropped instead of held or applied.
    fn forget(&mut self, rec: RecorderId) {
        self.records.remove(&rec);
        self.held.remove(&rec);
        self.open.retain(|id| *id != rec);
        self.remember_closed(rec);
    }

    fn remember_closed(&mut self, rec: RecorderId) {
        if self.closed.contains(&rec) {
            return;
        }
        self.closed.push_back(rec);
        while self.closed.len() > CLOSED_MEMORY {
            self.closed.pop_front();
        }
    }

    // -------------------------------------------------------------- reports

    /// One mismatch for `(site, row)`. A signature already reported for any of the
    /// ids is a `followup`; anything else is a new root and emits the event.
    fn report(&mut self, site: u16, row: u16, diff: DiffKind, ids: &[RecordingId]) {
        self.stats.row(site, row).mismatched += 1;
        let signature = (site, row, diff.code());
        let explained = ids.iter().any(|id| {
            self.signatures
                .get(&id.0)
                .is_some_and(|known| known.contains(&signature))
        });
        if explained {
            self.stats.followup += 1;
            return;
        }
        self.stats.mismatched_root += 1;
        for id in ids {
            self.signatures.entry(id.0).or_default().push(signature);
        }
        while self.signatures.len() > MAX_REMEMBERED_IDS {
            self.signatures.pop_first();
        }
        self.emit(MismatchEvent {
            site_id: site,
            input_kind: row,
            diff_kind: diff.code(),
        });
    }

    /// At most one event per `(site_id, input_kind, diff_kind)` per 60 s and at most
    /// 50 per session; every suppressed event is counted (C8.4 step 4).
    fn emit(&mut self, event: MismatchEvent) {
        let now = (self.clock)();
        let key = (event.site_id, event.input_kind, event.diff_kind);
        let within_window = self
            .event_last_ms
            .get(&key)
            .is_some_and(|last| now.saturating_sub(*last) < EVENT_WINDOW_MS);
        if within_window || self.events_sent >= MAX_EVENTS_PER_SESSION {
            self.stats.events_suppressed += 1;
            return;
        }
        self.event_last_ms.insert(key, now);
        self.events_sent += 1;
        (self.sink)(event);
    }
}
