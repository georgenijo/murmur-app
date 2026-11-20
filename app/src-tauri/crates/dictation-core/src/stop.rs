//! Stage 2a-2: the stop-family decide functions (appendix C2.3 sites 201, 202,
//! 202n, 203, 204a, 204b; rows of C3.5 R201.1-R204b.6).
//!
//! One pure function per D critical section of the stop command, the pipeline
//! guard and `IdleGuard`. Each takes the decision view (`&DictationCore`, or
//! [`LockFreeFacts`] for the `*_nolock` entries that hold no D today) and a typed
//! input, **mutates nothing**, and returns a [`Decision`] with the ordered,
//! placed effects and the typed reply of its row. The shell calls each one
//! inline at today's line with D (and T) held exactly as today, and runs the
//! effects through the executor at their placement; M1 commits at the `ST`
//! effect, never here (C1.4.2). None of these sites reads M3 or M4.
//!
//! Continuations (H1): R201.7/.8 return `K:StopContext`, R201.4
//! `K:CaptureCancel`, R202.1/.2 `K:CaptureStop`, R203.9/.10 `K:Pipeline`. Every
//! consuming input carries an `op_status` that no row branches on (C1.4.1
//! rule 3; the op-status parity tests pin it).
//!
//! Shapes the appendix does not print, decided from the code (code wins):
//!
//! * **Stale stop (R201.1).** The stale check runs under T with no D, so it is
//!   [`decide_stop_request_stale`], a pure function of the input that
//!   [`decide_stop_request`] also calls first. Decision D-S1-3: the stale path
//!   asks for `NeedObservation(TakeInterrupted{expected})` before it decides; the
//!   shell's answer *is* the destructive `TI(d)`, so the decision's effect list
//!   carries `Pre:TERM` only (no double take).
//! * **Pre effects.** `Pre` effects that are a pure function of the input
//!   ([`stop_capture_pre_effects`], [`guard_finish_pre_effects`]) run before D;
//!   the decision returns them prepended and the shell drops them with
//!   [`Decision::after_pre`].
//! * **Tail.** R204b.5/.6 are D-less and depend on the pipeline's result, so they
//!   are [`decide_pipeline_idle_emit_tail`], called after the D section.
//! * **X rows** (status/id combinations no schedule produces) keep the literal
//!   behaviour of the code (the guard writes `Idle` on an id match whatever the
//!   status), and take the row id of the nearest printed row.
//!
//! Rows that reproduce a preserved defect keep it: PD-3 (R202.3/.4), PD-6
//! (R203.2r, R203.5r, R203.7r, R204a.3, R204b.2), PD-7 (R202.2, R203.2,
//! R203.3, R203.10), PD-13 (R204a.6), PD-15 (R201.1, R201.7), PD-18 (c) (R203.4,
//! R203.6).

use crate::decision::{
    ContKind, ContinuationSpec, Decision, ObsKind, Refusal, RowId, StaleKind, Verdict,
};
use crate::effects::{
    AxMode, Effect, EventKind, PerfCause, PerfComplete, PerfOutcome, PerfStage, PipelineEnter,
    Placed, Placement as At,
};
use crate::ids::{Now, RecordingId};
use crate::inputs::{
    GuardFinishInput, GuardOrigin, PipelineIdleEmitInput, PipelineResult, PipelineTerminal,
    StopCaptureResult, StopCaptureResultInput, StopContextInput, StopInput,
};
use crate::lifecycle::{
    clear_context, decision, emit_status, nothing, placed, set_status, superseded, terminal,
};
use crate::replies::{ErrText, Reply, ReplyType, TranscriptText};
use crate::state::{DictationCore, LockFreeFacts};
use crate::status::{AudioCancelReason, ErrCode, Status, TerminalOutcome};

/// `MIN_RECORDING_SAMPLES` (6137): fewer samples than this (0.3 s at 16 kHz) are
/// discarded as phantom triggers; at least this many start the pipeline.
pub const MIN_RECORDING_SAMPLES: usize = 4_800;

fn with_reply(mut decided: Decision, reply: Reply) -> Decision {
    decided.reply = Some(reply);
    decided
}

