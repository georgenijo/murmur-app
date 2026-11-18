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
use crate::inputs::{
    GestureObs, Origin, SmartAutoFailure, SmartAutoSel, SpeculativeObs, StartContextInput,
};
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
        Effect::SetContext(id) => format!("CX+({})", id.0),
        Effect::SmartDestinationEmit => "SDE".to_string(),
        Effect::PerfBegin => "PB".to_string(),
        Effect::ModelPreparation => "PM".to_string(),
        Effect::TouchLastTranscription => "TL".to_string(),
        Effect::StartSideTasks => "SD".to_string(),
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
fn r101_14_a_failed_smart_auto_resolve_refuses_with_the_shells_text_and_burns_no_id() {
    let mut input = start();
    input.device = DeviceSel::SmartAuto;
    input.smart_auto_result = Some(Err(SmartAutoFailure::ResolveFailed));
    let decided = d_101(Status::Idle, input);
    assert_eq!(decided.row.0, "R101.14");
    assert_eq!(
        decided.verdict,
        // The resolve's text is the shell's; the core's verdict only says it failed.
        Verdict::Refused(Refusal::Admission(RefusalCode::SmartAutoFailed(
            String::new()
        )))
    );
    assert_eq!(decided.transition, None);
    assert_eq!(tags(&decided), ["PT:REL"]);
    assert_eq!(decided.reply, Some(Reply::Err(ErrText::SmartAuto)));
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
    smart.smart_auto_result = Some(Err(SmartAutoFailure::ResolveFailed));
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
    smart.smart_auto_result = Some(Err(SmartAutoFailure::ResolveFailed));
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

// ------------------------------------------------------------------ R103.x

fn live(cancelled_through: RecordingId) -> LiveObs {
    LiveObs {
        cancelled_through,
        ..LiveObs::default()
    }
}

#[test]
fn r103_0_a_current_capture_continues_and_selects_the_smart_destination_when_enabled() {
    for status in [
        Status::Starting,
        Status::Recording,
        Status::Recovering,
        Status::Processing,
    ] {
        let enabled = decide_start_destination(&core(status, RID), RID, true, NOW);
        assert_valid(&enabled);
        assert_eq!(enabled.row.0, "R103.0", "{status:?}");
        assert_eq!(enabled.verdict, Verdict::Continue, "{status:?}");
        assert_eq!(tags(&enabled), ["In:SDE"], "{status:?}");
        let disabled = decide_start_destination(&core(status, RID), RID, false, NOW);
        assert_eq!(disabled.verdict, Verdict::Continue, "{status:?}");
        assert!(disabled.effects.is_empty(), "{status:?}");
        assert_eq!(disabled.reply, None);
        assert_eq!(disabled.continuation, None);
    }
}

#[test]
fn r103_0_prime_an_idle_stale_or_cancelled_capture_is_stale_with_no_effect() {
    let cases = [
        (
            "idle",
            DictationCore::project(Status::Idle, RID, LiveObs::default()),
        ),
        ("stale id", core(Status::Starting, NEWER)),
        (
            "cancelled",
            DictationCore::project(Status::Starting, RID, live(RID)),
        ),
        (
            "cancelled through a newer id",
            DictationCore::project(Status::Recording, RID, live(NEWER)),
        ),
    ];
    for (name, view) in cases {
        for smart in [true, false] {
            let decided = decide_start_destination(&view, RID, smart, NOW);
            assert_valid(&decided);
            assert_eq!(decided.row.0, "R103.0", "{name}");
            assert_eq!(
                decided.verdict,
                Verdict::Stale(StaleKind::StaleRecording),
                "{name}"
            );
            assert!(decided.effects.is_empty(), "{name}");
            assert_eq!(decided.transition, None);
        }
    }
}

#[test]
fn r103_0_agrees_with_the_capture_current_query_for_every_status() {
    for status in ALL {
        for (current, cancelled) in [
            (RID, RecordingId::NONE),
            (RID, RID),
            (NEWER, RecordingId::NONE),
        ] {
            let view = DictationCore::project(status, current, live(cancelled));
            let decided = decide_start_destination(&view, RID, true, NOW);
            assert_eq!(
                decided.verdict == Verdict::Continue,
                view.q_capture_current(RID),
                "{status:?} {current:?} {cancelled:?}"
            );
        }
    }
}

fn context(id: RecordingId, op_status: OpStatus) -> StartContextInput {
    StartContextInput {
        token: token(ContKind::ResolveContext, id),
        op_status,
    }
}

fn d_103(status: Status, current: RecordingId) -> Decision {
    let decided = decide_start_context(
        &core(status, current),
        context(RID, OpStatus::Outstanding),
        NOW,
    );
    assert_valid(&decided);
    decided
}

fn recovering_reply(state: Status) -> Option<Reply> {
    Some(Reply::typed(ReplyType::AudioRecovering, state))
}

#[test]
fn r103_1_starting_installs_the_context_and_continues_with_ready_publish() {
    let decided = d_103(Status::Starting, RID);
    assert_eq!(decided.row.0, "R103.1");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.transition, None);
    assert_eq!(tags(&decided), ["In:CX+(7)"]);
    assert_eq!(decided.reply, None);
    assert_eq!(
        decided.continuation,
        Some(ContinuationSpec {
            recording_id: RID,
            kind: ContKind::ReadyPublish
        })
    );
}

