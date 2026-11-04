//! Silent first-tap capture-helper preparation (#933).
//!
//! A first clean double-tap release may spawn, signature-verify, and hello an
//! idle capture helper so the confirmed start skips that work. A hello'd helper
//! blocks on its next control frame and touches no Core Audio: no device is
//! resolved or opened, no Start is written, and no audio exists before the
//! confirmed start adopts it. Abandoned preparations are torn down with
//! confirmed termination. Nothing here records key labels, device identifiers,
//! or user content.
use super::{
    capture_identity, hello, spawn_helper, terminate_helper_within, HelperPreparationOutcome,
};
use crate::managed_child::ManagedChild;
use crate::MutexExt;
use murmur_capture_helper_protocol::SessionNonce;
use std::io::BufReader;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// Covers the 400 ms second-tap window, the ≤200 ms second press, and the
/// command hop to the supervisor, without letting a stray tap pin a helper.
pub(crate) const PREPARED_HELPER_TTL: Duration = Duration::from_millis(1_500);
const TERMINATION_RETRY: Duration = Duration::from_millis(250);

/// The gesture, listener, and input topology a preparation belongs to. A
/// confirmed start adopts it only when all three are still current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrepareIdentity {
    pub token: u64,
    pub listener_generation: u64,
    pub topology_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvalidateReason {
    SequenceAbandoned,
    SecondTapExpired,
    Escape,
    ListenerChanged,
    Processing,
    Disabled,
    TopologyChanged,
    CaptureClaimed,
    Superseded,
    Expired,
}

impl InvalidateReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::SequenceAbandoned => "sequence_abandoned",
            Self::SecondTapExpired => "second_tap_expired",
            Self::Escape => "escape",
            Self::ListenerChanged => "listener_changed",
            Self::Processing => "processing",
            Self::Disabled => "disabled",
            Self::TopologyChanged => "topology_changed",
            Self::CaptureClaimed => "capture_claimed",
            Self::Superseded => "superseded",
            Self::Expired => "expired",
        }
    }
}

enum SlotState<H> {
    Empty,
    Preparing(PrepareIdentity),
    Ready {
        identity: PrepareIdentity,
        ready_at: Instant,
        helper: H,
    },
}

/// Ownership of at most one preparation. Every begin and invalidation bumps
/// the epoch, so a preparation finishing after it was superseded cannot
/// publish its helper and must tear it down.
pub(crate) struct Slot<H> {
    epoch: u64,
    state: SlotState<H>,
}

impl<H> Slot<H> {
    pub(crate) const fn new() -> Self {
        Self {
            epoch: 0,
            state: SlotState::Empty,
        }
    }

    /// Claim the slot for a new preparation; returns its epoch and any helper
    /// it displaced.
    pub(crate) fn begin(&mut self, identity: PrepareIdentity) -> (u64, Option<H>) {
        let displaced = self.clear();
        self.state = SlotState::Preparing(identity);
        (self.epoch, displaced)
    }

    /// Publish a finished helper, or hand it back when superseded.
    pub(crate) fn finish(&mut self, epoch: u64, helper: H, now: Instant) -> Result<(), H> {
        match self.state {
            SlotState::Preparing(identity) if self.epoch == epoch => {
                self.state = SlotState::Ready {
                    identity,
                    ready_at: now,
                    helper,
                };
                Ok(())
            }
            _ => Err(helper),
        }
    }

    /// Drop a preparation that failed before producing a helper.
    pub(crate) fn fail(&mut self, epoch: u64) {
        if self.epoch == epoch && matches!(self.state, SlotState::Preparing(_)) {
            self.clear();
        }
    }

    /// Whether anything was outstanding, and the ready helper to tear down.
    pub(crate) fn invalidate(&mut self) -> (bool, Option<H>) {
        let outstanding = !matches!(self.state, SlotState::Empty);
        (outstanding, self.clear())
    }

    /// Expire only the exact preparation that was published at `epoch`.
    pub(crate) fn expire(&mut self, epoch: u64) -> Option<H> {
        if self.epoch == epoch && matches!(self.state, SlotState::Ready { .. }) {
            self.clear()
        } else {
            None
        }
    }

