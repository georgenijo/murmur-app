//! A small bounded ring for the shadow's live side (decision D-S2c-2 a).
//!
//! This is Vyukov's bounded queue with per-slot sequence numbers, cut down to what
//! the shadow needs: **many producers, one consumer**, and a producer that makes
//! **exactly one attempt** and never waits.
//!
//! The reason it is not `crossbeam_queue::ArrayQueue`: that queue's `push` loops
//! (it spins while a consumer that already advanced the head has not yet released the
//! slot, and it backs off, yielding the thread, when it loses a race). The live path
//! must not wait on the shadow in any form, so [`Ring::try_push`] has no loop, no
//! backoff, no yield and no system call. Every outcome that is not "the slot was
//! free and this producer claimed it" hands the value back as a [`Rejected`]:
//!
//! * [`Rejection::Full`]: every slot holds an unread message;
//! * [`Rejection::Contention`]: a slot is still in flight (the consumer advanced the
//!   head and has not released the slot yet), or another producer claimed the slot
//!   first (a lost compare-and-swap, or a stale tail).
//!
//! The consumer side is single: [`Ring::pop`] takes a one-bit guard, so a second
//! concurrent consumer gets `None` instead of racing.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Cell<T> {
    /// `pos` when free for the producer that owns `pos`, `pos + 1` when it holds a
    /// message for the consumer at `pos`, `pos + capacity` once it was released.
    seq: AtomicUsize,
    value: UnsafeCell<MaybeUninit<T>>,
}

/// Why a message was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// Every slot holds a message the consumer has not taken.
    Full,
    /// A slot was in flight, or another producer won the race for it.
    Contention,
}

/// A message the ring did not take, returned by value with the reason.
#[derive(Debug)]
pub(crate) struct Rejected<T> {
    pub value: T,
    pub reason: Rejection,
}

/// The bounded ring. Capacity is a power of two and is allocated once, in `new`.
pub(crate) struct Ring<T> {
    cells: Box<[Cell<T>]>,
    mask: usize,
    /// Next position a producer claims.
    tail: AtomicUsize,
    /// Next position the consumer reads.
    head: AtomicUsize,
    /// One-bit guard that keeps the consumer side single.
    consuming: AtomicBool,
}

// SAFETY: a cell's value is written only by the producer that claimed its position
// (the winner of the compare-and-swap on `tail`) and read only by the single
// consumer after the producer's release store of `seq`; the release / acquire pair
// on `seq` orders every access to the value.
unsafe impl<T: Send> Send for Ring<T> {}
// SAFETY: see above; all shared state is atomic or guarded by the `seq` protocol.
unsafe impl<T: Send> Sync for Ring<T> {}

impl<T> Ring<T> {
    /// A ring of `capacity` slots: a power of two, at least 2 (with one slot the
    /// "holds a message" and "free for the next lap" sequence values coincide).
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(
            capacity >= 2 && capacity.is_power_of_two(),
            "a ring's capacity is a power of two, at least 2"
        );
        let cells = (0..capacity)
            .map(|i| Cell {
                seq: AtomicUsize::new(i),
                value: UnsafeCell::new(MaybeUninit::uninit()),
            })
            .collect();
        Self {
            cells,
            mask: capacity - 1,
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
            consuming: AtomicBool::new(false),
        }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// One attempt to store `value`. Never loops, never waits.
    pub(crate) fn try_push(&self, value: T) -> Result<(), Rejected<T>> {
        self.try_push_between(value, || {})
    }

    /// [`Ring::try_push`] with a seam between reading the slot and claiming it, so a
    /// test can interleave another producer or the consumer at exactly that point.
    /// Production passes an empty closure.
    pub(crate) fn try_push_between(
        &self,
        value: T,
        between: impl FnOnce(),
    ) -> Result<(), Rejected<T>> {
        match self.claim(between) {
            Ok(pos) => {
                self.publish(pos, value);
                Ok(())
            }
            Err(reason) => Err(Rejected { value, reason }),
        }
    }

