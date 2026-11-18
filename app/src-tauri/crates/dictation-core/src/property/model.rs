//! The model world: the shell's state as the C3 tables see it (M1-M4, the
//! telemetry ledger, the performance and diagnostics sinks, `core_history`),
//! and the supervisor of the C7.3 `Attempt` grammar.
//!
//! The harness does not model the shell's code. It models the *state* the
//! decide functions read and the effects they return; every transition is
//! made by a real `decide_*` function and a placed effect interpreted here.

use crate::decision::{ContKind, Continuation, ContinuationSpec, OpStatus};
use crate::effects::{PerfOutcome, Terminal};
use crate::ids::{OperationId, RecordingId};
use crate::inputs::CaptureErr;
use crate::state::{DictationCore, LiveObs, LockFreeFacts};
use crate::status::{AudioCancelReason, ErrCode, Status, TerminalOutcome};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type Rid = u64;
pub type TaskId = u32;

// ------------------------------------------------------------ supervisor

/// The public phase of the capture owner (`audio_lifecycle.rs` `set_phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Starting,
    Recording,
    Stopping,
    Recovering,
}

/// How a stop's reply reaches the stop command (C7.3 `StopEnd`). Whether it reaches
/// it at all is the *client's* receiver timeout (`STOP_RESPONSE_TIMEOUT`), never part
/// of the plan: a reply is delivered at the seal or at the worker's exit
/// (`audio_lifecycle.rs:2279-2302`, `2358-2373`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// `RecordingSealed` hands the samples over: no `Idle`, a `CleanupFinished` after
    /// `OwnerClear`.
    Sealed,
    /// No seal: `finish_attempt` delivers at the worker's exit: no `Idle`, no
    /// `CleanupFinished` (PD-18 (c)).
    Unsealed,
}

/// What the supervisor does with a start call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartKind {
    /// Refuses before accepting: no notification, no owner (`Cancelled`, a failed
    /// spawn: `audio_lifecycle.rs:1402-1413`, `1487-1498`).
    Reject(CaptureErr),
    /// A queued pending start answered `Cancelled`: `Idle` with no `Accepted`
    /// (`audio_lifecycle.rs:1148-1160`, the standby handoff).
    PendingCancelled,
    /// `Accepted`, then the worker's `InitFailed` and exit: `InitializationFailed`,
    /// `Idle` (`audio_lifecycle.rs:2374-2389`).
    Fail,
    /// A live attempt.
    Live,
}

/// How a live attempt ends when nothing else ends it first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// Waits for a stop or a cancel command (L4).
    External,
    /// `RecoverEnd`: `RuntimeFailed` (or `TerminationUnconfirmed` when `init_failed`)
    /// for `RuntimeFailure`, a starting-only cancel for a device or power change, the
    /// hard deadline for `HardDeadline` (only while the attempt is `Starting`).
    Recover {
        reason: AudioCancelReason,
        /// The recovery reports a failure (`InitializationFailed`): the worker's
        /// `TerminationUnconfirmed`, or the hard deadline's.
        init_failed: bool,
        /// `Some(n)`: `Interrupted` with `n` samples, else `Idle`.
        interrupted: Option<usize>,
    },
    /// `CrashEnd`: the worker thread exits on its own.
    Crash,
}

/// The generated word of one attempt (C7.3), plus what the worker's stop delivers.
#[derive(Clone, Debug)]
pub struct AttemptPlan {
    pub kind: StartKind,
    /// The `StillConnecting` deadline fires while the attempt is `Starting`.
    pub connecting: bool,
    pub ready: bool,
    /// The teardown-stall guidance deadline fires once while stopping or recovering.
    pub stalled: bool,
    pub end: End,
    pub stop_delivery: Delivery,
    /// Samples a delivered stop returns.
    pub stop_samples: usize,
    /// The typed failure the attempt's own end reports (the `AudioFailureKind` mapped
    /// to its `ErrCode`, D-S2b-11 (b)): the worker's `InitFailed`, a runtime failure's
    /// kind (`TerminationUnconfirmed` when `init_failed`), the hard deadline's timeout,
    /// `BackendError` for a crash (`finish_attempt`'s default).
    pub fail: ErrCode,
}

