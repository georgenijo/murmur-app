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

/// How a stop's reply reaches the stop command (C7.3 `StopEnd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// `RecordingSealed`: no `Idle`, a `CleanupFinished` after `OwnerClear`.
    Sealed,
    /// Delivered without a seal: no `Idle`, no `CleanupFinished` (PD-18 (c)).
    Unsealed,
    /// Undelivered: the supervisor sends `Idle`, the stop gets an error.
    Undelivered,
}

/// What the supervisor does with a start call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartKind {
    /// Refuses before accepting: no notification, no owner (`AlreadyStarting`,
    /// `AudioRecovering`, anything else).
    Reject(CaptureErr),
    /// A queued pending start answered `Cancelled`: `Idle` with no `Accepted`.
    PendingCancelled,
    /// `Accepted`, `Pre*`, `InitializationFailed(None)`, `Idle`: the start fails.
    Fail,
    /// A live attempt.
    Live,
}

/// How a live attempt ends when nothing else ends it first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// Waits for a stop or a cancel command (L4).
    External,
    /// `RecoverEnd`.
    Recover {
        reason: AudioCancelReason,
        init_failed: bool,
        /// `Some(n)`: `Interrupted` with `n` samples, else `Idle`.
        interrupted: Option<usize>,
    },
    /// `CrashEnd`.
    Crash,
}

/// What a capture cancel's client sees (`cancel_dictation_capture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelMode {
    /// The supervisor handles the message and answers after `begin_recovery`
    /// (its `Recovering{User}` notification) returned.
    Normal,
    /// The supervisor is unavailable: `Err` at once, nothing happens.
    Unavailable,
    /// The supervisor handles the message but the client's wait times out first:
    /// `Err` while the cancel still takes effect.
    Timeout,
}

/// The generated word of one attempt (C7.3), plus the results the supervisor
/// answers the commands with.
#[derive(Clone, Debug)]
pub struct AttemptPlan {
    pub kind: StartKind,
    pub connecting: u8,
    pub ready: bool,
    pub stalled: u8,
    pub end: End,
    pub stop_delivery: Delivery,
    /// Samples a delivered stop returns.
    pub stop_samples: usize,
    pub cancel: CancelMode,
}

/// One step of the supervisor thread: a notification, or an internal step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupStep {
    Accepted,
    StillConnecting,
    /// `set_phase(Recording)`, then `Ready` (one supervisor step).
    Ready,
    RecoveryStalled,
    /// `set_phase(Recovering)` then `Recovering{reason}`.
    Recovering(AudioCancelReason),
    InitFailed {
        recovery: Option<AudioCancelReason>,
        crash: bool,
    },
    /// The preserved capture goes to the store, then `Interrupted` is notified.
    Interrupted(usize),
    Idle,
    /// `OwnerClear`: `clear_owner` + `set_phase(Idle)` (`audio_lifecycle.rs:2458-2461`).
    ClearOwner,
    CleanupFinished,
    /// The supervisor sends the start call's response (`Ok(())` after `Accepted`,
    /// `audio_lifecycle.rs:1589-1596`): a step of its own, not a notification.
    Answer,
    /// The supervisor sends a cancel's response, after `begin_recovery` (and so its
    /// `Recovering{User}` notification) returned (`audio_lifecycle.rs:1336`).
    CancelAnswer,
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

/// An end the worker may begin on its own (a recovery, a crash, a failed open):
/// not decided until it happens, so a stop or a cancel command can win first.
#[derive(Clone, Debug)]
pub struct PendingEnd {
    /// Steps of the `begin_recovery` handler (and the deadline handler).
    pub begin: VecDeque<SupStep>,
    /// Steps of the `finish_attempt` group, run when the worker thread exits.
    pub finish: VecDeque<SupStep>,
    pub ended_by: EndedBy,
}

#[derive(Clone, Debug)]
pub struct Attempt {
    pub rid: Rid,
    pub plan: AttemptPlan,
    /// Steps before an end: the notifications of a starting and recording attempt.
    pub prefix: VecDeque<SupStep>,
    /// The worker's own end, before it begins.
    pub pending_end: Option<PendingEnd>,
    /// The steps of a begun end before the worker exits: `begin_recovery`'s
    /// notifications, a cancel's answer, the one teardown stall.
    pub begin: VecDeque<SupStep>,
    /// `finish_attempt`: runs as one uninterrupted group on the supervisor thread
    /// (it handles no command until `OwnerClear` and what follows ran).
    pub finish: VecDeque<SupStep>,
    /// An end has begun (by the word itself, a stop or a cancel): a later stop or
    /// cancel can no longer take it.
    pub end_decided: bool,
    /// The `finish_attempt` group started and has steps left.
    pub finishing: bool,
    /// Whether `plan.end` is still waiting for an external stop or cancel.
    pub awaiting_external: bool,
    pub stop_called: bool,
    pub cancelled: bool,
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
    /// The step whose bridge arm is running on the supervisor thread: it handles
    /// no command and takes no step until the arm returns.
    pub busy: Option<SupStep>,
    /// The running arm already ran the D section that can restore `Idle`.
    pub busy_restored: bool,
    /// Attempts that have ended and whose steps are all done, newest last.
    pub finished: Vec<(Rid, EndedBy)>,
    /// The start calls' responses, once sent.
    pub start_answers: BTreeMap<Rid, super::supervisor::StartAnswer>,
    /// The capture cancels' responses (`Normal` mode), once sent.
    pub cancel_answers: BTreeMap<Rid, Result<bool, ()>>,
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

    /// The step the supervisor thread would run next, if it can run one.
    pub fn next_step(&self) -> Option<SupStep> {
        if self.busy.is_some() {
            return None;
        }
        let attempt = self.attempt.as_ref()?;
        attempt
            .prefix
            .front()
            .or_else(|| attempt.begin.front())
            .or_else(|| attempt.finish.front())
            .copied()
    }

    /// Steps still to run for the attempt (any, including an end the worker has
    /// not begun yet).
    pub fn pending_steps(&self) -> impl Iterator<Item = SupStep> + '_ {
        self.attempt.iter().flat_map(|a| {
            a.prefix
                .iter()
                .chain(a.begin.iter())
                .chain(a.finish.iter())
                .chain(
                    a.pending_end
                        .iter()
                        .flat_map(|p| p.begin.iter().chain(p.finish.iter())),
                )
                .copied()
        })
    }

    /// The response of the start call for `rid`, once the supervisor sent it.
    pub fn start_answer(&self, rid: Rid) -> Option<super::supervisor::StartAnswer> {
        self.start_answers.get(&rid).cloned()
    }

    /// The attempt can begin its own end: the notifications of its start ran and no
    /// command decided another end.
    pub fn can_begin_end(&self) -> bool {
        self.busy.is_none()
            && self.attempt.as_ref().is_some_and(|a| {
                a.pending_end.is_some()
                    && !a.end_decided
                    && a.prefix.is_empty()
                    && a.begin.is_empty()
                    && a.finish.is_empty()
            })
    }
}

// ------------------------------------------------------------------ world

/// A capture the supervisor preserved for the user (`take_interrupted_dictation`).
#[derive(Clone, Copy, Debug)]
pub struct Preserved {
    pub samples: usize,
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
}

/// A `claim_terminal` call and whether it won.
#[derive(Clone, Copy, Debug)]
pub struct ClaimTry {
    pub row: &'static str,
    pub won: bool,
    pub step: u32,
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
