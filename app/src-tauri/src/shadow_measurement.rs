//! Content-free, post-delivery evidence for #1081. No early decode is reused.
use crate::performance_metrics::{PerformanceStageV1, StageTimingV1};
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(crate) struct StopPress {
    pub recording_id: u64,
    pub pressed_at: Instant,
    pub sample: Option<usize>,
    pub preview_held_backend: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeterminismStatus {
    Disabled,
    Pending,
    Compared,
    BackendUnavailable,
    Superseded,
    DecodeFailed,
    UnsupportedBackend,
    NotAttempted,
    SkippedForegroundLatencyRisk,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ShadowMeasurementV1 {
    #[serde(deserialize_with = "supported_version")]
    pub schema_version: u32,
    pub press_sample: Option<u64>,
    pub pause_sample: Option<u64>,
    pub trailing_silence_before_press_ms: Option<u64>,
    pub trailing_silence_before_stop_ms: Option<u64>,
    pub press_input_matches: Option<bool>,
    pub pause_input_matches: Option<bool>,
    pub preview_held_backend_at_press: Option<bool>,
    pub determinism: DeterminismStatus,
    pub decode_matches: Option<bool>,
}
fn supported_version<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == 1 {
        Ok(version)
    } else {
        Err(serde::de::Error::custom("unsupported shadow schema"))
    }
}
impl Default for ShadowMeasurementV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            press_sample: None,
            pause_sample: None,
            trailing_silence_before_press_ms: None,
            trailing_silence_before_stop_ms: None,
            press_input_matches: None,
            pause_input_matches: None,
            preview_held_backend_at_press: None,
            determinism: DeterminismStatus::NotAttempted,
            decode_matches: None,
        }
    }
}
/// A process diagnostics switch; no settings/profile changes during a recording.
/// Launch with MURMUR_SHADOW_DETERMINISM=1 to opt in. Default is strictly off.
pub(crate) fn determinism_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// The shared final decoder cannot be preempted. Even an idle try-lock can
/// delay a foreground request arriving just afterwards, so never admit a
/// diagnostic repeat on this runtime. This policy needs no backend capability.
pub(crate) fn repeat_status(enabled: bool, supported: bool) -> DeterminismStatus {
    if !supported {
        DeterminismStatus::UnsupportedBackend
    } else if !enabled {
        DeterminismStatus::Disabled
    } else {
        DeterminismStatus::SkippedForegroundLatencyRisk
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PostDeliveryBoundary {
    pub idle_observed_at: Instant,
    pub delivered_at: Instant,
}
impl PostDeliveryBoundary {
    fn assert_precedes_setup(self, setup_at: Instant) {
        debug_assert!(self.idle_observed_at <= self.delivered_at);
        debug_assert!(self.delivered_at <= setup_at);
    }
}

/// Already-owned PCM moves here after delivery timing and Idle; no copy and
/// no text/hash is needed. The worker has only a VAD capability, never a shared
/// model runtime or foreground-owner lock.
pub(crate) struct ShadowAudio {
    pub original: Vec<f32>,
    pub filtered: Vec<f32>,
    pub bounds: Option<Vec<(usize, usize)>>,
    pub stamp: Option<StopStamp>,
    pub boundary: PostDeliveryBoundary,
}
impl ShadowAudio {
    pub fn measure(
        self,
        enabled: bool,
        supported: bool,
        mut filter: impl FnMut(&[f32], usize, &[f32]) -> Option<bool>,
    ) -> ShadowMeasurementV1 {
        self.boundary.assert_precedes_setup(Instant::now());
        let mut result = ShadowMeasurementV1 {
            press_sample: self.stamp.and_then(|s| s.press.sample).map(|s| s as u64),
            preview_held_backend_at_press: self.stamp.map(|s| s.press.preview_held_backend),
            determinism: repeat_status(enabled, supported),
            ..Default::default()
        };
        if supported {
            if let Some(bounds) = self.bounds {
                result.trailing_silence_before_stop_ms =
                    trailing_silence_ms(&bounds, self.original.len());
                let press = self
                    .stamp
                    .and_then(|s| s.press.sample)
                    .filter(|at| *at <= self.original.len());
                result.trailing_silence_before_press_ms =
                    press.and_then(|at| trailing_silence_ms(&bounds, at));
                let pause = last_pause(&bounds, self.original.len());
                result.pause_sample = pause.map(|at| at as u64);
                result.press_input_matches =
                    press.and_then(|at| filter(&self.original, at, &self.filtered));
                result.pause_input_matches =
                    pause.and_then(|at| filter(&self.original, at, &self.filtered));
            }
        }
        result
    }
}

/// Bounds are the same padded, clamped segments used to assemble final PCM.
/// Inside a speech segment silence is zero; no preceding speech is unavailable.
pub(crate) fn trailing_silence_ms(bounds: &[(usize, usize)], at: usize) -> Option<u64> {
    bounds
        .iter()
        .filter(|(start, _)| *start <= at)
        .map(|(_, end)| (*end).min(at))
        .max()
        .map(|end| (at.saturating_sub(end) / 16) as u64)
}
/// First 300ms point of the latest qualifying quiet interval, including
/// trailing silence. This is an actual hypothetical trigger before final stop.
pub(crate) fn last_pause(bounds: &[(usize, usize)], end: usize) -> Option<usize> {
    bounds
        .iter()
        .enumerate()
        .filter_map(|(i, (_, speech_end))| {
            let pause_end = bounds.get(i + 1).map_or(end, |(start, _)| *start);
            (pause_end.saturating_sub(*speech_end) >= 4_800)
                .then_some(speech_end.saturating_add(4_800))
        })
        .next_back()
}
pub(crate) fn pcm_equal(left: &[f32], right: &[f32]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.to_bits() == b.to_bits())
}
pub(crate) fn prefix_matches(
    path: &str,
    samples: &[f32],
    at: usize,
    threshold: f32,
    full: &[f32],
) -> Option<bool> {
    prefix_matches_with(samples, at, full, |prefix| {
        crate::vad::filter_speech(path, prefix, threshold)
    })
}

fn prefix_matches_with(
    samples: &[f32],
    at: usize,
    full: &[f32],
    filter: impl FnOnce(&[f32]) -> Result<crate::vad::VadResult, String>,
) -> Option<bool> {
    match filter(samples.get(..at)?).ok()? {
        crate::vad::VadResult::Speech(prefix) => Some(pcm_equal(&prefix, full)),
        crate::vad::VadResult::NoSpeech => Some(full.is_empty()),
    }
}

/// Keep post-paste work bounded without making foreground operations wait.
static SHADOW_RUNNING: AtomicBool = AtomicBool::new(false);
pub(crate) struct ShadowPermit;
impl ShadowPermit {
    pub fn acquire() -> Option<Self> {
        SHADOW_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self)
    }
}
impl Drop for ShadowPermit {
    fn drop(&mut self) {
        SHADOW_RUNNING.store(false, Ordering::SeqCst);
    }
}
/// This flag spans only the preview closure, after it owns the backend.
pub(crate) struct PreviewBackendLease<'a>(&'a AtomicBool);
impl<'a> PreviewBackendLease<'a> {
    pub fn new(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}
impl Drop for PreviewBackendLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
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
                sample: Some(16000),
                preview_held_backend: true,
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
    #[test]
    fn padded_silence_and_last_pause_have_explicit_missing_values() {
        let bounds = [(1600, 8000), (16000, 24000)];
        assert_eq!(trailing_silence_ms(&bounds, 1000), None);
        assert_eq!(trailing_silence_ms(&bounds, 4000), Some(0));
        assert_eq!(trailing_silence_ms(&bounds, 12800), Some(300));
        assert_eq!(trailing_silence_ms(&bounds, 32000), Some(500));
        assert_eq!(last_pause(&bounds, 32000), Some(28800));
        assert_eq!(last_pause(&bounds, 25000), Some(12800));
        assert_eq!(last_pause(&[(0, 16000)], 20000), None);
    }
    #[test]
    fn equality_is_byte_exact_including_signed_zero_and_nan() {
        assert!(pcm_equal(&[1.0, 2.0], &[1.0, 2.0]));
        assert!(!pcm_equal(&[1.0], &[1.0, 2.0]));
        assert!(!pcm_equal(&[0.0], &[-0.0]));
        assert!(pcm_equal(
            &[f32::from_bits(0x7fc00001)],
            &[f32::from_bits(0x7fc00001)]
        ));
    }
    #[test]
    fn prefix_equality_runs_the_same_filter_and_handles_true_false_and_failure() {
        let samples = [1.0, 2.0, 3.0, 0.0];
        let filter = |input: &[f32]| {
            Ok(crate::vad::VadResult::Speech(
                input.iter().copied().filter(|x| *x != 0.0).collect(),
            ))
        };
        assert_eq!(
            prefix_matches_with(&samples, 3, &[1.0, 2.0, 3.0], filter),
            Some(true)
        );
        assert_eq!(
            prefix_matches_with(&samples, 2, &[1.0, 2.0, 3.0], filter),
            Some(false)
        );
        assert_eq!(prefix_matches_with(&samples, 5, &[], filter), None);
        assert_eq!(
            prefix_matches_with(&samples, 3, &[], |_| Err("unavailable".into())),
            None
        );
    }