#[test]
fn r103_2_recovering_installs_the_context_flushes_and_replies_recovering() {
    // PD-8: the context stays installed after the flush.
    let decided = d_103(Status::Recovering, RID);
    assert_eq!(decided.row.0, "R103.2");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(tags(&decided), ["In:CX+(7)", "PT:PB", "PT:REL"]);
    assert_eq!(decided.reply, recovering_reply(Status::Recovering));
    assert_eq!(decided.continuation, None);
}

#[test]
fn r103_3_idle_flushes_without_installing_a_context() {
    let decided = d_103(Status::Idle, RID);
    assert_eq!(decided.row.0, "R103.3");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(tags(&decided), ["PT:PB", "PT:REL"]);
    assert_eq!(decided.reply, recovering_reply(Status::Idle));
    assert_eq!(decided.continuation, None);
}

#[test]
fn r103_4_and_5_a_current_recording_or_processing_start_is_superseded() {
    for (status, row) in [
        (Status::Recording, "R103.4"),
        (Status::Processing, "R103.5"),
    ] {
        let decided = d_103(status, RID);
        assert_eq!(decided.row.0, row);
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(
            tags(&decided),
            ["PDT:PA", "PDT:TERM(Superseded/StaleOwner)", "PT:REL"]
        );
        assert_eq!(decided.reply, recovering_reply(status));
        assert_eq!(decided.continuation, None);
    }
}

#[test]
fn r103_6_a_stale_id_is_superseded_with_the_live_status_in_the_reply() {
    for status in ALL {
        let decided = d_103(status, NEWER);
        assert_eq!(decided.row.0, "R103.6", "{status:?}");
        assert_eq!(decided.verdict, Verdict::Stale(StaleKind::StaleRecording));
        assert_eq!(
            tags(&decided),
            ["PDT:PA", "PDT:TERM(Superseded/StaleOwner)", "PT:REL"]
        );
        assert_eq!(decided.reply, recovering_reply(status), "{status:?}");
        assert_eq!(decided.transition, None);
    }
}

#[test]
fn start_context_never_branches_on_the_op_status() {
    for status in ALL {
        for current in [RID, NEWER] {
            let outstanding = decide_start_context(
                &core(status, current),
                context(RID, OpStatus::Outstanding),
                NOW,
            );
            let unknown =
                decide_start_context(&core(status, current), context(RID, OpStatus::Unknown), NOW);
            assert_eq!(outstanding, unknown, "{status:?} {current:?}");
        }
    }
}

#[test]
fn r103_flush_and_supersede_rows_release_the_claims_last_and_abort_before_the_terminal() {
    for status in ALL {
        for current in [RID, NEWER] {
            let decided = d_103(status, current);
            if decided.row.0 == "R103.1" {
                continue;
            }
            assert_eq!(tags(&decided).last().map(String::as_str), Some("PT:REL"));
            let tags = tags(&decided);
            if let Some(term) = tags.iter().position(|t| t.contains("TERM")) {
                assert_eq!(tags[term - 1], "PDT:PA", "PA runs before the claim");
            }
        }
    }
}

