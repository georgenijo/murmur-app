//! The live side of shadow capture: the preallocated ring, the flight-owned
//! recorder and the header ticket (appendix C8.2, decisions D-S2c-1 a and c,
//! D-S2c-2 a, D-S2c-3 a).
//!
//! **The producer path is one bounded `try_push` into the in-crate [`Ring`] and
//! nothing else.** `witness`, `mark`, `submit` and the recorder's `close` store a
//! message by value in one of the ring's 256 slots, **in exactly one attempt**: no
//! loop, no backoff, no yield, no allocation, no free, no lock, no waker, no thread
//! wake-up. A full ring, a lost compare-and-swap or a slot still in flight hands the
//! message back; the producer then bumps two atomic counters and nothing else: a
//! global `lost` and one per *site* (`lost_by_site`, E4). There is no loss log and no
//! subject id: the consumer cannot say what was lost, only that something was, and
//! it then gives up everything in flight (`compare.rs`, D-S2c-3 b). The consumer
//! polls on a fixed sleep (`mod.rs`); there is no channel, no `std::sync::mpsc` and
//! no `Mutex` on this side.
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

use murmur_dictation_core::decision::SiteId;
use murmur_dictation_core::effects::Placement;
use murmur_dictation_core::ids::RecordingId;

use super::ring::{Rejection, Ring};
use super::types::{
    CondWitness, Inline, LiveOp, MirrorWitness, NoReplyReason, ObservedFact, PredId, RecorderId,
    ReplyKind, ShadowHeader, ShadowMsg, TickerGate, TickerSkipReason, WitnessedEffect,
    COND_CAPACITY, MIRROR_CAPACITY, PRE_CAPACITY,
};

/// The queue's capacity (C8.2, D-S2c-1 a).
pub(crate) const QUEUE_CAPACITY: usize = 256;

/// Sites with a loss counter of their own: one per registered site (`SiteId::ALL`).
/// Slot `0` counts the losses nobody could attribute (no site, or a code the registry
/// does not know), so the counters are `SITE_COUNT + 1` and a registered site always
/// has its own.
pub(crate) const SITE_COUNT: usize = SiteId::ALL.len();

/// The largest registered site code.
const MAX_SITE_CODE: usize = {
    let mut max = 0;
    let mut i = 0;
    while i < SITE_COUNT {
        let code = SiteId::ALL[i].code() as usize;
        if code > max {
            max = code;
        }
        i += 1;
    }
    max
};

/// Site code to counter slot (`1 + its place in the registry`; `0` = not registered),
/// built at compile time from `SiteId::ALL`, so a new site is attributed without
/// touching this file and the producer's lookup is one array read.
static SITE_SLOT: [u8; MAX_SITE_CODE + 1] = {
    let mut table = [0_u8; MAX_SITE_CODE + 1];
    let mut i = 0;
    while i < SITE_COUNT {
        table[SiteId::ALL[i].code() as usize] = (i + 1) as u8;
        i += 1;
    }
    table
};

/// The recorder id counter starts here, so a `RecorderId` can never be mistaken for
/// a header `seq` (which starts at 1): they are different namespaces by construction.
const REC_BASE: u64 = 1 << 32;

/// Everything both sides share: the queue, the counters the live side bumps and
/// the two id counters.
pub(crate) struct Transport {
    queue: Ring<ShadowMsg>,
    /// Every message the live side could not hand over (a rejected push, or a ticket
    /// dropped with no header). Bumped with `Release` **before** the flight's next
    /// message can be published, so a consumer that reads a later message sees it
    /// (D-S2c-3 b).
    lost: AtomicU64,
    /// `lost` by the site of the decision whose flight lost it, indexed by
    /// [`SITE_SLOT`]; slot `0` is the unattributed rest. A fixed array, allocated
    /// once.
    sites: Box<[AtomicU64]>,
    /// The part of the rejected pushes that was contention (a slot in flight or a
    /// lost race) rather than a full queue.
    contended: AtomicU64,
    /// Capture-list overflows (a flight recorded more than a list can hold).
    overflow: AtomicU64,
    /// Next header `seq`; the first one handed out is 1.
    seq: AtomicU64,
    /// Next recorder id.
    rec: AtomicU64,
    /// Shutdown has begun (D-S2c-15 review, major 2): a push after it is refused and
    /// counted in `after_close`, so nothing can enter the queue behind the final drain.
    closing: AtomicBool,
    /// Pushes between their entry and their end. Shutdown waits for it to reach 0
    /// after it sets `closing`, so a push that passed the check is queued before the
    /// final drain looks.
    inflight: AtomicU64,
    /// Messages refused because shutdown had begun.
    after_close: AtomicU64,
    #[cfg(test)]
    sync: AtomicBool,
    #[cfg(test)]
    sync_comparator: std::sync::Mutex<Option<super::compare::Comparator>>,
}

