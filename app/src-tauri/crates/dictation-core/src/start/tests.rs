//! One test per C3.1 / C3.2 start row (R101.1-R101.15, R102.1-R102.6): guard,
//! observations consumed, mutation, ordered placed effects, continuation and
//! reply, for every status that reaches the row (stale, refusal and failure rows
//! included), plus the cross-cutting properties of the family.
//!
//! Expected effect lists are written as the appendix prints them
//! (`In:ID+`, `PDT:CS(8)`), through [`tags`], not through the builders `start.rs`
//! uses, so a reordered or re-placed effect fails here.

use super::*;
use crate::decision::{Continuation, OpStatus, SiteId};
use crate::ids::OperationId;
use crate::inputs::{GestureObs, Origin, SmartAutoSel, SpeculativeObs};
use crate::rows::rows_of_site;
use crate::state::LiveObs;

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);
const NEXT: RecordingId = RecordingId(8);
const NEWER: RecordingId = RecordingId(9);
const ALL: [Status; 5] = Status::ALL;

fn core(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(status, current, LiveObs::default())
}

fn tag(placed: &Placed) -> String {
    let at = placed.at.label();
    let body = match &placed.effect {
        Effect::AllocId => "ID+".to_string(),
        Effect::GestureAccepted => "GA".to_string(),
        Effect::PrepareCaptureClaimed => "PCC".to_string(),
        Effect::TraceRequested => "TRQ".to_string(),
        Effect::SetStatus(to) => format!("ST→{to:?}"),
        Effect::AxSnapshot(mode) => format!("AX({mode:?})"),
        Effect::SpeculativeConsume => "SPEC".to_string(),
        Effect::EmitEvent(kind) => format!("EV({kind:?})"),
        Effect::EmitStatus { status, cond } => match cond {
            Cond::Always => format!("EM({})", status.as_str()),
            Cond::UnlessSpeculativeFlowing => format!("EM({})?", status.as_str()),
            other => panic!("unexpected condition {other:?}"),
        },
        Effect::PerfRegister => "PR".to_string(),
        Effect::PerfAbort => "PA".to_string(),
        Effect::ClearContext(id) => format!("CX-({})", id.0),
        Effect::CaptureStart(id) => format!("CS({})", id.0),
        Effect::Terminal(t) => format!("TERM({:?}/{:?})", t.outcome, t.code),
        Effect::ReleaseClaims => "REL".to_string(),
        other => panic!("not a start-family effect: {other:?}"),
    };
    format!("{at}:{body}")
}

fn tags(decided: &Decision) -> Vec<String> {
    decided.effects.iter().map(tag).collect()
}

fn assert_valid(decided: &Decision) {
    assert_eq!(decided.validate(), Ok(()), "{}", decided.row.0);
}

fn start() -> StartInput {
    StartInput {
        origin: Origin::Hold,
        device: DeviceSel::SystemDefault,
        gesture: GestureObs::default(),
        speculative: SpeculativeObs::default(),
        kept_preview: false,
        app_disabled: false,
        fixed_and_smart: false,
        occupancy: Occupancy::default(),
        cleanup_pending: Some(false),
        smart_auto_result: None,
    }
}

fn d_101(status: Status, input: StartInput) -> Decision {
    let decided = decide_start_request(&core(status, RID), input, NOW);
    assert_valid(&decided);
    decided
}

fn typed(ty: ReplyType, state: Status) -> Option<Reply> {
    Some(Reply::typed(ty, state))
}

/// A refusal row: no mutation, `PT:REL`, a typed reply.
fn expect_refused(decided: &Decision, row: &str, code: RefusalCode, ty: ReplyType, state: Status) {
    assert_eq!(decided.row.0, row);
    assert_eq!(
        decided.verdict,
        Verdict::Refused(Refusal::Admission(code)),
        "{row} verdict"
    );
    assert_eq!(decided.transition, None, "{row} transition");
    assert_eq!(tags(decided), ["PT:REL"], "{row} effects");
    assert_eq!(decided.continuation, None, "{row} continuation");
    assert_eq!(decided.reply, typed(ty, state), "{row} reply");
}

