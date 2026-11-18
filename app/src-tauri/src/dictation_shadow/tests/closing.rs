//! Every opened recorder is closed exactly once (D-S2c-4 b), through one `close`.
//!
//! The round-6 probes (Sol's re-check of 1b47f67b) are regression tests here, and a
//! randomized stress of resets, losses and panics checks the conservation: opened ==
//! the sum of every close kind plus what is still open, and no second close ever
//! happens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use murmur_dictation_core::ids::{OperationId, RecordingId};
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::compare::Comparator;
use crate::dictation_shadow::transport::{submit, ShadowRecorder, ShadowTicket};
use crate::dictation_shadow::types::{HistoryAction, HistoryTable, RecorderId, ShadowMsg};

fn panicking_comparator() -> Comparator {
    Comparator::new(
        Box::new(|core, input, now| core.handle(input, now)),
        Box::new(|_| panic!("injected sink panic")),
        Box::new(|| 0),
    )
}

fn submit_ticket(ticket: ShadowTicket<'_>, rec: RecorderId, flight: &Flight) {
    let mut header = flight.header.clone();
    header.seq = ticket.seq();
    header.rec = rec;
    submit(ticket, header);
}

// ---------------------------------------------------- Sol's round-6 probes

#[test]
fn route_panic_accounts_for_both_open_flights() {
    let transport = default_transport();
    let open = transport.recorder();
    submit_header(&open, &short_idle(0, 0));
    let closing = transport.recorder();
    let flight = cancel(0, 0);
    submit_header(&closing, &flight);
    closing.finish(); // Completed close without the required later effects.
    let mut cmp = panicking_comparator();
    cmp.drain(&transport);
    assert_eq!(cmp.stats.shadow_panic, 1);
    assert_eq!(cmp.stats.abandoned_by_panic, 2, "{:?}", cmp.stats);
    assert_eq!(cmp.stats.double_close_suppressed, 0);
    cmp.assert_conserved();
}

#[test]
fn pending_close_panic_accounts_for_both_open_flights() {
    let transport = default_transport();
    let first = transport.recorder();
    let second = transport.recorder();
    let f = short_idle(0, 0);
    let c = cancel(0, 0);
    let t1 = first.ticket(f.header.site_id);
    submit_header(&second, &c); // #2 waits for #1.
    second.finish(); // #2's close is held while its header waits.
    submit_ticket(t1, first.id(), &f); // #1 drains the pending header and close.
    let mut cmp = panicking_comparator();
    cmp.drain(&transport);
    assert_eq!(cmp.stats.shadow_panic, 1);
    assert_eq!(cmp.stats.abandoned_by_panic, 2, "{:?}", cmp.stats);
    assert_eq!(cmp.stats.double_close_suppressed, 0);
    cmp.assert_conserved();
}

#[test]
fn two_post_loss_flights_survive_reverse_publication() {
    let transport = default_transport();
    let mut h = harness();
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    let flight = short_idle(0, 0);
    let first = transport.recorder();
    let t1 = first.ticket(flight.header.site_id);
    let second = transport.recorder();
    submit_header(&second, &flight);
    send_later(&second, &flight);
    h.comparator.drain(&transport);
    submit_ticket(t1, first.id(), &flight);
    send_later(&first, &flight);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
    assert_eq!(h.stats().late_header_discarded, 0, "{:?}", h.stats());
    h.comparator.assert_conserved();
}

#[test]
fn older_epoch_in_the_middle_does_not_strand_later_headers() {
    // A reserves seq 1 then pauses before reading lost. B reserves seq 2 and reads
    // epoch 0. A then reads epoch 1, and C takes seq 3 in epoch 1.
    let transport = default_transport();
    transport.lose_for_test(216);
    let mut h = harness();
    h.comparator.drain(&transport);
    h.feed(&at_epoch(short_idle(1, 1), 1));
    h.feed(&at_epoch(short_idle(2, 2), 0));
    h.feed(&at_epoch(short_idle(3, 3), 1));
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
    assert_eq!(
        h.stats().abandoned_by_loss,
        1,
        "the rejected header is a skip"
    );
    h.comparator.assert_conserved();
}

#[test]
fn a_later_witness_does_not_count_an_already_abandoned_flight_again() {
    let transport = default_transport();
    let recorder = transport.recorder();
    let flight = short_idle(0, 0);
    submit_header(&recorder, &flight);
    let mut h = harness();
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    send_later(&recorder, &flight);
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1, "{:?}", h.stats());
    assert!(h.stats().late_message_discarded > 0, "{:?}", h.stats());
    assert_eq!(h.stats().double_close_suppressed, 0);
    h.comparator.assert_conserved();
}

#[test]
fn post_loss_flight_already_staged_is_discarded_by_the_coarse_reset() {
    let transport = default_transport();
    transport.lose_for_test(216);
    let recorder = transport.recorder();
    let flight = short_idle(0, 0);
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1);
    assert_eq!(h.stats().matched, 0);
    h.comparator.assert_conserved();
}

