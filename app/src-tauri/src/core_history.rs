//! `AppState.core_history`: the operation history of the dictation core
//! (appendix C1.4.1, decision D1), the one lock the refactor adds.
//!
//! * **H1 `pending_ops`**: continuations a flight has registered and not yet
//!   consumed, keyed by an [`OperationId`] the **shell** draws from the static
//!   [`NEXT_OP`] (never the core, never the shadow).
//! * **H2 `pipelines`**: the transcription pipelines that are running.
//!
//! ## The lock is a leaf, by construction
//!
//! [`CoreHistoryLock`] is the only code that ever locks the mutex, every public
//! method is **one** short critical section (`critical`), and the guard never
//! escapes: there is no method that hands out a guard or runs a caller's
//! closure under it. So the lock is never held while another lock is taken,
//! while awaiting, or while an effect runs (C1.4.1 rule 1); callers take it
//! only after D when D is held (order D → `core_history`). The `HELD` flag and
//! [`assert_not_held`] back that up at runtime for the executor and adapter.
//!
//! ## Rules this module implements
//!
//! * **Ids** are monotonic and never reused; `NEXT_OP` is a `static` outside
//!   [`CoreHistory`] shared by every history, live and shadow.
//! * **`validate`** is an observation (`Outstanding` / `Unknown`); no decision
//!   branches into a no-effect outcome on it (rule 3). [`CoreHistoryLock::consume`]
//!   is that protocol: validate, run the site's decide **always**, retire.
//! * **`ret_if_matches(token)`** compares the op id, the recording id and the
//!   kind under one lock acquisition and removes the entry only on a full
//!   match; a mismatch preserves it (rule 4).
//! * **Nothing is evicted.** An outstanding op leaves only by consumption or its
//!   owner's `Drop` ([`ContinuationGuard`]); a recording-id advance retires
//!   nothing. The 8 / 64 / 16 caps are diagnostic high-water values (rule 5).
//!
//! Not yet called from a production path (Stage 2a PR 0): the decide functions
//! that register and consume continuations arrive with the family PRs.

use crate::MutexExt;
use murmur_dictation_core::decision::{ContKind, Continuation, ContinuationSpec, OpStatus};
use murmur_dictation_core::ids::{OperationId, RecordingId};
use murmur_dictation_core::state::DictationCore;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The one process-global operation counter, used by live and shadow (C1.4.1).
/// A `static` **outside** [`CoreHistory`]; only [`draw_op_id`] reads it.
static NEXT_OP: AtomicU64 = AtomicU64::new(1);

/// Diagnostic high-water thresholds (C1.4.1 rule 5). Crossing one counts and
/// (in test builds) asserts; it never evicts, refuses or changes a decision.
pub(crate) const OPS_PER_RECORDING_THRESHOLD: usize = 8;
pub(crate) const OPS_TOTAL_THRESHOLD: usize = 64;
pub(crate) const PIPELINES_THRESHOLD: usize = 16;

thread_local! {
    /// `true` while this thread is inside a `core_history` critical section.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Draw the next operation id. Shell only: the core never allocates one and
/// the shadow replays the id the live shell drew (C1.4.1 rule 2).
pub(crate) fn draw_op_id() -> OperationId {
    OperationId(NEXT_OP.fetch_add(1, Ordering::SeqCst))
}

/// Debug check for code that must not run under `core_history` (the executor's
/// `run`, the field adapter): the lock is a leaf and no effect runs under it.
pub(crate) fn assert_not_held() {
    debug_assert!(
        !HELD.with(Cell::get),
        "core_history is held: no effect, lock or await may run under the leaf lock"
    );
}

/// `true` while the current thread is inside a `core_history` critical section.
pub(crate) fn is_held_on_this_thread() -> bool {
    HELD.with(Cell::get)
}

/// What H1 stores for an outstanding continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingOp {
    pub(crate) recording_id: RecordingId,
    pub(crate) kind: ContKind,
}