/// One step of the supervisor thread: a notification, or an internal step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupStep {
    Accepted,
    StillConnecting,
    /// `Ready` (the phase became `Recording` when the first buffer was accepted).
    Ready,
    RecoveryStalled,
    /// `Recovering{reason}` (the phase became `Recovering` before it).
    Recovering(AudioCancelReason),
    InitFailed {
        recovery: Option<AudioCancelReason>,
        /// The typed failure (`from_audio_failure(kind)`), exactly what R213.2 must
        /// report as the terminal code.
        code: ErrCode,
    },
    /// The preserved capture goes to the store, then `Interrupted` is notified.
    Interrupted(usize),
    Idle,
    /// `OwnerClear`: `clear_owner` + `set_phase(Idle)` (`audio_lifecycle.rs:2458-2461`).
    ClearOwner,
    CleanupFinished,
    /// The supervisor sends the start call's response (`Ok(())` after `Accepted`,
    /// `audio_lifecycle.rs:1589-1596`, or a refusal, `1402-1433`): a step of its
    /// own, not a notification, and apart from the acceptance that decided it.
    StartAnswer(TaskId),
    /// An immediate response of a stop or a cancel (`Ok(Vec::new())`, an error,
    /// `Ok(false)`: `audio_lifecycle.rs:1235-1242`, `1291-1298`, `1301-1330`), sent
    /// by a step of its own after the message was accepted (D-S2b-11 (f)).
    Reply(TaskId, ReplyVal),
    /// A cancel's response `Ok(true)`, after `begin_recovery` (and so its
    /// `Recovering{User}` notification) returned (`audio_lifecycle.rs:1332-1336`).
    CancelAnswer(TaskId),
    /// A stop in `Starting`: `begin_recovery`, then `Ok(Vec::new())`
    /// (`audio_lifecycle.rs:1243-1246`).
    StopAnswer(TaskId),
}

impl SupStep {
    /// Whether the step restores (or can restore) status `Idle` for its attempt:
    /// the "`Idle`/`CleanupFinished`-capable notifications" of C7.1.
    pub fn restores_idle(&self) -> bool {
        match self {
            Self::Idle | Self::CleanupFinished => true,
            // S12 (R216) writes `Idle` for a short capture; a long one hands the
            // restore to the spawned stop (R215.2), which is not a notification.
            Self::Interrupted(samples) => *samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES,
            _ => false,
        }
    }
}

/// What the worker thread tells the supervisor (`AudioWorkerEvent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerEv {
    FirstBuffer,
    RuntimeFailed,
    TerminationUnconfirmed,
    InitFailed,
    RecordingSealed,
    ThreadExited,
}

/// A message in the supervisor's channel (`SupervisorMessage`). The channel is FIFO:
/// a worker event and a client command are taken in the order they were sent.
#[derive(Clone, Debug)]
pub enum Msg {
    Start {
        rid: Rid,
        /// The client task waiting on the start call's response.
        tid: TaskId,
        plan: Box<AttemptPlan>,
    },
    Stop {
        rid: Rid,
        tid: TaskId,
    },
    /// `tid: None`: the response receiver is gone (an environment cancel).
    Cancel {
        rid: Rid,
        tid: Option<TaskId>,
        reason: AudioCancelReason,
        starting_only: bool,
    },
    Worker {
        rid: Rid,
        ev: WorkerEv,
    },
}

/// What a command's reply carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyVal {
    /// The start call's response was sent (its value is in `Sup::start_answers`).
    Start,
    Stop(Result<usize, ()>),
    Cancel(Result<bool, ()>),
}

/// The state of a command's response channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyState {
    /// The receiver waits: `ticks` polls so far, the timeout fires at `limit`.
    Waiting { ticks: u16, limit: u16 },
    /// `send` succeeded; the client has not continued yet.
    Delivered(ReplyVal),
    /// `recv_timeout` returned: the receiver is dropped and a later `send` fails.
    TimedOut,
}

/// What the worker thread has done (the events it already sent) and was told to do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Worker {
    /// `send_worker_stop` was called (`audio_lifecycle.rs:638`).
    pub stop_sent: bool,
    pub first_buffer: bool,
    pub own_end: bool,
    pub sealed: bool,
    pub exited: bool,
}

