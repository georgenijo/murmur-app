//! One test per C3.5 row (R201.1 - R204b.6): guard, observations consumed,
//! mutation, ordered placed effects, continuation and reply, for every status
//! that reaches the row (stale and refusal rows included), plus the
//! cross-cutting properties of the family.
//!
//! Expected effect lists are written as the appendix prints them
//! (`Pre:TERM(...)`, `In:ST→Idle`), through [`tags`], not through the builders
//! `stop.rs` uses, so a reordered or re-placed effect fails here.

use super::*;
use crate::decision::{Continuation, OpStatus, SiteId};
use crate::effects::TakeMode;
use crate::ids::OperationId;
use crate::inputs::{InterruptedMeta, PostKind};
use crate::rows::rows_of_site;
use crate::state::LiveObs;
use crate::status::StopTrigger;

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);
const NEWER: RecordingId = RecordingId(8);
const ALL: [Status; 5] = Status::ALL;

fn core(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(status, current, LiveObs::default())
}

fn facts(current: RecordingId) -> LockFreeFacts {
    LockFreeFacts {
        current_id: current,
        cancelled_through: RecordingId::NONE,
    }
}

fn tag(placed: &Placed) -> String {
    let at = placed.at.label();
    let body = match &placed.effect {
        Effect::SetStatus(to) => format!("ST→{to:?}"),
        Effect::EmitStatus { status, .. } => format!("EM({})", status.as_str()),
        Effect::EmitEvent(kind) => format!("EV({kind:?})"),
        Effect::Terminal(t) => format!("TERM({:?}/{:?})", t.outcome, t.code),
        Effect::PerfComplete(pc) => format!("PC({:?},{:?},{:?})", pc.outcome, pc.stage, pc.cause),
        Effect::PipelineGuardFinish(o) => format!("PGF({o:?})"),
        Effect::PipelineEnter(_) => "PE(Vad)".into(),
        Effect::PipelineSpawn(id) => format!("PS({})", id.0),
        Effect::ClearContext(id) => format!("CX-({})", id.0),
        Effect::KeyboardProcessing(on) => format!("KB({on})"),
        Effect::AxSnapshot(mode) => format!("AX({mode:?})"),
        Effect::PreviewHideClaim => "PH".into(),
        Effect::TakeStopStamps => "TS".into(),
        Effect::CaptureStop(id) => format!("CP({})", id.0),
        Effect::TakeInterrupted { id, mode } => format!("TI({mode:?},{})", id.0),
        Effect::CaptureCancel { id, reason } => format!("CC({},{reason:?})", id.0),
        other => panic!("not a stop-family effect: {other:?}"),
    };
    format!("{at}:{body}")
}

fn tags(decided: &Decision) -> Vec<String> {
    decided.effects.iter().map(tag).collect()
}

fn token(kind: ContKind) -> Continuation {
    Continuation {
        recording_id: RID,
        op: OperationId(5),
        kind,
    }
}

fn typed(ty: ReplyType, state: Status) -> Option<Reply> {
    Some(Reply::typed(ty, state))
}

fn stop(expected: Option<RecordingId>, observed: RecordingId) -> StopInput {
    StopInput {
        expected,
        trigger: StopTrigger::User,
        observed_current_id: observed,
        interrupted: None,
        entered: NOW,
        taken_earlier: false,
    }
}

fn meta() -> InterruptedMeta {
    InterruptedMeta {
        samples: 9_000,
        reason_code: ErrCode::DeviceChanged,
        duration_ms: 500,
    }
}

fn assert_valid(decided: &Decision) {
    assert_eq!(decided.validate(), Ok(()), "{:?}", decided.row);
}

// ---------------------------------------------------------------------- 201

#[test]
fn r201_1_stale_expected_asks_for_the_take_then_supersedes_whatever_the_status() {
    // D-S1-3 / PD-15: the destructive take also happens for a stale owner, and
    // it is the observation's answer, so the effect list carries the terminal
    // only. No status is read: every status gives the same decision.
    let input = stop(Some(NEWER), RID);
    for status in ALL {
        let first = decide_stop_request(&core(status, RID), input, NOW);
        assert_eq!(
            first.verdict,
            Verdict::NeedObservation(ObsKind::TakeInterrupted { id: NEWER }),
            "{status:?}"
        );
        assert_eq!(first.row, RowId("R201.1"));
        assert!(first.effects.is_empty() && first.reply.is_none());
        // PD-25 fixed: the terminal is left to an earlier taker only when this stop's take
        // found nothing *and* an earlier take had consumed the capture. A take that found
        // it, or found nothing nobody had taken, still claims.
        for (answer, taken_earlier, claims) in [
            (None, false, true),
            (None, true, false),
            (Some(meta()), false, true),
            (Some(meta()), true, true),
        ] {
            let second = decide_stop_request(
                &core(status, RID),
                StopInput {
                    interrupted: Some(answer),
                    taken_earlier,
                    ..input
                },
                NOW,
            );
            assert_eq!(second.verdict, Verdict::Stale(StaleKind::StaleRecording));
            assert_eq!(second.row, RowId("R201.1"));
            let expected: &[&str] = if claims {
                &["Pre:TERM(Superseded/StaleOwner)"]
            } else {
                &[]
            };
            assert_eq!(
                tags(&second),
                expected,
                "{status:?} answer={answer:?} taken_earlier={taken_earlier}"
            );
            assert_eq!(second.transition, None);
            assert_eq!(second.continuation, None);
            assert_eq!(
                second.reply,
                typed(ReplyType::RecordingSuperseded, Status::Idle)
            );
            assert_valid(&second);
        }
    }
    // The pre-D entry is the same decision.
    assert_eq!(
        decide_stop_request_stale(input, NOW).map(|d| d.row),
        Some(RowId("R201.1"))
    );
}