#[test]
fn site_103_covers_every_registered_row() {
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(
        decide_start_destination(&core(Status::Starting, RID), RID, true, NOW)
            .row
            .0,
    );
    seen.insert(
        decide_start_destination(&core(Status::Idle, RID), RID, true, NOW)
            .row
            .0,
    );
    for status in ALL {
        for current in [RID, NEWER] {
            seen.insert(d_103(status, current).row.0);
        }
    }
    let expected: std::collections::BTreeSet<&str> = rows_of_site(SiteId::StartContext).collect();
    assert_eq!(seen, expected);
}

// ------------------------------------------------------------------ R104.x

fn ready(rid: RecordingId, audio_ready: bool, context_present: Option<bool>) -> ReadyPublishInput {
    ReadyPublishInput {
        token: token(ContKind::ReadyPublish, rid),
        op_status: OpStatus::Outstanding,
        rid,
        audio_ready,
        context_present,
        context_ready_at: NOW,
    }
}

fn d_104(status: Status, current: RecordingId, input: ReadyPublishInput) -> Decision {
    let decided = decide_ready_publish(&core(status, current), input, NOW);
    assert_valid(&decided);
    decided
}

fn facts() -> LockFreeFacts {
    LockFreeFacts {
        current_id: RID,
        cancelled_through: RecordingId::NONE,
    }
}

#[test]
fn r104_1_audio_not_ready_is_a_no_lock_continue_for_every_status() {
    let decided = decide_ready_publish_nolock(facts(), ready(RID, false, None), NOW);
    assert_valid(&decided);
    assert_eq!(decided.row.0, "R104.1");
    assert_eq!(decided.verdict, Verdict::Continue);
    assert!(decided.effects.is_empty());
    assert_eq!(decided.reply, None);
    for status in ALL {
        let under_d = d_104(status, RID, ready(RID, false, Some(true)));
        assert_eq!(under_d, decided, "{status:?}: the same row under D");
    }
}

#[test]
fn r104_1_the_no_lock_entry_refuses_a_ready_input() {
    let decided = decide_ready_publish_nolock(facts(), ready(RID, true, Some(true)), NOW);
    assert_eq!(
        decided.verdict,
        Verdict::Invalid(InvalidKind::Impossible(RowId("R104.1")))
    );
    assert!(decided.effects.is_empty());
}

#[test]
fn r104_2_a_starting_recording_with_a_context_publishes_recording() {
    let decided = d_104(Status::Starting, RID, ready(RID, true, Some(true)));
    assert_eq!(decided.row.0, "R104.2");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(
        decided.transition,
        Some((Status::Starting, Status::Recording))
    );
    assert_eq!(
        tags(&decided),
        ["In:ST→Recording", "In:TL", "In:EM(recording)", "In:SD"]
    );
    assert_eq!(decided.reply, None);
    assert_eq!(decided.continuation, None);
}

#[test]
fn r104_3_a_starting_recording_without_a_context_does_not_publish() {
    let decided = d_104(Status::Starting, RID, ready(RID, true, Some(false)));
    assert_eq!(decided.row.0, "R104.3");
    assert_eq!(decided.verdict, Verdict::Continue);
    assert!(decided.effects.is_empty());
    assert_eq!(decided.transition, None);
}

#[test]
fn r104_3_the_context_is_read_only_after_the_id_and_status_guards_passed() {
    let asked = d_104(Status::Starting, RID, ready(RID, true, None));
    assert_eq!(
        asked.verdict,
        Verdict::NeedObservation(ObsKind::ContextPresence { id: RID })
    );
    assert!(asked.effects.is_empty());
    // Any earlier guard answers without the observation (today's short circuit).
    for (status, current) in [
        (Status::Starting, NEWER),
        (Status::Idle, RID),
        (Status::Recovering, RID),
        (Status::Processing, RID),
        (Status::Recording, RID),
    ] {
        let decided = d_104(status, current, ready(RID, true, None));
        assert_eq!(decided.verdict, Verdict::Continue, "{status:?} {current:?}");
    }
}

