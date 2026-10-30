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

/// The native release paired with the stop that accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyRelease {
    released_at: Instant,
    stop_entered_at: Instant,
}

impl HotkeyRelease {
    /// Release → `stop_native_recording_for` entry.
    pub fn release_to_stop(&self) -> StageTimingV1 {
        StageTimingV1::measured(
            PerformanceStageV1::ReleaseToStop,
            millis(self.stop_entered_at.saturating_duration_since(self.released_at)),
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
    *LAST_RELEASE.lock_or_recover() = Some(ReleaseStamp {
        recording_id,
        released_at,
    });
}

/// Consume the stamp for the stop of `recording_id` entered at
/// `stop_entered_at`. The slot is always cleared.
pub fn take_for(recording_id: u64, stop_entered_at: Instant) -> Option<HotkeyRelease> {
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
    fn stop_without_release_has_no_release_timing() {
        let mut slot = None;
        assert_eq!(take_matching(&mut slot, 7, Instant::now()), None);
    }
}
