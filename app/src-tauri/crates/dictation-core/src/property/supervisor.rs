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
//! `src/audio_lifecycle.rs`; [`MIRROR`] is that table in data form and a unit test
//! checks every row against the source (the function is declared at the cited
//! line) and checks that every message, worker event, timer and supervisor step of
//! the model has a row. A step without a row is a defect. A reviewer checks a row
//! by reading the cited lines. Beyond the lines, every cited function is pinned **whole**
//! by the hash of its normalized token stream (`mirror_pins.rs`, D-S2b-13 (a)): any edit of
//! one fails the mirror test until its row is re-pinned after re-checking the model step.
//!
//! What is *not* modeled, and why: the speculative and standby phases (an optional
//! feature the dictation commands never enter), peek requests (no state change), the
//! `StartupDiagnostic` notifications (no decision), probe permits.

use super::model::*;
use crate::inputs::CaptureErr;
use crate::status::{AudioCancelReason, ErrCode};
use std::collections::VecDeque;

/// One row of the mirror table.
#[derive(Clone, Copy, Debug)]
pub struct Mirror {
    /// The model step (a `Msg` / `WorkerEv` / `SupStep` / timer name).
    pub step: &'static str,
    /// The source file under `src/`: `audio_lifecycle.rs`, or `audio.rs` for the
    /// worker thread's own end.
    pub file: &'static str,
    /// The function in that file.
    pub function: &'static str,
    /// The cited lines, inclusive.
    pub lines: (u32, u32),
    /// A fragment of source that must appear within the cited lines (the test reads
    /// the source, so a citation that drifted or never matched fails).
    pub needle: &'static str,
    /// What the model does there.
    pub what: &'static str,
    /// The guard expressions of the cited code, each a fragment that must appear in the
    /// cited lines: a model step is only as faithful as the condition it runs under, so
    /// a mutated guard (`is_none()` turned into `is_some()`, a dropped clause) fails the
    /// mirror test as well as the real test below (D-S2b-11 (c)).
    pub guards: &'static [&'static str],
    /// A test of the real supervisor in `audio_lifecycle.rs` that pins the behaviour by
    /// running it (the test must exist there).
    pub real_test: Option<&'static str>,
}

