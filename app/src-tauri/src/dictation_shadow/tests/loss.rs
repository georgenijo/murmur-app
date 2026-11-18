//! Loss is attributed by recorder and `seq`, never guessed (D-S2c-1 c).
//!
//! Review fix (Sol 6.2 blocker 3): a dropped witness used to surface as a root
//! mismatch on the surviving record, a lost header tainted its neighbours at the time
//! the loss *arrived* instead of at its `seq` position, and a header that arrived
//! after its gap was declared lost was applied anyway.

use murmur_dictation_core::ids::RecordingId;

use super::harness::*;
use crate::dictation_shadow::compare::{Comparator, MAX_PENDING_HEADERS};
use crate::dictation_shadow::transport::{mark, Mark};
use crate::dictation_shadow::types::{LossEntry, RecorderId, ReplyKind, ShadowMsg};

fn lost_header(rec: u64, seq: u64, site: u64, ids: [u64; 3]) -> LossEntry {
    LossEntry {
        rec,
        seq,
        site,
        ids,
    }
}

fn drain(comparator: &mut Comparator, transport: &crate::dictation_shadow::transport::Transport) {
    comparator.drain(transport);
}

#[test]
fn a_dropped_witness_invalidates_its_open_record_instead_of_a_root_mismatch() {
    // A real queue of 2: the header and one witness are applied, then the queue
    // fills, a witness is dropped and the loss names the flight.
    let transport = local_transport(2);
    let mut h = harness();
    let flight = cancel(0, 0);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    drain(&mut h.comparator, &transport);
    assert_eq!(h.stats().compared, 1);
    let witnesses: Vec<ShadowMsg> = flight
        .later
        .iter()
        .filter(|m| matches!(m, ShadowMsg::Witness { .. }))
        .cloned()
        .collect();
    assert!(witnesses.len() >= 4);
    // Two fit, the third and fourth are dropped.
    for msg in witnesses.into_iter().take(4) {
        let ShadowMsg::Witness {
            idx,
            placement,
            effect,
            ..
        } = msg
        else {
            unreachable!()
        };
        crate::dictation_shadow::transport::witness(&recorder, idx, placement, effect);
    }
    recorder.finish();
    drain(&mut h.comparator, &transport);
    assert!(h.stats().dropped >= 2, "{:?}", h.stats());
    assert_eq!(h.stats().mismatched_root, 0, "{:?}", h.events());
    assert!(h.events().is_empty());
    assert_eq!(
        h.stats().matched,
        0,
        "a record that lost a witness is not clean"
    );
    assert_eq!(h.stats().invalidated, 1);
    // What the lost message was about stays tainted: the next decision for the same
    // recording is not compared; an unrelated one is.
    h.feed(&short_idle(2, 2));
    assert_eq!(h.stats().skipped_tainted, 1);
    h.feed(&idle_for(3, 3, 41));
    assert_eq!(h.stats().skipped_tainted, 1);
}

#[test]
fn a_header_the_queue_dropped_is_attributed_to_its_recorder_seq_and_site() {
    // A queue of 1 that nobody drains: the second header is rejected and the loss
    // entry carries its recorder, its `seq`, its site and the ids it named.
    let transport = local_transport(1);
    let first = transport.recorder();
    let second = transport.recorder();
    let flight_a = idle_for(0, 0, 11);
    let flight_b = idle_for(0, 0, 22);
    submit_header(&first, &flight_a);
    let lost_seq = submit_header(&second, &flight_b);
    let mut h = harness();
    drain(&mut h.comparator, &transport);
    assert_eq!(h.stats().dropped, 1);
    assert_eq!(h.stats().compared, 1, "only the first header arrived");
    assert_eq!(
        h.stats().lost_gap,
        1,
        "the lost position was skipped in seq order"
    );
    assert_eq!(
        h.stats().lost_by_site.get(&216),
        Some(&1),
        "{:?}",
        h.stats()
    );
    assert_eq!(lost_seq, 2);
    // The ids the lost header named are tainted: recording 22 is skipped, 33 is not.
    h.feed(&idle_for(3, 3, 22));
    h.feed(&idle_for(4, 4, 33));
    assert_eq!(h.stats().skipped_tainted, 1);
    assert_eq!(h.stats().mismatched_root, 0);
    drop((first, second));
}

