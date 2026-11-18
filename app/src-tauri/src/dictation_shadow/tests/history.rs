//! History operations (H1 continuations, H2 pipelines): captured distinctly and
//! compared with the recorded tokens and outcomes. The shadow replays the ids the live
//! shell drew; it copies no history and allocates no op id (C8.1, D-S2c-1 d).

use murmur_dictation_core::decision::{ContKind, OpStatus};
use murmur_dictation_core::ids::{OperationId, RecordingId};
use murmur_dictation_core::status::Status;

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
    // `Outstanding`, so the retirement the live shell logged must have matched.
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Outstanding));
    assert_clean(&h);

    // One that found nothing disagrees with the observation.
    let mut h = harness();
    let mut flight = ready_publish(1, 1, OpStatus::Outstanding);
    for live in flight.header.live_ops.iter_mut() {
        if live.table == HistoryTable::H1 && matches!(live.action, HistoryAction::Retired { .. }) {
            live.action = HistoryAction::Retired { matched: false };
        }
    }
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());

    // A stale token the shell found nothing for is the matching case for `Unknown`.
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Unknown));
    assert_clean(&h);
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

/// Remove every operation of `table` / action kind from a flight's header.
fn without_ops(flight: &mut Flight, keep: impl Fn(&LiveOp) -> bool) {
    let kept: Vec<LiveOp> = flight
        .header
        .live_ops
        .to_vec()
        .into_iter()
        .filter(|op| keep(op))
        .collect();
    flight.header.live_ops.clear();
    for live in kept {
        flight.header.add_op(live);
    }
}

#[test]
fn each_row_declares_the_history_operations_it_requires() {
    // The probes of Sol 6.2 major 4, pinned to their rows: the live shell logs the
    // operations; the shadow derives what the row needs from the decision and the
    // input it was given, so a missing one is a `Continuation` mismatch.
    for (name, build) in [
        ("R203.9", pipeline_start as fn(u64, u64) -> Flight),
        ("R301.5", import_start),
    ] {
        // The faithful flight is clean, and is the row it says.
        let flight = build(1, 1);
        assert_eq!(flight.decision.row.0, name);
        let mut h = harness();
        h.feed(&flight);
        assert_clean(&h);

        // R203.9 / R301.5 start a pipeline: the H2 registration is required, and a
        // flight that closes without it is not clean (probe: R203.9 H2 missing
        // passed; R301.5 correct import reported `Continuation`).
        let mut missing = build(1, 1);
        without_ops(&mut missing, |op| op.table != HistoryTable::H2);
        let mut h = harness();
        h.feed(&missing);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{name}: H2 registration missing"
        );
        assert_eq!(h.stats().matched, 0, "{name}");

        // H2 is not H1: an H1-only registration is the same defect.
        let mut h1_only = build(1, 1);
        without_ops(&mut h1_only, |op| {
            !(op.table == HistoryTable::H2 && op.action == HistoryAction::Registered)
        });
        let mut h = harness();
        h.feed(&h1_only);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{name}"
        );

        // The H1 registration of the decided continuation is required too.
        let mut no_h1 = build(1, 1);
        without_ops(&mut no_h1, |op| {
            !(op.table == HistoryTable::H1 && op.action == HistoryAction::Registered)
        });
        let mut h = harness();
        h.feed_header(&no_h1);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{name}"
        );
    }
}

