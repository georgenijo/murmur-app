use murmur_capture_helper_protocol::{
    EchoCancellationBypassReason, EchoCancellationMode, EchoCancellationStatus,
};
use webrtc_audio_processing::{config::EchoCanceller, Config, Processor};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 480;
pub(super) const BACKLOG_HIGH_WATER_SAMPLES: usize = 48_000;

trait EchoProcessor {
    fn analyze_render(&mut self, frame: &[f32; FRAME_SAMPLES]) -> Result<(), ()>;
    fn process_capture(&mut self, frame: &mut [f32; FRAME_SAMPLES]) -> Result<(), ()>;
}

struct WebRtcEchoProcessor {
    processor: Processor,
}

impl WebRtcEchoProcessor {
    fn new() -> Result<Self, ()> {
        let processor = Processor::new(SAMPLE_RATE).map_err(|_| ())?;
        processor.set_config(Config {
            echo_canceller: Some(EchoCanceller::Full {
                stream_delay_ms: None,
            }),
            ..Config::default()
        });
        Ok(Self { processor })
    }
}

impl EchoProcessor for WebRtcEchoProcessor {
    fn analyze_render(&mut self, frame: &[f32; FRAME_SAMPLES]) -> Result<(), ()> {
        self.processor
            .analyze_render_frame([&frame[..]])
            .map_err(|_| ())
    }

    fn process_capture(&mut self, frame: &mut [f32; FRAME_SAMPLES]) -> Result<(), ()> {
        self.processor
            .process_capture_frame([&mut frame[..]])
            .map_err(|_| ())
    }
}

#[derive(Clone)]
struct RawTail {
    samples: [f32; FRAME_SAMPLES],
    len: usize,
}

impl Default for RawTail {
    fn default() -> Self {
        Self {
            samples: [0.0; FRAME_SAMPLES],
            len: 0,
        }
    }
}

struct RenderFramer {
    input_rate: u32,
    input_samples_seen: u64,
    next_output_numerator: u64,
    previous_sample: Option<f32>,
    frame: [f32; FRAME_SAMPLES],
    frame_len: usize,
}

impl RenderFramer {
    fn new(input_rate: u32) -> Result<Self, ()> {
        if input_rate < 8_000 {
            return Err(());
        }
        Ok(Self {
            input_rate,
            input_samples_seen: 0,
            next_output_numerator: 0,
            previous_sample: None,
            frame: [0.0; FRAME_SAMPLES],
            frame_len: 0,
        })
    }

    fn emit_sample(&mut self, sample: f32, processor: &mut dyn EchoProcessor) -> Result<(), ()> {
        self.frame[self.frame_len] = sample;
        self.frame_len += 1;
        if self.frame_len == FRAME_SAMPLES {
            processor.analyze_render(&self.frame)?;
            self.frame_len = 0;
        }
        Ok(())
    }

    fn push(&mut self, samples: &[f32], processor: &mut dyn EchoProcessor) -> Result<(), ()> {
        let output_rate = u64::from(SAMPLE_RATE);
        for &sample in samples {
            let input_index = self.input_samples_seen;
            if input_index == 0 {
                self.emit_sample(sample, processor)?;
                self.next_output_numerator = u64::from(self.input_rate);
            } else if let Some(previous) = self.previous_sample {
                let interval_start = (input_index - 1).saturating_mul(output_rate);
                let interval_end = input_index.saturating_mul(output_rate);
                while self.next_output_numerator <= interval_end {
                    let fraction =
                        (self.next_output_numerator - interval_start) as f32 / output_rate as f32;
                    self.emit_sample(previous * (1.0 - fraction) + sample * fraction, processor)?;
                    self.next_output_numerator = self
                        .next_output_numerator
                        .saturating_add(u64::from(self.input_rate));
                }
            }
            self.previous_sample = Some(sample);
            self.input_samples_seen = self.input_samples_seen.saturating_add(1);
        }
        Ok(())
    }
}

