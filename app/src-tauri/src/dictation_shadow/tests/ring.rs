//! The shadow's bounded ring (decision D-S2c-2 a): one attempt, never a wait, every
//! message delivered or counted as dropped exactly once.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use super::harness::*;
use crate::dictation_shadow::ring::{Rejected, Rejection, Ring};
use crate::dictation_shadow::transport::{mark, Mark};
use crate::dictation_shadow::types::ReplyKind;

fn reject_reason<T: std::fmt::Debug>(result: Result<(), Rejected<T>>) -> Rejection {
    result.expect_err("the push must be rejected").reason
}

#[test]
fn a_ring_takes_its_capacity_then_reports_full_and_gives_the_value_back() {
    let ring = Ring::new(4);
    for n in 0..4 {
        ring.try_push(n).unwrap();
    }
    let rejected = ring.try_push(99).expect_err("full");
    assert_eq!(rejected.reason, Rejection::Full);
    assert_eq!(rejected.value, 99, "the value comes back to its owner");
    assert_eq!(ring.pop(), Some(0));
    ring.try_push(4).unwrap();
    let all: Vec<_> = std::iter::from_fn(|| ring.pop()).collect();
    assert_eq!(all, [1, 2, 3, 4]);
    assert_eq!(ring.pop(), None);
}

#[test]
fn a_slot_still_in_flight_is_a_contention_drop_not_a_wait() {
    // The consumer advanced the head and has not released the slot (the state in which
    // `ArrayQueue::push` spun and yielded). The producer makes one attempt and gets the
    // value back at once, attributed as contention, not as a full queue.
    let ring = Ring::new(2);
    ring.try_push(1).unwrap();
    ring.try_push(2).unwrap();
    let (taken, pos) = ring.begin_pop().expect("one message to take");
    assert_eq!(taken, 1);
    assert_eq!(reject_reason(ring.try_push(3)), Rejection::Contention);
    // Once the consumer finishes, the same push is accepted.
    ring.release(pos);
    ring.try_push(3).unwrap();
    // And with every slot really occupied it is `Full`, not contention.
    assert_eq!(reject_reason(ring.try_push(4)), Rejection::Full);
    let all: Vec<_> = std::iter::from_fn(|| ring.pop()).collect();
    assert_eq!(all, [2, 3]);
}

#[test]
fn a_lost_compare_and_swap_is_a_contention_drop_not_a_retry() {
    // Another producer claims the slot between this one reading it and claiming it.
    let ring = Ring::new(4);
    let attempts = AtomicUsize::new(0);
    let rejected = ring
        .try_push_between(1, || {
            attempts.fetch_add(1, Ordering::Relaxed);
            ring.try_push(2).unwrap();
        })
        .expect_err("the claim was lost");
    assert_eq!(rejected.reason, Rejection::Contention);
    assert_eq!(rejected.value, 1);
    assert_eq!(
        attempts.load(Ordering::Relaxed),
        1,
        "one attempt: the loser did not loop"
    );
    // Only the winner's message is in the ring, exactly once.
    assert_eq!(ring.pop(), Some(2));
    assert_eq!(ring.pop(), None);
}

#[test]
fn a_ring_drops_what_it_still_holds() {
    let live = Arc::new(());
    {
        let ring = Ring::new(4);
        ring.try_push(Arc::clone(&live)).unwrap();
        ring.try_push(Arc::clone(&live)).unwrap();
        assert_eq!(Arc::strong_count(&live), 3);
    }
    assert_eq!(Arc::strong_count(&live), 1);
}

#[test]
#[should_panic(expected = "power of two")]
fn a_ring_needs_a_power_of_two_capacity_of_at_least_two() {
    let _ring: Ring<u8> = Ring::new(1);
}