impl Transport {
    /// A transport whose ring and counters are allocated here, once, and never again.
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            queue: Ring::new(capacity),
            lost: AtomicU64::new(0),
            sites: (0..=SITE_COUNT).map(|_| AtomicU64::new(0)).collect(),
            contended: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            seq: AtomicU64::new(1),
            rec: AtomicU64::new(REC_BASE),
            closing: AtomicBool::new(false),
            inflight: AtomicU64::new(0),
            after_close: AtomicU64::new(0),
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

    /// The ring's capacity.
    pub(crate) fn capacity(&self) -> usize {
        self.queue.capacity()
    }

    /// Messages claimed and not yet consumed, published or not.
    pub(crate) fn unconsumed(&self) -> usize {
        self.queue.unconsumed()
    }

    /// Of [`Transport::unconsumed`], the slots a producer reserved and has not
    /// published (the consumer stops at the first and never skips it).
    pub(crate) fn reserved_unpublished(&self) -> usize {
        self.queue.reserved_unpublished()
    }

    /// Test seam: the consumer between its two stores, the head advanced and the slot
    /// not yet released (the "in flight" state that used to stall the producer).
    #[cfg(test)]
    pub(crate) fn begin_pop_for_test(&self) -> Option<(ShadowMsg, usize)> {
        self.queue.begin_pop()
    }

    /// Test seam: release the slot `begin_pop_for_test` left in flight.
    #[cfg(test)]
    pub(crate) fn release_for_test(&self, pos: usize) {
        self.queue.release(pos);
    }

    /// Test seam: a producer that reserved a slot and was paused.
    #[cfg(test)]
    pub(crate) fn reserve_for_test(&self) -> usize {
        self.queue.claim_for_test().expect("a free slot")
    }

    /// Test seam: the paused producer's second step.
    #[cfg(test)]
    pub(crate) fn publish_for_test(&self, pos: usize, msg: ShadowMsg) {
        self.queue.publish_for_test(pos, msg);
    }

    /// Test seam: a loss at `site`, as if a push had been rejected.
    #[cfg(test)]
    pub(crate) fn lose_for_test(&self, site: u16) {
        self.lose(site);
    }

    /// Messages the live side could not hand over, so far.
    pub(crate) fn lost(&self) -> u64 {
        self.lost.load(Ordering::Acquire)
    }

    /// Losses by site, so far: `each(site code, count)` for every registered site
    /// with a loss, and `each(0, count)` for the unattributed rest.
    pub(crate) fn lost_by_site(&self, mut each: impl FnMut(u16, u64)) {
        for (slot, count) in self.sites.iter().enumerate() {
            let count = count.load(Ordering::Acquire);
            if count > 0 {
                let code = slot
                    .checked_sub(1)
                    .and_then(|index| SiteId::ALL.get(index))
                    .map_or(0, |site| site.code());
                each(code, count);
            }
        }
    }

    /// The part of the rejected pushes that was contention, so far.
    pub(crate) fn contended(&self) -> u64 {
        self.contended.load(Ordering::Relaxed)
    }

    /// Capture-list overflows, since the last call.
    pub(crate) fn take_overflow(&self) -> u64 {
        self.overflow.swap(0, Ordering::Relaxed)
    }

    /// A new flight-local recorder, created **before** `run(Pre)` (C4.1 step 2).
    pub(crate) fn recorder(self: &Arc<Self>) -> ShadowRecorder {
        ShadowRecorder {
            rec: RecorderId(self.rec.fetch_add(1, Ordering::Relaxed)),
            transport: Some(Arc::clone(self)),
            unavailable: None,
            next_idx: AtomicU16::new(0),
            site: AtomicU16::new(0),
            closed: AtomicBool::new(false),
            pre_entered: AtomicBool::new(false),
            seq_taken: AtomicBool::new(false),
            header_submitted: AtomicBool::new(false),
            pre: PreCapture::default(),
        }
    }

