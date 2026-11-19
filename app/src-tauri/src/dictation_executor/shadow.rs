//! Live shadow capture (Stage 2c-2, appendix C8.2): the producer side of the
//! dictation shadow, wired into the shell's decision sites.
//!
//! **Compiled only with the `dictation-shadow` Cargo feature** (a `#[cfg]` on this
//! module's declaration), and active only when `MURMUR_SHADOW=1` armed the
//! transport: otherwise [`Flight::begin`] returns an empty flight and every method
//! here returns at its first line.
//!
//! **Insertion only (D-S2c-8 a).** The shell's live statements are exactly main's; the
//! capture is gated lines inserted between them. The shell keeps one [`Flight`] per
//! decision in a local, opens a [`Scope`] around each synchronous section that runs an
//! executor (the executor reads the flight the scope names, so no executor is rebuilt
//! or rebound), and releases D exactly where the live code already does: a header is
//! sealed under D and submitted by [`AfterD`], right after the existing D release.
//!
//! The lifecycle of a flight:
//!
//! 1. [`Flight::begin`] creates the recorder (before `run(Pre)`, C4.1 step 2).
//! 2. `Pre` (and prefix) effects run under a scope; their witnesses are buffered inline
//!    ([`PreCapture`]) until the header exists.
//! 3. Under D (D-less entries: at decision time) the shell calls [`Flight::decided`]:
//!    the header is assembled from values the live path already holds; **no core work
//!    happens here**. The `In` effects the executor runs are appended to it.
//! 4. Just before D is released the shell calls [`Flight::seal_d`] (the header's `seq`
//!    is taken still under D, so `seq` **is** D order) and hands the [`Sealed`] header
//!    to [`AfterD`]; after the live code's own D release, [`AfterD::submit`] sends it,
//!    before the first blocking effect or deferred drain.
//! 5. Every later effect is one incremental message sent as it runs. A decision with a
//!    reply concludes when the command returns: the flight is handed to the
//!    command's [`commanded`] wrapper, which witnesses the reply the command actually
//!    returned (CP4, D-S2c-8 f).
//!
//! **Thread-locals** are used only inside one synchronous section, behind a
//! [`Scope`] (which is `!Send`, so a section cannot hold it across an `.await`): the
//! flight the executor records into, and the live-field/ticker values a write stashes
//! for the effect that made it. A task spawned by a section never sees them: it
//! carries its flight in a moved local. The reply sink is a tokio task-local scoped
//! by the command wrapper, one per invocation.
//!
//! **Nothing here allocates, locks or waits on the capture path.** Every list is a
//! fixed-capacity inline array, the queue is the in-crate ring (`try_push`, one
//! attempt), and the only `RefCell`s are read and written by the one thread that holds
//! the flight at that moment.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};

use murmur_dictation_core::decision::RowId;
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::{Now, RecordingId};
use murmur_dictation_core::inputs::Input;
use murmur_dictation_core::replies::Reply;

