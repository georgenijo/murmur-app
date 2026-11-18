//! Stage 4 historical bug replays, core half (appendix C13.3, plan §6 Stage 4).
//!
//! Each test is named as C13.3 names it (or `<name>_core` where the shell
//! owns the named test and this is its core-visible companion) and drives the
//! crate's public API only: `admit` for the admission rows, `decide_*` for the
//! C3 rows. The shell half (the same bugs against `start_native_recording` /
//! `stop_native_recording` on fakes) is `recording_bug_replay_tests.rs` in the
//! `ui` crate. Test-only: nothing here changes production behaviour.
//!
//! The mutation that must fail each test is recorded in the PR body and next to
//! the test.

use murmur_dictation_core::admission::{
    admit, Admission, ClaimKind, Occupancy, PreviewDisposition, RefusalCode, Request,
};
use murmur_dictation_core::decision::{
    ContKind, Continuation, Decision, OpStatus, StaleKind, Verdict,
};
use murmur_dictation_core::effects::{Effect, Placement};
use murmur_dictation_core::ids::{Now, OperationId, PreviewId, RecordingId};
use murmur_dictation_core::inputs::{
    CaptureErr, CaptureResult, CaptureResultInput, ReadyPublishInput,
};
use murmur_dictation_core::lifecycle::decide_sup_idle;
use murmur_dictation_core::start::{decide_ready_publish, decide_start_capture_result};
use murmur_dictation_core::state::{DictationCore, LiveObs};
use murmur_dictation_core::status::Status;

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);

fn core(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(status, current, LiveObs::default())
}

fn token(kind: ContKind) -> Continuation {
    Continuation {
        recording_id: RID,
        op: OperationId(5),
        kind,
    }
}

fn has(decision: &Decision, wanted: impl Fn(&Effect) -> bool) -> bool {
    decision.effects.iter().any(|placed| wanted(&placed.effect))
}

// ------------------------------------------------------------------ #216

/// #216 / PR #219 (`06296f79`), the publish half: a late `Ready` publish needs
/// status `Starting`. R104.5 (current id, status `Idle`, `Recovering` or
/// `Processing`, every context observation) is a `Continue` with no `ST`,
/// `EM(recording)` or `SD`. The shell-level `RuntimeFailure` witness of the same
/// name lives in `recording_bug_replay_tests.rs`.
///
/// Mutation: drop R104.5's `status == Starting` guard (so the publish falls
/// through to the context rows) and every loop iteration with a context fails.
#[test]
fn bug216_late_publish_requires_starting_core() {
    for status in [Status::Idle, Status::Recovering, Status::Processing] {
        for context_present in [None, Some(true), Some(false)] {
            let decided = decide_ready_publish(
                &core(status, RID),
                ReadyPublishInput {
                    token: token(ContKind::ReadyPublish),
                    op_status: OpStatus::Outstanding,
                    rid: RID,
                    audio_ready: true,
                    context_present,
                    context_ready_at: NOW,
                },
                NOW,
            );
            assert_eq!(decided.row.0, "R104.5", "{status:?} / {context_present:?}");
            assert_eq!(decided.verdict, Verdict::Continue);
            assert_eq!(decided.transition, None);
            assert!(
                decided.effects.is_empty(),
                "{status:?} / {context_present:?}: no ST, EM(recording) or SD"
            );
        }
    }
    // The control: the same publish from `Starting` with a context is R104.2.
    let applied = decide_ready_publish(
        &core(Status::Starting, RID),
        ReadyPublishInput {
            token: token(ContKind::ReadyPublish),
            op_status: OpStatus::Outstanding,
            rid: RID,
            audio_ready: true,
            context_present: Some(true),
            context_ready_at: NOW,
        },
        NOW,
    );
    assert_eq!(applied.row.0, "R104.2");
    assert_eq!(
        applied.transition,
        Some((Status::Starting, Status::Recording))
    );
    assert!(has(&applied, |e| matches!(e, Effect::StartSideTasks)));
}

// ------------------------------------------------------------------ #384