fn with_continuation(mut decided: Decision, rid: RecordingId, kind: ContKind) -> Decision {
    decided.continuation = Some(ContinuationSpec {
        recording_id: rid,
        kind,
    });
    decided
}

// ---------------------------------------------------------------------- 201

/// The stale path of site 201 (R201.1), decided with no D: `expected` is not the
/// recording observed under T (5900). `None` when the stop is not stale.
///
/// First call (`interrupted` absent): `NeedObservation(TakeInterrupted{expected})`
/// before any effect (decision D-S1-3, PD-15: the take is destructive and also
/// happens for a stale owner). Second call: `Pre:TERM(Superseded/StaleOwner)` for
/// the *expected* id and the `recording_superseded` reply.
pub fn decide_stop_request_stale(input: StopInput, _now: Now) -> Option<Decision> {
    if !input.is_stale() {
        return None;
    }
    let expected = input
        .expected
        .expect("a stale stop carries the expectation that mismatched");
    if input.interrupted.is_none() {
        return Some(nothing(
            Verdict::NeedObservation(ObsKind::TakeInterrupted { id: expected }),
            "R201.1",
        ));
    }
    Some(with_reply(
        decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            vec![superseded(At::Pre)],
            "R201.1",
        ),
        Reply::typed(ReplyType::RecordingSuperseded, Status::Idle),
    ))
}

/// Site 201, `decide_stop_request` (R201.1-.8): the stale check (T only), then the
/// D section 5905-5959.
///
/// * R201.1 stale: see [`decide_stop_request_stale`].
/// * R201.2 `Processing`: `already_processing`. R201.3 `Idle`: `not_recording`.
/// * R201.4 `Starting`: `PDT:TS`, `PT:CC(rid, User)`, `K:CaptureCancel`, reply
///   `recording_cancelled` (the capture-cancel `Err` is R206, a later PR).
/// * R201.5 `Recovering`, nothing taken yet: `NeedObservation(TakeInterrupted)`.
///   R201.6 nothing preserved (PD-15: another path already took it):
///   `audio_recovering`. R201.7 taken (**PD-15**: destructive):
///   `In:PH, ST→Processing, AX(s)`, `PDT:TS`, `K:StopContext`.
/// * R201.8 `Recording`: the same effects as R201.7 from `Recording`; `PH` comes
///   before the write so the ticker sees the claim whenever it sees the stop.
pub fn decide_stop_request(core: &DictationCore, input: StopInput, now: Now) -> Decision {
    if let Some(stale) = decide_stop_request_stale(input, now) {
        return stale;
    }
    let rid = core.current_id;
    let accepted = |from: Status, row: &'static str| {
        let decided = decision(
            Verdict::Applied,
            Some((from, Status::Processing)),
            vec![
                placed(At::In, Effect::PreviewHideClaim),
                set_status(Status::Processing),
                placed(At::In, Effect::AxSnapshot(AxMode::Stop)),
                placed(At::PostDT, Effect::TakeStopStamps),
            ],
            row,
        );
        with_continuation(decided, rid, ContKind::StopContext)
    };
    match core.status {
        Status::Processing => with_reply(
            nothing(Verdict::Refused(Refusal::AlreadyProcessing), "R201.2"),
            Reply::typed(ReplyType::AlreadyProcessing, Status::Processing),
        ),
        Status::Idle => with_reply(
            nothing(Verdict::Refused(Refusal::NotRecording), "R201.3"),
            Reply::typed(ReplyType::NotRecording, Status::Idle),
        ),
        Status::Starting => with_continuation(
            with_reply(
                decision(
                    Verdict::Applied,
                    None,
                    vec![
                        placed(At::PostDT, Effect::TakeStopStamps),
                        placed(
                            At::PostT,
                            Effect::CaptureCancel {
                                id: rid,
                                reason: AudioCancelReason::User,
                            },
                        ),
                    ],
                    "R201.4",
                ),
                Reply::typed(ReplyType::RecordingCancelled, Status::Recovering),
            ),
            rid,
            ContKind::CaptureCancel,
        ),
        Status::Recovering => match input.interrupted {
            None => nothing(
                Verdict::NeedObservation(ObsKind::TakeInterrupted { id: rid }),
                "R201.5",
            ),
            Some(None) => with_reply(
                nothing(Verdict::Refused(Refusal::AudioRecovering), "R201.6"),
                Reply::typed(ReplyType::AudioRecovering, Status::Recovering),
            ),
            Some(Some(_meta)) => accepted(Status::Recovering, "R201.7"),
        },
        Status::Recording => accepted(Status::Recording, "R201.8"),
    }
}

