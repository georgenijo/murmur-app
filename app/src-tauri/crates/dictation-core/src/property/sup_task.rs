//! The supervisor thread as an actor (`SupTask`) and the lifecycle-bridge arm of
//! each notification (`Bridge`, one task per notification, `Sup::busy` while it
//! runs). C7.5: the supervisor, the T-holder and the D sections are separately
//! sequenced actors.

use super::flights::{PipelineCfg, PipelineEnd, StopCfg, StopFlight};
use super::model::*;
use super::task::*;
use crate::decision::{Continuation, ObsKind, SiteId, Verdict};
use crate::effects::{Effect, Placement};
use crate::ids::{Handle, Now, RecordingId};
use crate::inputs::PipelineTerminal;
use crate::lifecycle as lc;
use crate::status::{AudioCancelReason, ErrCode};
use std::collections::VecDeque;

/// The supervisor thread: runs the next step of the message it handles, or takes the
/// next deadline or message when it is between messages (`run_supervisor`).
pub struct SupTask;

/// The frontend window's receiving end: delivers the next status event the shell wrote
/// (D-S2b-17 (b)). Frontend-gated stop callers read what it last delivered, not the
/// shell's current status.
pub struct FrontendTask;

impl Task for FrontendTask {
    fn name(&self) -> &'static str {
        "frontend"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        match cx.w.status_events.pop_front() {
            Some(status) => {
                cx.w.observed = status;
                Poll::Progress
            }
            None => Poll::Blocked,
        }
    }
}

/// The worker thread (`audio.rs`): sends its events into the supervisor's channel.
pub struct WorkerTask;

impl Task for WorkerTask {
    fn name(&self) -> &'static str {
        "worker"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        let coin = (cx.w.step / 2).is_multiple_of(2);
        if cx.w.worker_step(coin) {
            Poll::Progress
        } else {
            Poll::Blocked
        }
    }
}

impl Task for SupTask {
    fn name(&self) -> &'static str {
        "supervisor"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if cx.w.sup.busy.is_some() {
            // An arm runs on this thread.
            return Poll::Blocked;
        }
        let Some(step) = cx.w.sup.handling.pop_front() else {
            // Between messages: a deadline that elapsed, else the next message.
            let coin = cx.w.step.is_multiple_of(2);
            let timer = cx.w.sup_timer(coin);
            if timer == Some(true) {
                return Poll::Progress;
            }
            if !cx.w.sup.queue.is_empty() {
                cx.w.sup_accept();
                return Poll::Progress;
            }
            // A pending deadline that has not elapsed is a tick of time passing.
            return if timer == Some(false) {
                Poll::Progress
            } else {
                Poll::Blocked
            };
        };
        let rid = cx.w.sup.handling_rid;
        match step {
            SupStep::ClearOwner => {
                // `active = false`, `clear_owner`, `set_phase(Idle)`.
                cx.w.sup.owner = None;
                cx.w.sup.phase = None;
                cx.out.notes.push(Note::OwnerCleared { rid });
            }
            SupStep::StartAnswer(tid) => {
                // `request.response.send(..)` (1589-1596, or a refusal 1402-1433): a
                // step of its own, delivered only while the start caller still waits.
                let answer =
                    cx.w.sup
                        .start_pending
                        .remove(&tid)
                        .expect("the acceptance decided this start's response");
                if cx.w.send_reply_pub(tid, ReplyVal::Start) {
                    cx.w.sup.start_answers.insert(rid, answer);
                }
            }
            SupStep::Reply(tid, value) => {
                cx.w.send_reply_pub(tid, value);
            }
            SupStep::CancelAnswer(tid) => {
                // `response.send(Ok(cancelled))`, after `begin_recovery` returned.
                cx.w.send_reply_pub(tid, ReplyVal::Cancel(Ok(true)));
            }
            SupStep::StopAnswer(tid) => {
                // `response.send(Ok(Vec::new()))`, after `begin_recovery` returned.
                cx.w.send_reply_pub(tid, ReplyVal::Stop(Ok(0)));
            }
            other => {
                if let SupStep::Interrupted(samples) = other {
                    // The capture goes to the store; the notification follows (2419-2438).
                    let code =
                        cx.w.sup
                            .finished_fail
                            .get(&rid)
                            .copied()
                            .unwrap_or(ErrCode::RuntimeFailure);
                    cx.w.preserved.insert(rid, Preserved { samples, code });
                }
                cx.w.sup.busy = Some(other);
                cx.w.sup.busy_restored = false;
                cx.spawn.push(Box::new(Bridge::new(rid, other)));
            }
        }
        Poll::Progress
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bs {
    Start,
    Decide,
    ShortIdlePre,
    ShortIdleD,
    Finish,
}

pub struct Bridge {
    rid: Rid,
    step: SupStep,
    st: Bs,
    q: VecDeque<QItem>,
    auto: Option<Continuation>,
}

impl Bridge {
    pub fn new(rid: Rid, step: SupStep) -> Self {
        Self {
            rid,
            step,
            st: Bs::Start,
            q: VecDeque::new(),
            auto: None,
        }
    }

