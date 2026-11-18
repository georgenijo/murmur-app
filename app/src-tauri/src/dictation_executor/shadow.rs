//! Live shadow capture (Stage 2c-2, appendix C8.2): the producer side of the
//! dictation shadow, wired into the shell's decision sites through the executor.
//!
//! **Compiled only with the `dictation-shadow` Cargo feature** (a `#[cfg]` on this
//! module's declaration), and active only when `MURMUR_SHADOW=1` armed the
//! transport: otherwise [`Flight::begin`] returns an empty flight and every method
//! here returns at its first line.
//!
//! One [`Flight`] is the capture of **one decision** (one `decide_*`, one `rec`).
//! The shell creates it **before `run(Pre)`**, keeps it with the decision's
//! [`Executor`](super::executor::Executor) (and in its
//! [`Suspended`](super::executor::Suspended) while the executor is parked across an
//! await) and never looks anything up by thread: **no thread-local is used for
//! correlation** (C8.2), because later phases of one decision run on other threads
//! and tasks.
//!
//! The lifecycle of a flight:
//!
//! 1. [`Flight::begin`] creates the recorder.
//! 2. `Pre` (and prefix) effects run through an executor that has the flight
//!    attached in *pre mode*; their witnesses are buffered inline (`PreCapture`).
//! 3. Under D (D-less entries: at decision time) the shell calls
//!    `Executor::attach_shadow`, which assembles the [`ShadowHeader`] from values the
//!    live path already holds (the input, the view, the observed facts, the history
//!    operations): **no core work happens here**. The `In` effects the executor runs
//!    are appended to the header.
//! 4. Just before D is released the shell calls `Executor::seal_shadow`, which takes
//!    the header's `seq` (still under D, so `seq` **is** D order) and reads the
//!    persisted M1/M2 as the `In` effects committed them. **The shell submits the
//!    sealed header after releasing D** ([`Sealed::submit`]), before the first
//!    blocking effect or any deferred drain.
//! 5. Every later effect is one incremental message sent as it runs, and the end of
//!    the decision sends `Reply` and `Close`. Messages produced before the header
//!    exists (a prefix that runs ahead of its decision) wait in a small inline queue
//!    and follow the header.
//!
//! **Nothing here allocates, locks or waits.** Every list is a fixed-capacity inline
//! array, the queue is the in-crate ring (`try_push`, one attempt), and the only
//! `RefCell`s are read and written by the one thread that holds the flight at that
//! moment.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};

use murmur_dictation_core::decision::RowId;
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::{Now, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::replies::Reply;

use crate::dictation_shadow::transport::{self, Mark, PreCapture, ShadowRecorder, ShadowTicket};
use crate::dictation_shadow::types::{
    CondWitness, EffectResult, Inline, LiveOp, MirrorField, MirrorWitness, ObservedFact, PredId,
    Projection, ReplyKind, ShadowHeader, TickerGate, TickerSkipReason, WitnessedEffect,
    OBSERVED_CAPACITY, OPS_CAPACITY,
};

/// Messages a flight can hold until its header is submitted: a prefix that runs ahead
/// of its decision (`ACC`, `PCF`) and the end of a decision that finished under D.
const DEFERRED_CAPACITY: usize = 8;

/// One message that must follow the header.
#[derive(Clone, Debug)]
enum Later {
    Witness {
        idx: u16,
        placement: Placement,
        effect: WitnessedEffect,
    },
    Mark(Mark),
}

/// What the executor tells the flight about one effect it dealt with.
pub(crate) struct Noted<'a> {
    /// The effect's index in the executor's own decision.
    pub index: usize,
    /// The placement the decision gave it.
    pub placement: Placement,
    pub effect: &'a Effect,
    /// `false` when a shell predicate skipped it (a conditional that was not taken).
    pub taken: bool,
    pub result: EffectResult,
    /// The live-field write the effect made, before and after.
    pub mirror: Option<MirrorWitness>,
    /// The ticker registration attempt an `SD` made.
    pub ticker: Option<TickerNote>,
}

