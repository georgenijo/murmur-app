//! A `StartSideTasks` whose spawner returned before the gate is witnessed explicitly
//! (D-S2c-6): exactly one of a `TickerGate` or a `TickerSkipped`.
//!
//! Stage 2c-2's first E7 replay (`golden_s10` 10b, R207.4) found that a completed
//! record with a decided `PostDT:SD` and no gate was reported as `EffectSet`, though
//! the live spawner is right to return early when no context was recorded or the model
//! has no live preview. The comparator's assumption was the defect.

use super::harness::*;
use crate::dictation_shadow::transport::{mark, Mark};
use crate::dictation_shadow::types::{DiffKind, ShadowMsg, TickerSkipReason};

/// R207.3's `PostDT:SD`: a flight whose decision starts the side tasks after release.
fn spawning_flight() -> Flight {
    ready_while_recording(
        1,
        1,
        Live {
            ticker_before: RID,
            ..Live::default()
        },
    )
}

fn skip(flight: &Flight, reason: TickerSkipReason) -> ShadowMsg {
    ShadowMsg::TickerSkipped {
        rec: flight.header.rec,
        reason,
    }
}

/// The flight with its gate replaced by a skip (the gate's place in the stream).
fn skipping_flight(reason: TickerSkipReason) -> Flight {
    let mut flight = spawning_flight();
    let at = flight
        .later
        .iter()
        .position(|msg| matches!(msg, ShadowMsg::Ticker { .. }))
        .expect("R207.3 witnesses a gate");
    flight.later[at] = skip(&flight, reason);
    flight
}

fn row_skips(h: &Harness) -> Vec<u64> {
    h.stats()
        .per_row
        .values()
        .map(|row| row.ticker_skipped.iter().sum())
        .collect()
}

#[test]
fn a_gate_alone_matches() {
    let mut h = harness();
    h.feed(&spawning_flight());
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    assert_eq!(row_skips(&h), vec![0]);
}

#[test]
fn a_skip_alone_matches_whatever_the_reason() {
    for reason in TickerSkipReason::ALL {
        let mut h = harness();
        h.feed(&skipping_flight(reason));
        assert_clean(&h);
        assert_eq!(h.stats().matched, 1, "{reason:?}: {:?}", h.stats());
        let counted: Vec<u64> = h
            .stats()
            .per_row
            .values()
            .flat_map(|row| row.ticker_skipped)
            .collect();
        let mut expected = vec![0; TickerSkipReason::ALL.len()];
        expected[reason.index()] = 1;
        assert_eq!(counted, expected, "the reason is recorded per row");
        h.comparator.assert_conserved();
    }
}

#[test]
fn neither_a_gate_nor_a_skip_is_an_effect_set_mismatch() {
    // A dropped gate is still caught.
    let mut flight = spawning_flight();
    flight
        .later
        .retain(|msg| !matches!(msg, ShadowMsg::Ticker { .. }));
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
    assert_eq!(h.stats().matched, 0);
}

#[test]
fn a_gate_and_a_skip_together_are_an_effect_set_mismatch() {
    for skip_first in [false, true] {
        let mut flight = spawning_flight();
        let at = flight
            .later
            .iter()
            .position(|msg| matches!(msg, ShadowMsg::Ticker { .. }))
            .unwrap();
        let extra = skip(&flight, TickerSkipReason::NoContext);
        flight
            .later
            .insert(if skip_first { at } else { at + 1 }, extra);
        let mut h = harness();
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::EffectSet.code(),
            "skip first: {skip_first}"
        );
        assert_eq!(h.stats().matched, 0);
        assert_eq!(row_skips(&h).iter().sum::<u64>() == 0, !skip_first);
    }
}

#[test]
fn two_skips_for_one_side_task_are_an_effect_set_mismatch() {
    let mut flight = skipping_flight(TickerSkipReason::NoContext);
    let at = flight
        .later
        .iter()
        .position(|msg| matches!(msg, ShadowMsg::TickerSkipped { .. }))
        .unwrap();
    let again = skip(&flight, TickerSkipReason::NoLivePreview);
    flight.later.insert(at + 1, again);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_skip_for_a_decision_with_no_side_task_is_an_effect_set_mismatch() {
    let mut flight = capture_failed(1, 1, Live::default());
    let extra = skip(&flight, TickerSkipReason::NoContext);
    flight.later.insert(0, extra);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_skip_through_the_producer_path_matches_and_the_summary_names_the_reason() {
    // The same flight over the real one-attempt ring: `mark(TickerSkipped)`.
    let flight = skipping_flight(TickerSkipReason::NoLivePreview);
    let transport = default_transport();
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    let json = h.stats().to_json();
    let row = json["per_row"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(row["ticker_skipped"]["2"], 1, "{json}");
    assert_eq!(row["ticker_skipped"]["1"], 0, "{json}");
    h.comparator.assert_conserved();
    assert_eq!(h.stats().double_close_suppressed, 0);
    drop(recorder);
}

#[test]
fn a_skip_mark_is_one_attempt_and_counted_when_rejected() {
    // Same ring path as every other witness: a full ring hands the message back and
    // counts a loss, never waits.
    let transport = local_transport(2);
    let recorder = transport.recorder();
    for _ in 0..4 {
        mark(
            &recorder,
            Mark::TickerSkipped {
                reason: TickerSkipReason::NoContext,
            },
        );
    }
    assert!(transport.lost() > 0);
}

#[test]
fn skip_reason_codes_are_closed_and_one_based() {
    let codes: Vec<u8> = TickerSkipReason::ALL.iter().map(|r| r.code()).collect();
    assert_eq!(codes, vec![1, 2]);
    for (i, reason) in TickerSkipReason::ALL.iter().enumerate() {
        assert_eq!(reason.index(), i);
    }
}
