//! Loss is a coarse, fail-closed reset, never an attribution (D-S2c-3).
//!
//! The live side only counts (a global `lost` and one counter per site). The
//! consumer stages a batch, reads `lost` again, and if it moved gives up the batch and
//! every open, held or waiting record. Each round-4 probe either cannot be built any
//! more (the loss ring, the lost-subject limit, the folding of one loss before the
//! next and the hold-until-visible logic are deleted) or resets here.

use std::sync::atomic::{AtomicU64, Ordering};

use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::transport::{mark, witness, Mark, ShadowRecorder, Transport};
use crate::dictation_shadow::types::{ReplyKind, ShadowMsg};

fn pd_16(h: &Harness) -> u64 {
    h.stats().pd_witness.get(&16).copied().unwrap_or(0)
}

/// The same flight through a fresh queue: the control every loss test compares to.
fn clean_run(flight: &Flight) -> Harness {
    let transport = local_transport(256);
    let recorder = transport.recorder();
    submit_header(&recorder, flight);
    send_later(&recorder, flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    h
}

#[test]
fn a_loss_between_two_messages_of_one_batch_discards_the_batch() {
    // D-S2c-3 g: the epoch check, deterministic. A flight's header and every later
    // message are already queued; one message is lost after the first was taken,
    // before the batch ends. The whole batch is given up, not compared.
    let flight = cancel(0, 0);
    let control = clean_run(&flight);
    assert_eq!(control.stats().matched, 1, "{:?}", control.stats());
    assert_eq!(pd_16(&control), 1);

    let transport = local_transport(256);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain_with(&transport, |taken| {
        if taken == 1 {
            transport.lose_for_test(flight.header.site_id);
        }
    });
    assert_eq!(h.stats().compared, 0, "nothing of the batch was compared");
    assert_eq!(h.stats().matched, 0);
    assert_eq!(h.stats().abandoned_by_loss, 1, "{:?}", h.stats());
    assert_eq!(pd_16(&h), 0, "a given-up flight credits nothing");
    assert!(h.events().is_empty(), "{:?}", h.events());
    assert_eq!(
        h.stats().lost_by_site.get(&flight.header.site_id),
        Some(&1),
        "{:?}",
        h.stats()
    );
}

#[test]
fn a_loss_between_polls_gives_up_the_open_record() {
    // The loss falls after the poll that opened the record and before the poll that
    // reads the rest, so it is outside any one batch: the count the last poll ended
    // on is what the next one compares with. 129 losses and a close that follows
    // them cannot evict the evidence (Sol round 4, major 3), and two losses cannot be
    // folded one at a time with a held close applied between them (major 4): there is
    // one count and one reset.
    let flight = cancel(1, 1);
    let transport = local_transport(256);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(h.stats().compared, 1, "the record is open");
    for _ in 0..129 {
        transport.lose_for_test(flight.header.site_id);
    }
    send_later(&recorder, &flight);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
    assert_eq!(pd_16(&h), 0, "{:?}", h.stats().pd_witness);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    assert_eq!(h.stats().mismatched_root, 0);
    assert_eq!(
        h.stats().lost_by_site.get(&flight.header.site_id),
        Some(&129)
    );
}

#[test]
fn a_loss_with_nothing_behind_it_still_gives_up_what_is_open() {
    // No later message ever arrives: the open record is given up on the next poll
    // anyway, instead of waiting for a close that may be the lost message itself.
    let flight = cancel(1, 1);
    let transport = local_transport(256);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    transport.lose_for_test(flight.header.site_id);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    // A second poll with no new loss gives up nothing more.
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    drop(recorder);
}

#[test]
fn a_header_waiting_behind_a_gap_and_its_held_close_are_given_up() {
    // A loss while a header waits for an earlier `seq`: the waiting header and the
    // messages held for it are abandoned, and the sequence resynchronises on the next
    // header instead of waiting for the lost position.
    let transport = default_transport();
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    let waiting = short_idle(3, 3);
    h.feed(&waiting);
    assert_eq!(h.stats().compared, 1, "seq 3 waits for seq 2");
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1, "{:?}", h.stats());
    // The next header resynchronises, whatever its seq.
    h.feed(&short_idle(40, 40));
    assert_eq!(h.stats().compared, 2, "{:?}", h.stats());
    // A header the reset moved the sequence past is discarded, not applied.
    h.feed(&short_idle(39, 39));
    assert_eq!(h.stats().late_header_discarded, 1);
    assert_eq!(h.stats().compared, 2);
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn a_reset_forgets_every_cache_that_depended_on_the_flights_it_gave_up() {
    // The owner's registration was seen before the loss: after the reset it is gone,
    // so the pipeline's finish can no longer be verified and is `unverifiable`, never
    // clean (D-S2c-3 f). The control, with no loss, closes clean.
    let exit = || pipeline_exit(2, 2, Status::Recording, RecordingId(RID.0 + 1), RID);
    let mut control = harness();
    control.feed(&pipeline_start(1, 1));
    control.feed(&exit());
    assert_eq!(control.stats().matched, 2, "{:?}", control.stats());
    assert_eq!(control.stats().unverifiable, 0);

    let transport = default_transport();
    let mut h = harness();
    h.feed(&pipeline_start(1, 1));
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    h.feed(&exit());
    assert_eq!(h.stats().matched, 1, "only the start: {:?}", h.stats());
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn losses_are_counted_by_site_and_never_filed_under_site_zero() {
    let transport = default_transport();
    transport.lose_for_test(216);
    transport.lose_for_test(216);
    transport.lose_for_test(237);
    transport.lose_for_test(0);
    transport.lose_for_test(60_000);
    let mut h = harness();
    h.comparator.drain(&transport);
    let by_site = &h.stats().lost_by_site;
    assert_eq!(by_site.get(&216), Some(&2), "{by_site:?}");
    assert_eq!(by_site.get(&237), Some(&1));
    assert_eq!(by_site.get(&0), None, "site 0 is never a key");
    assert_eq!(
        h.stats().loss_unattributed,
        2,
        "no site and a site out of range"
    );
    assert_eq!(h.stats().dropped, 5);
}

#[test]
fn a_dropped_witness_gives_up_its_flight_instead_of_a_root_mismatch() {
    // A real queue of 2: the header and one witness fit, the next witnesses are
    // dropped by the producer, which counts them against the flight's site.
    let transport = local_transport(2);
    let recorder = transport.recorder();
    let flight = cancel(0, 0);
    submit_header(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(h.stats().compared, 1);
    let witnesses = flight
        .later
        .iter()
        .filter(|m| matches!(m, ShadowMsg::Witness { .. }))
        .take(4)
        .cloned();
    for msg in witnesses {
        let ShadowMsg::Witness {
            idx,
            placement,
            effect,
            ..
        } = msg
        else {
            unreachable!()
        };
        witness(&recorder, idx, placement, effect);
    }
    assert_eq!(transport.lost(), 2, "two witnesses did not fit");
    // The recorder's drop sends its `Close`, which does not fit either.
    drop(recorder);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().mismatched_root, 0, "{:?}", h.events());
    assert!(h.events().is_empty());
    assert_eq!(h.stats().matched, 0);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    assert_eq!(
        h.stats().lost_by_site.get(&flight.header.site_id),
        Some(&3),
        "two witnesses and the close: {:?}",
        h.stats()
    );
}

#[test]
fn a_ticket_dropped_without_a_submit_is_a_loss_not_a_pre_abandonment() {
    let transport = default_transport();
    let recorder = transport.recorder();
    recorder.enter_pre();
    let ticket = recorder.ticket(216);
    assert_eq!(ticket.seq(), 1);
    drop(ticket);
    drop(recorder);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(
        h.stats().lost_by_site.get(&216),
        Some(&1),
        "the dropped ticket keeps its site: {:?}",
        h.stats()
    );
    assert_eq!(h.stats().lost_by_site.get(&0), None, "never site 0");
    assert_eq!(
        h.stats().abandoned_pre_d,
        0,
        "past D entry, not a Pre abandonment"
    );
    // The next header is not stuck behind the lost position.
    h.feed(&idle_for(2, 2, 41));
    assert_eq!(h.stats().compared, 1);
}

#[test]
fn a_full_queue_never_blocks_and_counts_each_drop() {
    let transport = local_transport(4);
    let rec = transport.recorder();
    // Nobody drains: 4 fit, the other 6 are dropped, and none of them blocks.
    for _ in 0..10 {
        mark(&rec, Mark::Reply(ReplyKind::Ok));
    }
    assert_eq!(transport.lost(), 6);
    assert_eq!(transport.contended(), 0, "full, not contended");
    let mut messages = 0;
    while transport.pop().is_some() {
        messages += 1;
    }
    assert_eq!(messages, 4);
}

#[test]
fn a_reserved_unpublished_slot_wedges_nothing_in_the_comparison() {
    // Sol round 4, major 1: a producer reserved the head slot and was paused. The
    // consumer stops at it and never skips it; there is no second ring to wedge, and
    // the reservation is reported (shutdown: `output.rs`). Once it publishes, the
    // flight behind it is compared.
    let flight = short_idle(0, 0);
    let transport = local_transport(8);
    let paused = transport.reserve_for_test();
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(
        h.stats().compared,
        0,
        "nothing is read past the reservation"
    );
    assert_eq!(transport.reserved_unpublished(), 1);
    assert!(transport.unconsumed() > 1);
    assert_eq!(
        h.stats().abandoned_by_loss,
        0,
        "a reservation is not a loss"
    );
    transport.publish_for_test(
        paused,
        ShadowMsg::PreAbandoned {
            rec: crate::dictation_shadow::types::RecorderId(1),
        },
    );
    h.comparator.drain(&transport);
    assert_eq!(h.stats().compared, 1, "{:?}", h.stats());
    assert_eq!(transport.unconsumed(), 0);
}

#[test]
fn a_recorder_without_a_transport_is_a_loss_like_any_other() {
    // The first enabled startup: a recorder created before the transport existed
    // dropped messages. They never reached the ring, but the comparison cannot know
    // what they were, so what is open is given up.
    static UNAVAILABLE: AtomicU64 = AtomicU64::new(0);
    let transport = default_transport();
    let flight = cancel(1, 1);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    let mut h = harness();
    h.comparator.count_unavailable_from(&UNAVAILABLE);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().compared, 1);
    UNAVAILABLE.fetch_add(3, Ordering::Relaxed);
    send_later(&recorder, &flight);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().dropped_unavailable, 3);
    assert_eq!(h.stats().dropped, 3);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    assert_eq!(h.stats().matched, 0);
    assert_eq!(pd_16(&h), 0);
}