    /// Every capture start drains the slot. Only a ready helper for the same
    /// identity within its lifetime is adopted; anything else is returned for
    /// teardown so it can never outlive a start.
    pub(crate) fn take(
        &mut self,
        identity: Option<PrepareIdentity>,
        now: Instant,
        ttl: Duration,
    ) -> (HelperPreparationOutcome, Option<H>, Option<H>) {
        let state = std::mem::replace(&mut self.state, SlotState::Empty);
        if !matches!(state, SlotState::Empty) {
            self.epoch = self.epoch.wrapping_add(1);
        }
        match state {
            SlotState::Empty => (HelperPreparationOutcome::NotPrepared, None, None),
            SlotState::Preparing(prepared) => (
                if Some(prepared) == identity {
                    HelperPreparationOutcome::StillPreparing
                } else {
                    HelperPreparationOutcome::IdentityMismatch
                },
                None,
                None,
            ),
            SlotState::Ready {
                identity: prepared,
                ready_at,
                helper,
            } => {
                if Some(prepared) != identity {
                    (
                        HelperPreparationOutcome::IdentityMismatch,
                        None,
                        Some(helper),
                    )
                } else if now.saturating_duration_since(ready_at) > ttl {
                    (HelperPreparationOutcome::Expired, None, Some(helper))
                } else {
                    (HelperPreparationOutcome::Adopted, Some(helper), None)
                }
            }
        }
    }

    fn clear(&mut self) -> Option<H> {
        let state = std::mem::replace(&mut self.state, SlotState::Empty);
        if matches!(state, SlotState::Empty) {
            return None;
        }
        self.epoch = self.epoch.wrapping_add(1);
        match state {
            SlotState::Ready { helper, .. } => Some(helper),
            _ => None,
        }
    }
}

/// A spawned, signature-verified, hello'd helper waiting for its command.
pub(crate) struct PreparedHelper {
    pub(super) child: ManagedChild,
    pub(super) input: std::process::ChildStdin,
    pub(super) output: BufReader<std::process::ChildStdout>,
    pub(super) capture_id: u64,
    pub(super) nonce: SessionNonce,
}

static SLOT: Mutex<Slot<PreparedHelper>> = Mutex::new(Slot::new());
static PREPARED: AtomicU64 = AtomicU64::new(0);
static ADOPTED: AtomicU64 = AtomicU64::new(0);
static EXPIRED: AtomicU64 = AtomicU64::new(0);
static ABANDONED: AtomicU64 = AtomicU64::new(0);

fn log_totals(event_code: &'static str, reason: &'static str) {
    tracing::info!(
        target: "audio",
        event_code,
        reason,
        prepared_total = PREPARED.load(Ordering::Relaxed),
        adopted_total = ADOPTED.load(Ordering::Relaxed),
        expired_total = EXPIRED.load(Ordering::Relaxed),
        abandoned_total = ABANDONED.load(Ordering::Relaxed),
        "content-free capture helper preparation outcome"
    );
}

/// Prepare an idle helper for a first clean tap. `admitted` runs after the
/// slot is claimed, so a start racing this request either drains the claim
/// or is observed by admission; no helper survives a start it did not serve.
pub(crate) fn request(identity: PrepareIdentity, admitted: impl FnOnce() -> bool + Send + 'static) {
    let (epoch, displaced) = SLOT.lock_or_recover().begin(identity);
    if let Some(helper) = displaced {
        discard(helper, InvalidateReason::Superseded);
    }
    let spawned = thread::Builder::new()
        .name("murmur-capture-prepare".to_string())
        .spawn(move || {
            if !admitted() {
                SLOT.lock_or_recover().fail(epoch);
                log_totals("audio.helper_prepare_skipped", "not_admitted");
                return;
            }
            let started = Instant::now();
            let (capture_id, nonce, nonce_hex) = capture_identity();
            let (child, mut input, output) = match spawn_helper(capture_id, &nonce_hex, None, None)
            {
                Ok(value) => value,
                Err(_) => {
                    SLOT.lock_or_recover().fail(epoch);
                    log_totals("audio.helper_prepare_failed", "spawn");
                    return;
                }
            };
            let output = match hello(&mut input, output, capture_id, nonce) {
                Ok(output) => output,
                Err(_) => {
                    SLOT.lock_or_recover().fail(epoch);
                    terminate(child, input, capture_id, nonce);
                    log_totals("audio.helper_prepare_failed", "hello");
                    return;
                }
            };
            let helper = PreparedHelper {
                child,
                input,
                output,
                capture_id,
                nonce,
            };
            let published = SLOT.lock_or_recover().finish(epoch, helper, Instant::now());
            match published {
                Ok(()) => {
                    PREPARED.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.helper_prepared",
                        capture_id,
                        prepare_ms = started.elapsed().as_millis() as u64,
                        "capture helper prepared without audio"
                    );
                    thread::sleep(PREPARED_HELPER_TTL);
                    let expired = SLOT.lock_or_recover().expire(epoch);
                    if let Some(helper) = expired {
                        EXPIRED.fetch_add(1, Ordering::Relaxed);
                        teardown(helper);
                        log_totals(
                            "audio.helper_prepare_expired",
                            InvalidateReason::Expired.as_str(),
                        );
                    }
                }
                Err(helper) => discard(helper, InvalidateReason::Superseded),
            }
        });
    if spawned.is_err() {
        SLOT.lock_or_recover().fail(epoch);
    }
}

