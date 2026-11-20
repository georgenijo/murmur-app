//! One test per C3.5 cancel row (R205.1-R205.5, R206.1-R206.2): guard,
//! observations consumed, mutation, ordered placed effects, continuation and
//! reply, for every status that reaches the row (stale and no-op rows included),
//! plus the cross-cutting properties of the family.
//!
//! Expected effect lists are written as the appendix prints them
//! (`PDT:CX-`, `PT:CC`), through [`tags`], not through the builders `cancel.rs`
//! uses, so a reordered or re-placed effect fails here.

use super::*;
use crate::decision::{Continuation, OpStatus, SiteId};
use crate::ids::{OperationId, RecordingId};
use crate::rows::rows_of_site;
use crate::state::LiveObs;

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

fn tag(placed: &crate::effects::Placed) -> String {
    let at = placed.at.label();
    let body = match &placed.effect {
        Effect::SetStatus(to) => format!("ST→{to:?}"),
        Effect::EmitStatus { status, .. } => format!("EM({})", status.as_str()),
        Effect::EmitEvent(kind) => format!("EV({kind:?})"),
        Effect::Terminal(t) => format!("TERM({:?}/{:?})", t.outcome, t.code),
        Effect::PerfComplete(pc) => format!("PC({:?},{:?},{:?})", pc.outcome, pc.stage, pc.cause),
        Effect::ClearContext(id) => format!("CX-({})", id.0),
        Effect::MarkCancelled(id) => format!("MC({})", id.0),
        Effect::KeyboardProcessing(on) => format!("KB({on})"),
        Effect::CaptureCancel { id, reason } => format!("CC({},{reason:?})", id.0),
        other => panic!("not a cancel-family effect: {other:?}"),
    };
    format!("{at}:{body}")
}

fn tags(decided: &Decision) -> Vec<String> {
    decided.effects.iter().map(tag).collect()
}

fn assert_valid(decided: &Decision) {
    assert_eq!(decided.validate(), Ok(()), "{}", decided.row.0);
}

fn cancel(expected: Option<RecordingId>) -> CancelInput {
    CancelInput { expected }
}

fn d_205(status: Status, current: RecordingId, expected: Option<RecordingId>) -> Decision {
    let decided = decide_cancel_request(&core(status, current), cancel(expected), NOW);
    assert_valid(&decided);
    decided
}

/// The shared checks of one 205 row.
fn expect_205(
    decided: &Decision,
    row: &str,
    verdict: Verdict,
    transition: Option<(Status, Status)>,
    effects: &[&str],
    continuation: Option<ContKind>,
) {
    assert_eq!(decided.row.0, row);
    assert_eq!(decided.verdict, verdict, "{row} verdict");
    assert_eq!(decided.transition, transition, "{row} transition");
    assert_eq!(tags(decided), effects, "{row} effects");
    assert_eq!(
        decided.continuation,
        continuation.map(|kind| ContinuationSpec {
            recording_id: RID,
            kind
        }),
        "{row} continuation"
    );
    assert_eq!(decided.reply, Some(Reply::Ok), "{row} reply");
}

// ------------------------------------------------------------------ R205.1

#[test]
fn r205_1_a_stale_expected_owner_is_a_silent_no_op_whatever_the_status() {
    // The owner check precedes the status read (6390): PD-12, no terminal.
    for status in ALL {
        for expected in [NEWER, RecordingId(1)] {
            let decided = d_205(status, RID, Some(expected));
            expect_205(
                &decided,
                "R205.1",
                Verdict::Stale(StaleKind::StaleRecording),
                None,
                &[],
                None,
            );
        }
    }
}

#[test]
fn r205_1_the_expected_owner_matching_the_current_one_is_not_stale() {
    for status in ALL {
        let with = d_205(status, RID, Some(RID));
        let without = d_205(status, RID, None);
        assert_eq!(with, without, "{status:?}");
        assert_ne!(with.row.0, "R205.1");
    }
}

// ------------------------------------------------------------------ R205.2-.3

#[test]
fn r205_2_idle_is_a_no_op() {
    for expected in [None, Some(RID)] {
        let decided = d_205(Status::Idle, RID, expected);
        expect_205(&decided, "R205.2", Verdict::Continue, None, &[], None);
    }
}

#[test]
fn r205_3_recovering_is_a_no_op() {
    for expected in [None, Some(RID)] {
        let decided = d_205(Status::Recovering, RID, expected);
        expect_205(&decided, "R205.3", Verdict::Continue, None, &[], None);
    }
}

// ------------------------------------------------------------------ R205.4

#[test]
fn r205_4_starting_and_recording_cancel_the_capture_after_t_without_a_status_write() {
    for status in [Status::Starting, Status::Recording] {
        for expected in [None, Some(RID)] {
            let decided = d_205(status, RID, expected);
            expect_205(
                &decided,
                "R205.4",
                Verdict::Applied,
                None,
                &["PT:CC(7,User)"],
                Some(ContKind::CaptureCancel),
            );
        }
    }
}