/// The supervisor's private attempt (`Attempt` in `audio_lifecycle.rs:590`).
#[derive(Clone, Debug)]
pub struct Attempt {
    pub rid: Rid,
    pub plan: AttemptPlan,
    /// `AttemptPhase`: `Starting`, `Recording`, `Stopping` or `Recovering` (the
    /// public phase differs after a teardown stall).
    pub phase: Phase,
    pub recovery_reason: Option<AudioCancelReason>,
    /// `stop_response`: the client waiting for the stop's reply.
    pub stop_reply: Option<TaskId>,
    pub sealed_delivery: bool,
    /// `tail_deadline.is_some()`.
    pub tail_pending: bool,
    /// `stopping_guidance_emitted`.
    pub stall_emitted: bool,
    /// `still_connecting_emitted`.
    pub connecting_emitted: bool,
    /// `failure_reported`.
    pub failure_reported: bool,
    pub worker: Worker,
    /// How the end was decided (for the witnesses).
    pub ended_by: EndedBy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndedBy {
    Open,
    Stop(Delivery),
    Cancel,
    Recover { interrupted: Option<usize> },
    Crash,
    Fail,
}

#[derive(Debug, Default)]
pub struct Sup {
    pub owner: Option<Rid>,
    pub phase: Option<Phase>,
    pub attempt: Option<Attempt>,
    /// The channel (`SupervisorMessage`): FIFO.
    pub queue: VecDeque<Msg>,
    /// The steps of the message the supervisor thread is handling: it takes no
    /// other message and no deadline until they all ran.
    pub handling: VecDeque<SupStep>,
    /// The recording the handled message (or deadline) is about.
    pub handling_rid: Rid,
    /// The step whose bridge arm is running on the supervisor thread: it handles
    /// no command and takes no step until the arm returns.
    pub busy: Option<SupStep>,
    /// The running arm already ran the D section that can restore `Idle`.
    pub busy_restored: bool,
    /// Attempts that have ended and whose steps are all done, newest last.
    pub finished: Vec<(Rid, EndedBy)>,
    /// The typed failure of each attempt that ended (`failure.kind` mapped to its code),
    /// read when its preserved capture is stored.
    pub finished_fail: BTreeMap<Rid, ErrCode>,
    /// The start calls' responses, once sent.
    pub start_answers: BTreeMap<Rid, super::supervisor::StartAnswer>,
    /// The response the supervisor decided for a start at its acceptance and has not
    /// sent yet (its `StartAnswer` step), by client task.
    pub start_pending: BTreeMap<TaskId, super::supervisor::StartAnswer>,
    /// The start callers whose `caller_alive` flag is false (their receiver timed
    /// out, `audio_lifecycle.rs:3070`).
    pub caller_dead: BTreeSet<TaskId>,
    /// The recordings whose start call's receiver timed out (`await_start_response`).
    pub timed_out_starts: BTreeSet<Rid>,
    /// The response channels of the commands in flight, by client task.
    pub replies: BTreeMap<TaskId, ReplyState>,
}

impl Sup {
    pub fn phase(&self) -> Phase {
        self.phase.unwrap_or(Phase::Idle)
    }

    /// `is_dictation_cleanup_pending(rid)`: derived from the model, never generated:
    /// true from the phase change to `Stopping`/`Recovering` until `OwnerClear`.
    pub fn cleanup_pending(&self, rid: Rid) -> bool {
        self.owner == Some(rid) && matches!(self.phase(), Phase::Stopping | Phase::Recovering)
    }

    /// `is_dictation_recording(rid)`.
    pub fn audio_ready(&self, rid: Rid) -> bool {
        self.owner == Some(rid) && self.phase() == Phase::Recording
    }

    /// The response of the start call for `rid`, once the supervisor sent it.
    pub fn start_answer(&self, rid: Rid) -> Option<super::supervisor::StartAnswer> {
        self.start_answers.get(&rid).cloned()
    }

