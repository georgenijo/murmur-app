//! Stage 2a-1: the supervisor-lifecycle decide functions (appendix C2.3 sites
//! 207-216, rows of C3.6).
//!
//! One pure function per D critical section of the lifecycle bridge
//! (`handle_audio_lifecycle_with`). Each takes the decision view (`&DictationCore`,
//! or [`LockFreeFacts`] for the `*_nolock` entries that hold no D today) and a
//! typed input, **mutates nothing**, and returns a [`Decision`] with the ordered,
//! placed effects of its C3.6 row. The shell calls each one inline at today's line,
//! with D held exactly as today, and runs the effects through the executor at
//! their placement; M1 commits at the `ST` effect, never here (C1.4.2).
//!
//! None of these sites reads M3 (`cancelled_through`) or M4 (`context_for`):
//! `Ready` asks for the context through `NeedObservation(ContextPresence)`
//! instead (decision D-S1-2), and no row carries a continuation token, so the
//! op-status parity rule is vacuous here. `R215.2` returns the `AutoStop`
//! continuation spec.
//!
//! Row ids are the C3.6 ids. A row that reproduces a preserved defect keeps it
//! (PD-1, PD-4, PD-5, PD-7, ...); the row's doc names it.

use crate::decision::{ContKind, ContinuationSpec, Decision, ObsKind, RowId, StaleKind, Verdict};
use crate::effects::{
    Cond, Effect, EventKind, PerfCause, PerfComplete, PerfOutcome, PerfStage, Placed, Placement,
    PresentationKind, Terminal,
};
use crate::ids::{Handle, Now, RecordingId};
use crate::state::{DictationCore, LockFreeFacts};
use crate::status::{AudioCancelReason, ErrCode, Status, StopTrigger, TerminalOutcome};

/// `delivered_samples >= 8_000` auto-transcribes an interrupted capture (R215.2).
pub const AUTO_TRANSCRIBE_MIN_SAMPLES: usize = 8_000;

/// The row `decide_sup_interrupted_nolock` returns for a short capture: the shell
/// then enters site 216 (a separate D section, no id or status guard).
pub const R215_3: RowId = RowId("R215.3");

/// Input of [`decide_sup_ready`] (site 207).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupReadyInput {
    /// Recording id.
    pub rid: RecordingId,
    /// `active_context(rid).is_some()` (`[in]`, 4947). `None` until the shell
    /// answers `NeedObservation(ContextPresence)`.
    pub context_present: Option<bool>,
    /// Stamp taken at 4927 (shadow normalization only; no decision reads it).
    pub ready_received: Now,
}

/// Input of [`decide_sup_recovering`] (site 209).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupRecoveringInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Why the supervisor began recovery.
    pub reason: AudioCancelReason,
}

/// Input of [`decide_sup_init_failed_nolock`] (site 213).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupInitFailedInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Opaque failure kind: bound by the shell for `PCF` and the event payload.
    pub kind: Handle,
    /// `from_audio_failure(kind)`, computed by the shell before the decision
    /// (decision D-S1-4), so R213.2 is pure.
    pub error_code: ErrCode,
    /// Recovery reason, if the failure was a recovery.
    pub recovery_reason: Option<AudioCancelReason>,
}

/// Input of [`decide_sup_interrupted_nolock`] (site 215).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupInterruptedInput {
    /// Recording id.
    pub rid: RecordingId,
    /// Delivered samples (`>= 8000` auto-transcribes).
    pub delivered_samples: usize,
    /// Audio duration (a shell-bound payload of `PC` and the event).
    pub duration_ms: u64,
    /// Interruption reason (an event payload).
    pub reason: AudioCancelReason,
}

// ------------------------------------------------------------------ builders

pub(crate) fn placed(at: Placement, effect: Effect) -> Placed {
    Placed::new(at, effect)
}

pub(crate) fn terminal(at: Placement, outcome: TerminalOutcome, code: ErrCode) -> Placed {
    placed(at, Effect::Terminal(Terminal::new(outcome, code)))
}

pub(crate) fn superseded(at: Placement) -> Placed {
    terminal(at, TerminalOutcome::Superseded, ErrCode::StaleOwner)
}

pub(crate) fn set_status(to: Status) -> Placed {
    placed(Placement::In, Effect::SetStatus(to))
}

