//! Native hotkey-release stamp for the release-to-paste metric (#826).
//!
//! `totalProcessing` starts inside `stop_native_recording_for`, after the stop
//! already travelled rdev → `hold-down-stop` → webview → IPC. The rdev
//! listener stamps a monotonic `Instant` when it publishes a hold-down stop,
//! keyed by the recording generation live at that moment. The stop command
//! consumes the stamp for its own generation, so a release that never reached
//! a stop, or that belonged to an older recording, is never attributed to a
//! later run. A monotonic instant cannot cross the webview, so the stamp stays
//! in Rust. It holds no content: one instant and one generation ID.

use crate::performance_metrics::{PerformanceStageV1, StageTimingV1};
use crate::MutexExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A release older than this did not drive the stop it would be paired with.
const MAX_RELEASE_TO_STOP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReleaseStamp {
    recording_id: u64,
    released_at: Instant,
}

static LAST_RELEASE: Mutex<Option<ReleaseStamp>> = Mutex::new(None);

/// Atomic toggle-stop preview gate: keeps the in-progress key press and the
/// released tap under one lock so key-up transitions atomically without a gap
/// where a preview worker could sneak in, and a cancelled second press can
/// never clear an already released stop. Separate from `LAST_RELEASE` so toggle
/// stops never produce a hold-release tail anchor or `release_to_stop` metric.
#[derive(Debug, Default)]
struct ToggleStopGate {
    press: Option<ReleaseStamp>,
    released: Option<ReleaseStamp>,
}

static TOGGLE_STOP_GATE: Mutex<ToggleStopGate> = Mutex::new(ToggleStopGate {
    press: None,
    released: None,
});

/// The native release paired with the stop that accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyRelease {
    released_at: Instant,
    stop_entered_at: Instant,
}

impl HotkeyRelease {
    #[cfg(test)]
    pub fn for_test(released_at: Instant, stop_entered_at: Instant) -> Self {
        Self {
            released_at,
            stop_entered_at,
        }
    }

    /// The native key-release instant, for anchoring the release tail.
    pub fn released_at(&self) -> Instant {
        self.released_at
    }

    /// Release → `stop_native_recording_for` entry.
    pub fn release_to_stop(&self) -> StageTimingV1 {
        StageTimingV1::measured(
            PerformanceStageV1::ReleaseToStop,
            millis(
                self.stop_entered_at
                    .saturating_duration_since(self.released_at),
            ),
        )
    }