#[test]
fn a_matching_or_absent_expectation_is_not_stale() {
    assert!(decide_stop_request_stale(stop(None, RID), NOW).is_none());
    assert!(decide_stop_request_stale(stop(Some(RID), RID), NOW).is_none());
}

#[test]
fn r201_2_processing_refuses_without_effects() {
    let decided = decide_stop_request(&core(Status::Processing, RID), stop(None, RID), NOW);
    assert_eq!(
        decided.verdict,
        Verdict::Refused(Refusal::AlreadyProcessing)
    );
    assert_eq!(decided.row, RowId("R201.2"));
    assert!(decided.effects.is_empty() && decided.transition.is_none());
    assert_eq!(
        decided.reply,
        typed(ReplyType::AlreadyProcessing, Status::Processing)
    );
}

#[test]
fn r201_3_idle_refuses_without_effects() {
    let decided = decide_stop_request(&core(Status::Idle, RID), stop(Some(RID), RID), NOW);
    assert_eq!(decided.verdict, Verdict::Refused(Refusal::NotRecording));
    assert_eq!(decided.row, RowId("R201.3"));
    assert!(decided.effects.is_empty() && decided.transition.is_none());
    assert_eq!(decided.reply, typed(ReplyType::NotRecording, Status::Idle));
}

#[test]
fn r201_4_starting_cancels_the_exact_owner_after_t_is_released() {
    let decided = decide_stop_request(&core(Status::Starting, RID), stop(None, RID), NOW);
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.row, RowId("R201.4"));
    assert_eq!(
        decided.transition, None,
        "the supervisor's Recovering writes it later"
    );
    assert_eq!(tags(&decided), ["PDT:TS", "PT:CC(7,User)"]);
    assert_eq!(
        decided.continuation,
        Some(ContinuationSpec {
            recording_id: RID,
            kind: ContKind::CaptureCancel
        })
    );
    assert_eq!(
        decided.reply,
        typed(ReplyType::RecordingCancelled, Status::Recovering)
    );
    assert_valid(&decided);
}

#[test]
fn r201_5_recovering_asks_for_the_take_before_any_mutation() {
    let decided = decide_stop_request(&core(Status::Recovering, RID), stop(None, RID), NOW);
    assert_eq!(
        decided.verdict,
        Verdict::NeedObservation(ObsKind::TakeInterrupted { id: RID })
    );
    assert_eq!(decided.row, RowId("R201.5"));
    assert!(decided.effects.is_empty() && decided.transition.is_none());
    assert!(decided.continuation.is_none() && decided.reply.is_none());
}

#[test]
fn r201_6_recovering_with_nothing_preserved_refuses() {
    let input = StopInput {
        interrupted: Some(None),
        ..stop(None, RID)
    };
    let decided = decide_stop_request(&core(Status::Recovering, RID), input, NOW);
    assert_eq!(decided.verdict, Verdict::Refused(Refusal::AudioRecovering));
    assert_eq!(decided.row, RowId("R201.6"));
    assert!(decided.effects.is_empty() && decided.transition.is_none());
    assert_eq!(
        decided.reply,
        typed(ReplyType::AudioRecovering, Status::Recovering)
    );
}

#[test]
fn r201_7_recovering_with_a_taken_capture_moves_to_processing() {
    let input = StopInput {
        interrupted: Some(Some(meta())),
        ..stop(None, RID)
    };
    let decided = decide_stop_request(&core(Status::Recovering, RID), input, NOW);
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.row, RowId("R201.7"));
    assert_eq!(
        decided.transition,
        Some((Status::Recovering, Status::Processing))
    );
    assert_eq!(
        tags(&decided),
        ["In:PH", "In:ST→Processing", "In:AX(Stop)", "PDT:TS"]
    );
    assert_eq!(
        decided.continuation,
        Some(ContinuationSpec {
            recording_id: RID,
            kind: ContKind::StopContext
        })
    );
    assert_eq!(decided.reply, None);
    assert_valid(&decided);
}

#[test]
fn r201_8_recording_claims_the_preview_before_the_status_write() {
    let decided = decide_stop_request(&core(Status::Recording, RID), stop(Some(RID), RID), NOW);
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.row, RowId("R201.8"));
    assert_eq!(
        decided.transition,
        Some((Status::Recording, Status::Processing))
    );
    assert_eq!(
        tags(&decided),
        ["In:PH", "In:ST→Processing", "In:AX(Stop)", "PDT:TS"]
    );
    assert_eq!(
        decided.continuation.map(|c| c.kind),
        Some(ContKind::StopContext)
    );
    assert_eq!(decided.reply, None);
    assert_valid(&decided);
}

#[test]
fn recording_row_ignores_an_unneeded_observation_answer() {
    // `interrupted` is only read in Recovering.
    let input = StopInput {
        interrupted: Some(Some(meta())),
        ..stop(None, RID)
    };
    let a = decide_stop_request(&core(Status::Recording, RID), input, NOW);
    let b = decide_stop_request(&core(Status::Recording, RID), stop(None, RID), NOW);
    assert_eq!(a, b);
}

// ---------------------------------------------------------------------- 202