use crate::dictation_shadow::transport::{self, Mark, PreCapture, ShadowRecorder, ShadowTicket};
use crate::dictation_shadow::types::{
    CondWitness, EffectResult, Inline, LiveOp, MirrorWitness, ObservedFact, PredId, Projection,
    ReplyKind, ShadowHeader, TickerGate, TickerSkipReason, WitnessedEffect, OBSERVED_CAPACITY,
    OPS_CAPACITY,
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

// ------------------------------------------------- entry/header conservation
//
// D-S2c-8 c: the live shell counts every decision section it enters (where it acquires
// D, or, for a D-less entry, where it decides) while the shadow is armed, and the
// flights count the headers they submit. One relaxed increment each, no lock, no
// allocation. E7's synchronous replay asserts, per sub-run, that the entries equal the
// headers, plus the explicit no-decision outcomes (each one named and counted), plus the
// flights an unwind ended: an entry that returns without a capture fails closed,
// whichever branch it is.

static ENTRIES: AtomicU64 = AtomicU64::new(0);
static HEADERS: AtomicU64 = AtomicU64::new(0);
static NO_DECISION: AtomicU64 = AtomicU64::new(0);
/// Decisions whose flight was dropped by an unwind after the decision and before its
/// header: recorded as one loss each (D-S2c-7 e), so they balance the books.
static UNWOUND: AtomicU64 = AtomicU64::new(0);

/// A D-less family entry took a decision section (the decision point; there is no D
/// guard to count it in). Counted only while the shadow is armed.
pub(crate) fn entry() {
    if crate::dictation_shadow::enabled() {
        ENTRIES.fetch_add(1, Ordering::Relaxed);
    }
}

/// The dictation lock D was acquired: counted **inside the guard function**
/// (`MutexExt::lock_or_recover`), so a site cannot take D without being counted, and
/// whatever a site does or leaves out after it cannot take the count with it (D-S2c-9 a).
/// Test builds exclude test code and the named non-family acquisitions
/// (`replay::NON_FAMILY`). A disarmed shadow returns after one constant comparison and
/// one atomic load: no read of D, no allocation.
pub(crate) fn dictation_lock_taken<T>(at: &'static std::panic::Location<'static>) {
    if std::any::type_name::<T>() != std::any::type_name::<crate::state::DictationState>() {
        return;
    }
    if !crate::dictation_shadow::enabled() {
        return;
    }
    #[cfg(test)]
    if crate::dictation_shadow::replay::not_a_decision_section(at) {
        return;
    }
    #[cfg(not(test))]
    let _ = at;
    ENTRIES.fetch_add(1, Ordering::Relaxed);
}

/// A decision section ended with no decision to compare (an observation round that
/// the section decides again, a refusal before any decision): named at the site, never
/// implied.
pub(crate) fn no_decision() {
    if crate::dictation_shadow::enabled() {
        NO_DECISION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Test builds: completed flights whose reply no command return witnessed (a flight
/// that concluded with the kind it decided because no sink was there).
#[cfg(test)]
static UNRENDERED: AtomicU64 = AtomicU64::new(0);

/// Test builds: flights that ended as the **named no-reply close** (a deferred finish
/// with no command return of its own, or the import's own pipeline error). The comparator
/// sees each as `Close{completed:false}`; the gate accepts exactly this many and no other
/// flight that never completed (D-S2c-9 b).
#[cfg(test)]
static NO_REPLY: AtomicU64 = AtomicU64::new(0);

/// Test builds: the counters so far; a replay's report takes the difference between
/// its end and its start.
#[cfg(test)]
pub(crate) fn conservation_now() -> crate::dictation_shadow::replay::Conservation {
    crate::dictation_shadow::replay::Conservation {
        entries: ENTRIES.load(Ordering::Relaxed),
        headers: HEADERS.load(Ordering::Relaxed),
        no_decision: NO_DECISION.load(Ordering::Relaxed),
        unrendered: UNRENDERED.load(Ordering::Relaxed),
        no_reply: NO_REPLY.load(Ordering::Relaxed),
        unwound: UNWOUND.load(Ordering::Relaxed),
    }
}

struct Live {
    rec: ShadowRecorder,
    /// `Pre` witnesses buffered until the header exists.
    pre: RefCell<PreCapture>,
    /// Effect indices consumed by earlier executors of this flight (the dropped
    /// prefix of `after_pre` / `after_prefix`): a witness keeps the index it has in
    /// the **full** decided list (C8.2).
    base: Cell<u16>,
    /// History operations and observed facts recorded before the header exists.
    ops: RefCell<Inline<LiveOp, OPS_CAPACITY>>,
    observed: RefCell<Inline<ObservedFact, OBSERVED_CAPACITY>>,
    ops_overflowed: Cell<bool>,
    header: RefCell<Option<ShadowHeader>>,
    submitted: Cell<bool>,
    deferred: RefCell<Inline<Later, DEFERRED_CAPACITY>>,
    /// The reply kind the command **returned** (D-S2c-8 f), not the decided one.
    reply: Cell<Option<ReplyKind>>,
    /// The decision has a reply: `Reply` and `Close` wait for the returned kind.
    expects_reply: Cell<bool>,
    /// The command's reply is another site's to witness (see
    /// [`Flight::delegated_reply`]): this flight carries no reply disposition of its own.
    delegated: Cell<bool>,
    /// The decision models a success the command did not return: the import's pipeline
    /// failed, so the command returns `Err` (see [`import_pipeline_error`]). The actual
    /// return is what it is compared against.
    err_outcome: Cell<bool>,
    /// The decision was taken (D is held, or the entry is D-less) and its header has
    /// not been sealed: an unwind here is a recorded loss (D-S2c-7 e).
    in_decision: Cell<bool>,
    /// Every effect of the decision ran: `Reply` and `Close` follow the header.
    complete: Cell<bool>,
    concluded: Cell<bool>,
    /// A leading executor (the `Pre` effects, a prefix) holds the flight: its last
    /// effect does not end the decision, which has not been taken yet.
    leading: Cell<bool>,
    /// The slots the executor driving this flight writes the effect's witnesses into.
    slots: ShadowSlots,
}

impl Live {
    fn new(rec: ShadowRecorder) -> Self {
        Self {
            rec,
            pre: RefCell::new(PreCapture::default()),
            base: Cell::new(0),
            ops: RefCell::new(Inline::default()),
            observed: RefCell::new(Inline::default()),
            ops_overflowed: Cell::new(false),
            header: RefCell::new(None),
            submitted: Cell::new(false),
            deferred: RefCell::new(Inline::default()),
            reply: Cell::new(None),
            expects_reply: Cell::new(false),
            delegated: Cell::new(false),
            err_outcome: Cell::new(false),
            in_decision: Cell::new(false),
            complete: Cell::new(false),
            concluded: Cell::new(false),
            leading: Cell::new(false),
            slots: ShadowSlots::default(),
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
        if !self.submitted.get() || !self.complete.get() {
            return;
        }
        // A decision with a reply concludes when the command has returned it.
        if self.expects_reply.get() && self.reply.get().is_none() {
            return;
        }
        if self.concluded.replace(true) {
            return;
        }
        if let Some(kind) = self.reply.get() {
            transport::mark(&self.rec, Mark::Reply(kind));
        }
        self.rec.finish();
    }

    /// The flight's owner is done with it. A decision still waiting for its reply goes
    /// to the command wrapper's sink, to be concluded with the reply the command
    /// returns. A reply that is another site's (R201.4 / R205.4, see
    /// [`Flight::delegated_reply`]) and a finish a queued drain ran (see [`unreplied`]) have
    /// no reply of their own: each closes as the **named no-reply close** (incomplete, no
    /// `Reply` credited), never with a kind the shell decided (D-S2c-9 d, D-S2c-10 R5, R6).
    fn hand_over(self) {
        let waiting = self.complete.get()
            && self.submitted.get()
            && !self.concluded.get()
            && self.expects_reply.get()
            && self.reply.get().is_none();
        if !waiting {
            return;
        }
        if self.delegated.get() {
            #[cfg(test)]
            NO_REPLY.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let live = RefCell::new(Some(self));
        let handed = REPLY_SINK
            .try_with(|sink| {
                let Some(live) = live.borrow_mut().take() else {
                    return;
                };
                if sink.replyless {
                    #[cfg(test)]
                    NO_REPLY.fetch_add(1, Ordering::Relaxed);
                    drop(live);
                } else {
                    sink.flights.borrow_mut().push(live);
                }
            })
            .is_ok();
        if !handed {
            // No command return to witness it: the flight closes incomplete, with no
            // reply credited (counted for the tests, which require none).
            #[cfg(test)]
            UNRENDERED.fetch_add(1, Ordering::Relaxed);
            drop(live);
        }
    }
}

impl Drop for Live {
    /// A flight dropped after its decision was taken and before its header was
    /// sealed (an effect unwound under D): the ticket is taken and dropped unsent, which
    /// records the loss and keeps the recorder from reporting `PreAbandoned` for a
    /// decision that entered D (D-S2c-7 e).
    fn drop(&mut self) {
        if self.in_decision.get() {
            let site = self.header.get_mut().as_ref().map_or(0, |h| h.site_id);
            drop(self.rec.ticket(site));
            UNWOUND.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The capture of one decision. Empty (and free) unless shadow mode is active.
pub(crate) struct Flight {
    live: Option<Live>,
}

impl Drop for Flight {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            live.hand_over();
        }
    }
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
    pub(crate) fn enter_pre(&self) {
        if let Some(live) = &self.live {
            live.rec.enter_pre();
            live.leading.set(true);
        }
    }

    /// The effects a leading executor consumed keep their indices in the full list.
    fn advance(&self, consumed: usize) {
        if let Some(live) = &self.live {
            live.base.set(
                live.base
                    .get()
                    .saturating_add(u16::try_from(consumed).unwrap_or(u16::MAX)),
            );
        }
    }

    /// A value the live path read for this decision (C8.2 `observed`).
    pub(crate) fn observed(&self, fact: ObservedFact) {
        let Some(live) = &self.live else {
            return;
        };
        if let Some(header) = live.header.borrow_mut().as_mut() {
            header.add_observed(fact);
        } else if !live.observed.borrow_mut().push(fact) {
            live.ops_overflowed.set(true);
        }
    }

    /// An H1 / H2 operation the shell performed for this decision, in order. Before
    /// the header exists it is buffered; before the header is submitted it joins
    /// `live_ops`; after, it is its own message (an owner's `Drop`, a deferred
    /// retirement).
    pub(crate) fn history_of(&self, op: impl FnOnce() -> Option<LiveOp>) {
        // The check comes first; the op is built, and the live path's guard asked for it,
        // only for an active flight (D-S2c-10 amendment 1 i).
        if self.live.is_some() {
            if let Some(op) = op() {
                self.history(op);
            }
        }
    }

    /// [`Flight::history_of`] for an op that is already built.
    pub(crate) fn history(&self, op: LiveOp) {
        let Some(live) = &self.live else {
            return;
        };
        if live.submitted.get() {
            transport::mark(&live.rec, Mark::History(op));
        } else if let Some(header) = live.header.borrow_mut().as_mut() {
            header.add_op(op);
        } else if !live.ops.borrow_mut().push(op) {
            live.ops_overflowed.set(true);
        }
    }

    /// The decision was taken: assemble the header from values the live path holds.
    /// Called under D (D-less entries: at decision time); does **no core work**.
    pub(crate) fn decided(
        &self,
        input: Input,
        pre: Projection,
        now: Now,
        row: RowId,
        reply: Option<&Reply>,
        effects: usize,
    ) {
        let Some(live) = &self.live else {
            return;
        };
        let mut header = ShadowHeader::new(
            0,
            live.rec.id(),
            input.site().code(),
            row.ordinal_inline(),
            pre,
            input,
            now,
        );
        for fact in live.observed.borrow().iter() {
            header.add_observed(*fact);
        }
        for op in live.ops.borrow().iter() {
            header.add_op(*op);
        }
        live.observed.borrow_mut().clear();
        live.ops.borrow_mut().clear();
        header.overflowed |= live.ops_overflowed.get();
        header.adopt_pre(std::mem::take(&mut *live.pre.borrow_mut()));
        live.expects_reply.set(reply.is_some());
        live.in_decision.set(true);
        live.leading.set(false);
        *live.header.borrow_mut() = Some(header);
        if effects == 0 {
            live.complete.set(true);
        }
    }

    /// An effect ran (or a conditional was skipped): record it where its placement
    /// says (C8.2): `Pre` and `In` into the header's lists, a later placement as its
    /// own incremental message.
    pub(crate) fn effect(&self, noted: Noted<'_>) {
        let Some(live) = &self.live else {
            return;
        };
        let base = live.base.get();
        let idx = u16::try_from(usize::from(base) + noted.index).unwrap_or(u16::MAX);
        let pred = pred_of(noted.effect, base);
        let witnessed = WitnessedEffect {
            effect: in_full_list(noted.effect, base),
            result: noted.result,
        };
        match noted.placement {
            Placement::Pre | Placement::In => {
                let in_d = noted.placement == Placement::In;
                let mut header = live.header.borrow_mut();
                let mut stored = true;
                let mut pending_skip = None;
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
                                if in_d {
                                    header.add_in_mirror(mirror);
                                } else {
                                    stored &= header.pre_mirrors.push(mirror);
                                }
                            }
                            if in_d {
                                match noted.ticker {
                                    Some(TickerNote::Gate(gate)) => header.add_in_ticker(gate),
                                    // A skip under D cannot be in the header: it follows it.
                                    Some(TickerNote::Skipped { target, reason }) => {
                                        pending_skip = Some((target, reason));
                                    }
                                    None => {}
                                }
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
                            let mut pre = live.pre.borrow_mut();
                            if let Some(pred) = pred {
                                stored &= pre.conditionals.push(CondWitness {
                                    pred,
                                    taken: noted.taken,
                                });
                            }
                            if noted.taken {
                                stored &= pre.effects.push(witnessed);
                                if let Some(mirror) = noted.mirror {
                                    stored &= pre.mirrors.push(mirror);
                                }
                            }
                            if !stored {
                                pre.overflowed = true;
                            }
                        }
                    }
                }
                drop(header);
                if let Some((target, reason)) = pending_skip {
                    live.later(Later::Mark(Mark::TickerSkipped { target, reason }));
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
                        Some(TickerNote::Skipped { target, reason }) => {
                            live.later(Later::Mark(Mark::TickerSkipped { target, reason }));
                        }
                        None => {}
                    }
                }
            }
        }
    }

    /// The reply of this decision is another site's to return (R201.4 and R205.4
    /// decide a reply the command does not return at that point: the capture cancel's
    /// own result is site 206's, whose flight is witnessed for the command's actual
    /// reply). The decision's own reply is concluded with the kind it decided.
    pub(crate) fn delegated_reply(&self) {
        if let Some(live) = &self.live {
            live.delegated.set(true);
        }
    }

    /// The value an effect actually read, sent as an incremental observation right
    /// before the later conditional that depends on it (D-S2c-8; the comparator
    /// consumes one per post-D `SpecFlowing` conditional).
    pub(crate) fn observed_read(&self, fact: ObservedFact) {
        if let Some(live) = &self.live {
            live.later(Later::Mark(Mark::Observed(fact)));
        }
    }

    /// An executor finished its effects. A leading one (the `Pre` effects, a prefix)
    /// hands the indices on; the last executor ends the decision: `Reply` and
    /// `Close{completed:true}` follow the header (now, or when it is submitted).
    pub(crate) fn finish_leg(&self, consumed: usize) {
        if let Some(live) = &self.live {
            if live.leading.get() {
                self.advance(consumed);
                return;
            }
            live.complete.set(true);
            live.conclude();
        }
    }

    /// Take the header's `seq` (still under D) and fix `live_post_d`: persisted M1/M2
    /// as the `In` effects committed them. The caller then releases D and submits
    /// ([`AfterD`]). Dropping a [`Sealed`] without submitting reports the loss.
    pub(crate) fn seal(&self, post: Option<Projection>) -> Sealed<'_> {
        let Some(live) = &self.live else {
            return Sealed { inner: None };
        };
        let Some(mut header) = live.header.borrow_mut().take() else {
            return Sealed { inner: None };
        };
        live.in_decision.set(false);
        if let Some(post) = post {
            header.live_post_d = post;
        }
        let ticket = live.rec.ticket(header.site_id);
        header.seq = ticket.seq();
        Sealed {
            inner: Some((live, ticket, header)),
        }
    }

    /// [`Flight::seal`] under a held D: the post-D projection is read **only for an
    /// active flight** (a disarmed flight reads nothing under D, D-S2c-8 b).
    pub(crate) fn seal_d(
        &self,
        app: &crate::state::AppState,
        dictation: &crate::state::DictationState,
    ) -> Sealed<'_> {
        if !self.is_active() {
            return Sealed { inner: None };
        }
        self.seal(Some(Projection {
            status: crate::dictation_admission::core_status(dictation.status),
            current_id: RecordingId(app.recording_id.load(Ordering::SeqCst)),
            obs: murmur_dictation_core::state::LiveObs::default(),
        }))
    }

    /// Whether the header went out (tests).
    #[cfg(test)]
    pub(crate) fn is_submitted(&self) -> bool {
        self.live.as_ref().is_some_and(|live| live.submitted.get())
    }
}

// ------------------------------------------------------------ the flight's slots

/// The flight-local slots the capture writes into while an executor runs one effect
/// (D-S2c-9 c): borrowed from the flight by the executor that drives it, never found by
/// thread. Nothing here is shared: the flight is one decision's, held by the section
/// that runs it.
#[derive(Default)]
pub(crate) struct ShadowSlots {
    /// The live-field write the effect being run made.
    pub mirror: Cell<Option<MirrorWitness>>,
    /// The ticker registration attempt the `SD` being run made.
    pub ticker: Cell<Option<TickerNote>>,
    /// The boolean the port's own `speculative_flowing` read returned for the effect
    /// being run (D-S2c-9 d).
    pub spec: Cell<Option<bool>>,
}

impl ShadowSlots {
    /// Before an effect runs: nothing an earlier effect left may be attributed to it.
    pub(crate) fn clear(&self) {
        self.mirror.set(None);
        self.ticker.set(None);
        self.spec.set(None);
    }
}

impl Flight {
    /// The slots of an active flight; `None` for an empty one (the first check of every
    /// capture statement: a disarmed flight is never read, allocated or written).
    pub(crate) fn slots(&self) -> Option<&ShadowSlots> {
        self.live.as_ref().map(|live| &live.slots)
    }
}

// ------------------------------------------------ the keyboard's atomic write

// The keyboard module's `IS_PROCESSING` write, as a per-thread sequence: the count of writes
// this thread made in the high bits, the last one's operands in the low two (`was` << 1 |
// `now`). The executor brackets the one port call that performs the write with the count
// before and after, on the same thread and with no `.await` between: the operands are the
// effect's witness only if **exactly one** write happened in between, so a missing or a
// repeated write fails closed (no witness) and is never attributed, and another thread's
// write cannot be taken for this one (D-S2c-9 d). Nothing here names a flight.
//
// The sequence is private to this module, so every **read** of it goes through [`read`],
// which the test build counts (the bracket's opener and closer): a read added before an
// enable check cannot bypass the pin. The capture's own write (`record`) is accepted.
mod keyboard_sequence {
    use std::cell::Cell;

    thread_local! {
        static KEYBOARD_WRITES: Cell<u64> = const { Cell::new(0) };
    }

    /// This thread's sequence (the one instrumented read).
    pub(super) fn read() -> u64 {
        #[cfg(test)]
        super::super::probe::shadow_read();
        KEYBOARD_WRITES.with(Cell::get)
    }

    /// One more write, with its operands (the capture's write).
    pub(super) fn record(operands: u64) {
        KEYBOARD_WRITES.with(|writes| writes.set((((writes.get() >> 2) + 1) << 2) | operands));
    }
}

/// `set_processing` swapped the flag: `was` is what the atomic swap returned, `now` what it
/// stored. Counted only while the shadow is armed.
pub(crate) fn keyboard_written(was: bool, now: bool) {
    if !crate::dictation_shadow::enabled() {
        return;
    }
    keyboard_sequence::record((u64::from(was) << 1) | u64::from(now));
}

/// This thread's number of keyboard writes so far (the bracket's opening).
pub(crate) fn keyboard_writes() -> u64 {
    keyboard_sequence::read()
}

/// The write this thread made since `before`, if there was exactly one (the bracket's
/// closing).
pub(crate) fn keyboard_write_since(before: u64) -> Option<MirrorWitness> {
    let now = keyboard_sequence::read();
    if (now >> 2) != (before >> 2) + 1 {
        return None;
    }
    Some(MirrorWitness {
        field: crate::dictation_shadow::types::MirrorField::KeyboardProcessing,
        before: crate::dictation_shadow::types::MirrorValue::Flag(now & 0b10 != 0),
        after: crate::dictation_shadow::types::MirrorValue::Flag(now & 0b01 != 0),
    })
}

// --------------------------------------------------- the ports an executor observes

// ------------------------------------------------------- the header after D

/// A sealed header that is submitted when the shell has released D: the shell holds it
/// from the seal, submits right after its own D release ([`AfterD::submit`]), and any
/// return before that submits when the scope that owns this guard (declared before the
/// D guard, so it drops after it) ends. An unwind drops it unsent: the loss is recorded.
pub(crate) struct AfterD<'a> {
    held: Cell<Option<Sealed<'a>>>,
}

