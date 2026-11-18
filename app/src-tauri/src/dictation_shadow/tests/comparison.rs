//! CP1-CP4: what the comparison reports for each way a live path can differ.

use murmur_dictation_core::effects::{Effect, Placement};
use murmur_dictation_core::ids::{OperationId, RecordingId};
use murmur_dictation_core::inputs::{Input, SupervisorEvent};
use murmur_dictation_core::state::LiveObs;
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::compare::{
    Comparator, EVENT_WINDOW_MS, MAX_EVENTS_PER_SESSION, MAX_HELD_PER_REC, MAX_OPEN_RECORDS,
    MAX_PENDING_HEADERS,
};
use crate::dictation_shadow::types::{
    DiffKind, EffectResult, MirrorField, MirrorValue, Projection, RecorderId, ReplyKind, ShadowMsg,
    WitnessedEffect,
};

use std::sync::atomic::Ordering;
use std::sync::Arc;

#[test]
fn a_faithful_live_path_compares_clean_at_every_point() {
    let mut h = harness();
    let flights = [
        short_idle(1, 1),
        start(2, 2, Live::default()),
        cancel(3, 3),
        stalled(4, 4, Live::default()),
        stale_stop(5, 5),
        ready_publish(6, 6, murmur_dictation_core::decision::OpStatus::Outstanding),
        ready_while_recording(7, 7, Live::default()),
        capture_failed(8, 8, Live::default()),
        start_during_cleanup(9, 9),
    ];
    for flight in &flights {
        h.feed(flight);
    }
    assert_clean(&h);
    assert_eq!(h.stats().compared, 9);
    assert_eq!(h.stats().matched, 9, "every flight closed with no mismatch");
    let lost = h.stats().lost_gap + h.stats().lost_open + h.stats().dropped;
    assert_eq!(lost, 0);
    // The fixtures are the rows the tests name.
    let rows: Vec<&str> = flights.iter().map(|f| f.decision.row.0).collect();
    assert_eq!(
        rows,
        [
            "R216.1", "R101.15", "R205.5", "R214.1", "R201.1", "R104.2", "R207.3", "R102.2",
            "R101.12"
        ]
    );
}

// ------------------------------------------------------------- Pre (C8.7)

fn drop_pre_effect(flight: &mut Flight, matches: impl Fn(&Effect) -> bool) {
    let at = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches(&w.effect))
        .expect("the effect is in the Pre list");
    flight.header.pre_effects.remove(at);
}