#[test]
fn every_message_is_delivered_or_counted_as_dropped_exactly_once() {
    const PRODUCERS: usize = 4;
    const PER_PRODUCER: usize = 20_000;
    let ring = Arc::new(Ring::new(8));
    let start = Arc::new(Barrier::new(PRODUCERS + 1));
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            let (ring, start) = (Arc::clone(&ring), Arc::clone(&start));
            thread::spawn(move || {
                start.wait();
                let mut dropped = Vec::new();
                for i in 0..PER_PRODUCER {
                    // Each message names itself; a rejection hands it back.
                    if let Err(rejected) = ring.try_push((p, i)) {
                        dropped.push(rejected.value);
                    }
                }
                dropped
            })
        })
        .collect();
    let consumer = {
        let (ring, start) = (Arc::clone(&ring), Arc::clone(&start));
        thread::spawn(move || {
            start.wait();
            let mut delivered = Vec::new();
            let mut idle = 0_u32;
            // Pop until the producers have been quiet for a long while.
            while idle < 200_000 {
                match ring.pop() {
                    Some(message) => {
                        delivered.push(message);
                        idle = 0;
                    }
                    None => idle += 1,
                }
            }
            delivered
        })
    };
    let dropped: Vec<(usize, usize)> = producers
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect();
    let mut delivered = consumer.join().unwrap();
    delivered.extend(std::iter::from_fn(|| ring.pop()));

    let delivered_set: HashSet<_> = delivered.iter().copied().collect();
    let dropped_set: HashSet<_> = dropped.iter().copied().collect();
    assert_eq!(
        delivered_set.len(),
        delivered.len(),
        "a message was delivered twice"
    );
    assert_eq!(
        dropped_set.len(),
        dropped.len(),
        "a message was dropped twice"
    );
    assert!(
        delivered_set.is_disjoint(&dropped_set),
        "delivered and dropped"
    );
    assert_eq!(
        delivered.len() + dropped.len(),
        PRODUCERS * PER_PRODUCER,
        "a message vanished"
    );
    // Per producer, delivery order is send order.
    for p in 0..PRODUCERS {
        let mut last = None;
        for &(owner, i) in &delivered {
            if owner == p {
                assert!(last.is_none_or(|l| l < i), "reordered within a producer");
                last = Some(i);
            }
        }
    }
}

#[test]
fn the_transport_counts_contention_apart_from_a_full_queue() {
    let transport = local_transport(2);
    let rec = transport.recorder();
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    // Full: a drop, not contention.
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    assert_eq!(transport.take_contended(), 0);
    assert_eq!(transport.take_dropped(), 1);

    // The consumer is between its two stores: the next push is contention, and it
    // returns at once.
    let (_, pos) = transport.begin_pop_for_test().expect("a message");
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    assert_eq!(transport.take_dropped(), 1);
    assert_eq!(transport.take_contended(), 1, "attributed as contention");
    transport.release_for_test(pos);
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    assert_eq!(transport.take_dropped(), 0, "released: accepted again");
    // Every drop names its recorder (`seq` 0 for a mark, which has no header).
    let mut losses = 0;
    while transport.pop_loss().is_some() {
        losses += 1;
    }
    assert_eq!(losses, 2);
}

#[test]
fn contended_drops_reach_the_summary() {
    let transport = local_transport(2);
    let rec = transport.recorder();
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    let (_, pos) = transport.begin_pop_for_test().unwrap();
    mark(&rec, Mark::Reply(ReplyKind::Ok));
    transport.release_for_test(pos);
    let mut h = harness();
    h.comparator.drain(&transport);
    assert_eq!(h.stats().dropped, 1);
    assert_eq!(h.stats().dropped_contended, 1);
}

#[test]
fn a_claimed_but_unpublished_head_is_not_an_empty_ring() {
    // D-S2c-3 major 1: the consumer must tell "nothing reserved" from "the head slot
    // is reserved and not yet readable", because entries behind it may already be
    // published and `pop` cannot return them.
    let ring = Ring::new(4);
    assert!(!ring.head_unpublished(), "empty");
    let paused = ring.claim_for_test().unwrap();
    assert!(ring.head_unpublished(), "claimed, not published");
    ring.try_push(2).unwrap();
    assert_eq!(ring.pop(), None, "the entry behind it is not readable yet");
    assert!(ring.head_unpublished());
    ring.publish_for_test(paused, 1);
    assert!(!ring.head_unpublished(), "published");
    assert_eq!(ring.pop(), Some(1));
    assert_eq!(ring.pop(), Some(2));
    assert!(!ring.head_unpublished(), "drained");
}

#[test]
fn the_push_path_has_no_loop_backoff_yield_or_wait() {
    // The whole of `try_push_between` and the transport's `push` / `reject`, as text:
    // a loop, a backoff, a yield, a sleep or a spin hint would reintroduce the stall.
    let ring = super::policy::production_source(include_str!("../ring.rs"));
    let transport = super::policy::production_source(include_str!("../transport.rs"));
    let bodies = [
        super::policy::function_body(&ring, "pub(crate) fn try_push_between("),
        super::policy::function_body(&ring, "pub(crate) fn try_push("),
        super::policy::function_body(&ring, "fn claim("),
        super::policy::function_body(&ring, "fn publish("),
        super::policy::function_body(&transport, "fn push("),
        super::policy::function_body(&transport, "fn reject("),
        super::policy::function_body(&transport, "fn record_loss("),
    ];
    for body in bodies {
        for token in [
            "loop", "while", "for ", "Backoff", "snooze", "spin", "yield", "sleep", "park", "wait",
            "retry", "continue",
        ] {
            assert!(
                !body.contains(token),
                "the push path contains `{token}`: {body}"
            );
        }
    }
    for token in ["crossbeam", "ArrayQueue", "Backoff"] {
        assert!(!ring.contains(token), "ring.rs names `{token}`");
        assert!(!transport.contains(token), "transport.rs names `{token}`");
    }
}
