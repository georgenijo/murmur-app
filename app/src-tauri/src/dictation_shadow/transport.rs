//! The live side of shadow capture: the bounded channel, the flight-owned
//! recorder and the header ticket (appendix C8.2).
//!
//! **The capture path never blocks and takes no lock.** `witness`, `mark`, `submit`
//! and the two helpers they call (`send`, `try_deliver`) use `try_send` only: a full
//! channel drops the message and counts it (`dropped`), it never waits. They read
//! atomics and write one channel slot. The only lock in this file is the comparator
//! mutex of the **synchronous test mode** (`set_sync`), reached through
//! `deliver_sync`, a separate function that the live path never enters unless a
//! test asked for it; the `shadow_capture_takes_no_lock` test pins the bodies of
//! the others.
//!
//! **No thread-local is used for correlation** (C8.2): a flight's later phases run
//! on other threads and tasks, so the recorder travels with the flight instead.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use murmur_dictation_core::effects::Placement;

use super::compare::Comparator;
use super::types::{
    CondWitness, MirrorField, MirrorValue, MirrorWitness, PredId, RecorderId, ReplyKind,
    ShadowHeader, ShadowMsg, WitnessedEffect,
};

/// The one bounded channel's capacity (C8.2).
pub(crate) const CHANNEL_CAPACITY: usize = 256;

/// Inline `Pre` buffer capacity: the largest C3 `Pre` list is 4 today (R216.1), so
/// 8 leaves room without a heap allocation (`shadow_pre_buffer_covers_c3`).
pub(crate) const PRE_CAPACITY: usize = 8;

/// The recorder id counter starts here, so a `RecorderId` can never be mistaken for
/// a header `seq` (which starts at 1): they are different namespaces by construction.
const REC_BASE: u64 = 1 << 32;

/// Everything both sides share: the channel's sending end, the counters the live
/// side bumps and the comparator the receiving side owns.
pub(crate) struct Transport {
    tx: SyncSender<ShadowMsg>,
    /// Messages `try_send` could not queue (channel full or receiver gone).
    dropped: AtomicU64,
    /// `Pre` buffer overflows (a flight recorded more than `PRE_CAPACITY`).
    overflow: AtomicU64,
    /// Next header `seq`; the first one handed out is 1.
    seq: AtomicU64,
    /// Next recorder id.
    rec: AtomicU64,
    /// Test mode: compare on the calling thread instead of the channel.
    sync: AtomicBool,
    comparator: Arc<Mutex<Comparator>>,
}

impl Transport {
    /// A transport over a bounded channel of `capacity`, and the receiving end.
    pub(crate) fn new(capacity: usize, comparator: Comparator) -> (Arc<Self>, Receiver<ShadowMsg>) {
        let (tx, rx) = sync_channel(capacity);
        let transport = Arc::new(Self {
            tx,
            dropped: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            seq: AtomicU64::new(1),
            rec: AtomicU64::new(REC_BASE),
            sync: AtomicBool::new(false),
            comparator: Arc::new(Mutex::new(comparator)),
        });
        (transport, rx)
    }

    /// The comparator the receiving thread (or the synchronous mode) drives.
    pub(crate) fn comparator(&self) -> &Arc<Mutex<Comparator>> {
        &self.comparator
    }

    /// Compare on the calling thread, no channel (E7: the golden replay needs it
    /// fully deterministic). A test mode: it is the only path that takes the
    /// comparator lock from a capture call.
    pub(crate) fn set_sync(&self, on: bool) {
        self.sync.store(on, Ordering::SeqCst);
    }

    /// Messages the channel could not queue, since the last call.
    pub(crate) fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// `Pre` buffer overflows, since the last call.
    pub(crate) fn take_overflow(&self) -> u64 {
        self.overflow.swap(0, Ordering::Relaxed)
    }

    /// A new flight-local recorder, created **before** `run(Pre)` (C4.1 step 2).
    pub(crate) fn recorder(self: &Arc<Self>) -> ShadowRecorder {
        ShadowRecorder {
            rec: RecorderId(self.rec.fetch_add(1, Ordering::Relaxed)),
            transport: Arc::clone(self),
            flags: Arc::new(Flags::default()),
            next_idx: AtomicU16::new(0),
            closed: AtomicBool::new(false),
            pre: PreBuffer::default(),
        }
    }

    fn send(&self, msg: ShadowMsg) {
        if self.sync.load(Ordering::Relaxed) {
            self.deliver_sync(msg);
        } else {
            self.try_deliver(msg);
        }
    }

