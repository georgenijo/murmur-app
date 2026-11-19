//! The common field adapter (appendix C1.4.2, decision D2): the **only** writer
//! of the live dictation fields M1-M4 and of the persisted core copy.
//!
//! | Method | Effect | Writes live | Writes `DictationState.core` |
//! |---|---|---|---|
//! | [`FieldAdapter::set_status`] | `ST` | M1 `status` | M1 |
//! | [`FieldAdapter::alloc_id`] | `ID+` | M2 `recording_id` | M2 |
//! | [`FieldAdapter::cancel_through`] | `MC` | M3 `cancelled_id` | nothing |
//! | [`FieldAdapter::set_context`] / [`FieldAdapter::clear_context`] | `CX±` | M4 `active_context` | nothing |
//!
//! **Persisted core = M1 and M2 only.** M3 and M4 stay authoritative in their
//! live slots and are read back as immutable observations at every entry
//! ([`FieldAdapter::observe_live`]); `MC` and `CX±` write only the live field.
//!
//! **Zero behaviour change.** Every method is today's write, under the lock the
//! caller already holds, at the point the caller already writes: it adds the
//! persisted M1/M2 copy to the write that was there. The persisted fields commit
//! at their effect, never at decide time, so a panic between `decide` and the
//! effect leaves core and live equal (X6).
//!
//! **Seeding.** [`FieldAdapter::seed_core`] is the only place outside the core
//! crate that builds a `PersistedCore` from values (`PersistedCore::new`): a
//! caller cannot replace the whole persisted value without an effect commit, and
//! a re-sync from another adapter method calls `seed_core`.
//! `persisted_core_new_has_one_caller` (SP5 style) fails unless exactly one call
//! exists and it is in `seed_core`.

use crate::dictation_admission::core_status;
use crate::state::{AppState, DictationState, DictationStatus};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::state::{DictationCore, LiveObs, LockFreeFacts, PersistedCore};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Times a persisted M1/M2 copy was found different from its live field when an
/// effect committed (counted, then re-synced; C8.6). Today every production
/// writer goes through the adapter, so this stays 0 outside tests that write
/// the live fields directly.
static CORE_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// The divergences counted so far (in a test build, less the ones a test planted on purpose:
/// [`forgive_planted_divergences`]).
pub(crate) fn core_divergences() -> u64 {
    let counted = CORE_DIVERGENCES.load(Ordering::SeqCst);
    #[cfg(test)]
    let counted = counted.saturating_sub(FORGIVEN_DIVERGENCES.load(Ordering::SeqCst));
    counted
}

/// Divergences a test planted on purpose, which the golden harness's invariant
/// (`core_divergences() <= baseline`) must not count (cancel cutover tests).
#[cfg(test)]
static FORGIVEN_DIVERGENCES: AtomicU64 = AtomicU64::new(0);

/// Test builds: every divergence counted, planted ones included.
#[cfg(test)]
pub(crate) fn core_divergences_counted() -> u64 {
    CORE_DIVERGENCES.load(Ordering::SeqCst)
}

/// Test builds: allow `planted` more divergences to be counted without the harness's
/// invariant seeing them; `0` clears the allowance.
#[cfg(test)]
pub(crate) fn forgive_planted_divergences(planted: u64) {
    FORGIVEN_DIVERGENCES.store(planted, Ordering::SeqCst);
}

/// The app's status for a core status (the inverse of [`core_status`]): how the
/// executor's `ST` effect names the live write.
pub(crate) fn dictation_status(status: murmur_dictation_core::status::Status) -> DictationStatus {
    use murmur_dictation_core::status::Status;
    match status {
        Status::Idle => DictationStatus::Idle,
        Status::Starting => DictationStatus::Starting,
        Status::Recording => DictationStatus::Recording,
        Status::Recovering => DictationStatus::Recovering,
        Status::Processing => DictationStatus::Processing,
    }
}

/// Zero-sized namespace for the field writes. Each method names the state it
/// writes; none takes a lock of its own.
pub(crate) struct FieldAdapter;

impl FieldAdapter {
    /// `ST`: today's `transition_dictation_status` body plus the M1 copy. The
    /// caller holds D (`dictation` is the guard's contents). Order is unchanged:
    /// the live write, the trace, then `output_quieting::observe_transition`.
    /// Returns the previous status.
    pub(crate) fn set_status(
        recording_id: u64,
        dictation: &mut DictationState,
        to: DictationStatus,
    ) -> DictationStatus {
        crate::core_history::assert_not_held();
        // Test builds: a point where a test observes `core_history` at this `In`
        // effect (before the write).
        #[cfg(test)]
        crate::barrier_hook::at("effect.set_status");
        let from = dictation.status;
        if dictation.core.status() != core_status(from) {
            CORE_DIVERGENCES.fetch_add(1, Ordering::SeqCst);
        }
        dictation.status = to;
        dictation.core.set_status(core_status(to));
        crate::commands::recording::emit_dictation_state_changed(recording_id, from, to);
        // Sends one message to the output-quieting worker; never waits on it.
        crate::output_quieting::observe_transition(
            recording_id,
            from,
            to,
            dictation.mute_speakers_while_dictating,
        );
        from
    }