fn ctx_input(context_present: bool, op_status: OpStatus) -> StopContextInput {
    StopContextInput {
        token: token(ContKind::StopContext),
        op_status,
        rid: RID,
        context_present,
        interrupted_error: None,
    }
}

#[test]
fn r202_1_and_r202_2_context_present_is_d_less_and_reads_no_status() {
    // R202.2 is the schedule where a supervisor `Idle` landed first: with no D
    // the decision cannot tell, so it is the same decision as R202.1 (PD-7).
    for current in [RID, NEWER, RecordingId::NONE] {
        let decided =
            decide_stop_context_nolock(facts(current), ctx_input(true, OpStatus::Outstanding), NOW);
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(decided.row, RowId("R202.1"));
        assert_eq!(
            tags(&decided),
            ["PDT:KB(true)", "PDT:EM(processing)", "PT:CP(7)"]
        );
        assert_eq!(decided.transition, None);
        assert_eq!(
            decided.continuation,
            Some(ContinuationSpec {
                recording_id: RID,
                kind: ContKind::CaptureStop
            })
        );
        assert_eq!(decided.reply, None);
        assert_valid(&decided);
    }
}

fn missing_context(row: &str, status: Status, decided: &Decision, expect_st: bool) {
    assert_eq!(decided.verdict, Verdict::Applied, "{status:?}");
    assert_eq!(decided.row.0, row);
    let mut expected = vec!["Pre:TERM(PipelineFailure/MissingContext)".to_string()];
    if expect_st {
        expected.push("In:ST→Idle".into());
    }
    expected.extend([
        "In:KB(false)".to_string(),
        "In:EM(idle)".into(),
        "In:PC(Failed,CaptureFinalization,Some(InternalEarlyExit))".into(),
    ]);
    assert_eq!(tags(decided), expected, "{status:?}");
    assert_eq!(
        decided.transition,
        expect_st.then_some((status, Status::Idle))
    );
    assert_eq!(
        decided.reply,
        Some(Reply::Err(ErrText::MissingContext(RID)))
    );
    assert_eq!(decided.continuation, None);
    assert_valid(decided);
}

#[test]
fn the_context_less_pre_effects_are_the_decisions_leading_pre_effects() {
    // The shell runs `Pre:TERM` before D from this list and drops it from the
    // decision with `after_pre`: they must be the same effects.
    let pre = stop_context_pre_effects();
    let decided = decide_stop_context(
        &core(Status::Processing, RID),
        ctx_input(false, OpStatus::Outstanding),
        NOW,
    );
    assert_eq!(pre.as_slice(), &decided.effects[..pre.len()]);
    assert_eq!(
        decided.clone().after_pre().effects.len(),
        decided.effects.len() - pre.len()
    );
}

#[test]
fn r202_3_processing_without_context_writes_idle_and_fails() {
    let decided = decide_stop_context(
        &core(Status::Processing, RID),
        ctx_input(false, OpStatus::Outstanding),
        NOW,
    );
    missing_context("R202.3", Status::Processing, &decided, true);
}

#[test]
fn r202_4_idle_without_context_writes_idle_over_idle() {
    // PD-3 / PD-7: the supervisor's `Idle` landed first; `ST→Idle` is still
    // issued (the executor's self-loop skips only the trace), and `KB(false)`
    // and `EM(idle)` are unconditional.
    let decided = decide_stop_context(
        &core(Status::Idle, RID),
        ctx_input(false, OpStatus::Outstanding),
        NOW,
    );
    missing_context("R202.4", Status::Idle, &decided, true);
}

#[test]
fn r202_5_other_statuses_take_the_missing_context_path_or_the_present_one() {
    for status in [Status::Starting, Status::Recording, Status::Recovering] {
        let absent = decide_stop_context(
            &core(status, RID),
            ctx_input(false, OpStatus::Outstanding),
            NOW,
        );
        missing_context("R202.5", status, &absent, true);
        let present = decide_stop_context(
            &core(status, RID),
            ctx_input(true, OpStatus::Outstanding),
            NOW,
        );
        assert_eq!(present.row, RowId("R202.5"));
        assert_eq!(
            tags(&present),
            ["PDT:KB(true)", "PDT:EM(processing)", "PT:CP(7)"]
        );
        assert_eq!(
            present.continuation.map(|c| c.kind),
            Some(ContKind::CaptureStop)
        );
    }
}

#[test]
fn missing_context_for_a_stale_id_still_emits_but_never_writes_the_status() {
    // The write is guarded by `recording_id == rid` (6006); KB and EM are not.
    let decided = decide_stop_context(
        &core(Status::Recording, NEWER),
        ctx_input(false, OpStatus::Outstanding),
        NOW,
    );
    assert_eq!(decided.transition, None);
    assert_eq!(
        tags(&decided),
        [
            "Pre:TERM(PipelineFailure/MissingContext)",
            "In:KB(false)",
            "In:EM(idle)",
            "In:PC(Failed,CaptureFinalization,Some(InternalEarlyExit))"
        ]
    );
}

// ---------------------------------------------------------------------- 203

fn capture(result: StopCaptureResult, op_status: OpStatus) -> StopCaptureResultInput {
    StopCaptureResultInput {
        token: token(ContKind::CaptureStop),
        op_status,
        rid: RID,
        result,
    }
}

const ERR: StopCaptureResult = StopCaptureResult::Err;

fn samples(n: usize) -> StopCaptureResult {
    StopCaptureResult::Samples {
        n,
        interrupted: None,
    }
}