    #[test]
    fn shadow_setup_follows_delivery_and_idle_and_receives_moved_buffers() {
        let original = vec![1.0; 10_000];
        let filtered = vec![1.0; 1_000];
        let original_ptr = original.as_ptr() as usize;
        let filtered_ptr = filtered.as_ptr() as usize;
        let idle_observed_at = Instant::now();
        let delivered_at = Instant::now();
        let result = std::thread::spawn(move || {
            ShadowAudio {
                original,
                filtered,
                bounds: Some(vec![(0, 1_000)]),
                stamp: None,
                boundary: PostDeliveryBoundary {
                    idle_observed_at,
                    delivered_at,
                },
            }
            .measure(true, true, |original, _, filtered| {
                let setup_at = Instant::now();
                assert!(idle_observed_at <= delivered_at && delivered_at <= setup_at);
                assert_eq!(
                    original.as_ptr() as usize,
                    original_ptr,
                    "original PCM must move, not copy"
                );
                assert_eq!(
                    filtered.as_ptr() as usize,
                    filtered_ptr,
                    "filtered PCM must move, not copy"
                );
                Some(true)
            })
        })
        .join()
        .unwrap();
        assert_eq!(
            result.determinism,
            DeterminismStatus::SkippedForegroundLatencyRisk
        );
        assert_eq!(result.decode_matches, None);
        assert_eq!(repeat_status(false, true), DeterminismStatus::Disabled);
        assert_eq!(
            repeat_status(true, false),
            DeterminismStatus::UnsupportedBackend
        );
    }

    #[test]
    fn determinism_is_off_by_default() {
        assert!(!determinism_enabled(None));
        assert!(!determinism_enabled(Some("true")));
        assert!(determinism_enabled(Some("1")));
    }
}