    fn send(&self, msg: ShadowMsg, site: u16) {
        #[cfg(test)]
        if self.sync.load(Ordering::Relaxed) {
            self.deliver_sync(msg);
            return;
        }
        self.push(msg, site);
    }

    /// The live path: one `try_push`, one attempt. A rejected message is counted
    /// against its flight's site and nothing else is done with it.
    fn push(&self, msg: ShadowMsg, site: u16) {
        // Enter, then check `closing` (both `SeqCst`): either shutdown sees this push
        // in `inflight` and waits for it, or this push sees `closing` and refuses.
        self.inflight.fetch_add(1, Ordering::SeqCst);
        if self.closing.load(Ordering::SeqCst) {
            self.after_close.fetch_add(1, Ordering::Relaxed);
        } else if let Err(rejected) = self.queue.try_push(msg) {
            if rejected.reason == Rejection::Contention {
                self.contended.fetch_add(1, Ordering::Relaxed);
            }
            self.lose(site);
        }
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Shutdown, the consumer's side: refuse every later push, then wait (up to `limit`)
    /// for the pushes already past the check to finish. Returns whether the producers
    /// were quiet in time. After `true`, the queue holds every message that will ever
    /// be published; after `false` the summary says a push may still land.
    pub(crate) fn close_and_quiesce(&self, limit: std::time::Duration) -> bool {
        self.closing.store(true, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + limit;
        while self.inflight.load(Ordering::SeqCst) > 0 {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::yield_now();
        }
        true
    }

    /// Messages refused because shutdown had begun.
    pub(crate) fn after_close(&self) -> u64 {
        self.after_close.load(Ordering::Relaxed)
    }

    /// Test seam: a push paused between its entry and its end.
    #[cfg(test)]
    pub(crate) fn hold_inflight_for_test(&self) {
        self.inflight.fetch_add(1, Ordering::SeqCst);
    }

    /// Test seam: the paused push ends.
    #[cfg(test)]
    pub(crate) fn release_inflight_for_test(&self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }

    /// One message lost: the site's counter, then the global one (`Release`), so a
    /// consumer that sees the global count sees the site's.
    fn lose(&self, site: u16) {
        let slot = SITE_SLOT
            .get(usize::from(site))
            .map_or(0, |slot| usize::from(*slot));
        self.sites[slot].fetch_add(1, Ordering::Release);
        self.lost.fetch_add(1, Ordering::Release);
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
    /// `None` for a recorder created before the transport was published (the first
    /// enabled startup): every message of it is an `Unavailable` drop.
    transport: Option<Arc<Transport>>,
    /// Where an `Unavailable` drop is counted (a recorder without a transport).
    unavailable: Option<&'static AtomicU64>,
    next_idx: AtomicU16,
    /// The site of the decision, set when the ticket is taken (0 before).
    site: AtomicU16,
    closed: AtomicBool,
    /// `run(Pre)` was entered: the flight has done (or is doing) observable work.
    pre_entered: AtomicBool,
    /// A ticket was taken: `seq` was assigned, so the flight is past D entry.
    seq_taken: AtomicBool,
    header_submitted: AtomicBool,
    pre: PreCapture,
}

impl ShadowRecorder {
    /// A recorder with no transport yet: the comparison thread has not published it
    /// (the first enabled startup). It never blocks and never allocates; every
    /// message it would send is counted in `counter` as an `Unavailable` drop, and
    /// nothing is queued, so no counter or `seq` exists for it (D-S2c-3 minor 2).
    pub(crate) fn unavailable(counter: &'static AtomicU64) -> Self {
        Self {
            rec: RecorderId(0),
            transport: None,
            unavailable: Some(counter),
            next_idx: AtomicU16::new(0),
            site: AtomicU16::new(0),
            closed: AtomicBool::new(false),
            pre_entered: AtomicBool::new(false),
            seq_taken: AtomicBool::new(false),
            header_submitted: AtomicBool::new(false),
            pre: PreCapture::default(),
        }
    }

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
            if let Some(transport) = &self.transport {
                transport.count_overflow();
            }
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
    /// entries at decision time) for a decision at `site` (the C0.3 site number).
    /// The recorder keeps the site, so any later message of the flight that the ring
    /// cannot take is counted against it; the ticket counts the loss if it is dropped
    /// without a submit.
    pub(crate) fn ticket(&self, site: u16) -> ShadowTicket<'_> {
        let seq = self
            .transport
            .as_ref()
            .map_or(0, |transport| transport.seq.fetch_add(1, Ordering::Relaxed));
        self.site.store(site, Ordering::Relaxed);
        self.seq_taken.store(true, Ordering::Relaxed);
        ShadowTicket {
            seq,
            epoch: self
                .transport
                .as_ref()
                .map_or(0, |transport| transport.lost.load(Ordering::Acquire)),
            recorder: self,
            sent: false,
        }
    }

    fn send(&self, msg: ShadowMsg) {
        match &self.transport {
            Some(transport) => transport.send(msg, self.site.load(Ordering::Relaxed)),
            None => self.count_unavailable(),
        }
    }

    fn count_unavailable(&self) {
        if let Some(counter) = self.unavailable {
            counter.fetch_add(1, Ordering::Relaxed);
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
            self.send(ShadowMsg::Close {
                rec: self.rec,
                completed,
            });
        } else if !self.seq_taken.load(Ordering::Relaxed)
            && self.pre_entered.load(Ordering::Relaxed)
        {
            // `run(Pre)` began and D was never entered: no decision, no `seq`.
            // (A ticket that was taken and never submitted reports its own loss,
            // and a recorder that never entered `Pre` did nothing to report.)
            self.send(ShadowMsg::PreAbandoned { rec: self.rec });
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
    /// `lost` when the ticket was taken: one atomic load.
    epoch: u64,
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
            match &self.recorder.transport {
                Some(transport) => transport.lose(self.recorder.site.load(Ordering::Relaxed)),
                None => self.recorder.count_unavailable(),
            }
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
    /// A value the live path actually read (before the conditional that depends on
    /// it): `SPEC` sends `SpeculativeFlowing` here, once before each post-D
    /// `Cond{SpecFlowing}`; the comparator requires it (no header fallback).
    Observed(ObservedFact),
    /// A ticker registration attempt after D.
    Ticker(TickerGate),
    /// A `PostDT:SD` whose spawner returned before the gate: no gate exists, and
    /// this is the witness that says why (D-S2c-6). Numeric only.
    TickerSkipped {
        /// The recording the spawner was asked for (numeric).
        target: RecordingId,
        reason: TickerSkipReason,
    },
    /// An H1 / H2 operation after D.
    History(LiveOp),
    /// The command's reply kind.
    Reply(ReplyKind),
    /// The flight closes with no reply of its own, for this reason.
    NoReply(NoReplyReason),
}

/// Submit the header right after D is released and before the first `PostDT` effect
/// (C8.2): one `push`, no lock, never blocks. The header's `seq` and `rec` must be
/// the ticket's and its recorder's.
pub(crate) fn submit(mut ticket: ShadowTicket<'_>, mut header: ShadowHeader) {
    debug_assert_eq!(
        header.seq, ticket.seq,
        "a header is submitted under its own ticket"
    );
    debug_assert_eq!(
        header.rec, ticket.recorder.rec,
        "a header names its ticket's recorder"
    );
    header.epoch = ticket.epoch;
    ticket.sent = true;
    ticket
        .recorder
        .header_submitted
        .store(true, Ordering::Relaxed);
    ticket.recorder.send(ShadowMsg::Header(header));
}

/// An effect that ran after D: one `push` of a `Witness`.
pub(crate) fn witness(
    rec: &ShadowRecorder,
    idx: u16,
    placement: Placement,
    effect: WitnessedEffect,
) {
    rec.send(ShadowMsg::Witness {
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
        Mark::Observed(fact) => ShadowMsg::Observed { rec: rec_id, fact },
        Mark::Ticker(gate) => ShadowMsg::Ticker { rec: rec_id, gate },
        Mark::TickerSkipped { target, reason } => ShadowMsg::TickerSkipped {
            rec: rec_id,
            target,
            reason,
        },
        Mark::History(op) => ShadowMsg::History { rec: rec_id, op },
        Mark::Reply(kind) => ShadowMsg::Reply { rec: rec_id, kind },
        Mark::NoReply(reason) => ShadowMsg::NoReply {
            rec: rec_id,
            reason,
        },
    };
    rec.send(msg);
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
                comparator.handle_msg(msg);
            }
        }
    }
}