// ---------------------------------------------------------------------- 202

/// The effects of a context-present stop (R202.1/.2): `PDT:KB(true),
/// EM(processing)`, then, with T released (6053), `PT:CP(rid)`: the capture stop.
/// A capture the supervisor already preserved (an interrupted one) bypasses the
/// supervisor in the shell's `CP` port and goes straight to R203.
fn context_present_effects(rid: RecordingId) -> Vec<Placed> {
    vec![
        placed(At::PostDT, Effect::KeyboardProcessing(true)),
        emit_status(At::PostDT, Status::Processing),
        placed(At::PostT, Effect::CaptureStop(rid)),
    ]
}

/// Site 202n, `decide_stop_context_nolock` (R202.1-.2), D-less: the context is
/// present. Status is not read, so the schedule where a supervisor `Idle`
/// landed first (R202.2, **PD-7**) takes the same decision as R202.1: the
/// `processing` emit follows the supervisor's `idle` (window W-stop-1).
/// `PDT:KB(true), EM(processing)`, `PT:CP`; `K:CaptureStop`.
pub fn decide_stop_context_nolock(
    _facts: LockFreeFacts,
    input: StopContextInput,
    _now: Now,
) -> Decision {
    with_continuation(
        decision(
            Verdict::Applied,
            None,
            context_present_effects(input.rid),
            "R202.1",
        ),
        input.rid,
        ContKind::CaptureStop,
    )
}

/// The `Pre` effect of a context-less stop (R202.3-.5, 6003): `TERM(PipelineFailure /
/// MissingContext)` runs with T held and before D is taken, so the shell runs it
/// from here and drops it from the decision with [`Decision::after_pre`].
pub fn stop_context_pre_effects() -> Vec<Placed> {
    vec![terminal(
        At::Pre,
        TerminalOutcome::PipelineFailure,
        ErrCode::MissingContext,
    )]
}

/// Site 202, `decide_stop_context` (R202.3-.5): D is held (5996 read the context
/// with no D, and found none).
///
/// `Pre:TERM(PipelineFailure/MissingContext)` (T held, no D); then, under D,
/// `ST→Idle` **only if `rid == current_id`** (written even from `Idle`:
/// **PD-3**), `KB(false)` and `EM(idle)` unconditionally, and `PC(Failed,
/// CaptureFinalization, InternalEarlyExit)`. The shell releases T inside D
/// between `EM` and `PC` (B0 E1); there is no `PDT`/`PT` effect. Reply
/// `Err("Missing dictation context for recording {rid}")`.
///
/// * R202.3 `Processing` (**PD-3**), R202.4 `Idle` (**PD-3**, **PD-7**).
/// * R202.5 `Starting` / `Recording` / `Recovering` (X), and any status with the
///   context present: the context-present decision (R202.1's effects).
pub fn decide_stop_context(core: &DictationCore, input: StopContextInput, now: Now) -> Decision {
    if input.context_present {
        let mut decided = decide_stop_context_nolock(core.lock_free_facts(), input, now);
        decided.row = RowId("R202.5");
        return decided;
    }
    let row = match core.status {
        Status::Processing => "R202.3",
        Status::Idle => "R202.4",
        Status::Starting | Status::Recording | Status::Recovering => "R202.5",
    };
    let mut effects = stop_context_pre_effects();
    let transition = if input.rid == core.current_id {
        effects.push(set_status(Status::Idle));
        Some((core.status, Status::Idle))
    } else {
        None
    };
    effects.push(placed(At::In, Effect::KeyboardProcessing(false)));
    effects.push(emit_status(At::In, Status::Idle));
    effects.push(placed(
        At::In,
        Effect::PerfComplete(PerfComplete {
            outcome: PerfOutcome::Failed,
            stage: PerfStage::CaptureFinalization,
            cause: Some(PerfCause::InternalEarlyExit),
            with_audio_duration: false,
            only_if_run: false,
        }),
    ));
    with_reply(
        decision(Verdict::Applied, transition, effects, row),
        Reply::Err(ErrText::MissingContext(input.rid)),
    )
}

