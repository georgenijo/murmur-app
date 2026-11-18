//! The comparison, run on the `murmur-dictation-shadow` thread (appendix C8.3), or
//! on the calling thread in synchronous test mode.
//!
//! **It never executes an effect and never touches a live field, lock, port or
//! window.** It holds no port at all: `DictationCore::handle` is pure, the live
//! values it compares against arrive as plain data in the messages, and the only
//! state it owns is [`ShadowCore`] (persisted M1 and M2) and its own bookkeeping.
//! It never draws an op id: the header carries the ids the live shell drew
//! (`live_ops`) and the shadow replays them (`replayed_token`).
//!
//! The pieces:
//!
//! * **Ordering is by `seq`, never by time.** A header is applied when every smaller
//!   `seq` was applied or lost. Early headers wait in a buffer of at most 64
//!   ([`MAX_PENDING_HEADERS`]); a gap still open when more than 64 later headers
//!   wait is declared `lost_gap`. A flight lasts 2 to 17 s, so a time window could
//!   not tell a slow flight from a lost record, and none exists.
//! * **At most 64 records are open** ([`MAX_OPEN_RECORDS`]); the oldest beyond that
//!   is closed as `lost_open`. A witness that arrives before its header is held (at
//!   most 32 per recorder).
//! * **Comparison points** (CP1-CP4), each reporting the first failing level in
//!   `diff_kind` order, with `Pre` compared before `In`.
//! * **Cascade control**: a mismatch taints its recording id, and a later mismatch
//!   with the same `(site, row, diff_kind)` is a `followup`, not a new root. A lost
//!   record (`lost_gap`, `lost_open`, `Lost{seq}`, `dropped`) taints every id it could
//!   have touched, and the decisions that touch them are `skipped_tainted`.
//! * **Resync, not cascade**: every header resets M1/M2 from its own `pre`, so one
//!   wrong post-state never makes the next record fail.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};