    /// The first half of a push: one attempt to reserve the slot at the tail. The
    /// slot is **not readable** until [`Ring::publish`] runs; between the two, it is
    /// counted by [`Ring::reserved_unpublished`].
    fn claim(&self, between: impl FnOnce()) -> Result<usize, Rejection> {
        let pos = self.tail.load(Ordering::Relaxed);
        let cell = &self.cells[pos & self.mask];
        let seq = cell.seq.load(Ordering::Acquire);
        between();
        let lag = seq.wrapping_sub(pos).cast_signed();
        if lag == 0 {
            // The slot is free for `pos`: claim it, once.
            if self
                .tail
                .compare_exchange(
                    pos,
                    pos.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_err()
            {
                return Err(Rejection::Contention);
            }
            Ok(pos)
        } else if lag < 0 {
            // The slot still holds, or is still being released from, a message of the
            // previous lap. Full when every slot is occupied; otherwise the consumer
            // advanced the head and has not released the slot yet: in flight.
            let head = self.head.load(Ordering::Acquire);
            if pos.wrapping_sub(head) >= self.capacity() {
                Err(Rejection::Full)
            } else {
                Err(Rejection::Contention)
            }
        } else {
            // The tail moved under us: another producer claimed `pos` already.
            Err(Rejection::Contention)
        }
    }

    /// The second half of a push: write the value into the slot [`Ring::claim`]
    /// reserved and make it readable.
    fn publish(&self, pos: usize, value: T) {
        let cell = &self.cells[pos & self.mask];
        // SAFETY: the winning compare-and-swap made this producer the only one
        // allowed to write the cell at `pos`, and `seq == pos` said the consumer
        // finished with its previous message.
        unsafe { (*cell.value.get()).write(value) };
        cell.seq.store(pos.wrapping_add(1), Ordering::Release);
    }

    /// Slots claimed and not yet consumed, published or not.
    pub(crate) fn unconsumed(&self) -> usize {
        self.tail
            .load(Ordering::Acquire)
            .wrapping_sub(self.head.load(Ordering::Acquire))
    }

    /// Of [`Ring::unconsumed`], the slots a producer reserved and has not published
    /// yet. [`Ring::pop`] stops at the first of them and never skips it, so every
    /// message behind it waits too (D-S2c-3 e).
    pub(crate) fn reserved_unpublished(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let claimed = self
            .tail
            .load(Ordering::Acquire)
            .wrapping_sub(head)
            .min(self.capacity());
        (0..claimed)
            .filter(|offset| {
                let pos = head.wrapping_add(*offset);
                self.cells[pos & self.mask].seq.load(Ordering::Acquire) != pos.wrapping_add(1)
            })
            .count()
    }

    /// Test seam: reserve a slot and stop before publishing (a producer paused
    /// between its two steps).
    #[cfg(test)]
    pub(crate) fn claim_for_test(&self) -> Result<usize, Rejection> {
        self.claim(|| {})
    }

    /// Test seam: finish the push a paused producer started.
    #[cfg(test)]
    pub(crate) fn publish_for_test(&self, pos: usize, value: T) {
        self.publish(pos, value);
    }

    /// The consumer's side: the next message, or `None` when the ring is empty (or
    /// the next message is still being written by its producer).
    pub(crate) fn pop(&self) -> Option<T> {
        if self.consuming.swap(true, Ordering::Acquire) {
            return None;
        }
        let taken = self.begin_pop();
        let value = taken.map(|(value, pos)| {
            self.release(pos);
            value
        });
        self.consuming.store(false, Ordering::Release);
        value
    }

    /// Take the next message and advance the head, leaving its slot **unreleased**:
    /// the state a consumer is in between the two stores of `pop`.
    pub(crate) fn begin_pop(&self) -> Option<(T, usize)> {
        let pos = self.head.load(Ordering::Relaxed);
        let cell = &self.cells[pos & self.mask];
        if cell.seq.load(Ordering::Acquire) != pos.wrapping_add(1) {
            return None;
        }
        // SAFETY: `seq == pos + 1` says the producer finished writing the cell, and
        // the consumer is single, so nobody else reads or frees it.
        let value = unsafe { (*cell.value.get()).assume_init_read() };
        self.head.store(pos.wrapping_add(1), Ordering::Release);
        Some((value, pos))
    }

    /// Release the slot `begin_pop` left in flight.
    pub(crate) fn release(&self, pos: usize) {
        self.cells[pos & self.mask]
            .seq
            .store(pos.wrapping_add(self.capacity()), Ordering::Release);
    }
}

impl<T> Drop for Ring<T> {
    fn drop(&mut self) {
        // Drop the messages still held; `&mut self` means no producer or consumer.
        while self.pop().is_some() {}
    }
}