// --------------------------------------------------------------------- 204a

/// The `Pre` effect of `IdleGuard::finish` (1063): `CX-` runs before D is taken.
pub fn guard_finish_pre_effects(rid: RecordingId) -> Vec<Placed> {
    vec![clear_context(At::Pre, rid)]
}

fn row_204a(core: &DictationCore, input: &GuardFinishInput) -> &'static str {
    if input.origin == GuardOrigin::Dropped {
        return "R204a.6";
    }
    if input.rid == core.current_id {
        match core.status {
            Status::Processing => "R204a.1",
            Status::Idle => "R204a.2",
            Status::Recovering => "R204a.3",
            Status::Starting | Status::Recording => "R204a.4",
        }
    } else {
        match core.status {
            Status::Idle => "R204a.5a",
            Status::Starting => "R204a.5b",
            Status::Recording => "R204a.5c",
            Status::Recovering => "R204a.5d",
            Status::Processing => "R204a.5e",
        }
    }
}

/// Site 204a, `decide_guard_finish` (R204a.1-.6): the D section of
/// `IdleGuard::finish` / `Drop`, shared by the stop capture result, the pipeline
/// guard, the legacy parse failure and a dropped guard.
///
/// Fixed order: `Pre:CX-` (before D), then D, compare **id only** (status never
/// read), `ST→Idle`, `KB(false)`, and `EM(idle)` iff `closure_emit`. A stale
/// guard (`rid != current_id`) clears the context only: no write, no `KB`, no
/// `EM`. The status is written whatever it is (R204a.2 duplicate `idle`;
/// **PD-6** R204a.3 `Recovering→Idle`). R204a.4 (`Starting`/`Recording` with the
/// guard's own id) cannot occur; the code would still write `Idle`, so that is
/// what the row does. `Dropped` (**PD-13**): R204a.6, no `EM`, **no `TERM`**.
///
/// History (H2 `ret rid`, H1 `ret op_pl` for `PipelineExit` / `Dropped`) is the
/// shell's bookkeeping after this returns.
pub fn decide_guard_finish(core: &DictationCore, input: GuardFinishInput, _now: Now) -> Decision {
    let row = row_204a(core, &input);
    let mut effects = guard_finish_pre_effects(input.rid);
    if input.rid != core.current_id {
        return decision(
            Verdict::Stale(StaleKind::StaleRecording),
            None,
            effects,
            row,
        );
    }
    effects.push(set_status(Status::Idle));
    effects.push(placed(At::In, Effect::KeyboardProcessing(false)));
    if input.closure_emit {
        effects.push(emit_status(At::In, Status::Idle));
    }
    decision(
        Verdict::Applied,
        Some((core.status, Status::Idle)),
        effects,
        row,
    )
}

// ---------------------------------------------------------------------- 203

#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    Failed,
    Empty,
    Short,
    Long,
}

fn capture_kind(result: StopCaptureResult) -> Capture {
    match result {
        StopCaptureResult::Err => Capture::Failed,
        StopCaptureResult::Samples { n: 0, .. } => Capture::Empty,
        StopCaptureResult::Samples { n, .. } if n < MIN_RECORDING_SAMPLES => Capture::Short,
        StopCaptureResult::Samples { .. } => Capture::Long,
    }
}