/// What an `SD` that ran tells the flight about its ticker (D-S2c-6): the gate it
/// asked, or why its spawner returned before asking. Never both.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TickerNote {
    Gate(TickerGate),
    /// `target` is the recording the spawner was **asked** to start (the id it
    /// looked up), `reason` why it returned before the gate.
    Skipped {
        target: RecordingId,
        reason: TickerSkipReason,
    },
}

/// The reason a spawner returned before the gate, packed with its recording id
/// (`id << 2 | reason`; `0` is none). The spawner and the executor that ran its `SD`
/// can be on different threads (the lifecycle bridge delivers on its own), so the
/// correlation is by recording id, in one atomic: no lock, no thread-local.
static TICKER_SKIP: AtomicU64 = AtomicU64::new(0);

/// A `StartSideTasks` spawner returned before asking the gate. `recording_id` is the id
/// the spawner was asked to start.
pub(crate) fn note_ticker_skip(recording_id: u64, reason: TickerSkipReason) {
    TICKER_SKIP.store(
        (recording_id << 2) | u64::from(reason.code()),
        Ordering::Release,
    );
}

/// The skip the last spawner noted, with the id **it** was asked to start (not the id
/// the caller expects: the comparator checks them against each other). Clears the slot.
pub(crate) fn take_ticker_skip() -> Option<TickerNote> {
    let packed = TICKER_SKIP.swap(0, Ordering::AcqRel);
    if packed == 0 {
        return None;
    }
    let reason = TickerSkipReason::ALL
        .into_iter()
        .find(|reason| u64::from(reason.code()) == packed & 0b11)?;
    Some(TickerNote::Skipped {
        target: RecordingId(packed >> 2),
        reason,
    })
}

struct Live {
    rec: ShadowRecorder,
    /// `Pre` witnesses buffered until the header exists.
    pre: PreCapture,
    /// Effect indices consumed by earlier executors of this flight (the dropped
    /// prefix of `after_pre` / `after_prefix`): a witness keeps the index it has in
    /// the **full** decided list (C8.2).
    base: u16,
    /// History operations and observed facts recorded before the header exists.
    ops: Inline<LiveOp, OPS_CAPACITY>,
    observed: Inline<ObservedFact, OBSERVED_CAPACITY>,
    ops_overflowed: bool,
    header: RefCell<Option<ShadowHeader>>,
    submitted: Cell<bool>,
    deferred: RefCell<Inline<Later, DEFERRED_CAPACITY>>,
    reply: Cell<Option<ReplyKind>>,
    /// Every effect of the decision ran: `Reply` and `Close` follow the header.
    complete: Cell<bool>,
    concluded: Cell<bool>,
    /// A leading executor (the `Pre` effects, a prefix) holds the flight: its last
    /// effect does not end the decision, which has not been taken yet.
    leading: Cell<bool>,
}

impl Live {
    fn new(rec: ShadowRecorder) -> Self {
        Self {
            rec,
            pre: PreCapture::default(),
            base: 0,
            ops: Inline::default(),
            observed: Inline::default(),
            ops_overflowed: false,
            header: RefCell::new(None),
            submitted: Cell::new(false),
            deferred: RefCell::new(Inline::default()),
            reply: Cell::new(None),
            complete: Cell::new(false),
            concluded: Cell::new(false),
            leading: Cell::new(false),
        }
    }

    /// One message that follows the header: sent now if the header is out, queued if
    /// not. A full queue marks the (unsent) header incomplete: counted, never guessed.
    fn later(&self, message: Later) {
        if self.submitted.get() {
            self.send(message);
            return;
        }
        let stored = self.deferred.borrow_mut().push(message);
        if !stored {
            if let Some(header) = self.header.borrow_mut().as_mut() {
                header.overflowed = true;
            }
        }
    }

    fn send(&self, message: Later) {
        match message {
            Later::Witness {
                idx,
                placement,
                effect,
            } => transport::witness(&self.rec, idx, placement, effect),
            Later::Mark(mark) => transport::mark(&self.rec, mark),
        }
    }

    /// `Reply`, then `Close{completed:true}`, once, and only after the header.
    fn conclude(&self) {
        if !self.submitted.get() || !self.complete.get() || self.concluded.replace(true) {
            return;
        }
        if let Some(kind) = self.reply.get() {
            transport::mark(&self.rec, Mark::Reply(kind));
        }
        self.rec.finish();
    }
}