    /// The live path: `try_send` and nothing else.
    fn try_deliver(&self, msg: ShadowMsg) {
        if self.tx.try_send(msg).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Test mode: apply the message on the calling thread.
    fn deliver_sync(&self, msg: ShadowMsg) {
        let Ok(mut comparator) = self.comparator.lock() else {
            return;
        };
        comparator.note_dropped(self.take_dropped());
        comparator.note_overflow(self.take_overflow());
        comparator.handle_msg(msg);
    }
}

/// Flags the ticket and the recorder share, so each knows what the other did.
#[derive(Default)]
struct Flags {
    /// A ticket was taken: `seq` was assigned, so the flight is past D entry and an
    /// unsent header is reported by the ticket's `Lost`, not by `PreAbandoned`.
    seq_taken: AtomicBool,
    header_submitted: AtomicBool,
}

/// The `Pre` witnesses buffered inline before the header exists (C8.2): owned by
/// the flight, written on the decision's own thread, so it needs no lock and no
/// allocation.
#[derive(Default)]
struct PreBuffer {
    effects: [Option<WitnessedEffect>; PRE_CAPACITY],
    effect_len: usize,
    conditionals: [Option<CondWitness>; PRE_CAPACITY],
    conditional_len: usize,
    mirrors: [Option<MirrorWitness>; PRE_CAPACITY],
    mirror_len: usize,
}

/// The `Pre` witnesses a recorder buffered, ready to become the header's
/// `pre_effects`, `pre_conditionals` and `pre_mirrors`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PreCapture {
    pub effects: Vec<WitnessedEffect>,
    pub conditionals: Vec<CondWitness>,
    pub mirrors: Vec<MirrorWitness>,
}

/// The flight-owned recorder: created before `run(Pre)`, moved with the flight
/// (the command future, the spawned continuation, the guards, the supervisor-arm
/// closure; wrap it in an `Arc` to share it between tasks of one flight).
///
/// * `Pre` witnesses go to the inline buffer, then into the header (`take_pre`).
/// * Later witnesses are incremental messages, sent only after the header was
///   submitted, so every later message is causally after its header.
/// * `finish` sends `Close{completed:true}`. A drop without it sends
///   `Close{completed:false}` once the header was submitted, or `PreAbandoned`
///   (carrying the buffered `Pre` witnesses) when the flight was dropped before D
///   entry. A drop after the ticket was taken but before the header was sent is
///   the ticket's `Lost`.
pub(crate) struct ShadowRecorder {
    rec: RecorderId,
    transport: Arc<Transport>,
    flags: Arc<Flags>,
    next_idx: AtomicU16,
    closed: AtomicBool,
    pre: PreBuffer,
}