pub(crate) fn emit_status(at: Placement, status: Status) -> Placed {
    placed(
        at,
        Effect::EmitStatus {
            status,
            cond: Cond::Always,
        },
    )
}

pub(crate) fn emit_event(at: Placement, kind: EventKind) -> Placed {
    placed(at, Effect::EmitEvent(kind))
}

pub(crate) fn clear_context(at: Placement, rid: RecordingId) -> Placed {
    placed(at, Effect::ClearContext(rid))
}

fn perf_failed(at: Placement, with_audio_duration: bool) -> Placed {
    placed(
        at,
        Effect::PerfComplete(PerfComplete {
            outcome: PerfOutcome::Failed,
            stage: PerfStage::CaptureFinalization,
            cause: Some(PerfCause::AudioCaptureFailed),
            with_audio_duration,
            only_if_run: false,
        }),
    )
}

fn take_interrupted_dispose(at: Placement, id: RecordingId) -> Placed {
    placed(
        at,
        Effect::TakeInterrupted {
            id,
            mode: crate::effects::TakeMode::Dispose,
        },
    )
}

/// An ordered effect list that can pair a `PRES` with the emit just pushed.
#[derive(Default)]
pub(crate) struct Fx(pub(crate) Vec<Placed>);

impl Fx {
    pub(crate) fn push(&mut self, effect: Placed) -> &mut Self {
        self.0.push(effect);
        self
    }

    /// `PRES(kind)`, conditional on the emit pushed last (`P-emit-ok(i)`).
    fn presentation_of_last_emit(&mut self, at: Placement, kind: PresentationKind) -> &mut Self {
        let paired = u8::try_from(self.0.len() - 1).expect("a lifecycle row has few effects");
        self.0.push(placed(
            at,
            Effect::Presentation {
                kind,
                cond: Cond::EmitOk(paired),
            },
        ));
        self
    }
}

pub(crate) fn decision(
    verdict: Verdict,
    transition: Option<(Status, Status)>,
    effects: Vec<Placed>,
    row: &'static str,
) -> Decision {
    Decision {
        verdict,
        transition,
        effects,
        continuation: None,
        reply: None,
        row: RowId(row),
    }
}

/// A row with no effect: the caller's straight-line code proceeds.
pub(crate) fn nothing(verdict: Verdict, row: &'static str) -> Decision {
    decision(verdict, None, Vec::new(), row)
}

// ---------------------------------------------------------------------- 207

/// Site 207, `Ready` (C3.6 R207.1-.5; `handle_audio_lifecycle_with` `Ready` arm).
///
/// * R207.1 `rid != current`, R207.2 current but Idle / Recovering / Processing:
///   `In:TERM(Superseded/StaleOwner)`.
/// * R207.3 current and `Recording`: a repeat `Ready` for the live recording claims no
///   terminal (it used to claim `Superseded`, which split from the outcome of the
///   recording's own end: PD-23, and PD-4's terminal half). The `SD` attempt still runs
///   and still starts a second ticker loop (PD-4's remaining half).
/// * R207.4 current, `Starting`, no context: trace only.
/// * R207.5 current, `Starting`, context installed: `Starting→Recording`,
///   `In:ST, TL, EM(recording)`.
///
/// Every row ends with `PDT:SD`: the wrapper attempts the side tasks whatever the
/// arm decided (4874-4881). The context read happens only after the stale check
/// passed (D-S1-2): the first call answers `NeedObservation(ContextPresence)`
/// before any mutation.
pub fn decide_sup_ready(core: &DictationCore, input: SupReadyInput, _now: Now) -> Decision {
    let side_tasks = placed(Placement::PostDT, Effect::StartSideTasks);
    if input.rid != core.current_id {
        return stale_ready(StaleKind::StaleRecording, "R207.1", side_tasks);
    }
    match core.status {
        Status::Idle | Status::Recovering | Status::Processing => decision(
            Verdict::Applied,
            None,
            vec![superseded(Placement::In), side_tasks],
            "R207.2",
        ),
        // A repeat `Ready` for the live recording claims no terminal: the stop, cancel or
        // pipeline that really ends the recording owns it, so the terminal and the
        // performance outcome name the same end (PD-23). The side-tasks attempt stays.
        Status::Recording => decision(Verdict::Applied, None, vec![side_tasks], "R207.3"),
        Status::Starting => match input.context_present {
            None => nothing(
                Verdict::NeedObservation(ObsKind::ContextPresence { id: input.rid }),
                "R207.4",
            ),
            Some(false) => decision(Verdict::Continue, None, vec![side_tasks], "R207.4"),
            Some(true) => decision(
                Verdict::Applied,
                Some((Status::Starting, Status::Recording)),
                vec![
                    set_status(Status::Recording),
                    placed(Placement::In, Effect::TouchLastTranscription),
                    emit_status(Placement::In, Status::Recording),
                    side_tasks,
                ],
                "R207.5",
            ),
        },
    }
}

