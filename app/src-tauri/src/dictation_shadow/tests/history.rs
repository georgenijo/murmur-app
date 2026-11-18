//! History operations (H1 continuations, H2 pipelines): captured distinctly and
//! compared with the recorded tokens and outcomes. The shadow replays the ids the live
//! shell drew; it copies no history and allocates no op id (C8.1, D-S2c-1 d).

use murmur_dictation_core::decision::{ContKind, OpStatus};
use murmur_dictation_core::ids::{OperationId, RecordingId};

use super::harness::*;
use crate::dictation_shadow::compare::replayed_token;
use crate::dictation_shadow::types::{
    DiffKind, HistoryAction, HistoryTable, LiveOp, RecorderId, ShadowMsg,
};

fn op(table: HistoryTable, action: HistoryAction, cont: ContKind, op: u64) -> LiveOp {
    LiveOp {
        table,
        action,
        cont,
        recording_id: RID,
        op: OperationId(op),
    }
}

#[test]
fn shadow_replays_live_op_ids() {
    // The token a continuation input will carry is the live shell's id.
    let start = start(1, 1, Live::default());
    let live_ops: Vec<LiveOp> = start.header.live_ops.to_vec();
    let token = replayed_token(&start.decision, &live_ops).expect("a continuation");
    assert_eq!(token.op, OperationId(1_001));
    assert_eq!(token.kind, ContKind::CaptureStart);
    // A registration of another kind, or of none, is `diff_kind 6`.
    for damage in [0, 1] {
        let mut h = harness();
        let mut damaged = start.clone();
        match damage {
            0 => damaged.header.live_ops.clear(),
            _ => damaged.header.live_ops.iter_mut().next().unwrap().cont = ContKind::ReadyPublish,
        }
        h.feed(&damaged);
        assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
    }
    // A registration the decision never asked for.
    let mut h = harness();
    let mut extra = short_idle(1, 1);
    extra.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Registered,
        ContKind::CaptureStop,
        9,
    ));
    h.feed(&extra);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
    // Two registrations for one decided continuation.
    let mut h = harness();
    let mut twice = start.clone();
    twice.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Registered,
        ContKind::CaptureStart,
        10,
    ));
    h.feed(&twice);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
}

#[test]
fn an_h1_retirement_must_agree_with_the_token_the_input_carried() {
    // R104.2 consumes `K:ReadyPublish` (token op 500). The input says the token was
    // `Outstanding`, so the retirement must have matched; one that found nothing
    // disagrees with the observation. (Retiring is optional: no retirement at all is
    // clean.)
    let mut h = harness();
    let mut flight = ready_publish(1, 1, OpStatus::Outstanding);
    flight.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Retired { matched: true },
        ContKind::ReadyPublish,
        500,
    ));
    h.feed(&flight);
    assert_clean(&h);

    let mut h = harness();
    let mut flight = ready_publish(1, 1, OpStatus::Outstanding);
    flight.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Retired { matched: false },
        ContKind::ReadyPublish,
        500,
    ));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());

    // A stale token the shell found nothing for is the matching case for `Unknown`.
    let mut h = harness();
    let mut flight = ready_publish(1, 1, OpStatus::Unknown);
    flight.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Retired { matched: false },
        ContKind::ReadyPublish,
        500,
    ));
    h.feed_header(&flight);
    assert!(h.events().is_empty(), "{:?}", h.events());
}

#[test]
fn an_h2_registration_needs_a_decided_pipeline_and_h2_is_not_h1() {
    // R101.15 decides no `PS`: an H2 registration is not part of it.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    flight.header.add_op(op(
        HistoryTable::H2,
        HistoryAction::Registered,
        ContKind::Pipeline,
        77,
    ));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());

    // The same operation recorded as H1 is judged against the continuation instead,
    // and is also wrong here: the two tables are not interchangeable.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    flight.header.add_op(op(
        HistoryTable::H1,
        HistoryAction::Registered,
        ContKind::Pipeline,
        77,
    ));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());

    // An H2 retirement that found nothing is a defect of the pipeline's owner.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    flight.header.add_op(op(
        HistoryTable::H2,
        HistoryAction::Retired { matched: false },
        ContKind::Pipeline,
        78,
    ));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
}

#[test]
fn a_history_operation_after_d_is_compared_too() {
    // An owner's `Drop` retires its pipeline after D: a retirement that matched is
    // fine, one that found nothing is not.
    let mut h = harness();
    let flight = cancel(1, 1);
    h.feed_header(&flight);
    let rec = flight.header.rec;
    h.send(ShadowMsg::History {
        rec,
        op: op(
            HistoryTable::H2,
            HistoryAction::Retired { matched: true },
            ContKind::Pipeline,
            5,
        ),
    });
    assert!(h.events().is_empty());
    h.send(ShadowMsg::History {
        rec,
        op: op(
            HistoryTable::H2,
            HistoryAction::Retired { matched: false },
            ContKind::Pipeline,
            6,
        ),
    });
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
    let _ = (RecorderId(0), RecordingId(0));
}
