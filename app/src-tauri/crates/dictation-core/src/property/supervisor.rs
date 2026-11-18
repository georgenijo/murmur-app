//! The audio supervisor of the model, mirrored from `audio_lifecycle.rs` one step at
//! a time (D-S2b-9 (a)). The supervisor thread owns a FIFO channel and a private
//! `Attempt`; clients (the stop, cancel and start commands) and the worker thread
//! send messages into the channel and the supervisor handles one message at a time.
//!
//! A command has five separate events (D-S2b-9 (a)): the client's **enqueue**
//! (`sup_send_*`, never blocked by a busy supervisor), the supervisor's
//! **acceptance** (`sup_accept`), the reply's **delivery** at the production point
//! (a `send` that succeeds only while the client's receiver is still waiting), the
//! receiver's **timeout** (`sup_poll_reply`) and the client's **continuation**
//! (the poll after the reply or the timeout).
//!
//! # Mirror table
//!
//! Every modeled step cites the function and lines it mirrors in
//! `src/audio_lifecycle.rs`; [`MIRROR`] (`mirror_table.rs`) is that table in data form.
//! Two tests read it. In this crate, a unit test checks that every message, worker event,
//! timer and supervisor step of the model has a row (a step without a row is a defect).
//! In the app crate (`src/audio_lifecycle_mirror_tests.rs`, D-S2b-15), a test checks every
//! row against the source as tokens (the function exists, the cited lines lie inside it,
//! the fragment and every guard occur within them) and pins **every production function
//! of `audio_lifecycle.rs`** by the hash of its normalized token stream: any edit, addition
//! or removal fails there until its pin is updated after re-checking the model step. A
//! reviewer checks a row by reading the cited lines.
//!
//! What is *not* modeled, and why: the speculative and standby phases (an optional
//! feature the dictation commands never enter), peek requests (no state change), the
//! `StartupDiagnostic` notifications (no decision), probe permits.

use super::model::*;
use crate::inputs::CaptureErr;
use crate::status::{AudioCancelReason, ErrCode};
use std::collections::VecDeque;

#[allow(unused_imports)]
pub use super::mirror_table::{Mirror, MIRROR};

/// A start call's answer once the supervisor sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartAnswer {
    Started,
    Failed(CaptureErr),
}

/// What a client polling its reply sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyPoll {
    /// Still waiting (the poll counted a tick of the receiver's timeout).
    Waiting,
    /// The reply was delivered; this poll is the client's continuation.
    Ready(ReplyVal),
    /// `recv_timeout` returned; this poll is the client's continuation.
    TimedOut,
}

