//! The supervisor thread's side of the model: the commands it accepts (start,
//! stop, cancel) and the steps of the attempt word it takes between commands.
//!
//! One thread, so a command is handled only between steps: while a
//! notification's bridge arm runs (`Sup::busy`), or `finish_attempt`'s group runs
//! (`Attempt::finishing`), a command waits. Everything else competes with the
//! commands: the worker's own end (a recovery, a crash, a failed open) is a
//! `PendingEnd` that has not happened until the supervisor takes it, and a stop or
//! a cancel that arrives first wins it (D-S2b-7 (b)).

use super::model::*;
use crate::inputs::CaptureErr;
use crate::status::AudioCancelReason;
use std::collections::VecDeque;

/// A start call's answer once the supervisor sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartAnswer {
    Started,
    Failed(CaptureErr),
}

/// The result of a command that the single supervisor thread has not reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

/// A capture cancel's reply: now, or after `begin_recovery` returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReply {
    Now(Result<bool, ()>),
    /// The answer is the attempt's `CancelAnswer` step (`Sup::cancel_answers`).
    Pending,
}

impl World {
    /// The supervisor thread cannot take a command: a bridge arm runs, or
    /// `finish_attempt`'s group is in progress (it does not return to the command
    /// loop until `OwnerClear` and what follows ran).
    pub fn sup_busy(&self) -> bool {
        self.sup.busy.is_some() || self.sup.attempt.as_ref().is_some_and(|a| a.finishing)
    }

    /// `start_dictation_recording`: the supervisor accepts the command (between
    /// two of its steps) and builds the attempt's word. The answer follows as its
    /// own step (`SupStep::Answer`) once `Accepted` was notified.
    pub fn sup_start(&mut self, rid: Rid, plan: AttemptPlan) -> Result<(), Busy> {
        if self.sup_busy() {
            return Err(Busy);
        }
        if self.sup.owner.is_some() {
            // The owner is still set: `AlreadyStarting`, or `AudioRecovering` while
            // the previous capture retires.
            let error = if self.sup.phase() == Phase::Recovering {
                CaptureErr::AudioRecovering
            } else {
                CaptureErr::AlreadyStarting
            };
            self.sup
                .start_answers
                .insert(rid, StartAnswer::Failed(error));
            return Ok(());
        }
        let kind = plan.kind.clone();
        match kind.clone() {
            StartKind::Reject(error) => {
                self.sup
                    .start_answers
                    .insert(rid, StartAnswer::Failed(error));
                return Ok(());
            }
            StartKind::PendingCancelled => {
                // The response `Err(Cancelled)` first, then `Idle`, with no
                // `Accepted` and no owner (809, 825, 1152-1156).
                self.sup
                    .start_answers
                    .insert(rid, StartAnswer::Failed(CaptureErr::Other));
                self.sup.attempt = Some(Attempt::new(
                    rid,
                    plan.clone(),
                    VecDeque::from([SupStep::Idle]),
                ));
                return Ok(());
            }
            StartKind::Fail | StartKind::Live => {}
        }
        self.sup.owner = Some(rid);
        self.sup.phase = Some(Phase::Starting);
        let mut prefix = VecDeque::from([SupStep::Accepted, SupStep::Answer]);
        for _ in 0..plan.connecting {
            prefix.push_back(SupStep::StillConnecting);
        }
        let mut attempt;
        if kind == StartKind::Fail {
            attempt = Attempt::new(rid, plan.clone(), prefix);
            // The worker fails to open: `finish_attempt` in `Starting`.
            attempt.pending_end = Some(PendingEnd {
                begin: VecDeque::new(),
                finish: VecDeque::from([
                    SupStep::InitFailed {
                        recovery: None,
                        crash: false,
                    },
                    SupStep::Idle,
                    SupStep::ClearOwner,
                ]),
                ended_by: EndedBy::Fail,
            });
        } else {
            if plan.ready {
                prefix.push_back(SupStep::Ready);
            }
            attempt = Attempt::new(rid, plan.clone(), prefix);
            match plan.end {
                End::External => attempt.awaiting_external = true,
                End::Recover {
                    reason,
                    init_failed,
                    interrupted,
                } => {
                    let mut begin = VecDeque::from([SupStep::Recovering(reason)]);
                    if init_failed {
                        begin.push_back(SupStep::InitFailed {
                            recovery: Some(reason),
                            crash: false,
                        });
                    }
                    if plan.stalled > 0 {
                        begin.push_back(SupStep::RecoveryStalled);
                    }
                    let finish = VecDeque::from([
                        match interrupted {
                            Some(samples) => SupStep::Interrupted(samples),
                            None => SupStep::Idle,
                        },
                        SupStep::ClearOwner,
                    ]);
                    attempt.pending_end = Some(PendingEnd {
                        begin,
                        finish,
                        ended_by: EndedBy::Recover { interrupted },
                    });
                }
                End::Crash => {
                    attempt.pending_end = Some(PendingEnd {
                        begin: VecDeque::new(),
                        finish: VecDeque::from([
                            SupStep::InitFailed {
                                recovery: None,
                                crash: true,
                            },
                            SupStep::Idle,
                            SupStep::ClearOwner,
                        ]),
                        ended_by: EndedBy::Crash,
                    });
                }
            }
        }
        self.sup.attempt = Some(attempt);
        Ok(())
    }

