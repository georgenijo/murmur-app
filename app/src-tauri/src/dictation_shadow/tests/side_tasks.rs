//! D-S2c-7 (Stage 2c-2's API for the comparator): gate-or-skip for an `In:SD`, an
//! observation sent as a mark, header mirrors for the keyboard flag and the
//! cancelled-through mark, and a lost ticket.

use murmur_dictation_core::decision::{ContKind, Continuation, OpStatus};
use murmur_dictation_core::effects::{Effect, Placed, Placement};
use murmur_dictation_core::ids::{OperationId, RecordingId};
use murmur_dictation_core::inputs::{Input, StopContextInput};
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::compare::Comparator;
use crate::dictation_shadow::transport::{mark, Mark};
use crate::dictation_shadow::types::{
    DiffKind, MirrorField, MirrorValue, MirrorWitness, ObservedFact, ShadowMsg, TickerSkipReason,
    WitnessedEffect,
};

// ------------------------------------------------- gate-or-skip for an `In:SD`

/// R104.2: its `SD` runs under D, so a gate is in the header.
fn in_sd_flight() -> Flight {
    ready_publish(1, 1, OpStatus::Outstanding)
}

fn without_gate(mut flight: Flight) -> Flight {
    assert_eq!(flight.header.in_d_tickers.len(), 1, "R104.2 has one In:SD");
    flight.header.in_d_tickers.clear();
    flight
}

/// The skip is a mark after the header (the spawner ran under D, inside `run(In)`).
fn skip_after_header(flight: &mut Flight, target: RecordingId, reason: TickerSkipReason) {
    flight.later.insert(
        0,
        ShadowMsg::TickerSkipped {
            rec: flight.header.rec,
            target,
            reason,
        },
    );
}

#[test]
fn an_in_d_gate_alone_matches() {
    let mut h = harness();
    h.feed(&in_sd_flight());
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
}

#[test]
fn an_in_d_skip_after_the_header_alone_matches_for_every_reason() {
    for reason in TickerSkipReason::ALL {
        let mut flight = without_gate(in_sd_flight());
        skip_after_header(&mut flight, RID, reason);
        let mut h = harness();
        h.feed(&flight);
        assert_clean(&h);
        assert_eq!(h.stats().matched, 1, "{reason:?}: {:?}", h.stats());
        let counted: u64 = h
            .stats()
            .per_row
            .values()
            .map(|row| row.ticker_skipped[reason.index()])
            .sum();
        assert_eq!(counted, 1, "the reason is recorded per row");
    }
}

#[test]
fn an_in_d_sd_stays_open_until_the_close() {
    // The header has no gate: nothing is reported at the header, the expectation
    // waits for the skip, and a skip that arrives later satisfies it.
    let flight = without_gate(in_sd_flight());
    let mut h = harness();
    h.feed_header(&flight);
    assert_clean(&h);
    assert_eq!(h.stats().compared, 1);
    h.send(ShadowMsg::TickerSkipped {
        rec: flight.header.rec,
        target: RID,
        reason: TickerSkipReason::NoLivePreview,
    });
    h.feed_later(&flight);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
}

#[test]
fn an_in_d_sd_with_neither_a_gate_nor_a_skip_is_an_effect_set_mismatch() {
    let mut h = harness();
    h.feed(&without_gate(in_sd_flight()));
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
    assert_eq!(h.stats().matched, 0);
}

#[test]
fn an_in_d_gate_and_a_skip_together_are_an_effect_set_mismatch() {
    let mut flight = in_sd_flight();
    skip_after_header(&mut flight, RID, TickerSkipReason::NoContext);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
    assert_eq!(h.stats().matched, 0);
}