impl<'a> AfterD<'a> {
    pub(crate) const fn new() -> Self {
        Self {
            held: Cell::new(None),
        }
    }

    pub(crate) fn hold(&self, sealed: Sealed<'a>) {
        self.held.set(Some(sealed));
    }

    pub(crate) fn submit(&self) {
        if let Some(sealed) = self.held.take() {
            sealed.submit();
        }
    }
}

impl Drop for AfterD<'_> {
    /// A header that was sealed (the decision's `In` effects all ran) is submitted even
    /// while the thread unwinds (a later panic, a deferred drain run by an unwind): the
    /// submit is one non-blocking `try_push`. A decision whose `In` effects unwound was
    /// never sealed, and its flight records the loss instead.
    fn drop(&mut self) {
        self.submit();
    }
}

// ----------------------------------------------------- the reply at the return

/// The flights of one command invocation that wait for the reply it returns, or (a drain
/// that runs a queued finish) the scope in which a flight waits for no reply at all.
struct Sink {
    flights: RefCell<Vec<Live>>,
    replyless: bool,
}

tokio::task_local! {
    static REPLY_SINK: Sink;
}

/// Run one IPC command invocation (a `#[tauri::command]` wrapper) and witness the reply
/// it returns, at its return (CP4, D-S2c-8 f): every flight of the invocation that
/// waits for its reply concludes with the kind of the value the command handed to IPC.
/// A disarmed shadow runs the command unchanged.
pub(crate) async fn commanded<T, E, F>(command: F) -> Result<T, E>
where
    T: RenderedValue,
    F: std::future::Future<Output = Result<T, E>>,
{
    if !crate::dictation_shadow::enabled() {
        return command.await;
    }
    REPLY_SINK
        .scope(
            Sink {
                flights: RefCell::new(Vec::new()),
                replyless: false,
            },
            async {
                let result = command.await;
                witness_returned(&result);
                result
            },
        )
        .await
}

