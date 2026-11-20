//! Tasks: the actors the scheduler sequences, the event log every step writes,
//! and the interpreter of placed effects.
//!
//! **Atomicity.** A step is atomic. A D section (decide + its `In` effects) is
//! one step; `Pre`, `PostDT` and `PostT` effects are one step each, so another
//! actor's D section can land between any two of them (C7.5: "effects of class
//! PostD_T / PostT interleave with other actors' D sections").

use super::c3;
use super::model::*;
use crate::decision::{Decision, SiteId, Verdict};
use crate::effects::{Effect, EventKind, Placed, Placement, Terminal};
use crate::inputs::GuardOrigin;
use crate::status::{ErrCode, Status, TerminalOutcome};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    /// Nothing happened (the contract: a blocked step mutates nothing).
    Blocked,
    Progress,
    Done,
}

/// What is known about a decision when it ran (every decision is logged).
/// Fields are read through `Debug` in traces.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct DecisionEv {
    pub tid: TaskId,
    pub site: SiteId,
    pub row: &'static str,
    pub verdict: Verdict,
    /// The recording the input names.
    pub input_rid: Rid,
    pub origin: Option<GuardOrigin>,
    pub current_before: Rid,
    pub status_before: Status,
    pub cleanup_pending: bool,
    /// The owner attempt still has a notification that can restore `Idle`.
    pub restorer_pending: bool,
    pub transition: Option<(Status, Status)>,
    /// Another task's cancel tail for this recording is between its D release and its end.
    pub in_cancel_tail: bool,
    /// A pipeline for the recording is running (H2).
    pub pipeline_live: bool,
    pub owner: Option<Rid>,
    pub ended_by: Option<EndedBy>,
    pub phase: Phase,
    pub step: u32,
}

/// One effect applied to the world.
/// Fields are read through `Debug` in traces.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EffectEv {
    pub tid: TaskId,
    pub row: &'static str,
    pub site: SiteId,
    pub at: Placement,
    pub effect: Effect,
    pub input_rid: Rid,
    pub current: Rid,
    pub status_before: Status,
    pub status_after: Status,
    /// `claim_terminal`'s answer, for `TERM`.
    pub claim: Option<bool>,
    /// `accepted(id)` had run when a `TERM` was applied.
    pub accepted: bool,
    pub step: u32,
}

/// Facts the detectors need that are not an effect or a decision.
/// Fields are read through `Debug` in traces.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Note {
    /// A `TakeInterrupted` ran: `found` says whether a capture was there.
    Take {
        tid: TaskId,
        rid: Rid,
        row: &'static str,
        found: bool,
        /// The task and row that took it earlier, if one did.
        earlier: Option<(&'static str,)>,
    },
    /// The stop the supervisor's `Interrupted` arm spawned (`RS`, R215.2) ended its
    /// decision without taking the preserved capture of `rid` (refused at `Idle` or
    /// `Starting`, or nothing to take): the interrupted capture is left in the store.
    AutoStopTookNothing {
        rid: Rid,
    },
    /// A start future was dropped at an await point.
    StartDropped {
        rid: Rid,
        registered: bool,
        begun: bool,
    },
    /// A pipeline or stop task panicked: its guard's `Drop` ran.
    TaskPanicked {
        tid: TaskId,
        rid: Rid,
    },
    /// A partial ticker task exited by a panic: `hide_on_ticker_exit` never ran.
    TickerPanicked {
        rid: Rid,
    },
    TickerExited {
        rid: Rid,
    },
    /// A ticker took (`acquire`) or dropped the partial-decode lease.
    Lease {
        rid: Rid,
        tid: TaskId,
        acquire: bool,
    },
    /// The diagnostics `complete` of a `TERM` ran as its own step.
    DiagnosticsCompleted {
        rid: Rid,
        by: TerminalOutcome,
        first: bool,
        claim_winner: Option<TerminalOutcome>,
    },
    /// A paste ran through the ownership fence.
    Delivered {
        rid: Rid,
        owned: bool,
    },
    /// The stop IPC of a window's `handleStop` returned (its guard clears).
    WindowStopReturned {
        window: usize,
        tid: TaskId,
    },
    /// The supervisor's `OwnerClear` step ran.
    OwnerCleared {
        rid: Rid,
    },
    /// A supervisor notification was delivered (its arm returned).
    Notified {
        rid: Rid,
        step: SupStep,
    },
}