fn interrupted(n: usize) -> StopCaptureResult {
    StopCaptureResult::Samples {
        n,
        interrupted: Some(ErrCode::SystemSleep),
    }
}

fn d_203(status: Status, current: RecordingId, result: StopCaptureResult) -> Decision {
    let decided = decide_stop_capture_result(
        &core(status, current),
        capture(result, OpStatus::Outstanding),
        NOW,
    );
    assert_valid(&decided);
    decided
}

fn expect_203(
    decided: &Decision,
    row: &str,
    terminal: &str,
    in_d: bool,
    tail: &str,
    status: Status,
) {
    assert_eq!(decided.row.0, row);
    let mut expected = vec![
        format!("Pre:TERM({terminal})"),
        format!("Pre:CX-({})", RID.0),
    ];
    if in_d {
        expected.extend([
            "In:ST→Idle".to_string(),
            "In:KB(false)".into(),
            "In:EM(idle)".into(),
        ]);
    }
    expected.push(format!("PDT:{tail}"));
    assert_eq!(tags(decided), expected, "{row}");
    assert_eq!(
        decided.transition,
        in_d.then_some((status, Status::Idle)),
        "{row}"
    );
    assert_eq!(
        decided.verdict,
        if in_d {
            Verdict::Applied
        } else {
            Verdict::Stale(StaleKind::StaleRecording)
        },
        "{row}"
    );
}

const FAIL_TAIL: &str = "PC(Failed,CaptureFinalization,Some(AudioCaptureFailed))";
const NO_SPEECH_TAIL: &str = "PGF(NoSpeech)";

#[test]
fn r203_1_to_r203_2r_stop_failure_with_the_current_id() {
    let term = "StopFailure/StopFinalizationFailed";
    let cases = [
        (Status::Processing, "R203.1"),
        (Status::Idle, "R203.2"),
        (Status::Recovering, "R203.2r"),
        // X: a Starting or Recording status with the guard's own id cannot occur;
        // the code writes Idle regardless.
        (Status::Starting, "R203.1"),
        (Status::Recording, "R203.1"),
    ];
    for (status, row) in cases {
        let decided = d_203(status, RID, ERR);
        expect_203(&decided, row, term, true, FAIL_TAIL, status);
        assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
        assert_eq!(decided.continuation, None);
    }
}

#[test]
fn r203_3a_to_3e_stop_failure_for_a_superseded_recording() {
    let term = "StopFailure/StopFinalizationFailed";
    for (status, row) in [
        (Status::Idle, "R203.3a"),
        (Status::Starting, "R203.3b"),
        (Status::Recording, "R203.3c"),
        (Status::Recovering, "R203.3d"),
        (Status::Processing, "R203.3e"),
    ] {
        let decided = d_203(status, NEWER, ERR);
        expect_203(&decided, row, term, false, FAIL_TAIL, status);
        assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
    }
}

fn empty_reply() -> Option<Reply> {
    Some(Reply::Transcription {
        text: TranscriptText::Empty,
        state: Status::Idle,
    })
}

#[test]
fn r203_4_to_r203_5r_empty_capture_with_the_current_id() {
    for (status, row) in [
        (Status::Processing, "R203.4"),
        (Status::Idle, "R203.5"),
        (Status::Recovering, "R203.5r"),
    ] {
        let decided = d_203(status, RID, samples(0));
        expect_203(
            &decided,
            row,
            "NoSpeech/EmptyAudio",
            true,
            NO_SPEECH_TAIL,
            status,
        );
        assert_eq!(decided.reply, empty_reply());
        // An interrupted capture replaces the terminal, nothing else.
        let interrupted = d_203(status, RID, interrupted(0));
        expect_203(
            &interrupted,
            row,
            "RuntimeInterruption/SystemSleep",
            true,
            NO_SPEECH_TAIL,
            status,
        );
    }
}

#[test]
fn r203_6_to_r203_7r_short_capture_with_the_current_id() {
    for (status, row) in [
        (Status::Processing, "R203.6"),
        (Status::Idle, "R203.7"),
        (Status::Recovering, "R203.7r"),
    ] {
        for n in [1, MIN_RECORDING_SAMPLES - 1] {
            let decided = d_203(status, RID, samples(n));
            expect_203(
                &decided,
                row,
                "TooShort/BelowMinimumDuration",
                true,
                NO_SPEECH_TAIL,
                status,
            );
            assert_eq!(decided.reply, empty_reply());
        }
        let interrupted = d_203(status, RID, interrupted(100));
        expect_203(
            &interrupted,
            row,
            "RuntimeInterruption/SystemSleep",
            true,
            NO_SPEECH_TAIL,
            status,
        );
    }
}

#[test]
fn r203_8a_to_8e_empty_or_short_capture_for_a_superseded_recording() {
    for (status, row) in [
        (Status::Idle, "R203.8a"),
        (Status::Starting, "R203.8b"),
        (Status::Recording, "R203.8c"),
        (Status::Recovering, "R203.8d"),
        (Status::Processing, "R203.8e"),
    ] {
        let empty = d_203(status, NEWER, samples(0));
        expect_203(
            &empty,
            row,
            "NoSpeech/EmptyAudio",
            false,
            NO_SPEECH_TAIL,
            status,
        );
        assert_eq!(empty.reply, empty_reply());
        let short = d_203(status, NEWER, samples(10));
        expect_203(
            &short,
            row,
            "TooShort/BelowMinimumDuration",
            false,
            NO_SPEECH_TAIL,
            status,
        );
    }
}