    /// The worker begins its own end (a `begin_recovery` or a thread exit): only
    /// when nothing else decided one.
    pub fn sup_begin_pending_end(&mut self) {
        let attempt = self.sup.attempt.as_mut().expect("can_begin_end checked");
        let pending = attempt.pending_end.take().expect("can_begin_end checked");
        attempt.end_decided = true;
        attempt.awaiting_external = false;
        attempt.ended_by = pending.ended_by;
        attempt.begin = pending.begin;
        attempt.finish = pending.finish;
    }

    /// `stop_dictation_recording`: `Ok((samples, delivery))`, or `Err` when the
    /// supervisor has no capture to stop or its stop times out.
    pub fn sup_stop(&mut self, rid: Rid) -> Result<Result<(usize, Delivery), ()>, Busy> {
        if self.sup_busy() {
            return Err(Busy);
        }
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return Ok(Err(()));
        };
        if attempt.rid != rid
            || self.sup.owner != Some(rid)
            || self.sup.phase != Some(Phase::Recording)
            || attempt.end_decided
        {
            return Ok(Err(()));
        }
        attempt.end_decided = true;
        attempt.awaiting_external = false;
        attempt.stop_called = true;
        attempt.prefix.clear();
        attempt.pending_end = None;
        let delivery = attempt.plan.stop_delivery;
        attempt.ended_by = EndedBy::Stop(delivery);
        attempt.begin = if attempt.plan.stalled > 0 {
            VecDeque::from([SupStep::RecoveryStalled])
        } else {
            VecDeque::new()
        };
        attempt.finish = match delivery {
            Delivery::Sealed => VecDeque::from([SupStep::ClearOwner, SupStep::CleanupFinished]),
            Delivery::Unsealed => VecDeque::from([SupStep::ClearOwner]),
            Delivery::Undelivered => VecDeque::from([SupStep::Idle, SupStep::ClearOwner]),
        };
        let samples = attempt.plan.stop_samples;
        self.sup.phase = Some(Phase::Stopping);
        Ok(match delivery {
            Delivery::Undelivered => Err(()),
            other => Ok((samples, other)),
        })
    }

    /// `cancel_dictation_capture(rid, User)`: the supervisor takes the cancel
    /// (phase `Recovering` at once, then `Recovering{User}`, the answer, the
    /// attempt's own `Idle`), or has nothing to cancel.
    pub fn sup_cancel(&mut self, rid: Rid) -> Result<CancelReply, Busy> {
        if self.sup_busy() {
            return Err(Busy);
        }
        let phase = self.sup.phase();
        let owner = self.sup.owner;
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return Ok(CancelReply::Now(Ok(false)));
        };
        if attempt.plan.cancel == CancelMode::Unavailable {
            return Ok(CancelReply::Now(Err(())));
        }
        if attempt.rid != rid
            || owner != Some(rid)
            || !matches!(phase, Phase::Starting | Phase::Recording)
            || attempt.end_decided
        {
            return Ok(CancelReply::Now(Ok(false)));
        }
        attempt.end_decided = true;
        attempt.awaiting_external = false;
        attempt.cancelled = true;
        attempt.ended_by = EndedBy::Cancel;
        attempt.prefix.clear();
        attempt.pending_end = None;
        let mut begin = VecDeque::from([
            SupStep::Recovering(AudioCancelReason::User),
            SupStep::CancelAnswer,
        ]);
        if attempt.plan.stalled > 0 {
            begin.push_back(SupStep::RecoveryStalled);
        }
        attempt.begin = begin;
        attempt.finish = VecDeque::from([SupStep::Idle, SupStep::ClearOwner]);
        let timed_out = attempt.plan.cancel == CancelMode::Timeout;
        self.sup.cancel_answers.remove(&rid);
        // `begin_recovery` answers a start that has not been answered with `Cancelled`.
        self.sup
            .start_answers
            .entry(rid)
            .or_insert(StartAnswer::Failed(CaptureErr::Other));
        self.sup.phase = Some(Phase::Recovering);
        Ok(if timed_out {
            CancelReply::Now(Err(()))
        } else {
            CancelReply::Pending
        })
    }
}

impl Attempt {
    pub fn new(rid: Rid, plan: AttemptPlan, prefix: VecDeque<SupStep>) -> Self {
        Self {
            rid,
            plan,
            prefix,
            pending_end: None,
            begin: VecDeque::new(),
            finish: VecDeque::new(),
            end_decided: false,
            finishing: false,
            awaiting_external: false,
            stop_called: false,
            cancelled: false,
            ended_by: EndedBy::Open,
        }
    }
}