// ------------------------------------------------------------------ R101.1-.2

#[test]
fn r101_1_app_disabled_refuses_before_d_for_every_status() {
    let mut input = start();
    input.app_disabled = true;
    let pre = decide_start_request_pre(&input, NOW).expect("pre-D refusal");
    assert_valid(&pre);
    expect_refused(
        &pre,
        "R101.1",
        RefusalCode::AppDisabled,
        ReplyType::AppDisabled,
        Status::Idle,
    );
    // The status is not read: every status gives the same decision.
    for status in ALL {
        assert_eq!(d_101(status, input.clone()), pre, "{status:?}");
    }
}

#[test]
fn r101_2_fixed_device_with_smart_auto_refuses_before_d_with_its_text() {
    let mut input = start();
    input.fixed_and_smart = true;
    let pre = decide_start_request_pre(&input, NOW).expect("pre-D refusal");
    assert_valid(&pre);
    assert_eq!(pre.row.0, "R101.2");
    assert_eq!(
        pre.verdict,
        Verdict::Refused(Refusal::Admission(RefusalCode::SmartAutoWithFixedDevice))
    );
    assert_eq!(tags(&pre), ["PT:REL"]);
    assert_eq!(pre.transition, None);
    assert_eq!(
        pre.reply,
        Some(Reply::Err(ErrText::SmartAutoWithFixedDevice))
    );
    for status in ALL {
        assert_eq!(d_101(status, input.clone()), pre, "{status:?}");
    }
}

#[test]
fn app_disabled_precedes_fixed_smart_auto_precedes_every_blocker() {
    let mut input = start();
    input.app_disabled = true;
    input.fixed_and_smart = true;
    input.occupancy.meeting_active = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.1");
    input.app_disabled = false;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.2");
    input.fixed_and_smart = false;
    assert_eq!(d_101(Status::Idle, input).row.0, "R101.4");
}

#[test]
fn a_request_with_no_pre_d_refusal_has_no_pre_decision() {
    assert_eq!(decide_start_request_pre(&start(), NOW), None);
}

// ------------------------------------------------------------------ R101.3-.7

#[test]
fn r101_3_to_7_blockers_refuse_for_every_status_with_a_typed_idle_reply() {
    type Set = fn(&mut Occupancy);
    let cases: [(&str, Set, RefusalCode, ReplyType); 5] = [
        (
            "R101.3",
            |o| o.relaunch_committed = true,
            RefusalCode::BusyRestarting,
            ReplyType::BusyRestarting,
        ),
        (
            "R101.4",
            |o| o.meeting_active = true,
            RefusalCode::BusyMeeting,
            ReplyType::BusyMeeting,
        ),
        (
            "R101.5",
            |o| o.file_transcribing = true,
            RefusalCode::BusyTranscribingFile,
            ReplyType::BusyTranscribingFile,
        ),
        (
            "R101.6",
            |o| o.corpus_active = true,
            RefusalCode::BusyRecordingCorpus,
            ReplyType::BusyRecordingCorpus,
        ),
        (
            "R101.7",
            |o| o.benchmark_running = true,
            RefusalCode::BusyBenchmarking,
            ReplyType::BusyBenchmarking,
        ),
    ];
    for (row, set, code, ty) in cases {
        for status in ALL {
            let mut input = start();
            set(&mut input.occupancy);
            // Blockers precede status: the reply names `idle` whatever the status.
            expect_refused(&d_101(status, input), row, code.clone(), ty, Status::Idle);
        }
    }
}

#[test]
fn r101_4_meeting_inference_alone_blocks_like_a_meeting() {
    let mut input = start();
    input.occupancy.meeting_inference_active = true;
    expect_refused(
        &d_101(Status::Idle, input),
        "R101.4",
        RefusalCode::BusyMeeting,
        ReplyType::BusyMeeting,
        Status::Idle,
    );
}

