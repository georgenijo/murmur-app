//! The "wrong slot" family around `Pre:CX-` (R216.1): each is a way the live path
//! could have touched the wrong context. Review fix (Sol 6.2 major 5): the wrong-slot
//! `Pre:CX-` mutant survived all 46 original shadow tests. These tests kill it: with
//! `mirror_ok` relaxed (any `CX-` mirror accepted) every one of them fails.

use murmur_dictation_core::effects::Effect;
use murmur_dictation_core::ids::RecordingId;

use super::harness::*;
use crate::dictation_shadow::compare::mirror_ok;
use crate::dictation_shadow::types::{
    DiffKind, MirrorField, MirrorValue, MirrorWitness, ShadowMsg,
};

fn cx_flight() -> Flight {
    let flight = short_idle(1, 1);
    assert_eq!(flight.header.pre_mirrors.len(), 1);
    assert_eq!(
        flight.header.pre_mirrors.get(0).unwrap().field,
        MirrorField::ActiveContext
    );
    flight
}

fn mirror_of(flight: &mut Flight) -> &mut MirrorWitness {
    flight.header.pre_mirrors.iter_mut().next().unwrap()
}

#[test]
fn a_faithful_pre_cx_clear_is_clean() {
    let mut h = harness();
    h.feed(&cx_flight());
    assert_clean(&h);
}

#[test]
fn wrong_slot_pre_cx_erased_another_recordings_context() {
    let mut h = harness();
    let mut flight = cx_flight();
    mirror_of(&mut flight).before = MirrorValue::Slot(Some(RecordingId(99)));
    mirror_of(&mut flight).after = MirrorValue::Slot(None);
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn wrong_slot_pre_cx_replaced_the_slot() {
    let mut h = harness();
    let mut flight = cx_flight();
    mirror_of(&mut flight).after = MirrorValue::Slot(Some(RecordingId(99)));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn wrong_slot_pre_cx_left_the_slot_set() {
    // A `Pre:CX-` that is witnessed but leaves the matching slot set.
    let mut h = harness();
    let mut flight = cx_flight();
    assert_eq!(mirror_of(&mut flight).before, MirrorValue::Slot(Some(RID)));
    mirror_of(&mut flight).after = MirrorValue::Slot(Some(RID));
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn wrong_slot_pre_cx_witnessed_for_another_id() {
    let mut h = harness();
    let mut flight = cx_flight();
    for w in flight.header.pre_effects.iter_mut() {
        if matches!(w.effect, Effect::ClearContext(_)) {
            w.effect = Effect::ClearContext(RecordingId(99));
        }
    }
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn wrong_slot_pre_cx_mirror_on_another_field() {
    let mut h = harness();
    let mut flight = cx_flight();
    mirror_of(&mut flight).field = MirrorField::KeyboardProcessing;
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn a_stale_safe_clear_of_another_recordings_slot_is_clean() {
    // `CX-(7)` leaves a slot that holds 99 alone: nothing to report.
    let mut h = harness();
    let mut flight = cx_flight();
    let other = MirrorValue::Slot(Some(RecordingId(99)));
    mirror_of(&mut flight).before = other;
    mirror_of(&mut flight).after = other;
    h.feed(&flight);
    assert_clean(&h);
}

#[test]
fn an_omitted_pre_cx_mirror_is_a_missing_witness() {
    let mut h = harness();
    let mut flight = cx_flight();
    flight.header.pre_mirrors.clear();
    h.feed_header(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn a_late_cx_clear_is_checked_before_and_after() {
    // R205.5's later `CX-`: the live slot held another recording's id and the clear
    // erased it. With only the final value (`None`) that is indistinguishable from a
    // correct clear; the mirror message carries both.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror { mirror, .. } = msg {
            if mirror.field == MirrorField::ActiveContext {
                mirror.before = MirrorValue::Slot(Some(RecordingId(99)));
                mirror.after = MirrorValue::Slot(None);
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // And the stale-safe form (slot 99 untouched) is clean.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror { mirror, .. } = msg {
            if mirror.field == MirrorField::ActiveContext {
                mirror.before = MirrorValue::Slot(Some(RecordingId(99)));
                mirror.after = MirrorValue::Slot(Some(RecordingId(99)));
            }
        }
    }
    h.feed(&flight);
    assert_clean(&h);
}

#[test]
fn mirror_ok_pins_each_effect_kind() {
    let id = RecordingId(7);
    let slot = |v: Option<u64>| MirrorValue::Slot(v.map(RecordingId));
    let cx = |before, after| MirrorWitness {
        field: MirrorField::ActiveContext,
        before: slot(before),
        after: slot(after),
    };
    assert!(mirror_ok(&Effect::ClearContext(id), &cx(Some(7), None)));
    assert!(!mirror_ok(&Effect::ClearContext(id), &cx(Some(7), Some(7))));
    assert!(!mirror_ok(&Effect::ClearContext(id), &cx(Some(9), None)));
    assert!(mirror_ok(&Effect::ClearContext(id), &cx(Some(9), Some(9))));
    assert!(mirror_ok(&Effect::ClearContext(id), &cx(None, None)));
    assert!(mirror_ok(&Effect::SetContext(id), &cx(None, Some(7))));
    assert!(!mirror_ok(&Effect::SetContext(id), &cx(None, Some(9))));
    let mc = |before: u64, after: u64| MirrorWitness {
        field: MirrorField::CancelledThrough,
        before: MirrorValue::Id(RecordingId(before)),
        after: MirrorValue::Id(RecordingId(after)),
    };
    assert!(mirror_ok(&Effect::MarkCancelled(id), &mc(3, 7)));
    assert!(mirror_ok(&Effect::MarkCancelled(id), &mc(9, 9)));
    assert!(!mirror_ok(&Effect::MarkCancelled(id), &mc(3, 3)));
    assert!(!mirror_ok(&Effect::MarkCancelled(id), &mc(3, 9)));
    let kb = |after: bool| MirrorWitness {
        field: MirrorField::KeyboardProcessing,
        before: MirrorValue::Flag(true),
        after: MirrorValue::Flag(after),
    };
    assert!(mirror_ok(&Effect::KeyboardProcessing(false), &kb(false)));
    assert!(!mirror_ok(&Effect::KeyboardProcessing(false), &kb(true)));
}
