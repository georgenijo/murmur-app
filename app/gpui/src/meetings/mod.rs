//! Native Notetaker. Existing services retain capture, storage and privacy ownership.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
pub mod playback;
#[cfg(test)]
mod tests;
mod view;
use crate::{
    Button, ButtonSize, Choice, ChoiceKind, Choices, SelectionChanged, TextField,
    field::{InputEvent, TextFieldState},
    tokens::{self, Palette},
    workspace::Authority,
};
use gpui::{prelude::*, *};
use host::{Action, Host, PlaybackAction, Request};
use inbox::Inbox;
use model::*;
use std::{
    collections::BTreeMap,
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(PartialEq, Eq)]
struct ConfirmationOwner {
    detail: Option<String>,
    recording_generation: u64,
    recording_session: Option<String>,
    busy: bool,
}
pub struct Meetings {
    pub embedded: bool,
    list_actions_open: bool,
    pub model: Workspace<Snapshot>,
    host: Rc<dyn Host>,
    inbox: Inbox,
    authority: Authority,
    subscription: Option<Box<dyn host::Subscription>>,
    subscriptions: Vec<gpui::Subscription>,
    pub search: Entity<TextFieldState>,
    fields: BTreeMap<String, Entity<TextFieldState>>,
    attendees: Option<Entity<gpui_component::input::TextareaState>>,
    field_subscriptions: Vec<gpui::Subscription>,
    format_control: Entity<Choices>,
    channel_control: Entity<Choices>,
    seek: Entity<crate::settings::range::Range>,
    pub format: Format,
    pub naming: bool,
    pub speakers: bool,
    calendar_selection: Option<String>,
    confirmation: Option<(String, Instant)>,
    focus: BTreeMap<String, FocusHandle>,
    row_reveal: BTreeMap<String, Rc<std::cell::Cell<bool>>>,
    query_epoch: u64,
    detail_id: Option<String>,
    page: usize,
    text_pages: BTreeMap<u64, usize>,
    list_scroll: ListState,
    list_identity: Vec<(String, i64)>,
    list_selection: Option<String>,
    detail_scroll: ScrollHandle,
    content_scroll: ScrollHandle,
    seek_state: Option<(u64, u64, bool)>,
}
impl Meetings {
    pub fn new(host: Rc<dyn Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextFieldState::new(w, cx).placeholder("Search meetings"));
        let format_control = cx.new(|cx| {
            Choices::new(
                "Meeting transcript export format",
                ChoiceKind::Dropdown,
                Format::ALL
                    .iter()
                    .map(|f| Choice::new(f.wire(), f.label()))
                    .collect(),
                w,
                cx,
            )
        });
        format_control.update(cx, |s, cx| {
            s.compact = true;
            s.set_width(px(124.), cx);
            s.set_value("markdown", cx)
        });
        let channel_control = cx.new(|cx| {
            Choices::new(
                "Playback channel",
                ChoiceKind::Dropdown,
                [Channel::All, Channel::Me, Channel::Them]
                    .iter()
                    .map(|c| Choice::new(c.wire(), c.label()))
                    .collect(),
                w,
                cx,
            )
        });
        channel_control.update(cx, |s, cx| {
            s.compact = true;
            s.set_width(px(108.), cx);
            s.set_value("all", cx)
        });
        let seek = cx.new(|cx| {
            crate::settings::range::Range::new("Playback position", "ms", 0, 1, 100, w, cx)
        });
        seek.update(cx, |s, _| s.compact = true);
        let mut s = Self {
            embedded: false,
            list_actions_open: false,
            model: Workspace::default(),
            host,
            inbox: Inbox::default(),
            authority: Authority::default(),
            subscription: None,
            subscriptions: vec![],
            search: search.clone(),
            fields: BTreeMap::new(),
            attendees: None,
            field_subscriptions: vec![],
            format_control: format_control.clone(),
            channel_control: channel_control.clone(),
            seek: seek.clone(),
            format: Format::Markdown,
            naming: false,
            speakers: false,
            calendar_selection: None,
            confirmation: None,
            focus: BTreeMap::new(),
            row_reveal: BTreeMap::new(),
            query_epoch: 0,
            detail_id: None,
            page: 0,
            text_pages: BTreeMap::new(),
            list_scroll: ListState::new(0, ListAlignment::Top, px(80.)),
            list_identity: Vec::new(),
            list_selection: None,
            detail_scroll: ScrollHandle::new(),
            content_scroll: ScrollHandle::new(),
            seek_state: None,
        };
        for key in ["title", "attendees", "me", "them"] {
            s.fields
                .insert(key.into(), cx.new(|cx| TextFieldState::new(w, cx)));
        }
        s.subscriptions
            .push(cx.subscribe(&search, |s, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    s.query_epoch += 1;
                    let epoch = s.query_epoch;
                    s.model.invalidate();
                    s.authority.sync(&s.model);
                    s.confirmation = None;
                    cx.spawn(async move |weak, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(250))
                            .await;
                        let _ = weak.update(cx, |s, cx| {
                            if s.model.attached && s.query_epoch == epoch {
                                s.search_now(cx);
                            }
                        });
                    })
                    .detach();
                }
            }));
        s.subscriptions.push(
            cx.subscribe(&format_control, |s, _, e: &SelectionChanged, cx| {
                if let Some(f) = Format::ALL.into_iter().find(|f| f.wire() == e.0.as_ref()) {
                    s.format = f;
                    cx.notify();
                }
            }),
        );
        s.subscriptions.push(
            cx.subscribe(&channel_control, |s, _, e: &SelectionChanged, cx| {
                if let Some(channel) = [Channel::All, Channel::Me, Channel::Them]
                    .into_iter()
                    .find(|c| c.wire() == e.0.as_ref())
                {
                    s.playback(PlaybackAction::Channel(channel), cx);
                }
            }),
        );
        s.subscriptions.push(cx.subscribe(
            &seek,
            |s, _, e: &crate::settings::range::Committed, cx| {
                s.playback(PlaybackAction::Seek(e.0.max(0) as u64), cx)
            },
        ));
        s
    }
    fn focus_for(&mut self, id: &str, cx: &mut Context<Self>) -> FocusHandle {
        self.focus
            .entry(id.into())
            .or_insert_with(|| cx.focus_handle())
            .clone()
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
    fn detach(&mut self) {
        self.host.discard_calendar();
        self.host.pause_playback();
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut sub) = self.subscription.take() {
            sub.stop();
        }
        self.model.detach();
        self.clear_fields();
        self.detail_id = None;
        self.calendar_selection = None;
        self.confirmation = None;
        self.naming = false;
        self.speakers = false;
        self.text_pages.clear();
        self.row_reveal.clear();
        self.page = 0;
        self.query_epoch += 1;
        self.seek_state = None;
    }
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.detach();
        self.search.update(cx, |s, cx| s.set_value("", w, cx));
        cx.notify();
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
                        s.host.tick(s.model.attachment, s.inbox.clone());
                        s.drain(cx);
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
            let ownership = self.confirmation_owner();
            if self.model.event(event) {
                self.authority.sync(&self.model);
                if ownership != self.confirmation_owner() {
                    self.confirmation = None;
                }
                if let Some(s) = self.model.snapshot.clone() {
                    let id = s.detail.as_ref().map(|d| d.session.id.clone());
                    if s.busy() {
                        self.host.discard_calendar();
                        if let Some(current) = self.model.snapshot.as_mut() {
                            current.calendar = Calendar::Closed;
                        }
                        self.host.pause_playback();
                        self.calendar_selection = None;
                    }
                    if self.detail_id != id {
                        self.detail_id = id;
                        self.naming = false;
                        self.speakers = false;
                        self.calendar_selection = None;
                        self.clear_fields();
                        self.page = 0;
                        self.text_pages.clear();
                        self.seek_state = None;
                        self.host.pause_playback();
                    }
                    if matches!(s.calendar, Calendar::Closed) {
                        self.calendar_selection = None;
                    }
                    if s.notice.as_deref() == Some("Meeting details saved on this Mac.")
                        || s.notice.as_deref() == Some("Meeting named from calendar.")
                    {
                        self.naming = false;
                        self.clear_fields();
                    }
                }
                cx.notify();
            }
        }
    }
    pub fn request(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.allowed(&action) {
            return;
        }
        if matches!(
            action,
            Action::Back
                | Action::Playback {
                    action: PlaybackAction::Pause,
                    ..
                }
        ) {
            // Pause and navigation remain immediate while startup is pending.
            self.host.pause_playback();
            self.model.invalidate();
        }
        let Some(ticket) = self.model.begin() else {
            return;
        };
        self.authority.sync(&self.model);
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
    fn allowed(&self, a: &Action) -> bool {
        let Some(s) = &self.model.snapshot else {
            return matches!(a, Action::Read);
        };
        let detail = |id: &str| s.detail.as_ref().filter(|d| d.session.id == id);
        match a {
            Action::Start | Action::StartSuggested(_) => {
                !s.busy() && s.permission != Permission::Unsupported
            }
            Action::Stop { generation } => {
                s.phase.active()
                    && s.phase != Phase::Stopping
                    && *generation == s.recording_generation
            }
            Action::CheckAccess => !s.busy(),
            Action::SaveMetadata {
                id,
                title,
                attendees,
            } => {
                !s.busy()
                    && detail(id).is_some()
                    && title.as_ref().is_none_or(|t| t.chars().count() <= 200)
                    && attendees.len() <= 100
                    && attendees.iter().all(|t| t.chars().count() <= 200)
            }
            Action::SaveLabels { id, me, them, .. } => {
                detail(id).is_some()
                    && !me.trim().is_empty()
                    && !them.trim().is_empty()
                    && me.chars().count() <= 80
                    && them.chars().count() <= 80
            }
            Action::RenameSpeaker { id, speaker, label } => {
                detail(id).is_some_and(|d| d.remote.iter().any(|r| r.id == *speaker))
                    && !label.trim().is_empty()
                    && label.chars().count() <= 80
            }
            Action::CalendarLookup(id) => !s.busy() && detail(id).is_some_and(Detail::finished),
            Action::CalendarApply { id, token } => {
                !s.busy()
                    && detail(id).is_some_and(Detail::finished)
                    && matches!(&s.calendar,Calendar::Choosing(c) if c.iter().any(|c|&c.token==token))
            }
            Action::Copy { id, format } | Action::Export { id, format } => {
                detail(id).is_some_and(|d| !format.captions() || d.finished())
            }
            Action::BundlePreview(id) => {
                !s.busy() && detail(id).is_some_and(|d| d.finished() && d.session.retain_audio)
            }
            Action::BundleExport { id, .. } => {
                !s.busy()
                    && detail(id).is_some_and(|d| d.finished() && d.session.retain_audio)
                    && s.bundle
                        .as_ref()
                        .is_some_and(|b| b.audio_bytes <= b.max_bytes)
            }
            Action::Delete(id) => !(s.phase.active() && s.recording_session.as_ref() == Some(id)),
            Action::DeleteAll => !s.phase.busy(),
            Action::Playback { id, action } => {
                detail(id).is_some_and(|d| d.finished() && d.session.retain_audio)
                    && (matches!(action, PlaybackAction::Pause) || !s.busy())
            }
            _ => true,
        }
    }
    /// Called only after explicit acceptance in the existing native overlay.
    pub fn accept_suggestion(&mut self, token: String, cx: &mut Context<Self>) {
        if !token.is_empty() && token.len() <= 4096 {
            self.request(Action::StartSuggested(token), cx);
        }
    }
    pub fn record(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &self.model.snapshot {
            let action = if s.phase.active() {
                Action::Stop {
                    generation: s.recording_generation,
                }
            } else {
                Action::Start
            };
            self.host.pause_playback();
            self.request(action, cx);
        }
    }
    pub fn search_now(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_owned();
        self.query_epoch += 1;
        self.cancel_transient();
        self.model.invalidate();
        self.authority.sync(&self.model);
        self.request(Action::List { query, limit: 50 }, cx);
    }
    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        self.page = 0;
        self.list_selection = Some(id.clone());
        self.host.pause_playback();
        self.cancel_transient();
        self.model.invalidate();
        if let Some(s) = &mut self.model.snapshot {
            s.detail = None;
            s.calendar = Calendar::Closed;
            s.bundle = None;
        }
        self.authority.sync(&self.model);
        self.request(Action::Select(id), cx);
    }
    fn cancel_transient(&mut self) {
        self.host.discard_calendar();
        self.calendar_selection = None;
        self.naming = false;
        self.speakers = false;
        self.confirmation = None;
        self.clear_fields();
    }
    fn close_calendar(&mut self, cx: &mut Context<Self>) {
        self.host.discard_calendar();
        self.model.invalidate();
        if let Some(s) = &mut self.model.snapshot {
            s.calendar = Calendar::Closed;
        }
        self.calendar_selection = None;
        self.authority.sync(&self.model);
        cx.notify();
    }
    pub fn delete(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(s) = &self.model.snapshot else {
            return;
        };
        let id = if all {
            "*".into()
        } else {
            let Some(d) = &s.detail else {
                return;
            };
            d.session.id.clone()
        };
        let action = if all {
            Action::DeleteAll
        } else {
            Action::Delete(id.clone())
        };
        if !self.allowed(&action) {
            return;
        }
        if self
            .confirmation
            .as_ref()
            .is_some_and(|(key, deadline)| key == &id && Instant::now() < *deadline)
        {
            self.host.pause_playback();
            self.confirmation = None;
            self.request(action, cx);
        } else {
            self.confirmation = Some((id, Instant::now() + Duration::from_secs(4)));
            cx.notify();
        }
    }
    pub fn playback(&mut self, action: PlaybackAction, cx: &mut Context<Self>) {
        if let Some(d) = self.model.snapshot.as_ref().and_then(|s| s.detail.as_ref()) {
            self.request(
                Action::Playback {
                    id: d.session.id.clone(),
                    action,
                },
                cx,
            );
        }
    }
    fn play_segment(&mut self, channel: Channel, start_ms: u64, cx: &mut Context<Self>) {
        self.playback(PlaybackAction::Segment { channel, start_ms }, cx);
    }
    fn ensure_fields(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.model.snapshot.as_ref().and_then(|s| s.detail.as_ref()) else {
            return;
        };
        let attendees = d.session.attendees.join("\n");
        if self.attendees.is_none() {
            let input = cx.new(|cx| {
                let mut s = gpui_component::input::TextareaState::new(w, cx).auto_grow(2, 4);
                s.set_value(attendees, w, cx);
                s
            });
            self.field_subscriptions
                .push(cx.observe(&input, |_, _, cx| cx.notify()));
            self.attendees = Some(input);
        }
        let mut values = vec![
            ("title".into(), d.session.title.clone().unwrap_or_default()),
            ("me".into(), d.me.clone()),
            ("them".into(), d.them.clone()),
        ];
        values.extend(
            d.remote
                .iter()
                .map(|r| (format!("remote-{}", r.id), r.label.clone())),
        );
        for (key, value) in values {
            if !self.fields.contains_key(&key) {
                let max = if key == "title" { 200 } else { 80 };
                let input = cx.new(|cx| {
                    let mut input =
                        TextFieldState::new(w, cx).validate(move |s, _| s.chars().count() <= max);
                    input.set_value(value, w, cx);
                    input
                });
                self.field_subscriptions
                    .push(cx.observe(&input, |_, _, cx| cx.notify()));
                self.fields.insert(key, input);
            }
        }
    }
    fn clear_fields(&mut self) {
        self.fields.clear();
        self.attendees = None;
        self.field_subscriptions.clear();
    }
    fn value(&self, key: &str, cx: &App) -> String {
        if key == "attendees" {
            return self
                .attendees
                .as_ref()
                .map(|s| s.read(cx).value().to_string())
                .unwrap_or_default();
        }
        self.fields
            .get(key)
            .map(|s| s.read(cx).value().to_string())
            .unwrap_or_default()
    }
    pub fn name(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.ensure_fields(w, cx);
        self.naming = true;
        if let Some(f) = self.fields.get("title") {
            w.focus(&f.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }
    fn save_metadata(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.model.snapshot.as_ref().and_then(|s| s.detail.as_ref()) else {
            return;
        };
        let title = self.value("title", cx).trim().to_owned();
        let attendees = self
            .value("attendees", cx)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        self.request(
            Action::SaveMetadata {
                id: d.session.id.clone(),
                title: (!title.is_empty()).then_some(title),
                attendees,
            },
            cx,
        );
    }
    fn confirmation_owner(&self) -> Option<ConfirmationOwner> {
        self.model.snapshot.as_ref().map(|s| ConfirmationOwner {
            detail: s.detail.as_ref().map(|d| d.session.id.clone()),
            recording_generation: s.recording_generation,
            recording_session: s.recording_session.clone(),
            busy: s.busy(),
        })
    }
    fn detail_button(&self, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        Button::new(id, label).with_scroll_parents(vec![
            self.detail_scroll.clone(),
            self.content_scroll.clone(),
        ])
    }
    fn button(
        &mut self,
        id: impl Into<String>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Button {
        let id = id.into();
        let f = self.focus_for(&id, cx);
        let cancels_playback = matches!(
            action,
            Action::Back
                | Action::Playback {
                    action: PlaybackAction::Pause,
                    ..
                }
        );
        let disabled = (self.model.pending && !cancels_playback) || !self.allowed(&action);
        Button::new(id.clone(), label)
            .with_scroll_parent(Some(&self.content_scroll))
            .when(
                !matches!(
                    id.as_str(),
                    "back" | "retry" | "check-access" | "audio-settings" | "load-more"
                ),
                |b| {
                    b.with_scroll_parents(vec![
                        self.detail_scroll.clone(),
                        self.content_scroll.clone(),
                    ])
                },
            )
            .size(ButtonSize::Compact)
            .track_focus(&f)
            .disabled(disabled)
            .on_click(cx.listener(move |s, _, _, cx| s.request(action.clone(), cx)))
    }
    fn list_key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) -> bool {
        if e.prefer_character_input
            || e.keystroke.modifiers.platform
            || e.keystroke.modifiers.alt
            || e.keystroke.modifiers.control
        {
            return false;
        }
        let Some(snapshot) = self.model.snapshot.as_ref() else {
            return false;
        };
        let focused = snapshot.meetings.iter().position(|m| {
            self.focus
                .get(&format!("meeting-{}", m.id))
                .is_some_and(|f| f.is_focused(w))
        });
        let Some(current) = focused else {
            return false;
        };
        let index = match e.keystroke.key.as_str() {
            "down" => (current + 1).min(snapshot.meetings.len() - 1),
            "up" => current.saturating_sub(1),
            "home" => 0,
            "end" => snapshot.meetings.len() - 1,
            _ => return false,
        };
        let id = snapshot.meetings[index].id.clone();
        self.list_scroll.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        let focus = self.focus_for(&format!("meeting-{id}"), cx);
        w.focus(&focus, cx);
        if index != current {
            self.select(id, cx);
        }
        w.prevent_default();
        cx.stop_propagation();
        cx.notify();
        true
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.list_key(e, w, cx) {
            return;
        }
        if e.is_held {
            return;
        }
        let key = e.keystroke.key.as_str();
        if e.keystroke.modifiers.platform && key == "f" {
            w.focus(&self.search.read(cx).focus_handle(cx), cx);
            cx.stop_propagation();
        }
        if key == "enter" && self.search.read(cx).focus_handle(cx).is_focused(w) {
            self.search_now(cx);
            cx.stop_propagation();
        }
        if key == "escape" {
            if self.list_actions_open {
                self.list_actions_open = false;
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self
                .model
                .snapshot
                .as_ref()
                .is_some_and(|s| !matches!(s.calendar, Calendar::Closed))
            {
                self.close_calendar(cx);
                if let Some(f) = self.focus.get("calendar") {
                    w.focus(f, cx);
                }
            } else if self.naming || self.speakers {
                self.naming = false;
                self.speakers = false;
                self.clear_fields();
                if let Some(f) = self.focus.get("rename") {
                    w.focus(f, cx);
                }
            } else if self.search.read(cx).focus_handle(cx).is_focused(w) {
                self.search.update(cx, |s, cx| s.set_value("", w, cx));
                self.search_now(cx);
            }
            self.confirmation = None;
            cx.stop_propagation();
            cx.notify();
        }
    }
}
impl Drop for Meetings {
    fn drop(&mut self) {
        self.detach();
    }
}