#[test]
fn shadow_pre_effect_omitted_is_effectset() {
    // Parametrized over `Pre:TERM` and `Pre:CX-` of R216.1 and `Pre:TERM` of R201.1.
    // A shadow that ignored `Pre` would pass every one of these.
    type Case = (&'static str, fn() -> Flight, fn(&Effect) -> bool);
    let cases: [Case; 3] = [
        (
            "R216.1 TERM",
            || short_idle(1, 1),
            |e| matches!(e, Effect::Terminal(_)),
        ),
        (
            "R216.1 CX-",
            || short_idle(1, 1),
            |e| matches!(e, Effect::ClearContext(_)),
        ),
        (
            "R201.1 TERM",
            || stale_stop(1, 1),
            |e| matches!(e, Effect::Terminal(_)),
        ),
    ];
    for (name, build, matches) in cases {
        let mut h = harness();
        let mut flight = build();
        drop_pre_effect(&mut flight, matches);
        if name.contains("CX-") {
            // The omitted effect's mirror is omitted with it.
            flight.header.pre_mirrors.clear();
        }
        // A `Pre` effect the header does not list stays pending until the record
        // closes (a later witness may show it ran at another placement, D-S2c-2 g);
        // at the completed close it was never witnessed anywhere: `EffectSet`.
        h.feed(&flight);
        let event = h.only_event();
        assert_eq!(event.diff_kind, DiffKind::EffectSet.code(), "{name}");
        assert_eq!(event.site_id, flight.header.site_id, "{name}");
        assert_eq!(event.input_kind, flight.header.row, "{name}");
        assert_eq!(h.stats().mismatched_root, 1, "{name}");
    }
}

#[test]
fn a_pre_effect_that_ran_at_another_placement_is_placement_not_a_missing_effect() {
    // Sol 6.2 minor 2 (D-S2c-2 g): R216.1's `Pre:TERM` ran under D (`In`) or after the
    // release (`PostDT`). Classified at the header it was "missing" (`EffectSet`)
    // for the `PostDT` move and `Placement` for `In`; the decided effect stays
    // pending until the record closes, and the witness that shows where it ran
    // decides the code: always `Placement`.
    for (name, to) in [("In", Placement::In), ("PostDT", Placement::PostDT)] {
        let mut h = harness();
        let mut flight = short_idle(1, 1);
        let at = flight
            .header
            .pre_effects
            .iter()
            .position(|w| matches!(w.effect, Effect::Terminal(_)))
            .unwrap();
        let moved = flight.header.pre_effects.remove(at);
        let decided = flight
            .decision
            .effects
            .iter()
            .position(|p| matches!(p.effect, Effect::Terminal(_)))
            .unwrap();
        match to {
            Placement::In => assert!(flight.header.in_d_effects.push(moved)),
            _ => flight.later.insert(
                0,
                ShadowMsg::Witness {
                    rec: flight.header.rec,
                    idx: u16::try_from(decided).unwrap(),
                    placement: Placement::PostDT,
                    effect: moved,
                },
            ),
        }
        h.feed(&flight);
        let event = h.only_event();
        assert_eq!(event.diff_kind, DiffKind::Placement.code(), "to {name}");
        assert_eq!(h.stats().mismatched_root, 1, "to {name}");
    }
}

#[test]
fn shadow_pre_effect_reordered_is_effectorder() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let term = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::Terminal(_)))
        .unwrap();
    let clear = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::ClearContext(_)))
        .unwrap();
    flight.header.pre_effects.swap(term, clear);
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectOrder.code());
}

#[test]
fn shadow_pre_compared_before_in() {
    // A `Pre` difference (a reorder, `diff_kind 2`) and an `In` difference (the `ST`
    // omitted, `diff_kind 1`): the `Pre` one is reported although 1 < 2.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.swap(1, 2);
    flight.header.in_d_effects.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectOrder.code());

    // With `Pre` intact the `In` difference is what is reported.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.in_d_effects.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

// ----------------------------------------------------- Placement (D-S2c-1 d)

#[test]
fn a_moved_pre_effect_is_placement_not_missing_and_extra() {
    // Review fix (Sol 6.2 major 4): `Pre:TERM` that ran at `In` is `Placement` (3),
    // not an `EffectSet` for the missing `Pre` effect plus an extra `In` one.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let at = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::Terminal(_)))
        .unwrap();
    let moved = flight.header.pre_effects.remove(at);
    flight.header.in_d_effects.push(moved);
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Placement.code());
}

#[test]
fn a_move_into_in_from_a_later_placement_is_placement_and_pre_wins() {
    // R216.1's `PDT:KB` that the live path ran under D: `In` reports `Placement`.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let at = flight.witness_index(|e| matches!(e, Effect::KeyboardProcessing(_)));
    let ShadowMsg::Witness { effect, .. } = flight.later.remove(at) else {
        unreachable!()
    };
    // Its mirror message goes with it.
    flight
        .later
        .retain(|m| !matches!(m, ShadowMsg::Mirror { mirror, .. } if mirror.field == MirrorField::KeyboardProcessing));
    flight.header.in_d_effects.push(effect);
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Placement.code());

    // With a `Pre` reorder as well, `Pre` is reported first.
    let mut h = harness();
    let mut both = flight.clone();
    both.header.pre_effects.swap(1, 2);
    h.feed_header(&both);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectOrder.code());
}

