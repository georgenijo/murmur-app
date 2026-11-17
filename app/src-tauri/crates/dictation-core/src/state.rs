//! The core state vocabulary (appendix C1.2, C1.4, C2.3): what a decide
//! function sees.
//!
//! Only **M1 `status` and M2 `current_id`** are persisted ([`PersistedCore`]).
//! **M3 `cancelled_through` and M4 `context_for` are observations** of their
//! live slots (`AppState.cancelled_id`, `AppState.active_context`), filled by the
//! shell at every entry and never stored in the core ([`LiveObs`]). H1/H2 (the
//! operation history) are not core state either: they live in the shell's
//! `core_history` (C1.4.1).
//!
//! Decision views ([`DictationCore`]) are immutable: a decide function takes
//! `&DictationCore` and mutates nothing. The persisted fields commit **at their
//! effect** through [`PersistedCore::set_status`] / [`PersistedCore::alloc_id`]
//! (C1.4.2), never at decide time. Everything here is a pure value: the shell
//! reads the live fields and the atomics, this crate never does.

use crate::ids::RecordingId;
use crate::status::Status;

/// `DictationState.core`, inside D: **only** M1 and M2 (decision D5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PersistedCore {
    /// M1: `DictationState.status` (`state.rs:191`).
    pub status: Status,
    /// M2: `AppState.recording_id` (`state.rs:373`); `0` = none yet.
    pub current_id: RecordingId,
}

impl PersistedCore {
    /// Commit of the `ST` effect (C1.4.2): the same adapter call that writes the
    /// live `status`.
    pub fn set_status(&mut self, to: Status) {
        self.status = to;
    }

    /// Commit of the `ID+` effect: `fetch_add(1) + 1` (`state.rs:437-439`).
    /// Returns the new id so the executor can assert it equals the value the
    /// decision predicted (`current_id.next()`, C0.2).
    pub fn alloc_id(&mut self) -> RecordingId {
        self.current_id = self.current_id.next();
        self.current_id
    }
}

/// M3 and M4: immutable observations of the live slots, read by the shell at
/// section entry or at the read point today's code already uses (C1.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveObs {
    /// M3: `AppState.cancelled_id` (`is_cancelled(id)` = `cancelled_id >= id`,
    /// `state.rs:502-504`).
    pub cancelled_through: RecordingId,
    /// M4: the id of `AppState.active_context`, if a context is installed
    /// (`state.rs:405`). The snapshot payload stays in the shell.
    pub context_for: Option<RecordingId>,
}

/// The decision view every `decide_*` receives (C1.2): persisted M1/M2 (or the
/// live fields under `project()`) plus the M3/M4 observations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DictationCore {
    /// M1, persisted.
    pub status: Status,
    /// M2, persisted.
    pub current_id: RecordingId,
    /// M3, observation.
    pub cancelled_through: RecordingId,
    /// M4, observation.
    pub context_for: Option<RecordingId>,
}

/// The atomics a D-less entry reads today (C2.3 `LockFreeFacts`): M2 and M3
/// only. A `decide_*_nolock` entry reads no M1 or M4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockFreeFacts {
    /// `AppState.recording_id` (4897).
    pub current_id: RecordingId,
    /// `AppState.cancelled_id`.
    pub cancelled_through: RecordingId,
}

/// What `q_start_context_action` returns (`NativeStartContextAction`,
/// `recording.rs:2450-2455`; rows R103.1-.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartContextAction {
    /// Install the context and publish ready (R103.1).
    Activate,
    /// A lifecycle terminal landed first: flush (R103.2 / R103.3).
    FlushLifecycleTerminal {
        /// Install the context anyway (status `Recovering`, PD-8).
        install_context: bool,
    },
    /// The start was superseded (R103.4-.6).
    Superseded,
}

impl DictationCore {
    /// Assemble a view from the persisted core and the live observations.
    pub const fn new(persisted: PersistedCore, obs: LiveObs) -> Self {
        Self {
            status: persisted.status,
            current_id: persisted.current_id,
            cancelled_through: obs.cancelled_through,
            context_for: obs.context_for,
        }
    }