#[test]
fn r104_2_needs_its_h1_retirement_with_the_full_token() {
    // R104.2 consumes `K:ReadyPublish` (recording RID, op 500): the retirement must
    // name that token, in full. Missing, another op, another kind or another
    // recording all fail the header (probe: missing / wrong-token retirement passed).
    let retirement = |live: &LiveOp| {
        live.table == HistoryTable::H1 && matches!(live.action, HistoryAction::Retired { .. })
    };
    let faithful = ready_publish(1, 1, OpStatus::Outstanding);
    let mut h = harness();
    h.feed(&faithful);
    assert_clean(&h);

    let mut missing = faithful.clone();
    without_ops(&mut missing, |live| !retirement(live));
    let mut h = harness();
    h.feed_header(&missing);
    assert_eq!(
        h.only_event().diff_kind,
        DiffKind::Continuation.code(),
        "missing"
    );

    type Damage = fn(&mut LiveOp);
    let damages: [(&str, Damage); 3] = [
        ("op", |live| live.op = OperationId(501)),
        ("kind", |live| live.cont = ContKind::CaptureStart),
        ("recording", |live| live.recording_id = RecordingId(41)),
    ];
    for (name, damage) in damages {
        let mut wrong = faithful.clone();
        for live in wrong.header.live_ops.iter_mut() {
            if retirement(live) {
                damage(live);
            }
        }
        let mut h = harness();
        h.feed_header(&wrong);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "wrong {name}"
        );
    }

    // Retired twice is not the one retirement the input observed.
    let mut twice = faithful.clone();
    let again = *twice
        .header
        .live_ops
        .iter_mut()
        .find(|live| retirement(live))
        .unwrap();
    twice.header.add_op(again);
    let mut h = harness();
    h.feed_header(&twice);
    assert_eq!(
        h.only_event().diff_kind,
        DiffKind::Continuation.code(),
        "twice"
    );
}

fn retirements(flight: &Flight) -> Vec<LiveOp> {
    flight
        .header
        .live_ops
        .to_vec()
        .into_iter()
        .filter(|live| matches!(live.action, HistoryAction::Retired { .. }))
        .collect()
}

/// The owner's two retirements at a pipeline finish: the full `K:Pipeline` token in
/// H1, the finishing recording in H2.
fn is_owner_retirement(table: HistoryTable) -> impl Fn(&LiveOp) -> bool {
    move |live| live.table == table && matches!(live.action, HistoryAction::Retired { .. })
}

#[test]
fn a_pipeline_finish_needs_both_owner_retirements() {
    // Sol 6.2 round 3, major 2: an R204a.1 `PipelineExit` with neither retirement
    // closed clean. The owner's token is the one the registration drew (op 1001 at
    // `seq` 1), so the whole flow is: the pipeline starts, then its guard finishes.
    for (name, build) in [
        (
            "R204a.1",
            (|| pipeline_exit(2, 2, Status::Processing, RID, RID)) as fn() -> Flight,
        ),
        ("R302.1 guard", || legacy_parse(2, 2, true)),
        ("R302.1 site 302", || legacy_parse(2, 2, false)),
    ] {
        let expected = if name == "R204a.1" {
            "R204a.1"
        } else {
            "R302.1"
        };
        let faithful = build();
        assert_eq!(faithful.decision.row.0, expected, "{name}");
        let mut h = harness();
        h.feed(&pipeline_start(1, 1));
        h.feed(&faithful);
        assert_clean(&h);
        assert_eq!(h.stats().matched, 2, "{name}");

        // Missing: neither, or only one of the two.
        for (what, keep) in [
            ("neither", (|_: &LiveOp| false) as fn(&LiveOp) -> bool),
            ("H1 only", |live| live.table == HistoryTable::H1),
            ("H2 only", |live| live.table == HistoryTable::H2),
        ] {
            let mut missing = build();
            without_ops(&mut missing, |live| {
                !matches!(live.action, HistoryAction::Retired { .. }) || keep(live)
            });
            let mut h = harness();
            h.feed(&pipeline_start(1, 1));
            h.feed(&missing);
            assert_eq!(
                h.only_event().diff_kind,
                DiffKind::Continuation.code(),
                "{name}: {what} retired"
            );
            assert_eq!(h.stats().matched, 1, "{name}: only the start is clean");
        }

        // H1 names the owner's **full** token: its operation, its kind, its recording.
        type Damage = fn(&mut LiveOp);
        let damages: [(&str, Damage); 3] = [
            ("op", |live| live.op = OperationId(1_002)),
            ("kind", |live| live.cont = ContKind::CaptureStart),
            ("recording", |live| live.recording_id = RecordingId(41)),
        ];
        for (what, damage) in damages {
            let mut wrong = build();
            for live in wrong.header.live_ops.iter_mut() {
                if is_owner_retirement(HistoryTable::H1)(live) {
                    damage(live);
                }
            }
            let mut h = harness();
            h.feed(&pipeline_start(1, 1));
            h.feed(&wrong);
            assert_eq!(
                h.only_event().diff_kind,
                DiffKind::Continuation.code(),
                "{name}: H1 wrong {what}"
            );
        }
        // An H1 retirement that found nothing is not the owner's.
        let mut unmatched = build();
        for live in unmatched.header.live_ops.iter_mut() {
            if is_owner_retirement(HistoryTable::H1)(live) {
                live.action = HistoryAction::Retired { matched: false };
            }
        }
        let mut h = harness();
        h.feed(&pipeline_start(1, 1));
        h.feed(&unmatched);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{name}"
        );
        let _ = retirements(&faithful);
    }
}