/// The capture of one decision. Empty (and free) unless shadow mode is active.
pub(crate) struct Flight {
    live: Option<Live>,
}

impl Flight {
    /// No capture.
    pub(crate) const fn none() -> Self {
        Self { live: None }
    }

    /// The flight's recorder, created **before `run(Pre)`** (C4.1 step 2). Empty
    /// unless shadow mode is active.
    pub(crate) fn begin() -> Self {
        Self {
            live: crate::dictation_shadow::recorder().map(Live::new),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.live.is_some()
    }

    /// `run(Pre)` begins: a flight dropped from here until its decision was taken is
    /// reported as `PreAbandoned`.
    pub(crate) fn enter_pre(&mut self) {
        if let Some(live) = &self.live {
            live.rec.enter_pre();
            live.leading.set(true);
        }
    }

    /// The executor that ran a leading part of this decision (its `Pre` effects, or a
    /// prefix) hands the flight on: the effects it consumed keep their indices.
    pub(crate) fn advance(&mut self, consumed: usize) {
        if let Some(live) = &mut self.live {
            live.base = live
                .base
                .saturating_add(u16::try_from(consumed).unwrap_or(u16::MAX));
        }
    }

    /// A value the live path read for this decision (C8.2 `observed`).
    pub(crate) fn observed(&mut self, fact: ObservedFact) {
        let Some(live) = &mut self.live else {
            return;
        };
        if let Some(header) = live.header.get_mut().as_mut() {
            header.add_observed(fact);
        } else if !live.observed.push(fact) {
            live.ops_overflowed = true;
        }
    }

    /// An H1 / H2 operation the shell performed for this decision, in order. Before
    /// the header exists it is buffered; before the header is submitted it joins
    /// `live_ops`; after, it is its own message (an owner's `Drop`, a deferred
    /// retirement).
    pub(crate) fn history(&mut self, op: LiveOp) {
        let Some(live) = &mut self.live else {
            return;
        };
        if live.submitted.get() {
            transport::mark(&live.rec, Mark::History(op));
        } else if let Some(header) = live.header.get_mut().as_mut() {
            header.add_op(op);
        } else if !live.ops.push(op) {
            live.ops_overflowed = true;
        }
    }

    /// The decision was taken: assemble the header from values the live path holds.
    /// Called under D (D-less entries: at decision time); does **no core work**.
    pub(crate) fn decided(
        &mut self,
        input: Input,
        pre: Projection,
        now: Now,
        row: RowId,
        reply: Option<&Reply>,
        effects: usize,
    ) {
        let Some(live) = &mut self.live else {
            return;
        };
        let mut header = ShadowHeader::new(
            0,
            live.rec.id(),
            input.site().code(),
            row.ordinal(),
            pre,
            input,
            now,
        );
        for fact in live.observed.iter() {
            header.add_observed(*fact);
        }
        for op in live.ops.iter() {
            header.add_op(*op);
        }
        live.observed.clear();
        live.ops.clear();
        header.overflowed |= live.ops_overflowed;
        header.adopt_pre(std::mem::take(&mut live.pre));
        live.reply.set(reply.map(ReplyKind::of));
        live.leading.set(false);
        *live.header.get_mut() = Some(header);
        if effects == 0 {
            live.complete.set(true);
        }
    }

    /// An effect ran (or a conditional was skipped): record it where its placement
    /// says (C8.2): `Pre` and `In` into the header's lists, a later placement as its
    /// own incremental message.
    pub(crate) fn effect(&mut self, noted: Noted<'_>) {
        let Some(live) = &mut self.live else {
            return;
        };
        let idx = u16::try_from(usize::from(live.base) + noted.index).unwrap_or(u16::MAX);
        let pred = pred_of(noted.effect, live.base);
        let witnessed = WitnessedEffect {
            effect: in_full_list(noted.effect, live.base),
            result: noted.result,
        };
        match noted.placement {
            Placement::Pre | Placement::In => {
                let in_d = noted.placement == Placement::In;
                let header = live.header.get_mut();
                let mut stored = true;
                match header.as_mut() {
                    Some(header) => {
                        if let Some(pred) = pred {
                            let witness = CondWitness {
                                pred,
                                taken: noted.taken,
                            };
                            if in_d {
                                header.add_in_conditional(witness);
                            } else {
                                stored &= header.pre_conditionals.push(witness);
                            }
                        }
                        if noted.taken {
                            if in_d {
                                header.add_in_effect(witnessed);
                            } else {
                                stored &= header.pre_effects.push(witnessed);
                            }
                            if let Some(mirror) = noted.mirror {
                                if mirror.field == MirrorField::ActiveContext {
                                    if in_d {
                                        header.add_in_mirror(mirror);
                                    } else {
                                        stored &= header.pre_mirrors.push(mirror);
                                    }
                                }
                            }
                            if let (true, Some(TickerNote::Gate(gate))) = (in_d, noted.ticker) {
                                header.add_in_ticker(gate);
                            }
                        }
                        if !stored {
                            header.overflowed = true;
                        }
                    }
                    None => {
                        // The decision has not been taken yet: a `Pre` effect of the
                        // leading executor buffers inline. An `In` effect without a
                        // header means the site did not attach the decision: that is
                        // a wiring defect, caught by the site tests.
                        debug_assert!(!in_d, "an `In` effect ran with no shadow header");
                        if !in_d {
                            if let Some(pred) = pred {
                                stored &= live.pre.conditionals.push(CondWitness {
                                    pred,
                                    taken: noted.taken,
                                });
                            }
                            if noted.taken {
                                stored &= live.pre.effects.push(witnessed);
                                if let Some(mirror) = noted.mirror {
                                    if mirror.field == MirrorField::ActiveContext {
                                        stored &= live.pre.mirrors.push(mirror);
                                    }
                                }
                            }
                        }
                        if !stored {
                            live.pre.overflowed = true;
                        }
                    }
                }
            }
            Placement::PostDT | Placement::PostT => {
                if let Some(pred) = pred {
                    live.later(Later::Mark(Mark::Cond {
                        pred,
                        taken: noted.taken,
                    }));
                }
                if noted.taken {
                    live.later(Later::Witness {
                        idx,
                        placement: noted.placement,
                        effect: witnessed,
                    });
                    if let Some(mirror) = noted.mirror {
                        live.later(Later::Mark(Mark::Mirror(mirror)));
                    }
                    match noted.ticker {
                        Some(TickerNote::Gate(gate)) => {
                            live.later(Later::Mark(Mark::Ticker(gate)));
                        }
                        Some(TickerNote::Skipped { reason, .. }) => {
                            live.later(Later::Mark(Mark::TickerSkipped { reason }));
                        }
                        None => {}
                    }
                }
            }
        }
    }

    /// Every effect of the decision ran. `Reply` and `Close{completed:true}` follow
    /// the header (now, or when it is submitted).
    pub(crate) fn finish(&self) {
        if let Some(live) = &self.live {
            if live.leading.get() {
                return;
            }
            live.complete.set(true);
            live.conclude();
        }
    }

    /// Take the header's `seq` (still under D) and fix `live_post_d`: persisted M1/M2
    /// as the `In` effects committed them. The caller then releases D and calls
    /// [`Sealed::submit`]. Dropping a [`Sealed`] without submitting reports the loss.
    pub(crate) fn seal(&self, post: Option<Projection>) -> Sealed<'_> {
        let Some(live) = &self.live else {
            return Sealed { inner: None };
        };
        let Some(mut header) = live.header.borrow_mut().take() else {
            return Sealed { inner: None };
        };
        if let Some(post) = post {
            header.live_post_d = post;
        }
        let ticket = live.rec.ticket(header.site_id);
        header.seq = ticket.seq();
        Sealed {
            inner: Some((live, ticket, header)),
        }
    }