/// H2 value: the state of a registered pipeline. "Draining" (not current, or
/// `id <= cancelled_through`) is derived from the core, never stored (C3.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PipelinePhase {
    /// Registered by R203.9 / R301.5, removed by R204a / R302.1.
    Running,
}

/// Maximum outstanding counts seen so far (diagnostic only, C1.4.1 rule 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HighWater {
    pub(crate) ops_per_recording: usize,
    pub(crate) ops_total: usize,
    pub(crate) pipelines: usize,
}

/// Counters for events the history survives and reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HistCounters {
    /// A `validate` answered `Unknown`, or `ret_if_matches` met a mismatched
    /// token (the entry is preserved).
    pub(crate) unknown_op: u64,
    /// `ret_if_matches` for an op that is already gone (a duplicate or late
    /// retirement): a counted no-op.
    pub(crate) duplicate_op: u64,
    /// A high-water threshold was crossed.
    pub(crate) over_threshold: u64,
}

/// What `ret_if_matches` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retired {
    /// Op id, recording id and kind all matched: the entry was removed.
    Removed,
    /// The op exists but the recording id or kind differs: the entry stays.
    Mismatch,
    /// No such op: a counted no-op.
    Absent,
}

/// Pipeline facts for `Occupancy`, read outside D (C3.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OccupancySnapshot {
    /// A pipeline is registered for the current, uncancelled recording.
    pub(crate) pipeline_running: bool,
    /// A pipeline is registered for a recording that is no longer current or
    /// was cancelled. `admit()` allows a start over it.
    pub(crate) pipeline_draining: bool,
}

/// H1, H2 and their diagnostics. Only reachable through [`CoreHistoryLock`].
#[derive(Debug)]
pub(crate) struct CoreHistory {
    pending_ops: BTreeMap<OperationId, PendingOp>,
    pipelines: BTreeMap<RecordingId, PipelinePhase>,
    high_water: HighWater,
    counters: HistCounters,
    /// Test builds assert when a threshold is crossed (rule 5); tests that
    /// exercise the no-eviction rule past the thresholds switch it off.
    assert_on_over_threshold: bool,
}

// Not derivable: the assert flag defaults to `true` in test builds only.
#[allow(clippy::derivable_impls)]
impl Default for CoreHistory {
    fn default() -> Self {
        Self {
            pending_ops: BTreeMap::new(),
            pipelines: BTreeMap::new(),
            high_water: HighWater::default(),
            counters: HistCounters::default(),
            assert_on_over_threshold: cfg!(test),
        }
    }
}

impl CoreHistory {
    fn over_threshold(&mut self, what: &str) {
        self.counters.over_threshold += 1;
        if self.assert_on_over_threshold {
            debug_assert!(false, "core_history over its diagnostic threshold: {what}");
        }
    }

    fn register(&mut self, op: OperationId, spec: ContinuationSpec) {
        self.pending_ops.insert(
            op,
            PendingOp {
                recording_id: spec.recording_id,
                kind: spec.kind,
            },
        );
        let for_recording = self
            .pending_ops
            .values()
            .filter(|pending| pending.recording_id == spec.recording_id)
            .count();
        let total = self.pending_ops.len();
        self.high_water.ops_per_recording = self.high_water.ops_per_recording.max(for_recording);
        self.high_water.ops_total = self.high_water.ops_total.max(total);
        if for_recording > OPS_PER_RECORDING_THRESHOLD {
            self.over_threshold("ops per recording");
        }
        if total > OPS_TOTAL_THRESHOLD {
            self.over_threshold("ops total");
        }
    }

    fn validate(&mut self, token: Continuation) -> OpStatus {
        let matches = self.pending_ops.get(&token.op).is_some_and(|pending| {
            pending.recording_id == token.recording_id && pending.kind == token.kind
        });
        if matches {
            OpStatus::Outstanding
        } else {
            self.counters.unknown_op += 1;
            OpStatus::Unknown
        }
    }