fn stale_ready(kind: StaleKind, row: &'static str, side_tasks: Placed) -> Decision {
    decision(
        Verdict::Stale(kind),
        None,
        vec![superseded(Placement::In), side_tasks],
        row,
    )
}

// ---------------------------------------------------------------------- 208

/// Site 208, `StillConnecting` (R208.1-.2): the stall event is emitted under D
/// only for the current recording while `Starting`; every other case is silent.
pub fn decide_sup_still_connecting(core: &DictationCore, rid: RecordingId, _now: Now) -> Decision {
    if rid == core.current_id && core.status == Status::Starting {
        decision(
            Verdict::Applied,
            None,
            vec![emit_event(Placement::In, EventKind::InitStalled)],
            "R208.1",
        )
    } else if rid != core.current_id {
        nothing(Verdict::Stale(StaleKind::StaleRecording), "R208.2")
    } else {
        nothing(Verdict::Continue, "R208.2")
    }
}

// ---------------------------------------------------------------------- 209

/// Site 209, `Recovering{reason}` (R209.1-.7).
///
/// * R209.1 `rid != current`, R209.2 current but not Starting / Recording
///   (**PD-5**: a `HardDeadline` while `Processing` emits `Superseded` for a
///   current recording): `In:TERM(Superseded/StaleOwner)`.
/// * Otherwise `In:ST→Recovering`, then (D released) `PDT:` `CX-` unless the
///   reason is `RuntimeFailure`, `EM(recovering)`, `PRES` if it was `Ok`,
///   `EV(RecoveryStarted)`, `PC(Cancelled)` unless `HardDeadline` or
///   `RuntimeFailure` (**PD-24 fixed**: from `Starting` it completes only a run that
///   exists, so a start the failed call already aborted leaves no claim), then for `User` `TERM(UserCancelled…)` and
///   `EV(Cancelled)`, for a device or sleep reason `TERM(RuntimeInterruption/…)`,
///   and nothing more for `RuntimeFailure` / `HardDeadline` (R209.3-.7).
pub fn decide_sup_recovering(
    core: &DictationCore,
    input: SupRecoveringInput,
    _now: Now,
) -> Decision {
    let SupRecoveringInput { rid, reason } = input;
    if rid != core.current_id {
        return decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            vec![superseded(Placement::In)],
            "R209.1",
        );
    }
    let previous = core.status;
    if !matches!(previous, Status::Starting | Status::Recording) {
        return decision(
            Verdict::Applied,
            None,
            vec![superseded(Placement::In)],
            "R209.2",
        );
    }
    use AudioCancelReason::*;
    let mut fx = Fx::default();
    fx.push(set_status(Status::Recovering));
    if reason != RuntimeFailure {
        fx.push(clear_context(Placement::PostDT, rid));
    }
    fx.push(emit_status(Placement::PostDT, Status::Recovering))
        .presentation_of_last_emit(Placement::PostDT, PresentationKind::CleanupInProgress)
        .push(emit_event(Placement::PostDT, EventKind::RecoveryStarted));
    if !matches!(reason, HardDeadline | RuntimeFailure) {
        fx.push(placed(
            Placement::PostDT,
            Effect::PerfComplete(PerfComplete {
                outcome: PerfOutcome::Cancelled,
                stage: PerfStage::CaptureFinalization,
                cause: None,
                with_audio_duration: false,
                // PD-24: while `Starting` the start's own failure (the call's 2 s
                // receiver timeout queues this very cancel) may already have aborted
                // the run; the completion then has no run to complete.
                only_if_run: previous == Status::Starting,
            }),
        ));
    }
    let row = match reason {
        User => {
            let (outcome, code, row) = if previous == Status::Starting {
                (
                    TerminalOutcome::UserCancelledStarting,
                    ErrCode::CancelledStarting,
                    "R209.3",
                )
            } else {
                (
                    TerminalOutcome::UserCancelledRecording,
                    ErrCode::CancelledRecording,
                    "R209.4",
                )
            };
            fx.push(terminal(Placement::PostDT, outcome, code))
                .push(emit_event(Placement::PostDT, EventKind::Cancelled));
            row
        }
        DeviceChanged | SystemSleep | SystemWake => {
            let code = match reason {
                DeviceChanged => ErrCode::DeviceChanged,
                SystemSleep => ErrCode::SystemSleep,
                _ => ErrCode::SystemWake,
            };
            fx.push(terminal(
                Placement::PostDT,
                TerminalOutcome::RuntimeInterruption,
                code,
            ));
            "R209.5"
        }
        RuntimeFailure => "R209.6",
        HardDeadline => "R209.7",
    };
    decision(
        Verdict::Applied,
        Some((previous, Status::Recovering)),
        fx.0,
        row,
    )
}

