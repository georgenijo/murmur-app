//! Stage 2a-4: the start-family decide functions (appendix C2.3 sites 101-104;
//! rows of C3.1-C3.4). Sites 101 and 102 landed in PR 2a-4a; 103 and 104 (and the
//! smart-destination query that gates them) in 2a-4b.
//!
//! One pure function per D critical section of the start command. Each takes the
//! decision view (`&DictationCore`, or [`LockFreeFacts`] for the `*_nolock`
//! entries that hold no D today) and a typed input, **mutates nothing**, and
//! returns a [`Decision`] with the ordered, placed effects and the typed reply of
//! its row. The shell calls each one inline at today's line with D and T held
//! exactly as today and runs the effects through the executor at their placement;
//! M1 and M2 commit at the `ST` / `ID+` effects, never here (C1.4.2). Neither site
//! reads M3 or M4.
//!
//! Shapes the appendix does not print, decided from the code (code wins):
//!
//! * **Pre-D refusals (R101.1-.2).** `app_disabled` and Smart Auto with a fixed
//!   device are decided with no D, so they are [`decide_start_request_pre`], a pure
//!   function of the input that [`decide_start_request`] also calls first (the
//!   `decide_stop_request_stale` precedent).
//! * **Staged reads (decision D-S1-2).** The blockers and the status step are
//!   evaluated on the partial [`Occupancy`] the shell read lazily, in today's order.
//!   The retiring-capture read (`is_dictation_cleanup_pending`, not free: it builds
//!   the supervisor on first use) happens only when the decision answers
//!   `NeedObservation(CleanupPending)`, i.e. status is `Idle` and no blocker fired;
//!   the Smart Auto resolve only after that (`NeedObservation(SmartAutoSelection)`).
//!   Both answers are `Option`s on the input; no function builds a fully populated
//!   occupancy up front. The appendix's `cleanup_pending: bool [in]` is therefore an
//!   `Option<bool>` here.
//! * **R101.12 is `Applied`, not `Refused`**: it writes `Idle→Recovering` (with the
//!   old id) before it refuses, so its verdict is the one of a row that mutates.
//! * **R102.1 (capture `Ok`)** is D-less ([`decide_start_capture_result_nolock`]);
//!   the failure rows read status under D ([`decide_start_capture_result`]).
//!
//! Rows that reproduce a preserved defect keep it: PD-3 (R102.2-.6: the idle emit
//! is unconditional), PD-6 (R102.3: `Recovering` is overwritten with `Idle`),
//! PD-9 (R101.15: the perf entry is orphaned if the future is dropped), PD-10
//! (R102.x `TERM` is dropped before `accepted`), PD-18 (R101.12).

use crate::admission::{admit, Admission, Occupancy, RefusalCode, Request, SmartAutoObservation};
use crate::decision::{
    ContKind, ContinuationSpec, Decision, InvalidKind, ObsKind, Refusal, RowId, StaleKind, Verdict,
};
use crate::effects::{AxMode, Cond, Effect, EventKind, Placed, Placement as At};
use crate::ids::{Now, RecordingId};
use crate::inputs::{
    CaptureErr, CaptureResult, CaptureResultInput, DeviceSel, ReadyPublishInput, StartContextInput,
    StartInput,
};
use crate::lifecycle::{
    clear_context, decision, emit_status, nothing, placed, set_status, terminal,
};
use crate::replies::{ErrText, Reply, ReplyType};
use crate::state::{DictationCore, LockFreeFacts, StartContextAction};
use crate::status::{ErrCode, Status, TerminalOutcome};

fn with_reply(mut decided: Decision, reply: Reply) -> Decision {
    decided.reply = Some(reply);
    decided
}

/// `REL`: the drop of the gesture and speculative claims at command return.
fn release_claims() -> Placed {
    placed(At::PostT, Effect::ReleaseClaims)
}

// ---------------------------------------------------------------------- 101