    fn ret_if_matches(&mut self, token: Continuation) -> Retired {
        match self.pending_ops.get(&token.op) {
            Some(pending)
                if pending.recording_id == token.recording_id && pending.kind == token.kind =>
            {
                self.pending_ops.remove(&token.op);
                Retired::Removed
            }
            Some(_) => {
                self.counters.unknown_op += 1;
                Retired::Mismatch
            }
            None => {
                self.counters.duplicate_op += 1;
                Retired::Absent
            }
        }
    }

    fn register_pipeline(&mut self, id: RecordingId) {
        self.pipelines.insert(id, PipelinePhase::Running);
        let total = self.pipelines.len();
        self.high_water.pipelines = self.high_water.pipelines.max(total);
        if total > PIPELINES_THRESHOLD {
            self.over_threshold("pipelines");
        }
    }
}

/// `AppState.core_history`: the leaf `std::sync::Mutex<CoreHistory>`.
///
/// Cloning shares the same history (an `Arc`), which is how a
/// [`ContinuationGuard`] moved into a spawned task keeps a `'static` handle.
#[derive(Clone, Debug)]
pub(crate) struct CoreHistoryLock {
    inner: Arc<Mutex<CoreHistory>>,
    /// Test builds count lock acquisitions so a test can prove an operation is
    /// one critical section.
    #[cfg(test)]
    acquisitions: Arc<AtomicU64>,
}

impl Default for CoreHistoryLock {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CoreHistory::default())),
            #[cfg(test)]
            acquisitions: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// Sets `HELD` for the duration of a critical section and clears it on every
/// exit, including a panic.
struct HeldFlag;

impl HeldFlag {
    fn set() -> Self {
        let already = HELD.with(|held| held.replace(true));
        assert!(
            !already,
            "core_history entered re-entrantly on one thread (the lock is a non-reentrant leaf)"
        );
        Self
    }
}

impl Drop for HeldFlag {
    fn drop(&mut self) {
        HELD.with(|held| held.set(false));
    }
}

impl CoreHistoryLock {
    /// One short critical section. Private: nothing outside this module can run
    /// a closure under the lock, so the guard cannot span another lock, an await
    /// or an effect.
    fn critical<R>(&self, body: impl FnOnce(&mut CoreHistory) -> R) -> R {
        let _held = HeldFlag::set();
        #[cfg(test)]
        self.acquisitions.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.inner.lock_or_recover();
        body(&mut guard)
    }

    /// `reg`: draw an id from [`NEXT_OP`] (outside the lock), register the
    /// continuation and return the shell-held token. Called by the shell right
    /// after `decide_*` returns, before any effect runs (C4.3).
    pub(crate) fn reg(&self, spec: ContinuationSpec) -> Continuation {
        let op = draw_op_id();
        self.critical(|history| history.register(op, spec));
        Continuation {
            recording_id: spec.recording_id,
            op,
            kind: spec.kind,
        }
    }

    /// `validate`: the status a consuming decide receives as an observation.
    /// `Unknown` (absent, retired, forged or wrong kind) is counted; the site's
    /// own rule still runs (C1.4.1 rule 3).
    pub(crate) fn validate(&self, token: Continuation) -> OpStatus {
        self.critical(|history| history.validate(token))
    }

    /// `ret_if_matches(token)`: op id, recording id and kind compared and the
    /// entry removed under **one** lock acquisition (C1.4.1 rule 4).
    pub(crate) fn ret_if_matches(&self, token: Continuation) -> Retired {
        self.critical(|history| history.ret_if_matches(token))
    }