// ---------------------------------------------------------------------- 210

/// Site 210, `CleanupFinished` (R210.1-.4; the D section of
/// `complete_dictation_capture_cleanup`). Capture completion is independent of
/// ASR completion, so a newer recording is never reset (`rid != current`: no
/// terminal, no emit); `Idle` and `Recovering` are restored to `Idle`
/// (`ST→Idle` even from `Idle`: the self-loop); the other statuses only see the
/// event.
pub fn decide_sup_cleanup_finished(core: &DictationCore, rid: RecordingId, _now: Now) -> Decision {
    if rid != core.current_id {
        return nothing(Verdict::Stale(StaleKind::StaleRecording), "R210.1");
    }
    match core.status {
        Status::Idle | Status::Recovering => {
            let row = if core.status == Status::Idle {
                "R210.2"
            } else {
                "R210.3"
            };
            decision(
                Verdict::Applied,
                Some((core.status, Status::Idle)),
                vec![
                    set_status(Status::Idle),
                    emit_event(Placement::In, EventKind::CleanupFinished),
                    emit_status(Placement::In, Status::Idle),
                ],
                row,
            )
        }
        Status::Starting | Status::Recording | Status::Processing => decision(
            Verdict::Applied,
            None,
            vec![emit_event(Placement::In, EventKind::CleanupFinished)],
            "R210.4",
        ),
    }
}

// ---------------------------------------------------------------------- 211

/// Site 211, `Idle` (R211.1-.6), everything under D.
///
/// * R211.1 `rid != current`: `In:TERM(Superseded/StaleOwner)`.
/// * R211.2 current and already `Idle`: no terminal, no `KB` reset. The
///   recording is over, so the path that ended it owns the terminal. This `Idle` is the
///   supervisor's notification of an undelivered stop (or of a finished attempt) and can
///   arrive while a cancel's tail or a stop's continuation has still to claim its own
///   terminal: a `Superseded` claim here won that terminal and split it from the
///   performance outcome (PD-22, and with it PD-2, the same claim; D-PD-1).
/// * R211.3-.6 current and `Processing` / `Starting` / `Recording` / `Recovering`
///   (**PD-7** for `Processing`: the supervisor forces `Idle` over a running
///   pipeline): `In:TERM(PipelineFailure/
///   RuntimeFailure)`, `PC(Failed)`, `ST→Idle`, `CX-`, `KB(false)`, `EM(idle)`.
pub fn decide_sup_idle(core: &DictationCore, rid: RecordingId, _now: Now) -> Decision {
    if rid != core.current_id {
        return decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            vec![superseded(Placement::In)],
            "R211.1",
        );
    }
    let row = match core.status {
        // The recording is over: whichever path ended it owns the terminal (PD-22).
        Status::Idle => return nothing(Verdict::Applied, "R211.2"),
        Status::Processing => "R211.3",
        Status::Starting => "R211.4",
        Status::Recording => "R211.5",
        Status::Recovering => "R211.6",
    };
    decision(
        Verdict::Applied,
        Some((core.status, Status::Idle)),
        vec![
            terminal(
                Placement::In,
                TerminalOutcome::PipelineFailure,
                ErrCode::RuntimeFailure,
            ),
            perf_failed(Placement::In, false),
            set_status(Status::Idle),
            clear_context(Placement::In, rid),
            placed(Placement::In, Effect::KeyboardProcessing(false)),
            emit_status(Placement::In, Status::Idle),
        ],
        row,
    )
}