struct ActiveAec {
    processor: Box<dyn EchoProcessor>,
    render: RenderFramer,
    capture: RawTail,
}

enum PathState {
    Disabled,
    Active(Box<ActiveAec>),
    Bypassed {
        reason: EchoCancellationBypassReason,
        pending: Box<RawTail>,
    },
}

pub(super) struct MeetingMicrophonePath {
    state: PathState,
}

impl MeetingMicrophonePath {
    pub(super) fn new(mode: EchoCancellationMode, microphone_rate: u32, system_rate: u32) -> Self {
        if mode == EchoCancellationMode::Disabled {
            return Self {
                state: PathState::Disabled,
            };
        }
        if microphone_rate != SAMPLE_RATE {
            return Self::bypassed(EchoCancellationBypassReason::UnsupportedFormat);
        }
        match Self::new_active(system_rate) {
            Ok(active) => Self {
                state: PathState::Active(active),
            },
            Err(reason) => Self::bypassed(reason),
        }
    }

    fn new_active(system_rate: u32) -> Result<Box<ActiveAec>, EchoCancellationBypassReason> {
        let render = RenderFramer::new(system_rate)
            .map_err(|_| EchoCancellationBypassReason::UnsupportedFormat)?;
        let processor = WebRtcEchoProcessor::new()
            .map_err(|_| EchoCancellationBypassReason::InitializationFailed)?;
        Ok(Box::new(ActiveAec {
            processor: Box::new(processor),
            render,
            capture: RawTail::default(),
        }))
    }

    fn bypassed(reason: EchoCancellationBypassReason) -> Self {
        Self {
            state: PathState::Bypassed {
                reason,
                pending: Box::new(RawTail::default()),
            },
        }
    }

    #[cfg(test)]
    fn with_processor(processor: Box<dyn EchoProcessor>, system_rate: u32) -> Self {
        Self {
            state: PathState::Active(Box::new(ActiveAec {
                processor,
                render: RenderFramer::new(system_rate).unwrap(),
                capture: RawTail::default(),
            })),
        }
    }

    pub(super) fn status(&self) -> EchoCancellationStatus {
        match &self.state {
            PathState::Disabled => EchoCancellationStatus::Disabled,
            PathState::Active(_) => EchoCancellationStatus::Active,
            PathState::Bypassed { reason, .. } => {
                EchoCancellationStatus::Bypassed { reason: *reason }
            }
        }
    }

    pub(super) fn push_render(&mut self, samples: &[f32]) -> Option<EchoCancellationStatus> {
        let state = std::mem::replace(&mut self.state, PathState::Disabled);
        match state {
            PathState::Active(mut active) => {
                if active
                    .render
                    .push(samples, active.processor.as_mut())
                    .is_err()
                {
                    let ActiveAec { capture, .. } = *active;
                    self.state = PathState::Bypassed {
                        reason: EchoCancellationBypassReason::ProcessorFailed,
                        pending: Box::new(capture),
                    };
                    Some(self.status())
                } else {
                    self.state = PathState::Active(active);
                    None
                }
            }
            other => {
                self.state = other;
                None
            }
        }
    }

    pub(super) fn bypass_for_backlog(
        &mut self,
        microphone_samples: usize,
        system_samples: usize,
    ) -> Option<EchoCancellationStatus> {
        if microphone_samples < BACKLOG_HIGH_WATER_SAMPLES
            && system_samples < BACKLOG_HIGH_WATER_SAMPLES
        {
            return None;
        }
        let state = std::mem::replace(&mut self.state, PathState::Disabled);
        match state {
            PathState::Active(active) => {
                let ActiveAec { capture, .. } = *active;
                self.state = PathState::Bypassed {
                    reason: EchoCancellationBypassReason::ProcessingBacklog,
                    pending: Box::new(capture),
                };
                Some(self.status())
            }
            other => {
                self.state = other;
                None
            }
        }
    }