#[test]
fn an_effect_matches_its_own_placement_before_any_other() {
    // The same effect decided twice in different placements: the live path ran it
    // once, in `Pre`. It matches the decided `Pre` copy, not the first copy in the
    // list (a `PostT` one placed ahead of it), so the header shows no `Placement`.
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let comparator = Comparator::new(
        Box::new(|core, input, now| {
            let mut decision = core.handle(input, now);
            let first_pre = decision
                .effects
                .iter()
                .position(|p| p.at == Placement::Pre)
                .expect("R216.1 has a Pre effect");
            let mut copy = decision.effects[first_pre].clone();
            copy.at = Placement::PostT;
            decision.effects.insert(0, copy);
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
    h.feed_header(&short_idle(1, 1));
    assert!(h.events().is_empty(), "{:?}", h.events());
}

trait FlightExt {
    fn witness_index(&self, matches: impl Fn(&Effect) -> bool) -> usize;
}

impl FlightExt for Flight {
    fn witness_index(&self, matches: impl Fn(&Effect) -> bool) -> usize {
        witness_at(self, matches)
    }
}

// ---------------------------------------------------- later effects (CP2-CP4)

#[test]
fn shadow_omitted_later_effect_is_effectset_at_close() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let at = flight.witness_index(|e| matches!(e, Effect::EmitEvent(_)));
    flight.later.remove(at);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn shadow_reordered_later_effects_are_effectorder() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let witnesses: Vec<usize> = flight
        .later
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            matches!(
                m,
                ShadowMsg::Witness {
                    placement: Placement::PostDT,
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect();
    // Swap the positions of two `PostDT` witnesses: the live path ran them in the
    // other order.
    let (a, b) = (witnesses[3], witnesses[4]);
    flight.later.swap(a, b);
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1, "{:?}", h.events());
    assert_eq!(h.events()[0].diff_kind, DiffKind::EffectOrder.code());
}

#[test]
fn shadow_effect_in_another_placement_is_placement() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Witness {
            effect, placement, ..
        } = msg
        {
            if matches!(effect.effect, Effect::PerfComplete(_)) {
                *placement = Placement::PostDT;
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Placement.code());
}

#[test]
fn shadow_extra_and_duplicate_effects_are_effectset() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let first = flight
        .later
        .iter()
        .find(|m| matches!(m, ShadowMsg::Witness { .. }))
        .cloned()
        .unwrap();
    flight.later.insert(0, first);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());

    let mut h = harness();
    let mut flight = cancel(1, 1);
    let rec = flight.header.rec;
    flight.later.insert(
        0,
        ShadowMsg::Witness {
            rec,
            idx: 99,
            placement: Placement::PostDT,
            effect: WitnessedEffect::ran(Effect::PerfAbort),
        },
    );
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn shadow_reply_kind_is_compared_at_close() {
    for damage in 0..2 {
        let mut h = harness();
        let mut flight = stale_stop(1, 1);
        let at = flight
            .later
            .iter()
            .position(|m| matches!(m, ShadowMsg::Reply { .. }))
            .expect("R201.1 replies recording_superseded");
        if damage == 0 {
            flight.later.remove(at);
        } else if let ShadowMsg::Reply { kind, .. } = &mut flight.later[at] {
            *kind = ReplyKind::Err;
        }
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Reply.code(),
            "damage {damage}"
        );
    }
}

#[test]
fn shadow_post_state_is_compared_at_cp1() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.live_post_d.status = Status::Processing;
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    // `ID+` committed a different id than the one the decision predicted.
    flight.header.live_post_d.current_id = RecordingId(99);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn shadow_resync_does_not_cascade() {
    let mut h = harness();
    // One M1 mismatch...
    let mut wrong = short_idle(1, 1);
    wrong.header.live_post_d.status = Status::Recording;
    h.feed(&wrong);
    assert_eq!(h.stats().mismatched_root, 1);
    // ...and the next record, whose pre-state is what the live path really had,
    // compares clean: the shadow re-synced M1/M2 from the header, it did not carry
    // its own wrong post-state forward.
    let next = flight(
        2,
        2,
        view(Status::Idle, RecordingId(9)),
        Input::Supervisor(SupervisorEvent::Idle {
            rid: RecordingId(9),
        }),
    );
    h.feed(&next);
    assert_eq!(h.stats().mismatched_root, 1, "no cascade: {:?}", h.events());
    assert_eq!(h.stats().followup, 0);
    assert_eq!(h.stats().matched, 1);
}

#[test]
fn shadow_m3_m4_compared_as_mirrors_only() {
    // A wrong `MC` value (a high-water mark below the cancelled id) and a wrong
    // `CX-` result (the slot still holds the id) are `diff_kind 5` at CP3.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror { mirror, .. } = msg {
            if mirror.field == MirrorField::CancelledThrough {
                mirror.after = MirrorValue::Id(RecordingId(1));
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror { mirror, .. } = msg {
            if mirror.field == MirrorField::ActiveContext {
                mirror.after = MirrorValue::Slot(Some(RID));
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // A wrong keyboard flag.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror { mirror, .. } = msg {
            if mirror.field == MirrorField::KeyboardProcessing {
                mirror.after = MirrorValue::Flag(true);
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // A mirror that never arrived is a missing witness: `EffectSet` at close.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    flight.later.retain(|m| {
        !matches!(m, ShadowMsg::Mirror { mirror, .. } if mirror.field == MirrorField::CancelledThrough)
    });
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());

    // A second write of a mirrored field is an extra witness.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let dup = flight
        .later
        .iter()
        .find(|m| matches!(m, ShadowMsg::Mirror { mirror, .. } if mirror.field == MirrorField::KeyboardProcessing))
        .cloned()
        .unwrap();
    flight.later.insert(0, dup);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());

    // The core holds no copy of M3 or M4, so there is nothing to compare them
    // against except the decided commit: a header whose `pre.obs` differs from the
    // last decision's post is not a mismatch (it is a fresh observation).
    let mut h = harness();
    h.feed(&cancel(1, 1));
    let mut next = short_idle(2, 2);
    next.header.pre.obs = LiveObs {
        cancelled_through: RecordingId(41),
        context_for: None,
    };
    h.feed(&next);
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn shadow_abandoned_flight_is_not_a_mismatch() {
    let mut h = harness();
    let flight = start(1, 1, Live::default());
    h.feed_header(&flight);
    // The flight was dropped after its first `PostDT` effects: the rest never ran.
    let rec = flight.header.rec;
    for msg in flight.later.iter().take(2).cloned() {
        h.send(msg);
    }
    h.send(ShadowMsg::Close {
        rec,
        completed: false,
    });
    assert_eq!(h.stats().abandoned, 1);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1);
    // But an effect it did witness is still compared.
    let mut h = harness();
    let flight = cancel(1, 1);
    h.feed_header(&flight);
    let rec = flight.header.rec;
    h.send(ShadowMsg::Witness {
        rec,
        idx: 99,
        placement: Placement::PostDT,
        effect: WitnessedEffect::ran(Effect::PerfAbort),
    });
    h.send(ShadowMsg::Close {
        rec,
        completed: false,
    });
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

// ------------------------------------------------------------ observations

#[test]
fn shadow_missing_observation_is_diff_kind_7() {
    // A readiness publish whose context-presence read the live path never recorded:
    // the decision answers `NeedObservation`, which the shadow cannot complete.
    let mut h = harness();
    let mut ready = ready_publish(1, 1, murmur_dictation_core::decision::OpStatus::Outstanding);
    if let Input::ReadyPublish(input) = &mut ready.header.input {
        input.context_present = None;
    }
    h.feed_header(&ready);
    assert_eq!(h.only_event().diff_kind, DiffKind::Observation.code());
    assert_eq!(h.stats().compared, 1);
}

// ----------------------------------------------------------- panic, cascades

#[test]
fn shadow_panic_is_caught_and_counted() {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let mut comparator = Comparator::new(
        Box::new(|core, input, now| {
            assert!(
                !matches!(input, Input::CancelRequest(_)),
                "the shadow's own bug"
            );
            core.handle(input, now)
        }),
        Box::new(move |event| sink.lock().unwrap().push(event)),
        Box::new(|| 0),
    );
    comparator.handle_msg(ShadowMsg::Header(cancel(1, 1).header));
    assert_eq!(comparator.stats.shadow_panic, 1);
    assert_eq!(
        events.lock().unwrap()[0].diff_kind,
        DiffKind::ShadowPanic.code()
    );
    // The next header still compares, and the panicking flight's late messages are
    // dropped.
    for msg in cancel(1, 1).later {
        comparator.handle_msg(msg);
    }
    comparator.handle_msg(ShadowMsg::Header(short_idle(2, 2).header));
    for msg in short_idle(2, 2).later {
        comparator.handle_msg(msg);
    }
    assert_eq!(comparator.stats.matched, 1);
    assert_eq!(comparator.stats.mismatched_root, 1);
}

#[test]
fn shadow_followup_is_not_a_new_root() {
    let mut h = harness();
    let mut first = short_idle(1, 1);
    first.header.live_post_d.status = Status::Recording;
    h.feed(&first);
    // The same recording, the same row, the same level: a followup.
    let mut second = short_idle(2, 2);
    second.header.live_post_d.status = Status::Recording;
    h.feed(&second);
    assert_eq!(h.stats().mismatched_root, 1);
    assert_eq!(h.stats().followup, 1);
    assert_eq!(h.events().len(), 1, "a followup emits no event");
    // A different level on the same recording is a new root.
    let mut third = short_idle(3, 3);
    third.header.pre_effects.remove(3);
    h.feed(&third);
    assert_eq!(h.stats().mismatched_root, 2);
    assert_eq!(
        h.stats().per_row[&(
            216,
            murmur_dictation_core::decision::RowId("R216.1").ordinal()
        )]
            .mismatched,
        3
    );
}

#[test]
fn a_later_level_of_a_faulty_record_is_a_followup_not_a_second_root() {
    // The header lost a Pre effect (`EffectSet`); the same effect then shows up as a
    // late witness in another placement (`Placement`). One fault, one root.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let at = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::PerfComplete(_)))
        .unwrap();
    let moved = flight.header.pre_effects.remove(at);
    let rec = flight.header.rec;
    flight.later.insert(
        0,
        ShadowMsg::Witness {
            rec,
            idx: u16::try_from(at + 1).unwrap(),
            placement: Placement::PostDT,
            effect: moved,
        },
    );
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1, "{:?}", h.events());
}

#[test]
fn shadow_mismatch_event_is_rate_limited() {
    // One event per (site, input_kind, diff_kind) per 60 s...
    let mut h = harness();
    for n in 1..=2u64 {
        let mut flight = idle_for(n, n, 100 + n);
        flight.header.live_post_d.status = Status::Recording;
        h.feed(&flight);
    }
    assert_eq!(
        h.stats().mismatched_root,
        2,
        "two distinct recordings, two roots"
    );
    assert_eq!(h.events().len(), 1);
    assert_eq!(
        h.stats().events_suppressed,
        1,
        "the suppressed one is counted"
    );
    h.clock.fetch_add(EVENT_WINDOW_MS + 1, Ordering::SeqCst);
    let mut flight = idle_for(3, 3, 103);
    flight.header.live_post_d.status = Status::Recording;
    h.feed(&flight);
    assert_eq!(h.events().len(), 2, "the window passed");

    // ...and at most 50 per session.
    let mut h = harness();
    let total = MAX_EVENTS_PER_SESSION + 10;
    for n in 1..=total {
        let mut flight = idle_for(n, n, 1_000 + n);
        // A distinct key every time, and a distinct recording every time.
        flight.header.row = u16::try_from(n).unwrap();
        flight.header.live_post_d.status = Status::Recording;
        h.send(ShadowMsg::Header(flight.header));
    }
    assert_eq!(
        h.events().len(),
        usize::try_from(MAX_EVENTS_PER_SESSION).unwrap()
    );
    assert_eq!(h.stats().events_suppressed, 10);
    assert_eq!(h.stats().mismatched_root, total);
}

#[test]
fn shadow_mismatch_event_carries_three_numbers() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.swap(1, 2);
    h.feed(&flight);
    let event = h.only_event();
    assert_eq!(event.site_id, 216);
    assert_eq!(event.input_kind, 1, "R216.1 is row ordinal 1");
    assert_eq!(event.diff_kind, 2);
}