/// Abandon any outstanding preparation. Cheap when nothing is prepared.
pub(crate) fn invalidate(reason: InvalidateReason) {
    let (outstanding, helper) = SLOT.lock_or_recover().invalidate();
    if let Some(helper) = helper {
        discard(helper, reason);
    } else if outstanding {
        log_totals("audio.helper_prepare_abandoned", reason.as_str());
    }
}

/// Drain the slot for a capture start. Only a dictation's first attempt for
/// the prepared gesture adopts the helper; every other start tears it down.
pub(crate) fn take_for_start(
    identity: Option<PrepareIdentity>,
) -> (HelperPreparationOutcome, Option<PreparedHelper>) {
    let (outcome, adopted, stale) =
        SLOT.lock_or_recover()
            .take(identity, Instant::now(), PREPARED_HELPER_TTL);
    if let Some(helper) = stale {
        if outcome == HelperPreparationOutcome::Expired {
            EXPIRED.fetch_add(1, Ordering::Relaxed);
        }
        discard(helper, InvalidateReason::Superseded);
    }
    if adopted.is_some() {
        ADOPTED.fetch_add(1, Ordering::Relaxed);
        log_totals("audio.helper_prepare_adopted", outcome.as_str());
    }
    (outcome, adopted)
}

fn discard(helper: PreparedHelper, reason: InvalidateReason) {
    ABANDONED.fetch_add(1, Ordering::Relaxed);
    teardown(helper);
    log_totals("audio.helper_prepare_abandoned", reason.as_str());
}

/// Confirmed termination off the caller's thread. Closing stdin before Start
/// makes the helper exit; it never opened audio, so an unconfirmed exit keeps
/// retrying ownership here rather than blocking a capture start.
fn teardown(helper: PreparedHelper) {
    let PreparedHelper {
        child,
        input,
        output,
        capture_id,
        nonce,
    } = helper;
    drop(output);
    let _ = thread::Builder::new()
        .name("murmur-capture-prepare-reap".to_string())
        .spawn(move || terminate(child, input, capture_id, nonce));
}