#[test]
fn a_pipeline_finish_without_a_witnessed_registration_is_unverifiable_never_clean() {
    // Sol round 4, major 2 (D-S2c-3 f): an R204a.1 finish with a substituted H1
    // operation id and a valid H2 retirement closed clean because no registration
    // was in the cache to disagree with. The owner's full token must come from the
    // shadow's own registration witness; without one the finish is neither clean nor
    // a mismatch.
    for (name, build) in [
        (
            "R204a.1",
            (|| pipeline_exit(1, 1, Status::Processing, RID, RID)) as fn() -> Flight,
        ),
        ("R302.1 guard", || legacy_parse(1, 1, true)),
        ("R302.1 site 302", || legacy_parse(1, 1, false)),
    ] {
        // The faithful finish, and one that names a substituted operation id.
        for substituted in [false, true] {
            let mut finish = build();
            if substituted {
                for live in finish.header.live_ops.iter_mut() {
                    if is_owner_retirement(HistoryTable::H1)(live) {
                        live.op = OperationId(9_999);
                    }
                }
            }
            let mut h = harness();
            h.feed(&finish);
            assert!(h.events().is_empty(), "{name}: {:?}", h.events());
            assert_eq!(h.stats().matched, 0, "{name}: not clean");
            assert_eq!(
                h.stats().unverifiable,
                1,
                "{name} substituted={substituted}: {:?}",
                h.stats()
            );
            assert_eq!(h.stats().mismatched_root, 0);
        }
        // A retirement that is missing is a mismatch whether or not it can be checked.
        let mut missing = build();
        without_ops(&mut missing, |live| {
            !matches!(live.action, HistoryAction::Retired { .. })
        });
        let mut h = harness();
        h.feed(&missing);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{name}"
        );
        assert_eq!(h.stats().unverifiable, 0);
    }
    // With the registration witnessed, a substituted operation is a mismatch.
    let mut substituted = pipeline_exit(2, 2, Status::Processing, RID, RID);
    for live in substituted.header.live_ops.iter_mut() {
        if is_owner_retirement(HistoryTable::H1)(live) {
            live.op = OperationId(9_999);
        }
    }
    let mut h = harness();
    h.feed(&pipeline_start(1, 1));
    h.feed(&substituted);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
    assert_eq!(h.stats().unverifiable, 0);
}

#[test]
fn a_stale_pipeline_finish_retires_its_own_recording_not_the_current_one() {
    // R204a.5c: the finishing pipeline's recording is stale (RID) while a newer one
    // is `Recording`. Its H2 witness retired the newer current recording and closed
    // clean, because the newer id is among the ids the record names.
    let newer = RecordingId(RID.0 + 1);
    let build = || pipeline_exit(2, 2, Status::Recording, newer, RID);
    assert_eq!(build().decision.row.0, "R204a.5c");
    let mut h = harness();
    h.feed(&pipeline_start(1, 1));
    h.feed(&build());
    assert_clean(&h);

    for table in [HistoryTable::H2, HistoryTable::H1] {
        let mut misdirected = build();
        for live in misdirected.header.live_ops.iter_mut() {
            if is_owner_retirement(table)(live) {
                live.recording_id = newer;
            }
        }
        let mut h = harness();
        h.feed(&pipeline_start(1, 1));
        h.feed(&misdirected);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Continuation.code(),
            "{table:?} retired the newer current recording"
        );
    }
}