/// The pre-D refusals of site 101 (R101.1-.2), decided with no D: `app_disabled`
/// (5483-5486), then Smart Auto with a fixed device (5491-5493). `None` when the
/// request passes both. Both rows are `PT:REL` and a pure refusal.
///
/// * R101.1: `{"type":"app_disabled","state":"idle"}`.
/// * R101.2: `Err("Smart Auto cannot be combined with a fixed microphone.")`.
pub fn decide_start_request_pre(input: &StartInput, _now: Now) -> Option<Decision> {
    let refusal = input.pre_verdict()?;
    let (row, reply) = match &refusal {
        Refusal::Admission(RefusalCode::AppDisabled) => {
            ("R101.1", Reply::typed(ReplyType::AppDisabled, Status::Idle))
        }
        Refusal::Admission(RefusalCode::SmartAutoWithFixedDevice) => {
            ("R101.2", Reply::Err(ErrText::SmartAutoWithFixedDevice))
        }
        Refusal::Admission(other) => {
            unreachable!("pre_verdict returns only the two pre-D refusals, got {other:?}")
        }
        Refusal::NotRecording
        | Refusal::AlreadyProcessing
        | Refusal::AudioRecovering
        | Refusal::ProcessAudio(_) => {
            unreachable!("pre_verdict returns an admission refusal, got {refusal:?}")
        }
    };
    Some(with_reply(
        decision(Verdict::Refused(refusal), None, vec![release_claims()], row),
        reply,
    ))
}

/// The refusal row of an `admit(DictationStart)` refusal.
fn refusal_row(code: &RefusalCode, status: Status) -> Decision {
    let typed = |row: &'static str| -> Decision {
        let (ty, state) = code
            .reply()
            .unwrap_or_else(|| unreachable!("{code:?} has a typed reply"));
        with_reply(
            decision(
                Verdict::Refused(Refusal::Admission(code.clone())),
                None,
                vec![release_claims()],
                row,
            ),
            Reply::typed(ty, state),
        )
    };
    match code {
        RefusalCode::BusyRestarting => typed("R101.3"),
        RefusalCode::BusyMeeting => typed("R101.4"),
        RefusalCode::BusyTranscribingFile => typed("R101.5"),
        RefusalCode::BusyRecordingCorpus => typed("R101.6"),
        RefusalCode::BusyBenchmarking => typed("R101.7"),
        RefusalCode::AlreadyStarting => typed("R101.8"),
        RefusalCode::AlreadyRecording => typed("R101.9"),
        RefusalCode::AudioRecovering => typed("R101.10"),
        RefusalCode::AlreadyProcessing => typed("R101.11"),
        // R101.12 (PD-18 family): a retiring capture of the current generation.
        // The row writes `Idle→Recovering` with the *old* id (no id bump, no
        // terminal, no perf, no `GenStarted`, no `KB`), publishes it, then
        // refuses. `ST` is the status write of `Idle`, the only status this
        // check is reached in.
        RefusalCode::AudioRecoveringCleanupPending => {
            let (ty, state) = code
                .reply()
                .unwrap_or_else(|| unreachable!("{code:?} has a typed reply"));
            debug_assert_eq!(status, Status::Idle, "the cleanup check follows status");
            with_reply(
                decision(
                    Verdict::Applied,
                    Some((status, Status::Recovering)),
                    vec![
                        set_status(Status::Recovering),
                        emit_status(At::In, Status::Recovering),
                        release_claims(),
                    ],
                    "R101.12",
                ),
                Reply::typed(ty, state),
            )
        }
        // R101.14: the resolve's own text, no id burned.
        RefusalCode::SmartAutoFailed(text) => with_reply(
            decision(
                Verdict::Refused(Refusal::Admission(code.clone())),
                None,
                vec![release_claims()],
                "R101.14",
            ),
            Reply::Err(ErrText::SmartAuto(text.clone())),
        ),
        RefusalCode::AppDisabled | RefusalCode::SmartAutoWithFixedDevice => {
            unreachable!("{code:?} is a pre-D refusal (decide_start_request_pre)")
        }
        other => unreachable!("admit(DictationStart) never refuses with {other:?}"),
    }
}

fn start_occupancy(core: &DictationCore, input: &StartInput) -> Occupancy {
    let mut occupancy = input.occupancy.clone();
    // Status is the core's: the shell's lazy read of it is the last blocker group,
    // and no earlier check consults it, so the core view is the same fact.
    occupancy.status = core.status;
    occupancy.app_disabled = input.app_disabled;
    occupancy.fixed_and_smart_auto = input.fixed_and_smart;
    occupancy
}