/// The `Pre` effects of R203.1-.8 that depend on the input alone (6068, 6125-6132,
/// 6158-6165, then `IdleGuard::finish`'s `CX-` 1063): the result's terminal, then
/// `CX-`. Empty for a result that starts the pipeline (R203.9/.10).
pub fn stop_capture_pre_effects(input: &StopCaptureResultInput) -> Vec<Placed> {
    let (outcome, code) = match (input.result, capture_kind(input.result)) {
        (_, Capture::Long) => return Vec::new(),
        (_, Capture::Failed) => (
            TerminalOutcome::StopFailure,
            ErrCode::StopFinalizationFailed,
        ),
        (
            StopCaptureResult::Samples {
                interrupted: Some(code),
                ..
            },
            _,
        ) => (TerminalOutcome::RuntimeInterruption, code),
        (StopCaptureResult::Samples { .. }, Capture::Empty) => {
            (TerminalOutcome::NoSpeech, ErrCode::EmptyAudio)
        }
        (StopCaptureResult::Samples { .. }, Capture::Short) => {
            (TerminalOutcome::TooShort, ErrCode::BelowMinimumDuration)
        }
        (StopCaptureResult::Err, _) => unreachable!("Err is Capture::Failed"),
    };
    let mut effects = vec![terminal(At::Pre, outcome, code)];
    effects.extend(guard_finish_pre_effects(input.rid));
    effects
}

fn row_203(kind: Capture, core: &DictationCore, rid: RecordingId) -> &'static str {
    if rid == core.current_id {
        return match (kind, core.status) {
            (Capture::Failed, Status::Idle) => "R203.2",
            (Capture::Failed, Status::Recovering) => "R203.2r",
            (Capture::Failed, Status::Processing | Status::Starting | Status::Recording) => {
                "R203.1"
            }
            (Capture::Empty, Status::Idle) => "R203.5",
            (Capture::Empty, Status::Recovering) => "R203.5r",
            (Capture::Empty, Status::Processing | Status::Starting | Status::Recording) => "R203.4",
            (Capture::Short, Status::Idle) => "R203.7",
            (Capture::Short, Status::Recovering) => "R203.7r",
            (Capture::Short, Status::Processing | Status::Starting | Status::Recording) => "R203.6",
            (Capture::Long, _) => unreachable!("a long capture takes the D-less entry"),
        };
    }
    match (kind, core.status) {
        (Capture::Failed, Status::Idle) => "R203.3a",
        (Capture::Failed, Status::Starting) => "R203.3b",
        (Capture::Failed, Status::Recording) => "R203.3c",
        (Capture::Failed, Status::Recovering) => "R203.3d",
        (Capture::Failed, Status::Processing) => "R203.3e",
        (Capture::Empty | Capture::Short, Status::Idle) => "R203.8a",
        (Capture::Empty | Capture::Short, Status::Starting) => "R203.8b",
        (Capture::Empty | Capture::Short, Status::Recording) => "R203.8c",
        (Capture::Empty | Capture::Short, Status::Recovering) => "R203.8d",
        (Capture::Empty | Capture::Short, Status::Processing) => "R203.8e",
        (Capture::Long, _) => unreachable!("a long capture takes the D-less entry"),
    }
}