/// #384 (`26ceb195`): a supervisor `Idle` for the current id while the status is
/// `Processing` (the timed-out stop's teardown) forces `Idle`. R211.3:
/// `TERM(PipelineFailure/RuntimeFailure)`, `PC(Failed)`, `ST→Idle`, `CX-`,
/// `KB(false)` and the `idle` emit, all `In`.
///
/// Mutation: make R211.3 a no-op (no transition, no effects): this fails.
#[test]
fn bug384_idle_after_timed_out_stop_core() {
    let decided = decide_sup_idle(&core(Status::Processing, RID), RID, NOW);
    assert_eq!(decided.row.0, "R211.3");
    assert_eq!(decided.verdict, Verdict::Applied);
    assert_eq!(decided.transition, Some((Status::Processing, Status::Idle)));
    assert!(has(&decided, |e| matches!(
        e,
        Effect::SetStatus(Status::Idle)
    )));
    assert!(has(
        &decided,
        |e| matches!(e, Effect::ClearContext(id) if *id == RID)
    ));
    assert!(has(&decided, |e| matches!(
        e,
        Effect::KeyboardProcessing(false)
    )));
    assert!(has(&decided, |e| matches!(
        e,
        Effect::EmitStatus {
            status: Status::Idle,
            ..
        }
    )));
    assert!(has(&decided, |e| matches!(e, Effect::Terminal(_))));
    assert!(decided
        .effects
        .iter()
        .all(|placed| placed.at == Placement::In));
}

// ------------------------------------------------------------------ #388

/// #388 / PR #392, core-owned part (`66dd78d7`, supervisor-only, so no revert
/// applies): any grammar-legal sequence over a start ends in `Idle`. A supervisor
/// `Idle` over `Starting` (R211.4) or `Recovering` (R211.6), and a capture-start
/// failure over `Starting` (R102.2) or `Recovering` (R102.3), all write `Idle`.
///
/// Mutation: make R211.4, R211.6 and R102.3 no-ops (the C13.3 set).
#[test]
fn bug388_init_failure_reaches_idle_core() {
    for (status, row) in [(Status::Starting, "R211.4"), (Status::Recovering, "R211.6")] {
        let decided = decide_sup_idle(&core(status, RID), RID, NOW);
        assert_eq!(decided.row.0, row);
        assert_eq!(decided.transition, Some((status, Status::Idle)), "{row}");
        assert!(has(&decided, |e| matches!(e, Effect::Terminal(_))), "{row}");
    }
    for (status, row) in [(Status::Starting, "R102.2"), (Status::Recovering, "R102.3")] {
        let decided = decide_start_capture_result(
            &core(status, RID),
            CaptureResultInput {
                token: token(ContKind::CaptureStart),
                op_status: OpStatus::Outstanding,
                result: CaptureResult::Err(CaptureErr::Other),
            },
            NOW,
        );
        assert_eq!(decided.row.0, row);
        assert_eq!(decided.transition, Some((status, Status::Idle)), "{row}");
        assert!(has(&decided, |e| matches!(e, Effect::Terminal(_))), "{row}");
    }
}

// ------------------------------------------------------------------ #535

/// #535 (`ddf2b6a1`, `80747cd4`), the `TERM` rows: a start failure (R102.2) and
/// every status-forcing supervisor `Idle` (R211.3) carry exactly one `TERM`
/// effect; a stale `Idle` (R211.1) is the `Superseded` stale-owner terminal.
///
/// Mutation: drop `TERM` from R102.2, or make R211.3's `TERM` conditional.
#[test]
fn bug535_exactly_one_terminal_core() {
    let terminals = |decision: &Decision| {
        decision
            .effects
            .iter()
            .filter(|placed| matches!(placed.effect, Effect::Terminal(_)))
            .count()
    };
    let failure = decide_start_capture_result(
        &core(Status::Starting, RID),
        CaptureResultInput {
            token: token(ContKind::CaptureStart),
            op_status: OpStatus::Outstanding,
            result: CaptureResult::Err(CaptureErr::Other),
        },
        NOW,
    );
    assert_eq!(failure.row.0, "R102.2");
    assert_eq!(terminals(&failure), 1, "R102.2");
    let forced = decide_sup_idle(&core(Status::Processing, RID), RID, NOW);
    assert_eq!(forced.row.0, "R211.3");
    assert_eq!(terminals(&forced), 1, "R211.3");
    let stale = decide_sup_idle(&core(Status::Processing, RecordingId(8)), RID, NOW);
    assert_eq!(stale.row.0, "R211.1");
    assert_eq!(stale.verdict, Verdict::Stale(StaleKind::StaleRecording));
    assert_eq!(terminals(&stale), 1, "R211.1");
}