    /// A notification that must restore `Idle` is still to come for the owner (a
    /// step of the handler, the running arm, or what the attempt's own end makes
    /// `finish_attempt` send whatever else happens). Whether the stop's reply is
    /// delivered can still change (a receiver timeout), so a waiting unsealed reply
    /// counts as none.
    pub fn restorer_coming(&self) -> bool {
        if self.handling.iter().any(|step| step.restores_idle())
            || (self.busy.as_ref().is_some_and(|step| step.restores_idle()) && !self.busy_restored)
        {
            return true;
        }
        let Some(attempt) = self.attempt.as_ref() else {
            return false;
        };
        match attempt.phase {
            Phase::Recovering => match (attempt.recovery_reason, attempt.plan.end) {
                (
                    Some(AudioCancelReason::RuntimeFailure),
                    End::Recover {
                        interrupted: Some(samples),
                        ..
                    },
                ) => samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES,
                _ => true,
            },
            Phase::Stopping => {
                attempt.sealed_delivery
                    || attempt.stop_reply.is_none()
                    || attempt.plan.stop_delivery == Delivery::Sealed
            }
            Phase::Starting | Phase::Recording => {
                attempt.plan.kind == StartKind::Fail
                    || match attempt.plan.end {
                        End::External => false,
                        End::Crash => true,
                        End::Recover {
                            reason,
                            interrupted,
                            ..
                        } => match (reason, interrupted) {
                            (AudioCancelReason::RuntimeFailure, Some(samples)) => {
                                samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES
                            }
                            _ => true,
                        },
                    }
            }
            Phase::Idle => false,
        }
    }
}

// ------------------------------------------------------------------ world

/// A capture the supervisor preserved for the user (`take_interrupted_dictation`).
#[derive(Clone, Copy, Debug)]
pub struct Preserved {
    pub samples: usize,
    /// The code of the attempt's failure (`failure.kind.as_str()` ->
    /// `from_interruption_reason`, `audio_lifecycle.rs:2405-2432`): what the stop's
    /// `RuntimeInterruption` terminal carries.
    pub code: ErrCode,
}

/// What a first-wins ledger entry (the terminal claim, the performance completion)
/// saw when it was written: the context a signature's causal precondition reads.
#[derive(Clone, Copy, Debug)]
pub struct Won {
    pub row: &'static str,
    pub step: u32,
    pub input_rid: Rid,
    pub current: Rid,
    pub status_before: Status,
    /// Another task's cancel tail (its `PostDT`/`PostT` effects after the D release)
    /// for this recording was open.
    pub other_cancel_tail: bool,
    /// This task is itself the cancel whose tail was open.
    pub own_cancel_tail: bool,
    /// The cancel task whose tail (D released, the clears and emits still to run) was
    /// open for this recording, whoever wrote the entry: the window's identity.
    pub tail: Option<TaskId>,
}

/// A `claim_terminal` call and whether it won.
#[derive(Clone, Copy, Debug)]
pub struct ClaimTry {
    pub row: &'static str,
    pub won: bool,
    pub step: u32,
    /// The cancel tail open when the call ran (see [`Won::tail`]).
    pub tail: Option<TaskId>,
}

#[derive(Debug, Default)]
pub struct Ledger {
    /// The winner's context, by id.
    pub ctx: BTreeMap<Rid, Won>,
    /// Every claim call, by id.
    pub tries: BTreeMap<Rid, Vec<ClaimTry>>,
    pub accepted: BTreeSet<Rid>,
    /// The winner of each id: outcome, code and the row that claimed it.
    pub claimed: BTreeMap<Rid, (TerminalOutcome, ErrCode, &'static str)>,
    /// How many `claim_terminal` calls returned `true`, per id.
    pub wins: BTreeMap<Rid, u32>,
    /// `dictation_diagnostics.complete`: the first outcome taken, per id.
    pub diagnostics: BTreeMap<Rid, TerminalOutcome>,
}

#[derive(Debug, Default)]
pub struct Perf {
    pub registered: BTreeSet<Rid>,
    pub begun: BTreeSet<Rid>,
    pub aborted: BTreeSet<Rid>,
    /// First completion wins (C4.6).
    pub outcome: BTreeMap<Rid, (PerfOutcome, &'static str)>,
    /// The winner's context, by id.
    pub ctx: BTreeMap<Rid, Won>,
}

#[derive(Debug, Default)]
pub struct History {
    /// H1 `pending_ops`.
    pub ops: BTreeMap<u64, (Rid, ContKind)>,
    /// H2 `pipelines`.
    pub pipelines: BTreeSet<Rid>,
    pub next_op: u64,
    pub ops_high_water: usize,
}

#[derive(Debug)]
pub struct World {
    // M1-M4
    pub status: Status,
    pub current_id: Rid,
    pub cancelled_through: Rid,
    pub context_for: Option<Rid>,
    /// `keyboard::set_processing` (global, no id: PD-17).
    pub keyboard: bool,
    pub ledger: Ledger,
    pub perf: Perf,
    pub history: History,
    pub sup: Sup,
    /// Registrations that broke the continuation-id bound (SP6), drained per step.
    pub registry_faults: Vec<String>,
    /// Captures preserved for the user, by recording.
    pub preserved: BTreeMap<Rid, Preserved>,
    /// Which row took a preserved capture, by recording (PD-15).
    pub taken_by: BTreeMap<Rid, &'static str>,
    /// Partial tickers spawned, by recording (`SD`; PD-4's duplicate).
    pub tickers: BTreeMap<Rid, u32>,
    /// `ticker_started`'s `fetch_max`.
    pub ticker_started: Rid,
    /// The partial-decode CAS lease (`dictation_partial_in_flight_id`): taken by a
    /// ticker's `try_begin`, released only by that ticker's lease `Drop` (SP10).
    pub partial_lease: Option<(Rid, TaskId)>,
    /// Recordings whose result was delivered (pasted) by the pipeline fence.
    pub delivered: BTreeSet<Rid>,
    /// `idle` emits per recording, for PD-16.
    pub idle_emits: BTreeMap<Rid, u32>,
    /// T: the task holding the recording transition.
    pub t_holder: Option<TaskId>,
    /// Tasks currently inside a PD-16 window: a cancel's `PostDT` tail after its
    /// `In` section, by recording.
    pub cancel_tail: BTreeMap<Rid, TaskId>,
    /// The step counter (the model's clock).
    pub step: u32,
}

impl World {
    pub fn new() -> Self {
        Self {
            status: Status::Idle,
            current_id: 0,
            cancelled_through: 0,
            context_for: None,
            keyboard: false,
            ledger: Ledger::default(),
            perf: Perf::default(),
            history: History {
                next_op: 1,
                ..History::default()
            },
            sup: Sup::default(),
            registry_faults: Vec::new(),
            preserved: BTreeMap::new(),
            taken_by: BTreeMap::new(),
            tickers: BTreeMap::new(),
            ticker_started: 0,
            partial_lease: None,
            delivered: BTreeSet::new(),
            idle_emits: BTreeMap::new(),
            t_holder: None,
            cancel_tail: BTreeMap::new(),
            step: 0,
        }
    }