fn terminate(
    child: ManagedChild,
    input: std::process::ChildStdin,
    capture_id: u64,
    nonce: SessionNonce,
) {
    if let Err(mut child) =
        terminate_helper_within(child, Some(input), capture_id, nonce, None, Instant::now())
    {
        tracing::error!(
            target: "audio",
            event_code = "audio.helper_prepare_termination_unconfirmed",
            helper_pid = child.pid(),
            "prepared capture helper termination unconfirmed; retaining ownership"
        );
        while child
            .hard_kill_confirmed(Instant::now() + TERMINATION_RETRY)
            .is_none()
        {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(token: u64) -> PrepareIdentity {
        PrepareIdentity {
            token,
            listener_generation: 3,
            topology_generation: 5,
        }
    }

    const TTL: Duration = Duration::from_millis(1_500);

    #[test]
    fn matching_ready_helper_is_adopted_once() {
        let mut slot = Slot::new();
        let now = Instant::now();
        let (epoch, displaced) = slot.begin(identity(1));
        assert!(displaced.is_none());
        assert!(slot.finish(epoch, "helper", now).is_ok());
        assert_eq!(
            slot.take(Some(identity(1)), now, TTL),
            (HelperPreparationOutcome::Adopted, Some("helper"), None)
        );
        assert_eq!(
            slot.take(Some(identity(1)), now, TTL),
            (HelperPreparationOutcome::NotPrepared, None, None)
        );
    }

    #[test]
    fn mismatched_identity_is_torn_down_not_adopted() {
        for other in [
            identity(2),
            PrepareIdentity {
                listener_generation: 4,
                ..identity(1)
            },
            PrepareIdentity {
                topology_generation: 6,
                ..identity(1)
            },
        ] {
            let mut slot = Slot::new();
            let now = Instant::now();
            let (epoch, _) = slot.begin(identity(1));
            slot.finish(epoch, "helper", now).unwrap();
            assert_eq!(
                slot.take(Some(other), now, TTL),
                (
                    HelperPreparationOutcome::IdentityMismatch,
                    None,
                    Some("helper")
                )
            );
        }
    }

    #[test]
    fn toolbar_start_without_gesture_drains_preparation() {
        let mut slot = Slot::new();
        let now = Instant::now();
        let (epoch, _) = slot.begin(identity(1));
        slot.finish(epoch, "helper", now).unwrap();
        assert_eq!(
            slot.take(None, now, TTL),
            (
                HelperPreparationOutcome::IdentityMismatch,
                None,
                Some("helper")
            )
        );
    }

    #[test]
    fn expired_helper_is_not_adopted() {
        let mut slot = Slot::new();
        let ready = Instant::now();
        let (epoch, _) = slot.begin(identity(1));
        slot.finish(epoch, "helper", ready).unwrap();
        assert_eq!(
            slot.take(
                Some(identity(1)),
                ready + TTL + Duration::from_millis(1),
                TTL
            ),
            (HelperPreparationOutcome::Expired, None, Some("helper"))
        );
    }

    #[test]
    fn start_racing_preparation_supersedes_its_late_helper() {
        let mut slot = Slot::new();
        let (epoch, _) = slot.begin(identity(1));
        assert_eq!(
            slot.take(Some(identity(1)), Instant::now(), TTL),
            (HelperPreparationOutcome::StillPreparing, None, None)
        );
        assert_eq!(slot.finish(epoch, "late", Instant::now()), Err("late"));
    }

    #[test]
    fn invalidation_supersedes_in_flight_and_ready_preparations() {
        let mut slot = Slot::new();
        let (epoch, _) = slot.begin(identity(1));
        assert_eq!(slot.invalidate(), (true, None));
        assert_eq!(slot.finish(epoch, "late", Instant::now()), Err("late"));

        let (epoch, _) = slot.begin(identity(2));
        slot.finish(epoch, "ready", Instant::now()).unwrap();
        assert_eq!(slot.invalidate(), (true, Some("ready")));
        assert_eq!(slot.invalidate(), (false, None));
    }

    #[test]
    fn new_request_displaces_previous_helper() {
        let mut slot = Slot::new();
        let (epoch, _) = slot.begin(identity(1));
        slot.finish(epoch, "first", Instant::now()).unwrap();
        let (_, displaced) = slot.begin(identity(2));
        assert_eq!(displaced, Some("first"));
    }

    #[test]
    fn expiry_only_removes_its_own_publication() {
        let mut slot = Slot::new();
        let (first, _) = slot.begin(identity(1));
        slot.finish(first, "first", Instant::now()).unwrap();
        let (second, displaced) = slot.begin(identity(2));
        assert_eq!(displaced, Some("first"));
        slot.finish(second, "second", Instant::now()).unwrap();
        assert_eq!(slot.expire(first), None);
        assert_eq!(slot.expire(second), Some("second"));
    }

    #[test]
    fn failed_preparation_clears_only_its_claim() {
        let mut slot: Slot<&str> = Slot::new();
        let (first, _) = slot.begin(identity(1));
        let (second, _) = slot.begin(identity(2));
        slot.fail(first);
        assert_eq!(slot.finish(second, "second", Instant::now()), Ok(()));
    }
}
