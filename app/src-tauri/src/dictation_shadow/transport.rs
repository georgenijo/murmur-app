//! The live side of shadow capture: the preallocated queue, the flight-owned
//! recorder and the header ticket (appendix C8.2, decision D-S2c-1 a and c).
//!
//! **The producer path is a single lock-free `push` into a preallocated
//! `crossbeam_queue::ArrayQueue` and nothing else.** `witness`, `mark`, `submit`
//! and the recorder's `close` store a message by value in one of the queue's 256
//! slots. They allocate nothing, take no lock, register no waker, wake no thread and
//! never block: a full queue counts a drop through an atomic and records *which
//! flight and which `seq`* was lost in a second preallocated lock-free queue (the
//! loss log). The consumer polls on a fixed sleep (`mod.rs`); there is no channel,
//! no `std::sync::mpsc` and no `Mutex` on this side.
//!
//! The only synchronous dispatch and the only mutex in this file are in the
//! `test_mode` module at the end, compiled only under `cfg(test)`; the
//! `shadow_capture_takes_no_lock_allocates_nothing` test pins the production text
//! and counts allocations on the real path.
//!
//! **No thread-local is used for correlation** (C8.2): a flight's later phases run
//! on other threads and tasks, so the recorder travels with the flight instead.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use murmur_dictation_core::effects::Placement;

use super::types::{
    CondWitness, Inline, LiveOp, LossEntry, MirrorWitness, PredId, RecorderId, ReplyKind,
    ShadowHeader, ShadowMsg, TickerGate, WitnessedEffect, COND_CAPACITY, MIRROR_CAPACITY,
    PRE_CAPACITY,
};

/// The queue's capacity (C8.2, D-S2c-1 a).
pub(crate) const QUEUE_CAPACITY: usize = 256;

/// The loss log's capacity: one entry per message the queue could not take.
pub(crate) const LOSS_CAPACITY: usize = 256;

/// The recorder id counter starts here, so a `RecorderId` can never be mistaken for
/// a header `seq` (which starts at 1): they are different namespaces by construction.
const REC_BASE: u64 = 1 << 32;

/// Everything both sides share: the queues, the counters the live side bumps and
/// the two id counters.
pub(crate) struct Transport {
    queue: ArrayQueue<ShadowMsg>,
    /// What the queue could not take, by recorder and `seq` (D-S2c-1 c).
    losses: ArrayQueue<LossEntry>,
    /// Messages the queue could not take.
    dropped: AtomicU64,
    /// Losses the loss log itself could not take: the consumer can no longer say
    /// which records were affected and must invalidate every open one.
    loss_overflow: AtomicU64,
    /// Capture-list overflows (a flight recorded more than a list can hold).
    overflow: AtomicU64,
    /// Next header `seq`; the first one handed out is 1.
    seq: AtomicU64,
    /// Next recorder id.
    rec: AtomicU64,
    #[cfg(test)]
    sync: AtomicBool,
    #[cfg(test)]
    sync_comparator: std::sync::Mutex<Option<super::compare::Comparator>>,
}

