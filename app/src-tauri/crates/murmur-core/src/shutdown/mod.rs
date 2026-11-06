//! Process-local shutdown policy. Time is monotonic milliseconds supplied by the
//! host; effects contain identities, never payloads or host handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Quit,
    Restart,
    System,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    Restart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Admission,
    Capture,
    Helpers,
    Writers,
    Terminate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    pub epoch: u64,
    pub step: Step,
    pub deadline_ms: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Committed,
    PreservedForReplay,
    UnpreservedFailure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Admission(bool),
    Capture(bool),
    Helpers(bool),
    Writers(WriteOutcome),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub effect: Effect,
    pub outcome: Outcome,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    ClosingAdmission,
    DrainingCapture,
    ConfirmingHelpers,
    FlushingWriters,
    ReadyToTerminate,
    TerminationIssued,
    Exited,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub producers_fenced: bool,
    pub capture_stopped: bool,
    pub helpers_confirmed_empty: bool,
    pub writes: WriteOutcome,
    pub timed_out: bool,
}
impl Default for Receipt {
    fn default() -> Self {
        Self {
            producers_fenced: false,
            capture_stopped: false,
            helpers_confirmed_empty: false,
            writes: WriteOutcome::UnpreservedFailure,
            timed_out: false,
        }
    }
}
#[derive(Debug)]
pub struct Coordinator {
    pub state: State,
    pub receipt: Receipt,
    action: Action,
    start: u64,
    current: Option<Effect>,
}
impl Default for Coordinator {
    fn default() -> Self {
        Self {
            state: State::Running,
            receipt: Receipt::default(),
            action: Action::Quit,
            start: 0,
            current: None,
        }
    }
}
impl Coordinator {
    pub fn action(&self) -> Action {
        if self.action == Action::Restart
            && self.receipt.producers_fenced
            && self.receipt.capture_stopped
            && self.receipt.helpers_confirmed_empty
            && self.receipt.writes != WriteOutcome::UnpreservedFailure
        {
            Action::Restart
        } else {
            Action::Quit
        }
    }
    pub fn termination_permitted(&self) -> bool {
        matches!(self.state, State::TerminationIssued | State::Exited)
    }
    pub fn request(&mut self, request: Request, now_ms: u64) -> Vec<Effect> {
        if self.termination_permitted() {
            return vec![];
        }
        if self.state != State::Running {
            if request != Request::Restart {
                self.action = Action::Quit;
            }
            return self.tick(now_ms);
        }
        self.start = now_ms;
        self.action = if request == Request::Restart {
            Action::Restart
        } else {
            Action::Quit
        };
        vec![self.begin(Step::Admission)]
    }
    fn begin(&mut self, step: Step) -> Effect {
        let (state, offset) = match step {
            Step::Admission => (State::ClosingAdmission, 250),
            Step::Capture => (State::DrainingCapture, 5_000),
            Step::Helpers => (State::ConfirmingHelpers, 7_000),
            Step::Writers => (State::FlushingWriters, 10_000),
            Step::Terminate => (State::ReadyToTerminate, 10_000),
        };
        self.state = state;
        let effect = Effect {
            epoch: 1,
            step,
            deadline_ms: self.start.saturating_add(offset),
        };
        self.current = Some(effect);
        effect
    }
    fn next(&mut self) -> Effect {
        let step = match self.current.unwrap().step {
            Step::Admission => Step::Capture,
            Step::Capture => Step::Helpers,
            Step::Helpers => Step::Writers,
            Step::Writers => Step::Terminate,
            Step::Terminate => unreachable!(),
        };
        self.begin(step)
    }
    pub fn advance(&mut self, event: Event, now_ms: u64) -> Vec<Effect> {
        if self.termination_permitted() || self.current != Some(event.effect) {
            return vec![];
        }
        let valid = match (event.effect.step, event.outcome) {
            (Step::Admission, Outcome::Admission(v)) => {
                self.receipt.producers_fenced = v;
                true
            }
            (Step::Capture, Outcome::Capture(v)) => {
                self.receipt.capture_stopped = v;
                true
            }
            (Step::Helpers, Outcome::Helpers(v)) => {
                self.receipt.helpers_confirmed_empty = v;
                true
            }
            (Step::Writers, Outcome::Writers(v)) => {
                self.receipt.writes = v;
                true
            }
            _ => false,
        };
        if !valid {
            return vec![];
        }
        let mut effects = vec![self.next()];
        effects.extend(self.tick(now_ms));
        effects
    }
    pub fn tick(&mut self, now_ms: u64) -> Vec<Effect> {
        let mut effects = vec![];
        while let Some(current) = self.current {
            if current.step == Step::Terminate
                || now_ms < current.deadline_ms
                || self.termination_permitted()
            {
                break;
            }
            self.receipt.timed_out = true;
            effects.push(self.next());
        }
        effects
    }
    /// Called atomically with choosing the terminal action, before entering any
    /// host termination API. Requests after this point cannot rearm the process.
    pub fn issue(&mut self) -> Option<Action> {
        if self.state != State::ReadyToTerminate {
            return None;
        }
        self.state = State::TerminationIssued;
        Some(self.action())
    }
    pub fn exited(&mut self) {
        self.state = State::Exited;
    }
}
#[cfg(test)]
mod tests;
