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

/// A final decision was taken (not a `NeedObservation` round, not an `Invalid`
/// refusal that ran nothing). Counted only while the shadow is armed.
pub(crate) fn note_decision(decision: &murmur_dictation_core::decision::Decision) {
    use murmur_dictation_core::decision::Verdict;
    if matches!(
        decision.verdict,
        Verdict::NeedObservation(_) | Verdict::Invalid(_)
    ) {
        return;
    }
    note_decision_taken();
}

// ------------------------------------------------- entry/header conservation
//
// D-S2c-7 a: the live shell counts the final decisions it takes while the shadow is
// armed, and the flights count the headers they submit. One relaxed increment each, no
// lock, no allocation. E7's synchronous replay asserts the two are equal per sub-run
// (the harness reads them here and hands the delta to the replay's report), so a
// family entry that returns without a capture fails closed, whichever branch it is.

static DECISIONS: AtomicU64 = AtomicU64::new(0);
static HEADERS: AtomicU64 = AtomicU64::new(0);
/// Decisions whose flight was dropped by an unwind after the decision and before its
/// header: recorded as one loss each (D-S2c-7 e), so they balance the books.
static UNWOUND: AtomicU64 = AtomicU64::new(0);

/// A final decision was taken by the live shell. Counted only while the shadow is armed.
pub(crate) fn note_decision_taken() {
    if crate::dictation_shadow::enabled() {
        DECISIONS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Test builds: completed flights whose reply the shell never rendered through the
/// flight (a site missing its `rendered`).
#[cfg(test)]
static UNRENDERED: AtomicU64 = AtomicU64::new(0);

/// Test builds: the counters so far; a replay's report takes the difference between
/// its end and its start.
#[cfg(test)]
pub(crate) fn conservation_now() -> crate::dictation_shadow::replay::Conservation {
    crate::dictation_shadow::replay::Conservation {
        decisions: DECISIONS.load(Ordering::Relaxed),
        headers: HEADERS.load(Ordering::Relaxed),
        unrendered: UNRENDERED.load(Ordering::Relaxed),
        unwound: UNWOUND.load(Ordering::Relaxed),
    }
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
    /// The reply kind the shell **rendered** (D-S2c-7 f), not the decided one.
    reply: Cell<Option<ReplyKind>>,
    /// The decision has a reply: `Reply` and `Close` wait for the rendered kind.
    expects_reply: Cell<bool>,
    /// The kind the decision decided (a reply a later site renders: see
    /// [`Flight::delegated_reply`]).
    decided_reply: Cell<Option<ReplyKind>>,
    /// The decision was taken (D is held, or the entry is D-less) and its header has
    /// not been sealed: an unwind here is a recorded loss (D-S2c-7 e).
    in_decision: Cell<bool>,
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
            expects_reply: Cell::new(false),
            decided_reply: Cell::new(None),
            in_decision: Cell::new(false),
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
        if !self.submitted.get() || !self.complete.get() {
            return;
        }
        // A decision with a reply concludes when the shell has rendered it.
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
        live.expects_reply.set(reply.is_some());
        live.decided_reply.set(reply.map(ReplyKind::of));
        live.in_decision.set(true);
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
                            if let Some(pred) = pred {
                                stored &= live.pre.conditionals.push(CondWitness {
                                    pred,
                                    taken: noted.taken,
                                });
                            }
                            if noted.taken {
                                stored &= live.pre.effects.push(witnessed);
                                if let Some(mirror) = noted.mirror {
                                    stored &= live.pre.mirrors.push(mirror);
                                }
                            }
                        }
                        if !stored {
                            live.pre.overflowed = true;
                        }
                    }
                }
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

    /// The reply the shell rendered for the decision (D-S2c-7 f): its kind is read
    /// from the rendered value (the IPC `type` literal), not from the decision, so a
    /// wrong literal compares unclean. `Reply` and `Close` follow the header and the
    /// last effect.
    pub(crate) fn rendered<T: RenderedValue, E>(&self, reply: &Result<T, E>) {
        if let Some(live) = &self.live {
            if live.reply.get().is_none() {
                live.reply.set(Some(rendered_kind(reply)));
            }
            live.conclude();
        }
    }

    /// R201.4 and R205.4 decide a reply that the command does not render at that point:
    /// the capture cancel's own result is site 206's, which renders (and is compared
    /// for) the command's actual reply. The decision's own reply is concluded with the
    /// kind it decided.
    pub(crate) fn delegated_reply(&self) {
        if let Some(live) = &self.live {
            if live.reply.get().is_none() {
                live.reply.set(live.decided_reply.get());
            }
            live.conclude();
        }
    }

    /// The value an effect actually read, sent as an incremental observation right
    /// after the effect's own witness and before any later conditional that depends on
    /// it (`SPEC`'s `speculative_flowing`, D-S2c-7 f).
    pub(crate) fn observed_read(&self, fact: ObservedFact) {
        if let Some(live) = &self.live {
            live.later(Later::Mark(Mark::Observed(fact)));
        }
    }

    /// A flight that leaves its section with the decision, for the command's return
    /// to render the reply through (see [`DelegatingFlight`]).
    pub(crate) fn delegating(self) -> DelegatingFlight {
        DelegatingFlight(self)
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

    /// Whether the header went out (tests).
    #[cfg(test)]
    pub(crate) fn is_submitted(&self) -> bool {
        self.live.as_ref().is_some_and(|live| live.submitted.get())
    }
}

/// A flight carried out of its section to the command's return, where the reply is
/// rendered through it. A return that never reaches that point (an error the decision
/// does not cover: a pipeline's own failure is a shell tail outside the row, and a
/// parse failure always returns its error) concludes the flight with the kind the
/// decision decided when it drops, so the flight never stays open.
pub(crate) struct DelegatingFlight(Flight);

impl DelegatingFlight {
    /// Render the command's reply through the flight.
    pub(crate) fn rendered<T: RenderedValue, E>(&self, reply: &Result<T, E>) {
        self.0.rendered(reply);
    }
}

impl Drop for DelegatingFlight {
    fn drop(&mut self) {
        self.0.delegated_reply();
    }
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

/// The kind of a rendered reply, from the value the shell is about to hand to IPC.
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
        // A literal no decision names: it matches nothing the core can decide.
        Some(literal) => TYPES
            .into_iter()
            .find(|ty| ty.as_str() == literal)
            .map_or(ReplyKind::Err, ReplyKind::Typed),
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
        #[cfg(test)]
        if self.complete.get() && self.expects_reply.get() && self.reply.get().is_none() {
            UNRENDERED.fetch_add(1, Ordering::Relaxed);
        }
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