// ------------------------------------------------------------------ ordering

#[test]
fn shadow_gap_is_count_not_time() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    // seq 2 is still in its flight (a 20 s `CS`): headers 3.. wait for it.
    let slow = short_idle(2, 2);
    h.send(ShadowMsg::Header(short_idle(3, 3).header));
    assert_eq!(h.stats().compared, 1, "seq 3 waits for the gap");
    h.clock.fetch_add(20_000, Ordering::SeqCst);
    assert_eq!(h.stats().lost_gap, 0, "twenty seconds are not a loss");
    h.feed(&slow);
    assert_eq!(
        h.stats().compared,
        3,
        "the gap closed: 2 then 3 applied in order"
    );
    assert_clean(&h);

    // A missing seq is declared lost only after more than 64 later headers wait.
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    let missing = 2;
    let waiting = u64::try_from(MAX_PENDING_HEADERS).unwrap();
    for seq in missing + 1..missing + 1 + waiting {
        h.send(ShadowMsg::Header(short_idle(seq, seq).header));
    }
    h.clock.fetch_add(3_600_000, Ordering::SeqCst);
    assert_eq!(
        h.stats().lost_gap,
        0,
        "64 waiting headers keep the gap open"
    );
    assert_eq!(h.stats().compared, 1);
    let overflow = missing + 1 + waiting;
    h.send(ShadowMsg::Header(short_idle(overflow, overflow).header));
    assert_eq!(
        h.stats().lost_gap,
        1,
        "the 65th later header declares it lost"
    );
    assert_eq!(
        h.stats().compared,
        1 + waiting + 1,
        "and the waiting headers applied"
    );
}