use murmur_dictation_core::decision::{Continuation, Decision, Verdict};
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::{Now, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::state::{DictationCore, LiveObs, PersistedCore};
use murmur_dictation_core::status::Status;

use super::pd::pds_for_row;
use super::summary::Stats;
use super::types::{
    CondWitness, DiffKind, LiveOp, LiveOpKind, MirrorField, MirrorValue, MirrorWitness,
    ObservedFact, PredId, RecorderId, ReplyKind, ShadowHeader, ShadowMsg, WitnessedEffect,
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
/// Receives each mismatch event the rate limit lets through.
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

    /// `resync_m`: set M1 and M2 from the live values the header recorded (the
    /// plan's "re-synced from the live state after each input"). The M3/M4
    /// observations are not stored; `view` takes them per decision.
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
            op.kind == LiveOpKind::Registered
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

/// Header-borne mirrors (`pre_mirrors`, `in_d_mirrors`) carry the slot's value
/// before and after, so the stale-safe clear is checked exactly: `CX-(id)` clears
/// only a slot holding `id`.
fn header_mirror_ok(effect: &Effect, mirror: &MirrorWitness) -> bool {
    if mirror.field != MirrorField::ActiveContext {
        return false;
    }
    match effect {
        Effect::ClearContext(id) => {
            if mirror.before == MirrorValue::Slot(Some(*id)) {
                mirror.after == MirrorValue::Slot(None)
            } else {
                mirror.after == mirror.before
            }
        }
        Effect::SetContext(id) => mirror.after == MirrorValue::Slot(Some(*id)),
        _ => false,
    }
}

/// A `Mirror` message carries only the value written; the check is the weaker form
/// of the same commit (C8.3 CP3).
fn later_mirror_ok(effect: &Effect, value: MirrorValue) -> bool {
    match (effect, value) {
        (Effect::ClearContext(id), MirrorValue::Slot(slot)) => slot != Some(*id),
        (Effect::SetContext(id), MirrorValue::Slot(slot)) => slot == Some(*id),
        // `cancelled_id.fetch_max(id)`: at least `id` afterwards.
        (Effect::MarkCancelled(id), MirrorValue::Id(now)) => now >= *id,
        (Effect::KeyboardProcessing(flag), MirrorValue::Flag(now)) => now == *flag,
        _ => false,
    }
}

fn spec_flowing(observed: &[ObservedFact]) -> Option<bool> {
    observed.iter().find_map(|fact| match fact {
        ObservedFact::SpeculativeFlowing(flowing) => Some(*flowing),
        _ => None,
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

/// The recording ids a header names, for taint and cascade control.
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
        if !id.is_none() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Decided, not yet witnessed.
    Pending,
    Ran,
    /// A conditional the live path reported skipped.
    Skipped,
}

/// One open decision: what was decided and what has been witnessed so far.
struct Record {
    site_id: u16,
    row: u16,
    ids: Vec<RecordingId>,
    decision: Decision,
    observed: Vec<ObservedFact>,
    slots: Vec<Slot>,
    /// `Emitted{ok}` results by decided index: what `P-emit-ok(i)` reads.
    emit_ok: HashMap<usize, bool>,
    /// Decided indices of `PostDT` / `PostT` conditionals, in order.
    later_conds: Vec<usize>,
    later_cond_cursor: usize,
    /// Decided `PostDT` / `PostT` effects that write a mirrored live field.
    later_mirrors: Vec<(usize, bool)>,
    last_idx: [Option<usize>; 4],
    reply: Option<ReplyKind>,
    /// Some comparison reported a mismatch for this record.
    mismatched: bool,
    /// A lost record could have touched it: its messages are consumed unchecked.
    skipped: bool,
}

impl Record {
    fn new(
        header: &ShadowHeader,
        decision: Decision,
        ids: Vec<RecordingId>,
        skipped: bool,
    ) -> Self {
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
        Self {
            site_id: header.site_id,
            row: header.row,
            ids,
            slots: vec![Slot::Pending; decision.effects.len()],
            decision,
            observed: header.observed.clone(),
            emit_ok: HashMap::new(),
            later_conds,
            later_cond_cursor: 0,
            later_mirrors,
            last_idx: [None; 4],
            reply: None,
            mismatched: false,
            skipped,
        }
    }

    /// CP1(i)/(ii) for one placement group: the effects, conditionals and mirrors
    /// the live path actually ran, against the decided ones. Returns every failing
    /// level; the caller reports the lowest `diff_kind`.
    fn check_group(
        &mut self,
        group: Placement,
        actual: &[WitnessedEffect],
        conditionals: &[CondWitness],
        mirrors: &[MirrorWitness],
    ) -> Vec<DiffKind> {
        let mut diffs = Vec::new();
        let idxs: Vec<usize> = self
            .decision
            .effects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.at == group)
            .map(|(i, _)| i)
            .collect();

        // Conditionals first: which effects the live path skipped decides which
        // effects are expected.
        let cond_idxs: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|&i| pred_of(cond_of(&self.decision.effects[i].effect)).is_some())
            .collect();
        let mut cond_diff = conditionals.len() != cond_idxs.len();
        let mut skipped = Vec::new();
        for (k, &i) in cond_idxs.iter().enumerate() {
            let want = pred_of(cond_of(&self.decision.effects[i].effect));
            if let Some(witness) = conditionals.get(k) {
                cond_diff |= Some(witness.pred) != want;
                if !witness.taken {
                    skipped.push(i);
                    self.slots[i] = Slot::Skipped;
                }
            }
        }
        let expected: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|i| !skipped.contains(i))
            .collect();

        // Effects: set, then order, then placement.
        let expected_effects: Vec<&Effect> = expected
            .iter()
            .map(|&i| &self.decision.effects[i].effect)
            .collect();
        let actual_effects: Vec<&Effect> = actual.iter().map(|w| &w.effect).collect();
        if expected_effects == actual_effects {
            for (k, &i) in expected.iter().enumerate() {
                self.slots[i] = Slot::Ran;
                if let super::types::EffectResult::Emitted { ok } = actual[k].result {
                    self.emit_ok.insert(i, ok);
                }
            }
        } else {
            let mut taken = vec![false; expected.len()];
            let mut unmatched_actual = Vec::new();
            for (k, effect) in actual_effects.iter().enumerate() {
                match (0..expected.len()).find(|&e| !taken[e] && expected_effects[e] == *effect) {
                    Some(e) => {
                        taken[e] = true;
                        self.slots[expected[e]] = Slot::Ran;
                        if let super::types::EffectResult::Emitted { ok } = actual[k].result {
                            self.emit_ok.insert(expected[e], ok);
                        }
                    }
                    None => unmatched_actual.push(k),
                }
            }
            let missing = taken.iter().any(|t| !t);
            // An effect that ran earlier than decided: it is decided, pending, in
            // a later group.
            let mut all_placed_elsewhere = !unmatched_actual.is_empty();
            for &k in &unmatched_actual {
                let elsewhere = self.decision.effects.iter().enumerate().position(|(j, p)| {
                    p.at != group
                        && self.slots[j] == Slot::Pending
                        && p.effect == *actual_effects[k]
                });
                match elsewhere {
                    Some(j) => self.slots[j] = Slot::Ran,
                    None => all_placed_elsewhere = false,
                }
            }
            diffs.push(if !missing && unmatched_actual.is_empty() {
                DiffKind::EffectOrder
            } else if !missing && all_placed_elsewhere {
                DiffKind::Placement
            } else {
                DiffKind::EffectSet
            });
        }

        // M4 mirrors of the CX effects run in this group (`pre_mirrors`,
        // `in_d_mirrors`): the live slot after each effect against the decided commit.
        let mirrored: Vec<usize> = expected
            .iter()
            .copied()
            .filter(|&i| {
                matches!(
                    self.decision.effects[i].effect,
                    Effect::ClearContext(_) | Effect::SetContext(_)
                )
            })
            .collect();
        let mirrors_ok = mirrored.len() == mirrors.len()
            && mirrored
                .iter()
                .zip(mirrors)
                .all(|(&i, mirror)| header_mirror_ok(&self.decision.effects[i].effect, mirror));
        if !mirrors_ok {
            diffs.push(DiffKind::PostState);
        }

        // Conditionals: the predicate id, then taken or skipped against what the
        // observation says it must be.
        if !cond_diff {
            for (k, &i) in cond_idxs.iter().enumerate() {
                let expected_taken = match cond_of(&self.decision.effects[i].effect) {
                    Cond::Always => None,
                    Cond::UnlessSpeculativeFlowing => spec_flowing(&self.observed).map(|f| !f),
                    Cond::EmitOk(paired) => self.emit_ok.get(&usize::from(paired)).copied(),
                };
                if expected_taken.is_some_and(|want| want != conditionals[k].taken) {
                    cond_diff = true;
                }
            }
        }
        if cond_diff {
            diffs.push(DiffKind::Conditional);
        }
        diffs
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
                    self.slots[j] = Slot::Ran;
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
            self.slots[idx] = Slot::Ran;
        }
        let last = &mut self.last_idx[placement_index(placement)];
        *last = Some(last.map_or(idx, |l| l.max(idx)));
        if let super::types::EffectResult::Emitted { ok } = witnessed.result {
            self.emit_ok.insert(idx, ok);
        }
        diff
    }

    /// CP2: a shell-conditional after D, as (predicate id, taken).
    fn on_cond(&mut self, pred: PredId, taken: bool) -> Option<DiffKind> {
        let Some(&idx) = self.later_conds.get(self.later_cond_cursor) else {
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
        let expected_taken = match cond {
            Cond::Always => None,
            Cond::UnlessSpeculativeFlowing => spec_flowing(&self.observed).map(|f| !f),
            Cond::EmitOk(paired) => self.emit_ok.get(&usize::from(paired)).copied(),
        };
        expected_taken
            .is_some_and(|want| want != taken)
            .then_some(DiffKind::Conditional)
    }

    /// CP3: a live M-field written at effect time, against the decided commit.
    fn on_mirror(&mut self, field: MirrorField, value: MirrorValue) -> Option<DiffKind> {
        if field == MirrorField::TickerRegistered {
            // No decided commit exists for the ticker registration: recorded for
            // the soak, never compared.
            return None;
        }
        let entry = self.later_mirrors.iter_mut().find(|(i, consumed)| {
            !*consumed && mirror_field_of(&self.decision.effects[*i].effect) == Some(field)
        });
        let Some((idx, consumed)) = entry else {
            return Some(DiffKind::PostState);
        };
        *consumed = true;
        (!later_mirror_ok(&self.decision.effects[*idx].effect, value))
            .then_some(DiffKind::PostState)
    }

    /// CP4: the flight closed.
    fn on_close(&mut self, completed: bool) -> Option<DiffKind> {
        let mut diffs = Vec::new();
        let expected_reply = self.decision.reply.as_ref().map(ReplyKind::of);
        if completed {
            // Every decided effect, conditional and mirror was witnessed exactly
            // once. (The persisted M1/M2 were committed under D and no later
            // effect writes them, so the post-state check at CP1 is final.)
            // A record that already mismatched left its unmatched effects pending
            // on purpose: reporting them again would be the same fault twice.
            if !self.mismatched && self.slots.contains(&Slot::Pending) {
                diffs.push(DiffKind::EffectSet);
            }
            if expected_reply != self.reply {
                diffs.push(DiffKind::Reply);
            }
            if self.later_mirrors.iter().any(|(_, consumed)| !consumed) {
                diffs.push(DiffKind::PostState);
            }
        } else if self.reply.is_some() && expected_reply != self.reply {
            // An abandoned flight may never have replied; one that did must agree.
            diffs.push(DiffKind::Reply);
        }
        diffs.into_iter().min()
    }
}

// ----------------------------------------------------------------- comparator

/// What the comparison does with each message. Owned by the shadow thread, or
/// driven on the calling thread in synchronous mode.
pub(crate) struct Comparator {
    core: ShadowCore,
    decider: Decider,
    sink: EventSink,
    clock: Clock,
    next_seq: u64,
    pending: BTreeMap<u64, ShadowHeader>,
    lost_seqs: BTreeSet<u64>,
    records: HashMap<RecorderId, Record>,
    held: HashMap<RecorderId, Vec<ShadowMsg>>,
    /// Open recorders (records and held ones), oldest first.
    open: VecDeque<RecorderId>,
    closed: VecDeque<RecorderId>,
    /// Ids a lost record could have touched: their decisions are skipped.
    lost_tainted: BTreeSet<u64>,
    /// The next applied header's ids are tainted too (a lost record sat before it).
    taint_next: bool,
    last_ids: Vec<RecordingId>,
    /// Root mismatch signatures by recording id (cascade control).
    signatures: BTreeMap<u64, Vec<(u16, u16, u8)>>,
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
            lost_tainted: BTreeSet::new(),
            taint_next: false,
            last_ids: Vec::new(),
            signatures: BTreeMap::new(),
            event_last_ms: HashMap::new(),
            events_sent: 0,
            stats: Stats::default(),
        }
    }

    /// The production comparator: `DictationCore::handle`, the tracing event and a
    /// monotonic clock.
    pub(crate) fn production() -> Self {
        let origin = std::time::Instant::now();
        Self::new(
            Box::new(|core, input, now| core.handle(input, now)),
            Box::new(super::emit_mismatch_event),
            Box::new(move || u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)),
        )
    }

    /// The messages the channel could not queue: counted, and every id the missing
    /// records could have touched is tainted.
    pub(crate) fn note_dropped(&mut self, count: u64) {
        if count > 0 {
            self.stats.dropped += count;
            self.taint_surrounding();
        }
    }

    /// `Pre` buffer overflows the live side counted.
    pub(crate) fn note_overflow(&mut self, count: u64) {
        self.stats.witness_overflow += count;
    }

    /// Apply one message. Panics in the shadow's own decide are caught per header.
    pub(crate) fn handle_msg(&mut self, msg: ShadowMsg) {
        match msg {
            ShadowMsg::Header(header) => self.on_header(*header),
            ShadowMsg::Lost { seq } => self.on_lost(seq),
            ShadowMsg::PreAbandoned { rec, .. } => {
                // Before D entry: no decision, no seq, no gap, no taint. Reported,
                // not a mismatch.
                self.stats.abandoned_pre_d += 1;
                self.forget(rec);
            }
            ShadowMsg::Witness { rec, .. }
            | ShadowMsg::Cond { rec, .. }
            | ShadowMsg::Mirror { rec, .. }
            | ShadowMsg::Reply { rec, .. }
            | ShadowMsg::Close { rec, .. } => self.route(rec, msg),
        }
    }

    // ------------------------------------------------------------- ordering

    fn on_header(&mut self, header: ShadowHeader) {
        let seq = header.seq;
        if seq < self.next_seq {
            // Arrived after its gap was declared lost. Applying it is safe (every
            // header resyncs M1/M2 from its own `pre`) and its ids are already
            // tainted by the loss.
            self.apply_header(header);
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
            self.stats.lost_gap += 1;
            self.taint_surrounding();
            self.next_seq += 1;
            self.drain_ready();
        }
    }

    fn on_lost(&mut self, seq: u64) {
        if seq < self.next_seq || !self.lost_seqs.insert(seq) {
            return;
        }
        // A header that will never arrive is a gap resolved early: counted as one.
        self.stats.lost_gap += 1;
        self.taint_surrounding();
        self.drain_ready();
    }

    fn drain_ready(&mut self) {
        loop {
            if self.lost_seqs.remove(&self.next_seq) {
                self.next_seq += 1;
            } else if let Some(header) = self.pending.remove(&self.next_seq) {
                self.apply_header(header);
                self.next_seq += 1;
            } else {
                break;
            }
        }
    }

    // ---------------------------------------------------------------- taint

    fn taint_lost(&mut self, ids: &[RecordingId]) {
        for id in ids {
            self.lost_tainted.insert(id.0);
        }
        while self.lost_tainted.len() > MAX_REMEMBERED_IDS {
            self.lost_tainted.pop_first();
        }
    }

    /// A record went missing: every id it could have touched is the ids named by
    /// the headers on either side of it.
    fn taint_surrounding(&mut self) {
        let before = self.last_ids.clone();
        self.taint_lost(&before);
        self.taint_next = true;
    }

    // -------------------------------------------------------------- headers

    fn apply_header(&mut self, header: ShadowHeader) {
        let ids = header_ids(&header);
        if std::mem::take(&mut self.taint_next) {
            self.taint_lost(&ids);
        }
        let skipped = ids.iter().any(|id| self.lost_tainted.contains(&id.0));
        self.last_ids.clone_from(&ids);
        let (site, row) = (header.site_id, header.row);

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
        let mut record = Record::new(&header, decision, ids.clone(), skipped);
        if !skipped {
            if let Some(diff) = Self::cp1(&mut record, &header, post) {
                record.mismatched = true;
                self.report(site, row, diff, &ids);
            } else {
                self.credit_pd(&record);
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
        let pre = record.check_group(
            Placement::Pre,
            &header.pre_effects,
            &header.pre_conditionals,
            &header.pre_mirrors,
        );
        let mut in_d = record.check_group(
            Placement::In,
            &header.in_d_effects,
            &header.in_d_conditionals,
            &header.in_d_mirrors,
        );
        if post != (header.live_post_d.status, header.live_post_d.current_id) {
            in_d.push(DiffKind::PostState);
        }
        let registered: Vec<&LiveOp> = header
            .live_ops
            .iter()
            .filter(|op| op.kind == LiveOpKind::Registered)
            .collect();
        let continuation_ok = match (record.decision.continuation, registered.as_slice()) {
            (None, []) => true,
            (Some(_), [_]) => replayed_token(&record.decision, &header.live_ops).is_some(),
            _ => false,
        };
        if !continuation_ok {
            in_d.push(DiffKind::Continuation);
        }
        pre.into_iter().min().or_else(|| in_d.into_iter().min())
    }

    fn credit_pd(&mut self, record: &Record) {
        for pd in pds_for_row(record.decision.row.0) {
            *self.stats.pd_witness.entry(pd).or_default() += 1;
            self.stats.row(record.site_id, record.row).pd_witness += 1;
        }
    }

    // ------------------------------------------------------------- messages

    fn route(&mut self, rec: RecorderId, msg: ShadowMsg) {
        if self.closed.contains(&rec) {
            // A late message of a flight already closed (or abandoned): the
            // comparison is over.
            return;
        }
        let Some(mut record) = self.records.remove(&rec) else {
            self.hold(rec, msg);
            return;
        };
        let (site, row) = (record.site_id, record.row);
        let ids = record.ids.clone();
        let mut closing = false;
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
                ShadowMsg::Mirror { field, value, .. } => record.on_mirror(field, value),
                ShadowMsg::Reply { kind, .. } => {
                    record.reply = Some(kind);
                    None
                }
                ShadowMsg::Close { completed, .. } => {
                    closing = true;
                    if !completed {
                        self.stats.abandoned += 1;
                    }
                    record.on_close(completed)
                }
                _ => None,
            }
        };
        if let Some(diff) = diff {
            record.mismatched = true;
            self.report(site, row, diff, &ids);
        }
        if closing {
            if !record.mismatched && !record.skipped {
                self.stats.matched += 1;
            }
            self.forget(rec);
        } else {
            self.records.insert(rec, record);
        }
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
    /// taints what it could have touched.
    fn evict_over_cap(&mut self) {
        while self.open.len() > MAX_OPEN_RECORDS {
            let Some(oldest) = self.open.pop_front() else {
                break;
            };
            self.stats.lost_open += 1;
            match self.records.remove(&oldest) {
                Some(record) => self.taint_lost(&record.ids),
                None => self.taint_surrounding(),
            }
            self.held.remove(&oldest);
            self.remember_closed(oldest);
        }
    }

    /// The recorder is finished with: forget its state and remember the id so a late
    /// message is dropped instead of held.
    fn forget(&mut self, rec: RecorderId) {
        self.records.remove(&rec);
        self.held.remove(&rec);
        self.open.retain(|id| *id != rec);
        self.remember_closed(rec);
    }

    fn remember_closed(&mut self, rec: RecorderId) {
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
