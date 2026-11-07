//! Bounded, content-free evidence from an already-owned Settings preview.

use crate::microphone_preview::{
    classify_level, PreviewLevelAccumulator, PreviewLevelClassification,
};
use serde::Serialize;
use std::time::{Duration, Instant};

pub(crate) const VERIFICATION_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const VERIFICATION_COOLDOWN: Duration = Duration::from_secs(10);
const SIGNAL_HOLD: Duration = Duration::from_secs(1);
const FRAME: Duration = Duration::from_millis(20);
const MAX_CALLBACK_GAP: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SignalVerificationResult {
    Verified,
    NoPcm,
    InsufficientSignal,
    Interrupted,
}

pub(crate) struct SignalVerification {
    started_at: Instant,
    pub(crate) deadline: Instant,
    last_callback: Option<Instant>,
    sample_rate: Option<u32>,
    frame: PreviewLevelAccumulator,
    frame_samples: usize,
    signal_frames: u32,
    signal_since: Option<Instant>,
    observed_pcm: bool,
    verified: bool,
}

impl SignalVerification {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            deadline: now + VERIFICATION_WINDOW,
            last_callback: None,
            sample_rate: None,
            frame: PreviewLevelAccumulator::default(),
            frame_samples: 0,
            signal_frames: 0,
            signal_since: None,
            observed_pcm: false,
            verified: false,
        }
    }

    pub(crate) fn observe(&mut self, samples: &[f32], sample_rate: u32, now: Instant) {
        if now < self.started_at
            || now >= self.deadline
            || samples.is_empty()
            || sample_rate < 1_000
        {
            return;
        }
        self.observed_pcm = true;
        if self.sample_rate != Some(sample_rate)
            || self
                .last_callback
                .is_some_and(|last| now.saturating_duration_since(last) > MAX_CALLBACK_GAP)
        {
            self.frame = PreviewLevelAccumulator::default();
            self.frame_samples = 0;
            self.signal_frames = 0;
            self.signal_since = None;
        }
        self.sample_rate = Some(sample_rate);
        self.last_callback = Some(now);
        let frame_size = (u64::from(sample_rate) * FRAME.as_millis() as u64 / 1_000) as usize;
        for sample in samples {
            // Restart the full frame after invalid PCM rather than allowing
            // the meter's finite-only average to turn it into evidence.
            if !sample.is_finite() {
                self.frame = PreviewLevelAccumulator::default();
                self.frame_samples = 0;
                self.signal_frames = 0;
                self.signal_since = None;
                continue;
            }
            self.frame.observe(std::slice::from_ref(sample));
            self.frame_samples += 1;
            if self.frame_samples < frame_size {
                continue;
            }
            let (rms, peak) = self.frame.take();
            self.frame_samples = 0;
            if classify_level(rms, peak) == PreviewLevelClassification::SignalDetected {
                self.signal_frames += 1;
                let since = *self.signal_since.get_or_insert(now);
                if FRAME * self.signal_frames >= SIGNAL_HOLD
                    && now.saturating_duration_since(since) >= SIGNAL_HOLD
                {
                    self.verified = true;
                }
            } else {
                self.signal_frames = 0;
                self.signal_since = None;
            }
        }
    }

    pub(crate) fn result(&self) -> SignalVerificationResult {
        if self.verified {
            SignalVerificationResult::Verified
        } else if self.observed_pcm {
            SignalVerificationResult::InsufficientSignal
        } else {
            SignalVerificationResult::NoPcm
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustained_signal_requires_pcm_duration_and_wall_time_at_both_rates() {
        for rate in [16_000, 48_000] {
            let start = Instant::now();
            let mut verification = SignalVerification::new(start);
            let samples = vec![0.1; rate as usize / 50];
            for frame in 0..50 {
                verification.observe(&samples, rate, start + FRAME * frame);
            }
            assert_eq!(
                verification.result(),
                SignalVerificationResult::InsufficientSignal
            );
            verification.observe(&samples, rate, start + SIGNAL_HOLD);
            assert_eq!(verification.result(), SignalVerificationResult::Verified);

            let mut burst = SignalVerification::new(start);
            burst.observe(&vec![0.1; rate as usize * 2], rate, start);
            assert_eq!(burst.result(), SignalVerificationResult::InsufficientSignal);
        }
    }

    #[test]
    fn silence_clipping_quiet_and_invalid_pcm_never_verify() {
        for level in [0.0, 0.001, 1.0, f32::NAN, f32::INFINITY] {
            let start = Instant::now();
            let mut verification = SignalVerification::new(start);
            for frame in 0..200 {
                verification.observe(&[level; 320], 16_000, start + FRAME * frame);
            }
            assert_eq!(
                verification.result(),
                SignalVerificationResult::InsufficientSignal
            );
        }
    }

    #[test]
    fn gaps_rate_changes_and_brief_spikes_restart_the_hold() {
        let start = Instant::now();
        let mut verification = SignalVerification::new(start);
        for frame in 0..30 {
            verification.observe(&[0.1; 320], 16_000, start + FRAME * frame);
        }
        for frame in 50..80 {
            verification.observe(&[0.1; 320], 16_000, start + FRAME * frame);
        }
        assert_eq!(
            verification.result(),
            SignalVerificationResult::InsufficientSignal
        );
        verification.observe(&[0.1; 960], 48_000, start + FRAME * 80);
        verification.observe(&[0.0; 960], 48_000, start + FRAME * 81);
        verification.observe(&[0.1; 960], 48_000, start + FRAME * 82);
        assert_eq!(
            verification.result(),
            SignalVerificationResult::InsufficientSignal
        );
    }

    #[test]
    fn deadline_excludes_late_pcm() {
        let start = Instant::now();
        let mut verification = SignalVerification::new(start);
        assert_eq!(verification.result(), SignalVerificationResult::NoPcm);
        verification.observe(&[0.1; 320], 16_000, start - FRAME);
        assert_eq!(verification.result(), SignalVerificationResult::NoPcm);
        verification.observe(&[0.1; 320], 16_000, start + VERIFICATION_WINDOW);
        assert_eq!(verification.result(), SignalVerificationResult::NoPcm);
    }

    #[test]
    fn intermittent_invalid_samples_cannot_bridge_a_signal_hold() {
        let start = Instant::now();
        let mut verification = SignalVerification::new(start);
        let mut samples = [0.1; 320];
        samples[159] = f32::NAN;
        for frame in 0..200 {
            verification.observe(&samples, 16_000, start + FRAME * frame);
        }
        assert_eq!(
            verification.result(),
            SignalVerificationResult::InsufficientSignal
        );
    }
}
