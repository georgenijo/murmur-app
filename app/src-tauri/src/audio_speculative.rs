//! Fn/Globe key-down speculative capture (the only path that opens a
//! microphone before a dictation gesture is confirmed).
//!
//! The keyboard listener asks the audio supervisor to open the microphone the
//! instant Fn goes down, but only when Fn is the configured dictation trigger.
//! The capture has no UI, emits no levels, and is never transcribed, written
//! to disk, or logged beyond content-free event names and timings. It ends in
//! exactly one of two ways:
//!
//! - the confirmed dictation start for the same physical gesture adopts it, so
//!   audio captured since key-down becomes the start of that recording; or
//! - it is discarded: its retained samples are cleared and the exact capture
//!   worker is stopped and joined through the supervisor's existing confirmed
//!   teardown (cooperative stop, then process-group kill, ownership retained
//!   until exit is confirmed).
//!
//! Every other trigger key keeps the ordinary rule: no capture before a
//! confirmed start.
use crate::MutexExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hard backstop from key-down to discard. It covers the longest confirmable
/// gesture (a 200 ms first tap, the 400 ms second-tap window, and a 200 ms
/// second tap) plus the webview hop to `start_native_recording`, so a stray
/// press can never hold the microphone longer than this even if every
/// keyboard-side discard were missed.
pub(crate) const SPECULATIVE_CAPTURE_TTL: Duration = Duration::from_millis(1_500);

/// A confirmed gesture hands its capture to the start command through this
/// binding. Bindings are single-use and expire with the capture itself.
const BINDING_TTL: Duration = SPECULATIVE_CAPTURE_TTL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiscardReason {
    /// The physical sequence ended without a confirmed start (stray tap,
    /// Fn plus another key, held-too-long tap, expired second tap).
    NotConfirmed,
    Escape,
    ListenerChanged,
    Disabled,
    Processing,
    /// A confirmed start did not reach the audio supervisor (busy, disabled,
    /// or otherwise refused before capture ownership changed).
    StartRefused,
    /// A different capture or a non-matching start claimed the microphone.
    Superseded,
    /// The backstop elapsed before any start adopted the capture.
    Expired,
    TopologyChanged,
    /// Permission prompt, worker failure, or another capture fault.
    CaptureFailed,
}

impl DiscardReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotConfirmed => "not_confirmed",
            Self::Escape => "escape",
            Self::ListenerChanged => "listener_changed",
            Self::Disabled => "disabled",
            Self::Processing => "processing",
            Self::StartRefused => "start_refused",
            Self::Superseded => "superseded",
            Self::Expired => "expired",
            Self::TopologyChanged => "topology_changed",
            Self::CaptureFailed => "capture_failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Binding {
    gesture_token: u64,
    capture_id: u64,
    bound_at: Instant,
}

static BINDING: Mutex<Option<Binding>> = Mutex::new(None);

/// Record that the confirmed gesture `gesture_token` owns speculative capture
/// `capture_id`. A newer binding replaces (and discards) an unclaimed older one.
pub(crate) fn bind(gesture_token: u64, capture_id: u64) {
    let displaced = BINDING.lock_or_recover().replace(Binding {
        gesture_token,
        capture_id,
        bound_at: Instant::now(),
    });
    if let Some(old) = displaced.filter(|old| old.capture_id != capture_id) {
        super::discard_speculative_capture(old.capture_id, DiscardReason::Superseded);
    }
}

fn take_binding(gesture_token: Option<u64>, now: Instant) -> Option<u64> {
    let mut binding = BINDING.lock_or_recover();
    let current = (*binding)?;
    if now.saturating_duration_since(current.bound_at) >= BINDING_TTL {
        *binding = None;
        return None;
    }
    if Some(current.gesture_token) != gesture_token {
        return None;
    }
    *binding = None;
    Some(current.capture_id)
}

#[cfg(test)]
pub(crate) fn clear_binding() {
    *BINDING.lock_or_recover() = None;
}

/// The speculative capture a start request may adopt. Claimed at command
/// entry from the request's own gesture token; when the request does not
/// reach the supervisor, dropping the claim discards the capture at once
/// instead of waiting for the backstop.
#[must_use]
pub(crate) struct SpeculativeClaim {
    capture_id: Option<u64>,
}

impl SpeculativeClaim {
    /// Whether the claimed capture already has retained PCM, so the start can
    /// skip its visible connecting phase.
    pub(crate) fn is_flowing(&self) -> bool {
        self.capture_id
            .is_some_and(super::speculative_capture_is_flowing)
    }

    /// Hand the capture to the supervisor start request, which either adopts
    /// it or discards it itself.
    pub(crate) fn consume(mut self) -> Option<u64> {
        self.capture_id.take()
    }
}

impl Drop for SpeculativeClaim {
    fn drop(&mut self) {
        if let Some(capture_id) = self.capture_id.take() {
            super::discard_speculative_capture(capture_id, DiscardReason::StartRefused);
        }
    }
}

pub(crate) fn claim(gesture_token: Option<&str>) -> SpeculativeClaim {
    let token = gesture_token.and_then(|token| token.parse::<u64>().ok());
    SpeculativeClaim {
        capture_id: token.and_then(|token| take_binding(Some(token), Instant::now())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_is_single_use_and_requires_the_exact_gesture_token() {
        clear_binding();
        *BINDING.lock_or_recover() = Some(Binding {
            gesture_token: 41,
            capture_id: 9,
            bound_at: Instant::now(),
        });
        assert_eq!(take_binding(Some(40), Instant::now()), None);
        assert_eq!(take_binding(None, Instant::now()), None);
        assert_eq!(take_binding(Some(41), Instant::now()), Some(9));
        assert_eq!(take_binding(Some(41), Instant::now()), None);
    }

    #[test]
    fn expired_binding_is_never_adopted() {
        clear_binding();
        let bound_at = Instant::now();
        *BINDING.lock_or_recover() = Some(Binding {
            gesture_token: 7,
            capture_id: 3,
            bound_at,
        });
        assert_eq!(take_binding(Some(7), bound_at + BINDING_TTL), None);
        assert!(BINDING.lock_or_recover().is_none());
    }

    #[test]
    fn discard_reasons_are_stable_content_free_labels() {
        for (reason, label) in [
            (DiscardReason::NotConfirmed, "not_confirmed"),
            (DiscardReason::Escape, "escape"),
            (DiscardReason::ListenerChanged, "listener_changed"),
            (DiscardReason::Disabled, "disabled"),
            (DiscardReason::Processing, "processing"),
            (DiscardReason::StartRefused, "start_refused"),
            (DiscardReason::Superseded, "superseded"),
            (DiscardReason::Expired, "expired"),
            (DiscardReason::TopologyChanged, "topology_changed"),
            (DiscardReason::CaptureFailed, "capture_failed"),
        ] {
            assert_eq!(reason.as_str(), label);
        }
    }
}