/// Site 101, `decide_start_request` (R101.1-.15; D and T held, 5505-5605).
///
/// The first call evaluates the blockers (relaunch, meeting, file, corpus,
/// benchmark) and the status step on the occupancy the shell read lazily; then:
///
/// * R101.3-.7 blocker, R101.8-.11 status `Starting` / `Recording` / `Recovering` /
///   `Processing`: `PT:REL`, a typed `{"type","state"}` reply. No mutation.
/// * status `Idle`, no blocker, `cleanup_pending` unread:
///   `NeedObservation(CleanupPending)`, before any mutation.
/// * R101.12 cleanup pending: `In:ST→Recovering` (old id), `In:EM(recovering)`,
///   `PT:REL`; reply `audio_recovering` / `recovering`.
/// * R101.13 `device == SmartAuto`, result absent: `NeedObservation(SmartAutoSelection)`.
/// * R101.14 the resolve failed: `PT:REL`, `Err(text)`, **no id burned**.
/// * R101.15 (`Idle`, admitted): `current_id + 1`; `In:ID+, GA, PCC, TRQ, ST→Starting,
///   AX(a)`; `PDT:SPEC, EV(GenStarted), EM(starting)` (skipped when
///   `speculative_flowing`), `PR, CS`; continuation `K:CaptureStart`. The reply is
///   decided by site 102-104.
pub fn decide_start_request(core: &DictationCore, input: StartInput, now: Now) -> Decision {
    if let Some(pre) = decide_start_request_pre(&input, now) {
        return pre;
    }
    // Blockers and status (relaunch → meeting → file → corpus → benchmark →
    // status): the unread later facts are quiet defaults.
    let mut occupancy = start_occupancy(core, &input);
    occupancy.cleanup_pending = false;
    occupancy.smart_auto = SmartAutoObservation::NotRequested;
    match admit(Request::DictationStart, &occupancy) {
        Admission::RefuseWith(code) => return refusal_row(&code, core.status),
        Admission::Admit(_) => {}
        // `Admit` is returned with the cleanup and Smart Auto steps quiet.
        other @ (Admission::Takeover(_)
        | Admission::WaitThen(_)
        | Admission::NeedObservation(_)) => {
            unreachable!("the blocker steps of a dictation start return {other:?}")
        }
    }
    // Status is `Idle` and no blocker fired. The retiring-capture read is next
    // (5523), and only now (D-S1-2).
    let Some(cleanup_pending) = input.cleanup_pending else {
        return nothing(
            Verdict::NeedObservation(ObsKind::CleanupPending {
                id: core.current_id,
            }),
            "R101.12",
        );
    };
    occupancy.cleanup_pending = cleanup_pending;
    occupancy.smart_auto = match (input.device, &input.smart_auto_result) {
        (DeviceSel::SmartAuto, None) => SmartAutoObservation::Requested(None),
        (DeviceSel::SmartAuto, Some(Ok(_))) => SmartAutoObservation::Requested(Some(Ok(()))),
        (DeviceSel::SmartAuto, Some(Err(text))) => {
            SmartAutoObservation::Requested(Some(Err(text.clone())))
        }
        (DeviceSel::SystemDefault | DeviceSel::Explicit, _) => SmartAutoObservation::NotRequested,
    };
    match admit(Request::DictationStart, &occupancy) {
        Admission::RefuseWith(code) => refusal_row(&code, core.status),
        Admission::NeedObservation(ObsKind::SmartAutoSelection) => nothing(
            Verdict::NeedObservation(ObsKind::SmartAutoSelection),
            "R101.13",
        ),
        Admission::Admit(_) => accepted_start(core, &input),
        other @ (Admission::Takeover(_)
        | Admission::WaitThen(_)
        | Admission::NeedObservation(_)) => {
            unreachable!("a dictation start cannot return {other:?}")
        }
    }
}