    pub fn view(&self) -> DictationCore {
        DictationCore::project(
            self.status,
            RecordingId(self.current_id),
            LiveObs {
                cancelled_through: RecordingId(self.cancelled_through),
                context_for: self.context_for.map(RecordingId),
            },
        )
    }

    /// What a D-less entry reads (`AppState.recording_id`, `cancelled_id`).
    pub fn facts(&self) -> LockFreeFacts {
        LockFreeFacts {
            current_id: RecordingId(self.current_id),
            cancelled_through: RecordingId(self.cancelled_through),
        }
    }

    // -------- core_history (H1, H2): the shell's bookkeeping

    /// `ContinuationGuard::register`: a decision's continuation spec becomes a
    /// token right after the decide, before any effect.
    pub fn register(&mut self, spec: ContinuationSpec) -> Continuation {
        // SP6 at every registration (D-S2b-9 (d)): a continuation names a recording that
        // exists. `K:CaptureStart` registers before `ID+` allocates its own id, so it is
        // the one kind that may name `current + 1`.
        let limit = self.current_id + u64::from(spec.kind == ContKind::CaptureStart);
        if spec.recording_id.0 > limit {
            self.registry_faults.push(format!(
                "SP6: {:?} continuation names recording {}, above the bound {limit}",
                spec.kind, spec.recording_id.0
            ));
        }
        let op = self.history.next_op;
        self.history.next_op += 1;
        self.history
            .ops
            .insert(op, (spec.recording_id.0, spec.kind));
        self.history.ops_high_water = self.history.ops_high_water.max(self.history.ops.len());
        Continuation {
            recording_id: spec.recording_id,
            op: OperationId(op),
            kind: spec.kind,
        }
    }

    /// The validated status of a token (an observation), without consuming it.
    pub fn op_status(&self, token: Continuation) -> OpStatus {
        match self.history.ops.get(&token.op.0) {
            Some((rid, kind)) if *rid == token.recording_id.0 && *kind == token.kind => {
                OpStatus::Outstanding
            }
            _ => OpStatus::Unknown,
        }
    }