    pub(super) fn bypass(
        &mut self,
        reason: EchoCancellationBypassReason,
    ) -> Option<EchoCancellationStatus> {
        let state = std::mem::replace(&mut self.state, PathState::Disabled);
        match state {
            PathState::Active(active) => {
                let ActiveAec { capture, .. } = *active;
                self.state = PathState::Bypassed {
                    reason,
                    pending: Box::new(capture),
                };
                Some(self.status())
            }
            other => {
                self.state = other;
                None
            }
        }
    }

    /// Start a fresh processor after a transient timing failure. The caller
    /// must keep emitting raw microphone audio until the saved partial frame
    /// has drained, so recovery never consumes that frame twice.
    pub(super) fn restart(&mut self, system_rate: u32) -> Option<EchoCancellationStatus> {
        let state = std::mem::replace(&mut self.state, PathState::Disabled);
        match state {
            PathState::Bypassed { reason: _, pending } if pending.len == 0 => {
                match Self::new_active(system_rate) {
                    Ok(active) => {
                        self.state = PathState::Active(active);
                        Some(EchoCancellationStatus::Active)
                    }
                    Err(reason) => {
                        self.state = PathState::Bypassed { reason, pending };
                        Some(EchoCancellationStatus::Bypassed { reason })
                    }
                }
            }
            other => {
                self.state = other;
                None
            }
        }
    }

    pub(super) fn push_capture<E>(
        &mut self,
        samples: &[f32],
        mut emit: impl FnMut(&[f32]) -> Result<(), E>,
    ) -> Result<Option<EchoCancellationStatus>, E> {
        let state = std::mem::replace(&mut self.state, PathState::Disabled);
        match state {
            PathState::Disabled => {
                emit(samples)?;
                self.state = PathState::Disabled;
                Ok(None)
            }
            PathState::Bypassed {
                reason,
                mut pending,
            } => {
                if pending.len > 0 {
                    emit(&pending.samples[..pending.len])?;
                    pending.len = 0;
                }
                emit(samples)?;
                self.state = PathState::Bypassed { reason, pending };
                Ok(None)
            }
            PathState::Active(mut active) => {
                let mut consumed = 0;
                while consumed < samples.len() {
                    let copy = (FRAME_SAMPLES - active.capture.len).min(samples.len() - consumed);
                    active.capture.samples[active.capture.len..active.capture.len + copy]
                        .copy_from_slice(&samples[consumed..consumed + copy]);
                    active.capture.len += copy;
                    consumed += copy;
                    if active.capture.len != FRAME_SAMPLES {
                        continue;
                    }
                    let raw = active.capture.samples;
                    let mut processed = raw;
                    if active.processor.process_capture(&mut processed).is_err() {
                        emit(&raw)?;
                        if consumed < samples.len() {
                            emit(&samples[consumed..])?;
                        }
                        self.state = PathState::Bypassed {
                            reason: EchoCancellationBypassReason::ProcessorFailed,
                            pending: Box::new(RawTail::default()),
                        };
                        return Ok(Some(self.status()));
                    }
                    emit(&processed)?;
                    active.capture.len = 0;
                }
                self.state = PathState::Active(active);
                Ok(None)
            }
        }
    }