#[test]
fn r205_4_names_the_current_recording_not_the_expectation() {
    // The effect and the continuation carry the core's id (6407 reads the live
    // `recording_id`); an expectation can only equal it.
    let decided = d_205(Status::Recording, NEWER, Some(NEWER));
    assert_eq!(tags(&decided), ["PT:CC(8,User)"]);
    assert_eq!(decided.continuation.unwrap().recording_id, NEWER);
}

// ------------------------------------------------------------------ R205.5

#[test]
fn r205_5_processing_writes_idle_then_clears_cancels_and_announces() {
    for expected in [None, Some(RID)] {
        let decided = d_205(Status::Processing, RID, expected);
        expect_205(
            &decided,
            "R205.5",
            Verdict::Applied,
            Some((Status::Processing, Status::Idle)),
            &[
                "In:ST→Idle",
                "PDT:CX-(7)",
                "PDT:MC(7)",
                "PDT:TERM(UserCancelledProcessing/CancelledProcessing)",
                "PDT:KB(false)",
                "PDT:EM(idle)",
                "PDT:EV(Cancelled)",
                "PT:PC(Cancelled,InferenceDecode,None)",
            ],
            None,
        );
    }
}

#[test]
fn r205_5_registers_no_continuation_and_leaves_the_pipeline_draining() {
    // `pipelines[rid]` is not touched by this row (C3.9): there is no history
    // operation, only the missing one is the draining pipeline's.
    let decided = d_205(Status::Processing, RID, None);
    assert_eq!(decided.continuation, None);
    assert!(decided.effects.iter().all(|p| !matches!(
        p.effect,
        Effect::PipelineSpawn(_) | Effect::PipelineGuardFinish(_)
    )));
}

#[test]
fn r205_5_commits_mc_after_the_status_write_and_before_the_terminal() {
    // C4.7 ordering: ST (In) before CX- before MC before TERM, KB, EM, EV; the
    // window between the status write and MC is preserved (PD-16).
    let order = tags(&d_205(Status::Processing, RID, None));
    let at = |wanted: &str| {
        order
            .iter()
            .position(|t| t.contains(wanted))
            .unwrap_or_else(|| panic!("{wanted} missing"))
    };
    assert!(at("ST→") < at("CX-") && at("CX-") < at("MC"));
    assert!(at("MC") < at("TERM") && at("TERM") < at("KB"));
    assert!(at("KB") < at("EM(idle)") && at("EM(idle)") < at("EV(Cancelled)"));
    assert!(at("EV(Cancelled)") < at("PC"));
}

#[test]
fn r205_5_the_perf_completion_alone_runs_after_t_is_released() {
    let decided = d_205(Status::Processing, RID, None);
    let late: Vec<&str> = decided
        .effects
        .iter()
        .filter(|p| p.at == At::PostT)
        .map(|p| p.effect.code())
        .collect();
    assert_eq!(late, ["PC"]);
    // Nothing runs under D but the status write (D is released at 6402).
    let under_d: Vec<&str> = decided
        .effects
        .iter()
        .filter(|p| p.at == At::In)
        .map(|p| p.effect.code())
        .collect();
    assert_eq!(under_d, ["ST"]);
}

// ------------------------------------------------------------------ R206

fn token(kind: ContKind) -> Continuation {
    Continuation {
        recording_id: RID,
        op: OperationId(5),
        kind,
    }
}

fn result(
    result: Result<bool, ()>,
    origin: CancelOrigin,
    op_status: OpStatus,
) -> CancelCaptureResultInput {
    CancelCaptureResultInput {
        token: token(ContKind::CaptureCancel),
        op_status,
        rid: RID,
        result,
        origin,
    }
}

fn d_206(outcome: Result<bool, ()>, origin: CancelOrigin, op_status: OpStatus) -> Decision {
    let decided =
        decide_cancel_capture_result_nolock(facts(RID), result(outcome, origin, op_status), NOW);
    assert_valid(&decided);
    decided
}

#[test]
fn r206_1_a_successful_capture_cancel_has_no_effect_and_the_callers_own_reply() {
    for accepted in [true, false] {
        let stop = d_206(Ok(accepted), CancelOrigin::Stop, OpStatus::Outstanding);
        assert_eq!(stop.row.0, "R206.1");
        assert_eq!(stop.verdict, Verdict::Continue);
        assert!(stop.effects.is_empty() && stop.transition.is_none());
        assert_eq!(stop.continuation, None);
        assert_eq!(
            stop.reply,
            Some(Reply::typed(
                ReplyType::RecordingCancelled,
                Status::Recovering
            ))
        );
        let cancel = d_206(Ok(accepted), CancelOrigin::Cancel, OpStatus::Outstanding);
        assert_eq!(cancel.row.0, "R206.1");
        assert_eq!(cancel.verdict, Verdict::Continue);
        assert!(cancel.effects.is_empty() && cancel.transition.is_none());
        assert_eq!(cancel.reply, Some(Reply::Ok));
    }
}