#[test]
fn r203_9_and_r203_10_a_long_capture_starts_the_pipeline_without_d() {
    // Neither status nor id is read (R203.10, PD-7): any facts give one decision.
    for current in [RID, NEWER, RecordingId::NONE] {
        for result in [samples(MIN_RECORDING_SAMPLES), interrupted(1_000_000)] {
            let decided = decide_stop_capture_result_nolock(
                facts(current),
                capture(result, OpStatus::Outstanding),
                NOW,
            );
            assert_eq!(decided.verdict, Verdict::Applied);
            assert_eq!(decided.row, RowId("R203.9"));
            assert_eq!(tags(&decided), ["PDT:PE(Vad)", "PDT:PS(7)"]);
            assert_eq!(decided.transition, None);
            assert_eq!(
                decided.continuation,
                Some(ContinuationSpec {
                    recording_id: RID,
                    kind: ContKind::Pipeline
                })
            );
            assert_eq!(decided.reply, None);
            assert_valid(&decided);
        }
    }
    // The D entry delegates a long capture to the D-less one.
    for status in ALL {
        let decided = d_203(status, RID, samples(MIN_RECORDING_SAMPLES));
        assert_eq!(decided.row, RowId("R203.9"));
        assert_eq!(tags(&decided), ["PDT:PE(Vad)", "PDT:PS(7)"]);
    }
    // The D-less entry is not for a failed or short result.
    for result in [ERR, samples(0), samples(MIN_RECORDING_SAMPLES - 1)] {
        let decided = decide_stop_capture_result_nolock(
            facts(RID),
            capture(result, OpStatus::Outstanding),
            NOW,
        );
        assert!(matches!(decided.verdict, Verdict::Invalid(_)));
        assert!(decided.effects.is_empty());
    }
}

#[test]
fn pre_effects_are_the_decisions_leading_pre_effects() {
    for result in [
        ERR,
        samples(0),
        samples(10),
        interrupted(10),
        samples(MIN_RECORDING_SAMPLES),
    ] {
        let input = capture(result, OpStatus::Outstanding);
        let pre = stop_capture_pre_effects(&input);
        let decided = decide_stop_capture_result(&core(Status::Processing, RID), input, NOW);
        let leading: Vec<Placed> = decided
            .effects
            .iter()
            .take_while(|p| p.at == At::Pre)
            .cloned()
            .collect();
        assert_eq!(pre, leading, "{result:?}");
        assert_eq!(
            decided.clone().after_pre().effects.len() + pre.len(),
            decided.effects.len()
        );
    }
}

// --------------------------------------------------------------------- 204a

fn guard(rid: RecordingId, origin: GuardOrigin, closure_emit: bool) -> GuardFinishInput {
    GuardFinishInput {
        rid,
        origin,
        closure_emit,
    }
}

#[test]
fn r204a_1_to_4_the_guard_writes_idle_on_an_id_match_whatever_the_status() {
    for origin in [
        GuardOrigin::StopCapture,
        GuardOrigin::PipelineExit,
        GuardOrigin::LegacyParse,
    ] {
        for (status, row) in [
            (Status::Processing, "R204a.1"),
            (Status::Idle, "R204a.2"),
            (Status::Recovering, "R204a.3"),
            // X: the code would still write Idle.
            (Status::Starting, "R204a.4"),
            (Status::Recording, "R204a.4"),
        ] {
            let with_emit = decide_guard_finish(&core(status, RID), guard(RID, origin, true), NOW);
            assert_eq!(with_emit.verdict, Verdict::Applied);
            assert_eq!(with_emit.row, RowId(row));
            assert_eq!(
                tags(&with_emit),
                ["Pre:CX-(7)", "In:ST→Idle", "In:KB(false)", "In:EM(idle)"]
            );
            assert_eq!(with_emit.transition, Some((status, Status::Idle)));
            assert_valid(&with_emit);
            // The pipeline guard passes no closure: no `EM`.
            let without = decide_guard_finish(&core(status, RID), guard(RID, origin, false), NOW);
            assert_eq!(tags(&without), ["Pre:CX-(7)", "In:ST→Idle", "In:KB(false)"]);
            assert_eq!(without.row, RowId(row));
        }
    }
}

#[test]
fn r204a_5a_to_5e_a_stale_guard_clears_the_context_only() {
    for (status, row) in [
        (Status::Idle, "R204a.5a"),
        (Status::Starting, "R204a.5b"),
        (Status::Recording, "R204a.5c"),
        (Status::Recovering, "R204a.5d"),
        (Status::Processing, "R204a.5e"),
    ] {
        for closure_emit in [true, false] {
            let decided = decide_guard_finish(
                &core(status, NEWER),
                guard(RID, GuardOrigin::PipelineExit, closure_emit),
                NOW,
            );
            assert_eq!(decided.verdict, Verdict::Stale(StaleKind::StaleRecording));
            assert_eq!(decided.row, RowId(row));
            assert_eq!(tags(&decided), ["Pre:CX-(7)"]);
            assert_eq!(decided.transition, None);
            assert_valid(&decided);
        }
    }
}