/// A first registration with a wrong operation, then the real one: the recording has
/// two `K:Pipeline` registrations.
fn owner_with_two_registrations() -> Flight {
    let mut owner = pipeline_start(1, 1);
    let first = owner
        .header
        .live_ops
        .iter_mut()
        .find(|op| op.table == HistoryTable::H1 && op.action == HistoryAction::Registered)
        .unwrap();
    let actual_owner = *first;
    first.op = OperationId(9_999);
    owner.header.add_op(actual_owner);
    owner
}

#[test]
fn a_duplicate_registration_cannot_validate_a_wrong_owner_finish() {
    let mut h = harness();
    let owner = owner_with_two_registrations();
    let actual = owner.header.live_ops.iter().last().unwrap().op;
    h.feed(&owner);
    assert_eq!(h.stats().mismatched_root, 1);

    let mut finish = pipeline_exit(2, 2, Status::Processing, RID, RID);
    let retirement = finish
        .header
        .live_ops
        .iter_mut()
        .find(|op| {
            op.table == HistoryTable::H1 && matches!(op.action, HistoryAction::Retired { .. })
        })
        .unwrap();
    assert_eq!(retirement.op, actual);
    retirement.op = OperationId(9_999);
    h.feed(&finish);
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
    h.comparator.assert_conserved();
}

#[test]
fn an_ambiguous_registration_makes_even_the_right_retirement_unverifiable() {
    // The retirement names the real operation, but the recording had two
    // registrations: the finish is neither clean nor a mismatch.
    let mut h = harness();
    h.feed(&owner_with_two_registrations());
    h.feed(&pipeline_exit(2, 2, Status::Processing, RID, RID));
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
}

#[test]
fn a_later_conflicting_registration_invalidates_a_cached_token() {
    // The registration was valid and cached; a second one for the same recording
    // arrives later, in a message of the same flight. The finish that follows must
    // not be matched against the token that is no longer unique.
    let mut h = harness();
    let owner = pipeline_start(1, 1);
    h.feed_header(&owner);
    let mut second = *owner
        .header
        .live_ops
        .iter()
        .find(|op| op.table == HistoryTable::H1 && op.action == HistoryAction::Registered)
        .unwrap();
    second.op = OperationId(5_555);
    h.send(ShadowMsg::History {
        rec: owner.header.rec,
        op: second,
    });
    h.feed_later(&owner);
    h.feed(&pipeline_exit(
        2,
        2,
        Status::Recording,
        RecordingId(RID.0 + 1),
        RID,
    ));
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
    h.comparator.assert_conserved();
}

#[test]
fn two_flights_each_validating_a_registration_for_one_recording_make_it_ambiguous() {
    // Each registration is valid for its own flight, but the recording now has two
    // operations: neither can be the owner's token the finish retires.
    let mut h = harness();
    h.feed(&pipeline_start(1, 1));
    h.feed(&pipeline_start(2, 2));
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
    h.feed(&pipeline_exit(
        3,
        3,
        Status::Recording,
        RecordingId(RID.0 + 1),
        RID,
    ));
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
}

#[test]
fn a_conflict_between_the_finish_header_and_its_close_makes_it_unverifiable() {
    // The finish's header read a unique token; before its close, a second
    // registration for the recording arrives. The finish is verified against the
    // cache as it stands at its close.
    let mut h = harness();
    let owner = pipeline_start(1, 1);
    h.feed_header(&owner);
    let finish = pipeline_exit(2, 2, Status::Recording, RecordingId(RID.0 + 1), RID);
    h.feed_header(&finish);
    let mut second = *owner
        .header
        .live_ops
        .iter()
        .find(|op| op.table == HistoryTable::H1 && op.action == HistoryAction::Registered)
        .unwrap();
    second.op = OperationId(5_555);
    h.send(ShadowMsg::History {
        rec: owner.header.rec,
        op: second,
    });
    h.feed_later(&finish);
    h.feed_later(&owner);
    assert_eq!(h.stats().unverifiable, 1, "{:?}", h.stats());
    assert_eq!(h.stats().matched, 0, "{:?}", h.stats());
}

#[test]
fn a_unique_registration_still_verifies_its_finish() {
    // The control: one registration, one matching retirement.
    let mut h = harness();
    h.feed(&pipeline_start(1, 1));
    h.feed(&pipeline_exit(
        2,
        2,
        Status::Recording,
        RecordingId(RID.0 + 1),
        RID,
    ));
    assert_eq!(h.stats().matched, 2, "{:?}", h.stats());
    assert_eq!(h.stats().unverifiable, 0);
    h.comparator.assert_conserved();
}

// ------------------------------------------------------------ the close set

