//! Stop-key timing for the Runs timeline: how long the key was held and how
//! long the stop command took to start after release.
use crate::performance_metrics::{PerformanceStageV1, StageTimingV1};
use crate::MutexExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(crate) struct StopPress {
    pub recording_id: u64,
    pub pressed_at: Instant,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct StopStamp {
    pub press: StopPress,
    pub released_at: Instant,
}
static STOP: Mutex<Option<StopStamp>> = Mutex::new(None);

pub(crate) fn record(stamp: StopStamp) {
    let mut slot = STOP.lock_or_recover();
    if slot.is_none_or(|old| old.press.recording_id != stamp.press.recording_id) {
        *slot = Some(stamp);
    }
}
pub(crate) fn take_for(id: u64, entered: Instant) -> Option<StopStamp> {
    take_matching(&mut STOP.lock_or_recover(), id, entered)
}
fn take_matching(slot: &mut Option<StopStamp>, id: u64, entered: Instant) -> Option<StopStamp> {
    let stamp = slot.take()?;
    (stamp.press.recording_id == id
        && stamp.released_at >= stamp.press.pressed_at
        && entered.checked_duration_since(stamp.released_at)? <= Duration::from_secs(10))
    .then_some(stamp)
}
impl StopStamp {
    pub fn stages(self, entered: Instant) -> [StageTimingV1; 2] {
        [
            StageTimingV1::measured(
                PerformanceStageV1::StopPressToRelease,
                self.released_at
                    .saturating_duration_since(self.press.pressed_at)
                    .as_millis() as u64,
            ),
            StageTimingV1::measured(
                PerformanceStageV1::StopReleaseToCommand,
                entered
                    .saturating_duration_since(self.released_at)
                    .as_millis() as u64,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_stamps_are_generation_scoped_consumed_and_expire() {
        let now = Instant::now();
        let stamp = StopStamp {
            press: StopPress {
                recording_id: 7,
                pressed_at: now,
            },
            released_at: now + Duration::from_millis(80),
        };
        let entered = now + Duration::from_millis(92);
        let mut slot = Some(stamp);
        let result = take_matching(&mut slot, 7, entered).unwrap();
        assert_eq!(result.stages(entered)[0].duration_ms.value(), Some(&80));
        assert_eq!(result.stages(entered)[1].duration_ms.value(), Some(&12));
        assert!(slot.is_none());
        assert!(take_matching(&mut Some(stamp), 8, entered).is_none());
        assert!(take_matching(&mut Some(stamp), 7, now + Duration::from_secs(11)).is_none());
    }
}