    /// Whether the header went out (tests).
    #[cfg(test)]
    pub(crate) fn is_submitted(&self) -> bool {
        self.live.as_ref().is_some_and(|live| live.submitted.get())
    }
}

/// The effect as the **full** decided list names it: an `EmitOk(i)` pairing is an index
/// into the executor's own (possibly shortened) list, so a dropped prefix shifts it.
fn in_full_list(effect: &Effect, base: u16) -> Effect {
    let shifted = |cond: Cond| match cond {
        Cond::EmitOk(paired) => {
            Cond::EmitOk(paired.saturating_add(u8::try_from(base).unwrap_or(u8::MAX)))
        }
        other => other,
    };
    match effect {
        Effect::EmitStatus { status, cond } => Effect::EmitStatus {
            status: *status,
            cond: shifted(*cond),
        },
        Effect::Presentation { kind, cond } => Effect::Presentation {
            kind: *kind,
            cond: shifted(*cond),
        },
        other => other.clone(),
    }
}

fn pred_of(effect: &Effect, base: u16) -> Option<PredId> {
    let cond = match effect {
        Effect::EmitStatus { cond, .. } | Effect::Presentation { cond, .. } => *cond,
        _ => return None,
    };
    match cond {
        Cond::Always => None,
        Cond::UnlessSpeculativeFlowing => Some(PredId::SpecFlowing),
        // `EmitOk(i)` names an effect by its index in the executor's decision; the
        // shadow decides the full list.
        Cond::EmitOk(paired) => Some(PredId::EmitOk(
            paired.saturating_add(u8::try_from(base).unwrap_or(u8::MAX)),
        )),
    }
}