    /// `project()`: the same view built from the live M1/M2 under D (C1.4.2).
    /// Identical in shape to [`DictationCore::new`]; the *source* of
    /// `status`/`current_id` is what differs, and that is the shell's concern.
    pub const fn project(status: Status, current_id: RecordingId, obs: LiveObs) -> Self {
        Self::new(PersistedCore { status, current_id }, obs)
    }

    /// The persisted half (M1, M2).
    pub const fn persisted(&self) -> PersistedCore {
        PersistedCore {
            status: self.status,
            current_id: self.current_id,
        }
    }

    /// The facts a D-less entry may read (M2, M3).
    pub const fn lock_free_facts(&self) -> LockFreeFacts {
        LockFreeFacts {
            current_id: self.current_id,
            cancelled_through: self.cancelled_through,
        }
    }

    /// `is_cancelled(id)` = `cancelled_id >= id` (`state.rs:502-504`).
    pub fn is_cancelled(&self, id: RecordingId) -> bool {
        self.cancelled_through >= id
    }

    /// A pipeline is *draining* iff `id != current_id || id <= cancelled_through`
    /// (C3.0). Draining pipelines never block admission.
    pub fn is_draining(&self, id: RecordingId) -> bool {
        id != self.current_id || self.is_cancelled(id)
    }

    /// The id the next `ID+` allocates (C0.2): a pure function of the core.
    pub fn next_id(&self) -> RecordingId {
        self.current_id.next()
    }

    /// Query 401: `id > 0 && id == current_id && !is_cancelled(id)` while
    /// `Recording` (`recording.rs:106-111`).
    pub fn q_is_recording_current(&self, id: RecordingId) -> bool {
        !id.is_none()
            && id == self.current_id
            && self.status == Status::Recording
            && !self.is_cancelled(id)
    }

    /// Query 402: id current, not cancelled, status in {Starting, Recording,
    /// Recovering, Processing} (`recording_capture_is_current`,
    /// `recording.rs:1505-1519`; Idle is `false`).
    pub fn q_capture_current(&self, id: RecordingId) -> bool {
        id == self.current_id && !self.is_cancelled(id) && self.status != Status::Idle
    }

    /// Query 403: id current, not cancelled, status `Processing`
    /// (`delivery_event_recording_id`, `recording.rs:1492-1503`).
    pub fn q_owns_delivery(&self, id: RecordingId) -> bool {
        id == self.current_id && !self.is_cancelled(id) && self.status == Status::Processing
    }

    /// Query 406: `native_start_context_action(is_current, status)`
    /// (`recording.rs:2457-2476`, kept verbatim).
    pub fn q_start_context_action(&self, id: RecordingId) -> StartContextAction {
        if id != self.current_id {
            return StartContextAction::Superseded;
        }
        match self.status {
            Status::Starting => StartContextAction::Activate,
            Status::Recovering => StartContextAction::FlushLifecycleTerminal {
                install_context: true,
            },
            Status::Idle => StartContextAction::FlushLifecycleTerminal {
                install_context: false,
            },
            Status::Recording | Status::Processing => StartContextAction::Superseded,
        }
    }

