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

/// The supervisor thread: takes the next step of the attempt when no arm runs.
pub struct SupTask;

impl World {
    /// Retire the attempt once every step ran and nothing waits for a command.
    pub fn sup_finish_if_done(&mut self) {
        let done = self
            .sup
            .attempt
            .as_ref()
            .is_some_and(|a| a.prefix.is_empty() && a.end.is_empty() && !a.awaiting_external);
        if done && self.sup.busy.is_none() {
            let attempt = self.sup.attempt.take().expect("checked above");
            self.sup.finished.push((attempt.rid, attempt.ended_by));
        }
    }
}

impl Task for SupTask {
    fn name(&self) -> &'static str {
        "supervisor"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        cx.w.sup_finish_if_done();
        let Some(step) = cx.w.sup.next_step() else {
            return Poll::Blocked;
        };
        let rid =
            cx.w.sup
                .attempt
                .as_ref()
                .expect("a step has an attempt")
                .rid;
        {
            let attempt = cx.w.sup.attempt.as_mut().expect("a step has an attempt");
            if attempt.prefix.front() == Some(&step) {
                attempt.prefix.pop_front();
            } else {
                attempt.end.pop_front();
            }
        }
        match step {
            SupStep::ClearOwner => {
                // `active = false`, `clear_owner`, `set_phase(Idle)`.
                cx.w.sup.owner = None;
                cx.w.sup.phase = None;
                cx.out.notes.push(Note::OwnerCleared { rid });
                cx.w.sup_finish_if_done();
            }
            other => {
                match other {
                    SupStep::Ready => cx.w.sup.phase = Some(Phase::Recording),
                    SupStep::Recovering(_) => cx.w.sup.phase = Some(Phase::Recovering),
                    SupStep::Interrupted(samples) => {
                        cx.w.preserved.insert(rid, Preserved { samples });
                    }
                    _ => {}
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
                let decision = lc::decide_sup_interrupted_short_idle(&view, id, at).after_pre();
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
                if let Some(attempt) = cx.w.sup.attempt.as_mut() {
                    if attempt.rid == rid {
                        match self.step {
                            SupStep::Accepted => attempt.accepted_delivered = true,
                            SupStep::Idle => attempt.fail_delivered = true,
                            _ => {}
                        }
                    }
                }
                cx.out.notes.push(Note::Notified {
                    rid,
                    step: self.step,
                });
                cx.w.sup_finish_if_done();
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
                let decision = lc::decide_sup_accepted_nolock(cx.w.facts(), id, at)
                    .after_prefix(&lc::accepted_prefix());
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
            SupStep::InitFailed { recovery, crash } => {
                let kind = Handle(0);
                let decision = lc::decide_sup_init_failed_nolock(
                    cx.w.facts(),
                    lc::SupInitFailedInput {
                        rid: id,
                        kind,
                        error_code: if crash {
                            ErrCode::BackendError
                        } else {
                            ErrCode::HostUnavailable
                        },
                        recovery_reason: recovery,
                    },
                    at,
                )
                .after_prefix(&lc::init_failed_prefix(kind));
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
            SupStep::ClearOwner => unreachable!("OwnerClear is not a notification"),
        }
        self.st = Bs::Finish;
    }
}