    /// The shell protocol for a continuation-consuming input (C4.3 "Validation,
    /// not branching"): `validate` (lock, released), run the site's `decide`
    /// **whatever the status** with no history lock held, then
    /// `ret_if_matches` (lock, released). A site never skips its stale rule
    /// because the status is `Unknown`.
    pub(crate) fn consume<R>(&self, token: Continuation, decide: impl FnOnce(OpStatus) -> R) -> R {
        let status = self.validate(token);
        let outcome = decide(status);
        self.ret_if_matches(token);
        outcome
    }

    /// With the shadow compiled in: [`Self::consume`] that also returns what the
    /// retirement answered, the shadow's H1 witness (D-S2c-9 c: handed back to the
    /// consumer, never stashed for a later lookup).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn consume_observed<R>(
        &self,
        token: Continuation,
        decide: impl FnOnce(OpStatus) -> R,
    ) -> (R, Retired) {
        let status = self.validate(token);
        let outcome = decide(status);
        let retired = self.ret_if_matches(token);
        (outcome, retired)
    }

    /// `H2 reg` (R203.9, R301.5).
    pub(crate) fn reg_pipeline(&self, id: RecordingId) {
        self.critical(|history| history.register_pipeline(id));
    }

    /// Identity of this history (never dereferenced): the key under which a
    /// deferred item is run by an entry of the same state.
    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    /// `H2 ret` (R204a `PipelineExit` / `Dropped`, R302.1): `true` if it was
    /// registered.
    pub(crate) fn ret_pipeline(&self, id: RecordingId) -> bool {
        self.critical(|history| history.pipelines.remove(&id).is_some())
    }

    /// Pipeline facts for `Occupancy`. The ids are copied under the lock; the
    /// draining predicate runs after it is released.
    pub(crate) fn snapshot_for_occupancy(&self, core: &DictationCore) -> OccupancySnapshot {
        let ids: Vec<RecordingId> =
            self.critical(|history| history.pipelines.keys().copied().collect());
        let mut snapshot = OccupancySnapshot::default();
        for id in ids {
            if core.is_draining(id) {
                snapshot.pipeline_draining = true;
            } else {
                snapshot.pipeline_running = true;
            }
        }
        snapshot
    }

    /// Outstanding operation count.
    pub(crate) fn outstanding_ops(&self) -> usize {
        self.critical(|history| history.pending_ops.len())
    }

    /// Whether `token`'s op is outstanding with the same recording id and kind.
    /// Unlike [`validate`](Self::validate) it does not count.
    pub(crate) fn is_outstanding(&self, token: Continuation) -> bool {
        self.critical(|history| {
            history.pending_ops.get(&token.op).is_some_and(|pending| {
                pending.recording_id == token.recording_id && pending.kind == token.kind
            })
        })
    }

    /// The `(recording, kind)` of every outstanding operation, oldest first (test builds):
    /// which operations are outstanding, not only how many.
    #[cfg(test)]
    pub(crate) fn outstanding_for_test(&self) -> Vec<(RecordingId, ContKind)> {
        self.critical(|history| {
            history
                .pending_ops
                .values()
                .map(|pending| (pending.recording_id, pending.kind))
                .collect()
        })
    }

    /// Registered pipeline count.
    pub(crate) fn pipeline_count(&self) -> usize {
        self.critical(|history| history.pipelines.len())
    }

    /// Diagnostic high-water values.
    pub(crate) fn high_water(&self) -> HighWater {
        self.critical(|history| history.high_water)
    }

    /// Diagnostic counters.
    pub(crate) fn counters(&self) -> HistCounters {
        self.critical(|history| history.counters)
    }

    /// Test builds: stop the over-threshold `debug_assert!` (the counters still
    /// count). Used by tests that prove nothing is evicted past the thresholds.
    #[cfg(test)]
    pub(crate) fn allow_over_threshold_for_test(&self) {
        self.critical(|history| history.assert_on_over_threshold = false);
    }

    /// Test builds: lock acquisitions so far.
    #[cfg(test)]
    pub(crate) fn acquisitions(&self) -> u64 {
        self.acquisitions.load(Ordering::SeqCst)
    }

    /// Test builds: run `body` while this thread is inside a critical section,
    /// to prove that code which must not run under the leaf lock refuses to.
    #[cfg(test)]
    pub(crate) fn hold_for_test<R>(&self, body: impl FnOnce() -> R) -> R {
        self.critical(|_| body())
    }
}