impl Transport {
    /// A transport whose queues are allocated here, once, and never again.
    pub(crate) fn new(capacity: usize, loss_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            queue: ArrayQueue::new(capacity),
            losses: ArrayQueue::new(loss_capacity),
            dropped: AtomicU64::new(0),
            loss_overflow: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            seq: AtomicU64::new(1),
            rec: AtomicU64::new(REC_BASE),
            #[cfg(test)]
            sync: AtomicBool::new(false),
            #[cfg(test)]
            sync_comparator: std::sync::Mutex::new(None),
        })
    }

    /// The consumer's side: the next queued message.
    pub(crate) fn pop(&self) -> Option<ShadowMsg> {
        self.queue.pop()
    }

    /// Messages waiting in the queue.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.queue.len()
    }

    /// The consumer's side: the next recorded loss.
    pub(crate) fn pop_loss(&self) -> Option<LossEntry> {
        self.losses.pop()
    }

    /// Messages the queue could not take, since the last call.
    pub(crate) fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// Losses nobody could attribute, since the last call.
    pub(crate) fn take_loss_overflow(&self) -> u64 {
        self.loss_overflow.swap(0, Ordering::Relaxed)
    }

    /// Capture-list overflows, since the last call.
    pub(crate) fn take_overflow(&self) -> u64 {
        self.overflow.swap(0, Ordering::Relaxed)
    }

    /// A new flight-local recorder, created **before** `run(Pre)` (C4.1 step 2).
    pub(crate) fn recorder(self: &Arc<Self>) -> ShadowRecorder {
        ShadowRecorder {
            rec: RecorderId(self.rec.fetch_add(1, Ordering::Relaxed)),
            transport: Arc::clone(self),
            next_idx: AtomicU16::new(0),
            closed: AtomicBool::new(false),
            pre_entered: AtomicBool::new(false),
            seq_taken: AtomicBool::new(false),
            header_submitted: AtomicBool::new(false),
            pre: PreCapture::default(),
        }
    }

    fn send(&self, msg: ShadowMsg) {
        #[cfg(test)]
        if self.sync.load(Ordering::Relaxed) {
            self.deliver_sync(msg);
            return;
        }
        self.push(msg);
    }

    /// The live path: one `push`. A rejected message comes back by value, so the
    /// loss is attributed to its recorder and `seq` from the message itself.
    fn push(&self, msg: ShadowMsg) {
        if let Err(rejected) = self.queue.push(msg) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            self.record_loss(LossEntry::of(&rejected));
        }
    }

    fn record_loss(&self, loss: LossEntry) {
        if self.losses.push(loss).is_err() {
            self.loss_overflow.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn count_overflow(&self) {
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }
}

/// The `Pre` witnesses buffered inline before the header exists (C8.2): owned by
/// the flight, written on the decision's own thread, so it needs no lock and no
/// allocation. It is a recorder field, so it is preallocated with the recorder.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreCapture {
    pub effects: Inline<WitnessedEffect, PRE_CAPACITY>,
    pub conditionals: Inline<CondWitness, COND_CAPACITY>,
    pub mirrors: Inline<MirrorWitness, MIRROR_CAPACITY>,
    pub overflowed: bool,
}

/// The flight-owned recorder: created before `run(Pre)`, moved with the flight
/// (the command future, the spawned continuation, the guards, the supervisor-arm
/// closure; wrap it in an `Arc` to share it between tasks of one flight).
///
/// * `enter_pre` marks the start of `run(Pre)`. A flight dropped before that sent
///   nothing and owes nothing.
/// * `Pre` witnesses go to the inline buffer, then into the header (`take_pre`).
/// * Later witnesses are incremental messages, sent only after the header was
///   submitted, so every later message is causally after its header.
/// * `finish` sends `Close{completed:true}`. A drop without it sends
///   `Close{completed:false}` once the header was submitted, or `PreAbandoned` when
///   `run(Pre)` had begun and D was never entered. A drop after the ticket was taken
///   but before the header was sent is the ticket's loss.
pub(crate) struct ShadowRecorder {
    rec: RecorderId,
    transport: Arc<Transport>,
    next_idx: AtomicU16,
    closed: AtomicBool,
    /// `run(Pre)` was entered: the flight has done (or is doing) observable work.
    pre_entered: AtomicBool,
    /// A ticket was taken: `seq` was assigned, so the flight is past D entry.
    seq_taken: AtomicBool,
    header_submitted: AtomicBool,
    pre: PreCapture,
}

impl ShadowRecorder {
    /// The recorder's id (not a `seq`).
    pub(crate) fn id(&self) -> RecorderId {
        self.rec
    }

    /// `run(Pre)` begins. Only a flight that entered it can be `PreAbandoned`.
    pub(crate) fn enter_pre(&self) {
        self.pre_entered.store(true, Ordering::Relaxed);
    }