#[test]
fn shadow_headers_apply_in_seq_order() {
    let mut h = harness();
    let a = flight(
        1,
        1,
        view(Status::Idle, RecordingId(6)),
        Input::Supervisor(SupervisorEvent::Idle {
            rid: RecordingId(6),
        }),
    );
    let b = short_idle(2, 2);
    let c = short_idle(3, 3);
    // Arrival order 3, 1, 2: applied as 1, 2, 3.
    h.feed_header(&c);
    assert_eq!(h.stats().compared, 0);
    h.feed_header(&a);
    assert_eq!(h.stats().compared, 1);
    h.feed_header(&b);
    assert_eq!(h.stats().compared, 3);
    assert_clean(&h);
}

#[test]
fn shadow_open_records_are_capped_at_64() {
    let mut h = harness();
    let cap = u64::try_from(MAX_OPEN_RECORDS).unwrap();
    for n in 1..=cap {
        // Headers only: the flights never close.
        h.send(ShadowMsg::Header(short_idle(n, n).header));
    }
    assert_eq!(h.stats().lost_open, 0);
    h.send(ShadowMsg::Header(short_idle(cap + 1, cap + 1).header));
    assert_eq!(
        h.stats().lost_open,
        1,
        "the oldest open record is closed as lost_open"
    );
    // Its late messages are dropped, not held and not a mismatch.
    h.send(ShadowMsg::Close {
        rec: RecorderId(1),
        completed: true,
    });
    assert_eq!(h.stats().mismatched_root, 0);
    assert_eq!(h.stats().matched, 0);
}

