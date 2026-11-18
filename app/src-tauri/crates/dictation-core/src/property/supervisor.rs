//! The supervisor thread's side of the model: the commands it accepts (start,
//! stop, cancel) and the steps of the attempt word it takes between commands.
//!
//! One thread, so a command is handled only between steps: while a
//! notification's bridge arm runs (`Sup::busy`), a command waits.

use super::model::*;
use crate::inputs::CaptureErr;
use crate::status::AudioCancelReason;
use std::collections::VecDeque;

/// A start call's answer once its acknowledgement is out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartAnswer {
    Started,
    Failed(CaptureErr),
}

/// The result of a command that the single supervisor thread has not reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

impl World {
    /// The supervisor thread cannot take a command: a bridge arm runs, or an attempt
    /// that decided its end is still working through it (`finish_attempt` does not
    /// return to the command loop until `OwnerClear` and the notifications ran).
    pub fn sup_busy(&self) -> bool {
        self.sup.busy.is_some()
            || self
                .sup
                .attempt
                .as_ref()
                .is_some_and(|a| a.end_decided && !(a.prefix.is_empty() && a.end.is_empty()))
    }

    /// `start_dictation_recording`: the supervisor accepts the command (between
    /// two of its steps) and builds the attempt's word. The answer follows once
    /// the word's acknowledgement notifications ran (`Sup::ack_ready`).
    pub fn sup_start(&mut self, rid: Rid, plan: AttemptPlan) -> Result<StartAnswer, Busy> {
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
            return Ok(StartAnswer::Failed(error));
        }
        let kind = plan.kind.clone();
        match kind.clone() {
            StartKind::Reject(error) => return Ok(StartAnswer::Failed(error)),
            StartKind::PendingCancelled => {
                // `Idle` with no `Accepted`, and no owner (809, 825, 1156).
                self.sup.attempt = Some(Attempt::new(
                    rid,
                    plan.clone(),
                    VecDeque::from([SupStep::Idle]),
                ));
                return Ok(StartAnswer::Failed(CaptureErr::Other));
            }
            StartKind::Fail | StartKind::Live => {}
        }
        self.sup.owner = Some(rid);
        self.sup.phase = Some(Phase::Starting);
        let mut prefix = VecDeque::from([SupStep::Accepted]);
        for _ in 0..plan.connecting {
            prefix.push_back(SupStep::StillConnecting);
        }
        let mut attempt;
        if kind == StartKind::Fail {
            prefix.push_back(SupStep::InitFailed {
                recovery: None,
                crash: false,
            });
            prefix.push_back(SupStep::Idle);
            prefix.push_back(SupStep::ClearOwner);
            attempt = Attempt::new(rid, plan.clone(), prefix);
            attempt.end_decided = true;
            attempt.ended_by = EndedBy::Fail;
        } else {
            if plan.ready {
                prefix.push_back(SupStep::Ready);
                for _ in 0..plan.stalled {
                    prefix.push_back(SupStep::RecoveryStalled);
                }
            }
            attempt = Attempt::new(rid, plan.clone(), prefix);
            match plan.end {
                End::External => attempt.awaiting_external = true,
                End::Recover {
                    reason,
                    init_failed,
                    stalled,
                    interrupted,
                } => {
                    let mut end = VecDeque::from([SupStep::Recovering(reason)]);
                    if init_failed {
                        end.push_back(SupStep::InitFailed {
                            recovery: Some(reason),
                            crash: false,
                        });
                    }
                    for _ in 0..stalled {
                        end.push_back(SupStep::RecoveryStalled);
                    }
                    end.push_back(match interrupted {
                        Some(samples) => SupStep::Interrupted(samples),
                        None => SupStep::Idle,
                    });
                    end.push_back(SupStep::ClearOwner);
                    attempt.end = end;
                    attempt.end_decided = true;
                    attempt.ended_by = EndedBy::Recover { interrupted };
                }
                End::Crash => {
                    attempt.end = VecDeque::from([
                        SupStep::InitFailed {
                            recovery: None,
                            crash: true,
                        },
                        SupStep::Idle,
                        SupStep::ClearOwner,
                    ]);
                    attempt.end_decided = true;
                    attempt.ended_by = EndedBy::Crash;
                }
            }
        }
        self.sup.attempt = Some(attempt);
        Ok(StartAnswer::Started)
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
        let delivery = attempt.plan.stop_delivery;
        attempt.ended_by = EndedBy::Stop(delivery);
        attempt.end = match delivery {
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

    /// `cancel_dictation_capture(rid, User)`: `Ok(true)` when the supervisor
    /// took the cancel (phase `Recovering` at once, `Recovering{User}` and the
    /// attempt's own `Idle` follow), `Ok(false)` when it had nothing to cancel.
    pub fn sup_cancel(&mut self, rid: Rid) -> Result<Result<bool, ()>, Busy> {
        if self.sup_busy() {
            return Err(Busy);
        }
        let phase = self.sup.phase();
        let owner = self.sup.owner;
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return Ok(Ok(false));
        };
        if attempt.plan.cancel_err {
            return Ok(Err(()));
        }
        if attempt.rid != rid
            || owner != Some(rid)
            || !matches!(phase, Phase::Starting | Phase::Recording)
            || attempt.end_decided
        {
            return Ok(Ok(false));
        }
        attempt.end_decided = true;
        attempt.awaiting_external = false;
        attempt.cancelled = true;
        attempt.ended_by = EndedBy::Cancel;
        attempt.prefix.clear();
        attempt.end = VecDeque::from([
            SupStep::Recovering(AudioCancelReason::User),
            SupStep::Idle,
            SupStep::ClearOwner,
        ]);
        self.sup.phase = Some(Phase::Recovering);
        Ok(Ok(true))
    }
}

impl Attempt {
    pub fn new(rid: Rid, plan: AttemptPlan, prefix: VecDeque<SupStep>) -> Self {
        Self {
            rid,
            plan,
            prefix,
            end: VecDeque::new(),
            end_decided: false,
            awaiting_external: false,
            stop_called: false,
            cancelled: false,
            ended_by: EndedBy::Open,
            accepted_delivered: false,
            fail_delivered: false,
        }
    }
}