#[test]
fn two_in_d_skips_are_an_effect_set_mismatch() {
    let mut flight = without_gate(in_sd_flight());
    skip_after_header(&mut flight, RID, TickerSkipReason::NoContext);
    skip_after_header(&mut flight, RID, TickerSkipReason::NoLivePreview);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn an_in_d_skip_for_the_wrong_recording_is_a_post_state_mismatch_like_a_gate() {
    let previous = RecordingId(RID.0 - 1);
    // The skip.
    for reason in TickerSkipReason::ALL {
        let mut flight = without_gate(in_sd_flight());
        skip_after_header(&mut flight, previous, reason);
        let mut h = harness();
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::PostState.code(),
            "{reason:?}"
        );
        assert_eq!(h.stats().matched, 0);
    }
    // The same wrong id on the gate is the same difference.
    let mut flight = in_sd_flight();
    let mut gate = *flight.header.in_d_tickers.get(0).unwrap();
    gate.id = previous;
    gate.registered = gate.expected();
    flight.header.in_d_tickers.clear();
    flight.header.in_d_tickers.push(gate);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn an_in_d_skip_through_the_producer_path_matches() {
    let mut flight = without_gate(in_sd_flight());
    skip_after_header(&mut flight, RID, TickerSkipReason::NoLivePreview);
    let transport = default_transport();
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    h.comparator.assert_conserved();
    drop(recorder);
}

// --------------------------------------------- observations sent as a mark

/// R101.1's `EM(starting)`, conditional on `P-spec-flowing`.
fn spec_flight(spec_flowing: bool) -> Flight {
    start(
        1,
        1,
        Live {
            spec_flowing,
            ..Live::default()
        },
    )
}

fn observe_before_cond(flight: &mut Flight, flowing: bool) {
    let at = flight
        .later
        .iter()
        .position(|msg| matches!(msg, ShadowMsg::Cond { .. }))
        .expect("the flight has a conditional witness");
    flight.later.insert(
        at,
        ShadowMsg::Observed {
            rec: flight.header.rec,
            fact: ObservedFact::SpeculativeFlowing(flowing),
        },
    );
}

#[test]
fn an_observed_value_overrides_the_header() {
    // The live path took `EM` (not flowing). The header said flowing, so alone it
    // is a conditional mismatch; the value `SPEC` actually read, sent as a mark
    // before the conditional, is the one the rule is judged against.
    let mut stale_header = spec_flight(false);
    stale_header.header.observed.clear();
    stale_header
        .header
        .add_observed(ObservedFact::SpeculativeFlowing(true));
    let mut h = harness();
    h.feed(&stale_header);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());

    let mut flight = stale_header.clone();
    observe_before_cond(&mut flight, false);
    let mut h = harness();
    h.feed(&flight);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
}

#[test]
fn the_latest_observed_value_wins() {
    let mut flight = spec_flight(false);
    observe_before_cond(&mut flight, true);
    observe_before_cond(&mut flight, false);
    // The second insert lands just before the conditional, after the first one.
    let mut h = harness();
    h.feed(&flight);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
}

#[test]
fn an_observed_value_that_disagrees_with_the_conditional_is_a_mismatch() {
    // The header agrees with the live path, but the value actually read says flowing.
    let mut flight = spec_flight(false);
    observe_before_cond(&mut flight, true);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn a_conditional_with_no_observation_at_all_is_diff_kind_7() {
    let mut flight = spec_flight(false);
    flight.header.observed.clear();
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Observation.code());
}

#[test]
fn an_observation_through_the_producer_path_is_used() {
    let mut flight = spec_flight(false);
    flight.header.observed.clear();
    observe_before_cond(&mut flight, false);
    let transport = default_transport();
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    // And the mark is one attempt on the same ring as every other witness.
    let full = local_transport(2);
    let rec = full.recorder();
    for _ in 0..4 {
        mark(&rec, Mark::Observed(ObservedFact::SpeculativeFlowing(true)));
    }
    assert!(full.lost() > 0);
    drop(recorder);
}

// ------------------------------------------------ keyboard and cancel mirrors

/// A comparator whose decision gets one more effect at `In`, so a row can be
/// exercised that the core has no `In` effect of that kind for (`MC` is `PostDT` in
/// every row today).
fn comparator_adding_in_effect(h: &Harness, effect: Effect) -> Comparator {
    let events = std::sync::Arc::clone(&h.events);
    Comparator::new(
        Box::new(move |core, input, now| {
            let mut decision = core.handle(input, now);
            decision
                .effects
                .push(Placed::new(Placement::In, effect.clone()));
            decision
        }),
        Box::new(move |event| events.lock().unwrap().push(event)),
        Box::new(|| 0),
    )
}

/// The flight with the same effect witnessed under D, and the field it wrote.
fn with_in_effect(mut flight: Flight, effect: Effect, mirror: Option<MirrorWitness>) -> Flight {
    assert!(flight
        .header
        .in_d_effects
        .push(WitnessedEffect::ran(effect)));
    if let Some(mirror) = mirror {
        assert!(flight.header.in_d_mirrors.push(mirror));
    }
    flight
}

fn kb_mirror(after: bool) -> MirrorWitness {
    MirrorWitness {
        field: MirrorField::KeyboardProcessing,
        before: MirrorValue::Flag(true),
        after: MirrorValue::Flag(after),
    }
}

fn mc_mirror(before: u64, after: u64) -> MirrorWitness {
    MirrorWitness {
        field: MirrorField::CancelledThrough,
        before: MirrorValue::Id(RecordingId(before)),
        after: MirrorValue::Id(RecordingId(after)),
    }
}