#[test]
fn r101_7_a_shared_backend_change_is_not_a_benchmark() {
    let mut input = start();
    input.occupancy.shared_backend_change = true;
    let decided = d_101(Status::Idle, input);
    assert_eq!(decided.row.0, "R101.15");
}

#[test]
fn blockers_refuse_in_today_order() {
    let mut input = start();
    input.occupancy.benchmark_running = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.7");
    input.occupancy.corpus_active = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.6");
    input.occupancy.file_transcribing = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.5");
    input.occupancy.meeting_active = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.4");
    input.occupancy.relaunch_committed = true;
    assert_eq!(d_101(Status::Idle, input).row.0, "R101.3");
}

#[test]
fn a_blocker_precedes_the_status_step() {
    let mut input = start();
    input.occupancy.meeting_active = true;
    for status in [Status::Starting, Status::Recording, Status::Processing] {
        assert_eq!(d_101(status, input.clone()).row.0, "R101.4", "{status:?}");
    }
}

// ----------------------------------------------------------------- R101.8-.11

#[test]
fn r101_8_to_11_status_refusals_name_the_status() {
    let cases = [
        (
            Status::Starting,
            "R101.8",
            RefusalCode::AlreadyStarting,
            ReplyType::AlreadyStarting,
        ),
        (
            Status::Recording,
            "R101.9",
            RefusalCode::AlreadyRecording,
            ReplyType::AlreadyRecording,
        ),
        (
            Status::Recovering,
            "R101.10",
            RefusalCode::AudioRecovering,
            ReplyType::AudioRecovering,
        ),
        (
            Status::Processing,
            "R101.11",
            RefusalCode::AlreadyProcessing,
            ReplyType::AlreadyProcessing,
        ),
    ];
    for (status, row, code, ty) in cases {
        // The cleanup read is not reached for a non-Idle status, even when it
        // would answer "pending" (R101.10: no cleanup-pending write).
        for cleanup in [None, Some(false), Some(true)] {
            let mut input = start();
            input.cleanup_pending = cleanup;
            expect_refused(&d_101(status, input), row, code.clone(), ty, status);
        }
    }
}

// -------------------------------------------------------------------- R101.12

#[test]
fn the_cleanup_read_is_requested_only_after_the_blockers_and_status_passed() {
    let mut input = start();
    input.cleanup_pending = None;
    let decided = d_101(Status::Idle, input.clone());
    assert_eq!(
        decided.verdict,
        Verdict::NeedObservation(ObsKind::CleanupPending { id: RID })
    );
    assert!(decided.effects.is_empty());
    assert_eq!(decided.transition, None);
    assert_eq!(decided.reply, None);
    // A blocker or a non-Idle status never asks for it.
    input.occupancy.file_transcribing = true;
    assert_eq!(d_101(Status::Idle, input.clone()).row.0, "R101.5");
    input.occupancy.file_transcribing = false;
    for status in [
        Status::Starting,
        Status::Recording,
        Status::Recovering,
        Status::Processing,
    ] {
        assert!(
            !matches!(
                d_101(status, input.clone()).verdict,
                Verdict::NeedObservation(_)
            ),
            "{status:?}"
        );
    }
}

#[test]
fn r101_12_cleanup_pending_writes_recovering_with_the_old_id_then_refuses() {
    let mut input = start();
    input.cleanup_pending = Some(true);
    let decided = d_101(Status::Idle, input);
    assert_eq!(decided.row.0, "R101.12");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.transition, Some((Status::Idle, Status::Recovering)));
    // No `ID+` (the id is unchanged), no terminal, no perf, no `GenStarted`, no KB.
    assert_eq!(
        tags(&decided),
        ["In:ST→Recovering", "In:EM(recovering)", "PT:REL"]
    );
    assert_eq!(decided.continuation, None);
    assert_eq!(
        decided.reply,
        typed(ReplyType::AudioRecovering, Status::Recovering)
    );
}

