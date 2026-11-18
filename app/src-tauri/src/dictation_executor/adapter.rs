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

/// The divergences counted so far.
pub(crate) fn core_divergences() -> u64 {
    CORE_DIVERGENCES.load(Ordering::SeqCst)
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

    /// `CX+`: `set_active_context`. Live only: M4 is never stored in the core.
    pub(crate) fn set_context(
        app: &AppState,
        id: u64,
        snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
    ) {
        crate::core_history::assert_not_held();
        app.set_active_context(id, snapshot);
    }

    /// `CX-`: `clear_active_context(id)`, which also clears `vad_stream` and
    /// `stream_reuse` for the id. Stale-safe: clears only if the slot holds `id`.
    /// Live only.
    pub(crate) fn clear_context(app: &AppState, id: u64) -> bool {
        crate::core_history::assert_not_held();
        app.clear_active_context(id)
    }

    /// `MC` under the shadow: [`FieldAdapter::cancel_through`] and the high-water mark
    /// before and after, both from the one `fetch_max` (appendix C8.2).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn cancel_through_witnessed(
        app: &AppState,
        id: u64,
    ) -> crate::dictation_shadow::types::MirrorWitness {
        use crate::dictation_shadow::types::{MirrorField, MirrorValue, MirrorWitness};
        crate::core_history::assert_not_held();
        let before = app.cancel_recording_observed(id);
        MirrorWitness {
            field: MirrorField::CancelledThrough,
            before: MirrorValue::Id(RecordingId(before)),
            after: MirrorValue::Id(RecordingId(before.max(id))),
        }
    }

    /// `CX+` under the shadow: [`FieldAdapter::set_context`] and the slot before and
    /// after, read inside the slot's own lock section.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn set_context_witnessed(
        app: &AppState,
        id: u64,
        snapshot: Arc<crate::dictation_context::DictationContextSnapshot>,
    ) -> crate::dictation_shadow::types::MirrorWitness {
        use crate::dictation_shadow::types::{MirrorField, MirrorValue, MirrorWitness};
        crate::core_history::assert_not_held();
        let before = app.set_active_context_observed(id, snapshot);
        MirrorWitness {
            field: MirrorField::ActiveContext,
            before: MirrorValue::Slot(before.map(RecordingId)),
            after: MirrorValue::Slot(Some(RecordingId(id))),
        }
    }

    /// `CX-` under the shadow: [`FieldAdapter::clear_context`] and the slot before and
    /// after, read inside the slot's own lock section (appendix C8.2 `pre_mirrors`).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn clear_context_witnessed(
        app: &AppState,
        id: u64,
    ) -> crate::dictation_shadow::types::MirrorWitness {
        use crate::dictation_shadow::types::{MirrorField, MirrorValue, MirrorWitness};
        crate::core_history::assert_not_held();
        // The same test hook as the ordinary path (a cancel's tail, where another
        // thread's input can land before the `TERM` claim).
        #[cfg(test)]
        crate::barrier_hook::at("adapter.clear_context");
        let (cleared, before) = app.clear_active_context_observed(id);
        MirrorWitness {
            field: MirrorField::ActiveContext,
            before: MirrorValue::Slot(before.map(RecordingId)),
            after: MirrorValue::Slot(if cleared {
                None
            } else {
                before.map(RecordingId)
            }),
        }
    }

    /// The M3/M4 observations of the live slots (C1.2): `cancelled_id` is one
    /// atomic load and the context id is one leaf-mutex read, the same reads
    /// `DictationCore::project` performs and a cut-over entry will perform.
    pub(crate) fn observe_live(app: &AppState) -> LiveObs {
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
    pub(crate) fn project_lifecycle_view(
        app: &AppState,
        dictation: &DictationState,
    ) -> DictationCore {
        DictationCore::project(
            core_status(dictation.status),
            RecordingId(app.recording_id.load(Ordering::SeqCst)),
            LiveObs {
                cancelled_through: RecordingId(app.cancelled_id.load(Ordering::SeqCst)),
                context_for: None,
            },
        )
    }

    /// The atomics a D-less lifecycle entry (212-215) reads, with no lock.
    pub(crate) fn lock_free_facts(app: &AppState) -> LockFreeFacts {
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