#[test]
fn gap_taint_is_applied_to_the_neighbours_in_seq_order() {
    let mut h = harness();
    h.feed_header(&idle_for(1, 1, 11));
    // The loss of seq 3 arrives *before* seq 2 does. Taint belongs to the position:
    // seq 2's id and seq 4's id, not whoever happened to be applied when it arrived.
    h.comparator.on_loss(lost_header(903, 3, 216, [0; 3]));
    h.feed_header(&idle_for(2, 2, 12));
    h.feed_header(&idle_for(4, 4, 14));
    h.feed_header(&idle_for(5, 5, 12));
    h.feed_header(&idle_for(6, 6, 11));
    // Seq 2 precedes the gap and is compared; seq 4 follows it and is skipped; seq 5
    // shares seq 2's id and is skipped; seq 6 (id 11, not a neighbour) is compared.
    assert_eq!(h.stats().lost_gap, 1);
    assert_eq!(h.stats().skipped_tainted, 2);
    // Five headers arrived (the lost one did not); each one's decision is computed so
    // the shadow's own state keeps up, the tainted two are just never reported.
    assert_eq!(h.stats().compared, 5);
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn a_fresh_generation_after_a_lost_position_stays_clean() {
    let mut h = harness();
    h.feed(&idle_for(1, 1, 6));
    h.comparator.on_loss(lost_header(903, 2, 216, [0; 3]));
    // A start allocates a new id: it is about the new generation, whatever was lost.
    let fresh = start(3, 3, Live::default());
    assert!(fresh.header.live_post_d.current_id > fresh.header.pre.current_id);
    h.feed(&fresh);
    assert_eq!(h.stats().skipped_tainted, 0);
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
    assert_clean(&h);

    // A neighbour that is not a fresh generation is skipped.
    let mut h = harness();
    h.feed(&idle_for(1, 1, 6));
    h.comparator.on_loss(lost_header(903, 2, 216, [0; 3]));
    h.feed(&idle_for(3, 3, 6));
    assert_eq!(h.stats().skipped_tainted, 1);
}

#[test]
fn a_header_after_its_gap_was_declared_lost_is_discarded() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    for seq in 3..=u64::try_from(MAX_PENDING_HEADERS).unwrap() + 3 {
        h.send(ShadowMsg::Header(short_idle(seq, seq).header));
    }
    assert_eq!(h.stats().lost_gap, 1);
    let before = h.stats().compared;
    h.send(ShadowMsg::Header(short_idle(2, 2).header));
    assert_eq!(h.stats().compared, before, "the late header is not applied");
    assert_eq!(h.stats().late_header_discarded, 1);
    // Its late messages go nowhere either.
    for msg in short_idle(2, 2).later {
        h.send(msg);
    }
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn a_header_of_an_invalidated_flight_is_discarded_and_does_not_stall_what_follows() {
    let mut h = harness();
    // The flight of recorder 5 lost a message before its header got here.
    h.comparator.on_loss(lost_header(5, 0, 0, [0; 3]));
    h.feed(&short_idle(1, 5));
    assert_eq!(h.stats().late_header_discarded, 1);
    assert_eq!(h.stats().compared, 0);
    // Its position was skipped in seq order: seq 2 applies at once.
    h.feed(&idle_for(2, 6, 41));
    assert_eq!(h.stats().compared, 1);
    assert_eq!(h.stats().lost_gap, 1);
}

#[test]
fn an_open_record_that_shares_an_id_with_a_lost_header_is_invalidated() {
    let mut h = harness();
    let flight = cancel(1, 1);
    h.feed_header(&flight);
    // Another flight's header, about the same recording, was lost.
    h.comparator.on_loss(lost_header(77, 2, 205, [RID.0, 0, 0]));
    h.feed_later(&flight);
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn the_loss_log_overflowing_invalidates_every_open_record() {
    // Queue of 1 and a loss log of 1: the second and third headers are rejected and
    // only the first loss fits, so the second one cannot be attributed.
    let transport = crate::dictation_shadow::transport::Transport::new(1, 1);
    let first = transport.recorder();
    let (second, third) = (transport.recorder(), transport.recorder());
    submit_header(&first, &idle_for(0, 0, 11));
    submit_header(&second, &idle_for(0, 0, 22));
    submit_header(&third, &idle_for(0, 0, 33));
    let mut h = harness();
    drain(&mut h.comparator, &transport);
    assert_eq!(h.stats().dropped, 2);
    assert_eq!(h.stats().loss_unattributed, 1);
    // The open record of the surviving header is no longer trusted.
    first.finish();
    drain(&mut h.comparator, &transport);
    assert_eq!(h.stats().matched, 0);
    assert_eq!(h.stats().mismatched_root, 0);
    drop((first, second, third));
}

#[test]
fn a_ticket_dropped_without_a_submit_is_a_loss_not_a_pre_abandonment() {
    let transport = default_transport();
    let recorder = transport.recorder();
    recorder.enter_pre();
    let ticket = recorder.ticket();
    assert_eq!(ticket.seq(), 1);
    drop(ticket);
    drop(recorder);
    let mut h = harness();
    drain(&mut h.comparator, &transport);
    assert_eq!(h.stats().lost_gap, 1);
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
fn a_late_duplicate_loss_of_an_already_passed_position_is_not_a_second_gap() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    h.comparator.on_loss(lost_header(9, 2, 216, [0; 3]));
    assert_eq!(h.stats().lost_gap, 1);
    h.comparator.on_loss(lost_header(9, 2, 216, [0; 3]));
    assert_eq!(h.stats().lost_gap, 1);
    // Both are attributed to the site, though: that is what E4 counts.
    assert_eq!(h.stats().lost_by_site.get(&216), Some(&2));
}

#[test]
fn a_full_queue_never_blocks_and_counts_each_drop() {
    let transport = local_transport(4);
    let rec = transport.recorder();
    // Nobody drains: 4 fit, the other 6 are dropped, and none of them blocks.
    for _ in 0..10 {
        mark(&rec, Mark::Reply(ReplyKind::Ok));
    }
    assert_eq!(transport.take_dropped(), 6);
    assert_eq!(transport.take_dropped(), 0, "the count is taken, not kept");
    let mut messages = 0;
    while transport.pop().is_some() {
        messages += 1;
    }
    assert_eq!(messages, 4);
    let mut losses = 0;
    while transport.pop_loss().is_some() {
        losses += 1;
    }
    assert_eq!(losses, 6, "each drop names its recorder");
    let _ = (RecorderId(0), RecordingId(0));
}