    /// Release → the end of delivery, measured where `totalProcessing` ends.
    pub fn release_to_paste(&self, delivered_at: Instant) -> StageTimingV1 {
        StageTimingV1::measured(
            PerformanceStageV1::ReleaseToPaste,
            millis(delivered_at.saturating_duration_since(self.released_at)),
        )
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Stamp the native release that publishes `hold-down-stop`.
pub fn record(recording_id: u64, released_at: Instant) {
    record_into(
        &mut LAST_RELEASE.lock_or_recover(),
        ReleaseStamp {
            recording_id,
            released_at,
        },
    );
}

/// Mark an in-progress toggle-stop key press for `recording_id`, so live
/// preview and stream-reuse commits stop scheduling new non-cancellable Core ML
/// decodes while the tap is underway.
pub fn record_toggle_stop_press(recording_id: u64, at: Instant) {
    if recording_id == 0 {
        return;
    }
    TOGGLE_STOP_GATE.lock_or_recover().press = Some(ReleaseStamp {
        recording_id,
        released_at: at,
    });
}

/// Clear an in-progress toggle-stop key press when it is cancelled (e.g. turned
/// into a combo or held too long). Never clears an already released toggle stop.
pub fn clear_toggle_stop_press(recording_id: u64) {
    let mut gate = TOGGLE_STOP_GATE.lock_or_recover();
    if gate
        .press
        .is_some_and(|stamp| stamp.recording_id == recording_id)
    {
        gate.press = None;
    }
}

/// Atomically transition a toggle-stop press into a released stop gate for
/// `recording_id` while the stop traverses the webview/IPC hop. Refreshes the
/// preview gate timestamp on each clean release so a retry tap near the end of
/// an earlier unclaimed stop's window gets a full [`STOP_GESTURE_WINDOW`].
pub fn record_toggle_stop_released(recording_id: u64, at: Instant) {
    if recording_id == 0 {
        return;
    }
    let mut gate = TOGGLE_STOP_GATE.lock_or_recover();
    gate.released = Some(ReleaseStamp {
        recording_id,
        released_at: at,
    });
    if gate
        .press
        .is_some_and(|stamp| stamp.recording_id == recording_id)
    {
        gate.press = None;
    }
}

fn clear_toggle_stop_gate(recording_id: u64) {
    let mut gate = TOGGLE_STOP_GATE.lock_or_recover();
    if gate
        .press
        .is_some_and(|stamp| stamp.recording_id == recording_id)
    {
        gate.press = None;
    }
    if gate
        .released
        .is_some_and(|stamp| stamp.recording_id == recording_id)
    {
        gate.released = None;
    }
}

/// How long a native release counts as "a stop is on its way". A release that
/// the stop never claimed (a rejected stop) must not silence the live preview
/// for the rest of the recording. The check only reads the stamp, so it never
/// takes it from the stop. Much longer than the tail-anchor freshness window
/// the stop path applies (`audio_lifecycle::release_anchor`, at most one tail
/// after the release): the preview must stop earlier than the stop is accepted.
const STOP_GESTURE_WINDOW: Duration = Duration::from_millis(500);

/// Whether a hold release or toggle stop press/release for `recording_id` has
/// been seen and its stop has not been accepted yet. The gesture reaches the
/// stop command through the webview, so this is the earliest point at which the
/// stop is known. The live preview schedules no new decode from here on (a
/// running Core ML call cannot be cancelled, so the final decode must not queue
/// behind a later one).
pub fn stop_gesture_pending(recording_id: u64) -> bool {
    let now = Instant::now();
    if gesture_pending_in(&LAST_RELEASE.lock_or_recover(), recording_id, now) {
        return true;
    }
    let gate = TOGGLE_STOP_GATE.lock_or_recover();
    gesture_pending_in(&gate.press, recording_id, now)
        || gesture_pending_in(&gate.released, recording_id, now)
}

fn gesture_pending_in(slot: &Option<ReleaseStamp>, recording_id: u64, now: Instant) -> bool {
    slot.is_some_and(|stamp| {
        stamp.recording_id == recording_id
            && now.saturating_duration_since(stamp.released_at) <= STOP_GESTURE_WINDOW
    })
}

/// The first release of a recording is the one that stops it; a later
/// release while that stop is still queued must not overwrite it.
fn record_into(slot: &mut Option<ReleaseStamp>, stamp: ReleaseStamp) {
    if slot.is_some_and(|pending| pending.recording_id == stamp.recording_id) {
        return;
    }
    *slot = Some(stamp);
}

/// Consume the stamp for the stop of `recording_id` entered at
/// `stop_entered_at`. The slot is always cleared.
pub fn take_for(recording_id: u64, stop_entered_at: Instant) -> Option<HotkeyRelease> {
    clear_toggle_stop_gate(recording_id);
    take_matching(
        &mut LAST_RELEASE.lock_or_recover(),
        recording_id,
        stop_entered_at,
    )
}

fn take_matching(
    slot: &mut Option<ReleaseStamp>,
    recording_id: u64,
    stop_entered_at: Instant,
) -> Option<HotkeyRelease> {
    let stamp = slot.take()?;
    let age = stop_entered_at.checked_duration_since(stamp.released_at)?;
    (stamp.recording_id == recording_id && age <= MAX_RELEASE_TO_STOP).then_some(HotkeyRelease {
        released_at: stamp.released_at,
        stop_entered_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance_metrics::MeasurementV1;

    fn stamp(recording_id: u64, released_at: Instant) -> Option<ReleaseStamp> {
        Some(ReleaseStamp {
            recording_id,
            released_at,
        })
    }

    #[test]
    fn matching_release_yields_release_to_stop_and_release_to_paste() {
        let released_at = Instant::now();
        let mut slot = stamp(7, released_at);
        let release = take_matching(&mut slot, 7, released_at + Duration::from_millis(18))
            .expect("same-generation release pairs with its stop");
        assert_eq!(slot, None);
        let stop = release.release_to_stop();
        assert_eq!(stop.stage, PerformanceStageV1::ReleaseToStop);
        assert_eq!(stop.duration_ms, MeasurementV1::measured(18));
        let paste = release.release_to_paste(released_at + Duration::from_millis(171));
        assert_eq!(paste.stage, PerformanceStageV1::ReleaseToPaste);
        assert_eq!(paste.duration_ms, MeasurementV1::measured(171));
    }

    #[test]
    fn release_from_another_generation_is_discarded() {
        let released_at = Instant::now();
        let mut slot = stamp(6, released_at);
        assert_eq!(
            take_matching(&mut slot, 7, released_at + Duration::from_millis(5)),
            None
        );
        assert_eq!(slot, None, "a stale stamp must not survive to a later stop");
    }

    #[test]
    fn stale_or_future_release_is_discarded() {
        let released_at = Instant::now();
        let mut slot = stamp(7, released_at);
        assert_eq!(
            take_matching(
                &mut slot,
                7,
                released_at + MAX_RELEASE_TO_STOP + Duration::from_millis(1)
            ),
            None
        );
        let later = released_at + Duration::from_millis(50);
        let mut slot = stamp(7, later);
        assert_eq!(take_matching(&mut slot, 7, released_at), None);
    }

    #[test]
    fn first_release_of_a_recording_survives_a_later_queued_release() {
        let first = Instant::now();
        let second = first + Duration::from_millis(40);
        let mut slot = None;
        record_into(
            &mut slot,
            ReleaseStamp {
                recording_id: 7,
                released_at: first,
            },
        );
        record_into(
            &mut slot,
            ReleaseStamp {
                recording_id: 7,
                released_at: second,
            },
        );
        assert_eq!(slot, stamp(7, first));
        let release = take_matching(&mut slot, 7, first + Duration::from_millis(60)).unwrap();
        assert_eq!(
            release.release_to_stop().duration_ms,
            MeasurementV1::measured(60)
        );

        // A newer recording's release replaces an unclaimed older stamp.
        let mut slot = stamp(6, first);
        record_into(
            &mut slot,
            ReleaseStamp {
                recording_id: 7,
                released_at: second,
            },
        );
        assert_eq!(slot, stamp(7, second));
    }

    #[test]
    fn stop_gesture_is_pending_only_for_its_own_recent_unclaimed_release() {
        let released_at = Instant::now();
        let slot = stamp(7, released_at);
        let soon = released_at + Duration::from_millis(5);
        assert!(gesture_pending_in(&slot, 7, soon));
        assert!(!gesture_pending_in(&slot, 8, soon), "another recording");
        assert!(!gesture_pending_in(&None, 7, soon), "no release seen");
        assert!(
            !gesture_pending_in(&slot, 7, released_at + STOP_GESTURE_WINDOW * 2),
            "a release whose stop never came must not silence the preview forever"
        );
        // Claiming the stop clears the slot, so nothing is pending afterwards.
        let mut slot = slot;
        assert!(take_matching(&mut slot, 7, soon).is_some());
        assert!(!gesture_pending_in(&slot, 7, soon));
    }

    #[test]
    fn asking_whether_a_stop_is_pending_never_consumes_the_stamp_the_stop_needs() {
        let released_at = Instant::now();
        let slot = stamp(7, released_at);
        // Any number of preview ticks may ask, at any time within the window.
        for ms in [0, 1, 49, 50, 499] {
            assert!(gesture_pending_in(
                &slot,
                7,
                released_at + Duration::from_millis(ms)
            ));
        }
        // The stop still gets the same release instant, which is what anchors
        // its tail (D's freshness check, `tail` after the release, is far
        // shorter than this window, so a stamp the stop would trust is always
        // one the preview already treats as pending).
        let mut slot = slot;
        let stop_entered_at = released_at + Duration::from_millis(30);
        let release = take_matching(&mut slot, 7, stop_entered_at).expect("stamp intact");
        assert_eq!(release.released_at(), released_at);
        assert!(slot.is_none());
    }

    #[test]
    fn stop_without_release_has_no_release_timing() {
        let mut slot = None;
        assert_eq!(take_matching(&mut slot, 7, Instant::now()), None);
    }

    #[test]
    fn toggle_stop_pending_gates_preview_without_yielding_hold_release() {
        let id = 9_100_001;
        let now = Instant::now();
        let _ = take_for(id, now);
        assert!(!stop_gesture_pending(id));

        record_toggle_stop_press(id, now);
        assert!(stop_gesture_pending(id));
        assert!(!stop_gesture_pending(id + 1));

        // Clearing an aborted press restores preview immediately.
        clear_toggle_stop_press(id);
        assert!(!stop_gesture_pending(id));

        // Clean tap sets released gate; a subsequent aborted press must NOT clear it.
        record_toggle_stop_released(id, now);
        assert!(stop_gesture_pending(id));
        record_toggle_stop_press(id, now + Duration::from_millis(60));
        clear_toggle_stop_press(id);
        assert!(stop_gesture_pending(id));

        // Stop consumes the gate without returning a hold release.
        assert_eq!(take_for(id, now + Duration::from_millis(80)), None);
        assert!(!stop_gesture_pending(id));

        // A second clean tap near the end of an earlier unclaimed release's
        // window refreshes the preview gate for a full STOP_GESTURE_WINDOW.
        let nearly_expired = now
            .checked_sub(STOP_GESTURE_WINDOW - Duration::from_millis(10))
            .unwrap_or(now);
        record_toggle_stop_released(id, nearly_expired);
        record_toggle_stop_released(id, now);
        assert!(gesture_pending_in(
            &TOGGLE_STOP_GATE.lock_or_recover().released,
            id,
            now + Duration::from_millis(50),
        ));
        assert_eq!(take_for(id, now + Duration::from_millis(5)), None);
    }
}
