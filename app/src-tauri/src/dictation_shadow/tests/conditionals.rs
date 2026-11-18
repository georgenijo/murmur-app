//! Shell-conditional effects: each needs exactly one witness (D-S2c-1 d).

use std::sync::Arc;

use murmur_dictation_core::effects::{Effect, Placement};

use super::harness::*;
use crate::dictation_shadow::compare::Comparator;
use crate::dictation_shadow::types::{
    CondWitness, DiffKind, ObservedFact, PredId, ShadowMsg, WitnessedEffect,
};

fn em_witnessed(flight: &Flight) -> bool {
    flight
        .later
        .iter()
        .any(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::EmitStatus { .. })))
}

#[test]
fn shadow_conditional_compared_as_predicate() {
    // `EM(starting)` with `P-spec-flowing`: a skipped one while a speculative
    // capture flows, and a taken one while it does not, both compare clean.
    for spec_flowing in [true, false] {
        let mut h = harness();
        let flight = start(
            1,
            1,
            Live {
                spec_flowing,
                ..Live::default()
            },
        );
        assert_eq!(
            em_witnessed(&flight),
            !spec_flowing,
            "a skipped conditional is not an effect"
        );
        h.feed(&flight);
        assert_clean(&h);
        assert_eq!(h.stats().matched, 1, "spec_flowing={spec_flowing}");
    }
    // Flipping the predicate is `diff_kind 9`: the effect-time observation says a
    // capture was flowing but the live path took the emit, and the reverse.
    for observed_flowing in [true, false] {
        let mut h = harness();
        let mut flight = start(
            1,
            1,
            Live {
                spec_flowing: !observed_flowing,
                ..Live::default()
            },
        );
        for msg in &mut flight.later {
            if let ShadowMsg::Observed { fact, .. } = msg {
                *fact = ObservedFact::SpeculativeFlowing(observed_flowing);
            }
        }
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Conditional.code(),
            "observed {observed_flowing}"
        );
    }
    // A different predicate id is the same difference.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    for msg in &mut flight.later {
        if let ShadowMsg::Cond { pred, .. } = msg {
            *pred = PredId::EmitOk(0);
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn shadow_paired_presentation_follows_its_emit_result() {
    // `PRES` runs only when the paired emit returned `Ok`.
    for emit_ok in [true, false] {
        let mut h = harness();
        let flight = stalled(
            1,
            1,
            Live {
                emit_ok,
                ..Live::default()
            },
        );
        let pres_witnessed = flight
            .later
            .iter()
            .any(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::Presentation { .. })));
        assert_eq!(pres_witnessed, emit_ok);
        h.feed(&flight);
        assert_clean(&h);
    }
    // The emit failed but the live path presented anyway: a conditional mismatch.
    let mut h = harness();
    let mut flight = stalled(
        1,
        1,
        Live {
            emit_ok: false,
            ..Live::default()
        },
    );
    for msg in &mut flight.later {
        if let ShadowMsg::Cond { taken, .. } = msg {
            *taken = true;
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn shadow_conditional_that_ran_after_being_skipped_is_conditional() {
    let mut h = harness();
    let mut flight = start(
        1,
        1,
        Live {
            spec_flowing: true,
            ..Live::default()
        },
    );
    let rec = flight.header.rec;
    // The live path said "skipped" and then ran the effect anyway.
    let idx = flight
        .decision
        .effects
        .iter()
        .position(|p| matches!(p.effect, Effect::EmitStatus { .. }))
        .unwrap();
    let after_cond = flight
        .later
        .iter()
        .position(|m| matches!(m, ShadowMsg::Cond { .. }))
        .expect("the skipped conditional is reported")
        + 1;
    flight.later.insert(
        after_cond,
        ShadowMsg::Witness {
            rec,
            idx: u16::try_from(idx).unwrap(),
            placement: Placement::PostDT,
            effect: WitnessedEffect::emitted(flight.decision.effects[idx].effect.clone(), true),
        },
    );
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn a_removed_conditional_witness_with_a_retained_emit_is_effectset() {
    // Review fix (Sol 6.2 major 4): the conditional's witness is missing, the emit it
    // guards ran. The old comparison accepted that; a completed close needs exactly
    // one witness per conditional.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    flight
        .later
        .retain(|m| !matches!(m, ShadowMsg::Cond { .. }));
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_second_witness_of_one_conditional_is_conditional() {
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    let cond = flight
        .later
        .iter()
        .find(|m| matches!(m, ShadowMsg::Cond { .. }))
        .cloned()
        .unwrap();
    let at = flight
        .later
        .iter()
        .position(|m| matches!(m, ShadowMsg::Cond { .. }))
        .unwrap();
    flight.later.insert(at, cond);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn a_missing_speculative_flow_observation_is_observation() {
    // Review fix (Sol 6.2 major 4, amendment 2): `P-spec-flowing` needs the effect-time
    // read; without it the conditional cannot be checked, and that is `Observation`
    // (7), not a pass, whatever the header observed.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    flight
        .later
        .retain(|m| !matches!(m, ShadowMsg::Observed { .. }));
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Observation.code());
}

#[test]
fn a_header_conditional_without_a_witness_is_effectset() {
    // The `Pre`/`In` lists are complete when the header is sent: a conditional that
    // has no witness there is a missing witness.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    // R101.15's conditional is a `PostDT` one; make a header one by removing the
    // `In`-placed conditionals of a flight that has them.
    flight.header.in_d_conditionals.clear();
    h.feed(&flight);
    assert_clean(&h);

    let mut h = harness();
    let mut paired = stalled(1, 1, Live::default());
    paired
        .later
        .retain(|m| !matches!(m, ShadowMsg::Cond { .. }));
    h.feed(&paired);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_header_conditional_whose_paired_emit_has_no_result_is_observation() {
    // The `Pre`/`In` lists are judged when the header is applied. A `PRES` paired with
    // an `In` emit needs that emit's own result; a witness that only says the emit
    // "ran" cannot tell if it succeeded, and that is `Observation` (7), not a pass.
    // No shipped row pairs in `In`, so the decision places `R214.1`'s pair there.
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let comparator = Comparator::new(
        Box::new(|core, input, now| {
            let mut decision = core.handle(input, now);
            for placed in &mut decision.effects {
                placed.at = Placement::In;
            }
            decision
        }),
        Box::new(move |event| sink.lock().unwrap().push(event)),
        Box::new(|| 0),
    );
    let mut h = Harness {
        comparator,
        events,
        clock: Arc::default(),
    };
    let mut flight = stalled(1, 1, Live::default());
    let moved: Vec<ShadowMsg> = std::mem::take(&mut flight.later);
    for msg in moved {
        match msg {
            ShadowMsg::Witness { effect, .. } => {
                let ran = WitnessedEffect::ran(effect.effect);
                flight.header.add_in_effect(ran);
            }
            ShadowMsg::Cond { pred, taken, .. } => {
                flight
                    .header
                    .add_in_conditional(CondWitness { pred, taken });
            }
            other => flight.later.push(other),
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Observation.code());
}