/// Site 203, `decide_stop_capture_result` (R203.1-.8): the capture stop failed
/// (`Err`) or returned fewer than [`MIN_RECORDING_SAMPLES`] samples. The D
/// section is 204a with `origin = StopCapture` and `closure_emit = true`; this
/// adds the result-specific effects:
///
/// `Pre:TERM(...)`, `Pre:CX-`, then 204a's `In:ST→Idle, KB(false), EM(idle)` iff
/// the guard's id is current (a stale result: no `In` effect), then `PDT:` `PC(Failed,
/// CaptureFinalization, AudioCaptureFailed)` for `Err` (reply `Err(e)`) or
/// `PGF(NoSpeech)` for a short or empty capture (reply `transcription ""`). A
/// supervisor `RuntimeInterruption` terminal replaces the `NoSpeech` / `TooShort`
/// one. Rows by (result, status, id): see [`row_203`]; `Idle` / `Recovering`
/// with the current id keep the defects **PD-6** / **PD-7**; **PD-18 (c)** for
/// R203.4 / R203.6.
///
/// A capture of at least [`MIN_RECORDING_SAMPLES`] samples takes no D: this
/// delegates to [`decide_stop_capture_result_nolock`] (R203.9/.10).
pub fn decide_stop_capture_result(
    core: &DictationCore,
    input: StopCaptureResultInput,
    now: Now,
) -> Decision {
    let kind = capture_kind(input.result);
    if kind == Capture::Long {
        return decide_stop_capture_result_nolock(core.lock_free_facts(), input, now);
    }
    let row = row_203(kind, core, input.rid);
    let (outcome_effect, reply) = match kind {
        Capture::Failed => (
            Effect::PerfComplete(PerfComplete {
                outcome: PerfOutcome::Failed,
                stage: PerfStage::CaptureFinalization,
                cause: Some(PerfCause::AudioCaptureFailed),
                with_audio_duration: false,
                only_if_run: false,
            }),
            Reply::Err(ErrText::Shell),
        ),
        Capture::Empty | Capture::Short => (
            Effect::PipelineGuardFinish(PerfOutcome::NoSpeech),
            Reply::Transcription {
                text: TranscriptText::Empty,
                state: Status::Idle,
            },
        ),
        Capture::Long => unreachable!("handled above"),
    };
    let mut effects = stop_capture_pre_effects(&input);
    let guard = decide_guard_finish(
        core,
        GuardFinishInput {
            rid: input.rid,
            origin: GuardOrigin::StopCapture,
            closure_emit: true,
        },
        now,
    );
    // `stop_capture_pre_effects` already carries the guard's `Pre:CX-`.
    effects.extend(guard.effects.into_iter().filter(|p| p.at != At::Pre));
    effects.push(placed(At::PostDT, outcome_effect));
    with_reply(
        decision(guard.verdict, guard.transition, effects, row),
        reply,
    )
}

/// Site 203, the D-less entry (R203.9/.10): the capture returned at least
/// [`MIN_RECORDING_SAMPLES`] samples, so the pipeline starts. Neither status nor
/// id is read: a stale, cancelled or already-`Idle` stop still starts its
/// pipeline (R203.10, **PD-7**); the pipeline's own guard (204a `PipelineExit`)
/// and the delivery fence fence it. `PDT:PE(Vad)`, `PDT:PS`; `K:Pipeline`; the
/// shell disarms the stop guard first, retires `op_cs` and registers `H2 rid`.
/// Any other result is not for this entry: `Invalid`.
pub fn decide_stop_capture_result_nolock(
    _facts: LockFreeFacts,
    input: StopCaptureResultInput,
    _now: Now,
) -> Decision {
    if capture_kind(input.result) != Capture::Long {
        return nothing(
            Verdict::Invalid(crate::decision::InvalidKind::Impossible(RowId("R203.9"))),
            "R203.9",
        );
    }
    with_continuation(
        decision(
            Verdict::Applied,
            None,
            vec![
                placed(At::PostDT, Effect::PipelineEnter(PipelineEnter::Vad)),
                placed(At::PostDT, Effect::PipelineSpawn(input.rid)),
            ],
            "R203.9",
        ),
        input.rid,
        ContKind::Pipeline,
    )
}

// ---------------------------------------------------------------------- 204b

fn row_204b(core: &DictationCore, rid: RecordingId) -> &'static str {
    if rid == core.current_id {
        return match core.status {
            Status::Idle => "R204b.1",
            Status::Recovering => "R204b.2",
            Status::Processing | Status::Starting | Status::Recording => "R204b.3",
        };
    }
    match core.status {
        Status::Idle => "R204b.4a",
        Status::Starting => "R204b.4b",
        Status::Recording => "R204b.4c",
        Status::Recovering => "R204b.4d",
        Status::Processing => "R204b.4e",
    }
}