#[test]
fn r204a_6_a_dropped_guard_never_emits_and_never_terminates() {
    for status in ALL {
        for current in [RID, NEWER] {
            let decided = decide_guard_finish(
                &core(status, current),
                guard(RID, GuardOrigin::Dropped, false),
                NOW,
            );
            assert_eq!(decided.row, RowId("R204a.6"));
            // An early return: the caller claims the terminal, so the guard claims none.
            assert!(!tags(&decided)
                .iter()
                .any(|t| t.contains("TERM") || t.contains("EM(")));
            let expected: &[&str] = if current == RID {
                &["Pre:CX-(7)", "In:ST→Idle", "In:KB(false)"]
            } else {
                &["Pre:CX-(7)"]
            };
            assert_eq!(tags(&decided), expected, "{status:?} current={}", current.0);
        }
    }
}

/// R204a.7, the PD-13 fix (#1191): a guard dropped while its thread unwinds claims the
/// recording's terminal before `CX-`, for the current recording and a stale one alike, and
/// never emits `idle`. The shell's `Pre` phase runs the same list.
#[test]
fn r204a_7_an_unwinding_guard_claims_a_worker_panicked_terminal() {
    for status in ALL {
        for current in [RID, NEWER] {
            let decided = decide_guard_finish(
                &core(status, current),
                guard(RID, GuardOrigin::Unwound, false),
                NOW,
            );
            assert_eq!(decided.row, RowId("R204a.7"));
            let expected: &[&str] = if current == RID {
                &[
                    "Pre:TERM(PipelineFailure/WorkerPanicked)",
                    "Pre:CX-(7)",
                    "In:ST→Idle",
                    "In:KB(false)",
                ]
            } else {
                &["Pre:TERM(PipelineFailure/WorkerPanicked)", "Pre:CX-(7)"]
            };
            assert_eq!(tags(&decided), expected, "{status:?} current={}", current.0);
            assert_valid(&decided);
        }
    }
    assert_eq!(
        guard_finish_pre_effects(RID, GuardOrigin::Unwound),
        decide_guard_finish(
            &core(Status::Processing, RID),
            guard(RID, GuardOrigin::Unwound, false),
            NOW
        )
        .effects
        .into_iter()
        .filter(|placed| placed.at == crate::effects::Placement::Pre)
        .collect::<Vec<_>>(),
    );
    for origin in [
        GuardOrigin::StopCapture,
        GuardOrigin::PipelineExit,
        GuardOrigin::LegacyParse,
        GuardOrigin::Dropped,
    ] {
        assert_eq!(
            guard_finish_pre_effects(RID, origin).len(),
            1,
            "{origin:?}: CX- only"
        );
    }
}

// --------------------------------------------------------------------- 204b

fn idle_emit(rid: RecordingId) -> PipelineIdleEmitInput {
    PipelineIdleEmitInput {
        rid,
        kind: PostKind::StopPost,
        result: None,
        interrupted: None,
        char_count: 0,
    }
}

#[test]
fn r204b_1_to_3_the_second_d_section_emits_idle_for_the_current_id() {
    for kind in [PostKind::StopPost, PostKind::LegacyPost] {
        for (status, row) in [
            (Status::Idle, "R204b.1"),
            (Status::Recovering, "R204b.2"),
            // X: 204a wrote Idle for this id; the code would still emit.
            (Status::Processing, "R204b.3"),
            (Status::Starting, "R204b.3"),
            (Status::Recording, "R204b.3"),
        ] {
            let decided = decide_pipeline_idle_emit(
                &core(status, RID),
                PipelineIdleEmitInput {
                    kind,
                    ..idle_emit(RID)
                },
                NOW,
            );
            assert_eq!(decided.verdict, Verdict::Applied);
            assert_eq!(decided.row, RowId(row));
            assert_eq!(tags(&decided), ["In:KB(false)", "In:EM(idle)"]);
            assert_eq!(decided.transition, None, "status is never written here");
            assert_eq!(decided.reply, None);
            assert_valid(&decided);
        }
    }
}

#[test]
fn r204b_4a_to_4e_a_newer_start_between_the_sections_silences_the_emit() {
    for (status, row) in [
        (Status::Idle, "R204b.4a"),
        (Status::Starting, "R204b.4b"),
        (Status::Recording, "R204b.4c"),
        (Status::Recovering, "R204b.4d"),
        (Status::Processing, "R204b.4e"),
    ] {
        let decided = decide_pipeline_idle_emit(&core(status, NEWER), idle_emit(RID), NOW);
        assert_eq!(decided.verdict, Verdict::Stale(StaleKind::StaleRecording));
        assert_eq!(decided.row, RowId(row));
        assert!(decided.effects.is_empty() && decided.transition.is_none());
    }
}

fn tail(result: PipelineResult, interrupted: Option<ErrCode>, chars: usize) -> Decision {
    let decided = decide_pipeline_idle_emit_tail(
        PipelineIdleEmitInput {
            result: Some(result),
            interrupted,
            char_count: chars,
            ..idle_emit(RID)
        },
        NOW,
    );
    assert_valid(&decided);
    decided
}

#[test]
fn r204b_5_pipeline_failure_terminal_and_error_reply() {
    let decided = tail(PipelineResult::Err, None, 0);
    assert_eq!(decided.row, RowId("R204b.5"));
    assert_eq!(
        tags(&decided),
        ["PDT:TERM(PipelineFailure/TranscriptionFailed)"]
    );
    assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
    let interrupted = tail(PipelineResult::Err, Some(ErrCode::DeviceChanged), 0);
    assert_eq!(
        tags(&interrupted),
        ["PDT:TERM(RuntimeInterruption/DeviceChanged)"]
    );
}