/// A header with its `seq` taken (under D), waiting for the shell to release D.
pub(crate) struct Sealed<'a> {
    inner: Option<(&'a Live, ShadowTicket<'a>, ShadowHeader)>,
}

impl Sealed<'_> {
    /// Submit the header: **call it right after D is released**, before any blocking
    /// effect or deferred drain. One `try_push`; never blocks. Then the messages that
    /// waited for it, in order, and the end of the decision if it already ran.
    pub(crate) fn submit(self) {
        let Some((live, ticket, header)) = self.inner else {
            return;
        };
        // Test builds: a point where a test observes whether D is still held.
        #[cfg(test)]
        crate::barrier_hook::at("shadow.submit");
        transport::submit(ticket, header);
        live.submitted.set(true);
        let queued = std::mem::take(&mut *live.deferred.borrow_mut());
        for message in queued.iter() {
            live.send(message.clone());
        }
        live.conclude();
    }
}

// ------------------------------------------------------------------ helpers

/// The projection of a decision view (persisted M1/M2 and the M3/M4 observations).
pub(crate) fn view_projection(view: &murmur_dictation_core::state::DictationCore) -> Projection {
    Projection {
        status: view.status,
        current_id: view.current_id,
        obs: murmur_dictation_core::state::LiveObs {
            cancelled_through: view.cancelled_through,
            context_for: view.context_for,
        },
    }
}

/// The projection of a D-less entry: it reads only M2 and M3 (`LockFreeFacts`), never
/// M1 or M4, so the status is the default and the decision cannot depend on it.
pub(crate) fn facts_projection(facts: murmur_dictation_core::state::LockFreeFacts) -> Projection {
    Projection {
        status: murmur_dictation_core::status::Status::default(),
        current_id: facts.current_id,
        obs: murmur_dictation_core::state::LiveObs {
            cancelled_through: facts.cancelled_through,
            context_for: None,
        },
    }
}

/// An H1 registration: the token the shell drew and registered for a continuation.
pub(crate) fn registered(token: murmur_dictation_core::decision::Continuation) -> LiveOp {
    use crate::dictation_shadow::types::{HistoryAction, HistoryTable};
    LiveOp {
        table: HistoryTable::H1,
        action: HistoryAction::Registered,
        cont: token.kind,
        recording_id: token.recording_id,
        op: token.op,
    }
}

/// An H1 retirement (`ret_if_matches`) of `token`: matched when the entry was removed.
pub(crate) fn retired(
    token: murmur_dictation_core::decision::Continuation,
    outcome: crate::core_history::Retired,
) -> LiveOp {
    use crate::dictation_shadow::types::{HistoryAction, HistoryTable};
    LiveOp {
        table: HistoryTable::H1,
        action: HistoryAction::Retired {
            matched: outcome == crate::core_history::Retired::Removed,
        },
        cont: token.kind,
        recording_id: token.recording_id,
        op: token.op,
    }
}