/// The status the live op reported at a consume, kept for the flight's `decided` (the
/// shell's `Cell` of D-S2c-9). Both the store and the read are gated on the flight being
/// active, so a disarmed run neither writes nor reads it; both are counted shadow
/// state accesses (D-S2c-10 amendment 1).
pub(crate) struct SeenStatus(Cell<Option<murmur_dictation_core::decision::OpStatus>>);

impl SeenStatus {
    pub(crate) fn new() -> Self {
        Self(Cell::new(None))
    }

    pub(crate) fn set(&self, flight: &Flight, status: murmur_dictation_core::decision::OpStatus) {
        if flight.is_active() {
            #[cfg(test)]
            super::probe::shadow_read();
            self.0.set(Some(status));
        }
    }

    pub(crate) fn get(&self, flight: &Flight) -> Option<murmur_dictation_core::decision::OpStatus> {
        if !flight.is_active() {
            return None;
        }
        #[cfg(test)]
        super::probe::shadow_read();
        self.0.get()
    }
}

/// The detached auto-stop's task, armed (the caller checked the enable flag and boxes the
/// result): run it in a reply scope of its own, so its flights conclude with the result it
/// returns at its return, and hand that result back **unchanged**. `None` is a stop
/// refused as re-entrant before it began (no flight waits for a reply then).
pub(crate) async fn witnessed_detached<F, E>(task: F) -> Option<Result<serde_json::Value, E>>
where
    F: std::future::Future<Output = Option<Result<serde_json::Value, E>>>,
{
    REPLY_SINK
        .scope(
            Sink {
                flights: RefCell::new(Vec::new()),
                replyless: false,
            },
            async move {
                let result = task.await;
                witness_kind(match &result {
                    None => ReplyKind::Ok,
                    Some(Err(_)) => ReplyKind::Err,
                    Some(Ok(value)) => value_kind(value),
                });
                result
            },
        )
        .await
}