    fn ctx(&self, cx: &Cx<'_>, row: &'static str, site: SiteId) -> FxCtx {
        FxCtx {
            tid: cx.tid,
            rid: self.rid,
            row,
            site,
        }
    }
}

impl Queued for Bridge {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, cx: &mut Cx<'_>, fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        match effect {
            Effect::SelfStop { id, .. } => {
                // `RS`: spawn the stop task for the interrupted capture (R215.2).
                let auto = self.auto.take();
                let cfg = StopCfg {
                    panic_after_cp: false,
                    pipeline: PipelineCfg {
                        work: (id.0 % 3) as u8,
                        end: PipelineEnd::Ok(PipelineTerminal::Success),
                    },
                };
                let _ = fx;
                cx.spawn
                    .push(Box::new(StopFlight::new(Some(id.0), auto, cfg)));
                Ok(())
            }
            other => unreachable!("a bridge arm does not run {other:?}"),
        }
    }
}

impl Task for Bridge {
    fn name(&self) -> &'static str {
        "bridge"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if let Some(poll) = drain_one(self, cx) {
            return poll;
        }
        let at = Now(u64::from(cx.w.step));
        let rid = self.rid;
        let id = RecordingId(rid);
        match self.st {
            Bs::Start => {
                match self.step {
                    SupStep::Accepted => {
                        let ctx = self.ctx(cx, "R212.pre", SiteId::SupAccepted);
                        for placed in lc::accepted_prefix() {
                            self.q.push_back(QItem::Fx(ctx, placed));
                        }
                    }
                    SupStep::InitFailed { .. } => {
                        let ctx = self.ctx(cx, "R213.pre", SiteId::SupInitFailed);
                        for placed in lc::init_failed_prefix(Handle(0)) {
                            self.q.push_back(QItem::Fx(ctx, placed));
                        }
                    }
                    _ => {}
                }
                self.st = Bs::Decide;
            }
            Bs::Decide => self.decide(cx, at, id),
            Bs::ShortIdlePre => {
                let ctx = self.ctx(cx, "R216.pre", SiteId::SupInterruptedShortIdle);
                for placed in lc::short_idle_pre_effects(id) {
                    self.q.push_back(QItem::Fx(ctx, placed));
                }
                self.st = Bs::ShortIdleD;
            }
            Bs::ShortIdleD => {
                let view = cx.w.view();
                let decision = lc::decide_sup_interrupted_short_idle(&view, id, at);
                let decision = after_pre(cx, decision);
                log_decision(cx, SiteId::SupInterruptedShortIdle, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupInterruptedShortIdle);
                apply_in(cx, &ctx, &decision);
                cx.w.sup.busy_restored = true;
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.st = Bs::Finish;
            }
            Bs::Finish => {
                cx.w.sup.busy = None;
                cx.w.sup.busy_restored = false;
                cx.out.notes.push(Note::Notified {
                    rid,
                    step: self.step,
                });
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

impl Bridge {
    fn decide(&mut self, cx: &mut Cx<'_>, at: Now, id: RecordingId) {
        let rid = self.rid;
        match self.step {
            SupStep::Accepted => {
                let decision = lc::decide_sup_accepted_nolock(cx.w.facts(), id, at);
                let decision = after_prefix(cx, decision, &lc::accepted_prefix());
                log_decision(cx, SiteId::SupAccepted, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupAccepted);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
            }
            SupStep::StillConnecting => {
                let decision = lc::decide_sup_still_connecting(&cx.w.view(), id, at);
                log_decision(cx, SiteId::SupStillConnecting, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupStillConnecting);
                apply_in(cx, &ctx, &decision);
            }
            SupStep::Ready => {
                let view = cx.w.view();
                let mut input = lc::SupReadyInput {
                    rid: id,
                    context_present: None,
                    ready_received: at,
                };
                let decision = loop {
                    let decision = lc::decide_sup_ready(&view, input, at);
                    match decision.verdict {
                        Verdict::NeedObservation(ObsKind::ContextPresence { .. })
                            if input.context_present.is_none() =>
                        {
                            input.context_present = Some(cx.w.context_for == Some(rid));
                        }
                        _ => break decision,
                    }
                };
                log_decision(cx, SiteId::SupReady, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupReady);
                apply_in(cx, &ctx, &decision);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
            }
            SupStep::RecoveryStalled => {
                let decision = lc::decide_sup_stalled_nolock(cx.w.facts(), id, at);
                log_decision(cx, SiteId::SupStalled, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupStalled);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
            }
            SupStep::Recovering(reason) => {
                let decision = lc::decide_sup_recovering(
                    &cx.w.view(),
                    lc::SupRecoveringInput { rid: id, reason },
                    at,
                );
                log_decision(cx, SiteId::SupRecovering, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupRecovering);
                apply_in(cx, &ctx, &decision);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
            }
            SupStep::InitFailed { recovery, code } => {
                let kind = Handle(0);
                let decision = lc::decide_sup_init_failed_nolock(
                    cx.w.facts(),
                    lc::SupInitFailedInput {
                        rid: id,
                        kind,
                        error_code: code,
                        recovery_reason: recovery,
                    },
                    at,
                );
                let decision = after_prefix(cx, decision, &lc::init_failed_prefix(kind));
                cx.out.input_code = Some(code);
                log_decision(cx, SiteId::SupInitFailed, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupInitFailed);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
            }
            SupStep::Interrupted(samples) => {
                let decision = lc::decide_sup_interrupted_nolock(
                    cx.w.facts(),
                    lc::SupInterruptedInput {
                        rid: id,
                        delivered_samples: samples,
                        duration_ms: samples as u64 / 16,
                        reason: AudioCancelReason::RuntimeFailure,
                    },
                    at,
                );
                log_decision(cx, SiteId::SupInterrupted, &decision, rid, None);
                if let Some(spec) = decision.continuation {
                    self.auto = Some(cx.w.register(spec));
                }
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupInterrupted);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                if decision.row == lc::R215_3 {
                    self.st = Bs::ShortIdlePre;
                    return;
                }
            }
            SupStep::Idle => {
                let decision = lc::decide_sup_idle(&cx.w.view(), id, at);
                log_decision(cx, SiteId::SupIdle, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupIdle);
                apply_in(cx, &ctx, &decision);
                cx.w.sup.busy_restored = true;
            }
            SupStep::CleanupFinished => {
                let decision = lc::decide_sup_cleanup_finished(&cx.w.view(), id, at);
                log_decision(cx, SiteId::SupCleanupFinished, &decision, rid, None);
                let ctx = self.ctx(cx, decision.row.0, SiteId::SupCleanupFinished);
                apply_in(cx, &ctx, &decision);
                cx.w.sup.busy_restored = true;
            }
            SupStep::ClearOwner
            | SupStep::StartAnswer(_)
            | SupStep::Reply(..)
            | SupStep::CancelAnswer(_)
            | SupStep::StopAnswer(_) => {
                unreachable!("{:?} is not a notification", self.step)
            }
        }
        self.st = Bs::Finish;
    }
}