fn mix(tid: TaskId, rid: Rid, salt: u64) -> u64 {
    (u64::from(tid)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(rid.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(salt.wrapping_mul(0x1656_67B1_9E37_79F9)))
        >> 29
}

/// How many polls a client's receiver waits before `recv_timeout` returns: often
/// soon, sometimes after a while, otherwise past any reply that can still come.
fn reply_limit(tid: TaskId, rid: Rid, salt: u64) -> u16 {
    let m = mix(tid, rid, salt);
    match m % 6 {
        0 => 1 + (m / 6 % 8) as u16,
        1 => 9 + (m / 6 % 60) as u16,
        _ => 250,
    }
}

impl World {
    // ------------------------------------------------------ client events

    /// `send_start_request`: the client enqueues the start (`audio_lifecycle.rs:3035`).
    pub fn sup_send_start(&mut self, rid: Rid, tid: TaskId, plan: AttemptPlan) {
        let limit = reply_limit(tid, rid, 3);
        self.sup
            .replies
            .insert(tid, ReplyState::Waiting { ticks: 0, limit });
        self.sup.queue.push_back(Msg::Start {
            rid,
            tid,
            plan: Box::new(plan),
        });
    }

    /// `await_start_response`'s timeout continuation (`3067-3078`): the caller flag
    /// is revoked, then a `Cancel{owner}` with its response dropped is enqueued.
    pub fn sup_start_timed_out(&mut self, rid: Rid, tid: TaskId) {
        self.sup.caller_dead.insert(tid);
        self.sup.timed_out_starts.insert(rid);
        self.sup.queue.push_back(Msg::Cancel {
            rid,
            tid: None,
            reason: AudioCancelReason::User,
            starting_only: false,
        });
    }

    /// `stop_with_trigger`: the client enqueues the stop and waits (`3267-3277`).
    pub fn sup_send_stop(&mut self, rid: Rid, tid: TaskId) {
        let limit = reply_limit(tid, rid, 1);
        self.sup
            .replies
            .insert(tid, ReplyState::Waiting { ticks: 0, limit });
        self.sup.queue.push_back(Msg::Stop { rid, tid });
    }

    /// `cancel`: the client enqueues the cancel and waits (`3351-3361`).
    pub fn sup_send_cancel(&mut self, rid: Rid, tid: TaskId) {
        let limit = reply_limit(tid, rid, 2);
        self.sup
            .replies
            .insert(tid, ReplyState::Waiting { ticks: 0, limit });
        self.sup.queue.push_back(Msg::Cancel {
            rid,
            tid: Some(tid),
            reason: AudioCancelReason::User,
            starting_only: false,
        });
    }

    /// The client's wait on its reply. A poll while waiting counts a tick; when the
    /// limit is reached the *timeout* happens (the receiver is dropped, a later
    /// `send` fails) and the *continuation* is the next poll.
    pub fn sup_poll_reply(&mut self, tid: TaskId) -> ReplyPoll {
        match self.sup.replies.get(&tid).copied() {
            Some(ReplyState::Delivered(value)) => {
                self.sup.replies.remove(&tid);
                ReplyPoll::Ready(value)
            }
            Some(ReplyState::TimedOut) => {
                self.sup.replies.remove(&tid);
                ReplyPoll::TimedOut
            }
            Some(ReplyState::Waiting { ticks, limit }) => {
                let ticks = ticks + 1;
                self.sup.replies.insert(
                    tid,
                    if ticks >= limit {
                        ReplyState::TimedOut
                    } else {
                        ReplyState::Waiting { ticks, limit }
                    },
                );
                ReplyPoll::Waiting
            }
            None => unreachable!("a client polled a reply nobody was waiting for"),
        }
    }

    /// `response.send(value)` from a handler step (see [`World::send_reply`]).
    pub fn send_reply_pub(&mut self, tid: TaskId, value: ReplyVal) -> bool {
        self.send_reply(Some(tid), value)
    }

    /// `response.send(value)`: delivered only while the receiver still waits.
    fn send_reply(&mut self, tid: Option<TaskId>, value: ReplyVal) -> bool {
        let Some(tid) = tid else {
            return false;
        };
        match self.sup.replies.get(&tid) {
            Some(ReplyState::Waiting { .. }) => {
                self.sup.replies.insert(tid, ReplyState::Delivered(value));
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------- the supervisor thread

    /// The supervisor takes the next message off its channel and handles it
    /// (`handle_message`): the state it changes at once is set here, the
    /// notifications and answers that follow are the `handling` steps.
    pub fn sup_accept(&mut self) {
        let message = self.sup.queue.pop_front().expect("a message is queued");
        self.sup.handling_rid = match &message {
            Msg::Start { rid, .. }
            | Msg::Stop { rid, .. }
            | Msg::Cancel { rid, .. }
            | Msg::Worker { rid, .. } => *rid,
        };
        match message {
            Msg::Start { rid, tid, plan } => self.on_start(rid, tid, *plan),
            Msg::Stop { rid, tid } => self.on_stop(rid, tid),
            Msg::Cancel {
                rid,
                tid,
                reason,
                starting_only,
            } => self.on_cancel(rid, tid, reason, starting_only),
            Msg::Worker { rid, ev } => self.on_worker(rid, ev),
        }
    }

    /// The response of a start is decided here and sent by its own step.
    fn answer_start(&mut self, tid: TaskId, answer: StartAnswer, then: Option<SupStep>) {
        self.sup.start_pending.insert(tid, answer);
        self.sup.handling = VecDeque::from([SupStep::StartAnswer(tid)]);
        self.sup.handling.extend(then);
    }

    fn on_start(&mut self, rid: Rid, tid: TaskId, plan: AttemptPlan) {
        if self.sup.caller_dead.contains(&tid) {
            // `handle_start` 1402-1405: a revoked caller is refused with `Cancelled`.
            self.answer_start(tid, StartAnswer::Failed(CaptureErr::Other), None);
            return;
        }
        if let Some(current) = &self.sup.attempt {
            // `handle_start` 1425-1433: by the attempt's own phase.
            let error = match current.phase {
                Phase::Starting => CaptureErr::AlreadyStarting,
                Phase::Recording | Phase::Idle => CaptureErr::Other,
                Phase::Stopping | Phase::Recovering => CaptureErr::AudioRecovering,
            };
            self.answer_start(tid, StartAnswer::Failed(error), None);
            return;
        }
        match plan.kind.clone() {
            StartKind::Reject(error) => {
                self.answer_start(tid, StartAnswer::Failed(error), None);
            }
            StartKind::PendingCancelled => {
                self.answer_start(
                    tid,
                    StartAnswer::Failed(CaptureErr::Other),
                    Some(SupStep::Idle),
                );
            }
            StartKind::Fail | StartKind::Live => {
                self.sup.owner = Some(rid);
                self.sup.phase = Some(Phase::Starting);
                self.sup.attempt = Some(Attempt::new(rid, plan));
                self.sup.start_pending.insert(tid, StartAnswer::Started);
                self.sup.handling = VecDeque::from([SupStep::Accepted, SupStep::StartAnswer(tid)]);
            }
        }
    }

    fn on_stop(&mut self, rid: Rid, tid: TaskId) {
        let Some(attempt) = self.sup.attempt.as_mut() else {
            // 1235-1238: no attempt, an empty capture.
            self.sup.handling = VecDeque::from([SupStep::Reply(tid, ReplyVal::Stop(Ok(0)))]);
            return;
        };
        if attempt.rid != rid {
            self.sup.handling = VecDeque::from([SupStep::Reply(tid, ReplyVal::Stop(Err(())))]);
            return;
        }
        match attempt.phase {
            Phase::Starting => {
                attempt.ended_by = EndedBy::Cancel;
                let mut steps = self.begin_recovery(AudioCancelReason::User, None);
                steps.push_back(SupStep::StopAnswer(tid));
                self.sup.handling = steps;
            }
            Phase::Recording => {
                attempt.phase = Phase::Stopping;
                attempt.tail_pending = true;
                attempt.stop_reply = Some(tid);
                attempt.ended_by = EndedBy::Stop(attempt.plan.stop_delivery);
                self.sup.phase = Some(Phase::Stopping);
            }
            Phase::Recovering | Phase::Stopping | Phase::Idle => {
                self.sup.handling = VecDeque::from([SupStep::Reply(tid, ReplyVal::Stop(Err(())))]);
            }
        }
    }

    fn on_cancel(
        &mut self,
        rid: Rid,
        tid: Option<TaskId>,
        reason: AudioCancelReason,
        starting_only: bool,
    ) {
        let abandon = self.sup.attempt.as_ref().is_some_and(|attempt| {
            attempt.rid == rid
                && matches!(attempt.phase, Phase::Starting | Phase::Recording)
                && (!starting_only || attempt.phase == Phase::Starting)
        });
        if !abandon {
            if let Some(tid) = tid {
                self.sup.handling =
                    VecDeque::from([SupStep::Reply(tid, ReplyVal::Cancel(Ok(false)))]);
            }
            return;
        }
        if let Some(attempt) = self.sup.attempt.as_mut() {
            attempt.ended_by = if reason == AudioCancelReason::User {
                EndedBy::Cancel
            } else {
                EndedBy::Recover { interrupted: None }
            };
        }
        let mut steps = self.begin_recovery(reason, None);
        if let Some(tid) = tid {
            steps.push_back(SupStep::CancelAnswer(tid));
        }
        self.sup.handling = steps;
    }

    /// `begin_recovery` (`audio_lifecycle.rs:2630-2698`): the state it sets before
    /// its notification, and the steps that follow.
    fn begin_recovery(
        &mut self,
        reason: AudioCancelReason,
        failure: Option<ErrCode>,
    ) -> VecDeque<SupStep> {
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return VecDeque::new();
        };
        if matches!(attempt.phase, Phase::Recovering | Phase::Stopping) {
            return VecDeque::new();
        }
        attempt.worker.stop_sent = true;
        attempt.phase = Phase::Recovering;
        attempt.recovery_reason = Some(reason);
        self.sup.phase = Some(Phase::Recovering);
        let mut steps = VecDeque::from([SupStep::Recovering(reason)]);
        if let Some(code) = failure.filter(|_| !attempt.failure_reported) {
            attempt.failure_reported = true;
            steps.push_back(SupStep::InitFailed {
                recovery: Some(reason),
                code,
            });
        }
        steps
    }

    fn on_worker(&mut self, rid: Rid, ev: WorkerEv) {
        let Some(attempt) = self.sup.attempt.as_ref() else {
            // 1932-1939: a stale event after the owner was cleared.
            return;
        };
        if attempt.rid != rid {
            return;
        }
        let phase = attempt.phase;
        match ev {
            WorkerEv::FirstBuffer => {
                if phase == Phase::Starting {
                    let attempt = self.sup.attempt.as_mut().expect("checked above");
                    attempt.phase = Phase::Recording;
                    self.sup.phase = Some(Phase::Recording);
                    self.sup.handling = VecDeque::from([SupStep::Ready]);
                }
            }
            WorkerEv::InitFailed => {}
            WorkerEv::RuntimeFailed => {
                if matches!(phase, Phase::Starting | Phase::Recording) {
                    self.mark_recover_end();
                    self.sup.handling =
                        self.begin_recovery(AudioCancelReason::RuntimeFailure, None);
                }
            }
            WorkerEv::TerminationUnconfirmed => {
                if phase == Phase::Stopping {
                    self.report_cleanup_stalled();
                } else {
                    // `begin_recovery` returns at once in `Recovering` (2640-2645): the
                    // attempt then keeps the end it already has.
                    if matches!(phase, Phase::Starting | Phase::Recording) {
                        self.mark_recover_end();
                    }
                    self.sup.handling = self.begin_recovery(
                        AudioCancelReason::RuntimeFailure,
                        Some(ErrCode::TerminationUnconfirmed),
                    );
                }
            }
            WorkerEv::RecordingSealed => {
                let attempt = self.sup.attempt.as_mut().expect("checked above");
                if attempt.phase == Phase::Stopping
                    && !attempt.tail_pending
                    && attempt.stop_reply.is_some()
                {
                    let tid = attempt.stop_reply.take();
                    let samples = attempt.plan.stop_samples;
                    let delivered = self.send_reply(tid, ReplyVal::Stop(Ok(samples)));
                    if let Some(attempt) = self.sup.attempt.as_mut() {
                        attempt.sealed_delivery = delivered;
                    }
                }
            }
            WorkerEv::ThreadExited => self.finish_attempt(),
        }
    }

    fn mark_recover_end(&mut self) {
        if let Some(attempt) = self.sup.attempt.as_mut() {
            let interrupted = match attempt.plan.end {
                End::Recover { interrupted, .. } => interrupted,
                _ => None,
            };
            attempt.ended_by = EndedBy::Recover { interrupted };
        }
    }

    fn report_cleanup_stalled(&mut self) {
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return;
        };
        if attempt.stall_emitted {
            return;
        }
        attempt.stall_emitted = true;
        self.sup.phase = Some(Phase::Recovering);
        self.sup.handling = VecDeque::from([SupStep::RecoveryStalled]);
    }

    /// `finish_attempt` (`audio_lifecycle.rs:2317-2483`): the attempt is taken, a
    /// parked stop response is sent (before any notification), and the steps are the
    /// notifications and the owner clear in production order.
    fn finish_attempt(&mut self) {
        let Some(mut attempt) = self.sup.attempt.take() else {
            return;
        };
        let rid = attempt.rid;
        if attempt.ended_by == EndedBy::Open {
            attempt.ended_by = if attempt.plan.kind == StartKind::Fail {
                EndedBy::Fail
            } else {
                EndedBy::Crash
            };
        }
        let mut steps: VecDeque<SupStep> = VecDeque::new();
        match attempt.phase {
            Phase::Stopping => {
                let delivered = if attempt.sealed_delivery {
                    true
                } else if let Some(tid) = attempt.stop_reply.take() {
                    self.send_reply(Some(tid), ReplyVal::Stop(Ok(attempt.plan.stop_samples)))
                } else {
                    false
                };
                if !delivered {
                    steps.push_back(SupStep::Idle);
                }
                steps.push_back(SupStep::ClearOwner);
                if attempt.sealed_delivery {
                    steps.push_back(SupStep::CleanupFinished);
                }
            }
            Phase::Starting | Phase::Recording => {
                if !attempt.failure_reported {
                    // `failure.take().unwrap_or_else(BackendError)` (`2374-2389`): the
                    // worker's own `InitFailed` kind, else a crash's `BackendError`.
                    steps.push_back(SupStep::InitFailed {
                        recovery: None,
                        code: if attempt.plan.kind == StartKind::Fail {
                            attempt.plan.fail
                        } else {
                            ErrCode::BackendError
                        },
                    });
                }
                steps.push_back(SupStep::Idle);
                steps.push_back(SupStep::ClearOwner);
            }
            Phase::Recovering => {
                match (attempt.recovery_reason, attempt.plan.end) {
                    (
                        Some(AudioCancelReason::RuntimeFailure),
                        End::Recover {
                            interrupted: Some(samples),
                            ..
                        },
                    ) => steps.push_back(SupStep::Interrupted(samples)),
                    _ => steps.push_back(SupStep::Idle),
                }
                steps.push_back(SupStep::ClearOwner);
            }
            Phase::Idle => {}
        }
        self.sup.finished.push((rid, attempt.ended_by));
        self.sup.finished_fail.insert(rid, attempt.plan.fail);
        self.sup.handling = steps;
    }

    /// A deadline the supervisor loop handles (`handle_deadlines_at`). The tail runs
    /// before a queued message; the others only on an empty channel
    /// (`run_supervisor`: a message is received first, a timeout handles deadlines).
    /// `coin` stands for whether the deadline has elapsed yet. `None`: no deadline is
    /// pending; `Some(false)`: one is pending and has not elapsed; `Some(true)`: it
    /// was handled.
    pub fn sup_timer(&mut self, coin: bool) -> Option<bool> {
        let queue_empty = self.sup.queue.is_empty();
        let attempt = self.sup.attempt.as_ref()?;
        self.sup.handling_rid = attempt.rid;
        let phase = attempt.phase;
        let end = attempt.plan.end;
        let live = attempt.plan.kind == StartKind::Live;
        let connecting = attempt.plan.connecting && !attempt.connecting_emitted;
        let stalled = attempt.plan.stalled && !attempt.stall_emitted;
        if phase == Phase::Stopping && attempt.tail_pending {
            if !coin {
                return Some(false);
            }
            let attempt = self.sup.attempt.as_mut().expect("checked above");
            attempt.tail_pending = false;
            attempt.worker.stop_sent = true;
            return Some(true);
        }
        let pending = match phase {
            Phase::Starting => {
                (live
                    && matches!(
                        end,
                        End::Recover {
                            reason: AudioCancelReason::HardDeadline,
                            ..
                        }
                    ))
                    || connecting
            }
            Phase::Stopping | Phase::Recovering => stalled,
            Phase::Recording | Phase::Idle => false,
        };
        if !queue_empty || !pending {
            return None;
        }
        if !coin {
            return Some(false);
        }
        match phase {
            Phase::Starting => {
                if let End::Recover {
                    reason: AudioCancelReason::HardDeadline,
                    init_failed,
                    ..
                } = end
                {
                    if live {
                        if let Some(attempt) = self.sup.attempt.as_mut() {
                            attempt.ended_by = EndedBy::Recover { interrupted: None };
                        }
                        let fail = self.sup.attempt.as_ref().map(|a| a.plan.fail);
                        let steps = self.begin_recovery(
                            AudioCancelReason::HardDeadline,
                            fail.filter(|_| init_failed),
                        );
                        self.sup.handling = steps;
                        return Some(true);
                    }
                }
                if connecting {
                    if let Some(attempt) = self.sup.attempt.as_mut() {
                        attempt.connecting_emitted = true;
                    }
                    self.sup.handling = VecDeque::from([SupStep::StillConnecting]);
                    return Some(true);
                }
                None
            }
            Phase::Stopping | Phase::Recovering => {
                self.report_cleanup_stalled();
                Some(true)
            }
            Phase::Recording | Phase::Idle => None,
        }
    }

    // ------------------------------------------------- the worker thread

    /// The worker thread sends its next event (`audio.rs`), or the environment
    /// cancels a starting attempt. `coin` picks between two events that can come in
    /// either order. Returns whether a message was sent.
    pub fn worker_step(&mut self, coin: bool) -> bool {
        let Some(attempt) = self.sup.attempt.as_mut() else {
            return false;
        };
        let rid = attempt.rid;
        let send = |queue: &mut VecDeque<Msg>, ev| queue.push_back(Msg::Worker { rid, ev });
        let plan_end = attempt.plan.end;
        let kind = attempt.plan.kind.clone();
        let worker = &mut attempt.worker;
        if worker.exited {
            return false;
        }
        // After its own end, or once told to stop, the thread winds down: a seal for
        // a stop that waits for one, then the exit (`audio.rs:3735-3745`, `3865`).
        if worker.stop_sent || worker.own_end {
            if attempt.phase == Phase::Stopping
                && worker.stop_sent
                && !worker.sealed
                && attempt.plan.stop_delivery == Delivery::Sealed
            {
                worker.sealed = true;
                send(&mut self.sup.queue, WorkerEv::RecordingSealed);
                return true;
            }
            worker.exited = true;
            send(&mut self.sup.queue, WorkerEv::ThreadExited);
            return true;
        }
        match kind {
            StartKind::Fail => {
                worker.own_end = true;
                send(&mut self.sup.queue, WorkerEv::InitFailed);
                true
            }
            StartKind::Live => {
                // Retained audio implies a delivered first buffer: the worker appends the
                // PCM to the shared buffer and enqueues `FirstBuffer` before anything it
                // later reports, on the same FIFO (`audio.rs`, the capture callback), so a
                // runtime failure that preserved samples comes after `FirstBuffer`
                // (D-S2b-11 (e)).
                if matches!(
                    plan_end,
                    End::Recover {
                        reason: AudioCancelReason::RuntimeFailure,
                        interrupted: Some(_),
                        ..
                    }
                ) && !worker.first_buffer
                {
                    worker.first_buffer = true;
                    send(&mut self.sup.queue, WorkerEv::FirstBuffer);
                    return true;
                }
                let may_end_first = coin;
                if !worker.first_buffer && attempt.plan.ready && !may_end_first {
                    worker.first_buffer = true;
                    send(&mut self.sup.queue, WorkerEv::FirstBuffer);
                    return true;
                }
                match plan_end {
                    End::External => {
                        if !worker.first_buffer && attempt.plan.ready {
                            worker.first_buffer = true;
                            send(&mut self.sup.queue, WorkerEv::FirstBuffer);
                            return true;
                        }
                        false
                    }
                    End::Crash => {
                        worker.own_end = true;
                        worker.exited = true;
                        send(&mut self.sup.queue, WorkerEv::ThreadExited);
                        true
                    }
                    End::Recover {
                        reason: AudioCancelReason::RuntimeFailure,
                        init_failed,
                        ..
                    } => {
                        worker.own_end = true;
                        send(
                            &mut self.sup.queue,
                            if init_failed {
                                WorkerEv::TerminationUnconfirmed
                            } else {
                                WorkerEv::RuntimeFailed
                            },
                        );
                        true
                    }
                    End::Recover {
                        reason: AudioCancelReason::HardDeadline,
                        ..
                    } => {
                        // The deadline is the supervisor's own (`Timer::HardDeadline`).
                        if !worker.first_buffer && attempt.plan.ready {
                            worker.first_buffer = true;
                            send(&mut self.sup.queue, WorkerEv::FirstBuffer);
                            return true;
                        }
                        false
                    }
                    End::Recover { reason, .. } => {
                        // A device or power change cancels a *starting* attempt only
                        // (`cancel_starting_for_environment_change`).
                        if attempt.phase == Phase::Starting {
                            worker.own_end = true;
                            self.sup.queue.push_back(Msg::Cancel {
                                rid,
                                tid: None,
                                reason,
                                starting_only: true,
                            });
                            return true;
                        }
                        false
                    }
                }
            }
            StartKind::Reject(_) | StartKind::PendingCancelled => false,
        }
    }
}

impl Attempt {
    pub fn new(rid: Rid, plan: AttemptPlan) -> Self {
        Self {
            rid,
            plan,
            phase: Phase::Starting,
            recovery_reason: None,
            stop_reply: None,
            sealed_delivery: false,
            tail_pending: false,
            stall_emitted: false,
            connecting_emitted: false,
            failure_reported: false,
            worker: Worker::default(),
            ended_by: EndedBy::Open,
        }
    }
}