#[test]
fn a_batch_is_at_most_the_staging_capacity_and_each_is_checked_on_its_own() {
    // More messages than one batch holds: the loss lands in the second batch only, so
    // the first is processed and the second given up.
    let transport = local_transport(256);
    let recorders: Vec<ShadowRecorder> = (0..100).map(|_| transport.recorder()).collect();
    let flights: Vec<Flight> = (0..100).map(|n| idle_for(0, 0, 100 + n)).collect();
    for (recorder, flight) in recorders.iter().zip(&flights) {
        submit_header(recorder, flight);
        send_later(recorder, flight);
    }
    let mut h = harness();
    let mut batch = 0;
    h.comparator.drain_with(&transport, |taken| {
        if taken == 1 {
            batch += 1;
            if batch == 2 {
                transport.lose_for_test(216);
            }
        }
    });
    assert!(h.stats().matched > 0, "the first batch was compared");
    assert!(h.stats().abandoned_by_loss > 0, "the second was given up");
    assert!(h.stats().matched + h.stats().abandoned_by_loss <= 100);
    let _: &Transport = &transport;
}

#[test]
fn a_late_message_of_a_given_up_flight_is_dropped_not_held() {
    // The flights given up by a reset are remembered, so what they send afterwards is
    // dropped. If it were held instead, those 64 flights and the 36 open ones that
    // follow would overflow the 64-record cap and evict real ones (`lost_open`).
    let transport = local_transport(1024);
    let flights: Vec<Flight> = (0..100).map(|n| idle_for(0, 0, 100 + n)).collect();
    let recorders: Vec<ShadowRecorder> = flights.iter().map(|_| transport.recorder()).collect();
    for (recorder, flight) in recorders.iter().zip(&flights) {
        submit_header(recorder, flight);
    }
    let mut h = harness();
    let mut lost = false;
    h.comparator.drain_with(&transport, |taken| {
        if taken == 1 && !lost {
            lost = true;
            transport.lose_for_test(216);
        }
    });
    assert_eq!(
        h.stats().abandoned_by_loss,
        64,
        "one batch: {:?}",
        h.stats()
    );
    assert_eq!(h.stats().compared, 36, "the next batch was compared");
    for (recorder, flight) in recorders.iter().zip(&flights) {
        send_later(recorder, flight);
    }
    h.comparator.drain(&transport);
    assert_eq!(h.stats().lost_open, 0, "{:?}", h.stats());
    assert_eq!(h.stats().matched, 36, "{:?}", h.stats());
    assert!(h.events().is_empty(), "{:?}", h.events());
}

#[test]
fn the_header_of_an_evicted_flight_is_discarded_not_compared_without_its_witnesses() {
    // Witnesses that arrive before their header are held, at most 64 flights at a
    // time. The oldest beyond that is evicted and remembered; its header, which
    // arrives afterwards, is discarded instead of being compared against nothing.
    let mut h = harness();
    let flights: Vec<Flight> = (0..65).map(|n| cancel(n + 2, n + 2)).collect();
    for flight in &flights {
        h.feed_later(flight);
    }
    assert_eq!(h.stats().lost_open, 1, "{:?}", h.stats());
    // Seq 1 is applied first so the others are in order.
    h.feed(&short_idle(1, 1));
    h.feed_header(&flights[0]);
    assert_eq!(h.stats().late_header_discarded, 1, "{:?}", h.stats());
    assert_eq!(h.stats().compared, 1, "only seq 1 was compared");
    assert!(h.events().is_empty(), "{:?}", h.events());
}