#[test]
fn r204b_6_pipeline_result_terminal_performance_finish_and_completion_event() {
    let reply = Some(Reply::Transcription {
        text: TranscriptText::FromPipeline,
        state: Status::Idle,
    });
    let success = tail(PipelineResult::Ok(PipelineTerminal::Success), None, 12);
    assert_eq!(success.row, RowId("R204b.6"));
    assert_eq!(
        tags(&success),
        [
            "PDT:TERM(Success/None)",
            "PDT:PGF(Completed)",
            "PDT:EV(TranscriptionComplete)"
        ]
    );
    assert_eq!(success.reply, reply);
    // No text: no completion event.
    let empty = tail(PipelineResult::Ok(PipelineTerminal::Success), None, 0);
    assert_eq!(
        tags(&empty),
        ["PDT:TERM(Success/None)", "PDT:PGF(Completed)"]
    );
    let no_speech = tail(
        PipelineResult::Ok(PipelineTerminal::NoSpeech(ErrCode::VadNoSpeech)),
        None,
        0,
    );
    assert_eq!(
        tags(&no_speech),
        ["PDT:TERM(NoSpeech/VadNoSpeech)", "PDT:PGF(NoSpeech)"]
    );
    let cancelled = tail(PipelineResult::Ok(PipelineTerminal::Cancelled), None, 0);
    assert_eq!(
        tags(&cancelled),
        [
            "PDT:TERM(UserCancelledProcessing/CancelledProcessing)",
            "PDT:PGF(Cancelled)"
        ]
    );
    // An interruption replaces the terminal; the performance finish still
    // follows the pipeline's own terminal.
    let interrupted = tail(
        PipelineResult::Ok(PipelineTerminal::Success),
        Some(ErrCode::SystemSleep),
        4,
    );
    assert_eq!(
        tags(&interrupted),
        [
            "PDT:TERM(RuntimeInterruption/SystemSleep)",
            "PDT:PGF(Completed)",
            "PDT:EV(TranscriptionComplete)"
        ]
    );
}

#[test]
fn the_tail_is_only_for_a_stop_post_with_a_result() {
    let none = decide_pipeline_idle_emit_tail(idle_emit(RID), NOW);
    assert!(matches!(none.verdict, Verdict::Invalid(_)) && none.effects.is_empty());
    let legacy = decide_pipeline_idle_emit_tail(
        PipelineIdleEmitInput {
            kind: PostKind::LegacyPost,
            result: Some(PipelineResult::Err),
            ..idle_emit(RID)
        },
        NOW,
    );
    assert!(matches!(legacy.verdict, Verdict::Invalid(_)) && legacy.effects.is_empty());
}

// ------------------------------------------------------------ cross-cutting

/// Every stop-family decision this file can produce, for the properties below.
fn every_decision() -> Vec<Decision> {
    let mut all = Vec::new();
    for status in ALL {
        for current in [RID, NEWER] {
            let c = core(status, current);
            for interrupted in [None, Some(None), Some(Some(meta()))] {
                for expected in [None, Some(RID), Some(NEWER)] {
                    all.push(decide_stop_request(
                        &c,
                        StopInput {
                            interrupted,
                            ..stop(expected, current)
                        },
                        NOW,
                    ));
                }
            }
            for present in [false, true] {
                all.push(decide_stop_context(
                    &c,
                    ctx_input(present, OpStatus::Outstanding),
                    NOW,
                ));
            }
            for result in [
                ERR,
                samples(0),
                samples(10),
                interrupted(10),
                samples(MIN_RECORDING_SAMPLES),
            ] {
                all.push(decide_stop_capture_result(
                    &c,
                    capture(result, OpStatus::Outstanding),
                    NOW,
                ));
            }
            for origin in [
                GuardOrigin::StopCapture,
                GuardOrigin::PipelineExit,
                GuardOrigin::LegacyParse,
                GuardOrigin::Dropped,
                GuardOrigin::Unwound,
            ] {
                for emit in [true, false] {
                    all.push(decide_guard_finish(&c, guard(RID, origin, emit), NOW));
                }
            }
            all.push(decide_pipeline_idle_emit(&c, idle_emit(RID), NOW));
        }
        all.push(decide_stop_context_nolock(
            facts(RID),
            ctx_input(true, OpStatus::Unknown),
            NOW,
        ));
    }
    for result in [
        PipelineResult::Err,
        PipelineResult::Ok(PipelineTerminal::Success),
        PipelineResult::Ok(PipelineTerminal::NoSpeech(ErrCode::EmptyOutput)),
        PipelineResult::Ok(PipelineTerminal::Cancelled),
    ] {
        all.push(tail(result, None, 3));
    }
    all.push(decide_stop_capture_result_nolock(
        facts(RID),
        capture(samples(MIN_RECORDING_SAMPLES), OpStatus::Outstanding),
        NOW,
    ));
    all
}

#[test]
fn every_stop_decision_is_structurally_valid_and_owned_by_a_stop_site() {
    let stop_sites = [
        SiteId::StopRequest,
        SiteId::StopContext,
        SiteId::StopContextNoLock,
        SiteId::StopCaptureResult,
        SiteId::GuardFinish,
        SiteId::PipelineIdleEmit,
    ];
    let owned: Vec<&str> = stop_sites.iter().flat_map(|s| rows_of_site(*s)).collect();
    for decided in every_decision() {
        assert_valid(&decided);
        assert!(
            owned.contains(&decided.row.0),
            "{} is not a stop-family row",
            decided.row.0
        );
    }
}

