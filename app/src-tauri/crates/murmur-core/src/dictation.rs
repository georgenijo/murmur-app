//! Dictation decisions independent of a window, event bus, capture engine or runtime.
//! The host serializes inputs, supplies monotonic time, and executes generation-bound
//! effects through AppContext. Completion delivery/history is the 1.3 boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Starting,
    Recording,
    Processing,
    Recovering,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Origin {
    Hold,
    #[default]
    Toggle,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchGate {
    pub configured: bool,
    pub onboarding_complete: bool,
    pub model_installed: bool,
    pub accessibility: bool,
    pub disabled: bool,
}
impl LaunchGate {
    pub fn armed(self) -> bool {
        self.configured && self.onboarding_complete && self.model_installed && self.accessibility
    }
    pub fn can_start(self) -> bool {
        self.armed() && !self.disabled
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Sync,
    HoldStart(Option<String>),
    HoldStop,
    HoldCancel,
    HoldOverStart,
    HoldOverEnd,
    Toggle(Option<String>),
    Escape,
    PasteLast,
    Lock,
    Level {
        generation: u64,
        level: f64,
        at_ms: u64,
    },
    SmartAnswer {
        generation: u64,
        pause: u64,
        probability: Option<f64>,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Start {
        origin: Origin,
        gesture_token: Option<String>,
    },
    Stop {
        generation: u64,
        auto: bool,
        hold: bool,
    },
    Cancel {
        generation: u64,
    },
    Score {
        generation: u64,
        pause: u64,
    },
    PasteLast,
}
#[derive(Debug, Default)]
pub struct Controller {
    pub gate: LaunchGate,
    pub silence_ms: u64,
    pub smart_enabled: bool,
    pub locked: bool,
    status: Status,
    generation: u64,
    origin: Origin,
    silence: Silence,
    checked_pause: u64,
    smart_in_flight: Option<(u64, u64)>,
    terminal_requested: bool,
    cancel_requested: bool,
}
impl Controller {
    /// Called at every authoritative native transition, never from React status.
    pub fn observe(&mut self, generation: u64, status: Status, origin: Origin) {
        if generation != self.generation || status != self.status {
            if generation != self.generation || status != Status::Recording {
                self.silence = Silence::default();
                self.checked_pause = 0;
            }
            if generation != self.generation || status == Status::Idle {
                self.terminal_requested = false;
                self.cancel_requested = false;
            }
            if (generation != self.generation && self.status != Status::Idle)
                || matches!(status, Status::Idle | Status::Recovering)
            {
                self.locked = false;
            }
            if generation != self.generation {
                self.origin = origin;
            }
            if matches!(
                status,
                Status::Idle | Status::Processing | Status::Recovering
            ) {
                self.origin = Origin::Toggle;
            }
            self.generation = generation;
            self.status = status;
        }
    }
    pub fn reject_start(&mut self, generation: u64) -> bool {
        if self.generation != generation || self.status != Status::Idle {
            return false;
        }
        self.locked = false;
        self.origin = Origin::Toggle;
        true
    }
    pub fn keyboard_recording(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Recording) && self.origin != Origin::Hold
    }
    fn finish(&mut self, cancel: bool, auto: bool, hold: bool) -> Option<Effect> {
        if (if cancel {
            self.cancel_requested
        } else {
            self.terminal_requested
        }) || matches!(self.status, Status::Idle | Status::Recovering)
        {
            return None;
        }
        if !cancel && self.status == Status::Processing {
            return None;
        }
        self.terminal_requested = true;
        self.cancel_requested |= cancel || self.status == Status::Starting;
        self.locked = false;
        Some(if cancel || self.status == Status::Starting {
            Effect::Cancel {
                generation: self.generation,
            }
        } else {
            Effect::Stop {
                generation: self.generation,
                auto,
                hold,
            }
        })
    }
    fn auto_eligible(&self) -> bool {
        self.gate.armed()
            && self.status == Status::Recording
            && self.origin != Origin::Hold
            && self.silence_ms > 0
            && !self.terminal_requested
    }
    pub fn input(&mut self, input: Input) -> Option<Effect> {
        match input {
            Input::PasteLast => return Some(Effect::PasteLast),
            Input::Escape if self.gate.armed() => return self.finish(true, false, false),
            Input::HoldCancel => {
                self.origin = Origin::Toggle;
                return self.finish(true, false, false);
            }
            Input::HoldStop => {
                self.origin = Origin::Toggle;
                return self.finish(false, false, true);
            }
            Input::HoldOverStart => {
                self.origin = Origin::Hold;
                return None;
            }
            Input::HoldOverEnd => {
                self.origin = Origin::Toggle;
                return None;
            }
            Input::SmartAnswer {
                generation,
                pause,
                probability,
            } => {
                if self.smart_in_flight != Some((generation, pause)) {
                    return None;
                }
                self.smart_in_flight = None;
                if generation != self.generation
                    || !self.auto_eligible()
                    || !self.smart_enabled
                    || self.silence.stopped
                    || self.silence.pause != pause
                    || self.silence.silence_ms == 0
                {
                    return None;
                }
                if probability.is_some_and(|p| p.is_finite() && p >= 0.7) {
                    self.silence.stopped = true;
                    return self.finish(false, true, false);
                }
                return None;
            }
            Input::Level {
                generation,
                level,
                at_ms,
            } => {
                if generation != self.generation || !self.auto_eligible() {
                    return None;
                }
                if self.silence.sample(level, at_ms, self.silence_ms) {
                    return self.finish(false, true, false);
                }
                if self.smart_enabled
                    && self.smart_in_flight.is_none()
                    && self.silence.armed
                    && !self.silence.stopped
                    && self.silence.silence_ms >= 300
                    && self.silence.pause != self.checked_pause
                {
                    let pause = self.silence.pause;
                    self.checked_pause = pause;
                    self.smart_in_flight = Some((generation, pause));
                    return Some(Effect::Score { generation, pause });
                }
                return None;
            }
            _ => {}
        }
        // Admission gates apply only to inputs that would start capture.
        // Existing owners must still be stoppable/cancellable after Disable.
        if self.status == Status::Idle && !self.gate.can_start() {
            return None;
        }
        match input {
            Input::HoldStart(gesture_token) if self.status == Status::Idle => {
                self.origin = Origin::Hold;
                Some(Effect::Start {
                    origin: Origin::Hold,
                    gesture_token,
                })
            }
            Input::Toggle(gesture_token) => {
                self.origin = Origin::Toggle;
                if self.status == Status::Idle {
                    Some(Effect::Start {
                        origin: Origin::Toggle,
                        gesture_token,
                    })
                } else {
                    self.finish(false, false, false)
                }
            }
            Input::Lock => {
                if self.status == Status::Idle {
                    self.locked = true;
                    Some(Effect::Start {
                        origin: Origin::Toggle,
                        gesture_token: None,
                    })
                } else if matches!(self.status, Status::Starting | Status::Recording) {
                    if self.locked || self.status == Status::Starting {
                        self.finish(false, false, false)
                    } else {
                        self.locked = true;
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
#[derive(Debug, Default)]
pub struct Silence {
    pub peak: f64,
    pub speech_ms: u64,
    pub armed: bool,
    pub silence_ms: u64,
    pub pause: u64,
    pub last_sample_ms: Option<u64>,
    pub stopped: bool,
}
impl Silence {
    pub fn threshold(&self) -> f64 {
        0.015_f64.max(self.peak * 0.08)
    }
    pub fn sample(&mut self, level: f64, at_ms: u64, ceiling: u64) -> bool {
        if self.stopped || ceiling == 0 {
            return false;
        }
        let level = if level.is_finite() {
            level.max(0.0)
        } else {
            0.0
        };
        self.peak = self.peak.max(level);
        let elapsed = self
            .last_sample_ms
            .map_or(0, |last| at_ms.saturating_sub(last).min(500));
        self.last_sample_ms = Some(at_ms);
        if level >= self.threshold() {
            self.speech_ms += elapsed;
            self.armed |= self.speech_ms >= 400;
            self.silence_ms = 0;
        } else if self.armed {
            if self.silence_ms == 0 && elapsed > 0 {
                self.pause += 1;
            }
            self.silence_ms += elapsed;
            self.stopped = self.silence_ms >= ceiling;
        }
        self.stopped
    }
}

/// Bind delayed native input to the recording visible when it was enqueued.
/// A release during the asynchronous start belongs to that accepted start,
/// even if readiness overtook the start response; it must cancel, not deliver.
#[derive(Default)]
pub struct QueueGuard {
    accepted_start: Option<(u64, u64, std::time::Instant)>,
}
impl QueueGuard {
    pub fn accepted(&mut self, before: u64, generation: u64, completed: std::time::Instant) {
        self.accepted_start = Some((before, generation, completed));
    }
    pub fn route(
        &self,
        before: u64,
        current: u64,
        queued: std::time::Instant,
        input: Input,
    ) -> Option<Input> {
        if matches!(
            input,
            Input::Sync | Input::PasteLast | Input::Level { .. } | Input::SmartAnswer { .. }
        ) {
            return Some(input);
        }
        let behind_start = self
            .accepted_start
            .is_some_and(|(previous, accepted, completed)| {
                (before == previous || before == accepted)
                    && current == accepted
                    && queued <= completed
            });
        if before != current && !behind_start {
            return None;
        }
        Some(if behind_start && input == Input::HoldStop {
            Input::HoldCancel
        } else {
            input
        })
    }
}