/// R101.15: the accepted start.
fn accepted_start(core: &DictationCore, _input: &StartInput) -> Decision {
    // `ID+` allocates `current_id + 1`, a pure function of the core (C0.2).
    let rid = core.next_id();
    let mut decided = decision(
        Verdict::Applied,
        Some((Status::Idle, Status::Starting)),
        vec![
            placed(At::In, Effect::AllocId),
            placed(At::In, Effect::GestureAccepted),
            placed(At::In, Effect::PrepareCaptureClaimed),
            placed(At::In, Effect::TraceRequested),
            set_status(Status::Starting),
            placed(At::In, Effect::AxSnapshot(AxMode::Accept)),
            placed(At::PostDT, Effect::SpeculativeConsume),
            placed(At::PostDT, Effect::EmitEvent(EventKind::GenStarted)),
            placed(
                At::PostDT,
                Effect::EmitStatus {
                    status: Status::Starting,
                    cond: Cond::UnlessSpeculativeFlowing,
                },
            ),
            placed(At::PostDT, Effect::PerfRegister),
            placed(At::PostDT, Effect::CaptureStart(rid)),
        ],
        "R101.15",
    );
    decided.continuation = Some(ContinuationSpec {
        recording_id: rid,
        kind: ContKind::CaptureStart,
    });
    decided
}

// ---------------------------------------------------------------------- 102

/// The `Pre` effects of every capture-start failure row (R102.2-.6): `PA`, `CX-`,
/// run **before D** (5661, 5663). A pure function of the flight's id.
pub fn start_capture_failure_pre_effects(rid: RecordingId) -> Vec<Placed> {
    vec![
        placed(At::Pre, Effect::PerfAbort),
        clear_context(At::Pre, rid),
    ]
}

/// Site 102, the capture-start result with **no D** (R102.1): the supervisor
/// acknowledged the start. No effect; the flight continues
/// (`K:ResolveContext`). An `Err` result reads status under D and is
/// [`decide_start_capture_result`]; given one here, the entry is `Invalid`.
pub fn decide_start_capture_result_nolock(
    _facts: LockFreeFacts,
    input: CaptureResultInput,
    _now: Now,
) -> Decision {
    match input.result {
        CaptureResult::Ok => {
            let mut decided = nothing(Verdict::Continue, "R102.1");
            decided.continuation = Some(ContinuationSpec {
                recording_id: input.token.recording_id,
                kind: ContKind::ResolveContext,
            });
            decided
        }
        CaptureResult::Err(_) => nothing(
            Verdict::Invalid(InvalidKind::Impossible(RowId("R102.1"))),
            "R102.1",
        ),
    }
}

/// Site 102, the capture-start result under D+T (5664-5668, R102.2-.6). `Ok`
/// reads no status and is [`decide_start_capture_result_nolock`].
///
/// Every failure row is `Pre:PA, Pre:CX-` (before D, [`start_capture_failure_pre_effects`]),
/// then `In:ST→Idle` **only for the current id**, `In:EM(idle)` (**unconditional**,
/// PD-3) and `In:TERM(CaptureInitFailure/RuntimeFailure)` (dropped by the ledger
/// when `accepted` never ran, PD-10). The reply is the error's:
/// `AlreadyStarting` → `already_starting` / `idle`, `AudioRecovering` →
/// `audio_recovering` / `idle`, anything else `Err(<the supervisor's text>)`.
///
/// * R102.2 `Starting`, R102.3 `Recovering` (**PD-3, PD-6**: `Idle` is written over
///   `Recovering` although cleanup may be pending), R102.4 `Idle` (a duplicate
///   `Idle→Idle`), R102.5 `Recording` / `Processing`: the current id.
/// * R102.6 any status, `rid != current_id`: no `ST`; stale.
pub fn decide_start_capture_result(
    core: &DictationCore,
    input: CaptureResultInput,
    now: Now,
) -> Decision {
    let error = match &input.result {
        CaptureResult::Ok => {
            return decide_start_capture_result_nolock(core.lock_free_facts(), input, now)
        }
        CaptureResult::Err(error) => error,
    };
    let rid = input.token.recording_id;
    let reply = match error {
        CaptureErr::AlreadyStarting => Reply::typed(ReplyType::AlreadyStarting, Status::Idle),
        CaptureErr::AudioRecovering => Reply::typed(ReplyType::AudioRecovering, Status::Idle),
        CaptureErr::Other => Reply::Err(ErrText::Shell),
    };
    let mut effects = start_capture_failure_pre_effects(rid);
    let (verdict, transition, row) = if rid == core.current_id {
        effects.push(set_status(Status::Idle));
        let row = match core.status {
            Status::Starting => "R102.2",
            Status::Recovering => "R102.3",
            Status::Idle => "R102.4",
            Status::Recording | Status::Processing => "R102.5",
        };
        (Verdict::Applied, Some((core.status, Status::Idle)), row)
    } else {
        (Verdict::Stale(StaleKind::StaleRecording), None, "R102.6")
    };
    effects.push(emit_status(At::In, Status::Idle));
    effects.push(terminal(
        At::In,
        TerminalOutcome::CaptureInitFailure,
        ErrCode::RuntimeFailure,
    ));
    with_reply(decision(verdict, transition, effects, row), reply)
}

