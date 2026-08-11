use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

pub(crate) const NO_SIGNAL_RMS_THRESHOLD: f32 = 0.001;
pub(crate) const NO_SIGNAL_PEAK_THRESHOLD: f32 = 0.005;
pub(crate) const QUIET_RMS_THRESHOLD: f32 = 0.01;
pub(crate) const QUIET_PEAK_THRESHOLD: f32 = 0.05;
pub(crate) const CLIPPING_SAMPLE_THRESHOLD: f32 = 0.99;
const CLASSIFICATION_HOLD: Duration = Duration::from_millis(250);
const CLIPPING_HOLD: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewPhase {
    Connecting,
    Active,
    Stopping,
}

impl PreviewPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Active => "active",
            Self::Stopping => "stopping",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreviewError {
    kind: String,
    message: String,
}

#[derive(Clone, Debug)]
struct ActivePreview {
    id: u64,
    phase: PreviewPhase,
    still_connecting: bool,
    error: Option<PreviewError>,
}

#[derive(Default)]
struct PreviewInner {
    active: Option<ActivePreview>,
    terminal_error: Option<(u64, PreviewError)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophonePreviewStatus {
    pub(crate) preview_id: Option<u64>,
    pub(crate) state: String,
    pub(crate) still_connecting: bool,
    pub(crate) error_kind: Option<String>,
    pub(crate) message: Option<String>,
}

pub(crate) struct MicrophonePreviewState {
    next_id: AtomicU64,
    inner: Mutex<PreviewInner>,
    changed: Notify,
}

impl Default for MicrophonePreviewState {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(0),
            inner: Mutex::new(PreviewInner::default()),
            changed: Notify::new(),
        }
    }
}