#[test]
fn r101_12_precedes_the_smart_auto_read() {
    let mut input = start();
    input.device = DeviceSel::SmartAuto;
    input.cleanup_pending = Some(true);
    assert_eq!(d_101(Status::Idle, input).row.0, "R101.12");
}

// ------------------------------------------------------------- R101.13 / .14

#[test]
fn r101_13_smart_auto_without_a_result_asks_for_the_selection_before_mutating() {
    let mut input = start();
    input.device = DeviceSel::SmartAuto;
    let decided = d_101(Status::Idle, input);
    assert_eq!(decided.row.0, "R101.13");
    assert_eq!(
        decided.verdict,
        Verdict::NeedObservation(ObsKind::SmartAutoSelection)
    );
    assert!(decided.effects.is_empty());
    assert_eq!(decided.transition, None);
    assert_eq!(decided.continuation, None);
    assert_eq!(decided.reply, None);
}

#[test]
fn r101_14_a_failed_smart_auto_resolve_refuses_with_its_text_and_burns_no_id() {
    let mut input = start();
    input.device = DeviceSel::SmartAuto;
    input.smart_auto_result = Some(Err("no input device".to_string()));
    let decided = d_101(Status::Idle, input);
    assert_eq!(decided.row.0, "R101.14");
    assert_eq!(
        decided.verdict,
        Verdict::Refused(Refusal::Admission(RefusalCode::SmartAutoFailed(
            "no input device".to_string()
        )))
    );
    assert_eq!(decided.transition, None);
    assert_eq!(tags(&decided), ["PT:REL"]);
    assert_eq!(
        decided.reply,
        Some(Reply::Err(ErrText::SmartAuto(
            "no input device".to_string()
        )))
    );
}

#[test]
fn smart_auto_is_only_read_for_a_smart_auto_request() {
    for device in [DeviceSel::SystemDefault, DeviceSel::Explicit] {
        let mut input = start();
        input.device = device;
        assert_eq!(d_101(Status::Idle, input).row.0, "R101.15");
    }
}

// -------------------------------------------------------------------- R101.15

const R101_15_EFFECTS: [&str; 11] = [
    "In:ID+",
    "In:GA",
    "In:PCC",
    "In:TRQ",
    "In:ST→Starting",
    "In:AX(Accept)",
    "PDT:SPEC",
    "PDT:EV(GenStarted)",
    "PDT:EM(starting)?",
    "PDT:PR",
    "PDT:CS(8)",
];

fn expect_accepted(decided: &Decision) {
    assert_eq!(decided.row.0, "R101.15");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.transition, Some((Status::Idle, Status::Starting)));
    assert_eq!(tags(decided), R101_15_EFFECTS);
    assert_eq!(
        decided.continuation,
        Some(ContinuationSpec {
            recording_id: NEXT,
            kind: ContKind::CaptureStart
        })
    );
    // The reply is decided by sites 102-104.
    assert_eq!(decided.reply, None);
}

#[test]
fn r101_15_accepted_start_allocates_the_next_id_first_and_continues_with_capture_start() {
    expect_accepted(&d_101(Status::Idle, start()));
}

#[test]
fn r101_15_id_is_the_core_next_id_even_when_the_current_id_is_none() {
    let decided = decide_start_request(&core(Status::Idle, RecordingId::NONE), start(), NOW);
    assert_valid(&decided);
    assert_eq!(
        decided.continuation,
        Some(ContinuationSpec {
            recording_id: RecordingId(1),
            kind: ContKind::CaptureStart
        })
    );
    assert_eq!(tags(&decided).last().map(String::as_str), Some("PDT:CS(1)"));
}

#[test]
fn r101_15_smart_auto_with_an_ok_result_proceeds() {
    let mut input = start();
    input.device = DeviceSel::SmartAuto;
    input.smart_auto_result = Some(Ok(SmartAutoSel {
        handle: crate::ids::Handle(3),
    }));
    expect_accepted(&d_101(Status::Idle, input));
}