#[test]
fn a_second_close_is_a_counted_no_op() {
    let mut h = harness();
    let flight = short_idle(1, 1);
    h.feed(&flight);
    assert_eq!(h.stats().matched, 1);
    assert_eq!(h.stats().double_close_suppressed, 0);
    h.comparator.close_again_for_test(flight.header.rec);
    assert_eq!(h.stats().matched, 1, "nothing is counted twice");
    assert_eq!(h.stats().double_close_suppressed, 1);
    h.comparator.assert_conserved();
}

#[test]
fn every_way_a_recorder_closes_is_one_kind() {
    let transport = default_transport();
    let mut h = harness();
    // Matched and a pre-D abandonment, then a loss that gives up the one still open.
    let done = transport.recorder();
    submit_header(&done, &short_idle(0, 0));
    send_later(&done, &short_idle(0, 0));
    let pre = transport.recorder();
    pre.enter_pre();
    drop(pre);
    let open = transport.recorder();
    submit_header(&open, &short_idle(0, 0));
    h.comparator.drain(&transport);
    assert_eq!(h.stats().matched, 1, "{:?}", h.stats());
    assert_eq!(h.stats().abandoned_pre_d, 1, "{:?}", h.stats());
    assert_eq!(h.comparator.open_flights(), 1);
    h.comparator.assert_conserved();
    transport.lose_for_test(216);
    h.comparator.drain(&transport);
    assert_eq!(h.stats().abandoned_by_loss, 1, "{:?}", h.stats());
    assert_eq!(h.comparator.open_flights(), 0);
    assert_eq!(h.stats().opened, 3);
    h.comparator.assert_conserved();
    drop((done, open));
}

// -------------------------------------------------------------- the stress

/// A small deterministic generator (xorshift): the stress must be repeatable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
}

enum Slot<'a> {
    Fresh,
    Ticketed(ShadowTicket<'a>),
    Submitted,
    Done,
}

fn stress(seed: u64, steps: usize) {
    let transport = default_transport();
    let panic_now = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&panic_now);
    let mut comparator = Comparator::new(
        Box::new(move |core, input, now| {
            assert!(!flag.load(Ordering::SeqCst), "stress panic in the decider");
            core.handle(input, now)
        }),
        Box::new(|_| {}),
        Box::new(|| 0),
    );
    let recorders: Vec<ShadowRecorder> = (0..steps).map(|_| transport.recorder()).collect();
    let flights: Vec<Flight> = (0..steps)
        .map(|i| {
            if i % 2 == 0 {
                short_idle(0, 0)
            } else {
                cancel(0, 0)
            }
        })
        .collect();
    let mut slots: Vec<Slot<'_>> = (0..steps).map(|_| Slot::Fresh).collect();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..steps {
        let i = rng.below(steps);
        match rng.below(9) {
            0 => {
                if matches!(slots[i], Slot::Fresh) {
                    submit_header(&recorders[i], &flights[i]);
                    slots[i] = Slot::Submitted;
                }
            }
            1 => {
                if matches!(slots[i], Slot::Fresh) {
                    recorders[i].enter_pre();
                    slots[i] = Slot::Ticketed(recorders[i].ticket(flights[i].header.site_id));
                }
            }
            2 => {
                if matches!(slots[i], Slot::Ticketed(_)) {
                    let Slot::Ticketed(ticket) = std::mem::replace(&mut slots[i], Slot::Submitted)
                    else {
                        unreachable!()
                    };
                    submit_ticket(ticket, recorders[i].id(), &flights[i]);
                }
            }
            3 => {
                if matches!(slots[i], Slot::Submitted) {
                    send_later(&recorders[i], &flights[i]);
                    slots[i] = Slot::Done;
                }
            }
            4 => transport.lose_for_test(216),
            5 => panic_now.store(rng.below(4) == 0, Ordering::SeqCst),
            6 => {
                let pre = transport.recorder();
                pre.enter_pre();
                drop(pre);
            }
            _ => {
                comparator.drain(&transport);
                comparator.assert_conserved();
            }
        }
    }
    panic_now.store(false, Ordering::SeqCst);
    comparator.drain(&transport);
    comparator.assert_conserved();
    assert_eq!(
        comparator.stats.double_close_suppressed, 0,
        "seed {seed}: {:?}",
        comparator.stats
    );
    // A last loss gives up whatever is still open: nothing is left unclosed.
    transport.lose_for_test(216);
    comparator.drain(&transport);
    assert_eq!(comparator.open_flights(), 0, "seed {seed}");
    assert_eq!(
        comparator.stats.opened,
        comparator.stats.closed_total(),
        "seed {seed}: {:?}",
        comparator.stats
    );
    assert_eq!(comparator.stats.double_close_suppressed, 0, "seed {seed}");
    // Slots that were never finished are dropped here, after every assertion.
    drop(slots);
}

#[test]
fn opened_equals_closed_under_random_resets_losses_and_panics() {
    for seed in 1..=300 {
        stress(seed, 48);
    }
}