#[test]
fn an_in_d_keyboard_mirror_is_compared() {
    let effect = || Effect::KeyboardProcessing(false);
    // Match: the port left the flag at the decided value.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(
        short_idle(1, 1),
        effect(),
        Some(kb_mirror(false)),
    ));
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    // Mismatch: the port committed another value than the one decided.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(
        short_idle(1, 1),
        effect(),
        Some(kb_mirror(true)),
    ));
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
    // No mirror at all for an effect placed `In`: it is an extra/missing witness.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(short_idle(1, 1), effect(), None));
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn an_in_d_cancelled_through_mirror_is_compared() {
    let effect = || Effect::MarkCancelled(RecordingId(9));
    // Match: the mark is left at `max(before, id)`.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(
        short_idle(1, 1),
        effect(),
        Some(mc_mirror(3, 9)),
    ));
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    // `MC` never lowers the mark: a higher one stays.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(
        short_idle(1, 1),
        effect(),
        Some(mc_mirror(12, 12)),
    ));
    assert_clean(&h);
    // Mismatch: the mark was lowered, or not raised.
    for after in [3, 8] {
        let mut h = harness();
        h.comparator = comparator_adding_in_effect(&h, effect());
        h.feed(&with_in_effect(
            short_idle(1, 1),
            effect(),
            Some(mc_mirror(3, after)),
        ));
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::PostState.code(),
            "after {after}"
        );
    }
    // A mirror of the wrong field.
    let mut h = harness();
    h.comparator = comparator_adding_in_effect(&h, effect());
    h.feed(&with_in_effect(
        short_idle(1, 1),
        effect(),
        Some(kb_mirror(false)),
    ));
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn a_real_in_d_keyboard_flight_carries_and_is_checked_against_its_mirror() {
    // R202.3: a stop whose context is gone places `KB(false)` at `In`; the harness
    // writes the mirror into the header, and damaging it is `PostState`.
    let flight = flight(
        1,
        1,
        view(Status::Processing, RID),
        Input::StopContext(StopContextInput {
            token: Continuation {
                recording_id: RID,
                op: OperationId(77),
                kind: ContKind::StopContext,
            },
            op_status: OpStatus::Outstanding,
            rid: RID,
            context_present: false,
            interrupted_error: None,
        }),
    );
    assert!(
        flight.decision.effects.iter().any(|p| {
            p.at == Placement::In && matches!(p.effect, Effect::KeyboardProcessing(_))
        }),
        "R202.3 places KB at In"
    );
    let mut h = harness();
    h.feed(&flight);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());

    let mut damaged = flight.clone();
    let mirrors: Vec<MirrorWitness> = damaged.header.in_d_mirrors.iter().copied().collect();
    let at = mirrors
        .iter()
        .position(|m| m.field == MirrorField::KeyboardProcessing)
        .expect("the header carries the KB mirror");
    let mut wrong = mirrors[at];
    wrong.after = MirrorValue::Flag(true);
    damaged.header.in_d_mirrors.clear();
    for (i, mirror) in mirrors.into_iter().enumerate() {
        assert!(damaged
            .header
            .in_d_mirrors
            .push(if i == at { wrong } else { mirror }));
    }
    let mut h = harness();
    h.feed(&damaged);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

// ------------------------------------------------------------- a lost ticket

#[test]
fn a_ticket_lost_after_other_flights_sealed_is_a_loss_with_no_stranded_sequence() {
    // The #1163 guard takes `rec.ticket(site)` and drops it when a flight that
    // entered `In` is dropped unsealed. Other flights sealed their tickets after it,
    // so their headers wait behind the missing sequence number.
    let transport = default_transport();
    let mut h = harness();
    let flight = short_idle(0, 0);
    let site = flight.header.site_id;

    let lost = transport.recorder();
    lost.enter_pre();
    let ticket = lost.ticket(site); // the lowest sequence number
    let first = transport.recorder();
    submit_header(&first, &flight);
    send_later(&first, &flight);
    let second = transport.recorder();
    submit_header(&second, &flight);
    send_later(&second, &flight);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().compared, 0, "both wait for the missing sequence");

    drop(ticket); // the flight unwound: its ticket is reported as a loss
    h.comparator.drain(&transport);
    assert_eq!(transport.lost(), 1);
    assert_eq!(h.stats().abandoned_by_loss, 2, "{:?}", h.stats());
    assert_eq!(h.stats().matched, 0);
    h.comparator.assert_conserved();

    // The sequence is not stranded: the next flight is compared.
    let third = transport.recorder();
    submit_header(&third, &flight);
    send_later(&third, &flight);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    assert_eq!(h.stats().double_close_suppressed, 0);
    h.comparator.assert_conserved();
    // The unsealed flight's recorder is not a `PreAbandoned`: a ticket was taken.
    drop(lost);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_pre_d, 0, "{:?}", h.stats());
    h.comparator.assert_conserved();
    drop((first, second, third));
}