#[test]
fn r101_15_does_not_branch_on_gesture_speculative_or_kept_preview() {
    // `GA`, `SPEC` and `EM(starting)?` are placed unconditionally; the shell skips
    // the ones whose claim is absent (the cond is the speculative-flowing one).
    for gesture in [
        GestureObs::default(),
        GestureObs {
            has_token: true,
            hold: true,
        },
        GestureObs {
            has_token: true,
            hold: false,
        },
    ] {
        for claimed in [false, true] {
            for kept_preview in [false, true] {
                let mut input = start();
                input.gesture = gesture;
                input.speculative = SpeculativeObs { claimed };
                input.kept_preview = kept_preview;
                expect_accepted(&d_101(Status::Idle, input));
            }
        }
    }
}

#[test]
fn r101_15_has_no_release_effect_the_claims_drop_at_the_command_return() {
    let decided = d_101(Status::Idle, start());
    assert!(!tags(&decided).iter().any(|t| t.ends_with(":REL")));
}

// ------------------------------------------------------------------ properties

#[test]
fn every_refusal_row_releases_the_claims_after_the_transition_guard() {
    let mut rows = Vec::new();
    for status in [
        Status::Starting,
        Status::Recording,
        Status::Recovering,
        Status::Processing,
    ] {
        rows.push(d_101(status, start()));
    }
    let mut app = start();
    app.app_disabled = true;
    rows.push(d_101(Status::Idle, app));
    let mut smart = start();
    smart.device = DeviceSel::SmartAuto;
    smart.smart_auto_result = Some(Err("x".to_string()));
    rows.push(d_101(Status::Idle, smart));
    for decided in rows {
        assert_eq!(decided.effects.last().map(tag).as_deref(), Some("PT:REL"));
        assert_eq!(decided.transition, None, "{}", decided.row.0);
    }
}

#[test]
fn decide_start_request_is_pure() {
    let state = core(Status::Idle, RID);
    let input = start();
    let first = decide_start_request(&state, input.clone(), NOW);
    let second = decide_start_request(&state, input, NOW);
    assert_eq!(first, second);
    assert_eq!(state, core(Status::Idle, RID));
}

#[test]
fn every_start_request_row_is_produced_by_some_input() {
    let mut seen = std::collections::BTreeSet::new();
    let mut note = |decided: Decision| {
        seen.insert(decided.row.0);
    };
    let mut app = start();
    app.app_disabled = true;
    note(d_101(Status::Idle, app));
    let mut fixed = start();
    fixed.fixed_and_smart = true;
    note(d_101(Status::Idle, fixed));
    type Set = fn(&mut Occupancy);
    let blockers: [Set; 5] = [
        |o| o.relaunch_committed = true,
        |o| o.meeting_active = true,
        |o| o.file_transcribing = true,
        |o| o.corpus_active = true,
        |o| o.benchmark_running = true,
    ];
    for set in blockers {
        let mut input = start();
        set(&mut input.occupancy);
        note(d_101(Status::Idle, input));
    }
    for status in ALL {
        note(d_101(status, start()));
    }
    let mut pending = start();
    pending.cleanup_pending = Some(true);
    note(d_101(Status::Idle, pending));
    let mut smart = start();
    smart.device = DeviceSel::SmartAuto;
    note(d_101(Status::Idle, smart.clone()));
    smart.smart_auto_result = Some(Err("e".to_string()));
    note(d_101(Status::Idle, smart));
    let expected: std::collections::BTreeSet<&str> = rows_of_site(SiteId::StartRequest).collect();
    assert_eq!(seen, expected);
}

// ------------------------------------------------------------------ R102.x

fn token(kind: ContKind, id: RecordingId) -> Continuation {
    Continuation {
        recording_id: id,
        op: OperationId(5),
        kind,
    }
}

fn capture(result: CaptureResult, id: RecordingId, op_status: OpStatus) -> CaptureResultInput {
    CaptureResultInput {
        token: token(ContKind::CaptureStart, id),
        op_status,
        result,
    }
}