// ---------------------------------------------------------------------- 212

/// The effects of site 212 that today's code runs **before** it reads the current
/// id (4875-4881): the telemetry `accepted` claim and the diagnostics claim (`ACC`,
/// both mutex-taking). The shell runs them first, reads [`LockFreeFacts`], decides,
/// and drops them from the decision ([`Decision::after_prefix`]), so a generation
/// that advances during them is seen exactly as it is today. Always the leading
/// effects of [`decide_sup_accepted_nolock`].
pub fn accepted_prefix() -> Vec<Placed> {
    vec![placed(Placement::PostDT, Effect::AcceptTelemetry)]
}

/// The row of [`accepted_prefix`] when run alone.
pub const ACCEPTED_PREFIX_ROW: RowId = RowId("R212.1");

/// Site 212, `Accepted` (R212.1), D-less: the telemetry `accepted` claim, then a
/// `Superseded` terminal if the recording is no longer current (read without D).
/// The claim is [`accepted_prefix`], run before the facts are read.
pub fn decide_sup_accepted_nolock(facts: LockFreeFacts, rid: RecordingId, _now: Now) -> Decision {
    let mut effects = vec![placed(Placement::PostDT, Effect::AcceptTelemetry)];
    if rid != facts.current_id {
        effects.push(superseded(Placement::PostDT));
    }
    decision(Verdict::Applied, None, effects, "R212.1")
}

// ---------------------------------------------------------------------- 213

/// The effect of site 213 that today's code runs **before** it reads the current
/// id (4893): `PCF(kind)`, which takes the production-metrics mutex. The shell runs
/// it first, reads [`LockFreeFacts`], decides, and drops it from the decision
/// ([`Decision::after_prefix`]). Always the leading effect of
/// [`decide_sup_init_failed_nolock`].
pub fn init_failed_prefix(kind: Handle) -> Vec<Placed> {
    vec![placed(
        Placement::PostDT,
        Effect::ProductionCaptureFailure(kind),
    )]
}

/// The row of [`init_failed_prefix`] when run alone (common to R213.1 and R213.2).
pub const INIT_FAILED_PREFIX_ROW: RowId = RowId("R213.1");

/// Site 213, `InitializationFailed` (R213.1-.2), D-less; **the status is never
/// written** (it stays `Starting` until the `Idle` event, R211.4, or R102.2).
/// `PCF(kind)` is always first (4893), and is [`init_failed_prefix`].
pub fn decide_sup_init_failed_nolock(
    facts: LockFreeFacts,
    input: SupInitFailedInput,
    _now: Now,
) -> Decision {
    let at = Placement::PostDT;
    let mut fx = Fx::default();
    fx.push(placed(at, Effect::ProductionCaptureFailure(input.kind)));
    if input.rid != facts.current_id {
        fx.push(superseded(at));
        return decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            fx.0,
            "R213.1",
        );
    }
    let outcome = if input.recovery_reason == Some(AudioCancelReason::RuntimeFailure) {
        TerminalOutcome::RuntimeInterruption
    } else {
        TerminalOutcome::CaptureInitFailure
    };
    fx.push(terminal(at, outcome, input.error_code))
        .push(clear_context(at, input.rid))
        .push(perf_failed(at, false))
        .push(emit_event(at, EventKind::InitFailed))
        .presentation_of_last_emit(at, PresentationKind::InitFailed);
    decision(Verdict::Applied, None, fx.0, "R213.2")
}

// ---------------------------------------------------------------------- 214

/// Site 214, `RecoveryStalled` (R214.1-.2), D-less: the stall event and its
/// presentation only for the current recording.
pub fn decide_sup_stalled_nolock(facts: LockFreeFacts, rid: RecordingId, _now: Now) -> Decision {
    if rid != facts.current_id {
        return nothing(Verdict::Stale(StaleKind::StaleRecording), "R214.2");
    }
    let mut fx = Fx::default();
    fx.push(emit_event(Placement::PostDT, EventKind::RecoveryStalled))
        .presentation_of_last_emit(Placement::PostDT, PresentationKind::RecoveryStalled);
    decision(Verdict::Applied, None, fx.0, "R214.1")
}