// ---------------------------------------------------------------------- 103

/// Row R103.0 / R103.0' (query 402 + the destination choice, 5699-5716): after the
/// smart-destination capture, under a short D read, is the capture still current
/// (`q_capture_current`: id current, not cancelled, status not `Idle`)?
///
/// * R103.0, current: `Continue`; `PDT:SDE` (the `smart-destination-selected` emit,
///   with T held and D released, as today) only when Smart destination is enabled.
/// * R103.0', not current: `Stale`; no effect. The shell then forces the
///   destination to `Unavailable` (enabled) or `Disabled`, so a late result of a
///   failed or cancelled start never selects a profile.
///
/// The appendix's `In:SDE` is `PDT` here because the emit does not run under D
/// today (code wins); the registry has one id for both rows (`R103.0`).
pub fn decide_start_destination(
    core: &DictationCore,
    rid: RecordingId,
    smart_enabled: bool,
    _now: Now,
) -> Decision {
    if core.q_capture_current(rid) {
        let effects = if smart_enabled {
            vec![placed(At::PostDT, Effect::SmartDestinationEmit)]
        } else {
            Vec::new()
        };
        decision(Verdict::Continue, None, effects, "R103.0")
    } else {
        nothing(Verdict::Stale(StaleKind::StaleRecording), "R103.0")
    }
}

/// Site 103, the context install under D+T (5745-5762, R103.1-.6).
/// `q_start_context_action(rid)` picks the row; M4 (the context) is written by
/// `CX+` only (live slot), never read here.
///
/// * R103.1 `Starting`, current: `In:CX+`; the flight continues with
///   `K:ReadyPublish`.
/// * R103.2 `Recovering`, current: `In:CX+` (**PD-8**: the context stays installed
///   after the flush), `PT:PB`, `PT:REL`; reply `audio_recovering` / `recovering`.
/// * R103.3 `Idle`, current: `PT:PB`, `PT:REL`; reply `audio_recovering` / `idle`.
/// * R103.4 `Recording`, R103.5 `Processing` (current), R103.6 (`rid` not current,
///   any status): `PDT:PA`, `PDT:TERM(Superseded/StaleOwner)`, `PT:REL`; reply
///   `audio_recovering` with the **live** status.
pub fn decide_start_context(core: &DictationCore, input: StartContextInput, _now: Now) -> Decision {
    let rid = input.token.recording_id;
    let recovering = |state: Status| Reply::typed(ReplyType::AudioRecovering, state);
    match core.q_start_context_action(rid) {
        StartContextAction::Activate => {
            let mut decided = decision(
                Verdict::Applied,
                None,
                vec![placed(At::In, Effect::SetContext(rid))],
                "R103.1",
            );
            decided.continuation = Some(ContinuationSpec {
                recording_id: rid,
                kind: ContKind::ReadyPublish,
            });
            decided
        }
        StartContextAction::FlushLifecycleTerminal { install_context } => {
            let (row, state) = if install_context {
                ("R103.2", Status::Recovering)
            } else {
                ("R103.3", Status::Idle)
            };
            let mut effects = Vec::new();
            if install_context {
                effects.push(placed(At::In, Effect::SetContext(rid)));
            }
            effects.push(placed(At::PostT, Effect::PerfBegin));
            effects.push(release_claims());
            with_reply(
                decision(Verdict::Applied, None, effects, row),
                recovering(state),
            )
        }
        StartContextAction::Superseded => {
            let row = if rid != core.current_id {
                "R103.6"
            } else if core.status == Status::Recording {
                "R103.4"
            } else {
                "R103.5"
            };
            let verdict = if rid == core.current_id {
                Verdict::Applied
            } else {
                Verdict::Stale(StaleKind::StaleRecording)
            };
            let effects = vec![
                placed(At::PostDT, Effect::PerfAbort),
                terminal(At::PostDT, TerminalOutcome::Superseded, ErrCode::StaleOwner),
                release_claims(),
            ];
            with_reply(
                decision(verdict, None, effects, row),
                recovering(core.status),
            )
        }
    }
}