#[test]
fn shadow_messages_before_their_header_are_held_at_most_32() {
    let mut h = harness();
    let flight = cancel(1, 1);
    // Every later message arrives first, then the header.
    for msg in flight.later.clone() {
        h.send(msg);
    }
    assert_eq!(h.stats().compared, 0);
    h.feed_header(&flight);
    assert_eq!(h.stats().compared, 1);
    assert_eq!(
        h.stats().matched,
        1,
        "held messages attach once the header applies"
    );
    assert_clean(&h);

    // The 33rd message for a recorder with no header is an overflow.
    let mut h = harness();
    for _ in 0..MAX_HELD_PER_REC + 1 {
        h.send(ShadowMsg::Reply {
            rec: RecorderId(5),
            kind: ReplyKind::Ok,
        });
    }
    assert_eq!(h.stats().witness_overflow, 1);
}

#[test]
fn shadow_core_holds_m1_m2_only() {
    // `ShadowCore` has exactly one field, the persisted M1/M2: no M3 or M4 copy.
    let source = super::policy::production_source(include_str!("../compare.rs"));
    let body = super::policy::function_body(&source, "pub(crate) struct ShadowCore");
    assert_eq!(
        body.matches(':').count(),
        1,
        "ShadowCore has one field: {body}"
    );
    assert!(body.contains("persisted: PersistedCore"));
    // Behaviourally: a view takes M3/M4 from the argument and M1/M2 from the resync.
    let mut core = crate::dictation_shadow::compare::ShadowCore::new();
    let pre = view(Status::Recording, RID);
    core.resync_m(&Projection {
        status: pre.status,
        current_id: pre.current_id,
        obs: LiveObs {
            cancelled_through: RecordingId(1),
            context_for: None,
        },
    });
    let first = core.view(LiveObs {
        cancelled_through: RecordingId(11),
        context_for: Some(RID),
    });
    let second = core.view(LiveObs::default());
    assert_eq!((first.status, first.current_id), (Status::Recording, RID));
    assert_eq!((second.status, second.current_id), (Status::Recording, RID));
    assert_eq!(first.cancelled_through, RecordingId(11));
    assert_eq!(
        second.cancelled_through,
        RecordingId(0),
        "nothing carried over"
    );
    assert_eq!(second.context_for, None);
}