/// The mirror table: model step -> `audio_lifecycle.rs` function and lines.
pub const MIRROR: &[Mirror] = &[
    Mirror {
        step: "Msg::Start (client enqueue)",
        needle: "SupervisorMessage::Start",
        file: "audio_lifecycle.rs",
        function: "send_start_request",
        lines: (3023, 3056),
        guards: &[],
        real_test: None,
        what: "the client sends the request and waits; never blocked by the supervisor",
    },
    Mirror {
        step: "Msg::Start (accept, an attempt exists)",
        needle: "AudioStartError::AlreadyStarting",
        file: "audio_lifecycle.rs",
        function: "handle_start",
        lines: (1425, 1433),
        guards: &[],
        real_test: None,
        what: "answers AlreadyStarting / AlreadyRecording / AudioRecovering by the attempt's own phase",
    },
    Mirror {
        step: "Msg::Start (accept, refused)",
        needle: "AudioStartError::Cancelled",
        file: "audio_lifecycle.rs",
        function: "handle_start",
        lines: (1402, 1413),
        guards: &[
            "!request.caller_alive.load(Ordering::Acquire)",
            "response.send(Err(AudioStartError::Cancelled))",
        ],
        real_test: None,
        what: "a dead caller or an invalid permit: response Err, no owner, no notification",
    },
    Mirror {
        step: "Msg::Start (accept, pending cancelled)",
        needle: "AudioStartError::Cancelled",
        file: "audio_lifecycle.rs",
        function: "run_supervisor",
        lines: (1148, 1160),
        guards: &[],
        real_test: None,
        what: "a queued handoff start with the app disabled: Err(Cancelled), then Idle, no Accepted",
    },
    Mirror {
        step: "Msg::Start (accept, live)",
        needle: "public.set_phase(PublicPhase::Starting)",
        file: "audio_lifecycle.rs",
        function: "handle_start",
        lines: (1481, 1546),
        guards: &[],
        real_test: None,
        what: "set_owner, set_phase(Starting), the attempt installed",
    },
    Mirror {
        step: "SupStep::Accepted",
        needle: "AudioLifecycleEvent::Accepted",
        file: "audio_lifecycle.rs",
        function: "handle_start",
        lines: (1584, 1588),
        guards: &[],
        real_test: None,
        what: "notify(Accepted)",
    },
    Mirror {
        step: "SupStep::StartAnswer",
        needle: "response.send(Ok(()))",
        file: "audio_lifecycle.rs",
        function: "handle_start",
        lines: (1589, 1596),
        guards: &[],
        real_test: None,
        what: "response.send(Ok(())) after Accepted (wait_until_ready is false for dictation); a refusal's response is a step of its own too",
    },
    Mirror {
        step: "SupStep::Reply",
        needle: "response.send(Err(\"Audio owner changed",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1239, 1242),
        guards: &[],
        real_test: None,
        what: "an immediate stop or cancel response (Ok(Vec::new()), Err, Ok(false)), sent after the message was accepted",
    },
    Mirror {
        step: "Msg::Start (client timeout / continuation)",
        needle: "caller_alive.store(false",
        file: "audio_lifecycle.rs",
        function: "await_start_response",
        lines: (3065, 3079),
        guards: &[],
        real_test: None,
        what: "recv_timeout: a delivered response, or caller_alive=false then Cancel{owner, User, starting_only: false} with the response dropped, and SupervisorUnavailable",
    },
    Mirror {
        step: "Msg::Stop (client enqueue)",
        needle: "SupervisorMessage::Stop",
        file: "audio_lifecycle.rs",
        function: "stop_with_trigger",
        lines: (3261, 3275),
        guards: &[],
        real_test: None,
        what: "the client sends Stop{owner} and waits STOP_RESPONSE_TIMEOUT; never blocked",
    },
    Mirror {
        step: "Msg::Stop (client timeout / continuation)",
        needle: "recv_timeout(STOP_RESPONSE_TIMEOUT)",
        file: "audio_lifecycle.rs",
        function: "stop_with_trigger",
        lines: (3276, 3287),
        guards: &[],
        real_test: None,
        what: "recv_timeout: a delivered reply, or the timeout error (the receiver is dropped)",
    },
    Mirror {
        step: "Msg::Stop (accept, no attempt)",
        needle: "response.send(Ok(Vec::new()))",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1235, 1238),
        guards: &[],
        real_test: None,
        what: "an empty capture: response Ok(Vec::new()), not Err",
    },
    Mirror {
        step: "Msg::Stop (accept, other owner)",
        needle: "Audio owner changed before stop",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1239, 1242),
        guards: &[],
        real_test: None,
        what: "response Err(owner changed)",
    },
    Mirror {
        step: "Msg::Stop (accept, Starting)",
        needle: "begin_recovery(attempt, AudioCancelReason::User",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1243, 1247),
        guards: &[],
        real_test: None,
        what: "begin_recovery(User), then response Ok(Vec::new()) (SupStep::StopAnswer)",
    },
    Mirror {
        step: "Msg::Stop (accept, Recording)",
        needle: "current.phase = AttemptPhase::Stopping",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1250, 1286),
        guards: &[
            "current.tail_deadline = Some(deadline)",
            "current.stop_response = Some(response)",
        ],
        real_test: None,
        what: "phase Stopping, the tail deadline set, the response parked, set_phase(Stopping)",
    },
    Mirror {
        step: "Msg::Stop (accept, Recovering or Stopping)",
        needle: "Audio is already stopping",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1291, 1298),
        guards: &[],
        real_test: None,
        what: "response Err(still recovering / already stopping)",
    },
    Mirror {
        step: "Msg::Cancel (client enqueue)",
        needle: "SupervisorMessage::Cancel",
        file: "audio_lifecycle.rs",
        function: "cancel",
        lines: (3345, 3359),
        guards: &[],
        real_test: None,
        what: "the client sends Cancel{owner, reason} and waits 2 s; never blocked",
    },
    Mirror {
        step: "Msg::Cancel (client timeout / continuation)",
        needle: "recv_timeout",
        file: "audio_lifecycle.rs",
        function: "cancel",
        lines: (3360, 3363),
        guards: &[],
        real_test: None,
        what: "recv_timeout: a delivered reply, or Err(did not acknowledge)",
    },
    Mirror {
        step: "Msg::Cancel (accept)",
        needle: "should_abandon",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1301, 1337),
        guards: &[
            "let owner_matches = owner.is_none_or(|owner| owner == current.owner)",
            "AttemptPhase::Starting",
            "let _ = response.send(Ok(cancelled));",
        ],
        real_test: Some("a_cancel_from_recording_stops_capture_and_answers_true"),
        what: "should_abandon in Starting|Recording, begin_recovery, then response Ok(cancelled); from Recovering or Stopping nothing is abandoned (a_cancel_from_recovering_has_nothing_to_abandon)",
    },
    Mirror {
        step: "begin_recovery",
        needle: "AudioLifecycleEvent::Recovering",
        file: "audio_lifecycle.rs",
        function: "begin_recovery",
        lines: (2630, 2698),
        guards: &[],
        real_test: None,
        what: "phase Recovering, send_worker_stop, set_phase(Recovering), Recovering notification, failure report",
    },
    Mirror {
        step: "SupStep::Recovering",
        needle: "AudioLifecycleEvent::Recovering { reason }",
        file: "audio_lifecycle.rs",
        function: "begin_recovery",
        lines: (2683, 2689),
        guards: &[],
        real_test: None,
        what: "notify(Recovering{reason})",
    },
    Mirror {
        step: "SupStep::CancelAnswer",
        needle: "response.send(Ok(cancelled))",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1335, 1336),
        guards: &[],
        real_test: None,
        what: "response.send(Ok(cancelled)) after begin_recovery returned",
    },
    Mirror {
        step: "SupStep::StopAnswer",
        needle: "response.send(Ok(Vec::new()))",
        file: "audio_lifecycle.rs",
        function: "handle_message",
        lines: (1244, 1245),
        guards: &[],
        real_test: None,
        what: "response.send(Ok(Vec::new())) after begin_recovery returned",
    },
    Mirror {
        step: "WorkerEv::FirstBuffer",
        needle: "current.phase = AttemptPhase::Recording",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2134, 2205),
        guards: &[],
        real_test: None,
        what: "Starting -> Recording, set_phase(Recording), notify(Ready); a duplicate is ignored (2206-2215)",
    },
    Mirror {
        step: "SupStep::Ready",
        needle: "AudioLifecycleEvent::Ready",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2158, 2162),
        guards: &[],
        real_test: None,
        what: "notify(Ready)",
    },
    Mirror {
        step: "WorkerEv::InitFailed",
        needle: "current.failure = Some(failure.clone())",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2217, 2235),
        guards: &[],
        real_test: None,
        what: "the failure is stored; the worker thread exits on its own",
    },
    Mirror {
        step: "WorkerEv::RuntimeFailed",
        needle: "AudioCancelReason::RuntimeFailure",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2236, 2278),
        guards: &[],
        real_test: None,
        what: "Starting|Recording: begin_recovery(RuntimeFailure) with no failure report for a dictation",
    },
    Mirror {
        step: "WorkerEv::TerminationUnconfirmed",
        needle: "report_cleanup_stalled",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2036, 2065),
        guards: &[
            "if current.phase == AttemptPhase::Stopping",
            "AudioCancelReason::RuntimeFailure",
        ],
        real_test: None,
        what: "Stopping: report_cleanup_stalled; else begin_recovery(RuntimeFailure, Some(failure))",
    },
    Mirror {
        step: "WorkerEv::RecordingSealed",
        needle: "current.sealed_delivery",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2279, 2302),
        guards: &[
            "current.phase == AttemptPhase::Stopping",
            "current.owner.dictation_id().is_some()",
            "current.tail_deadline.is_none()",
            "current.stop_response.is_some()",
            "response.send(result).is_ok()",
        ],
        real_test: Some("a_sealed_stop_reply_is_delivered_at_the_seal_before_worker_exit"),
        what: "Stopping, no tail, response parked: take_samples, response.send, sealed_delivery = send ok",
    },
    Mirror {
        step: "WorkerEv::ThreadExited",
        needle: "finish_attempt(attempt, sink, public)",
        file: "audio_lifecycle.rs",
        function: "handle_worker_event",
        lines: (2311, 2313),
        guards: &[],
        real_test: None,
        what: "finish_attempt",
    },
    Mirror {
        step: "finish_attempt (Stopping)",
        needle: "AudioLifecycleEvent::Idle",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2358, 2373),
        guards: &[
            "if finished.sealed_delivery",
            "response.send(take_samples(&mut finished)).is_ok()",
            "!response_delivered && !idle_after_clear",
        ],
        real_test: Some("sealed_delivery_overlaps_exit_but_retains_exclusive_owner_and_processing"),
        what: "deliver the parked response unless sealed; if not delivered, notify(Idle)",
    },
    Mirror {
        step: "finish_attempt (Starting)",
        needle: "AudioLifecycleEvent::Idle",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2374, 2389),
        guards: &[],
        real_test: None,
        what: "report the failure (InitializationFailed), then notify(Idle)",
    },
    Mirror {
        step: "finish_attempt (Recording)",
        needle: "AudioLifecycleEvent::Idle",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2390, 2403),
        guards: &[],
        real_test: None,
        what: "report BackendError (InitializationFailed), then notify(Idle)",
    },
    Mirror {
        step: "finish_attempt (Recovering)",
        needle: "AudioLifecycleEvent::Interrupted",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2404, 2456),
        guards: &[],
        real_test: None,
        what: "RuntimeFailure with samples: store the capture, then notify(Interrupted); else notify(Idle)",
    },
    Mirror {
        step: "SupStep::InitFailed",
        needle: "AudioLifecycleEvent::InitializationFailed",
        file: "audio_lifecycle.rs",
        function: "report_failure_once",
        lines: (2734, 2758),
        guards: &[],
        real_test: None,
        what: "notify(InitializationFailed{recovery_reason}), once",
    },
    Mirror {
        step: "SupStep::Interrupted",
        needle: "interrupted_captures()",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2416, 2438),
        guards: &[],
        real_test: None,
        what: "the capture goes to the store (2419-2429), then notify(Interrupted) (2430-2438)",
    },
    Mirror {
        step: "SupStep::Idle",
        needle: "AudioLifecycleEvent::Idle",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2366, 2372),
        guards: &[],
        real_test: None,
        what: "notify(Idle)",
    },
    Mirror {
        step: "SupStep::ClearOwner",
        needle: "public.clear_owner(finished.owner)",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2458, 2462),
        guards: &[],
        real_test: None,
        what: "active = false, clear_owner, set_phase(Idle)",
    },
    Mirror {
        step: "SupStep::CleanupFinished",
        needle: "AudioLifecycleEvent::CleanupFinished",
        file: "audio_lifecycle.rs",
        function: "finish_attempt",
        lines: (2463, 2469),
        guards: &[],
        real_test: None,
        what: "notify(CleanupFinished) after the owner clear, for a sealed delivery",
    },
    Mirror {
        step: "Timer::Tail",
        needle: "send_worker_stop",
        file: "audio_lifecycle.rs",
        function: "handle_deadlines_at",
        lines: (2818, 2837),
        guards: &[],
        real_test: None,
        what: "the tail deadline: send_worker_stop (the worker then seals)",
    },
    Mirror {
        step: "Timer::StillConnecting",
        needle: "AudioLifecycleEvent::StillConnecting",
        file: "audio_lifecycle.rs",
        function: "handle_deadlines_at",
        lines: (2888, 2908),
        guards: &[],
        real_test: None,
        what: "Starting, once: notify(StillConnecting)",
    },
    Mirror {
        step: "Timer::HardDeadline",
        needle: "HardDeadline",
        file: "audio_lifecycle.rs",
        function: "handle_deadlines_at",
        lines: (2909, 2929),
        guards: &[],
        real_test: None,
        what: "Starting past the hard deadline: begin_recovery(HardDeadline, Some(failure))",
    },
    Mirror {
        step: "Timer::Stalled",
        needle: "report_cleanup_stalled",
        file: "audio_lifecycle.rs",
        function: "handle_deadlines_at",
        lines: (2930, 2941),
        guards: &[],
        real_test: None,
        what: "Stopping|Recovering past the guidance deadline: report_cleanup_stalled",
    },
    Mirror {
        step: "SupStep::RecoveryStalled",
        needle: "AudioLifecycleEvent::RecoveryStalled",
        file: "audio_lifecycle.rs",
        function: "report_cleanup_stalled",
        lines: (2944, 2963),
        guards: &[],
        real_test: None,
        what: "set_phase(Recovering), notify(RecoveryStalled), once",
    },
    Mirror {
        step: "Msg::Cancel (environment, starting only)",
        needle: "starting_only: true",
        file: "audio_lifecycle.rs",
        function: "cancel_starting_for_environment_change",
        lines: (3322, 3330),
        guards: &[],
        real_test: None,
        what: "Cancel{owner: None, starting_only: true} with the response dropped",
    },
    Mirror {
        step: "supervisor loop (channel first, deadlines on an empty channel)",
        needle: "handle_deadlines_at",
        file: "audio_lifecycle.rs",
        function: "run_supervisor",
        lines: (721, 1174),
        guards: &[],
        real_test: None,
        what: "one message at a time; handle_deadlines_at on timeout, and before a message only for a tail",
    },
    Mirror {
        step: "worker events after its own end",
        needle: "AudioWorkerEvent::ThreadExited",
        file: "audio.rs",
        function: "spawn_capture_worker",
        lines: (3845, 3868),
        guards: &[],
        real_test: None,
        what: "after RuntimeFailed / InitFailed the worker returns and sends ThreadExited on its own",
    },
];

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