    /// The index the next effect of the **full** decided list will have, and
    /// advance. A conditional effect the live path skipped still consumes its index
    /// (`skip_idx`), so `Witness::idx` stays aligned with the decided list.
    pub(crate) fn next_idx(&self) -> u16 {
        self.next_idx.fetch_add(1, Ordering::Relaxed)
    }

    /// Consume the index of an effect that is not run (a skipped conditional, or a
    /// prefix `after_pre` / `after_prefix` dropped before the executor).
    pub(crate) fn skip_idx(&self) {
        self.next_idx.fetch_add(1, Ordering::Relaxed);
    }

    fn note_overflow(&mut self, stored: bool) -> bool {
        if !stored {
            self.pre.overflowed = true;
            self.transport.count_overflow();
        }
        stored
    }

    /// Buffer a `Pre` effect that actually ran. `false` (and an overflow count)
    /// when the buffer is full; the header is then marked not comparable.
    pub(crate) fn pre_effect(&mut self, effect: WitnessedEffect) -> bool {
        let stored = self.pre.effects.push(effect);
        self.note_overflow(stored)
    }

    /// Buffer a shell-conditional `Pre` effect (taken or skipped).
    pub(crate) fn pre_conditional(&mut self, pred: PredId, taken: bool) -> bool {
        let stored = self.pre.conditionals.push(CondWitness { pred, taken });
        self.note_overflow(stored)
    }

    /// Buffer a `Pre` effect's live-field write, before and after.
    pub(crate) fn pre_mirror(&mut self, mirror: MirrorWitness) -> bool {
        let stored = self.pre.mirrors.push(mirror);
        self.note_overflow(stored)
    }

    /// Move the buffered `Pre` witnesses out, for the header. Call it before
    /// `ticket`, which borrows the recorder.
    pub(crate) fn take_pre(&mut self) -> PreCapture {
        std::mem::take(&mut self.pre)
    }

    /// Take the header's `seq` at the existing decision boundary (under D; D-less
    /// entries at decision time). The ticket records the loss if it is dropped
    /// without a submit.
    pub(crate) fn ticket(&self) -> ShadowTicket<'_> {
        self.seq_taken.store(true, Ordering::Relaxed);
        ShadowTicket {
            seq: self.transport.seq.fetch_add(1, Ordering::Relaxed),
            recorder: self,
            sent: false,
        }
    }

    /// The flight ran to its end: send `Close{completed:true}`. Idempotent.
    pub(crate) fn finish(&self) {
        self.close(true);
    }

    fn close(&self, completed: bool) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.header_submitted.load(Ordering::Relaxed) {
            self.transport.send(ShadowMsg::Close {
                rec: self.rec,
                completed,
            });
        } else if !self.seq_taken.load(Ordering::Relaxed)
            && self.pre_entered.load(Ordering::Relaxed)
        {
            // `run(Pre)` began and D was never entered: no decision, no `seq`.
            // (A ticket that was taken and never submitted reports its own loss,
            // and a recorder that never entered `Pre` did nothing to report.)
            self.transport
                .send(ShadowMsg::PreAbandoned { rec: self.rec });
        }
    }
}

impl Drop for ShadowRecorder {
    /// A flight dropped without `finish` (a dropped future, a panic, a discarded
    /// task): the abandoned flight of C4.4a.
    fn drop(&mut self) {
        self.close(false);
    }
}

/// The right to submit one header: `seq` was taken at the decision boundary. Records
/// the loss from its `Drop` if the thread unwinds between D release and `submit`.
pub(crate) struct ShadowTicket<'a> {
    seq: u64,
    recorder: &'a ShadowRecorder,
    sent: bool,
}

impl ShadowTicket<'_> {
    /// The header's `seq`.
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    /// The recorder the ticket was taken from.
    pub(crate) fn rec(&self) -> RecorderId {
        self.recorder.rec
    }
}

