//! Native stop-key gesture stamps for the early-final shadow measurement (#1081).
//!
//! A double-tap-mode stop travels rdev -> `double-tap-toggle` -> webview ->
//! `stop_native_recording`. The keyboard listener stamps the monotonic press
//! and release of the stop key when the detector fires, keyed by the recording
//! generation live at that moment, and publishes the pair only when the toggle
//! is emitted. The stop command consumes the pair for its own generation, so a
//! gesture that was rejected, never fired or belonged to an older recording is
//! never attributed to a later run. Monotonic instants cannot cross the
//! webview, so the stamp stays in Rust. It holds no content: two instants, one
//! generation ID and one flag.
//!
//! This is deliberately not folded into [`crate::hotkey_release`]: a matching
//! hold-release stamp turns a stop into `HoldRelease`, which changes the
//! release tail the supervisor applies. This module only measures.

use crate::performance_metrics::{PerformanceStageV1, StageTimingV1};
use crate::MutexExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A gesture older than this did not drive the stop it would be paired with.
const MAX_GESTURE_TO_STOP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GestureStamp {
    recording_id: u64,
    pressed_at: Instant,
    released_at: Instant,
    /// A live preview decode was in flight when the key went down. `None` when
    /// the app state was not reachable from the listener.
    preview_in_flight_at_press: Option<bool>,
}

static LAST_GESTURE: Mutex<Option<GestureStamp>> = Mutex::new(None);

/// The native stop gesture paired with the stop command that accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopGesture {
    pressed_at: Instant,
    released_at: Instant,
    stop_entered_at: Instant,
    preview_in_flight_at_press: Option<bool>,
}

impl StopGesture {
    /// Press -> release of the stop key.
    pub fn press_to_release(&self) -> StageTimingV1 {
        StageTimingV1::measured(
            PerformanceStageV1::StopPressToRelease,
            millis(self.released_at.saturating_duration_since(self.pressed_at)),
        )
    }

    /// Release -> `stop_native_recording_for` entry, the webview hop included.
    pub fn release_to_command(&self) -> StageTimingV1 {
        StageTimingV1::measured(
            PerformanceStageV1::StopReleaseToCommand,
            millis(
                self.stop_entered_at
                    .saturating_duration_since(self.released_at),
            ),
        )
    }

    /// When the stop key went down.
    pub fn pressed_at(&self) -> Instant {
        self.pressed_at
    }

    /// Whether a live preview decode was in flight at the press.
    pub fn preview_in_flight_at_press(&self) -> Option<bool> {
        self.preview_in_flight_at_press
    }

