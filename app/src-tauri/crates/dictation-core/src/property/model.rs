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
        stalled: u8,
        /// `Some(n)`: `Interrupted` with `n` samples, else `Idle`.
        interrupted: Option<usize>,
    },
    /// `CrashEnd`.
    Crash,
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
    pub cancel_err: bool,
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

#[derive(Clone, Debug)]
pub struct Attempt {
    pub rid: Rid,
    pub plan: AttemptPlan,
    /// Steps before the end is decided.
    pub prefix: VecDeque<SupStep>,
    /// The end: spontaneous (`Recover`, `Crash`), or set by a stop or a cancel.
    pub end: VecDeque<SupStep>,
    /// The end is decided (by the word itself, a stop or a cancel): a later stop or
    /// cancel can no longer take it.
    pub end_decided: bool,
    /// Whether `plan.end` is still waiting for an external stop or cancel.
    pub awaiting_external: bool,
    pub stop_called: bool,
    pub cancelled: bool,
    /// How the end was decided (for the witnesses).
    pub ended_by: EndedBy,
    /// Notifications delivered so far (a gate for the start's acknowledgement).
    pub accepted_delivered: bool,
    pub fail_delivered: bool,
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
            .or_else(|| attempt.end.front())
            .copied()
    }

    /// Steps still to run for the attempt (any, including a gated end).
    pub fn pending_steps(&self) -> impl Iterator<Item = SupStep> + '_ {
        self.attempt
            .iter()
            .flat_map(|a| a.prefix.iter().chain(a.end.iter()).copied())
    }

    /// Whether the start call's acknowledgement has been sent: `Accepted` was
    /// delivered (a live attempt), or the failure notifications were (a failed
    /// start answers after `InitializationFailed` and `Idle`, T3).
    pub fn ack_ready(&self, rid: Rid) -> bool {
        match &self.attempt {
            Some(a) if a.rid == rid => match a.plan.kind {
                StartKind::Live => a.accepted_delivered,
                StartKind::Fail => a.fail_delivered,
                StartKind::PendingCancelled => a.fail_delivered,
                StartKind::Reject(_) => true,
            },
            // The attempt is gone (its steps all ran): the answer is out.
            _ => true,
        }
    }
}

// ------------------------------------------------------------------ world

/// A capture the supervisor preserved for the user (`take_interrupted_dictation`).
#[derive(Clone, Copy, Debug)]
pub struct Preserved {
    pub samples: usize,
}

#[derive(Debug, Default)]
pub struct Ledger {
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
    /// `ticker_started`'s `fetch_max` (SP10: never lowered).
    pub ticker_started: Rid,
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
    pub fn claim_terminal(&mut self, rid: Rid, terminal: Terminal, row: &'static str) -> bool {
        if !self.ledger.accepted.contains(&rid) || self.ledger.claimed.contains_key(&rid) {
            return false;
        }
        self.ledger
            .claimed
            .insert(rid, (terminal.outcome, terminal.code, row));
        *self.ledger.wins.entry(rid).or_default() += 1;
        true
    }

    /// `dictation_diagnostics.complete`: the first completion is kept.
    pub fn diagnostics_complete(&mut self, rid: Rid, outcome: TerminalOutcome) {
        self.ledger.diagnostics.entry(rid).or_insert(outcome);
    }

    pub fn perf_complete(&mut self, rid: Rid, outcome: PerfOutcome, row: &'static str) {
        self.perf.outcome.entry(rid).or_insert((outcome, row));
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
