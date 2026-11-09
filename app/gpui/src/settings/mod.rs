//! Native Settings workspace in Root. All I/O is injected by the owning host.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
mod panes;
pub mod range;
pub mod schema;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent, ButtonSize, Choice, ChoiceKind, Choices, SelectionChanged, TextField,
    Toggle,
    field::{InputEvent, TextFieldState},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::{Action, Authority, Cue, Destination, Host, Request, RequestKind};
use inbox::Inbox;
use model::*;
use schema::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Done;
impl EventEmitter<Done> for Settings {}
pub struct Settings {
    embedded: Vec<(Page, AnyView)>,
    pub model: Model,
    host: Rc<dyn Host>,
    inbox: Inbox,
    authority: Authority,
    subscription: Option<Box<dyn host::Subscription>>,
    subscriptions: Vec<gpui::Subscription>,
    pub search: Entity<TextFieldState>,
    navigation: Entity<Choices>,
    pickers: BTreeMap<&'static str, Entity<Choices>>,
    range_drafts: BTreeMap<&'static str, Rc<std::cell::Cell<Option<i64>>>>,
    ranges: BTreeMap<&'static str, Entity<range::Range>>,
    numbers: BTreeMap<&'static str, Entity<TextFieldState>>,
    focus: BTreeMap<String, FocusHandle>,
    scroll: ScrollHandle,
    nav_scroll: ScrollHandle,
    back: Vec<(Page, String)>,
    editor_return: Option<String>,
    reveal: Option<&'static str>,
    confirmation: Option<(Action, Ticket, Instant)>,
    calibration: Option<i64>,
    lab_selection: BTreeSet<String>,
    synced_revision: Option<u64>,
    synced_pending: bool,
    synced_values: BTreeMap<&'static str, Value>,
    pub fixture: bool,
    #[cfg(any(test, feature = "gallery"))]
    pub fixture_shortcuts: bool,
    #[cfg(any(test, feature = "gallery"))]
    fixture_capture: Option<&'static str>,
    #[cfg(any(test, feature = "gallery"))]
    fixture_capture_message: Option<&'static str>,
    compact_search: bool,
    operation: Option<(Action, Ticket)>,
    restore_focus: Option<&'static str>,
    returned_opener: Option<String>,
    /// A control another screen asked to reveal once this page has its snapshot.
    pending_target: Option<(Page, &'static str)>,
    /// A revealed control to focus as soon as its row has been drawn.
    reveal_focus: Option<Row>,
    reveal_anchor: ScrollAnchor,
    reveal_scroll: bool,
    was_compact: Option<bool>,
}
impl Settings {
    pub fn embed(&mut self, page: Page, view: impl Into<AnyView>) {
        self.embedded.retain(|(p, _)| *p != page);
        self.embedded.push((page, view.into()));
    }
    pub fn new(host: Rc<dyn Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextFieldState::new(w, cx).placeholder("Search settings"));
        let navigation = cx.new(|cx| {
            Choices::new(
                "Settings pages",
                ChoiceKind::Navigation,
                Page::NAV
                    .into_iter()
                    .map(|p| Choice::new(p.id(), p.label()))
                    .collect(),
                w,
                cx,
            )
        });
        navigation.update(cx, |s, cx| s.set_value("general", cx));
        let scroll = ScrollHandle::new();
        let nav_scroll = ScrollHandle::new();
        navigation.update(cx, |n, _| n.scroll_parent = Some(nav_scroll.clone()));
        let mut s = Self {
            embedded: vec![],
            model: Model::default(),
            host,
            inbox: Inbox::default(),
            authority: Authority::default(),
            subscription: None,
            subscriptions: vec![],
            search: search.clone(),
            navigation: navigation.clone(),
            pickers: BTreeMap::new(),
            range_drafts: BTreeMap::new(),
            ranges: BTreeMap::new(),
            numbers: BTreeMap::new(),
            focus: BTreeMap::new(),
            reveal_anchor: ScrollAnchor::for_handle(scroll.clone()),
            reveal_scroll: false,
            was_compact: None,
            scroll,
            nav_scroll,
            back: vec![],
            editor_return: None,
            reveal: None,
            confirmation: None,
            calibration: None,
            lab_selection: BTreeSet::new(),
            synced_revision: None,
            synced_pending: false,
            synced_values: BTreeMap::new(),
            fixture: false,
            #[cfg(any(test, feature = "gallery"))]
            fixture_shortcuts: false,
            #[cfg(any(test, feature = "gallery"))]
            fixture_capture: None,
            #[cfg(any(test, feature = "gallery"))]
            fixture_capture_message: None,
            compact_search: false,
            operation: None,
            restore_focus: None,
            returned_opener: None,
            pending_target: None,
            reveal_focus: None,
        };
        s.subscriptions
            .push(cx.subscribe(&search, |s, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    s.model.query = s.search.read(cx).value().to_string();
                    s.confirmation = None;
                    cx.notify();
                }
            }));
        s.subscriptions.push(cx.subscribe_in(
            &navigation,
            w,
            |s, _, e: &SelectionChanged, w, cx| s.navigate(Page::resolve(&e.0), None, w, cx),
        ));
        for &row in ROWS {
            match row.kind {
                Kind::Choice(options) => {
                    let picker = cx.new(|cx| {
                        Choices::new(
                            row.label,
                            ChoiceKind::Dropdown,
                            options.iter().map(|(v, l)| Choice::new(*v, *l)).collect(),
                            w,
                            cx,
                        )
                    });
                    picker.update(cx, |p, _| p.scroll_parent = Some(s.scroll.clone()));
                    s.subscriptions.push(cx.subscribe_in(
                        &picker,
                        w,
                        move |s, _, e: &SelectionChanged, w, cx| {
                            let value = if row.key == "microphone" {
                                Some(json!(e.0.as_ref()))
                            } else {
                                row.parse(&e.0)
                            };
                            if let Some(value) = value {
                                s.edit(row, value, w, cx);
                            }
                        },
                    ));
                    s.pickers.insert(row.key, picker);
                }
                Kind::Range(min, max, step, unit) => {
                    let range =
                        cx.new(|cx| range::Range::new(row.label, unit, min, max, step, w, cx));
                    range.update(cx, |r, _| r.scroll_parent = Some(s.scroll.clone()));
                    s.subscriptions.push(cx.subscribe_in(
                        &range,
                        w,
                        move |s, _, e: &range::Committed, w, cx| s.edit(row, json!(e.0), w, cx),
                    ));
                    s.range_drafts.insert(row.key, range.read(cx).draft.clone());
                    s.ranges.insert(row.key, range);
                }
                Kind::Number(_, _) => {
                    let field = cx.new(|cx| TextFieldState::new(w, cx));
                    s.subscriptions.push(cx.subscribe_in(
                        &field,
                        w,
                        move |s, _, e: &InputEvent, w, cx| {
                            if matches!(
                                e,
                                InputEvent::PressEnter {
                                    secondary: false,
                                    shift: false
                                }
                            ) {
                                s.commit_number(row, w, cx);
                            }
                        },
                    ));
                    s.numbers.insert(row.key, field);
                }
                _ => (),
            }
        }
        s
    }
    fn focus_for(&mut self, id: impl Into<String>, cx: &mut Context<Self>) -> FocusHandle {
        self.focus
            .entry(id.into())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.subscription.is_some() {
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
        self.drain(cx);
        self.host.page(Some(self.model.page), self.model.attachment);
        self.request(RequestKind::Read, cx);
    }
    fn detach(&mut self) {
        self.authority.revoke();
        self.host.page(None, self.model.attachment);
        self.inbox.close();
        if let Some(mut s) = self.subscription.take() {
            s.stop();
        }
        self.model.detach();
        self.back.clear();
        self.editor_return = None;
        self.reveal = None;
        self.confirmation = None;
        self.calibration = None;
        #[cfg(any(test, feature = "gallery"))]
        {
            self.fixture_capture = None;
            self.fixture_capture_message = None;
        }
        self.lab_selection.clear();
        self.synced_revision = None;
        self.synced_values.clear();
        self.operation = None;
        self.restore_focus = None;
        self.returned_opener = None;
        self.pending_target = None;
        self.reveal_focus = None;
    }
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.flush_ranges(w, cx);
        self.detach();
        self.search.update(cx, |s, cx| s.set_value("", w, cx));
        for field in self.numbers.values() {
            field.update(cx, |s, cx| s.set_value("", w, cx));
        }
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
                        s.drain(cx);
                        if s.confirmation
                            .as_ref()
                            .is_some_and(|(_, _, t)| t.elapsed() >= Duration::from_secs(4))
                        {
                            s.confirmation = None;
                            cx.notify();
                        }
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
        for e in self.inbox.drain() {
            let completion = match &e {
                Event::Reply {
                    ticket,
                    result: Ok(_),
                } => self
                    .operation
                    .as_ref()
                    .filter(|(_, t)| t == ticket)
                    .map(|(a, _)| a.clone()),
                _ => None,
            };
            if self.model.event(e) {
                self.invalidate_changed_drafts();
                self.synced_revision = None;
                self.authority.sync(&self.model);
                // Save the acknowledged request, not a failed preview or the
                // display-clamped applied position. The model fences stale replies.
                if self.model.problem.is_none()
                    && let Some(Action::Calibration {
                        offset,
                        finish: false,
                    }) = &completion
                {
                    self.calibration = Some(*offset);
                }
                if matches!(
                    completion,
                    Some(Action::CancelCalibration | Action::Calibration { finish: true, .. })
                ) {
                    self.calibration = None;
                    self.restore_focus = Some("calibrate");
                }
                if !self.model.pending {
                    self.operation = None;
                }
                if self
                    .confirmation
                    .as_ref()
                    .is_some_and(|(_, ticket, _)| *ticket != self.model.ticket())
                {
                    self.confirmation = None;
                }
                cx.notify();
            }
        }
    }
    pub fn receive(&mut self, e: Event, cx: &mut Context<Self>) {
        if self.model.event(e) {
            self.invalidate_changed_drafts();
        }
        self.authority.sync(&self.model);
        cx.notify();
    }
    fn request(&mut self, kind: RequestKind, cx: &mut Context<Self>) {
        let Some(ticket) = self.model.begin() else {
            return;
        };
        self.authority.sync(&self.model);
        if let RequestKind::Action(action) = &kind {
            self.operation = Some((action.clone(), ticket));
        }
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                kind,
            },
            self.inbox.clone(),
        );
        self.drain(cx);
        cx.notify();
    }
    fn edit(&mut self, row: Row, value: Value, w: &mut Window, cx: &mut Context<Self>) {
        if self.model.disabled(row) || !row.visible(self.model.settings()) {
            return;
        }
        let current = self.model.settings().clone();
        let mut candidate = current.clone();
        candidate[row.key] = value.clone();
        if !murmur_core::state::settings(candidate).is_ok_and(|v| v[row.key] == value) {
            self.model.problem = Some(Problem::Invalid);
            cx.notify();
            return;
        }
        if current[row.key] == value
            && !(row.key == "microphone"
                && (current["smartAutoMicrophoneEnabled"] == true
                    || current["microphoneIdMigrationComplete"] != true))
        {
            return;
        }
        // Immediately remove hidden branches from input/AX. Restore contained focus
        // to the controlling switch before their controls disappear on the next draw.
        if value == false {
            for child in ROWS.iter().filter(|r| r.parent == Some(row.key)) {
                if self
                    .control_focus(*child, cx)
                    .is_some_and(|f| f.is_focused(w))
                {
                    w.focus(&self.focus_for(row.key, cx), cx);
                }
            }
        }
        let kind = if row.key == "microphone" {
            RequestKind::ManualMicrophone {
                id: value.as_str().unwrap().to_string(),
            }
        } else {
            RequestKind::Patch {
                key: row.key,
                value,
            }
        };
        self.request(kind, cx);
    }
    fn flush_ranges(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        for &row in ROWS {
            if let Some(value) = self.range_drafts.get(row.key).and_then(|d| d.get())
                && self.model.settings()[row.key] != json!(value)
                && self.model.snapshot.is_some()
            {
                self.edit(row, json!(value), w, cx);
            }
        }
    }
    fn invalidate_changed_drafts(&mut self) {
        for (&key, draft) in &self.range_drafts {
            if self.synced_values.get(key) != self.model.settings().get(key) {
                draft.set(None);
            }
        }
    }
    fn commit_number(&mut self, row: Row, w: &mut Window, cx: &mut Context<Self>) {
        let text = self.numbers[row.key].read(cx).value().to_string();
        if let Some(v) = row.parse(&text) {
            self.edit(row, v, w, cx);
        } else {
            self.model.problem = Some(Problem::Invalid);
            cx.notify();
        }
    }
    fn control_focus(&self, row: Row, cx: &App) -> Option<FocusHandle> {
        if let Some(e) = self.ranges.get(row.key) {
            Some(e.read(cx).focus_handle(cx))
        } else if let Some(e) = self.pickers.get(row.key) {
            Some(e.read(cx).focus_handle(cx))
        } else if let Some(e) = self.numbers.get(row.key) {
            Some(e.read(cx).focus_handle(cx))
        } else {
            self.focus.get(row.key).cloned()
        }
    }
    fn synchronize(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.synced_revision == Some(self.model.revision)
            && self.synced_pending == self.model.pending
        {
            return;
        }
        let runtime = self.model.runtime();
        let settings = self.model.settings().clone();
        let mut mic = vec![Choice::new("system_default", "System default")];
        mic.extend(runtime.devices.iter().map(|d| {
            Choice::new(
                d.id.clone(),
                if d.available {
                    d.label.clone()
                } else {
                    format!("{} — unavailable", d.label)
                },
            )
            .disabled(!d.available || runtime.inventory_stale)
        }));
        let selected = settings["microphone"].as_str().unwrap_or("system_default");
        if !mic.iter().any(|d| d.value == selected) {
            mic.push(
                Choice::new(selected.to_string(), "Saved microphone — unavailable").disabled(true),
            );
        }
        self.pickers["microphone"].update(cx, |s, cx| s.replace_choices(mic, cx));
        for &row in ROWS {
            if let Some(e) = self.pickers.get(row.key) {
                let v = if row.key == "pasteLastShortcut" && settings[row.key].is_null() {
                    "disabled".into()
                } else {
                    settings[row.key]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| settings[row.key].to_string())
                };
                e.update(cx, |s, cx| {
                    if row.key != "microphone"
                        && let Kind::Choice(options) = row.kind
                    {
                        let mut choices = options
                            .iter()
                            .map(|(v, l)| Choice::new(*v, *l))
                            .collect::<Vec<_>>();
                        if !choices.iter().any(|c| c.value == v) {
                            choices.push(Choice::new(v.clone(), format!("{v} (saved)")));
                        }
                        s.replace_choices(choices, cx);
                    }
                    s.set_saved_value(&v, cx);
                    s.set_disabled(self.model.disabled(row), cx);
                });
            }
        }
        for &row in ROWS {
            let changed = self.synced_values.get(row.key) != settings.get(row.key);
            let revert = self.model.problem.is_some();
            if let Some(e) = self.ranges.get(row.key) {
                e.update(cx, |s, cx| {
                    if changed || revert || runtime.recording && row.capture_locked {
                        s.set(
                            settings[row.key].as_i64().unwrap_or(0),
                            self.model.disabled(row),
                            w,
                            cx,
                        );
                    } else {
                        s.set_disabled(self.model.disabled(row), cx);
                    }
                });
            }
            if let Some(e) = self.numbers.get(row.key)
                && (changed || revert)
            {
                e.update(cx, |s, cx| {
                    s.set_value(settings[row.key].to_string(), w, cx)
                });
            }
            self.synced_values
                .insert(row.key, settings[row.key].clone());
        }
        self.synced_revision = Some(self.model.revision);
        self.synced_pending = self.model.pending;
    }

    pub fn navigate(
        &mut self,
        page: Page,
        back: Option<(Page, String)>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_ranges(w, cx);
        if self.operation.is_some() {
            self.authority.revoke();
            self.model.generation += 1;
            self.model.pending = false;
            self.operation = None;
            self.authority.sync(&self.model);
        }
        self.confirmation = None;
        self.calibration = None;
        self.reveal = None;
        self.reveal_scroll = false;
        self.pending_target = None;
        self.reveal_focus = None;
        self.back = back.into_iter().collect();
        self.editor_return = None;
        #[cfg(any(test, feature = "gallery"))]
        {
            self.fixture_capture = None;
            self.fixture_capture_message = None;
        }
        self.model.page = page;
        self.model.query.clear();
        self.search.update(cx, |s, cx| s.set_value("", w, cx));
        self.navigation
            .update(cx, |s, cx| s.set_value(page.id(), cx));
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.host.page(Some(page), self.model.attachment);
        cx.notify();
    }
    fn go_back(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some((page, id)) = self.back.pop() {
            let stack = std::mem::take(&mut self.back);
            self.navigate(page, None, w, cx);
            self.back = stack;
            let f = self.focus_for(id, cx);
            w.focus(&f, cx);
        }
    }
    fn open_detail(&mut self, page: Page, id: &str, w: &mut Window, cx: &mut Context<Self>) {
        let owner = self.model.page;
        let mut stack = std::mem::take(&mut self.back);
        stack.push((owner, id.into()));
        self.navigate(page, None, w, cx);
        self.back = stack;
        w.focus(&self.focus_for("back", cx), cx);
    }
    fn open_editor(&mut self, destination: Destination, id: &str, cx: &mut Context<Self>) {
        if self.model.pending {
            return;
        }
        self.editor_return = Some(id.into());
        self.request(RequestKind::Action(Action::Open(destination)), cx);
    }
    /// The control that opened the editor now on screen, and the Back stack of
    /// the page it was opened from. Main takes both before the editor replaces
    /// this page, which clears them, so Back can return to exactly that page.
    pub fn take_editor_return(&mut self) -> (Option<String>, Vec<(Page, String)>) {
        (self.editor_return.take(), std::mem::take(&mut self.back))
    }
    /// Puts the page's Back stack back and focuses the opener once this page
    /// has its snapshot again after the editor closes.
    pub fn return_from_editor(
        &mut self,
        opener: Option<String>,
        back: Vec<(Page, String)>,
        cx: &mut Context<Self>,
    ) {
        self.back = back;
        self.returned_opener = opener;
        cx.notify();
    }
    /// Opens `page` and reveals the control named `target` once the page has
    /// its snapshot, opening Advanced when the control lives there.
    pub fn navigate_to_control(
        &mut self,
        page: Page,
        target: &'static str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(page, None, w, cx);
        self.pending_target = Some((page, target));
        cx.notify();
    }
    /// Owning 3.5e sheet calls this only when it closes; Settings never steals
    /// focus from an active editor on its request acknowledgment.
    pub fn editor_closed(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.editor_return.take() {
            w.focus(&self.focus_for(id, cx), cx);
        }
    }
    fn search_result(&mut self, row: Row, w: &mut Window, cx: &mut Context<Self>) {
        self.navigate(row.page, None, w, cx);
        self.reveal_row(row, w, cx);
    }
    /// Scrolls to a row, opens Advanced when it lives there, and focuses it.
    fn reveal_row(&mut self, row: Row, w: &mut Window, cx: &mut Context<Self>) {
        self.reveal = Some(row.reveal(self.model.settings()));
        self.reveal_scroll = true;
        if row.advanced && !self.model.advanced() {
            if let Some(snapshot) = self.model.snapshot.as_mut() {
                snapshot.disclosure["panes"][row.page.id()] = json!(true);
            }
            self.request(
                RequestKind::Disclosure {
                    page: row.page,
                    open: true,
                },
                cx,
            );
        }
        let key = row.reveal(self.model.settings());
        if let Some(target) = ROWS.iter().find(|r| r.key == key) {
            match self.control_focus(*target, cx) {
                Some(f) => w.focus(&f, cx),
                // An Advanced row exists only after the next draw.
                None => self.reveal_focus = Some(*target),
            }
        }
        cx.notify();
    }
    fn search_link(&mut self, link: SearchLink, w: &mut Window, cx: &mut Context<Self>) {
        self.navigate(link.page, None, w, cx);
        if matches!(
            link.id,
            "calibrate" | "reset-statistics" | "app-overrides" | "commands" | "project-scan"
        ) && !self.model.advanced()
        {
            if let Some(snapshot) = self.model.snapshot.as_mut() {
                snapshot.disclosure["panes"][link.page.id()] = json!(true);
            }
            self.request(
                RequestKind::Disclosure {
                    page: link.page,
                    open: true,
                },
                cx,
            );
        }
        if link.id == "project-scan" && self.model.settings()["codeVocabEnabled"] != true {
            self.reveal = Some("codeVocabEnabled");
            self.reveal_scroll = true;
            w.focus(&self.focus_for("codeVocabEnabled", cx), cx);
        } else {
            self.restore_focus = Some(link.id);
        }
    }
    fn toggle_advanced(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let open = !self.model.advanced();
        if !open {
            let f = self.focus_for("advanced", cx);
            w.focus(&f, cx);
        }
        self.confirmation = None;
        if let Some(snapshot) = self.model.snapshot.as_mut() {
            snapshot.disclosure["panes"][self.model.page.id()] = json!(open);
        }
        self.request(
            RequestKind::Disclosure {
                page: self.model.page,
                open,
            },
            cx,
        );
    }
    fn preview_pending(&self) -> bool {
        self.model.pending
            && self.operation.as_ref().is_some_and(|(action, ticket)| {
                matches!(action, Action::Calibration { finish: false, .. })
                    && *ticket == self.model.ticket()
            })
    }
    fn action(&mut self, action: Action, destructive: bool, cx: &mut Context<Self>) {
        if action == Action::CancelCalibration && self.preview_pending() {
            // Cancellation supersedes the revocable preview. A delayed host
            // continuation must recheck its lease before moving the window.
            self.authority.revoke();
            self.model.generation += 1;
            self.model.pending = false;
            self.operation = None;
            self.authority.sync(&self.model);
        }
        if self.model.pending || self.model.snapshot.is_none() {
            return;
        }
        if destructive {
            let confirmed = self.confirmation.as_ref().is_some_and(|(a, t, at)| {
                *a == action && *t == self.model.ticket() && at.elapsed() < Duration::from_secs(4)
            });
            if !confirmed {
                self.confirmation = Some((action, self.model.ticket(), Instant::now()));
                cx.notify();
                return;
            }
        }
        self.confirmation = None;
        self.request(RequestKind::Action(action), cx);
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if e.prefer_character_input || e.is_held {
            return;
        }
        #[cfg(any(test, feature = "gallery"))]
        if self.fixture_capture_key(e, w, cx) {
            w.prevent_default();
            cx.stop_propagation();
            return;
        }
        use crate::palette::model::Shortcut;
        let k = &e.keystroke;
        if let Some(shortcut) = crate::palette::main_shortcut(e, w) {
            // The palette, transcript search and opening Settings belong to
            // the main window, which handles them after this page declines
            // the key. Opening Settings from Settings stays on this page.
            match shortcut {
                Shortcut::Logs => self.navigate(Page::Performance, None, w, cx),
                Shortcut::Settings | Shortcut::Palette | Shortcut::Search => return,
            }
            w.prevent_default();
            cx.stop_propagation();
        } else if k.key == "escape" && k.modifiers.number_of_modifiers() == 0 {
            if self
                .embedded
                .iter()
                .any(|(page, _)| *page == self.model.page)
                && self.model.query.is_empty()
                && self.confirmation.is_none()
                && self.calibration.is_none()
            {
                return;
            }
            if self.pickers.values().any(|p| p.read(cx).is_open()) {
                return;
            }
            if self.confirmation.take().is_some() {
                cx.notify();
            } else if self.calibration.is_some() {
                self.action(Action::CancelCalibration, false, cx);
            } else if !self.model.query.is_empty() {
                self.model.query.clear();
                self.search.update(cx, |s, cx| s.set_value("", w, cx));
            } else {
                self.go_back(w, cx);
            }
            w.prevent_default();
            cx.stop_propagation();
        }
    }
    fn action_button(
        &mut self,
        id: &str,
        label: &str,
        action: Action,
        destructive: bool,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let armed = self
            .confirmation
            .as_ref()
            .is_some_and(|(a, _, _)| *a == action);
        let e = cx.entity();
        Button::new(
            SharedString::from(id.to_string()),
            if armed {
                format!("Confirm {label}")
            } else {
                label.into()
            },
        )
        .with_scroll_parent(Some(&self.scroll))
        .track_focus(&self.focus_for(id, cx))
        .disabled(
            disabled
                || (self.model.pending
                    && !(action == Action::CancelCalibration && self.preview_pending()))
                || self.model.snapshot.is_none(),
        )
        .intent(if destructive {
            ButtonIntent::Danger
        } else {
            ButtonIntent::Secondary
        })
        .on_click(move |_, _, cx| e.update(cx, |s, cx| s.action(action.clone(), destructive, cx)))
    }
    fn editor_button(
        &mut self,
        id: &str,
        label: &str,
        destination: Destination,
        cx: &mut Context<Self>,
    ) -> Button {
        let e = cx.entity();
        let key = id.to_string();
        Button::new(SharedString::from(id.to_string()), label.to_string())
            .with_scroll_parent(Some(&self.scroll))
            .track_focus(&self.focus_for(id, cx))
            .disabled(self.model.pending || self.model.snapshot.is_none())
            .on_click(move |_, _, cx| e.update(cx, |s, cx| s.open_editor(destination, &key, cx)))
    }
    fn route_button(
        &mut self,
        id: &str,
        label: &str,
        page: Page,
        cx: &mut Context<Self>,
    ) -> Button {
        let e = cx.entity();
        let key = id.to_string();
        Button::new(SharedString::from(id.to_string()), label.to_string())
            .with_scroll_parent(Some(&self.scroll))
            .track_focus(&self.focus_for(id, cx))
            .on_click(move |_, w, cx| e.update(cx, |s, cx| s.open_detail(page, &key, w, cx)))
    }
    fn row(&mut self, row: Row, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::get(cx);
        let value = if row.key == "autoPaste" && self.model.runtime().delivery_isolated {
            json!(false)
        } else if row.key == "launchAtLogin" {
            self.model.runtime().login_enabled.map_or_else(
                || self.model.settings()[row.key].clone(),
                serde_json::Value::Bool,
            )
        } else {
            self.model.settings()[row.key].clone()
        };
        let disabled = self.model.disabled(row);
        let file = self.model.settings()["saveTranscript"] == true
            || self.model.settings()["saveAudio"] == true;
        let detail = if row.key == "autoPaste" && self.model.runtime().delivery_isolated {
            "Test build: paste stays inside Murmur"
        } else if row.key == "autoPaste" && file {
            if value == true {
                "Paused while file output is on; your preference resumes when file output is off"
            } else {
                "Unavailable while file output is on"
            }
        } else if row.key == "language" && self.model.runtime().english_only {
            "English only — use Whisper Large Turbo for others"
        } else {
            row.detail
        };
        let mut controls = div().flex().items_center().gap(px(8.));
        controls = match row.kind {
            Kind::Toggle => {
                let e = cx.entity();
                controls.child(
                    Toggle::new(row.key, row.label, value == true)
                        .disabled(disabled)
                        .with_scroll_parent(&self.scroll)
                        .track_focus(&self.focus_for(row.key, cx))
                        .on_change(move |v, w, cx| {
                            e.update(cx, |s, cx| s.edit(row, json!(v), w, cx))
                        }),
                )
            }
            Kind::Choice(_) => controls.child(self.pickers[row.key].clone()),
            Kind::Range(_, _, _, _) => {
                controls.child(div().w(px(200.)).child(self.ranges[row.key].clone()))
            }
            Kind::Number(_, _) => {
                let e = cx.entity();
                controls
                    .child(
                        div().w(px(120.)).child(
                            TextField::new(&self.numbers[row.key], row.label)
                                .with_scroll_parent(&self.scroll)
                                .disabled(disabled),
                        ),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("apply-{}", row.key)),
                            format!("Save {}", row.label),
                        )
                        .with_scroll_parent(Some(&self.scroll))
                        .caption("Save")
                        .size(ButtonSize::Compact)
                        .disabled(disabled)
                        .on_click(move |_, w, cx| {
                            e.update(cx, |s, cx| s.commit_number(row, w, cx))
                        }),
                    )
            }
            Kind::Folder => {
                let text = value
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Default folder");
                controls
                    .child(
                        div()
                            .max_w(px(200.))
                            .overflow_x_hidden()
                            .text_ellipsis()
                            .child(text.to_string()),
                    )
                    .child(self.action_button(
                        &format!("choose-{}", row.key),
                        "Choose folder…",
                        Action::ChooseFolder(row.key),
                        false,
                        disabled,
                        cx,
                    ))
            }
        };
        // Reserve reset space even when unchanged; reset uses the exact same owner.
        let default = murmur_core::state::settings_defaults()[row.key].clone();
        let mut reset = div().w(px(28.));
        if default != value {
            let e = cx.entity();
            reset = reset.child(
                Button::new(
                    SharedString::from(format!("reset-{}", row.key)),
                    format!("Reset {} to default", row.label),
                )
                .with_scroll_parent(Some(&self.scroll))
                .caption("↶")
                .size(ButtonSize::ExtraSmall)
                .disabled(disabled)
                .on_click(move |_, w, cx| {
                    e.update(cx, |s, cx| s.edit(row, default.clone(), w, cx))
                }),
            );
        }
        let copy = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().font_weight(FontWeight::MEDIUM).child(row.label))
            .when(!detail.is_empty(), |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("{}-description", row.key)))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(p.muted)
                        .when(
                            row.key == "autoPaste" && self.model.runtime().delivery_isolated,
                            |d| d.role(Role::Label).aria_label(detail),
                        )
                        .child(detail),
                )
            });
        div()
            .id(row.key)
            .w_full()
            .px(px(16.))
            .py(px(12.))
            .border_b_1()
            .border_color(p.hairline)
            .flex()
            .when(compact, |d| d.flex_col())
            .gap(px(if compact { 12. } else { 32. }))
            .when(!compact, |d| d.items_center())
            .when(self.reveal == Some(row.key), |d| {
                d.anchor_scroll(Some(self.reveal_anchor.clone()))
                    .border_l_2()
                    .border_color(p.focus)
            })
            .child(copy)
            .child(if self.model.page == Page::General {
                controls
            } else {
                controls.child(reset)
            })
            .into_any_element()
    }
}
impl Drop for Settings {
    fn drop(&mut self) {
        // The host admits and retains the final user intent synchronously, independently
        // of this presentation lifetime. A closed inbox suppresses only its UI reply.
        if self.model.attached && !self.model.pending {
            for (&key, draft) in &self.range_drafts {
                let Some(draft) = draft.get() else { continue };
                let value = json!(draft);
                if self.model.settings()[key] != value {
                    let ticket = self.model.ticket();
                    self.host.request(
                        Request {
                            ticket,
                            lease: self.authority.lease(ticket),
                            kind: RequestKind::Patch { key, value },
                        },
                        self.inbox.clone(),
                    );
                }
            }
        }
        self.detach();
    }
}
impl Render for Settings {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain(cx);
        self.synchronize(w, cx);
        if let Some(id) = self.restore_focus.take() {
            w.focus(&self.focus_for(id, cx), cx);
        }
        if self.model.snapshot.is_some()
            && let Some(id) = self.returned_opener.take()
        {
            w.focus(&self.focus_for(id, cx), cx);
        }
        if self.model.snapshot.is_some()
            && !self.model.pending
            && let Some((page, target)) = self.pending_target.take()
            && self.model.page == page
            && let Some(&row) = ROWS.iter().find(|r| r.page == page && r.target == target)
        {
            self.reveal_row(row, w, cx);
        }
        let p = Palette::get(cx);
        let compact = w.viewport_size().width < px(768.);
        if self.was_compact != Some(compact) {
            if compact && self.navigation.read(cx).focus_handle(cx).is_focused(w) {
                w.focus(
                    &self.focus_for(format!("nav-{}", self.model.page.id()), cx),
                    cx,
                );
            }
            if compact && self.search.read(cx).focus_handle(cx).is_focused(w) {
                self.compact_search = true;
            }
            if !compact
                && self
                    .focus
                    .iter()
                    .any(|(id, f)| id.starts_with("nav-") && f.is_focused(w))
            {
                w.focus(&self.navigation.read(cx).focus_handle(cx), cx);
            }
            self.was_compact = Some(compact);
        }
        let narrow = w.viewport_size().width < px(768.);
        self.navigation
            .update(cx, |s, cx| s.set_width(px(224.), cx));
        let picker_width =
            (w.viewport_size().width - px(if compact { 56. } else { 240. }) - px(120.))
                .min(px(200.));
        for picker in self.pickers.values() {
            picker.update(cx, |s, cx| s.set_width(picker_width, cx));
        }
        let mut content = div()
            .id("settings-content")
            .role(Role::TabPanel)
            .max_w(px(896.))
            .aria_label(self.model.page.label())
            .flex_1()
            .min_w(px(0.))
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(24.))
            .pb(px(48.))
            .flex()
            .flex_col()
            .gap(px(12.));
        if let Some((page, _)) = self.back.last() {
            let e = cx.entity();
            content = content.child(
                Button::new("back", format!("Back to {}", page.label()))
                    .with_scroll_parent(Some(&self.scroll))
                    .track_focus(&self.focus_for("back", cx))
                    .on_click(move |_, w, cx| e.update(cx, |s, cx| s.go_back(w, cx))),
            );
        }
        if let Some(problem) = self.model.problem {
            let e = cx.entity();
            content = content
                .child(
                    div()
                        .id("settings-problem")
                        .role(Role::Alert)
                        .aria_label(problem.message())
                        .text_color(p.danger)
                        .child(problem.message()),
                )
                .child(
                    Button::new("reload", "Reload settings")
                        .with_scroll_parent(Some(&self.scroll))
                        .disabled(self.model.pending)
                        .on_click(move |_, _, cx| {
                            e.update(cx, |s, cx| s.request(RequestKind::Read, cx))
                        }),
                );
        }
        if self.model.snapshot.is_none() {
            content = content.child(
                div()
                    .id("settings-loading")
                    .role(Role::Status)
                    .aria_label("Loading settings…")
                    .child("Loading settings…"),
            );
        }
        if !self.model.query.is_empty() {
            let q = self.model.query.to_lowercase();
            let matches: Vec<_> = ROWS
                .iter()
                .copied()
                .filter(|row| {
                    format!(
                        "{} {} {} {}",
                        row.label,
                        row.detail,
                        row.target,
                        row.page.label()
                    )
                    .to_lowercase()
                    .contains(&q)
                })
                .collect();
            let link_matches: Vec<_> = LINKS
                .iter()
                .copied()
                .filter(|l| {
                    format!("{} {}", l.label, l.keywords)
                        .to_lowercase()
                        .contains(&q)
                })
                .collect();
            if matches.is_empty() && link_matches.is_empty() {
                content = content.child("No settings match your search");
            }
            for row in matches {
                let e = cx.entity();
                content = content.child(
                    Button::new(row.key, format!("{} · {}", row.label, row.page.label()))
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| {
                            e.update(cx, |s, cx| s.search_result(row, w, cx))
                        }),
                );
            }
            for link in link_matches {
                let e = cx.entity();
                content = content.child(
                    Button::new(link.id, format!("{} · {}", link.label, link.page.label()))
                        .with_scroll_parent(Some(&self.scroll))
                        .on_click(move |_, w, cx| e.update(cx, |s, cx| s.search_link(link, w, cx))),
                );
            }
        } else if let Some((_, view)) = self
            .embedded
            .iter()
            .find(|(page, _)| *page == self.model.page)
        {
            content = content.child(view.clone());
        } else if self.model.page == Page::General {
            content = content.child(self.general(w, cx));
        } else {
            let settings = self.model.settings().clone();
            let runtime = self.model.runtime();
            let mut rows = div()
                .rounded(tokens::CARD_RADIUS)
                .border_1()
                .border_color(p.hairline_strong)
                .bg(p.tint_raised)
                .flex()
                .flex_col();
            let mut has_rows = false;
            for &row in ROWS {
                if row.page != self.model.page
                    || row.advanced && !self.model.advanced()
                    || !row.visible(&settings)
                    || row.key == "launchAtLogin" && runtime.internal_build
                    || row.key == "mirrorToNotchPill" && runtime.notchpill == Some(false)
                    || row.key == "meetingDiarization" && runtime.speakers_supported == Some(false)
                    || row.key == "outputDir"
                        && settings["saveAudio"] != true
                        && settings["saveTranscript"] != true
                {
                    continue;
                }
                has_rows = true;
                rows = rows.child(self.row(row, narrow, cx));
            }
            if has_rows {
                content = content.child(rows);
            }
            if self.model.page == Page::General && self.model.runtime().login_requires_approval {
                content = content
                    .child(gpui::text!(
                        "Launch at login needs your approval in System Settings."
                    ))
                    .child(self.action_button(
                        "login-approval",
                        "Open Login Items…",
                        Action::LoginSettings,
                        false,
                        false,
                        cx,
                    ));
            }
            if self.model.page.has_advanced() {
                let e = cx.entity();
                content = content.child(
                    Button::new(
                        "advanced",
                        if self.model.advanced() {
                            "Hide Advanced"
                        } else {
                            "Advanced"
                        },
                    )
                    .with_scroll_parent(Some(&self.scroll))
                    .track_focus(&self.focus_for("advanced", cx))
                    .toggled(self.model.advanced())
                    .intent(ButtonIntent::Ghost)
                    .disabled(self.model.pending || self.model.snapshot.is_none())
                    .on_click(move |_, w, cx| e.update(cx, |s, cx| s.toggle_advanced(w, cx))),
                );
            }
            content = content.children(self.extra(w, cx));
        }
        let mut nav = div()
            .id("settings-nav")
            .w(px(if compact { 56. } else { 240. }))
            .h_full()
            .bg(p.surface_container_low)
            .p(px(8.))
            .pt(px(48.))
            .relative()
            .flex()
            .flex_col()
            .gap(px(4.));
        if !compact {
            nav = nav.child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left(px(80.))
                    .flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .text_color(p.accent)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Murmur"),
                    )
                    .child(
                        div()
                            .text_size(tokens::CAPTION_SIZE)
                            .text_color(p.muted)
                            .child("Settings"),
                    ),
            );
        }
        if compact {
            let e = cx.entity();
            nav = nav.child(
                Button::new("open-search", "Search settings")
                    .with_scroll_parent(Some(&self.nav_scroll))
                    .icon_only(crate::icons::Glyph::Search)
                    .on_click(move |_, w, cx| {
                        e.update(cx, |s, cx| {
                            s.compact_search = true;
                            w.focus(&s.search.read(cx).focus_handle(cx), cx);
                            cx.notify();
                        })
                    }),
            );
            for page in Page::NAV {
                let e = cx.entity();
                nav = nav.child(
                    Button::new(
                        SharedString::from(format!("nav-{}", page.id())),
                        page.label(),
                    )
                    .with_scroll_parent(Some(&self.nav_scroll))
                    .icon_only(crate::icons::settings_glyph(page.label()))
                    .toggled(self.model.page == page)
                    .track_focus(&self.focus_for(format!("nav-{}", page.id()), cx))
                    .on_click(move |_, w, cx| e.update(cx, |s, cx| s.navigate(page, None, w, cx))),
                );
            }
            if self.compact_search || !self.model.query.is_empty() {
                content = div()
                    .id("compact-content")
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        TextField::new(&self.search, "Search settings")
                            .search_chrome()
                            .search_radius(px(6.)),
                    )
                    .child(content);
            }
        } else {
            nav = nav
                .child(
                    TextField::new(&self.search, "Search settings")
                        .search_chrome()
                        .search_radius(px(6.))
                        .with_scroll_parent(&self.nav_scroll),
                )
                .child(
                    div()
                        .id("settings-nav-scroll")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.nav_scroll)
                        .child(self.navigation.clone()),
                );
        }

        if self.reveal_scroll {
            self.reveal_anchor.scroll_to(w, cx);
            self.reveal_scroll = false;
        }
        if let Some(row) = self.reveal_focus
            && let Some(f) = self.control_focus(row, cx)
        {
            w.focus(&f, cx);
            self.reveal_focus = None;
        }
        div()
            .id("settings")
            .relative()
            .capture_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(nav)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(43.))
                            .flex_shrink_0()
                            .flex()
                            .justify_end()
                            .items_center()
                            .px(px(12.)),
                    )
                    .child(
                        div()
                            .px(px(24.))
                            .pt(px(12.))
                            .pb(px(10.))
                            .border_b_1()
                            .border_color(p.hairline)
                            .child(
                                div()
                                    .id("settings-title")
                                    .role(Role::Heading)
                                    .aria_label(if self.model.page == Page::Customize {
                                        "Customize Murmur"
                                    } else {
                                        self.model.page.label()
                                    })
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if self.model.page == Page::Customize {
                                        "Customize Murmur"
                                    } else {
                                        self.model.page.label()
                                    }),
                            ),
                    )
                    .child(content),
            )
            .child(
                div().absolute().top(px(4.)).right(px(12.)).child(
                    Button::new("settings-done", "Done")
                        .pill()
                        .on_click(cx.listener(|_, _, _, cx| crate::events::publish(cx, Done))),
                ),
            )
    }
}