fn d_102(status: Status, current: RecordingId, input: CaptureResultInput) -> Decision {
    let decided = decide_start_capture_result(&core(status, current), input, NOW);
    assert_valid(&decided);
    decided
}

const FAILURE_TERM: &str = "In:TERM(CaptureInitFailure/RuntimeFailure)";

#[test]
fn r102_1_capture_ok_has_no_effects_and_continues_with_resolve_context() {
    for op_status in [OpStatus::Outstanding, OpStatus::Unknown] {
        let decided = decide_start_capture_result_nolock(
            LockFreeFacts {
                current_id: RID,
                cancelled_through: RecordingId::NONE,
            },
            capture(CaptureResult::Ok, RID, op_status),
            NOW,
        );
        assert_valid(&decided);
        assert_eq!(decided.row.0, "R102.1");
        assert_eq!(decided.verdict, Verdict::Continue);
        assert_eq!(decided.transition, None);
        assert!(decided.effects.is_empty());
        assert_eq!(decided.reply, None);
        assert_eq!(
            decided.continuation,
            Some(ContinuationSpec {
                recording_id: RID,
                kind: ContKind::ResolveContext
            })
        );
    }
}

#[test]
fn r102_1_is_the_same_row_under_d_for_every_status() {
    for status in ALL {
        let decided = d_102(
            status,
            RID,
            capture(CaptureResult::Ok, RID, OpStatus::Unknown),
        );
        assert_eq!(decided.row.0, "R102.1", "{status:?}");
        assert_eq!(decided.verdict, Verdict::Continue);
        assert!(decided.effects.is_empty());
    }
}

#[test]
fn r102_1_given_an_error_the_nolock_entry_is_invalid() {
    let decided = decide_start_capture_result_nolock(
        LockFreeFacts {
            current_id: RID,
            cancelled_through: RecordingId::NONE,
        },
        capture(
            CaptureResult::Err(CaptureErr::Other),
            RID,
            OpStatus::Outstanding,
        ),
        NOW,
    );
    assert!(matches!(decided.verdict, Verdict::Invalid(_)));
    assert!(decided.effects.is_empty());
}

fn failure_reply(error: &CaptureErr) -> Reply {
    match error {
        CaptureErr::AlreadyStarting => Reply::typed(ReplyType::AlreadyStarting, Status::Idle),
        CaptureErr::AudioRecovering => Reply::typed(ReplyType::AudioRecovering, Status::Idle),
        CaptureErr::Other => Reply::Err(ErrText::Shell),
    }
}

const ERRORS: [CaptureErr; 3] = [
    CaptureErr::AlreadyStarting,
    CaptureErr::AudioRecovering,
    CaptureErr::Other,
];

#[test]
fn r102_2_to_5_failure_for_the_current_id_writes_idle_over_every_status() {
    let rows = [
        (Status::Starting, "R102.2"),
        (Status::Recovering, "R102.3"),
        (Status::Idle, "R102.4"),
        (Status::Recording, "R102.5"),
        (Status::Processing, "R102.5"),
    ];
    for (status, row) in rows {
        for error in ERRORS {
            // The token status is an observation: no row branches on it.
            for op_status in [OpStatus::Outstanding, OpStatus::Unknown] {
                let decided = d_102(
                    status,
                    RID,
                    capture(CaptureResult::Err(error.clone()), RID, op_status),
                );
                assert_eq!(decided.row.0, row, "{status:?}");
                assert_eq!(decided.verdict, Verdict::Applied);
                assert_eq!(decided.transition, Some((status, Status::Idle)));
                assert_eq!(
                    tags(&decided),
                    [
                        "Pre:PA",
                        "Pre:CX-(7)",
                        "In:ST→Idle",
                        "In:EM(idle)",
                        FAILURE_TERM
                    ],
                    "{row}"
                );
                assert_eq!(decided.continuation, None);
                assert_eq!(decided.reply, Some(failure_reply(&error)), "{row}");
            }
        }
    }
}