    /// `ID+`: `next_recording_id` (`fetch_add(1) + 1`) plus the M2 copy. The
    /// caller holds D and T. A persisted copy that did not predict the live id
    /// is counted and re-synced to the live value, never ignored.
    pub(crate) fn alloc_id(app: &AppState, dictation: &mut DictationState) -> u64 {
        crate::core_history::assert_not_held();
        // Test builds: as for `set_status`, at the `ID+` effect.
        #[cfg(test)]
        crate::barrier_hook::at("effect.alloc_id");
        let id = app.next_recording_id();
        let predicted = dictation.core.alloc_id();
        if predicted != RecordingId(id) {
            CORE_DIVERGENCES.fetch_add(1, Ordering::SeqCst);
            // Re-sync through the one seeder: no other method builds a core.
            Self::seed_core(app, dictation);
        }
        id
    }

    /// `MC`: `cancel_recording` (`cancelled_id.fetch_max`). Live only: M3 is
    /// never stored in the core.
    pub(crate) fn cancel_through(app: &AppState, id: u64) {
        crate::core_history::assert_not_held();
        app.cancel_recording(id);
    }

    /// [`Self::cancel_through`], returning the shadow's witness of the write.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn cancel_through_observed(
        app: &AppState,
        id: u64,
    ) -> crate::dictation_shadow::types::MirrorWitness {
        crate::core_history::assert_not_held();
        app.cancel_recording_observed(id)
    }

    /// `CX+`: `set_active_context`. Live only: M4 is never stored in the core.
    pub(crate) fn set_context(
        app: &AppState,
        id: u64,
        snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
    ) {
        crate::core_history::assert_not_held();
        app.set_active_context(id, snapshot);
    }

    /// [`Self::set_context`], returning the shadow's witness of the write.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn set_context_observed(
        app: &AppState,
        id: u64,
        snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
    ) -> crate::dictation_shadow::types::MirrorWitness {
        crate::core_history::assert_not_held();
        app.set_active_context_observed(id, snapshot)
    }

    /// `CX-`: `clear_active_context(id)`, which also clears `vad_stream` and
    /// `stream_reuse` for the id. Stale-safe: clears only if the slot holds `id`.
    /// Live only.
    pub(crate) fn clear_context(app: &AppState, id: u64) -> bool {
        crate::core_history::assert_not_held();
        // Test builds: the first statement of a cancel's `PostDT` tail, where
        // another thread's input can land before the cancel's `TERM` claim.
        #[cfg(test)]
        crate::barrier_hook::at("adapter.clear_context");
        app.clear_active_context(id)
    }

    /// [`Self::clear_context`], returning the shadow's witness of the write.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn clear_context_observed(
        app: &AppState,
        id: u64,
    ) -> (bool, crate::dictation_shadow::types::MirrorWitness) {
        crate::core_history::assert_not_held();
        #[cfg(test)]
        crate::barrier_hook::at("adapter.clear_context");
        app.clear_active_context_observed(id)
    }

    /// The M3/M4 observations of the live slots (C1.2): `cancelled_id` is one
    /// atomic load and the context id is one leaf-mutex read, the same reads
    /// `DictationCore::project` performs and a cut-over entry will perform.
    pub(crate) fn observe_live(app: &AppState) -> LiveObs {
        #[cfg(test)]
        super::probe::view();
        LiveObs {
            cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
            context_for: app.active_context_id().map(RecordingId),
        }
    }

    /// The decision view of the supervisor-lifecycle sites (207-211, 216), built
    /// under the D hold the caller already has: M1 and M2 from the live fields
    /// (`project()`, authoritative until the lifecycle cutover) and the M3
    /// atomic. **M4 is left `None` on purpose**: no lifecycle decide function
    /// reads it (`Ready` asks for the context through
    /// `NeedObservation(ContextPresence)` after its stale check, decision
    /// D-S1-2), so the leaf-mutex read stays where today's code does it. The crate
    /// test `no_lifecycle_decision_reads_m3_or_m4` pins that.
    #[cfg_attr(test, track_caller)]
    pub(crate) fn project_lifecycle_view(
        app: &AppState,
        dictation: &DictationState,
    ) -> DictationCore {
        #[cfg(test)]
        super::probe::view();
        #[cfg(test)]
        super::probe::projected(std::panic::Location::caller());
        DictationCore::project(
            core_status(dictation.status),
            RecordingId(app.recording_id.load(Ordering::SeqCst)),
            LiveObs {
                cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
                context_for: None,
            },
        )
    }

    /// The decision view of the lifecycle family's D entries (207-211, 216) after the
    /// cutover (C9.7 b, C8.6): M1 and M2 come from the **persisted core**, not from the
    /// live fields, and M3 is the same atomic observation [`Self::project_lifecycle_view`]
    /// makes (M4 stays `None` for the same reason). `DictationCore::project` is not
    /// called.
    ///
    /// **The C8.6 assertion** runs here, on every lifecycle entry and under the D hold the
    /// entry already has: the persisted M1/M2 are compared with the live `status` and
    /// `recording_id`. A difference is **counted** ([`core_divergences`], the summary's
    /// `core_divergences` and the soak's `diff_kind 5` gate) and then **re-synced** from
    /// live through [`Self::seed_core`], never ignored, so the decision below runs on a
    /// persisted core that equals live. Before either happens, a test build records the
    /// persisted and live values exactly as read (`probe::lifecycle_entry`), so a test can
    /// prove the entry read the persisted fields before any later adapter write repaired
    /// them.
    ///
    /// Nothing is added under D beyond what [`Self::project_lifecycle_view`] did: the
    /// same two atomic loads and the same single view.
    pub(crate) fn lifecycle_view(app: &AppState, dictation: &mut DictationState) -> DictationCore {
        Self::persisted_view(
            app,
            dictation,
            #[cfg(test)]
            |persisted, live, _| super::probe::lifecycle_entry(persisted, live),
            #[cfg(test)]
            std::panic::Location::caller(),
        )
    }

    /// The one body of the cut-over views (see [`Self::lifecycle_view`]). `record` and
    /// `caller` are the family's probe and the entry's call site in a test build.
    fn persisted_view(
        app: &AppState,
        dictation: &mut DictationState,
        #[cfg(test)] record: fn(
            (
                murmur_dictation_core::status::Status,
                murmur_dictation_core::ids::RecordingId,
            ),
            (
                murmur_dictation_core::status::Status,
                murmur_dictation_core::ids::RecordingId,
            ),
            &'static std::panic::Location<'static>,
        ),
        #[cfg(test)] caller: &'static std::panic::Location<'static>,
    ) -> DictationCore {
        #[cfg(test)]
        super::probe::view();
        let live_status = core_status(dictation.status);
        let live_id = RecordingId(app.recording_id.load(Ordering::SeqCst));
        #[cfg(test)]
        record(
            (dictation.core.status(), dictation.core.current_id()),
            (live_status, live_id),
            caller,
        );
        if dictation.core.status() != live_status || dictation.core.current_id() != live_id {
            CORE_DIVERGENCES.fetch_add(1, Ordering::SeqCst);
            // Re-sync through the one seeder: no other method builds a core.
            Self::seed_core(app, dictation);
        }
        DictationCore::new(
            dictation.core,
            LiveObs {
                cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
                context_for: None,
            },
        )
    }

    /// The decision view of the stop family's entries after the cutover (C9.7 b, C8.6):
    /// sites 201, 202 (no context), 203, 204a, 204b and the legacy import's 301 (and its
    /// destination row 301.6), whose finish sections 204a and 204b also serve site 302 and
    /// all four `IdleGuard` origins. Only the stop family's own wrapper over
    /// [`Self::persisted_view`] (D-S2c-18): the same persisted M1/M2, the same M3 atomic, M4
    /// `None` for the same reason, the same C8.6 count-and-re-sync. A test build records
    /// its reads apart, with the caller (`probe::stop_entry`), so a stop test sees only stop
    /// entries and which one.
    #[cfg_attr(test, track_caller)]
    pub(crate) fn stop_view(app: &AppState, dictation: &mut DictationState) -> DictationCore {
        Self::persisted_view(
            app,
            dictation,
            #[cfg(test)]
            super::probe::stop_entry,
            #[cfg(test)]
            std::panic::Location::caller(),
        )
    }

    /// The decision view of the cancel family's one D entry after the cutover (C9.7 b,
    /// C8.6): site 205 (`decide_cancel_request`, R205.1-.5), decided under the D hold the
    /// section already takes with T held. Site 206 (the capture-cancel result, shared by the
    /// cancel command and the stop's R201.4) is D-less and keeps its lock-free reads. Only
    /// the cancel family's own wrapper over [`Self::persisted_view`] (D-S2c-18): the same
    /// persisted M1/M2, the same M3 atomic, M4 left `None` (no cancel decide reads it), the
    /// same C8.6 count-and-re-sync.
    /// A test build records its reads apart, with the caller (`probe::cancel_entry`), so a
    /// cancel test sees only cancel entries and which one.
    #[cfg_attr(test, track_caller)]
    pub(crate) fn cancel_view(app: &AppState, dictation: &mut DictationState) -> DictationCore {
        Self::persisted_view(
            app,
            dictation,
            #[cfg(test)]
            super::probe::cancel_entry,
            #[cfg(test)]
            std::panic::Location::caller(),
        )
    }

    /// The decision view of the start family's D entries after the cutover (C9.7 b, C8.6):
    /// site 101 (R101.3-.15, `decide_start_request`), 102 (the capture-failure section), 103.0
    /// (`decide_start_destination`), 103 (the context install) and 104 (the ready publish,
    /// R104.2-.7).
    /// Exactly [`Self::lifecycle_view`] for the start family: the same persisted M1/M2, the same
    /// M3 atomic, M4 left `None` (no start decide reads it; the context presence is a
    /// `NeedObservation`), the same C8.6 count-and-re-sync. A test build records its reads
    /// apart, with the caller (`probe::start_entry`), so a start test sees only start entries
    /// and which one.
    #[cfg_attr(test, track_caller)]
    pub(crate) fn start_view(app: &AppState, dictation: &mut DictationState) -> DictationCore {
        Self::persisted_view(
            app,
            dictation,
            #[cfg(test)]
            super::probe::start_entry,
            #[cfg(test)]
            std::panic::Location::caller(),
        )
    }

    /// The view of the start family's queries that hold only `&DictationState`: 401/404, the
    /// ticker registration (`register_partial_ticker_locked`). Its two callers are handed the
    /// shared reference: the ready entry's `SD` (run under the same D hold whose
    /// [`Self::start_view`] has just re-synced, so no difference can exist there) and the
    /// lifecycle bridge's detached `StartSideTasks` closure (404), which takes D itself with no
    /// entry before it. 403 (the delivery fence) and 402 hold a guard they may borrow mutably,
    /// so they take [`Self::start_view`] and re-sync like an entry.
    ///
    /// The persisted M1/M2 and the same M3 atomic, not `DictationCore::project` over the live
    /// fields. C8.6 is count, then re-sync, never ignore: a persisted/live difference is
    /// **counted** (`core_divergences`) and, because a shared reference cannot re-sync, the
    /// returned core carries the **live** M1/M2 (a core seeded from them, through the one
    /// seeder, on a clone of the state: this is the defect path, so its cost does not matter,
    /// and `PersistedCore::new` keeps its single caller). The query therefore answers as the
    /// code did before the cutover, and the stored core is left for the next entry to re-sync
    /// and count again: one divergence read by a query and then by the next entry is counted
    /// twice (once per read), by design.
    #[cfg_attr(test, track_caller)]
    pub(crate) fn start_query_view(app: &AppState, dictation: &DictationState) -> DictationCore {
        #[cfg(test)]
        super::probe::view();
        let live_status = core_status(dictation.status);
        let live_id = RecordingId(app.recording_id.load(Ordering::SeqCst));
        #[cfg(test)]
        super::probe::start_query(
            (dictation.core.status(), dictation.core.current_id()),
            (live_status, live_id),
            std::panic::Location::caller(),
        );
        let core =
            if dictation.core.status() != live_status || dictation.core.current_id() != live_id {
                CORE_DIVERGENCES.fetch_add(1, Ordering::SeqCst);
                let mut live = dictation.clone();
                Self::seed_core(app, &mut live);
                live.core
            } else {
                dictation.core
            };
        DictationCore::new(
            core,
            LiveObs {
                cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
                context_for: None,
            },
        )
    }

    /// The atomics a D-less lifecycle entry (212-215) reads, with no lock.
    pub(crate) fn lock_free_facts(app: &AppState) -> LockFreeFacts {
        #[cfg(test)]
        super::probe::view();
        LockFreeFacts {
            current_id: RecordingId(app.recording_id.load(Ordering::SeqCst)),
            cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
        }
    }

    /// Seed the persisted core from the live fields: how a test (or a re-sync)
    /// starts a core equal to live. Not an effect commit, and the only caller of
    /// `PersistedCore::new` outside the core crate.
    pub(crate) fn seed_core(app: &AppState, dictation: &mut DictationState) {
        dictation.core = PersistedCore::new(
            core_status(dictation.status),
            RecordingId(app.recording_id.load(Ordering::SeqCst)),
        );
    }
}