// ---------------------------------------------------------------------- 215

/// Site 215, `Interrupted` (R215.1-.3), D-less (`is_current()` is read without
/// D, 5159).
///
/// * R215.1 `rid != current`: `PDT:TI(d)`, `TERM(Superseded/StaleOwner)`,
///   `PC(Failed, audio duration)`.
/// * R215.2 current and `delivered_samples >= 8000`: `PDT:EV(Interrupted{auto}),
///   PRES, RS(rid, Other)`; returns the `AutoStop` continuation spec (consumed by
///   the spawned stop's R201).
/// * R215.3 current and shorter: `PDT:EV(Interrupted{auto: false}), PRES`; the
///   shell then enters site 216 ([`R215_3`]).
pub fn decide_sup_interrupted_nolock(
    facts: LockFreeFacts,
    input: SupInterruptedInput,
    _now: Now,
) -> Decision {
    let at = Placement::PostDT;
    let rid = input.rid;
    if rid != facts.current_id {
        return decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            vec![
                take_interrupted_dispose(at, rid),
                superseded(at),
                perf_failed(at, true),
            ],
            "R215.1",
        );
    }
    let auto_transcribe = input.delivered_samples >= AUTO_TRANSCRIBE_MIN_SAMPLES;
    let mut fx = Fx::default();
    fx.push(emit_event(at, EventKind::Interrupted { auto_transcribe }))
        .presentation_of_last_emit(at, PresentationKind::Interrupted);
    if auto_transcribe {
        fx.push(placed(
            at,
            Effect::SelfStop {
                id: rid,
                trigger: StopTrigger::Other,
            },
        ));
        let mut decided = decision(Verdict::Applied, None, fx.0, "R215.2");
        decided.continuation = Some(ContinuationSpec {
            recording_id: rid,
            kind: ContKind::AutoStop,
        });
        decided
    } else {
        decision(Verdict::Applied, None, fx.0, R215_3.0)
    }
}

// ---------------------------------------------------------------------- 216

/// The input-derived effects of site 216 that run **before** its D section
/// (C2.1 `pre_effects()`, 5214-5227): destructive `TI(d)`, `TERM`, `CX-`, `PC`.
pub fn short_idle_pre_effects(rid: RecordingId) -> Vec<Placed> {
    let at = Placement::Pre;
    vec![
        take_interrupted_dispose(at, rid),
        terminal(
            at,
            TerminalOutcome::RuntimeInterruption,
            ErrCode::RuntimeFailure,
        ),
        clear_context(at, rid),
        perf_failed(at, true),
    ]
}

/// The row id of [`short_idle_pre_effects`] when run alone (the decision's own
/// row depends on the status the D section reads).
pub const SHORT_IDLE_PRE_ROW: RowId = RowId("R216.1");

/// Site 216, the second half of a short `Interrupted` (R216.1-.5): a D section
/// that holds D for **one statement** (5228-5230) with **no id or status guard**
/// (**PD-1**), then `PDT:KB(false)`, `EM(idle)`. The `Pre` effects come first.
///
/// R216.1 `Recovering` (**PD-15**: `TI(d)` is destructive),
/// R216.2 `Idle`, R216.3 `Starting` / `Recording` (kills a live recording of the
/// same id), R216.4 `Processing` (**PD-7**), R216.5 `rid != current`: `Idle`
/// written over a newer recording (no production schedule, D3).
pub fn decide_sup_interrupted_short_idle(
    core: &DictationCore,
    rid: RecordingId,
    _now: Now,
) -> Decision {
    let row = if rid != core.current_id {
        "R216.5"
    } else {
        match core.status {
            Status::Recovering => "R216.1",
            Status::Idle => "R216.2",
            Status::Starting | Status::Recording => "R216.3",
            Status::Processing => "R216.4",
        }
    };
    let mut effects = short_idle_pre_effects(rid);
    effects.push(set_status(Status::Idle));
    effects.push(placed(Placement::PostDT, Effect::KeyboardProcessing(false)));
    effects.push(emit_status(Placement::PostDT, Status::Idle));
    decision(
        Verdict::Applied,
        Some((core.status, Status::Idle)),
        effects,
        row,
    )
}

#[cfg(test)]
mod tests;