impl ShadowRecorder {
    /// The recorder's id (not a `seq`).
    pub(crate) fn id(&self) -> RecorderId {
        self.rec
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

    /// Buffer a `Pre` effect that actually ran. `false` (and a `witness_overflow`
    /// count) when the buffer is full.
    pub(crate) fn pre_effect(&mut self, effect: WitnessedEffect) -> bool {
        match self.pre.effects.get_mut(self.pre.effect_len) {
            Some(slot) => {
                *slot = Some(effect);
                self.pre.effect_len += 1;
                true
            }
            None => {
                self.transport.overflow.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Buffer a shell-conditional `Pre` effect (taken or skipped).
    pub(crate) fn pre_conditional(&mut self, pred: PredId, taken: bool) -> bool {
        match self.pre.conditionals.get_mut(self.pre.conditional_len) {
            Some(slot) => {
                *slot = Some(CondWitness { pred, taken });
                self.pre.conditional_len += 1;
                true
            }
            None => {
                self.transport.overflow.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Buffer the M4 slot's before/after value of a `Pre:CX-`.
    pub(crate) fn pre_mirror(&mut self, mirror: MirrorWitness) -> bool {
        match self.pre.mirrors.get_mut(self.pre.mirror_len) {
            Some(slot) => {
                *slot = Some(mirror);
                self.pre.mirror_len += 1;
                true
            }
            None => {
                self.transport.overflow.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Move the buffered `Pre` witnesses out, for the header.
    pub(crate) fn take_pre(&mut self) -> PreCapture {
        let pre = std::mem::take(&mut self.pre);
        PreCapture {
            effects: pre.effects.into_iter().flatten().collect(),
            conditionals: pre.conditionals.into_iter().flatten().collect(),
            mirrors: pre.mirrors.into_iter().flatten().collect(),
        }
    }

    /// Take the header's `seq` at the existing decision boundary (under D; D-less
    /// entries at decision time). The ticket sends `Lost{seq}` if it is dropped
    /// without a submit.
    pub(crate) fn ticket(&self) -> ShadowTicket {
        self.flags.seq_taken.store(true, Ordering::Relaxed);
        ShadowTicket {
            seq: self.transport.seq.fetch_add(1, Ordering::Relaxed),
            rec: self.rec,
            transport: Arc::clone(&self.transport),
            flags: Arc::clone(&self.flags),
            sent: false,
        }
    }

    /// The flight ran to its end: send `Close{completed:true}`. Idempotent. A flight
    /// that ends without ever submitting a header (it never reached D entry) is
    /// reported as abandoned, never silently lost.
    pub(crate) fn finish(&self) {
        self.close(true);
    }

    fn close(&self, completed: bool) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.flags.header_submitted.load(Ordering::Relaxed) {
            self.transport.send(ShadowMsg::Close {
                rec: self.rec,
                completed,
            });
        } else if !self.flags.seq_taken.load(Ordering::Relaxed) {
            // Before D entry: no decision, no `seq`. (A ticket that was taken and
            // never submitted reports itself as `Lost{seq}`.)
            let pre = self.snapshot_pre();
            self.transport.send(ShadowMsg::PreAbandoned {
                rec: self.rec,
                pre_effects: pre.effects,
                pre_conditionals: pre.conditionals,
            });
        }
    }

    fn snapshot_pre(&self) -> PreCapture {
        PreCapture {
            effects: self.pre.effects.iter().flatten().cloned().collect(),
            conditionals: self.pre.conditionals.iter().flatten().copied().collect(),
            mirrors: self.pre.mirrors.iter().flatten().copied().collect(),
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

/// The right to submit one header: `seq` was taken at the decision boundary. Sends
/// `Lost{seq}` from its `Drop` if the thread unwinds between D release and `submit`.
pub(crate) struct ShadowTicket {
    seq: u64,
    rec: RecorderId,
    transport: Arc<Transport>,
    flags: Arc<Flags>,
    sent: bool,
}

impl ShadowTicket {
    /// The header's `seq`.
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }

    /// The recorder the ticket was taken from.
    pub(crate) fn rec(&self) -> RecorderId {
        self.rec
    }
}

impl Drop for ShadowTicket {
    fn drop(&mut self) {
        if !self.sent {
            self.transport.send(ShadowMsg::Lost { seq: self.seq });
        }
    }
}

/// What `mark` sends: a conditional, a live-field mirror or the reply kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// A shell-conditional effect after D.
    Cond { pred: PredId, taken: bool },
    /// A live M-field written at effect time after D.
    Mirror {
        field: MirrorField,
        value: MirrorValue,
    },
    /// The command's reply kind.
    Reply(ReplyKind),
}

/// Submit the header right after D is released and before the first `PostDT` effect
/// (C8.2): `try_send`, no lock, never blocks. The header's `seq` and `rec` must be
/// the ticket's and its recorder's.
pub(crate) fn submit(mut ticket: ShadowTicket, header: ShadowHeader) {
    debug_assert_eq!(
        header.seq, ticket.seq,
        "a header is submitted under its own ticket"
    );
    debug_assert_eq!(
        header.rec, ticket.rec,
        "a header names its ticket's recorder"
    );
    ticket.sent = true;
    ticket.flags.header_submitted.store(true, Ordering::Relaxed);
    ticket.transport.send(ShadowMsg::Header(Box::new(header)));
}

/// An effect that ran after D: `try_send` of a `Witness`, no lock, never blocks.
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

/// A conditional, a mirror or the reply of a flight: `try_send`, no lock, never blocks.
pub(crate) fn mark(rec: &ShadowRecorder, mark: Mark) {
    let rec_id = rec.rec;
    let msg = match mark {
        Mark::Cond { pred, taken } => ShadowMsg::Cond {
            rec: rec_id,
            pred,
            taken,
        },
        Mark::Mirror { field, value } => ShadowMsg::Mirror {
            rec: rec_id,
            field,
            value,
        },
        Mark::Reply(kind) => ShadowMsg::Reply { rec: rec_id, kind },
    };
    rec.transport.send(msg);
}