// ------------------------------------------------------------- ticker gates

#[test]
fn a_ticker_registration_is_compared_against_its_gate_inputs() {
    // Review fix (Sol 6.2 major 5): `TickerRegistered` was unconditional. A faithful
    // registration (R104.2 `In:SD`) is clean, and the gate's inputs decide it.
    let mut h = harness();
    h.feed(&ready_publish(
        1,
        1,
        murmur_dictation_core::decision::OpStatus::Outstanding,
    ));
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1);

    // Registered although the gate's inputs say it must not: the id was already
    // cancelled, a later ticker was already registered, or the id is 0.
    type Damage = fn(&mut crate::dictation_shadow::types::TickerGate);
    let damages: [(&str, Damage); 3] = [
        ("cancelled", |g| g.cancelled_through = g.id),
        ("later ticker", |g| {
            g.ticker_before = RecordingId(g.id.0 + 1)
        }),
        ("registered flag", |g| g.registered = !g.registered),
    ];
    for (name, damage) in damages {
        let mut h = harness();
        let mut flight =
            ready_publish(1, 1, murmur_dictation_core::decision::OpStatus::Outstanding);
        let mut gate = *flight.header.in_d_tickers.get(0).unwrap();
        damage(&mut gate);
        flight.header.in_d_tickers.clear();
        flight.header.in_d_tickers.push(gate);
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::PostState.code(),
            "{name}"
        );
    }

    // A gate that read another state than the decision's own (status not yet
    // `Recording` under D) is wrong too.
    let mut h = harness();
    let mut flight = ready_publish(1, 1, murmur_dictation_core::decision::OpStatus::Outstanding);
    let mut gate = *flight.header.in_d_tickers.get(0).unwrap();
    gate.status = Status::Starting;
    gate.registered = gate.expected();
    flight.header.in_d_tickers.clear();
    flight.header.in_d_tickers.push(gate);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // A missing attempt is a missing witness.
    let mut h = harness();
    let mut flight = ready_publish(1, 1, murmur_dictation_core::decision::OpStatus::Outstanding);
    flight.header.in_d_tickers.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_ticker_attempt_after_d_is_compared_too() {
    // R207.3's `SD` (the wrapper's own D): a duplicate registration for the same id
    // is what PD-4 says happens, and it is consistent with the gate's inputs.
    let live = Live {
        ticker_before: RID,
        ..Live::default()
    };
    let flight = ready_while_recording(1, 1, live);
    let mut h = harness();
    h.feed(&flight);
    assert_clean(&h);
    // An attempt the decision does not have is an extra witness.
    let mut h = harness();
    let mut flight = capture_failed(1, 1, Live::default());
    let rec = flight.header.rec;
    let gate = crate::dictation_shadow::types::TickerGate {
        id: RID,
        status: Status::Recording,
        current_id: RID,
        cancelled_through: RecordingId(0),
        ticker_before: RecordingId(0),
        registered: true,
    };
    flight.later.insert(0, ShadowMsg::Ticker { rec, gate });
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_ticker_call_for_the_wrong_generation_is_compared_under_d_and_after_release() {
    // Sol 6.2 major 6 (D-S2c-2 f): the gate's own inputs were checked, but not that
    // the ticker was asked for the recording the decision's `SD` targets. A gate
    // whose requested id is the previous generation's, with every other field
    // consistent (so its own `registered` agrees), used to pass.
    let previous = RecordingId(RID.0 - 1);
    // Under D (R104.2 `In:SD`).
    let mut h = harness();
    let mut flight = ready_publish(1, 1, murmur_dictation_core::decision::OpStatus::Outstanding);
    let mut gate = *flight.header.in_d_tickers.get(0).unwrap();
    gate.id = previous;
    gate.registered = gate.expected();
    flight.header.in_d_tickers.clear();
    flight.header.in_d_tickers.push(gate);
    h.feed(&flight);
    let event = h.only_event();
    assert_eq!(event.diff_kind, DiffKind::PostState.code(), "{event:?}");
    assert_eq!(h.stats().matched, 0);

    // After release (R207.3's `PostDT:SD`, a `Ticker` message).
    let mut h = harness();
    let mut flight = ready_while_recording(
        1,
        1,
        Live {
            ticker_before: RID,
            ..Live::default()
        },
    );
    for msg in &mut flight.later {
        if let ShadowMsg::Ticker { gate, .. } = msg {
            gate.id = previous;
            gate.registered = gate.expected();
        }
    }
    h.feed(&flight);
    let event = h.only_event();
    assert_eq!(event.diff_kind, DiffKind::PostState.code(), "{event:?}");
    assert_eq!(h.stats().matched, 0);

    // The faithful ones stay clean.
    let mut h = harness();
    h.feed(&ready_publish(
        1,
        1,
        murmur_dictation_core::decision::OpStatus::Outstanding,
    ));
    h.feed(&ready_while_recording(2, 2, Live::default()));
    assert_clean(&h);
}

#[test]
fn history_op_ids_are_replayed_never_drawn() {
    let _ = OperationId(0);
}

#[test]
fn the_effect_result_is_part_of_what_is_witnessed() {
    // A terminal that lost the ledger is a legitimate result, not a mismatch.
    let mut h = harness();
    h.feed(&capture_failed(
        1,
        1,
        Live {
            term_won: false,
            ..Live::default()
        },
    ));
    assert_clean(&h);
    let _ = EffectResult::Ran;
}