    /// `ret_if_matches(token)`: retires the entry only if op id, recording id and
    /// kind all match. Returns whether an entry was retired.
    pub fn retire(&mut self, token: Continuation) -> bool {
        if self.op_status(token) == OpStatus::Outstanding {
            self.history.ops.remove(&token.op.0);
            true
        } else {
            false
        }
    }

    /// An owner's `Drop`: retire by op id alone.
    pub fn retire_op(&mut self, token: Continuation) {
        self.history.ops.remove(&token.op.0);
    }

    // -------- the telemetry ledger and the diagnostics sink (TERM)

    /// `claim_terminal`: first wins, and only for an id `accepted()` already ran
    /// for (PD-10: a terminal before `accepted` is dropped).
    pub fn claim_terminal(
        &mut self,
        rid: Rid,
        terminal: Terminal,
        row: &'static str,
        ctx: Won,
    ) -> bool {
        let won = self.ledger.accepted.contains(&rid) && !self.ledger.claimed.contains_key(&rid);
        self.ledger.tries.entry(rid).or_default().push(ClaimTry {
            row,
            won,
            step: ctx.step,
            tail: ctx.tail,
        });
        if !won {
            return false;
        }
        self.ledger
            .claimed
            .insert(rid, (terminal.outcome, terminal.code, row));
        self.ledger.ctx.insert(rid, ctx);
        *self.ledger.wins.entry(rid).or_default() += 1;
        true
    }

    /// `dictation_diagnostics.complete`: the first completion is kept.
    pub fn diagnostics_complete(&mut self, rid: Rid, outcome: TerminalOutcome) {
        self.ledger.diagnostics.entry(rid).or_insert(outcome);
    }

    pub fn perf_complete(&mut self, rid: Rid, outcome: PerfOutcome, row: &'static str, ctx: Won) {
        if let std::collections::btree_map::Entry::Vacant(slot) = self.perf.outcome.entry(rid) {
            slot.insert((outcome, row));
            self.perf.ctx.insert(rid, ctx);
        }
    }

    /// The context a ledger entry written now by `fx_row` for `rid` by task `tid` records.
    pub fn won(&self, rid: Rid, tid: TaskId, row: &'static str) -> Won {
        Won {
            row,
            step: self.step,
            input_rid: rid,
            current: self.current_id,
            status_before: self.status,
            other_cancel_tail: self.cancel_tail.get(&rid).is_some_and(|t| *t != tid),
            own_cancel_tail: self.cancel_tail.get(&rid) == Some(&tid),
            tail: self.cancel_tail.get(&rid).copied(),
        }
    }

    /// `take_interrupted_dictation(rid)`: destructive.
    pub fn take_interrupted(&mut self, rid: Rid, row: &'static str) -> Option<Preserved> {
        let taken = self.preserved.remove(&rid);
        if taken.is_some() {
            self.taken_by.insert(rid, row);
        }
        taken
    }

    /// The model's `q_ticker_may_register`: status `Recording`, id current and not
    /// cancelled, then `note_ticker_started` (`fetch_max(id) <= id`).
    pub fn register_ticker(&mut self, rid: Rid) -> bool {
        let allowed = rid > 0
            && rid == self.current_id
            && self.status == Status::Recording
            && self.cancelled_through < rid;
        if !allowed {
            return false;
        }
        let may = self.ticker_started <= rid;
        self.ticker_started = self.ticker_started.max(rid);
        if may {
            *self.tickers.entry(rid).or_default() += 1;
        }
        may
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// The class of a performance outcome compared by C7.4.
pub fn perf_class_allowed(terminal: TerminalOutcome, perf: PerfOutcome) -> bool {
    use PerfOutcome::*;
    use TerminalOutcome as T;
    match terminal {
        T::Success => perf == Completed,
        T::NoSpeech | T::TooShort => perf == NoSpeech,
        T::UserCancelledStarting | T::UserCancelledRecording | T::UserCancelledProcessing => {
            perf == Cancelled
        }
        T::CaptureInitFailure | T::StopFailure | T::PipelineFailure => perf == Failed,
        // Failed (S12), or the interrupted partial's own outcome (C7.4).
        T::RuntimeInterruption => true,
        T::Superseded => false,
    }
}