#[test]
fn the_family_produces_every_c3_5_row_but_the_status_blind_aliases() {
    // R202.2 (the schedule where `Idle` landed first) and R203.10 are the same
    // decision as R202.1 / R203.9: no status is read there (C3.0 "not read (no
    // D)"), so a decide function cannot tell them apart.
    let aliases = ["R202.2", "R203.10"];
    let produced: std::collections::BTreeSet<&str> =
        every_decision().iter().map(|d| d.row.0).collect();
    for site in [
        SiteId::StopRequest,
        SiteId::StopContext,
        SiteId::StopContextNoLock,
        SiteId::StopCaptureResult,
        SiteId::GuardFinish,
        SiteId::PipelineIdleEmit,
    ] {
        for row in rows_of_site(site) {
            assert!(
                produced.contains(row) || aliases.contains(&row),
                "no test input produces {row}"
            );
        }
    }
}

#[test]
fn op_status_never_changes_a_decision() {
    // C1.4.1 rule 3: `Unknown` takes the site's own rule with the same effects.
    for status in ALL {
        for current in [RID, NEWER] {
            let c = core(status, current);
            for present in [false, true] {
                assert_eq!(
                    decide_stop_context(&c, ctx_input(present, OpStatus::Outstanding), NOW),
                    decide_stop_context(&c, ctx_input(present, OpStatus::Unknown), NOW)
                );
            }
            for result in [
                ERR,
                samples(0),
                samples(10),
                interrupted(10),
                samples(MIN_RECORDING_SAMPLES),
            ] {
                assert_eq!(
                    decide_stop_capture_result(&c, capture(result, OpStatus::Outstanding), NOW),
                    decide_stop_capture_result(&c, capture(result, OpStatus::Unknown), NOW),
                    "{result:?} {status:?}"
                );
            }
        }
    }
    assert_eq!(
        decide_stop_context_nolock(facts(RID), ctx_input(true, OpStatus::Outstanding), NOW),
        decide_stop_context_nolock(facts(RID), ctx_input(true, OpStatus::Unknown), NOW)
    );
}

#[test]
fn no_stop_decision_reads_m3_or_m4() {
    for status in ALL {
        let base = core(status, RID);
        let varied = DictationCore {
            cancelled_through: RecordingId(99),
            context_for: Some(RID),
            ..base
        };
        for interrupted in [None, Some(Some(meta()))] {
            let input = StopInput {
                interrupted,
                ..stop(None, RID)
            };
            assert_eq!(
                decide_stop_request(&base, input, NOW),
                decide_stop_request(&varied, input, NOW)
            );
        }
        let ctx = ctx_input(false, OpStatus::Outstanding);
        assert_eq!(
            decide_stop_context(&base, ctx, NOW),
            decide_stop_context(&varied, ctx, NOW)
        );
        let cap = capture(ERR, OpStatus::Outstanding);
        assert_eq!(
            decide_stop_capture_result(&base, cap, NOW),
            decide_stop_capture_result(&varied, cap, NOW)
        );
        let g = guard(RID, GuardOrigin::PipelineExit, false);
        assert_eq!(
            decide_guard_finish(&base, g, NOW),
            decide_guard_finish(&varied, g, NOW)
        );
        assert_eq!(
            decide_pipeline_idle_emit(&base, idle_emit(RID), NOW),
            decide_pipeline_idle_emit(&varied, idle_emit(RID), NOW)
        );
    }
}

#[test]
fn continuations_come_only_from_the_rows_that_register_one() {
    for decided in every_decision() {
        let expected = match decided.row.0 {
            "R201.4" => Some(ContKind::CaptureCancel),
            "R201.7" | "R201.8" => Some(ContKind::StopContext),
            "R202.1" => Some(ContKind::CaptureStop),
            "R203.9" => Some(ContKind::Pipeline),
            // R202.5 with a context present is R202.1's decision.
            "R202.5" if decided.effects.first().map(tag).as_deref() == Some("PDT:KB(true)") => {
                Some(ContKind::CaptureStop)
            }
            _ => None,
        };
        assert_eq!(
            decided.continuation.map(|c| c.kind),
            expected,
            "{}",
            decided.row.0
        );
        if let Some(spec) = decided.continuation {
            // Site 201 continues the core's current recording; the others the
            // input's own `rid` (always `RID` here).
            if decided.row.0.starts_with("R201") {
                assert!([RID, NEWER].contains(&spec.recording_id));
            } else {
                assert_eq!(spec.recording_id, RID);
            }
        }
    }
}

#[test]
fn only_row_201_1_asks_for_the_take_outside_recovering() {
    // `take_attempts_iff_recovering` (stop half): the take is requested exactly
    // for a stale expectation (any status) and for `Recovering`.
    for status in ALL {
        for expected in [None, Some(RID), Some(NEWER)] {
            let decided = decide_stop_request(&core(status, RID), stop(expected, RID), NOW);
            let asked = matches!(decided.verdict, Verdict::NeedObservation(_));
            let stale = expected == Some(NEWER);
            assert_eq!(
                asked,
                stale || status == Status::Recovering,
                "{status:?} {expected:?}"
            );
        }
    }
    let _ = TakeMode::Dispose;
}

#[test]
fn st_appears_only_where_a_transition_is_reported_and_always_in_d() {
    for decided in every_decision() {
        let st: Vec<&Placed> = decided
            .effects
            .iter()
            .filter(|p| p.effect.is_set_status())
            .collect();
        assert_eq!(
            st.is_empty(),
            decided.transition.is_none(),
            "{}",
            decided.row.0
        );
        assert!(st.iter().all(|p| p.at == At::In));
    }
}
