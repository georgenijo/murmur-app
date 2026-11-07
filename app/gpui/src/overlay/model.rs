//! Presentation only. Capture, delivery, settings, completion and shutdown stay
//! with the host's existing owners. All clocks below are monotonic milliseconds.
use murmur_core::{dictation::Status, overlay_geometry::*};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Meeting {
    #[default]
    Idle,
    Starting,
    Recording,
    Stopping,
    Processing,
    Failed,
}
impl Meeting {
    pub fn busy(self) -> bool {
        !matches!(self, Self::Idle | Self::Failed)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Retry,
    Permission,
    ChooseInput,
    Partial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Clipboard,
    Retrying,
    Pasted,
    Nothing,
    Retry,
}
impl Delivery {
    pub fn retryable(self) -> bool {
        matches!(self, Self::Clipboard | Self::Retry)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    Idle,
    Starting,
    Slow,
    Recording,
    Recovering,
    Processing,
    Done,
    Cancelled,
    TapMissed,
    Failure(Failure),
    Calibrating,
    Meeting,
    MeetingProcessing,
    Delivery(Delivery),
}
impl Indicator {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Starting => "Connecting",
            Self::Slow => "Still connecting",
            Self::Recording => "Recording",
            Self::Recovering => "Audio recovering",
            Self::Processing => "Transcribing",
            Self::Done => "Done",
            Self::Cancelled => "Cancelled",
            Self::TapMissed => "Tap missed",
            Self::Calibrating => "Adjusting overlay position",
            Self::Meeting => "Notetaker recording",
            Self::MeetingProcessing => "Notetaker processing",
            Self::Failure(Failure::Retry) => "Microphone failed. Try again",
            Self::Failure(Failure::Permission) => "Allow microphone in Settings",
            Self::Failure(Failure::ChooseInput) => "Choose another microphone in Settings",
            Self::Failure(Failure::Partial) => "Wait for partial transcription",
            Self::Delivery(Delivery::Clipboard) => "Copied. Paste manually or try again",
            Self::Delivery(Delivery::Retrying) => "Trying delivery again",
            Self::Delivery(Delivery::Pasted) => "Pasted",
            Self::Delivery(Delivery::Nothing) => "Nothing to paste",
            Self::Delivery(Delivery::Retry) => "Try delivery again",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Collapsed,
    Opening,
    Open,
    Closing,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub notch: Option<NotchInfo>,
    pub scale_factor: f64,
    /// Logical global coordinates, top-left origin, independent of old scale.
    pub bounds: (f64, f64, f64, f64),
    pub work: (f64, f64, f64, f64),
    pub hidden_dock: Option<HiddenDock>,
    pub dock: DockLimit,
}
impl Default for Screen {
    fn default() -> Self {
        Self {
            notch: None,
            scale_factor: 1.,
            bounds: (0., 0., 1440., 900.),
            work: (0., 30., 1440., 850.),
            hidden_dock: None,
            dock: DockLimit::Unknown,
        }
    }
}
/// A committed presentation snapshot, hydrated after subscription. Revision
/// rejects reads overtaken by events. It never authorizes capture or persistence.
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub generation: u64,
    pub elapsed_ms: u64,
    pub status: Status,
    pub locked: bool,
    pub disabled: bool,
    pub auto_paste: bool,
    pub file_output: bool,
    pub hotkey_feedback: bool,
    pub placement: OverlayPlacement,
    pub offset: f64,
    pub mode: String,
    pub next_mode: Option<String>,
    pub destination: Option<String>,
    pub smart_auto: Option<String>,
    pub meeting: Meeting,
    pub processing_style: ProcessingStyle,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            generation: 0,
            elapsed_ms: 0,
            status: Status::Idle,
            locked: false,
            disabled: false,
            auto_paste: true,
            file_output: false,
            hotkey_feedback: false,
            placement: OverlayPlacement::Notch,
            offset: 0.,
            mode: "Everyday".into(),
            next_mode: None,
            destination: None,
            smart_auto: None,
            meeting: Meeting::Idle,
            processing_style: ProcessingStyle::Sweep,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProcessingStyle {
    #[default]
    Sweep,
    Flow,
    Pulse,
}
/// No Debug/Serialize: a suggestion may contain a private calendar title.
#[derive(Clone)]
pub struct Suggestion {
    pub token: String,
    pub title: String,
    pub busy: bool,
    pub failed: bool,
}
/// No transcript-bearing event belongs here. Live Words is the separate 3.3
/// preview owner; this model publishes only its lift/anchor relationship.
pub enum Event {
    Snapshot(Snapshot),
    Hydrate {
        snapshot: Snapshot,
        observed_epoch: u64,
    },
    Lifecycle {
        generation: u64,
        status: Status,
        at_ms: u64,
    },
    Cancelled(u64),
    Failure(u64, Failure),
    Delivery(u64, Delivery),
    RetryResult {
        generation: u64,
        attempt: u64,
        result: Delivery,
    },
    Stalled(u64),
    TapMissed,
    Screen(Screen),
    Dock(DockLimit),
    Visible(bool),
    Calibration(bool),
    Suggestion(Option<Suggestion>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Host routes to existing Controller with QueueGuard; it rechecks exact
    /// generation immediately before executing native capture/stop/cancel.
    Control {
        generation: u64,
        input: Control,
    },
    RetryDelivery {
        generation: u64,
        attempt: u64,
    },
    Disabled(bool),
    AutoPaste(bool),
    CycleMode,
    OpenSettings,
    AcceptSuggestion(String),
    DismissSuggestion(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Lock,
    Stop,
    Cancel,
}
impl Control {
    pub fn input(self) -> murmur_core::dictation::Input {
        use murmur_core::dictation::Input;
        match self {
            Self::Lock => Input::Lock,
            Self::Stop => Input::Toggle(None),
            Self::Cancel => Input::Escape,
        }
    }
}
#[derive(Default)]
pub struct Model {
    pub snapshot: Snapshot,
    pub screen: Screen,
    pub phase: Phase,
    pub suggestion: Option<Suggestion>,
    pub visible: bool,
    pub calibrating: bool,
    pub level: f32,
    pub started_ms: Option<u64>,
    cancelled_until: u64,
    failure: Option<(Failure, u64)>,
    miss_until: u64,
    done_until: u64,
    delivery: Option<(Delivery, u64)>,
    delivered: u64,
    slow: bool,
    hover_due: Option<u64>,
    // An explicit close owns the current pointer visit, even when a delayed
    // first enter arrives after the click. Only a real exit rearms hover.
    hover_closed: bool,
    leave_due: Option<u64>,
    shrink_due: Option<u64>,
    click: Option<(u64, u64)>,
    pub retry_attempt: u64,
    event_epoch: u64,
}
impl Model {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            started_ms: (snapshot.status == Status::Recording).then_some(0),
            snapshot,
            visible: true,
            ..Self::default()
        }
    }
    pub fn hydration_epoch(&self) -> u64 {
        self.event_epoch
    }
    pub fn event(&mut self, event: Event, now: u64) {
        if !matches!(
            &event,
            Event::Snapshot(_) | Event::Hydrate { .. } | Event::Dock(_) | Event::Screen(_)
        ) {
            self.event_epoch = self.event_epoch.saturating_add(1);
        }
        match event {
            Event::Hydrate {
                snapshot,
                observed_epoch,
            } if observed_epoch == self.event_epoch => self.event(Event::Snapshot(snapshot), now),
            Event::Snapshot(next)
                if next.revision > self.snapshot.revision
                    && next.generation >= self.snapshot.generation =>
            {
                let changed = next.generation != self.snapshot.generation
                    || next.status != self.snapshot.status;
                if changed {
                    self.lifecycle(next.generation, next.status, now);
                }
                let reset = next.placement != self.snapshot.placement;
                self.snapshot = next;
                // elapsed_ms is authoritative at receipt, including refreshes
                // within the same recording. Do not count the old local delta
                // a second time on top of it.
                self.started_ms = (self.snapshot.status == Status::Recording).then_some(now);
                if reset {
                    self.collapse();
                }
            }
            Event::Lifecycle {
                generation,
                status,
                at_ms,
            } => self.lifecycle(generation, status, at_ms),
            Event::Cancelled(g) if g == self.snapshot.generation => {
                self.cancelled_until = now + 800;
                self.delivery = None;
                self.retry_attempt += 1;
            }
            Event::Failure(g, f) if g == self.snapshot.generation => {
                self.failure = Some((f, now + 5000))
            }
            Event::Delivery(g, d) if g == self.snapshot.generation && g > self.delivered => {
                self.delivered = g;
                self.retry_attempt += 1;
                self.delivery = (d == Delivery::Clipboard).then_some((d, now + 5000));
            }
            Event::RetryResult {
                generation,
                attempt,
                result,
            } if generation == self.snapshot.generation
                && attempt == self.retry_attempt
                && self.delivery.is_some_and(|(d, _)| d == Delivery::Retrying) =>
            {
                self.delivery = Some((result, now + 5000))
            }
            Event::Stalled(g)
                if g == self.snapshot.generation && self.snapshot.status == Status::Starting =>
            {
                self.slow = true
            }
            Event::TapMissed
                if self.snapshot.hotkey_feedback && self.snapshot.status == Status::Idle =>
            {
                self.miss_until = now + 500
            }
            Event::Dock(limit) => self.screen.dock = limit,
            Event::Screen(screen) => {
                let topology = screen.notch != self.screen.notch
                    || screen.bounds != self.screen.bounds
                    || screen.work != self.screen.work
                    || screen.scale_factor != self.screen.scale_factor;
                self.screen = screen;
                if topology {
                    self.collapse();
                }
            }
            Event::Visible(v) => {
                self.visible = v;
                if !v {
                    self.collapse();
                }
            }
            Event::Calibration(v) => self.calibrating = v,
            Event::Suggestion(s) => self.suggestion = s,
            _ => {}
        }
        if self.snapshot.status != Status::Idle
            || self.snapshot.meeting.busy()
            || self.snapshot.disabled
            || self.calibrating
        {
            self.suggestion = None;
        }
    }
    fn lifecycle(&mut self, g: u64, status: Status, at: u64) {
        if g < self.snapshot.generation {
            return;
        }
        let newer = g > self.snapshot.generation;
        if newer || (status != Status::Idle && status != self.snapshot.status) {
            self.delivery = None;
            self.failure = None;
            self.done_until = 0;
            self.miss_until = 0;
            self.retry_attempt += 1;
        }
        if newer {
            self.cancelled_until = 0;
            self.click = None;
        }
        if status == Status::Idle && self.snapshot.status == Status::Processing {
            self.done_until = at + 400;
        }
        if status == Status::Recording && (newer || self.snapshot.status != status) {
            self.started_ms = Some(at);
            self.snapshot.elapsed_ms = 0;
        }
        if status != Status::Recording {
            self.started_ms = None;
            self.level = 0.;
        }
        if status != Status::Starting {
            self.slow = false;
        }
        self.snapshot.generation = g;
        self.snapshot.status = status;
    }
    pub fn indicator(&self, now: u64) -> Indicator {
        if self.calibrating {
            Indicator::Calibrating
        } else if self.cancelled_until > now {
            Indicator::Cancelled
        } else if let Some((f, until)) = self.failure.filter(|(_, u)| *u > now) {
            let _ = until;
            Indicator::Failure(f)
        } else if self.miss_until > now {
            Indicator::TapMissed
        } else if self.snapshot.meeting.busy() {
            if self.snapshot.meeting == Meeting::Processing {
                Indicator::MeetingProcessing
            } else {
                Indicator::Meeting
            }
        } else {
            match self.snapshot.status {
                Status::Starting => {
                    if self.slow {
                        Indicator::Slow
                    } else {
                        Indicator::Starting
                    }
                }
                Status::Recording => Indicator::Recording,
                Status::Recovering => Indicator::Recovering,
                Status::Processing => Indicator::Processing,
                Status::Idle => {
                    if let Some((d, _)) = self.delivery.filter(|(_, u)| *u > now) {
                        Indicator::Delivery(d)
                    } else if self.done_until > now {
                        Indicator::Done
                    } else {
                        Indicator::Idle
                    }
                }
            }
        }
    }
    pub fn hidden(&self, now: u64) -> bool {
        !self.visible
            || (self.snapshot.placement == OverlayPlacement::FloatingPill
                && self.indicator(now) == Indicator::Idle
                && self.suggestion.is_none())
    }
    pub fn waveform(&self, now: u64) -> bool {
        self.visible && self.indicator(now) == Indicator::Recording
    }
    pub fn sample(&mut self, g: u64, level: f32, now: u64) {
        if g == self.snapshot.generation && self.waveform(now) {
            self.level = if level.is_finite() {
                level.clamp(0., 1.)
            } else {
                0.
            };
        }
    }
    pub fn hover(&mut self, inside: bool, now: u64) {
        if inside {
            self.leave_due = None;
            if !self.hover_closed
                && !self.hidden(now)
                && matches!(self.phase, Phase::Collapsed | Phase::Closing)
            {
                self.hover_due.get_or_insert(now + 150);
            }
        } else {
            self.hover_closed = false;
            self.hover_due = None;
            if self.phase == Phase::Opening {
                self.collapse();
            }
            if self.phase == Phase::Open {
                self.leave_due.get_or_insert(now + 300);
            }
        }
    }
    pub(super) fn hover_suppressed(&self) -> bool {
        self.hover_closed
    }
    /// An outside sample can end an explicit-close visit even if the panel
    /// never received its enter. It must not manufacture an AX-opened exit.
    pub(super) fn rearm_hover(&mut self) {
        self.hover_closed = false;
    }
    pub fn needs_tick(&self) -> bool {
        self.hover_due.is_some()
            || self.leave_due.is_some()
            || self.shrink_due.is_some()
            || self.click.is_some()
    }
    /// Keyboard/AX disclosure uses the same acknowledged surface transaction
    /// as hover, without requiring pointer movement or a dwell.
    pub fn expand(&mut self, now: u64) {
        if !self.hidden(now) {
            self.hover_closed = false;
            self.leave_due = None;
            if matches!(self.phase, Phase::Collapsed | Phase::Closing) {
                self.hover_due = Some(now);
            }
        }
    }
    pub fn close(&mut self, now: u64, pointer_inside: bool) {
        self.hover_closed = pointer_inside;
        self.hover_due = None;
        if self.phase == Phase::Open && self.suggestion.is_none() {
            self.leave_due = Some(now);
        }
    }
    pub fn collapse(&mut self) {
        self.phase = Phase::Collapsed;
        self.hover_closed = false;
        self.hover_due = None;
        self.leave_due = None;
        self.shrink_due = None;
    }
    /// The sole surface writer calls this on main. Reveal follows a successful
    /// frame/material transaction; a rejected grow never exposes controls.
    pub fn tick(
        &mut self,
        now: u64,
        reduced: bool,
        mut apply: impl FnMut(OverlayFrame, bool) -> bool,
    ) -> Option<Intent> {
        if self.hover_due.is_some_and(|u| now >= u)
            || (self.suggestion.is_some() && self.phase == Phase::Collapsed && !self.hidden(now))
        {
            self.hover_due = None;
            self.phase = Phase::Opening;
            if apply(self.frame(true, now), false) {
                self.phase = Phase::Open;
                self.shrink_due = None;
            } else {
                self.collapse();
            }
        }
        if self.leave_due.is_some_and(|u| now >= u) && self.suggestion.is_none() {
            self.leave_due = None;
            self.phase = Phase::Closing;
            self.shrink_due = Some(now + if reduced { 0 } else { 380 });
        }
        if self.shrink_due.is_some_and(|u| now >= u) {
            if apply(self.frame(false, now), self.hidden(now)) {
                // A pointer can re-enter the persistent top band before the
                // shrink commits. Its dwell still owns the next grow, even if
                // no further enter event arrives after shrinking.
                self.phase = Phase::Collapsed;
                self.shrink_due = None;
            } else {
                self.shrink_due = Some(now + 100);
            }
        }
        if let Some((due, g)) = self.click.filter(|(u, _)| now >= *u) {
            let _ = due;
            self.click = None;
            if g == self.snapshot.generation
                && !self.calibrating
                && !self.snapshot.meeting.busy()
                && self.suggestion.is_none()
            {
                let input = match self.snapshot.status {
                    Status::Starting => Some(Control::Cancel),
                    Status::Recording => Some(Control::Stop),
                    _ => None,
                };
                return input.map(|input| Intent::Control {
                    generation: g,
                    input,
                });
            }
        }
        None
    }
    /// Assistive activation is one deliberate action, not a timed double click.
    pub fn accessible_activate(&mut self, now: u64) -> Option<Intent> {
        if self.calibrating || self.snapshot.meeting.busy() || self.suggestion.is_some() {
            return None;
        }
        match self.snapshot.status {
            Status::Idle => self.click(2, now),
            Status::Recording => Some(Intent::Control {
                generation: self.snapshot.generation,
                input: Control::Stop,
            }),
            Status::Starting => Some(Intent::Control {
                generation: self.snapshot.generation,
                input: Control::Cancel,
            }),
            _ => None,
        }
    }
    pub fn click(&mut self, count: usize, now: u64) -> Option<Intent> {
        if self.calibrating || self.snapshot.meeting.busy() || self.suggestion.is_some() {
            return None;
        }
        if count >= 2 {
            self.click = None;
            if !matches!(self.phase, Phase::Collapsed | Phase::Closing)
                || matches!(
                    self.snapshot.status,
                    Status::Processing | Status::Recovering
                )
                || (self.snapshot.disabled && self.snapshot.status == Status::Idle)
            {
                return None;
            }
            Some(Intent::Control {
                generation: self.snapshot.generation,
                input: Control::Lock,
            })
        } else {
            self.click = Some((now + 250, self.snapshot.generation));
            None
        }
    }
    pub fn suggestion_action(&mut self, token: &str, accept: bool) -> Option<Intent> {
        if self.snapshot.status != Status::Idle
            || self.snapshot.meeting.busy()
            || self.snapshot.disabled
        {
            return None;
        }
        let suggestion = self.suggestion.as_mut()?;
        if suggestion.token != token || suggestion.busy {
            return None;
        }
        suggestion.busy = true;
        Some(if accept {
            Intent::AcceptSuggestion(token.into())
        } else {
            Intent::DismissSuggestion(token.into())
        })
    }
    pub fn pause_delivery(&mut self, now: u64) {
        if let Some((d, until)) = self.delivery.as_mut()
            && d.retryable()
            && *until > now
        {
            *until = u64::MAX;
        }
    }
    pub fn resume_delivery(&mut self, now: u64) {
        if let Some((d, until)) = self.delivery.as_mut()
            && d.retryable()
            && *until == u64::MAX
        {
            *until = now + 5000;
        }
    }
    pub fn retry(&mut self, now: u64) -> Option<Intent> {
        if self.snapshot.status != Status::Idle
            || !self.delivery.is_some_and(|(d, u)| d.retryable() && u > now)
        {
            return None;
        }
        self.retry_attempt += 1;
        self.delivery = Some((Delivery::Retrying, u64::MAX));
        Some(Intent::RetryDelivery {
            generation: self.snapshot.generation,
            attempt: self.retry_attempt,
        })
    }
    pub fn geometry(&self, expanded: bool, now: u64) -> OverlayGeometry {
        geometry_for_layout(
            self.screen.notch,
            if self.suggestion.is_some() {
                OverlayContent::MeetingSuggestion
            } else {
                OverlayContent::Controls
            },
            OverlayLayout {
                activity: if self.indicator(now) == Indicator::Idle {
                    OverlayActivity::Idle
                } else {
                    OverlayActivity::Processing
                },
                placement: self.snapshot.placement,
                expanded,
            },
        )
    }
    pub fn frame(&self, expanded: bool, now: u64) -> OverlayFrame {
        let g = self.geometry(expanded, now);
        let h = if expanded {
            g.expanded_h
        } else {
            g.collapsed_h
        };
        let (left, top, right, bottom) = self.screen.bounds;
        let (x, y) = if g.floating {
            let (wl, wt, wr, wb) = self.screen.work;
            pill_logical_position(
                (
                    wl,
                    wt,
                    wr,
                    pill_work_bottom(bottom, wb, self.screen.hidden_dock, self.screen.dock),
                ),
                (g.window_w, h),
            )
        } else {
            let offset = clamp_vertical_offset(self.snapshot.offset);
            let y = top
                + if self.screen.notch.is_some_and(|n| n.physical_notch) {
                    offset
                } else {
                    offset.max(0.)
                };
            ((left + right - g.window_w) / 2., y.min(bottom - h))
        };
        OverlayFrame {
            x,
            y,
            width: g.window_w,
            height: h,
        }
    }
    pub fn elapsed(&self, now: u64) -> String {
        let seconds = self.started_ms.map_or(0, |t| {
            (self.snapshot.elapsed_ms + now.saturating_sub(t)) / 1000
        });
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
    pub fn bars(&self, count: usize) -> Vec<f32> {
        if self.snapshot.placement == OverlayPlacement::FloatingPill {
            // React's VoiceIslandFixture freezes this per-bar profile. Apply it
            // after the live gain saturates, so quiet speech still moves the
            // bars and loud speech keeps the rhythm.
            const PROFILE: [f32; 18] = [
                0.3, 0.55, 0.8, 1., 0.7, 0.45, 0.6, 0.9, 0.5, 0.35, 0.75, 0.95, 0.65, 0.4, 0.85,
                0.55, 0.3, 0.2,
            ];
            let level = (self.level * 16.).clamp(0., 1.);
            return (0..count)
                .map(|i| (level * PROFILE[i % PROFILE.len()]).clamp(0.14, 1.))
                .collect();
        }
        (0..count)
            .map(|i| {
                let center = (count - 1) as f32 / 2.;
                let envelope = 1. - 0.5 * (i as f32 - center).abs() / center.max(1.);
                let level = (self.level * 16.).clamp(0., 1.);
                let variation = ((i * 17 + 11) % 23) as f32 / 23.;
                (0.08 + 0.07 * variation + level * envelope + level * level * 0.4 * variation)
                    .clamp(0.14, 1.)
            })
            .collect()
    }
}

/// Reconcile delivered tracking events with watchdog samples. Only transitions
/// generate hover intent: repeated samples cannot postpone leave, restart a
/// cancelled dwell, or close an AX-opened panel before a pointer has entered it.
#[derive(Default)]
pub struct PointerPresence {
    inside: bool,
}
impl PointerPresence {
    pub fn inside(&self) -> bool {
        self.inside
    }
    pub fn observe(&mut self, inside: bool) -> bool {
        let changed = self.inside != inside;
        self.inside = inside;
        changed
    }
}

/// Retain only the previous visual's native bounds during a contraction; a
/// later exact-size transaction releases the temporary transparent hit area.
/// Main-thread synchronous writes cannot produce late IPC acknowledgements.
#[derive(Default)]
pub struct SurfaceWriter {
    pending: Option<(u64, OverlayFrame)>,
}
impl SurfaceWriter {
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn resolve(
        &mut self,
        target: OverlayFrame,
        previous: Option<OverlayFrame>,
        now: u64,
        floating: bool,
        animate: bool,
    ) -> OverlayFrame {
        let previous = previous.filter(|p| {
            (p.x + p.width / 2. - target.x - target.width / 2.).abs() < 0.01
                && (if floating {
                    p.y + p.height - target.y - target.height
                } else {
                    p.y - target.y
                })
                .abs()
                    < 0.01
                && (p.width > target.width || p.height > target.height)
        });
        let Some(previous) = previous.filter(|_| animate) else {
            self.pending = None;
            return target;
        };
        if !self.pending.is_some_and(|(_, t)| t == target) {
            self.pending = Some((now + 380, target));
        }
        if self.pending.is_some_and(|(until, _)| now >= until) {
            self.pending = None;
            return target;
        }
        let size = animation_surface(
            AppliedSurface {
                window_w: target.width,
                window_h: target.height,
            },
            Some(previous),
            true,
        );
        OverlayFrame {
            x: target.x + (target.width - size.window_w) / 2.,
            y: if floating {
                target.y + target.height - size.window_h
            } else {
                target.y
            },
            width: size.window_w,
            height: size.window_h,
        }
    }
}