#[test]
fn r104_4_a_stale_recording_id_does_not_publish_in_any_status() {
    for status in ALL {
        for context_present in [None, Some(true), Some(false)] {
            let decided = d_104(status, NEWER, ready(RID, true, context_present));
            assert_eq!(decided.row.0, "R104.4", "{status:?}");
            assert_eq!(decided.verdict, Verdict::Continue);
            assert!(decided.effects.is_empty());
            assert_eq!(decided.transition, None);
        }
    }
}

#[test]
fn r104_5_a_current_recording_that_is_not_starting_does_not_publish() {
    // #216: the late publish needs `Starting`, whatever the context says.
    for status in [Status::Idle, Status::Recovering, Status::Processing] {
        for context_present in [None, Some(true), Some(false)] {
            let decided = d_104(status, RID, ready(RID, true, context_present));
            assert_eq!(decided.row.0, "R104.5", "{status:?}");
            assert_eq!(decided.verdict, Verdict::Continue);
            assert!(decided.effects.is_empty(), "{status:?}: no ST, EM or SD");
            assert_eq!(decided.transition, None);
        }
    }
}

#[test]
fn r104_6_a_recording_recording_does_not_publish_again() {
    for context_present in [None, Some(true), Some(false)] {
        let decided = d_104(Status::Recording, RID, ready(RID, true, context_present));
        assert_eq!(decided.row.0, "R104.6");
        assert_eq!(decided.verdict, Verdict::Continue);
        assert!(decided.effects.is_empty());
    }
}

#[test]
fn r104_7_the_flight_end_prepares_the_model_begins_the_performance_and_replies_starting() {
    let decided = decide_start_flight_end(RID, NOW);
    assert_valid(&decided);
    assert_eq!(decided.row.0, "R104.7");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(tags(&decided), ["PDT:PM", "PT:PB", "PT:REL"]);
    assert_eq!(
        decided.reply,
        Some(Reply::Typed {
            ty: ReplyType::RecordingStarting,
            state: Status::Starting,
            recording_id: Some(RID),
        })
    );
    assert_eq!(decided.continuation, None);
}

#[test]
fn ready_publish_decisions_never_branch_on_the_op_status() {
    for status in ALL {
        for current in [RID, NEWER] {
            for audio_ready in [true, false] {
                let mut unknown = ready(RID, audio_ready, Some(true));
                unknown.op_status = OpStatus::Unknown;
                assert_eq!(
                    d_104(status, current, ready(RID, audio_ready, Some(true))),
                    d_104(status, current, unknown),
                    "{status:?} {current:?}"
                );
            }
        }
    }
}

#[test]
fn site_104_covers_every_registered_row() {
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(
        decide_ready_publish_nolock(facts(), ready(RID, false, None), NOW)
            .row
            .0,
    );
    for status in ALL {
        for current in [RID, NEWER] {
            for context_present in [Some(true), Some(false)] {
                seen.insert(
                    d_104(status, current, ready(RID, true, context_present))
                        .row
                        .0,
                );
            }
        }
    }
    seen.insert(decide_start_flight_end(RID, NOW).row.0);
    let expected: std::collections::BTreeSet<&str> = rows_of_site(SiteId::ReadyPublish).collect();
    assert_eq!(seen, expected);
}

#[test]
fn site_103_and_104_decisions_are_pure() {
    let view = core(Status::Starting, RID);
    let a = decide_start_context(&view, context(RID, OpStatus::Outstanding), NOW);
    let b = decide_start_context(&view, context(RID, OpStatus::Outstanding), NOW);
    assert_eq!(a, b);
    let a = decide_ready_publish(&view, ready(RID, true, Some(true)), NOW);
    let b = decide_ready_publish(&view, ready(RID, true, Some(true)), NOW);
    assert_eq!(a, b);
    assert_eq!(view, core(Status::Starting, RID));
}