// ---------------------------------------------------------------------- 104

/// Site 104, the readiness publish when the audio is **not** ready (R104.1): the
/// supervisor phase/owner read before D said no, D is not taken and nothing is
/// written. The publish returns `false`; the flight goes on to its end (R104.7).
/// An `audio_ready` input is [`decide_ready_publish`].
pub fn decide_ready_publish_nolock(
    _facts: LockFreeFacts,
    input: ReadyPublishInput,
    _now: Now,
) -> Decision {
    if input.audio_ready {
        nothing(
            Verdict::Invalid(InvalidKind::Impossible(RowId("R104.1"))),
            "R104.1",
        )
    } else {
        nothing(Verdict::Continue, "R104.1")
    }
}

/// Site 104, the readiness publish under D (5311-5333, R104.1-.6), after the
/// D-less audio-ready read. The guards run in today's order: id current, then
/// status `Starting`, then (read only now, D-S1-2) the installed context.
///
/// * R104.2 `Starting`, current, context installed: `Starting→Recording`;
///   `In:ST→Recording`, `In:TL`, `In:EM(recording)`, `In:SD` (the VAD stream, and
///   the ticker `register` under this same D hold). `Applied`: the publish
///   returns `true`.
/// * R104.3 context missing, R104.4 `rid` not current, R104.5 current but not
///   `Starting` (`Idle`, `Recovering`, `Processing`: a supervisor event landed
///   after the context install), R104.6 `Recording`: `Continue` with no effect;
///   the publish returns `false`.
pub fn decide_ready_publish(core: &DictationCore, input: ReadyPublishInput, now: Now) -> Decision {
    if !input.audio_ready {
        return decide_ready_publish_nolock(
            LockFreeFacts {
                current_id: core.current_id,
                cancelled_through: RecordingId::NONE,
            },
            input,
            now,
        );
    }
    if input.rid != core.current_id {
        return nothing(Verdict::Continue, "R104.4");
    }
    match core.status {
        Status::Recording => nothing(Verdict::Continue, "R104.6"),
        Status::Idle | Status::Recovering | Status::Processing => {
            nothing(Verdict::Continue, "R104.5")
        }
        Status::Starting => match input.context_present {
            None => nothing(
                Verdict::NeedObservation(ObsKind::ContextPresence { id: input.rid }),
                "R104.3",
            ),
            Some(false) => nothing(Verdict::Continue, "R104.3"),
            Some(true) => decision(
                Verdict::Applied,
                Some((Status::Starting, Status::Recording)),
                vec![
                    set_status(Status::Recording),
                    placed(At::In, Effect::TouchLastTranscription),
                    emit_status(At::In, Status::Recording),
                    placed(At::In, Effect::StartSideTasks),
                ],
                "R104.2",
            ),
        },
    }
}

/// Row R104.7, the end of the start flight (5841-5850), after the publish whatever
/// its outcome: `PDT:PM` (the model-preparation attempt, self-gating), `PT:PB` (the
/// performance begin, after T is released), `PT:REL`; reply `recording_starting` /
/// `starting` with the recording id. Pure: it reads nothing.
pub fn decide_start_flight_end(rid: RecordingId, _now: Now) -> Decision {
    with_reply(
        decision(
            Verdict::Applied,
            None,
            vec![
                placed(At::PostDT, Effect::ModelPreparation),
                placed(At::PostT, Effect::PerfBegin),
                release_claims(),
            ],
            "R104.7",
        ),
        Reply::Typed {
            ty: ReplyType::RecordingStarting,
            state: Status::Starting,
            recording_id: Some(rid),
        },
    )
}

#[cfg(test)]
mod tests;