#[test]
fn r102_3_recovering_is_overwritten_with_idle_pd_6() {
    let decided = d_102(
        Status::Recovering,
        RID,
        capture(
            CaptureResult::Err(CaptureErr::Other),
            RID,
            OpStatus::Outstanding,
        ),
    );
    assert_eq!(decided.transition, Some((Status::Recovering, Status::Idle)));
    assert!(tags(&decided).contains(&"In:ST→Idle".to_string()));
}

#[test]
fn r102_4_idle_to_idle_still_writes_the_status_and_the_duplicate_idle_emit() {
    let decided = d_102(
        Status::Idle,
        RID,
        capture(
            CaptureResult::Err(CaptureErr::AlreadyStarting),
            RID,
            OpStatus::Outstanding,
        ),
    );
    assert_eq!(decided.transition, Some((Status::Idle, Status::Idle)));
    assert!(tags(&decided).contains(&"In:ST→Idle".to_string()));
    assert!(tags(&decided).contains(&"In:EM(idle)".to_string()));
}

#[test]
fn r102_6_a_stale_failure_skips_the_status_write_but_not_the_idle_emit_pd_3() {
    for status in ALL {
        for current in [NEXT, NEWER, RecordingId::NONE] {
            for error in ERRORS {
                let decided = d_102(
                    status,
                    current,
                    capture(
                        CaptureResult::Err(error.clone()),
                        RID,
                        OpStatus::Outstanding,
                    ),
                );
                assert_eq!(decided.row.0, "R102.6", "{status:?}");
                assert_eq!(decided.verdict, Verdict::Stale(StaleKind::StaleRecording));
                assert_eq!(decided.transition, None);
                assert_eq!(
                    tags(&decided),
                    ["Pre:PA", "Pre:CX-(7)", "In:EM(idle)", FAILURE_TERM]
                );
                assert_eq!(decided.reply, Some(failure_reply(&error)));
            }
        }
    }
}

#[test]
fn the_failure_pre_effects_are_a_pure_function_of_the_flight_id() {
    let pre = start_capture_failure_pre_effects(RID);
    assert_eq!(
        pre.iter().map(tag).collect::<Vec<_>>(),
        ["Pre:PA", "Pre:CX-(7)"]
    );
    // They are the head of every failure row's list.
    let decided = d_102(
        Status::Starting,
        RID,
        capture(
            CaptureResult::Err(CaptureErr::Other),
            RID,
            OpStatus::Outstanding,
        ),
    );
    assert_eq!(&decided.effects[..2], &pre[..]);
}

#[test]
fn every_capture_result_row_is_produced_by_some_input() {
    let mut seen = std::collections::BTreeSet::new();
    let err = |id| {
        capture(
            CaptureResult::Err(CaptureErr::Other),
            id,
            OpStatus::Outstanding,
        )
    };
    seen.insert(
        d_102(
            Status::Idle,
            RID,
            capture(CaptureResult::Ok, RID, OpStatus::Unknown),
        )
        .row
        .0,
    );
    for status in ALL {
        seen.insert(d_102(status, RID, err(RID)).row.0);
    }
    seen.insert(d_102(Status::Starting, NEWER, err(RID)).row.0);
    let expected: std::collections::BTreeSet<&str> =
        rows_of_site(SiteId::StartCaptureResult).collect();
    assert_eq!(seen, expected);
}

#[test]
fn start_decisions_never_read_the_live_observations() {
    // M3 and M4 are live-only: the same decision for any `LiveObs`.
    let plain = decide_start_request(&core(Status::Idle, RID), start(), NOW);
    let live = LiveObs {
        cancelled_through: RecordingId(100),
        ..LiveObs::default()
    };
    let observed = decide_start_request(
        &DictationCore::project(Status::Idle, RID, live),
        start(),
        NOW,
    );
    assert_eq!(plain, observed);
}