/// Run the finish a drain took from the deferred queue. The command that queued it has
/// already returned (or returns something else): the finish has no reply of its own, so
/// its flights close as the named no-reply close (D-S2c-10 R6). The scope is the finish's
/// own, so a synchronous command nested inside it still sees its own sink. A disarmed
/// shadow runs `finish` unchanged.
pub(crate) fn unreplied<R>(finish: impl FnOnce() -> R) -> R {
    if !crate::dictation_shadow::enabled() {
        return finish();
    }
    REPLY_SINK.sync_scope(
        Sink {
            flights: RefCell::new(Vec::new()),
            replyless: true,
        },
        finish,
    )
}

/// The command is about to return the import pipeline's own error, which the core's
/// R302.2 does not model (it decides the success reply): the decisions already waiting
/// for the command's reply take the `Err` outcome, and the actual return is what they are
/// compared against (D-S2c-10 R3). An `Err` return closes them as the named no-reply close;
/// any other return is concluded with a kind no decision names, which the comparator reports. Does
/// nothing without a command wrapper.
pub(crate) fn import_pipeline_error() {
    if !crate::dictation_shadow::enabled() {
        return;
    }
    let _ = REPLY_SINK.try_with(|sink| {
        for live in sink.flights.borrow().iter() {
            live.err_outcome.set(true);
        }
    });
}