/// Site 204b, `decide_pipeline_idle_emit` (R204b.1-.4): the **second**, independent
/// D section after the pipeline returned (6190-6196 and, for the legacy path,
/// 3903-3909), with its own id guard. T was released long before and a newer
/// start may have run since 204a. Status is never read for the decision beyond
/// the row, and never written.
///
/// `rid == current_id`: `In:KB(false), In:EM(idle)`: R204b.1 `Idle` (the normal
/// case), R204b.2 `Recovering` (**PD-6**: `idle` is emitted and the keyboard gate
/// cleared while the retiring capture is still pending), R204b.3 `Processing` /
/// `Starting` / `Recording` (cannot occur: 204a wrote `Idle`; the code would
/// still emit). `rid != current_id`: R204b.4a-e, no effect at all.
///
/// The tail input of [`PipelineIdleEmitInput`] is ignored here; see
/// [`decide_pipeline_idle_emit_tail`].
pub fn decide_pipeline_idle_emit(
    core: &DictationCore,
    input: PipelineIdleEmitInput,
    _now: Now,
) -> Decision {
    let row = row_204b(core, input.rid);
    if input.rid != core.current_id {
        return nothing(Verdict::Stale(StaleKind::StaleRecording), row);
    }
    decision(
        Verdict::Applied,
        None,
        vec![
            placed(At::In, Effect::KeyboardProcessing(false)),
            emit_status(At::In, Status::Idle),
        ],
        row,
    )
}

/// Sites 204b R204b.5 / R204b.6, the D-less tail of the stop command: no D is
/// held and status is not read. `StopPost` with `result` only; anything else is
/// `Invalid`.
///
/// * R204b.5 `Err`: `PDT:TERM(PipelineFailure/TranscriptionFailed)` or
///   `TERM(RuntimeInterruption/code)` if `interrupted`; reply `Err(e)`.
/// * R204b.6 `Ok(terminal)`: `PDT:TERM(...)` (`interrupted` wins; else `Success`,
///   `NoSpeech(code)`, or `Cancelled` → `UserCancelledProcessing/CancelledProcessing`:
///   a second source of that outcome besides S9, first wins in the ledger), then
///   `PGF` (the shell's performance-guard finish, 6325), then, if the text is
///   non-empty, `EV(TranscriptionComplete)`; reply `transcription` with the
///   pipeline's text.
pub fn decide_pipeline_idle_emit_tail(input: PipelineIdleEmitInput, _now: Now) -> Decision {
    use crate::inputs::PostKind;
    let (PostKind::StopPost, Some(result)) = (input.kind, input.result) else {
        return nothing(
            Verdict::Invalid(crate::decision::InvalidKind::Impossible(RowId("R204b.6"))),
            "R204b.6",
        );
    };
    let at = At::PostDT;
    match result {
        PipelineResult::Err => {
            let (outcome, code) = match input.interrupted {
                Some(code) => (TerminalOutcome::RuntimeInterruption, code),
                None => (
                    TerminalOutcome::PipelineFailure,
                    ErrCode::TranscriptionFailed,
                ),
            };
            with_reply(
                decision(
                    Verdict::Applied,
                    None,
                    vec![terminal(at, outcome, code)],
                    "R204b.5",
                ),
                Reply::Err(ErrText::Shell),
            )
        }
        PipelineResult::Ok(pipeline) => {
            let (outcome, code) = match (input.interrupted, pipeline) {
                (Some(code), _) => (TerminalOutcome::RuntimeInterruption, code),
                (None, PipelineTerminal::Success) => (TerminalOutcome::Success, ErrCode::None),
                (None, PipelineTerminal::NoSpeech(code)) => (TerminalOutcome::NoSpeech, code),
                (None, PipelineTerminal::Cancelled) => (
                    TerminalOutcome::UserCancelledProcessing,
                    ErrCode::CancelledProcessing,
                ),
            };
            let perf = match pipeline {
                PipelineTerminal::Success => PerfOutcome::Completed,
                PipelineTerminal::NoSpeech(_) => PerfOutcome::NoSpeech,
                PipelineTerminal::Cancelled => PerfOutcome::Cancelled,
            };
            let mut effects = vec![
                terminal(at, outcome, code),
                placed(at, Effect::PipelineGuardFinish(perf)),
            ];
            if input.char_count > 0 {
                effects.push(placed(
                    at,
                    Effect::EmitEvent(EventKind::TranscriptionComplete),
                ));
            }
            with_reply(
                decision(Verdict::Applied, None, effects, "R204b.6"),
                Reply::Transcription {
                    text: TranscriptText::FromPipeline,
                    state: Status::Idle,
                },
            )
        }
    }
}

#[cfg(test)]
mod tests;