    /// A paired gesture for tests of the code that consumes one.
    #[cfg(test)]
    pub(crate) fn for_test(
        pressed_at: Instant,
        released_at: Instant,
        stop_entered_at: Instant,
    ) -> Self {
        Self {
            pressed_at,
            released_at,
            stop_entered_at,
            preview_in_flight_at_press: Some(false),
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Publish the stop gesture that just fired for `recording_id`.
pub fn record(
    recording_id: u64,
    pressed_at: Instant,
    released_at: Instant,
    preview_in_flight_at_press: Option<bool>,
) {
    record_into(
        &mut LAST_GESTURE.lock_or_recover(),
        GestureStamp {
            recording_id,
            pressed_at,
            released_at,
            preview_in_flight_at_press,
        },
    );
}

/// The first gesture of a recording is the one that stops it; a later tap while
/// that stop is still queued must not overwrite it.
fn record_into(slot: &mut Option<GestureStamp>, stamp: GestureStamp) {
    if slot.is_some_and(|pending| pending.recording_id == stamp.recording_id) {
        return;
    }
    *slot = Some(stamp);
}

/// Consume the stamp for the stop of `recording_id` entered at
/// `stop_entered_at`. The slot is always cleared.
pub fn take_for(recording_id: u64, stop_entered_at: Instant) -> Option<StopGesture> {
    take_matching(
        &mut LAST_GESTURE.lock_or_recover(),
        recording_id,
        stop_entered_at,
    )
}

fn take_matching(
    slot: &mut Option<GestureStamp>,
    recording_id: u64,
    stop_entered_at: Instant,
) -> Option<StopGesture> {
    let stamp = slot.take()?;
    let age = stop_entered_at.checked_duration_since(stamp.released_at)?;
    (stamp.recording_id == recording_id && age <= MAX_GESTURE_TO_STOP).then_some(StopGesture {
        pressed_at: stamp.pressed_at,
        released_at: stamp.released_at,
        stop_entered_at,
        preview_in_flight_at_press: stamp.preview_in_flight_at_press,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance_metrics::MeasurementV1;

    fn stamp(recording_id: u64, pressed_at: Instant, released_at: Instant) -> Option<GestureStamp> {
        Some(GestureStamp {
            recording_id,
            pressed_at,
            released_at,
            preview_in_flight_at_press: Some(false),
        })
    }

    #[test]
    fn matching_gesture_yields_both_stop_stages() {
        let pressed_at = Instant::now();
        let released_at = pressed_at + Duration::from_millis(96);
        let mut slot = stamp(7, pressed_at, released_at);
        let gesture = take_matching(&mut slot, 7, released_at + Duration::from_millis(31))
            .expect("same-generation gesture pairs with its stop");
        assert_eq!(slot, None);
        let hold = gesture.press_to_release();
        assert_eq!(hold.stage, PerformanceStageV1::StopPressToRelease);
        assert_eq!(hold.duration_ms, MeasurementV1::measured(96));
        let hop = gesture.release_to_command();
        assert_eq!(hop.stage, PerformanceStageV1::StopReleaseToCommand);
        assert_eq!(hop.duration_ms, MeasurementV1::measured(31));
        assert_eq!(gesture.pressed_at(), pressed_at);
        assert_eq!(gesture.preview_in_flight_at_press(), Some(false));
    }

    #[test]
    fn gesture_from_another_generation_is_discarded() {
        let pressed_at = Instant::now();
        let released_at = pressed_at + Duration::from_millis(80);
        let mut slot = stamp(6, pressed_at, released_at);
        assert_eq!(
            take_matching(&mut slot, 7, released_at + Duration::from_millis(5)),
            None
        );
        assert_eq!(slot, None, "a stale stamp must not survive to a later stop");
    }

    #[test]
    fn stale_or_future_gesture_is_discarded() {
        let pressed_at = Instant::now();
        let released_at = pressed_at + Duration::from_millis(80);
        let mut slot = stamp(7, pressed_at, released_at);
        assert_eq!(
            take_matching(
                &mut slot,
                7,
                released_at + MAX_GESTURE_TO_STOP + Duration::from_millis(1)
            ),
            None
        );
        let mut slot = stamp(7, pressed_at, released_at + Duration::from_millis(50));
        assert_eq!(take_matching(&mut slot, 7, released_at), None);
    }

    #[test]
    fn first_gesture_of_a_recording_survives_a_later_queued_tap() {
        let pressed_at = Instant::now();
        let released_at = pressed_at + Duration::from_millis(70);
        let mut slot = None;
        record_into(&mut slot, stamp(7, pressed_at, released_at).unwrap());
        record_into(
            &mut slot,
            stamp(
                7,
                pressed_at + Duration::from_millis(300),
                released_at + Duration::from_millis(300),
            )
            .unwrap(),
        );
        assert_eq!(slot, stamp(7, pressed_at, released_at));

        // A newer recording's gesture replaces an unclaimed older stamp.
        let mut slot = stamp(6, pressed_at, released_at);
        record_into(&mut slot, stamp(7, pressed_at, released_at).unwrap());
        assert_eq!(slot, stamp(7, pressed_at, released_at));
    }

    #[test]
    fn stop_without_a_native_gesture_has_no_stop_stages() {
        let mut slot = None;
        assert_eq!(take_matching(&mut slot, 7, Instant::now()), None);
    }
}