#[derive(Default, Debug)]
pub struct StepOut {
    pub decisions: Vec<DecisionEv>,
    pub effects: Vec<EffectEv>,
    pub notes: Vec<Note>,
    /// Defects the step itself detected (a decision that differs by op status,
    /// a decision that fails `validate`).
    pub violations: Vec<String>,
    /// The effects the shell ran before the decide (`Pre` prefix) and stripped from the
    /// decision it logs next: the C3 check counts the whole list.
    pub stripped: Vec<Effect>,
    /// The code the decision's input carries (an `AudioFailureKind` mapped to its
    /// `ErrCode`, an interrupted capture's reason, a pipeline's no-speech code), which
    /// C3's `@in` terminal codes must equal (D-S2b-11 (b)). Taken by `log_decision`.
    pub input_code: Option<ErrCode>,
    /// The `NeedObservation` rounds the step's flight answered, by the row the verdict names:
    /// they are not decisions the flight acts on, and the gate counts them apart.
    pub observations: Vec<&'static str>,
}

impl StepOut {
    pub fn clear(&mut self) {
        self.observations.clear();
        self.stripped.clear();
        self.input_code = None;
        self.decisions.clear();
        self.effects.clear();
        self.notes.clear();
        self.violations.clear();
    }
}

pub struct Cx<'a> {
    pub w: &'a mut World,
    pub out: &'a mut StepOut,
    pub spawn: &'a mut Vec<Box<dyn Task>>,
    pub tid: TaskId,
    /// Results posted by a pipeline for the stop that awaits it.
    pub mailbox: &'a mut Vec<(TaskId, PipelineReply)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineReply {
    Ok(crate::inputs::PipelineTerminal),
    Err,
    /// The pipeline's task unwound: the stop task unwinds with it.
    Panicked,
}