/// The owner of a registered continuation (C1.4.1 rule 4). It sits in the
/// flight's stack frame (start, stop and cancel are straight-line async fns) or
/// moves into the spawned task (`RS`); its `Drop` retires the op through
/// `ret_if_matches(token)` (leaf lock, no D), so a future dropped mid-flight, a
/// panic unwind or a task that never runs cannot leave the op outstanding.
#[derive(Debug)]
pub(crate) struct ContinuationGuard {
    history: CoreHistoryLock,
    token: Continuation,
    consumed: bool,
    /// The H1 retirement this guard's consumption made, for the one flight that
    /// witnesses it (taken by that site; nothing unless the shadow is armed).
    #[cfg(feature = "dictation-shadow")]
    retired: Option<crate::dictation_shadow::types::LiveOp>,
}

impl ContinuationGuard {
    /// Register `spec` on `history` and own the resulting token.
    pub(crate) fn register(history: &CoreHistoryLock, spec: ContinuationSpec) -> Self {
        Self {
            history: history.clone(),
            token: history.reg(spec),
            consumed: false,
            #[cfg(feature = "dictation-shadow")]
            retired: None,
        }
    }

    /// The shell-held token.
    pub(crate) fn token(&self) -> Continuation {
        self.token
    }

    /// Identity of the history this guard retires on (see
    /// [`CoreHistoryLock::identity`]).
    pub(crate) fn history_identity(&self) -> usize {
        self.history.identity()
    }

    /// Whether dropping this guard still has an op to retire.
    pub(crate) fn is_armed(&self) -> bool {
        !self.consumed
    }

    /// Move the retirement duty into a new guard and leave this one inert: an owner
    /// dropped inside a core span hands it to the deferred queue (D-S2-4, D-S2-6)
    /// instead of retiring on the spot.
    pub(crate) fn detach(&mut self) -> Self {
        let moved = Self {
            history: self.history.clone(),
            token: self.token,
            consumed: self.consumed,
            #[cfg(feature = "dictation-shadow")]
            retired: None,
        };
        self.consumed = true;
        moved
    }

    /// Consume the continuation (the shell protocol of
    /// [`CoreHistoryLock::consume`]); the guard's `Drop` then has nothing to
    /// retire. The guard is disarmed only **after** consumption and retirement
    /// returned: if `decide` panics, the owner's `Drop` still retires the op
    /// (nothing is evicted, so a skipped retirement would leak it forever).
    #[cfg(not(feature = "dictation-shadow"))]
    pub(crate) fn consume<R>(&mut self, decide: impl FnOnce(OpStatus) -> R) -> R {
        let outcome = self.history.consume(self.token, decide);
        self.consumed = true;
        outcome
    }

    /// With the shadow compiled in: the same consumption, keeping the retirement's
    /// answer (armed only) for [`Self::take_retired`].
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn consume<R>(&mut self, decide: impl FnOnce(OpStatus) -> R) -> R {
        let (outcome, retired) = self.history.consume_observed(self.token, decide);
        if crate::dictation_shadow::enabled() {
            self.retired = Some(crate::dictation_executor::shadow::retired(
                self.token, retired,
            ));
        }
        self.consumed = true;
        outcome
    }

    /// The retirement this guard's consumption made, once (the flight's H1 witness).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn take_retired(&mut self) -> Option<crate::dictation_shadow::types::LiveOp> {
        #[cfg(test)]
        crate::dictation_executor::probe::shadow_read();
        self.retired.take()
    }
}