    pub(super) fn finish<E>(self, mut emit: impl FnMut(&[f32]) -> Result<(), E>) -> Result<(), E> {
        match self.state {
            PathState::Active(active) => {
                if active.capture.len > 0 {
                    emit(&active.capture.samples[..active.capture.len])?;
                }
            }
            PathState::Bypassed { pending, .. } => {
                if pending.len > 0 {
                    emit(&pending.samples[..pending.len])?;
                }
            }
            PathState::Disabled => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PassThrough;

    impl EchoProcessor for PassThrough {
        fn analyze_render(&mut self, _: &[f32; FRAME_SAMPLES]) -> Result<(), ()> {
            Ok(())
        }

        fn process_capture(&mut self, _: &mut [f32; FRAME_SAMPLES]) -> Result<(), ()> {
            Ok(())
        }
    }

    struct MutateThenFail;

    impl EchoProcessor for MutateThenFail {
        fn analyze_render(&mut self, _: &[f32; FRAME_SAMPLES]) -> Result<(), ()> {
            Ok(())
        }

        fn process_capture(&mut self, frame: &mut [f32; FRAME_SAMPLES]) -> Result<(), ()> {
            frame.fill(0.0);
            Err(())
        }
    }

    #[test]
    fn disabled_path_is_bit_exact() {
        let input = [0.25, -0.5, 1.0];
        let mut output = Vec::new();
        let mut path =
            MeetingMicrophonePath::new(EchoCancellationMode::Disabled, SAMPLE_RATE, SAMPLE_RATE);
        path.push_capture(&input, |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn active_path_preserves_every_sample_across_frame_boundaries() {
        let input = (0..(FRAME_SAMPLES + 1))
            .map(|value| value as f32 / 1000.0)
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        let mut path = MeetingMicrophonePath::with_processor(Box::new(PassThrough), SAMPLE_RATE);
        path.push_capture(&input, |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        path.finish(|samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn processor_failure_emits_saved_raw_frame_then_bypasses() {
        let input = (0..(FRAME_SAMPLES + 7))
            .map(|value| value as f32 / 1000.0)
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        let mut path = MeetingMicrophonePath::with_processor(Box::new(MutateThenFail), SAMPLE_RATE);
        let status = path
            .push_capture(&input, |samples| {
                output.extend_from_slice(samples);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(output, input);
        assert_eq!(
            status,
            Some(EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::ProcessorFailed,
            })
        );
    }

    #[test]
    fn backlog_bypass_keeps_partial_capture_tail() {
        let mut path = MeetingMicrophonePath::with_processor(Box::new(PassThrough), SAMPLE_RATE);
        let mut output = Vec::new();
        path.push_capture(&[0.5; 17], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(
            path.bypass_for_backlog(BACKLOG_HIGH_WATER_SAMPLES, 0),
            Some(EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::ProcessingBacklog,
            })
        );
        path.push_capture(&[0.25; 3], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output, [vec![0.5; 17], vec![0.25; 3]].concat());
    }

    #[test]
    fn render_discontinuity_bypass_keeps_partial_capture_tail() {
        let mut path = MeetingMicrophonePath::with_processor(Box::new(PassThrough), SAMPLE_RATE);
        let mut output = Vec::new();
        path.push_capture(&[0.5; 17], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(
            path.bypass(EchoCancellationBypassReason::RenderDiscontinuity),
            Some(EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::RenderDiscontinuity,
            })
        );
        path.push_capture(&[0.25; 3], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output, [vec![0.5; 17], vec![0.25; 3]].concat());
    }

    #[test]
    fn native_processor_recovers_after_discontinuity_without_losing_raw_transition_audio() {
        let mut path =
            MeetingMicrophonePath::new(EchoCancellationMode::Enabled, SAMPLE_RATE, SAMPLE_RATE);
        assert_eq!(path.status(), EchoCancellationStatus::Active);

        let mut output = Vec::new();
        path.push_capture(&[0.5; 17], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(
            path.bypass(EchoCancellationBypassReason::RenderDiscontinuity),
            Some(EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::RenderDiscontinuity,
            })
        );
        assert_eq!(path.restart(SAMPLE_RATE), None);

        path.push_capture(&[0.25; 3], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output, [vec![0.5; 17], vec![0.25; 3]].concat());
        assert_eq!(
            path.restart(SAMPLE_RATE),
            Some(EchoCancellationStatus::Active)
        );

        path.push_render(&[0.0; FRAME_SAMPLES]);
        path.push_capture(&[0.75; FRAME_SAMPLES], |samples| {
            output.extend_from_slice(samples);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(output.len(), 20 + FRAME_SAMPLES);
    }
}