    /// Query 407 (status part only): the idle unload may proceed only while
    /// `Idle` (`resource_monitor.rs:203-211`). The meeting-inference and
    /// benchmark parts stay in the shell.
    pub fn q_idle_unload_ok(&self) -> bool {
        self.status == Status::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(status: Status, current: u64, cancelled: u64) -> DictationCore {
        DictationCore::new(
            PersistedCore {
                status,
                current_id: RecordingId(current),
            },
            LiveObs {
                cancelled_through: RecordingId(cancelled),
                context_for: None,
            },
        )
    }

    #[test]
    fn only_status_and_current_id_are_persisted() {
        // The persisted struct has exactly two fields; M3/M4 live in `LiveObs`.
        let PersistedCore { status, current_id } = PersistedCore::default();
        assert_eq!((status, current_id), (Status::Idle, RecordingId::NONE));
        let LiveObs {
            cancelled_through,
            context_for,
        } = LiveObs::default();
        assert_eq!((cancelled_through, context_for), (RecordingId::NONE, None));
    }

    #[test]
    fn view_round_trips_through_its_parts() {
        let persisted = PersistedCore {
            status: Status::Recording,
            current_id: RecordingId(7),
        };
        let obs = LiveObs {
            cancelled_through: RecordingId(5),
            context_for: Some(RecordingId(7)),
        };
        let core = DictationCore::new(persisted, obs);
        assert_eq!(core.persisted(), persisted);
        assert_eq!(core.cancelled_through, obs.cancelled_through);
        assert_eq!(core.context_for, obs.context_for);
        assert_eq!(
            DictationCore::project(Status::Recording, RecordingId(7), obs),
            core
        );
        assert_eq!(
            core.lock_free_facts(),
            LockFreeFacts {
                current_id: RecordingId(7),
                cancelled_through: RecordingId(5)
            }
        );
    }

    #[test]
    fn persisted_fields_commit_only_through_their_effects() {
        let mut persisted = PersistedCore::default();
        let before = DictationCore::new(persisted, LiveObs::default());
        // A decision predicts the id; the commit allocates the same one.
        let predicted = before.next_id();
        assert_eq!(persisted.alloc_id(), predicted);
        persisted.set_status(Status::Starting);
        // The earlier view is a value: committing never changes it.
        assert_eq!(before.status, Status::Idle);
        assert_eq!(before.current_id, RecordingId::NONE);
        let after = DictationCore::new(persisted, LiveObs::default());
        assert_eq!(
            (after.status, after.current_id),
            (Status::Starting, predicted)
        );
    }

    #[test]
    fn cancellation_and_draining_follow_c3_0() {
        let core = view(Status::Processing, 5, 3);
        assert!(core.is_cancelled(RecordingId(3)));
        assert!(!core.is_cancelled(RecordingId(4)));
        assert!(core.is_draining(RecordingId(4)), "not current");
        assert!(!core.is_draining(RecordingId(5)));
        let cancelled_current = view(Status::Processing, 5, 5);
        assert!(cancelled_current.is_draining(RecordingId(5)));
    }

    #[test]
    fn q_is_recording_current_matches_c3_8() {
        for status in Status::ALL {
            let core = view(status, 2, 1);
            assert_eq!(
                core.q_is_recording_current(RecordingId(2)),
                status == Status::Recording,
                "{status:?}"
            );
        }
        let core = view(Status::Recording, 2, 2);
        assert!(!core.q_is_recording_current(RecordingId(2)), "cancelled");
        assert!(!core.q_is_recording_current(RecordingId(1)), "stale");
        let none = view(Status::Recording, 0, 0);
        assert!(!none.q_is_recording_current(RecordingId::NONE), "id 0");
    }

    #[test]
    fn q_capture_current_and_owns_delivery_match_c3_8() {
        for status in Status::ALL {
            let core = view(status, 2, 1);
            assert_eq!(
                core.q_capture_current(RecordingId(2)),
                status != Status::Idle,
                "{status:?}"
            );
            assert_eq!(
                core.q_owns_delivery(RecordingId(2)),
                status == Status::Processing,
                "{status:?}"
            );
        }
        let cancelled = view(Status::Processing, 2, 2);
        assert!(!cancelled.q_capture_current(RecordingId(2)));
        assert!(!cancelled.q_owns_delivery(RecordingId(2)));
        let stale = view(Status::Processing, 3, 0);
        assert!(!stale.q_capture_current(RecordingId(2)));
        assert!(!stale.q_owns_delivery(RecordingId(2)));
    }

    #[test]
    fn q_start_context_action_is_the_r103_table() {
        use StartContextAction::*;
        let cases = [
            (Status::Starting, Activate),
            (
                Status::Recovering,
                FlushLifecycleTerminal {
                    install_context: true,
                },
            ),
            (
                Status::Idle,
                FlushLifecycleTerminal {
                    install_context: false,
                },
            ),
            (Status::Recording, Superseded),
            (Status::Processing, Superseded),
        ];
        for (status, expected) in cases {
            assert_eq!(
                view(status, 4, 0).q_start_context_action(RecordingId(4)),
                expected,
                "{status:?}"
            );
            // A non-current generation is always superseded (R103.6).
            assert_eq!(
                view(status, 4, 0).q_start_context_action(RecordingId(3)),
                Superseded,
                "{status:?} stale"
            );
        }
    }

    #[test]
    fn idle_unload_reads_only_status() {
        for status in Status::ALL {
            assert_eq!(
                view(status, 1, 0).q_idle_unload_ok(),
                status == Status::Idle
            );
        }
    }
}