pub trait Task {
    fn step(&mut self, cx: &mut Cx<'_>) -> Poll;
    fn name(&self) -> &'static str;
    /// A start future parked on an await point can be dropped there (D-S2-16:
    /// never inside a span, only between polls).
    fn droppable(&self) -> bool {
        false
    }
    fn drop_now(&mut self, _cx: &mut Cx<'_>) {}
}

/// Where an effect runs and for whom.
#[derive(Clone, Copy, Debug)]
pub struct FxCtx {
    pub tid: TaskId,
    pub rid: Rid,
    pub row: &'static str,
    pub site: SiteId,
}

#[derive(Clone, Debug)]
pub enum QItem {
    Fx(FxCtx, Placed),
    /// The second half of a split `TERM`: `dictation_diagnostics.complete`.
    TermDiag(FxCtx, Terminal),
    ReleaseT,
}

/// `Decision::after_pre`, remembering the stripped `Pre` effects for the next
/// [`log_decision`] (the shell ran them before the decide).
pub fn after_pre(cx: &mut Cx<'_>, decision: Decision) -> Decision {
    let before = decision.effects.clone();
    let after = decision.after_pre();
    let cut = before.len() - after.effects.len();
    cx.out.stripped = before[..cut].iter().map(|p| p.effect.clone()).collect();
    after
}

/// `Decision::after_prefix`, remembering the stripped prefix for the next [`log_decision`].
pub fn after_prefix(cx: &mut Cx<'_>, decision: Decision, prefix: &[Placed]) -> Decision {
    let before = decision.effects.clone();
    let after = decision.after_prefix(prefix);
    let cut = before.len() - after.effects.len();
    cx.out.stripped = before[..cut].iter().map(|p| p.effect.clone()).collect();
    after
}

/// Record a decision; returns it for the caller.
#[allow(clippy::too_many_arguments)]
pub fn log_decision(
    cx: &mut Cx<'_>,
    site: SiteId,
    decision: &Decision,
    input_rid: Rid,
    origin: Option<GuardOrigin>,
) {
    if let Err(error) = decision.validate() {
        cx.out
            .violations
            .push(format!("{}: invalid decision {error:?}", decision.row.0));
    }
    // SP3 / C3 (D-S2b-9 (d)): the decision's whole effect list is one allowed outcome
    // of its row in the hand-transcribed fixture.
    let stripped = std::mem::take(&mut cx.out.stripped);
    let whole: Vec<Effect> = stripped
        .into_iter()
        .chain(decision.effects.iter().map(|placed| placed.effect.clone()))
        .collect();
    let input_code = cx.out.input_code.take();
    let w = &*cx.w;
    if let Err(message) = c3::check_decision(
        decision.row.0,
        w.status,
        input_rid != w.current_id,
        &whole,
        input_code,
    ) {
        cx.out.violations.push(message);
    }
    let owner = w.sup.owner;
    let restorer_pending = w.sup.restorer_coming();
    cx.out.decisions.push(DecisionEv {
        tid: cx.tid,
        site,
        row: decision.row.0,
        verdict: decision.verdict.clone(),
        input_rid,
        origin,
        current_before: w.current_id,
        status_before: w.status,
        cleanup_pending: w.sup.cleanup_pending(w.current_id),
        restorer_pending,
        transition: decision.transition,
        in_cancel_tail: w.cancel_tail.get(&input_rid).is_some_and(|t| *t != cx.tid),
        pipeline_live: w.history.pipelines.contains(&input_rid),
        owner,
        // The owner's attempt, or how it ended once `finish_attempt` took it (its
        // notifications and the owner clear are still to come).
        ended_by: w.sup.attempt.as_ref().map(|a| a.ended_by).or_else(|| {
            owner.and_then(|rid| {
                w.sup
                    .finished
                    .iter()
                    .rev()
                    .find(|(finished, _)| *finished == rid)
                    .map(|(_, ended)| *ended)
            })
        }),
        phase: w.sup.phase(),
        step: w.step,
    });
}

/// Acquire the recording transition (T); `false` is a blocked step.
pub fn acquire_t(cx: &mut Cx<'_>) -> bool {
    if cx.w.t_holder.is_none() {
        cx.w.t_holder = Some(cx.tid);
        true
    } else {
        false
    }
}

pub fn release_t(cx: &mut Cx<'_>) {
    if cx.w.t_holder == Some(cx.tid) {
        cx.w.t_holder = None;
    }
}

/// Effects of a decision by placement.
pub fn by_placement(decision: &Decision, at: Placement) -> Vec<Placed> {
    decision
        .effects
        .iter()
        .filter(|placed| placed.at == at)
        .cloned()
        .collect()
}

/// Whether an `EmitStatus` with this condition emits in the model. `UnlessSpeculativeFlowing`
/// skips the `starting` emit while the speculative recorder is flowing; the model has no
/// speculative flow, so it never skips. `EmitOk` pairs a presentation with an emit and does
/// not gate an emit; the model's emit always succeeds.
pub(crate) fn emission_runs(cond: crate::effects::Cond) -> bool {
    use crate::effects::Cond;
    match cond {
        Cond::Always | Cond::UnlessSpeculativeFlowing | Cond::EmitOk(_) => true,
    }
}

/// Apply one effect atomically. Returns `false` for the effects a flight
/// handles itself (they cross to another actor): `CS`, `CP`, `CC`, `RS`, `PS`.
pub fn apply(cx: &mut Cx<'_>, fx: &FxCtx, at: Placement, effect: &Effect) -> bool {
    let before = cx.w.status;
    let accepted = cx.w.ledger.accepted.contains(&fx.rid);
    let mut claim = None;
    match effect {
        Effect::CaptureStart(_)
        | Effect::CaptureStop(_)
        | Effect::CaptureCancel { .. }
        | Effect::SelfStop { .. }
        | Effect::PipelineSpawn(_) => return false,
        Effect::SetStatus(to) => {
            // A write only: a window learns nothing until the status is *emitted*
            // (`EmitStatus`, often a later placement of the same row), so two rows can
            // write in one order and emit in the other (D-S2b-18).
            cx.w.status = *to;
        }
        Effect::AllocId => {
            // SP6: `next_recording_id` only under D+T.
            debug_assert_eq!(cx.w.t_holder, Some(fx.tid), "ID+ without T");
            cx.w.current_id += 1;
        }
        Effect::EmitStatus { status, cond } => {
            if emission_runs(*cond) {
                if *status == Status::Idle {
                    *cx.w.idle_emits.entry(fx.rid).or_default() += 1;
                }
                // Every window gets the event in emit order, each at its own pace.
                let latest_generation = cx.w.gen_emitted;
                for window in &mut cx.w.windows {
                    window.inbox.push_back(Inbound {
                        status: *status,
                        latest_generation,
                    });
                }
            }
        }
        Effect::Terminal(terminal) => {
            let ctx = cx.w.won(fx.rid, fx.tid, fx.row);
            claim = Some(cx.w.claim_terminal(fx.rid, *terminal, fx.row, ctx));
            // `In` sections run both halves under D; the later placements split
            // them (`QItem::TermDiag`), which is how the PD-11 window opens.
            if at == Placement::In {
                term_diag(cx, fx, *terminal);
            }
        }
        Effect::AcceptTelemetry => {
            cx.w.ledger.accepted.insert(fx.rid);
        }
        Effect::PerfRegister => {
            cx.w.perf.registered.insert(fx.rid);
        }
        Effect::PerfBegin => {
            if cx.w.perf.registered.contains(&fx.rid) {
                cx.w.perf.begun.insert(fx.rid);
                // `resolve_begin(succeeded)`: the completion requested while the begin was
                // in flight is the run's outcome now.
                if let Some((outcome, row, ctx)) = cx.w.perf.pending.remove(&fx.rid) {
                    cx.w.perf.outcome.entry(fx.rid).or_insert((outcome, row));
                    cx.w.perf.ctx.entry(fx.rid).or_insert(ctx);
                }
            }
        }
        Effect::PerfAbort => {
            if !cx.w.perf.begun.contains(&fx.rid) {
                cx.w.perf.registered.remove(&fx.rid);
                // `abort`: the entry, and the completion it held, are gone.
                cx.w.perf.pending.remove(&fx.rid);
                cx.w.perf.aborted.insert(fx.rid);
            }
        }
        Effect::PerfComplete(complete) => {
            // `only_if_run` (PD-24): the coordinator answers `NoRun` for an id with no
            // registered or begun run and claims nothing.
            let run = cx.w.perf.registered.contains(&fx.rid) || cx.w.perf.begun.contains(&fx.rid);
            // `follows_terminal` (PD-20): the second half of the terminal claim just before it,
            // only while that claim stands (it won, or it lost to a stale-owner supersede).
            let stands = !complete.follows_terminal || cx.w.terminal_stands(fx.rid);
            if !stands {
                cx.w.perf.dropped.insert((fx.rid, fx.row));
            } else if run || !complete.only_if_run {
                let ctx = cx.w.won(fx.rid, fx.tid, fx.row);
                cx.w.perf_complete(fx.rid, complete.outcome, fx.row, ctx);
            } else {
                cx.w.perf.dropped.insert((fx.rid, fx.row));
            }
        }
        Effect::PipelineGuardFinish(outcome) => {
            let ctx = cx.w.won(fx.rid, fx.tid, fx.row);
            cx.w.perf_complete(fx.rid, *outcome, fx.row, ctx);
        }
        Effect::ClearContext(id) => {
            if cx.w.context_for == Some(id.0) {
                cx.w.context_for = None;
            }
        }
        Effect::SetContext(id) => cx.w.context_for = Some(id.0),
        Effect::KeyboardProcessing(on) => cx.w.keyboard = *on,
        Effect::MarkCancelled(id) => {
            cx.w.cancelled_through = cx.w.cancelled_through.max(id.0);
        }
        Effect::StartSideTasks => {
            if cx.w.register_ticker(fx.rid) {
                let ticker = super::flights::Ticker::new(fx.rid);
                // One ticker in six panics (PD-14): decided by the clock, so a seed
                // reproduces it.
                cx.spawn.push(Box::new(if cx.w.step % 6 == 3 {
                    ticker.panicking()
                } else {
                    ticker
                }));
            }
        }
        Effect::TakeInterrupted { id, .. } => {
            let earlier = cx.w.taken_by.get(&id.0).map(|row| (*row,));
            let found = cx.w.take_interrupted(id.0, fx.row).is_some();
            cx.out.notes.push(Note::Take {
                tid: fx.tid,
                rid: id.0,
                row: fx.row,
                found,
                earlier,
            });
        }
        // Observations and effects with no model state.
        Effect::EmitEvent(EventKind::GenStarted) => {
            // `dictation-generation-started`: every status event emitted after it names this
            // generation as the newest one (`Inbound::latest_generation`).
            cx.w.gen_emitted = cx.w.gen_emitted.max(fx.rid);
        }
        Effect::EmitEvent(_)
        | Effect::Presentation { .. }
        | Effect::TouchLastTranscription
        | Effect::AxSnapshot(_)
        | Effect::GestureAccepted
        | Effect::PrepareCaptureClaimed
        | Effect::PreviewHideClaim
        | Effect::TakeStopStamps
        | Effect::ModelPreparation
        | Effect::ReleaseClaims
        | Effect::TraceRequested
        | Effect::Observe(_)
        | Effect::ProductionCaptureFailure(_)
        | Effect::SpeculativeConsume
        | Effect::PipelineEnter(_)
        | Effect::SmartDestinationEmit => {}
    }
    let step = cx.w.step;
    cx.out.effects.push(EffectEv {
        tid: fx.tid,
        row: fx.row,
        site: fx.site,
        at,
        effect: effect.clone(),
        input_rid: fx.rid,
        current: cx.w.current_id,
        status_before: before,
        status_after: cx.w.status,
        claim,
        accepted,
        step,
    });
    true
}

/// The `In` effects of a decision, atomically (one D section).
pub fn apply_in(cx: &mut Cx<'_>, fx: &FxCtx, decision: &Decision) {
    for placed in by_placement(decision, Placement::In) {
        let handled = apply(cx, fx, Placement::In, &placed.effect);
        debug_assert!(handled, "an In effect crosses to another actor: {placed:?}");
    }
}

/// Queue the effects of `at` for step-by-step execution.
pub fn queue_phase(q: &mut VecDeque<QItem>, fx: FxCtx, decision: &Decision, at: Placement) {
    for placed in by_placement(decision, at) {
        q.push_back(QItem::Fx(fx, placed));
    }
}

/// The second half of a split `TERM`.
pub fn term_diag(cx: &mut Cx<'_>, fx: &FxCtx, terminal: Terminal) {
    let first = !cx.w.ledger.diagnostics.contains_key(&fx.rid);
    let claim_winner = cx.w.ledger.claimed.get(&fx.rid).map(|(o, _, _)| *o);
    cx.w.diagnostics_complete(fx.rid, terminal.outcome);
    cx.out.notes.push(Note::DiagnosticsCompleted {
        rid: fx.rid,
        by: terminal.outcome,
        first,
        claim_winner,
    });
}

/// Builds the decision a site takes for a token in the world it is delivered to.
pub type Build = std::rc::Rc<dyn Fn(&World, crate::decision::OpStatus) -> Decision>;

/// Consume a continuation: validate it, decide, retire it (`consume`). The
/// decision is also made with the other op status: an `Unknown` token must take
/// the same row with the same effects as an `Outstanding` one (SP7, C1.4.1
/// rule 3). A duplicate or late delivery of the token is then scheduled as its own
/// task (`Redeliver`) for some of the tokens, so the site decides again, later, in
/// whatever world it finds (D-S2b-7 (e)).
pub fn consume(
    cx: &mut Cx<'_>,
    token: crate::decision::Continuation,
    site: SiteId,
    rid: Rid,
    build: impl Fn(&World, crate::decision::OpStatus) -> Decision + 'static,
) -> Decision {
    use crate::decision::OpStatus;
    let status = cx.w.op_status(token);
    let decided = build(cx.w, status);
    let other = build(
        cx.w,
        match status {
            OpStatus::Outstanding => OpStatus::Unknown,
            OpStatus::Unknown => OpStatus::Outstanding,
        },
    );
    if other != decided {
        cx.out.violations.push(format!(
            "SP7: {} differs by op status ({:?} vs {:?})",
            decided.row.0, decided.verdict, other.verdict
        ));
    }
    cx.w.retire(token);
    // One token in four is delivered again: soon (a duplicate) or after a few steps
    // (a late one). The choice is a function of the token, so a seed reproduces it.
    let mix = token.op.0.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
    if mix.is_multiple_of(4) {
        cx.spawn.push(Box::new(super::redeliver::Redeliver::new(
            token,
            site,
            rid,
            std::rc::Rc::new(build),
            (mix / 4 % 6) as u8,
        )));
    }
    decided
}

/// The queue every flight drains one item per step, before it advances.
pub trait Queued {
    fn queue(&mut self) -> &mut VecDeque<QItem>;
    /// An effect that crosses to another actor. `Err(())`: not yet (blocked).
    fn cross(&mut self, cx: &mut Cx<'_>, fx: &FxCtx, effect: &Effect) -> Result<(), ()>;
}

pub fn is_cross(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::CaptureStart(_)
            | Effect::CaptureStop(_)
            | Effect::CaptureCancel { .. }
            | Effect::SelfStop { .. }
            | Effect::PipelineSpawn(_)
    )
}

/// One queued micro-step. `None`: the queue is empty.
pub fn drain_one<F: Queued>(flight: &mut F, cx: &mut Cx<'_>) -> Option<Poll> {
    let item = flight.queue().front().cloned()?;
    match item {
        QItem::ReleaseT => release_t(cx),
        QItem::TermDiag(fx, terminal) => term_diag(cx, &fx, terminal),
        QItem::Fx(fx, placed) => {
            if is_cross(&placed.effect) {
                if flight.cross(cx, &fx, &placed.effect).is_err() {
                    return Some(Poll::Blocked);
                }
            } else {
                apply(cx, &fx, placed.at, &placed.effect);
                // A completion that follows its terminal claim (PD-20) is the same step: no
                // other actor runs between the claim and the completion.
                if matches!(placed.effect, Effect::Terminal(_)) {
                    let paired = matches!(
                        flight.queue().get(1),
                        Some(QItem::Fx(_, next))
                            if matches!(&next.effect, Effect::PerfComplete(pc) if pc.follows_terminal)
                    );
                    if paired {
                        if let Some(QItem::Fx(paired_fx, next)) = flight.queue().remove(1) {
                            apply(cx, &paired_fx, next.at, &next.effect);
                        }
                    }
                }
                if let Effect::Terminal(terminal) = placed.effect {
                    if placed.at != Placement::In {
                        flight.queue().insert(1, QItem::TermDiag(fx, terminal));
                    }
                }
            }
        }
    }
    flight.queue().pop_front();
    Some(Poll::Progress)
}