#[test]
fn r206_2_a_failed_capture_cancel_claims_no_terminal_for_both_callers() {
    // PD-21 fixed: the recording is not over (a stop goes on, or the supervisor still
    // handles the queued `Cancel{User}`), so the failure only answers the caller's error.
    for origin in [CancelOrigin::Stop, CancelOrigin::Cancel] {
        let decided = d_206(Err(()), origin, OpStatus::Outstanding);
        assert_eq!(decided.row.0, "R206.2");
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(decided.transition, None);
        assert!(decided.effects.is_empty(), "{origin:?}: no terminal claim");
        assert_eq!(tags(&decided), Vec::<String>::new(), "{origin:?}");
        assert_eq!(decided.continuation, None);
        assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
    }
}

#[test]
fn site_206_reads_neither_status_nor_id_nor_the_cancel_mark() {
    // D-less: the facts are not consulted, so any facts give the same decision.
    let other = LockFreeFacts {
        current_id: NEWER,
        cancelled_through: RecordingId(99),
    };
    for outcome in [Ok(true), Ok(false), Err(())] {
        for origin in [CancelOrigin::Stop, CancelOrigin::Cancel] {
            let input = result(outcome, origin, OpStatus::Outstanding);
            assert_eq!(
                decide_cancel_capture_result_nolock(facts(RID), input, NOW),
                decide_cancel_capture_result_nolock(other, input, NOW)
            );
        }
    }
}

// ------------------------------------------------------------ cross-cutting

fn every_decision() -> Vec<Decision> {
    let mut all = Vec::new();
    for status in ALL {
        for expected in [None, Some(RID), Some(NEWER)] {
            all.push(d_205(status, RID, expected));
        }
    }
    for outcome in [Ok(true), Ok(false), Err(())] {
        for origin in [CancelOrigin::Stop, CancelOrigin::Cancel] {
            all.push(d_206(outcome, origin, OpStatus::Outstanding));
        }
    }
    all
}

#[test]
fn every_cancel_decision_is_structurally_valid_and_owned_by_a_cancel_site() {
    let owned: Vec<&str> = [SiteId::CancelRequest, SiteId::CancelCaptureResult]
        .iter()
        .flat_map(|s| rows_of_site(*s))
        .collect();
    for decided in every_decision() {
        assert_valid(&decided);
        assert!(
            owned.contains(&decided.row.0),
            "{} is not a cancel-family row",
            decided.row.0
        );
    }
}

#[test]
fn the_family_produces_every_c3_5_cancel_row() {
    let produced: std::collections::BTreeSet<&str> =
        every_decision().iter().map(|d| d.row.0).collect();
    for site in [SiteId::CancelRequest, SiteId::CancelCaptureResult] {
        for row in rows_of_site(site) {
            assert!(produced.contains(row), "no test input produces {row}");
        }
    }
}

#[test]
fn op_status_never_changes_a_decision() {
    // C1.4.1 rule 3: `Unknown` takes the site's own rule with the same effects.
    for outcome in [Ok(true), Ok(false), Err(())] {
        for origin in [CancelOrigin::Stop, CancelOrigin::Cancel] {
            assert_eq!(
                d_206(outcome, origin, OpStatus::Outstanding),
                d_206(outcome, origin, OpStatus::Unknown),
                "{outcome:?} {origin:?}"
            );
        }
    }
}

#[test]
fn no_cancel_decision_reads_m3_or_m4() {
    for status in ALL {
        let base = core(status, RID);
        let varied = DictationCore {
            cancelled_through: RecordingId(99),
            context_for: Some(RID),
            ..base
        };
        for expected in [None, Some(RID), Some(NEWER)] {
            assert_eq!(
                decide_cancel_request(&base, cancel(expected), NOW),
                decide_cancel_request(&varied, cancel(expected), NOW),
                "{status:?} {expected:?}"
            );
        }
    }
}

#[test]
fn only_r205_4_registers_a_continuation() {
    for decided in every_decision() {
        let expected = (decided.row.0 == "R205.4").then_some(ContKind::CaptureCancel);
        assert_eq!(
            decided.continuation.map(|c| c.kind),
            expected,
            "{}",
            decided.row.0
        );
    }
}

#[test]
fn st_appears_only_where_a_transition_is_reported_and_always_in_d() {
    for decided in every_decision() {
        let st: Vec<_> = decided
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
        // Only R205.5 writes the status (R205.4 leaves `Recovering{User}` to the
        // supervisor, R209).
        assert_eq!(!st.is_empty(), decided.row.0 == "R205.5");
    }
}

#[test]
fn a_decision_never_writes_a_status_it_did_not_read() {
    // The cancel command's only write is `Processing→Idle`.
    for status in ALL {
        let decided = d_205(status, RID, None);
        if let Some(pair) = decided.transition {
            assert_eq!(pair, (Status::Processing, Status::Idle));
        }
    }
}