// ------------------------------------------- 0b798457, bb66238b, 3c44d9e4

/// `0b798457`: a queued preparation must not run while a preview is connecting.
/// `admit(DictationPrepare)` with `preview_active` refuses `preview_active`.
///
/// Mutation: delete the `preview_active` check (the second check) from the
/// C6.2 #2 order.
#[test]
fn bug_prep_admission_preview() {
    let preview = Occupancy {
        preview_active: true,
        ..Occupancy::default()
    };
    assert_eq!(
        admit(Request::DictationPrepare, &preview),
        Admission::RefuseWith(RefusalCode::PreviewActive)
    );
    assert!(matches!(
        admit(Request::DictationPrepare, &Occupancy::default()),
        Admission::Admit(_)
    ));
}

/// `bb66238b`: a committed permission relaunch refuses a meeting claim after the
/// stores (`MeetingPostClaim` with `relaunch_recheck = Some(true)`), and refuses
/// a dictation start with `busy_restarting`.
///
/// Mutation: remove the `relaunch_recheck` refusal from `MeetingPostClaim` and
/// R101.3's `relaunch_committed` check.
#[test]
fn bug_relaunch_vs_meeting_claim() {
    let recheck = Occupancy {
        relaunch_recheck: Some(true),
        ..Occupancy::default()
    };
    assert_eq!(
        admit(Request::MeetingPostClaim, &recheck),
        Admission::RefuseWith(RefusalCode::MeetingRestarting)
    );
    let clear = Occupancy {
        relaunch_recheck: Some(false),
        ..Occupancy::default()
    };
    assert!(matches!(
        admit(Request::MeetingPostClaim, &clear),
        Admission::Admit(_)
    ));
    let committed = Occupancy {
        relaunch_committed: true,
        ..Occupancy::default()
    };
    assert_eq!(
        admit(Request::DictationStart, &committed),
        Admission::RefuseWith(RefusalCode::BusyRestarting)
    );
}

/// `3c44d9e4` (PR #1128): a key-down over an open mic test with a handoff
/// candidate takes the preview over (`Takeover(p)`) instead of refusing
/// `preview_active`; without a candidate the old refusal stands; a dictation
/// start over a preview stops it and retries, or keeps it for the handoff.
///
/// Mutation: change KeyDownCapture x preview-with-candidate from K to R, and
/// DictationStart x preview from K to stop-and-retry.
#[test]
fn bug_preview_handoff_admission() {
    let candidate = Occupancy {
        preview_active: true,
        handoff_candidate: Some(11),
        ..Occupancy::default()
    };
    assert_eq!(
        admit(Request::KeyDownCapture, &candidate),
        Admission::Takeover(PreviewId(11))
    );
    let none = Occupancy {
        preview_active: true,
        ..Occupancy::default()
    };
    assert_eq!(
        admit(Request::KeyDownCapture, &none),
        Admission::RefuseWith(RefusalCode::PreviewActive)
    );
    match admit(Request::DictationStart, &candidate) {
        Admission::Admit(claims) => {
            assert_eq!(claims.claim, ClaimKind::DictationTransition);
            assert_eq!(
                claims.preview,
                PreviewDisposition::KeptForHandoff(PreviewId(11))
            );
        }
        other => panic!("a start over a kept preview is admitted for the handoff: {other:?}"),
    }
    match admit(Request::DictationStart, &none) {
        Admission::Admit(claims) => {
            assert_eq!(claims.preview, PreviewDisposition::StopThenRetry);
        }
        other => panic!("a start over a plain preview stops it and retries: {other:?}"),
    }
}
