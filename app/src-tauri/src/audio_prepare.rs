//! Silent first-tap capture-helper preparation (#933).
//!
//! A first clean double-tap release may spawn, signature-verify, and hello an
//! idle capture helper so the confirmed start skips that work. A separate
//! PrepareInput command performs bounded passive HAL UID/default lookup under
//! an existing TCC grant. It creates/initializes/starts no Audio Unit, writes no
//! Start, and receives no audio before the confirmed start adopts it. Abandoned
//! preparations are torn down with
//! confirmed termination. Nothing here records key labels, device identifiers,
//! or user content.
use super::{
    capture_identity, spawn_helper_cancellable, terminate_helper_within, HelperPreparationOutcome,
};
use crate::managed_child::ManagedChild;
use crate::MutexExt;
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, ProductionFrame, ProductionHelperMessage,
    ProductionHostMessage, SessionNonce,
};
use std::io::{BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Covers the 400 ms second-tap window, the ≤200 ms second press, and the
/// command hop to the supervisor, without letting a stray tap pin a helper.
pub(crate) const PREPARED_HELPER_TTL: Duration = Duration::from_millis(1_500);
const TERMINATION_RETRY: Duration = Duration::from_millis(250);

/// The gesture, listener, and input topology a preparation belongs to. A
/// confirmed start adopts it only when these and the settings generation match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrepareIdentity {
    pub token: u64,
    pub listener_generation: u64,
    pub topology_generation: u64,
    pub configuration_generation: u64,
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
    Preparing {
        identity: PrepareIdentity,
        requested_at: Instant,
    },
    Ready {
        identity: PrepareIdentity,
        requested_at: Instant,
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
        self.state = SlotState::Preparing {
            identity,
            requested_at: Instant::now(),
        };
        (self.epoch, displaced)
    }

    /// Publish a finished helper, or hand it back when superseded.
    pub(crate) fn finish(&mut self, epoch: u64, helper: H, now: Instant) -> Result<(), H> {
        match self.state {
            SlotState::Preparing {
                identity,
                requested_at,
            } if self.epoch == epoch
                && now.saturating_duration_since(requested_at) < PREPARED_HELPER_TTL =>
            {
                self.state = SlotState::Ready {
                    identity,
                    requested_at,
                    helper,
                };
                Ok(())
            }
            _ => Err(helper),
        }
    }

    /// Drop a preparation that failed before producing a helper.
    pub(crate) fn fail(&mut self, epoch: u64) {
        if self.epoch == epoch && matches!(self.state, SlotState::Preparing { .. }) {
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
            SlotState::Preparing {
                identity: prepared, ..
            } => (
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
                requested_at,
                helper,
            } => {
                if Some(prepared) != identity {
                    (
                        HelperPreparationOutcome::IdentityMismatch,
                        None,
                        Some(helper),
                    )
                } else if now.saturating_duration_since(requested_at) >= ttl {
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

struct Preparation {
    slot: Slot<PreparedHelper>,
    revocation_generation: u64,
    // Remains occupied through confirmed cleanup. Repeated taps cannot create
    // another worker, reader, or child while the previous owner is retiring.
    active: Option<Arc<Cancellation>>,
    retiring: Option<PreparedHelper>,
}
impl Preparation {
    const fn new() -> Self {
        Self {
            slot: Slot::new(),
            revocation_generation: 0,
            active: None,
            retiring: None,
        }
    }
    fn invalidate(&mut self) -> bool {
        self.revocation_generation = self.revocation_generation.wrapping_add(1);
        let (outstanding, helper) = self.slot.invalidate();
        if let Some(cancel) = &self.active {
            cancel.cancelled.store(true, Ordering::Release);
        }
        if helper.is_some() {
            assert!(self.retiring.is_none());
            self.retiring = helper;
        }
        outstanding
    }
}
struct Cancellation {
    cancelled: AtomicBool,
    deadline: Instant,
}
impl Cancellation {
    fn valid(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.deadline
    }
}
static PREPARATION: Mutex<Preparation> = Mutex::new(Preparation::new());
static CHANGED: Condvar = Condvar::new();
pub(crate) fn revocation_generation() -> u64 {
    PREPARATION.lock_or_recover().revocation_generation
}
static CONFIGURATION_GENERATION: AtomicU64 = AtomicU64::new(0);
pub(crate) fn configuration_generation() -> u64 {
    CONFIGURATION_GENERATION.load(Ordering::Acquire)
}
pub(crate) fn configuration_changed() {
    CONFIGURATION_GENERATION.fetch_add(1, Ordering::AcqRel);
    invalidate(InvalidateReason::Superseded);
}
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

/// Caller holds recording_transition through admission and this claim. The
/// worker is single-flight, including retiring children. No queue of stale taps.
pub(crate) fn request(identity: PrepareIdentity, revocation_generation: u64) {
    request_with(identity, revocation_generation, |epoch, cancel| {
        thread::Builder::new()
            .name("murmur-capture-prepare".into())
            .spawn(move || {
                prepare(epoch, &cancel);
                let mut state = PREPARATION.lock_or_recover();
                state.slot.fail(epoch);
                state.active = None;
                CHANGED.notify_all();
            })
            .map(|_| ())
    });
}

fn request_with(
    identity: PrepareIdentity,
    revocation_generation: u64,
    spawn: impl FnOnce(u64, Arc<Cancellation>) -> std::io::Result<()>,
) -> bool {
    let mut state = PREPARATION.lock_or_recover();
    if state.revocation_generation != revocation_generation
        || state.active.is_some()
        || identity.configuration_generation != configuration_generation()
        || identity.topology_generation != crate::audio_lifecycle::input_topology_generation()
    {
        return false;
    }
    let cancel = Arc::new(Cancellation {
        cancelled: AtomicBool::new(false),
        deadline: Instant::now() + PREPARED_HELPER_TTL,
    });
    let (epoch, displaced) = state.slot.begin(identity);
    assert!(displaced.is_none());
    state.active = Some(Arc::clone(&cancel));
    // No child exists at the only fallible ownership handoff. All later cleanup
    // is synchronous on this same worker, even if OS termination is delayed.
    if spawn(epoch, cancel).is_err() {
        state.slot.fail(epoch);
        state.active = None;
        return false;
    }
    true
}

/// Poll before *every* pipe read (including partial frames), without creating
/// a detached reader. Cancellation and the request deadline bound hello and
/// passive resolution alike; neither path can write Start.
struct CancellableOutput<'a> {
    output: std::process::ChildStdout,
    cancel: &'a Cancellation,
    #[cfg(test)]
    before_poll: Option<Box<dyn FnOnce() + Send>>,
}
impl Read for CancellableOutput<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        use std::os::fd::AsRawFd;
        loop {
            if !self.cancel.valid() {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            let mut fd = libc::pollfd {
                fd: self.output.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            #[cfg(test)]
            if let Some(gate) = self.before_poll.take() {
                gate();
            }
            // SAFETY: fd is an owned pipe and poll only borrows this stack value.
            let result = unsafe { libc::poll(&mut fd, 1, 5) };
            if result > 0 {
                return self.output.read(bytes);
            }
            if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
}

fn prepare(epoch: u64, cancel: &Cancellation) {
    prepare_with_spawn(epoch, cancel, |capture_id, nonce_hex| {
        spawn_helper_cancellable(capture_id, nonce_hex, || cancel.valid())
    });
}

fn prepare_with_spawn(
    epoch: u64,
    cancel: &Cancellation,
    spawn: impl FnOnce(
        u64,
        &str,
    ) -> Result<
        (
            ManagedChild,
            std::process::ChildStdin,
            std::process::ChildStdout,
        ),
        super::AudioFailure,
    >,
) {
    if !cancel.valid() {
        return;
    }
    let started = Instant::now();
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let Ok((child, mut input, output)) = spawn(capture_id, &nonce_hex) else {
        return;
    };
    let mut output = CancellableOutput {
        output,
        cancel,
        #[cfg(test)]
        before_poll: None,
    };
    let exchange = |input: &mut std::process::ChildStdin,
                    output: &mut CancellableOutput<'_>,
                    command,
                    expected|
     -> Result<(), ()> {
        if !cancel.valid() {
            return Err(());
        }
        write_production_control(input, capture_id, nonce, &command).map_err(|_| ())?;
        match read_production_frame::<ProductionHelperMessage>(output, capture_id, nonce)
            .map_err(|_| ())?
        {
            ProductionFrame::Control(message) if message == expected => Ok(()),
            _ => Err(()),
        }
    };
    let ready = exchange(
        &mut input,
        &mut output,
        ProductionHostMessage::Hello,
        ProductionHelperMessage::HelloAck,
    )
    .and_then(|()| {
        exchange(
            &mut input,
            &mut output,
            ProductionHostMessage::PrepareInput,
            ProductionHelperMessage::InputPrepared,
        )
    });
    if ready.is_err() || !cancel.valid() {
        drop(output);
        terminate(child, input, capture_id, nonce);
        return;
    }
    let helper = PreparedHelper {
        child,
        input,
        output: BufReader::new(output.output),
        capture_id,
        nonce,
    };
    let mut state = PREPARATION.lock_or_recover();
    if let Err(helper) = state.slot.finish(epoch, helper, Instant::now()) {
        drop(state);
        teardown(helper);
        return;
    }
    PREPARED.fetch_add(1, Ordering::Relaxed);
    tracing::info!(target: "audio", event_code = "audio.helper_prepared", capture_id, prepare_ms = started.elapsed().as_millis() as u64, "capture helper prepared without audio");
    while cancel.valid() && state.slot.epoch == epoch {
        let remaining = cancel.deadline.saturating_duration_since(Instant::now());
        state = CHANGED
            .wait_timeout(state, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    let expired = state.slot.expire(epoch);
    let helper = expired.or_else(|| state.retiring.take());
    drop(state);
    if let Some(helper) = helper {
        if !cancel.valid() && Instant::now() >= cancel.deadline {
            EXPIRED.fetch_add(1, Ordering::Relaxed);
            log_totals(
                "audio.helper_prepare_expired",
                InvalidateReason::Expired.as_str(),
            );
        }
        teardown(helper);
    }
}

pub(crate) fn invalidate(reason: InvalidateReason) {
    let outstanding = PREPARATION.lock_or_recover().invalidate();
    CHANGED.notify_all();
    if outstanding {
        ABANDONED.fetch_add(1, Ordering::Relaxed);
        log_totals("audio.helper_prepare_abandoned", reason.as_str());
    }
}

/// Delayed expiry must never invalidate a different, newer gesture.
pub(crate) fn invalidate_token(token: u64, listener_generation: u64, reason: InvalidateReason) {
    let mut state = PREPARATION.lock_or_recover();
    let identity = match state.slot.state {
        SlotState::Preparing { identity, .. } | SlotState::Ready { identity, .. } => Some(identity),
        SlotState::Empty => None,
    };
    if identity.is_some_and(|i| i.token == token && i.listener_generation == listener_generation) {
        state.invalidate();
        CHANGED.notify_all();
        ABANDONED.fetch_add(1, Ordering::Relaxed);
        log_totals("audio.helper_prepare_abandoned", reason.as_str());
    }
}

/// Under the shared transition, a confirmed keyboard claim preserves only its
/// exact gesture; toolbar and other capture claims invalidate immediately.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CaptureClaim {
    Dictation(Option<PrepareIdentity>),
    Query,
    Meeting,
    Preview,
    Standby,
}

pub(crate) async fn admission_transition<'a>(
    transition: &'a tokio::sync::Mutex<()>,
    admitted: impl FnOnce() -> bool,
) -> Option<tokio::sync::MutexGuard<'a, ()>> {
    let guard = transition.lock().await;
    admitted().then_some(guard)
}

pub(crate) fn capture_claimed(claim: CaptureClaim) {
    let identity = match claim {
        CaptureClaim::Dictation(identity) => identity,
        _ => None,
    };
    let mut state = PREPARATION.lock_or_recover();
    let matches = match state.slot.state {
        SlotState::Preparing {
            identity: prepared, ..
        }
        | SlotState::Ready {
            identity: prepared, ..
        } => Some(prepared) == identity,
        SlotState::Empty => false,
    };
    if !matches && state.invalidate() {
        CHANGED.notify_all();
        ABANDONED.fetch_add(1, Ordering::Relaxed);
        log_totals(
            "audio.helper_prepare_abandoned",
            InvalidateReason::CaptureClaimed.as_str(),
        );
    }
}

pub(crate) fn take_for_start(
    identity: Option<PrepareIdentity>,
) -> (HelperPreparationOutcome, Option<PreparedHelper>) {
    let mut state = PREPARATION.lock_or_recover();
    let (outcome, adopted, stale) = state
        .slot
        .take(identity, Instant::now(), PREPARED_HELPER_TTL);
    if let Some(cancel) = &state.active {
        cancel.cancelled.store(true, Ordering::Release);
    }
    if let Some(stale) = stale {
        assert!(state.retiring.is_none());
        state.retiring = Some(stale);
    }
    CHANGED.notify_all();
    if adopted.is_some() {
        ADOPTED.fetch_add(1, Ordering::Relaxed);
        log_totals("audio.helper_prepare_adopted", outcome.as_str());
    }
    (outcome, adopted)
}

fn teardown(helper: PreparedHelper) {
    let PreparedHelper {
        child,
        input,
        output,
        capture_id,
        nonce,
    } = helper;
    drop(output);
    terminate(child, input, capture_id, nonce);
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
            configuration_generation: 7,
        }
    }

    const TTL: Duration = Duration::from_millis(1_500);

    fn current_identity(token: u64) -> PrepareIdentity {
        PrepareIdentity {
            token,
            listener_generation: 3,
            topology_generation: crate::audio_lifecycle::input_topology_generation(),
            configuration_generation: configuration_generation(),
        }
    }
    fn reset_preparation() {
        let mut state = PREPARATION.lock_or_recover();
        assert!(state.retiring.is_none());
        let (_, helper) = state.slot.invalidate();
        assert!(helper.is_none());
        state.active = None;
    }
    fn claim_test_request(identity: PrepareIdentity) -> Arc<Cancellation> {
        let mut ticket = None;
        assert!(request_with(
            identity,
            revocation_generation(),
            |_, cancel| {
                ticket = Some(cancel);
                Ok(())
            }
        ));
        ticket.unwrap()
    }

    #[tokio::test]
    async fn shared_transition_serializes_keyboard_admission_and_all_capture_claims() {
        use crate::state::{AppState, DictationStatus};
        for claim in [
            CaptureClaim::Dictation(None),
            CaptureClaim::Query,
            CaptureClaim::Meeting,
            CaptureClaim::Preview,
            CaptureClaim::Standby,
        ] {
            reset_preparation();
            let app = Arc::new(AppState::default());
            let owner = Arc::new(AtomicBool::new(false));
            // First race direction: admission owns the actual production
            // transition through request; the next claimant invalidates it.
            let guard =
                admission_transition(&app.recording_transition, || !owner.load(Ordering::SeqCst))
                    .await
                    .unwrap();
            let id = current_identity(100);
            let cancel = claim_test_request(id);
            let capture_app = Arc::clone(&app);
            let capture_owner = Arc::clone(&owner);
            let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
            let capture = tokio::spawn(async move {
                queued_tx.send(()).unwrap();
                let _transition = capture_app.recording_transition.lock().await;
                capture_claimed(claim);
                capture_owner.store(true, Ordering::SeqCst);
                capture_app.dictation.lock_or_recover().status = DictationStatus::Starting;
            });
            queued_rx.await.unwrap();
            assert!(cancel.valid());
            drop(guard);
            capture.await.unwrap();
            assert!(
                !cancel.valid(),
                "{claim:?} must revoke before async context/setup"
            );
            assert!(matches!(
                PREPARATION.lock_or_recover().slot.state,
                SlotState::Empty
            ));
            // Reverse direction: a capture that claimed first blocks admission,
            // even when keyboard preparation was queued before lock release.
            let guard = app.recording_transition.lock().await;
            let prepare_app = Arc::clone(&app);
            let prepare_owner = Arc::clone(&owner);
            let queued = tokio::spawn(async move {
                admission_transition(&prepare_app.recording_transition, || {
                    !prepare_owner.load(Ordering::SeqCst)
                })
                .await
                .is_some()
            });
            drop(guard);
            assert!(!queued.await.unwrap());
            reset_preparation();
        }
    }

    #[test]
    fn exact_keyboard_claim_survives_but_toolbar_and_wrong_gestures_revoke() {
        reset_preparation();
        let id = current_identity(101);
        let cancel = claim_test_request(id);
        capture_claimed(CaptureClaim::Dictation(Some(id)));
        assert!(cancel.valid());
        capture_claimed(CaptureClaim::Dictation(Some(PrepareIdentity {
            token: 102,
            ..id
        })));
        assert!(!cancel.valid());
        // Cleanup ownership stays single-flight after invalidation, until its
        // existing worker (not an extra reaper thread) confirms exit.
        assert!(!request_with(
            current_identity(103),
            revocation_generation(),
            |_, _| panic!("must not spawn during cleanup")
        ));
        reset_preparation();
    }

    #[test]
    fn failed_worker_handoff_occurs_before_child_creation_and_releases_claim() {
        reset_preparation();
        assert!(!request_with(
            current_identity(104),
            revocation_generation(),
            |_, _| Err(std::io::ErrorKind::Other.into())
        ));
        let state = PREPARATION.lock_or_recover();
        assert!(state.active.is_none());
        assert!(state.retiring.is_none());
        assert!(matches!(state.slot.state, SlotState::Empty));
    }

    #[test]
    fn delayed_expiry_cannot_cancel_newer_preparation() {
        reset_preparation();
        let old = current_identity(105);
        let old_cancel = claim_test_request(old);
        let (gate_tx, gate_rx) = std::sync::mpsc::channel();
        let expiry = thread::spawn(move || {
            gate_rx.recv().unwrap();
            invalidate_token(
                old.token,
                old.listener_generation,
                InvalidateReason::SecondTapExpired,
            );
        });
        // The old detector has expired and cleanup completed while its timer
        // is descheduled before the identity-scoped invalidation call.
        reset_preparation();
        old_cancel.cancelled.store(true, Ordering::Release);
        let newer = current_identity(106);
        let cancel = claim_test_request(newer);
        gate_tx.send(()).unwrap();
        expiry.join().unwrap();
        assert!(cancel.valid());
        invalidate_token(
            newer.token,
            newer.listener_generation,
            InvalidateReason::SecondTapExpired,
        );
        assert!(!cancel.valid());
        reset_preparation();
    }

    #[test]
    fn settings_generation_prevents_late_request_resurrection_and_adoption() {
        reset_preparation();
        let id = current_identity(107);
        let cancel = claim_test_request(id);
        configuration_changed();
        assert!(!cancel.valid());
        reset_preparation();
        assert!(!request_with(id, revocation_generation(), |_, _| panic!(
            "old settings must not spawn"
        )));
    }

    #[test]
    fn keyboard_revocation_rejects_a_continuation_that_already_passed_admission() {
        for revoke in [
            (|| crate::keyboard::set_app_disabled(true)) as fn(),
            || crate::keyboard::set_processing(true),
            crate::keyboard::stop_listener,
        ] {
            reset_preparation();
            let identity = current_identity(108);
            let generation = revocation_generation();
            let (resume, paused) = std::sync::mpsc::channel();
            // Pause exactly between earlier admission and the final slot claim.
            let continuation = thread::spawn(move || {
                paused.recv().unwrap();
                request_with(identity, generation, |_, _| {
                    panic!("revoked continuation must not start a worker")
                })
            });
            revoke();
            resume.send(()).unwrap();
            assert!(!continuation.join().unwrap());
            crate::keyboard::set_app_disabled(false);
            crate::keyboard::set_processing(false);
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_during_spawn_retains_and_reaps_the_returned_child() {
        let cancel = Arc::new(Cancellation {
            cancelled: AtomicBool::new(false),
            deadline: Instant::now() + TTL,
        });
        let worker_cancel = Arc::clone(&cancel);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            prepare_with_spawn(0, &worker_cancel, |_, _| {
                let owned = ManagedChild::spawn_with_arguments(
                    std::path::Path::new("/bin/sh"),
                    &["-c", "exec /bin/sleep 60"],
                    &[],
                )
                .unwrap();
                entered_tx.send(owned.0.pid()).unwrap();
                release_rx.recv().unwrap();
                Ok(owned)
            });
        });
        let pid = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        cancel.cancelled.store(true, Ordering::Release);
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_during_hello_or_passive_resolution_reaps_the_exact_child() {
        for command in [
            ProductionHostMessage::Hello,
            ProductionHostMessage::PrepareInput,
        ] {
            let cancel = Arc::new(Cancellation {
                cancelled: AtomicBool::new(false),
                deadline: Instant::now() + TTL,
            });
            let (child, mut input, output) = ManagedChild::spawn_with_arguments(
                std::path::Path::new("/bin/sh"),
                &["-c", "exec /bin/sleep 60"],
                &[],
            )
            .unwrap();
            let pid = child.pid();
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let worker_cancel = Arc::clone(&cancel);
            let worker = thread::spawn(move || {
                let nonce = [7; 16];
                write_production_control(&mut input, 77, nonce, &command).unwrap();
                let mut output = CancellableOutput {
                    output,
                    cancel: &worker_cancel,
                    before_poll: Some(Box::new(move || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    })),
                };
                assert!(
                    read_production_frame::<ProductionHelperMessage>(&mut output, 77, nonce)
                        .is_err()
                );
                drop(output);
                terminate(child, input, 77, nonce);
            });
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            cancel.cancelled.store(true, Ordering::Release);
            release_tx.send(()).unwrap();
            worker.join().unwrap();
            assert_eq!(
                unsafe { libc::kill(pid as i32, 0) },
                -1,
                "child must have confirmed exit, not a dropped handoff"
            );
        }
    }

    #[test]
    fn request_deadline_includes_spawn_and_hello_time() {
        let mut slot = Slot::new();
        let (epoch, _) = slot.begin(identity(1));
        let requested = match slot.state {
            SlotState::Preparing { requested_at, .. } => requested_at,
            _ => unreachable!(),
        };
        assert!(slot
            .finish(epoch, "helper", requested + TTL - Duration::from_millis(1))
            .is_ok());
        assert_eq!(
            slot.take(Some(identity(1)), requested + TTL, TTL),
            (HelperPreparationOutcome::Expired, None, Some("helper"))
        );
        let (epoch, _) = slot.begin(identity(2));
        assert_eq!(
            slot.finish(epoch, "late", Instant::now() + TTL),
            Err("late")
        );
    }

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