// The ticker spawner's outcome, handed to the port that called it. The spawner runs
// synchronously inside the port's `spawn` call and there is no `.await` between its
// write and the port's take, and **every** spawner call overwrites it (a skip or `None`),
// so a value an earlier call left is never this one's. It names no flight and is written
// only while armed and read, once, by the active port that made the call.
thread_local! {
    static TICKER_SKIP: Cell<Option<TickerSkipReason>> = const { Cell::new(None) };
}

/// The skip a spawner lookup took, read from its result: the argument of
/// [`ticker_outcome`], evaluated by the caller **inside** its enable check (counted in test
/// builds, so evaluating it before the check fails the disarmed read pin).
pub(crate) fn ticker_skip_of<T>(started: &Result<T, TickerSkipReason>) -> Option<TickerSkipReason> {
    #[cfg(test)]
    super::probe::shadow_read();
    started.as_ref().err().copied()
}

/// The ticker spawner's one lookup is done: `skip` is the skip it takes before it asks for
/// the registration gate (no recorded context, or a model with no live preview), `None`
/// when it goes on to ask. The caller has made the enable check (its one flag load), so this
/// only writes the mailbox.
pub(crate) fn ticker_outcome(skip: Option<TickerSkipReason>) {
    TICKER_SKIP.with(|slot| slot.set(skip));
}