impl Drop for ContinuationGuard {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        // D-S2-6: dropped inside a core span (a listener drops the flight that
        // holds it), the retirement joins the deferred FIFO behind any finish queued
        // earlier and runs when the outermost entry drains, after D is released.
        // Outside a span it retires at once.
        if crate::dictation_executor::executor::in_core() {
            crate::commands::recording::defer_op_retirement(self.detach());
            return;
        }
        self.history.ret_if_matches(self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(recording: u64, kind: ContKind) -> ContinuationSpec {
        ContinuationSpec {
            recording_id: RecordingId(recording),
            kind,
        }
    }

    #[test]
    fn ids_are_monotonic_and_shared_by_every_history() {
        let live = CoreHistoryLock::default();
        let other = CoreHistoryLock::default();
        let a = live.reg(spec(1, ContKind::CaptureStart));
        let b = other.reg(spec(1, ContKind::CaptureStart));
        let c = live.reg(spec(1, ContKind::CaptureStop));
        assert!(a.op < b.op && b.op < c.op, "one process-global counter");
    }

    #[test]
    fn registering_then_consuming_retires_exactly_once() {
        let history = CoreHistoryLock::default();
        let token = history.reg(spec(3, ContKind::CaptureStart));
        assert_eq!(history.validate(token), OpStatus::Outstanding);
        assert_eq!(history.ret_if_matches(token), Retired::Removed);
        assert_eq!(history.validate(token), OpStatus::Unknown);
        assert_eq!(history.ret_if_matches(token), Retired::Absent);
        assert_eq!(
            history.counters(),
            HistCounters {
                unknown_op: 1,
                duplicate_op: 1,
                over_threshold: 0
            }
        );
    }

    #[test]
    fn a_recording_id_advance_retires_nothing() {
        let history = CoreHistoryLock::default();
        let token = history.reg(spec(1, ContKind::AutoStop));
        // The core's current id moves on; nothing in the history reacts.
        let core = DictationCore {
            current_id: RecordingId(9),
            ..DictationCore::default()
        };
        let _ = history.snapshot_for_occupancy(&core);
        assert!(history.is_outstanding(token));
    }

    #[test]
    fn pipelines_register_and_retire_and_feed_occupancy() {
        let history = CoreHistoryLock::default();
        history.reg_pipeline(RecordingId(2));
        history.reg_pipeline(RecordingId(3));
        let core = DictationCore {
            current_id: RecordingId(3),
            cancelled_through: RecordingId(0),
            ..DictationCore::default()
        };
        assert_eq!(
            history.snapshot_for_occupancy(&core),
            OccupancySnapshot {
                pipeline_running: true,
                pipeline_draining: true
            }
        );
        assert!(history.ret_pipeline(RecordingId(2)));
        assert!(!history.ret_pipeline(RecordingId(2)), "already removed");
        assert_eq!(history.pipeline_count(), 1);
    }

    #[test]
    #[should_panic(expected = "over its diagnostic threshold")]
    fn crossing_a_threshold_asserts_in_test_builds() {
        let history = CoreHistoryLock::default();
        for _ in 0..=OPS_PER_RECORDING_THRESHOLD {
            history.reg(spec(1, ContKind::CaptureStart));
        }
    }

    #[test]
    fn crossing_a_threshold_counts_and_never_evicts_when_the_assert_is_off() {
        let history = CoreHistoryLock::default();
        history.allow_over_threshold_for_test();
        for _ in 0..=OPS_PER_RECORDING_THRESHOLD {
            history.reg(spec(1, ContKind::CaptureStart));
        }
        assert_eq!(history.outstanding_ops(), OPS_PER_RECORDING_THRESHOLD + 1);
        assert_eq!(history.counters().over_threshold, 1);
        assert_eq!(
            history.high_water().ops_per_recording,
            OPS_PER_RECORDING_THRESHOLD + 1
        );
    }
}
