//! Native onboarding in Root. Permission/model/configuration work belongs to Host.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent, Choice, ChoiceKind, Choices, SelectionChanged,
    data_components::Progress,
    tokens::{self, Palette},
    workspace::Authority,
};
use gpui::{prelude::*, *};
use host::{Action, Host, Request};
use inbox::Inbox;
use model::*;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

pub struct Onboarding {
    pub model: Session<Snapshot>,
    pub step: Step,
    pub finished: bool,
    pub replay: bool,
    host: Rc<dyn Host>,
    inbox: Inbox,
    authority: Authority,
    subscription: Option<Box<dyn host::Subscription>>,
    subscriptions: Vec<gpui::Subscription>,
    pub selected_model: String,
    pub recording_mode: String,
    pub trigger_key: String,
    modes: Entity<Choices>,
    keys: Entity<Choices>,
    heading: FocusHandle,
    primary: FocusHandle,
    focused_step: Option<Step>,
    scroll: ScrollHandle,
    operation: Option<Action>,
    visit_started: Instant,
    last_poll: Instant,
    baseline: bool,
    fresh: Option<Instant>,
    mic_auto_consumed: bool,
    mic_wait: Option<Instant>,
    ax_requested: bool,
    seeded: bool,
    standalone: bool,
    verified_model: Option<String>,
    cancelling: bool,
}
impl Onboarding {
    pub fn new(host: Rc<dyn Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let scroll = ScrollHandle::new();
        let modes = cx.new(|cx| {
            let mut choices = Choices::new(
                "Recording trigger",
                ChoiceKind::Dropdown,
                [
                    ("hold_down", "Hold down"),
                    ("double_tap", "Double-tap"),
                    ("both", "Both"),
                ]
                .into_iter()
                .map(|(v, l)| Choice::new(v, l))
                .collect(),
                w,
                cx,
            );
            choices.scroll_parent = Some(scroll.clone());
            choices
        });
        let options = crate::settings::schema::ROWS
            .iter()
            .find(|r| r.key == "doubleTapKey")
            .and_then(|r| {
                if let crate::settings::schema::Kind::Choice(o) = r.kind {
                    Some(o)
                } else {
                    None
                }
            })
            .unwrap();
        let keys = cx.new(|cx| {
            let mut choices = Choices::new(
                "Trigger key",
                ChoiceKind::Dropdown,
                options.iter().map(|(v, l)| Choice::new(*v, *l)).collect(),
                w,
                cx,
            );
            choices.scroll_parent = Some(scroll.clone());
            choices
        });
        let subscriptions = vec![
            cx.observe_window_activation(w, |s, w, cx| {
                if w.is_window_active() && s.model.attached {
                    s.refresh(cx);
                }
            }),
            cx.subscribe(&modes, |s, _, e: &SelectionChanged, cx| {
                s.recording_mode = e.0.to_string();
                cx.notify();
            }),
            cx.subscribe(&keys, |s, _, e: &SelectionChanged, cx| {
                s.trigger_key = e.0.to_string();
                cx.notify();
            }),
        ];
        Self {
            model: Session::default(),
            step: Step::Welcome,
            finished: false,
            replay: false,
            host,
            inbox: Inbox::default(),
            authority: Authority::default(),
            subscription: None,
            subscriptions,
            selected_model: String::new(),
            recording_mode: String::new(),
            trigger_key: String::new(),
            modes,
            keys,
            heading: cx.focus_handle().tab_stop(false),
            primary: cx.focus_handle(),
            focused_step: None,
            scroll,
            operation: None,
            visit_started: Instant::now(),
            last_poll: Instant::now(),
            baseline: false,
            fresh: None,
            mic_auto_consumed: false,
            mic_wait: None,
            ax_requested: false,
            seeded: false,
            standalone: false,
            verified_model: None,
            cancelling: false,
        }
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.model.attached {
            return;
        }
        self.model.attach();
        self.inbox = Inbox::default();
        self.authority.sync(&self.model);
        self.subscription = Some(self.host.subscribe(
            murmur_core::events::Consumer::Main,
            self.model.attachment,
            self.inbox.clone(),
        ));
        self.request(Action::Read, cx);
    }
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        self.detach();
        cx.notify();
    }
    fn detach(&mut self) {
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut s) = self.subscription.take() {
            s.stop();
        }
        if self.model.attached {
            self.host.detach(self.model.attachment);
        }
        self.model.detach();
        self.operation = None;
        self.cancelling = false;
        self.selected_model.clear();
        self.recording_mode.clear();
        self.trigger_key.clear();
        self.seeded = false;
        self.fresh = None;
        self.mic_wait = None;
        self.mic_auto_consumed = false;
        self.finished = false;
        self.ax_requested = false;
        self.standalone = false;
        self.verified_model = None;
        self.step = Step::Welcome;
        self.focused_step = None;
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        let attachment = self.model.attachment;
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(32))
                    .await;
                if !weak
                    .update(cx, |s, cx| {
                        if !s.model.attached
                            || s.model.attachment != attachment
                            || s.inbox.is_closed()
                        {
                            return false;
                        }
                        s.drain(cx);
                        s.tick(Instant::now(), cx);
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub fn drain(&mut self, cx: &mut Context<Self>) {
        for event in self.inbox.drain() {
            let terminal = matches!(&event,Event::Reply{ticket,..} if *ticket==self.model.ticket());
            let ok = matches!(&event, Event::Reply { result: Ok(_), .. });
            let before = self.access();
            let action = self.operation.clone();
            if !self.model.event(event) {
                continue;
            }
            self.authority.sync(&self.model);
            if !terminal && !self.model.pending && self.downloading() {
                // A source revision revoked the download lease. Keep navigation
                // locked while the attachment's exact native owner is cancelled.
                self.cancelling = false;
                self.cancel_download(cx);
            }
            if terminal && self.cancelling && !ok {
                // A failed cancellation is not a termination receipt. Keep the
                // download owner/Back lock and offer another explicit Cancel.
                self.cancelling = false;
                cx.notify();
                continue;
            }
            if terminal {
                self.operation = None;
                if ok {
                    match action {
                        Some(Action::Complete { .. } | Action::ResumeExisting) => {
                            self.finished = true
                        }
                        Some(Action::Download(id)) if !self.cancelling => {
                            if self
                                .model
                                .snapshot
                                .as_ref()
                                .is_some_and(|s| s.installed(&id))
                            {
                                self.selected_model = id;
                                self.advance(cx);
                            }
                        }
                        Some(Action::Request(Permission::Microphone)) => {
                            if self.access() == Access::NotDetermined {
                                self.mic_wait = Some(Instant::now());
                            }
                        }
                        Some(Action::Reset(Permission::Microphone)) => self.mic_wait = None,
                        Some(Action::Catalog) => {
                            if let Some(snapshot) = &self.model.snapshot {
                                self.selected_model = self
                                    .verified_model
                                    .clone()
                                    .unwrap_or_else(|| snapshot.preferred());
                            }
                        }
                        _ => (),
                    }
                }
                self.cancelling = false;
            }
            if !self.seeded && self.model.snapshot.is_some() {
                let snapshot = self.model.snapshot.as_ref().unwrap();
                self.selected_model = snapshot.preferred();
                self.recording_mode = snapshot.settings["recordingMode"]
                    .as_str()
                    .unwrap_or("hold_down")
                    .into();
                self.trigger_key = snapshot.settings["doubleTapKey"]
                    .as_str()
                    .unwrap_or("shift_l")
                    .into();
                self.seeded = true;
                if !self.replay && (snapshot.complete || snapshot.grandfathered()) {
                    if !snapshot.installed(snapshot.settings["model"].as_str().unwrap_or("")) {
                        let grandfather = !snapshot.complete;
                        self.standalone = true;
                        self.step = Step::Model;
                        if grandfather {
                            self.request(Action::Grandfather, cx);
                        }
                    } else {
                        self.request(Action::ResumeExisting, cx);
                    }
                }
            }
            if self.step.permission().is_some() {
                let now = self.access();
                if now != Access::NotDetermined {
                    self.mic_wait = None;
                }
                if self.baseline && before != Access::Granted && now == Access::Granted {
                    self.fresh = Some(Instant::now());
                }
                if now != Access::Granted {
                    self.fresh = None;
                }
                self.baseline = true;
            }
            cx.notify();
        }
    }
    pub fn request(&mut self, action: Action, cx: &mut Context<Self>) {
        // The retained download owns cancellation until its terminal receipt.
        if self.downloading() {
            return;
        }
        let Some(ticket) = self.model.begin() else {
            return;
        };
        self.authority.sync(&self.model);
        self.operation = Some(action.clone());
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                action,
            },
            self.inbox.clone(),
        );
        cx.notify();
    }
    pub fn visit(&mut self, step: Step, cx: &mut Context<Self>) {
        if self.downloading() || matches!(self.operation, Some(Action::Complete { .. })) {
            return;
        }
        if self.step == Step::Microphone {
            self.mic_auto_consumed = true;
            self.mic_wait = None;
        }
        self.model.invalidate();
        self.authority.sync(&self.model);
        self.operation = None;
        self.step = step;
        self.baseline = false;
        self.fresh = None;
        self.visit_started = Instant::now();
        self.last_poll = Instant::now();
        self.scroll.set_offset(point(px(0.), px(0.)));
        if let Some(p) = step.permission() {
            self.request(Action::Probe(p), cx);
        }
        if step == Step::Model {
            self.request(Action::Catalog, cx);
        }
        cx.notify();
    }
    fn advance(&mut self, cx: &mut Context<Self>) {
        if self.step == Step::Model {
            self.verified_model = Some(self.selected_model.clone());
        }
        if self.standalone {
            self.finish(cx);
        } else {
            self.visit(Step::ALL[(self.step.index() + 1).min(7)], cx);
        }
    }
    fn finish(&mut self, cx: &mut Context<Self>) {
        self.request(
            Action::Complete {
                model: self.selected_model.clone(),
                recording_mode: self.recording_mode.clone(),
                trigger_key: self.trigger_key.clone(),
            },
            cx,
        );
    }
    fn access(&self) -> Access {
        self.step
            .permission()
            .and_then(|p| {
                self.model
                    .snapshot
                    .as_ref()
                    .map(|s| s.permissions[p.index()])
            })
            .unwrap_or(Access::Unknown)
    }
    fn downloading(&self) -> bool {
        matches!(self.operation, Some(Action::Download(_)))
    }
    pub fn tick(&mut self, now: Instant, cx: &mut Context<Self>) {
        if self.finished || !self.model.attached {
            return;
        }
        if self
            .mic_wait
            .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_secs(15))
        {
            self.mic_wait = None;
            cx.notify();
        }
        if self
            .fresh
            .is_some_and(|t| now.saturating_duration_since(t) >= Duration::from_millis(900))
            && !self.model.pending
        {
            self.advance(cx);
            return;
        }
        if self.step == Step::Microphone
            && self.access() == Access::NotDetermined
            && !self.mic_auto_consumed
            && now.saturating_duration_since(self.visit_started) >= Duration::from_millis(600)
            && !self.model.pending
        {
            self.mic_auto_consumed = true;
            self.request(Action::Request(Permission::Microphone), cx);
            return;
        }
        if self.step.permission().is_some()
            && now.saturating_duration_since(self.last_poll) >= Duration::from_secs(1)
            && !self.model.pending
        {
            self.last_poll = now;
            self.request(Action::Probe(self.step.permission().unwrap()), cx);
        }
    }
    /// Host forwards native window focus here; optional probes are visit-scoped.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.step.permission() {
            self.request(Action::Probe(p), cx);
        }
    }
    fn primary_label(&self) -> String {
        if self.finished {
            return "Setup complete".into();
        }
        if self.step == Step::Welcome {
            return "Get started".into();
        }
        if self.step == Step::Done {
            return if self.model.pending {
                "Configuring model…"
            } else if self.model.problem.is_some() {
                "Retry"
            } else {
                "Start using Murmur"
            }
            .into();
        }
        if self.step == Step::Model {
            let Some(s) = self.model.snapshot.as_ref() else {
                return "Loading…".into();
            };
            if s.catalog_error {
                return "Retry".into();
            }
            if s.catalog.is_none() {
                return "Loading…".into();
            }
            if self.downloading() {
                return if s.installing {
                    "Installing…"
                } else {
                    "Downloading…"
                }
                .into();
            }
            return if s.installed(&self.selected_model) {
                "Continue"
            } else {
                "Download model"
            }
            .into();
        }
        if self.step.permission().is_some() && self.access() == Access::Denied
            || self.step == Step::Accessibility && self.access() != Access::Granted
        {
            return "Open System Settings…".into();
        }
        "Continue".into()
    }
    fn activate(&mut self, cx: &mut Context<Self>) {
        if self.model.pending
            || self.downloading()
            || self.mic_wait.is_some()
            || self.model.snapshot.is_none()
            || self.finished
        {
            return;
        }
        if self.step == Step::Done {
            self.finish(cx);
            return;
        }
        if self.step == Step::Model {
            let Some(s) = self.model.snapshot.as_ref() else {
                return;
            };
            if s.catalog_error || s.catalog.is_none() {
                self.request(Action::Catalog, cx);
            } else if s.installed(&self.selected_model) {
                self.advance(cx);
            } else {
                self.request(Action::Download(self.selected_model.clone()), cx);
            }
            return;
        }
        if let Some(p) = self.step.permission() {
            match self.access() {
                Access::Granted | Access::Restricted | Access::Unsupported => self.advance(cx),
                Access::Denied => self.request(Action::OpenSettings(p), cx),
                _ => {
                    if p == Permission::Accessibility {
                        self.ax_requested = true;
                    }
                    if p == Permission::Microphone {
                        self.mic_auto_consumed = true;
                    }
                    self.request(Action::Request(p), cx);
                }
            }
        } else {
            self.advance(cx);
        }
    }
    #[cfg(feature = "gallery")]
    pub fn fixture_download(&mut self, cx: &mut Context<Self>) {
        let _ = self.model.begin();
        self.operation = Some(Action::Download(self.selected_model.clone()));
        if let Some(snapshot) = self.model.snapshot.as_mut() {
            snapshot.download_percent = Some(42);
        }
        cx.notify();
    }
    /// Enter the real cancellation path without native UI input in QA.
    #[cfg(feature = "gallery")]
    pub fn fixture_cancel_download(&mut self, cx: &mut Context<Self>) {
        self.cancel_download(cx);
    }
    fn cancel_download(&mut self, cx: &mut Context<Self>) {
        let Some(Action::Download(id)) = self.operation.clone() else {
            return;
        };
        if !self.model.attached || self.cancelling {
            return;
        }
        self.cancelling = true;
        if !self.model.pending {
            // Cancelling is still part of the failed download. Keep its
            // explanation and disabled fallback choices until the terminal
            // receipt, rather than hiding them as if a retry had begun.
            let problem = self.model.problem;
            if self.model.begin().is_none() {
                self.cancelling = false;
                return;
            }
            self.model.problem = problem;
            self.authority.sync(&self.model);
        }
        let ticket = self.model.ticket();
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                action: Action::CancelDownload(id),
            },
            self.inbox.clone(),
        );
        cx.notify();
    }
    fn body_button(&self, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        Button::new(id, label).with_scroll_parent(Some(&self.scroll))
    }
    fn select_model(&mut self, id: String, cx: &mut Context<Self>) {
        if self.downloading() {
            return;
        }
        self.selected_model = id;
        cx.notify();
    }
    fn default_navigation(&self) -> bool {
        match self.step {
            Step::Welcome | Step::Hotkey | Step::Done => true,
            Step::Model => self
                .model
                .snapshot
                .as_ref()
                .is_some_and(|s| s.installed(&self.selected_model)),
            _ => matches!(
                self.access(),
                Access::Granted | Access::Restricted | Access::Unsupported
            ),
        }
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.default_navigation()
            && !e.is_held
            && !e.prefer_character_input
            && e.keystroke.key == "enter"
            && e.keystroke.modifiers.number_of_modifiers() == 0
            && self.heading.is_focused(w)
        {
            self.activate(cx);
            w.prevent_default();
            cx.stop_propagation();
        }
    }
}
impl Drop for Onboarding {
    fn drop(&mut self) {
        self.detach();
    }
}
impl Render for Onboarding {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain(cx);
        if self.focused_step != Some(self.step) {
            w.focus(&self.heading, cx);
            self.focused_step = Some(self.step);
        }
        let p = Palette::get(cx);
        let _ = &self.subscriptions;
        if self.modes.read(cx).value().map(|v| v.as_ref()) != Some(self.recording_mode.as_str()) {
            self.modes
                .update(cx, |s, cx| s.set_value(&self.recording_mode, cx));
        }
        if self.keys.read(cx).value().map(|v| v.as_ref()) != Some(self.trigger_key.as_str()) {
            self.keys
                .update(cx, |s, cx| s.set_value(&self.trigger_key, cx));
        }
        let title = if self.standalone {
            "Download a Model"
        } else {
            self.step.title()
        };
        let mut body = div()
            .id("onboarding-content")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .size(px(64.))
                        .rounded(px(16.))
                        .bg(p.accent.opacity(0.12))
                        .text_color(p.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            crate::icons::icon(match self.step {
                                Step::Microphone => crate::icons::Glyph::Mic,
                                Step::Calendar => crate::icons::Glyph::Calendar,
                                Step::Model => crate::icons::Glyph::Download,
                                Step::Done => crate::icons::Glyph::Check,
                                _ => crate::icons::Glyph::Settings,
                            })
                            .size(px(28.)),
                        ),
                ),
            )
            .child(
                div()
                    .id("setup-heading")
                    .text_center()
                    .role(Role::Heading)
                    .aria_label(title)
                    .track_focus(&self.heading)
                    .relative()
                    .child(crate::data_components::focus_reveal(
                        self.heading.clone(),
                        Some(self.scroll.clone()),
                        None,
                        w,
                        cx,
                    ))
                    .text_size(px(24.))
                    .line_height(px(30.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_center()
                    .text_color(p.muted)
                    .max_w(px(420.))
                    .self_center()
                    .child(text!(self.step.subtitle())),
            );
        if self.model.snapshot.is_none() {
            body = body.child(text!("Loading setup…"));
        }
        if let Some(problem) = self.model.problem {
            let message = if self.step == Step::Done {
                "Could not verify or configure the selected transcription model. Your setup choices are kept. Retry, or go Back to review the model."
            } else {
                problem.message()
            };
            body = body.child(
                div()
                    .id("setup-error")
                    .role(Role::Alert)
                    .aria_label(message)
                    .text_color(p.danger)
                    .child(message),
            );
        }
        if self.model.snapshot.is_none() && self.model.problem.is_some() {
            let e = cx.entity();
            body = body.child(
                self.body_button("retry-setup", "Retry")
                    .disabled(self.model.pending)
                    .on_click(move |_, _, cx| e.update(cx, |s, cx| s.request(Action::Read, cx))),
            );
        }
        if let Some(permission) = self.step.permission() {
            let access = self.access();
            let waiting = self.model.pending || self.mic_wait.is_some();
            let text = if access == Access::Granted {
                "Allowed"
            } else if waiting {
                "Waiting for macOS…"
            } else {
                access.label()
            };
            body = body.child(
                div()
                    .id("permission-status")
                    .role(Role::Status)
                    .aria_label(format!("{} access {text}", permission.label()))
                    .px(px(16.))
                    .py(px(12.))
                    .border_1()
                    .border_color(p.hairline_strong)
                    .bg(p.tint_raised)
                    .shadow(p.card_shadow())
                    .rounded(tokens::CARD_RADIUS)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(crate::icons::icon(
                                if permission == Permission::Microphone {
                                    crate::icons::Glyph::Mic
                                } else {
                                    crate::icons::Glyph::Settings
                                },
                            ))
                            .child(permission.label()),
                    )
                    .child(
                        div()
                            .rounded_full()
                            .px(px(10.))
                            .py(px(3.))
                            .text_size(tokens::CAPTION_SIZE)
                            .text_color(if access == Access::Denied {
                                p.danger
                            } else {
                                p.muted
                            })
                            .bg(if access == Access::Denied {
                                p.surface.blend(p.danger.opacity(0.10))
                            } else {
                                p.surface_container_low
                            })
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .when(access == Access::Denied, |d| {
                                d.child(
                                    crate::icons::icon(crate::icons::Glyph::Alert).size(px(12.)),
                                )
                            })
                            .child(text),
                    ),
            );
            let detail = match permission {
                Permission::Microphone if self.mic_wait.is_some() => {
                    "Waiting for your answer in the macOS dialog…"
                }
                Permission::Microphone if access == Access::Denied => {
                    "Enable Murmur under Privacy & Security → Microphone, then come back. This screen updates automatically."
                }
                Permission::Accessibility if access != Access::Granted => {
                    "Turn on Murmur in System Settings, then come back. This screen updates automatically."
                }
                Permission::SystemAudio if access == Access::Unsupported => {
                    "Meeting capture requires macOS 14.2 or newer. Dictation is still available."
                }
                Permission::SystemAudio if access == Access::Unknown => {
                    "Continue shows the macOS permission prompt."
                }
                Permission::SystemAudio => {
                    "macOS may list Murmur under Privacy & Security → Screen & System Audio Recording."
                }
                Permission::Calendar if access == Access::Denied => {
                    "Enable Murmur under Privacy & Security → Calendars, or name meetings manually."
                }
                Permission::Calendar
                    if matches!(access, Access::Restricted | Access::Unsupported) =>
                {
                    "You can still name meetings manually."
                }
                Permission::Calendar if access == Access::NotDetermined => {
                    "Continue shows the macOS permission prompt."
                }
                Permission::Calendar => "Calendar details stay on this Mac.",
                _ => "",
            };
            body = body.child(
                div()
                    .w_full()
                    .max_w(px(420.))
                    .self_center()
                    .text_center()
                    .text_color(p.muted)
                    .child(text!(detail)),
            );
            if self
                .model
                .snapshot
                .as_ref()
                .is_some_and(|s| s.needs_relaunch)
                && permission == Permission::SystemAudio
            {
                body=body.child(text!("macOS lists Murmur as allowed, but the permission has not reached this session yet. Quit and reopen Murmur, then check again."));
            }
            if access == Access::Denied && permission != Permission::SystemAudio
                || permission == Permission::Accessibility
                    && self.ax_requested
                    && access != Access::Granted
            {
                let e = cx.entity();
                body = body.child(
                    self.body_button(
                        "reset-permission",
                        if permission == Permission::Calendar {
                            "Reset Calendar permission…"
                        } else {
                            "Reset permission…"
                        },
                    )
                    .intent(ButtonIntent::Ghost)
                    .size(crate::ButtonSize::Compact)
                    .disabled(self.model.pending)
                    .on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| s.request(Action::Reset(permission), cx))
                    }),
                );
            }
            if permission == Permission::SystemAudio
                && matches!(access, Access::Granted | Access::Denied)
            {
                let e = cx.entity();
                body = body.child(
                    self.body_button(
                        "recheck-permission",
                        if access == Access::Denied {
                            "Check again"
                        } else {
                            "Re-check system audio access"
                        },
                    )
                    .disabled(self.model.pending)
                    .on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| s.request(Action::Request(permission), cx))
                    }),
                );
            }
        }
        if self.step == Step::Welcome {
            body=body.child(text!("Setup covers permissions, an offline transcription model, and your recording shortcut."));
        }
        if self.step == Step::Model
            && let Some(snapshot) = &self.model.snapshot
        {
            if let Some(summary) = &snapshot.hardware_summary {
                body = body.child(text!(summary.clone()));
            }
            if snapshot.catalog_error {
                body = body.child(text!(
                    "Could not check installed models. Retry to continue setup."
                ));
            }
            if let Some(models) = &snapshot.catalog {
                for id in CURATED.iter().copied().chain(
                    models
                        .iter()
                        .filter(|m| m.installed && !CURATED.contains(&m.id.as_str()))
                        .map(|m| m.id.as_str()),
                ) {
                    if let Some(model) = models.iter().find(|m| m.id == id) {
                        let id = model.id.clone();
                        let e = cx.entity();
                        let caption = format!(
                            "{} · {}{}",
                            model.label,
                            if model.installed {
                                "Installed"
                            } else {
                                &model.size
                            },
                            if snapshot.recommended.as_ref() == Some(&model.id) {
                                " · Recommended"
                            } else {
                                ""
                            }
                        );
                        body = body.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    self.body_button(
                                        SharedString::from(format!("model-{}", model.id)),
                                        caption,
                                    )
                                    .toggled(self.selected_model == model.id)
                                    .intent(if self.selected_model == model.id {
                                        ButtonIntent::Primary
                                    } else {
                                        ButtonIntent::Secondary
                                    })
                                    .disabled(self.downloading())
                                    .on_click(
                                        move |_, _, cx| {
                                            e.update(cx, |s, cx| {
                                                s.select_model(id.clone(), cx);
                                            })
                                        },
                                    ),
                                )
                                .child(div().text_color(p.muted).text_size(px(12.)).child(text!(
                                    id = SharedString::from(format!("model-detail-{}", model.id)),
                                    model.detail.clone()
                                ))),
                        );
                    }
                }
            }
            if snapshot.warned_models.contains(&self.selected_model) {
                body = body.child(text!(
                    "Higher memory — choose a smaller model if other apps need memory."
                ));
            }
            if self.downloading() {
                let e = cx.entity();
                body=body.child(Progress::new("Model installation in progress",snapshot.download_percent.map(|v|f64::from(v)/100.))).child(self.body_button("cancel-download",if self.cancelling{"Cancelling…"}else{"Cancel download"}).disabled(self.cancelling).on_click(move|_,_,cx|e.update(cx,|s,cx|s.cancel_download(cx)))).child(text!("Cancel to go back or choose another model. Retry resumes supported downloads."));
            }
            if self.model.problem.is_some() && self.selected_model == CURATED[0] {
                for (id, label) in [
                    ("base.en", "Use Whisper base"),
                    ("parakeet-tdt-0.6b-v2-fp16", "Use CPU Parakeet"),
                ] {
                    let e = cx.entity();
                    body = body.child(
                        self.body_button(id, label)
                            .disabled(self.downloading())
                            .on_click(move |_, _, cx| {
                                e.update(cx, |s, cx| {
                                    s.select_model(id.into(), cx);
                                })
                            }),
                    );
                }
            }
        }
        if self.step == Step::Hotkey {
            body = body.child(self.modes.clone()).child(self.keys.clone());
            if self.trigger_key.starts_with('f') && self.trigger_key[1..].parse::<u8>().is_ok() {
                body=body.child(text!("On Apple keyboards, F1–F12 may require Fn or the macOS setting that uses function keys as standard keys. Murmur does not suppress the key's normal system or app action."));
            }
        }
        if self.step == Step::Done {
            if let Some(s) = &self.model.snapshot {
                for (p, missing) in [
                    (
                        Permission::Microphone,
                        "Not granted — enable later in Settings",
                    ),
                    (
                        Permission::Accessibility,
                        "Not granted — shortcut unavailable outside Murmur",
                    ),
                    (
                        Permission::SystemAudio,
                        "Optional — enable later in Meetings",
                    ),
                    (Permission::Calendar, "Optional — name meetings manually"),
                ] {
                    body = body.child(text!(
                        id = SharedString::from(format!("permission-summary-{p:?}")),
                        format!(
                            "{} · {}",
                            p.label(),
                            if s.permissions[p.index()] == Access::Granted {
                                "Granted"
                            } else {
                                missing
                            }
                        )
                    ));
                }
                body = body.child(text!(format!(
                    "Model · {}",
                    if s.installed(&self.selected_model) {
                        "Installed"
                    } else {
                        "Not verified"
                    }
                )));
            }
            let label = crate::settings::schema::ROWS
                .iter()
                .find(|r| r.key == "doubleTapKey")
                .and_then(|r| {
                    if let crate::settings::schema::Kind::Choice(o) = r.kind {
                        o.iter()
                            .find(|(v, _)| *v == self.trigger_key)
                            .map(|(_, l)| *l)
                    } else {
                        None
                    }
                })
                .unwrap_or(&self.trigger_key);
            body = body.child(text!("Try it out")).child(text!(format!(
                "{} {label}{}. Your words are transcribed and copied to the clipboard.",
                if self.recording_mode == "double_tap" {
                    "Double-tap"
                } else {
                    "Hold"
                },
                match self.recording_mode.as_str() {
                    "double_tap" => " to start recording and tap once to stop",
                    "both" => " and speak, then release (or double-tap to toggle)",
                    _ => " and speak, then release",
                }
            )));
        }
        let mut nav = div()
            .id("setup-actions")
            .role(Role::Navigation)
            .aria_label("Setup step actions")
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.));
        if self.step != Step::Welcome && !self.standalone {
            let e = cx.entity();
            nav = nav.child(
                Button::new("setup-back", "Back")
                    .intent(ButtonIntent::Ghost)
                    .disabled(
                        self.downloading()
                            || matches!(self.operation, Some(Action::Complete { .. })),
                    )
                    .on_click(move |_, w, cx| {
                        e.update(cx, |s, cx| {
                            s.visit(Step::ALL[s.step.index().saturating_sub(1)], cx);
                            w.focus(&s.heading, cx);
                        })
                    }),
            );
        }
        nav = nav.child(div().flex_1());
        if self.step.permission().is_some()
            && self.access() != Access::Granted
            && self.mic_wait.is_none()
        {
            let e = cx.entity();
            nav = nav.child(
                Button::new(
                    "setup-skip",
                    match self.step {
                        Step::SystemAudio => "Skip meetings for now",
                        Step::Calendar => "Skip Calendar for now",
                        _ => "Skip for now",
                    },
                )
                .intent(ButtonIntent::Ghost)
                .disabled(self.model.pending)
                .on_click(move |_, w, cx| {
                    e.update(cx, |s, cx| {
                        s.advance(cx);
                        w.focus(&s.heading, cx);
                    })
                }),
            );
        }
        let e = cx.entity();
        let disabled = self.model.pending
            || self.downloading()
            || self.mic_wait.is_some()
            || self.model.snapshot.is_none()
            || self.finished
            || self.step == Step::Model
                && self
                    .model
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.catalog.is_none() && !s.catalog_error);
        nav = nav.child(
            Button::new("setup-next", self.primary_label())
                .track_focus(&self.primary)
                .default_confirm(self.default_navigation())
                .intent(ButtonIntent::Primary)
                .disabled(disabled)
                .on_click(move |_, w, cx| {
                    e.update(cx, |s, cx| {
                        let step = s.step;
                        s.activate(cx);
                        if s.step != step {
                            w.focus(&s.heading, cx);
                        }
                    })
                }),
        );
        div()
            .id("onboarding")
            .capture_key_down(cx.listener(Self::key))
            .size_full()
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .flex()
            .flex_col()
            .px(px(24.))
            .pt(px(100.))
            .pb(px(48.))
            .child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left(px(80.))
                    .text_color(p.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Murmur"),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(520.))
                    .h_full()
                    .self_center()
                    .flex()
                    .flex_col()
                    .gap(px(26.))
                    .child(
                        div()
                            .id("setup-progress")
                            .role(Role::ProgressIndicator)
                            .aria_numeric_value((self.step.index() + 1) as f64)
                            .aria_min_numeric_value(0.)
                            .aria_max_numeric_value(8.)
                            .aria_label(format!("Step {} of 8", self.step.index() + 1))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().flex().items_center().gap(px(8.)).children((0..8).map(
                                |i| {
                                    div()
                                        .h(px(6.))
                                        .w(px(if i == self.step.index() { 24. } else { 6. }))
                                        .rounded_full()
                                        .bg(if i <= self.step.index() {
                                            p.accent
                                        } else {
                                            p.muted
                                        })
                                },
                            )))
                            .child(
                                div()
                                    .text_size(tokens::CAPTION_SIZE)
                                    .text_color(p.muted)
                                    .child(format!("Step {} of 8", self.step.index() + 1)),
                            ),
                    )
                    .child(body)
                    .child(nav),
            )
    }
}