/// The spawner's outcome of the port's call (an active port only, once per call).
pub(crate) fn take_ticker_skip() -> Option<TickerSkipReason> {
    #[cfg(test)]
    super::probe::shadow_read();
    TICKER_SKIP.with(Cell::take)
}

/// Test builds: whether this task (and nothing it spawned) is inside a command wrapper's
/// reply scope.
#[cfg(test)]
pub(crate) fn reply_sink_visible() -> bool {
    REPLY_SINK.try_with(|_| ()).is_ok()
}

fn witness_returned<T: RenderedValue, E>(result: &Result<T, E>) {
    witness_kind(rendered_kind(result));
}

fn witness_kind(kind: ReplyKind) {
    let _ = REPLY_SINK.try_with(|sink| {
        let waiting = std::mem::take(&mut *sink.flights.borrow_mut());
        for live in waiting {
            if live.err_outcome.get() {
                if kind == ReplyKind::Err {
                    // The modelled outcome happened: the pipeline's own error is returned.
                    #[cfg(test)]
                    NO_REPLY.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                // The pipeline failed and the command returned something else, whatever it
                // is (a success included): the expected outcome did not happen. Credit a
                // reply kind no decision names, so the comparator reports it (diff 4)
                // instead of matching it against the success the core decided.
                live.reply.set(Some(ReplyKind::Unrecognized));
                live.conclude();
                continue;
            }
            live.reply.set(Some(kind));
            live.conclude();
        }
    });
}

/// A reply value a command hands to IPC: the kind it renders as.
pub(crate) trait RenderedValue {
    fn rendered_kind(&self) -> ReplyKind;
}

/// `Ok(())` (the cancel command).
impl RenderedValue for () {
    fn rendered_kind(&self) -> ReplyKind {
        ReplyKind::Ok
    }
}

impl RenderedValue for serde_json::Value {
    fn rendered_kind(&self) -> ReplyKind {
        value_kind(self)
    }
}

/// The kind of a returned reply, from the value the command hands to IPC.
fn rendered_kind<T: RenderedValue, E>(reply: &Result<T, E>) -> ReplyKind {
    match reply {
        Err(_) => ReplyKind::Err,
        Ok(value) => value.rendered_kind(),
    }
}

fn value_kind(value: &serde_json::Value) -> ReplyKind {
    use murmur_dictation_core::replies::ReplyType;
    const TYPES: [ReplyType; 14] = [
        ReplyType::AppDisabled,
        ReplyType::BusyRestarting,
        ReplyType::BusyMeeting,
        ReplyType::BusyTranscribingFile,
        ReplyType::BusyRecordingCorpus,
        ReplyType::BusyBenchmarking,
        ReplyType::AlreadyStarting,
        ReplyType::AlreadyRecording,
        ReplyType::AudioRecovering,
        ReplyType::AlreadyProcessing,
        ReplyType::RecordingSuperseded,
        ReplyType::NotRecording,
        ReplyType::RecordingCancelled,
        ReplyType::RecordingStarting,
    ];
    match value.get("type").and_then(serde_json::Value::as_str) {
        None => ReplyKind::Ok,
        Some("transcription") => ReplyKind::Transcription,
        // A literal no decision names is its own kind: it matches nothing the core can
        // decide, an `Err` included (D-S2c-8 f).
        Some(literal) => TYPES
            .into_iter()
            .find(|ty| ty.as_str() == literal)
            .map_or(ReplyKind::Unrecognized, ReplyKind::Typed),
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
        // Test builds: a point where a test observes whether D is still held (never while
        // unwinding: a hook that blocks or panics there would abort).
        #[cfg(test)]
        if !std::thread::panicking() {
            crate::barrier_hook::at("shadow.submit");
        }
        transport::submit(ticket, header);
        HEADERS.fetch_add(1, Ordering::Relaxed);
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
    #[cfg(test)]
    super::probe::shadow_read();
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

/// An H2 retirement of `recording_id`'s pipeline entry by its owner.
pub(crate) fn h2_retired(
    recording_id: RecordingId,
    op: murmur_dictation_core::ids::OperationId,
    matched: bool,
) -> LiveOp {
    use crate::dictation_shadow::types::{HistoryAction, HistoryTable};
    LiveOp {
        table: HistoryTable::H2,
        action: HistoryAction::Retired { matched },
        cont: murmur_dictation_core::decision::ContKind::Pipeline,
        recording_id,
        op,
    }
}