impl MicrophonePreviewState {
    fn inner(&self) -> MutexGuard<'_, PreviewInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn claim(&self) -> Result<u64, String> {
        let mut inner = self.inner();
        if inner.active.is_some() {
            return Err("A microphone test is already active".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        inner.terminal_error = None;
        inner.active = Some(ActivePreview {
            id,
            phase: PreviewPhase::Connecting,
            still_connecting: false,
            error: None,
        });
        drop(inner);
        self.changed.notify_waiters();
        Ok(id)
    }

    pub(crate) fn current_id(&self) -> Option<u64> {
        self.inner().active.as_ref().map(|preview| preview.id)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.inner().active.is_some()
    }

    pub(crate) fn is_current(&self, preview_id: u64) -> bool {
        self.current_id() == Some(preview_id)
    }

    pub(crate) fn set_phase_if(&self, preview_id: u64, phase: PreviewPhase) -> bool {
        let mut inner = self.inner();
        let Some(active) = inner
            .active
            .as_mut()
            .filter(|active| active.id == preview_id)
        else {
            return false;
        };
        active.phase = phase;
        if phase != PreviewPhase::Connecting {
            active.still_connecting = false;
        }
        if phase == PreviewPhase::Active {
            active.error = None;
        }
        drop(inner);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn set_still_connecting_if(&self, preview_id: u64) -> bool {
        let mut inner = self.inner();
        let Some(active) = inner
            .active
            .as_mut()
            .filter(|active| active.id == preview_id)
        else {
            return false;
        };
        active.still_connecting = true;
        drop(inner);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn set_error_if(
        &self,
        preview_id: u64,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> bool {
        let mut inner = self.inner();
        let Some(active) = inner
            .active
            .as_mut()
            .filter(|active| active.id == preview_id)
        else {
            return false;
        };
        active.error = Some(PreviewError {
            kind: kind.into(),
            message: message.into(),
        });
        drop(inner);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn fail_and_clear(
        &self,
        preview_id: u64,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> bool {
        let mut inner = self.inner();
        if inner.active.as_ref().map(|active| active.id) != Some(preview_id) {
            return false;
        }
        let error = PreviewError {
            kind: kind.into(),
            message: message.into(),
        };
        inner.active = None;
        inner.terminal_error = Some((preview_id, error));
        drop(inner);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn clear_if(&self, preview_id: u64) -> bool {
        let mut inner = self.inner();
        if inner.active.as_ref().map(|active| active.id) != Some(preview_id) {
            return false;
        }
        let active = inner.active.take().expect("preview id was checked above");
        inner.terminal_error = active.error.map(|error| (preview_id, error));
        drop(inner);
        self.changed.notify_waiters();
        true
    }

    pub(crate) fn status(&self) -> MicrophonePreviewStatus {
        let inner = self.inner();
        if let Some(active) = inner.active.as_ref() {
            let error = active.error.as_ref();
            return MicrophonePreviewStatus {
                preview_id: Some(active.id),
                state: if error.is_some() {
                    "error".to_string()
                } else {
                    active.phase.as_str().to_string()
                },
                still_connecting: active.still_connecting,
                error_kind: error.map(|error| error.kind.clone()),
                message: error.map(|error| error.message.clone()),
            };
        }
        if let Some((_, error)) = inner.terminal_error.as_ref() {
            return MicrophonePreviewStatus {
                // Terminal errors retain their message, not capture ownership.
                // A null ID tells the UI Retry may safely claim a new preview.
                preview_id: None,
                state: "error".to_string(),
                still_connecting: false,
                error_kind: Some(error.kind.clone()),
                message: Some(error.message.clone()),
            };
        }
        MicrophonePreviewStatus {
            preview_id: None,
            state: "idle".to_string(),
            still_connecting: false,
            error_kind: None,
            message: None,
        }
    }

    pub(crate) async fn wait_until_inactive(&self, preview_id: u64, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.changed.notified();
                if !self.is_current(preview_id) {
                    return;
                }
                changed.await;
            }
        })
        .await
        .is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreviewLevelClassification {
    NoSignal,
    TooQuiet,
    SignalDetected,
    Clipping,
}

impl PreviewLevelClassification {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoSignal => "no_signal",
            Self::TooQuiet => "too_quiet",
            Self::SignalDetected => "signal_detected",
            Self::Clipping => "clipping",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Self::NoSignal => 1,
            Self::TooQuiet => 2,
            Self::SignalDetected => 4,
            Self::Clipping => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MicrophonePreviewLevel {
    pub(crate) preview_id: u64,
    pub(crate) rms: f32,
    pub(crate) peak: f32,
    pub(crate) classification: PreviewLevelClassification,
}

#[derive(Default)]
pub(crate) struct PreviewLevelAccumulator {
    sum_squares: f64,
    sample_count: u64,
    peak: f32,
}

impl PreviewLevelAccumulator {
    pub(crate) fn observe(&mut self, samples: &[f32]) {
        for sample in samples.iter().copied().filter(|sample| sample.is_finite()) {
            let magnitude = sample.abs().min(1.0);
            self.sum_squares += f64::from(magnitude) * f64::from(magnitude);
            self.sample_count += 1;
            self.peak = self.peak.max(magnitude);
        }
    }

    pub(crate) fn take(&mut self) -> (f32, f32) {
        let rms = if self.sample_count == 0 {
            0.0
        } else {
            (self.sum_squares / self.sample_count as f64).sqrt() as f32
        };
        let peak = self.peak;
        *self = Self::default();
        (rms.clamp(0.0, 1.0), peak.clamp(0.0, 1.0))
    }
}

pub(crate) fn classify_level(rms: f32, peak: f32) -> PreviewLevelClassification {
    let rms = if rms.is_finite() {
        rms.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let peak = if peak.is_finite() {
        peak.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if peak >= CLIPPING_SAMPLE_THRESHOLD {
        PreviewLevelClassification::Clipping
    } else if rms < NO_SIGNAL_RMS_THRESHOLD && peak < NO_SIGNAL_PEAK_THRESHOLD {
        PreviewLevelClassification::NoSignal
    } else if rms < QUIET_RMS_THRESHOLD || peak < QUIET_PEAK_THRESHOLD {
        PreviewLevelClassification::TooQuiet
    } else {
        PreviewLevelClassification::SignalDetected
    }
}

#[derive(Default)]
pub(crate) struct PreviewLevelTracker {
    current: Option<PreviewLevelClassification>,
    candidate: Option<(PreviewLevelClassification, Instant)>,
    clipping_until: Option<Instant>,
    observed: u8,
}

impl PreviewLevelTracker {
    pub(crate) fn stabilize(
        &mut self,
        raw: PreviewLevelClassification,
        now: Instant,
    ) -> (PreviewLevelClassification, bool) {
        let classification = if raw == PreviewLevelClassification::Clipping {
            self.current = Some(raw);
            self.candidate = None;
            self.clipping_until = Some(now + CLIPPING_HOLD);
            raw
        } else if self.current == Some(PreviewLevelClassification::Clipping)
            && self.clipping_until.is_some_and(|until| now < until)
        {
            PreviewLevelClassification::Clipping
        } else {
            match self.current {
                None => {
                    self.current = Some(raw);
                    self.candidate = None;
                    raw
                }
                Some(current) if current == raw => {
                    self.candidate = None;
                    current
                }
                Some(current) => {
                    let promote = self.candidate.is_some_and(|(candidate, since)| {
                        candidate == raw
                            && now.saturating_duration_since(since) >= CLASSIFICATION_HOLD
                    });
                    if promote {
                        self.current = Some(raw);
                        self.candidate = None;
                        self.clipping_until = None;
                        raw
                    } else {
                        if self.candidate.map(|(candidate, _)| candidate) != Some(raw) {
                            self.candidate = Some((raw, now));
                        }
                        current
                    }
                }
            }
        };
        let first_observation = self.observed & classification.bit() == 0;
        self.observed |= classification.bit();
        (classification, first_observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_classification_boundaries_are_deterministic() {
        assert_eq!(
            classify_level(0.0, 0.0),
            PreviewLevelClassification::NoSignal
        );
        assert_eq!(
            classify_level(NO_SIGNAL_RMS_THRESHOLD, NO_SIGNAL_PEAK_THRESHOLD),
            PreviewLevelClassification::TooQuiet
        );
        assert_eq!(
            classify_level(QUIET_RMS_THRESHOLD, QUIET_PEAK_THRESHOLD),
            PreviewLevelClassification::SignalDetected
        );
        assert_eq!(
            classify_level(0.2, CLIPPING_SAMPLE_THRESHOLD),
            PreviewLevelClassification::Clipping
        );
        assert_eq!(
            classify_level(f32::NAN, f32::INFINITY),
            PreviewLevelClassification::NoSignal
        );
    }

    #[test]
    fn accumulator_keeps_peaks_between_throttled_emissions_and_drops_non_finite_values() {
        let mut accumulator = PreviewLevelAccumulator::default();
        accumulator.observe(&[0.1, -0.2, f32::NAN]);
        accumulator.observe(&[1.4, f32::INFINITY]);
        let (rms, peak) = accumulator.take();
        assert!((rms - ((0.01_f64 + 0.04 + 1.0) / 3.0).sqrt() as f32).abs() < 0.0001);
        assert_eq!(peak, 1.0);
        assert_eq!(accumulator.take(), (0.0, 0.0));
    }

    #[test]
    fn classification_changes_are_held_and_clipping_is_immediate() {
        let start = Instant::now();
        let mut tracker = PreviewLevelTracker::default();
        assert_eq!(
            tracker.stabilize(PreviewLevelClassification::NoSignal, start),
            (PreviewLevelClassification::NoSignal, true)
        );
        assert_eq!(
            tracker.stabilize(
                PreviewLevelClassification::SignalDetected,
                start + Duration::from_millis(200)
            ),
            (PreviewLevelClassification::NoSignal, false)
        );
        assert_eq!(
            tracker.stabilize(
                PreviewLevelClassification::SignalDetected,
                start + Duration::from_millis(451)
            ),
            (PreviewLevelClassification::SignalDetected, true)
        );
        assert_eq!(
            tracker.stabilize(
                PreviewLevelClassification::Clipping,
                start + Duration::from_millis(452)
            ),
            (PreviewLevelClassification::Clipping, true)
        );
        assert_eq!(
            tracker.stabilize(
                PreviewLevelClassification::SignalDetected,
                start + Duration::from_millis(800)
            ),
            (PreviewLevelClassification::Clipping, false)
        );
    }

    #[tokio::test]
    async fn coordinator_uses_monotonic_ids_and_ignores_stale_mutations() {
        let state = MicrophonePreviewState::default();
        let first = state.claim().unwrap();
        assert_eq!(first, 1);
        assert!(state.set_phase_if(first, PreviewPhase::Active));
        assert!(!state.set_phase_if(first + 1, PreviewPhase::Stopping));
        assert!(state.clear_if(first));
        assert!(
            state
                .wait_until_inactive(first, Duration::from_millis(10))
                .await
        );
        let second = state.claim().unwrap();
        assert_eq!(second, 2);
        assert!(!state.clear_if(first));
        assert_eq!(state.current_id(), Some(second));
    }

    #[test]
    fn terminal_errors_survive_owner_clear_but_not_a_new_claim() {
        let state = MicrophonePreviewState::default();
        let id = state.claim().unwrap();
        assert!(state.set_error_if(id, "device_unavailable", "Choose another microphone."));
        assert!(state.clear_if(id));
        let status = state.status();
        assert_eq!(status.state, "error");
        assert_eq!(status.preview_id, None);
        assert_eq!(status.error_kind.as_deref(), Some("device_unavailable"));
        assert!(!state.is_active());

        state.claim().unwrap();
        assert_eq!(state.status().state, "connecting");
    }
}