impl Drop for ShadowTicket<'_> {
    fn drop(&mut self) {
        if !self.sent {
            self.recorder.transport.record_loss(LossEntry {
                rec: self.recorder.rec.0,
                seq: self.seq,
                ..LossEntry::default()
            });
        }
    }
}

/// What `mark` sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// A shell-conditional effect after D.
    Cond { pred: PredId, taken: bool },
    /// A live M-field written at effect time after D, before and after.
    Mirror(MirrorWitness),
    /// A ticker registration attempt after D.
    Ticker(TickerGate),
    /// An H1 / H2 operation after D.
    History(LiveOp),
    /// The command's reply kind.
    Reply(ReplyKind),
}

/// Submit the header right after D is released and before the first `PostDT` effect
/// (C8.2): one `push`, no lock, never blocks. The header's `seq` and `rec` must be
/// the ticket's and its recorder's.
pub(crate) fn submit(mut ticket: ShadowTicket<'_>, header: ShadowHeader) {
    debug_assert_eq!(
        header.seq, ticket.seq,
        "a header is submitted under its own ticket"
    );
    debug_assert_eq!(
        header.rec, ticket.recorder.rec,
        "a header names its ticket's recorder"
    );
    ticket.sent = true;
    ticket
        .recorder
        .header_submitted
        .store(true, Ordering::Relaxed);
    ticket.recorder.transport.send(ShadowMsg::Header(header));
}

/// An effect that ran after D: one `push` of a `Witness`.
pub(crate) fn witness(
    rec: &ShadowRecorder,
    idx: u16,
    placement: Placement,
    effect: WitnessedEffect,
) {
    rec.transport.send(ShadowMsg::Witness {
        rec: rec.rec,
        idx,
        placement,
        effect,
    });
}

/// A conditional, a mirror, a ticker attempt, a history operation or the reply of a
/// flight: one `push`.
pub(crate) fn mark(rec: &ShadowRecorder, mark: Mark) {
    let rec_id = rec.rec;
    let msg = match mark {
        Mark::Cond { pred, taken } => ShadowMsg::Cond {
            rec: rec_id,
            pred,
            taken,
        },
        Mark::Mirror(mirror) => ShadowMsg::Mirror {
            rec: rec_id,
            mirror,
        },
        Mark::Ticker(gate) => ShadowMsg::Ticker { rec: rec_id, gate },
        Mark::History(op) => ShadowMsg::History { rec: rec_id, op },
        Mark::Reply(kind) => ShadowMsg::Reply { rec: rec_id, kind },
    };
    rec.transport.send(msg);
}

/// Synchronous test-mode dispatch and its mutex: compiled only under `cfg(test)`
/// (D-S2c-1 a). The golden replay of Stage 2c-2 compares on the calling thread
/// through it so the result is fully deterministic.
#[cfg(test)]
mod test_mode {
    use super::{Ordering, Transport};
    use crate::dictation_shadow::compare::Comparator;
    use crate::dictation_shadow::types::ShadowMsg;

    impl Transport {
        /// Compare on the calling thread through `comparator`, no queue.
        pub(crate) fn set_sync(&self, comparator: Option<Comparator>) {
            let on = comparator.is_some();
            *self
                .sync_comparator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = comparator;
            self.sync.store(on, Ordering::SeqCst);
        }

        /// Run `f` on the synchronous comparator.
        pub(crate) fn with_sync<R>(&self, f: impl FnOnce(&mut Comparator) -> R) -> R {
            let mut guard = self
                .sync_comparator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            f(guard.as_mut().expect("sync comparator installed"))
        }

        pub(super) fn deliver_sync(&self, msg: ShadowMsg) {
            let mut guard = self
                .sync_comparator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(comparator) = guard.as_mut() {
                comparator.pump_losses(self);
                comparator.handle_msg(msg);
            }
        }
    }
}
