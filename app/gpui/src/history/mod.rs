//! Native-parity History workspace, hosted inside Root. No production I/O.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent, ButtonSize, Choice, ChoiceKind, Choices, SelectionChanged, TextField,
    Toggle,
    data_components::Progress,
    field::{InputEvent, TextFieldState},
    icons::Glyph,
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use gpui_base::ElementExt as _;
use host::{Authority, Host, Request, RequestKind};
use inbox::Inbox;
use model::*;
use std::{cell::Cell, collections::BTreeMap, ops::Range, rc::Rc, time::Instant};

actions!(history, [NextRowAction, PreviousRowAction]);
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextRowAction, Some("HistoryList")),
        KeyBinding::new("shift-tab", PreviousRowAction, Some("HistoryList")),
    ]);
}

// The list callback only constructs visible rows. Detail pages independently
// bound shaping/AX to 2048 scalars; a megabyte transcript never becomes one label.
const DETAIL_PAGE: usize = 2048;
const PREVIEW_CHARS: usize = 2048;
const ROW_HEIGHT_HINT: Pixels = px(64.);
struct HistoryScroll {
    state: ListState,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}
impl HistoryScroll {
    fn new() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(80.)),
            bounds: Default::default(),
        }
    }
    fn scroll_to_item(&self, item_ix: usize, strategy: ScrollStrategy) {
        let viewport = self.bounds.get();
        if strategy != ScrollStrategy::Top
            && let Some(row) = self.state.bounds_for_item(item_ix)
        {
            if row.top() >= viewport.top() && row.bottom() <= viewport.bottom() {
                return;
            }
            self.state.scroll_to_reveal_item(item_ix);
        } else {
            // Anchor unmeasured destinations directly; estimated preceding heights
            // must not strand End/reverse-Tab before the requested transcript.
            self.state.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
    }
    #[cfg(test)]
    fn assert_visible(&self, item: usize) {
        let row = self
            .state
            .bounds_for_item(item)
            .expect("focused row must be measured");
        let viewport = self.bounds.get();
        assert!(
            row.top() >= viewport.top() - px(1.),
            "row {item} above viewport: {row:?} / {viewport:?}"
        );
        assert!(
            row.bottom() <= viewport.bottom() + px(1.),
            "row {item} below viewport: {row:?} / {viewport:?}"
        );
    }
}
/// Only byte offsets are retained; the original entry/JSON is always borrowed.
#[derive(Default)]
struct DetailPages {
    identity: Option<(u64, u64)>,
    boundaries: Vec<usize>,
    #[cfg(any(test, feature = "gallery"))]
    scanned_bytes: usize,
    #[cfg(any(test, feature = "gallery"))]
    shaped_bytes: usize,
}
impl DetailPages {
    fn ensure(&mut self, entry: &Entry, revision: u64) {
        let identity = (entry.key, revision);
        if self.identity == Some(identity) {
            return;
        }
        self.boundaries.clear();
        self.boundaries.push(0);
        for (scalar, (byte, _)) in entry.text.char_indices().enumerate() {
            if scalar > 0 && scalar.is_multiple_of(DETAIL_PAGE) {
                self.boundaries.push(byte);
            }
        }
        self.boundaries.push(entry.text.len());
        self.identity = Some(identity);
        #[cfg(any(test, feature = "gallery"))]
        {
            self.scanned_bytes += entry.text.len();
        }
    }
}
pub struct History {
    /// Heading supplied by a containing workspace; data ownership is unchanged.
    pub title: &'static str,
    pub model: Model,
    host: Rc<dyn Host>,
    inbox: Inbox,
    authority: Authority,
    subscription: Option<Box<dyn host::Subscription>>,
    subscriptions: Vec<gpui::Subscription>,
    pub search: Entity<TextFieldState>,
    source: Entity<Choices>,
    date: Entity<Choices>,
    actions: Entity<Choices>,
    detail_actions: Entity<Choices>,
    copied_key: Option<u64>,
    embedded: bool,
    focus: FocusHandle,
    filter_focus: FocusHandle,
    back_focus: FocusHandle,
    detail_focus: FocusHandle,
    detail_scroll: ScrollHandle,
    detail_pages: DetailPages,
    scroll: HistoryScroll,
    row_focus: BTreeMap<u64, FocusHandle>,
    row_copy_focus: BTreeMap<u64, FocusHandle>,
    row_reveal: BTreeMap<u64, bool>,
    list_identity: Vec<u64>,
    list_revision: u64,
    list_end_focus: FocusHandle,
    #[cfg(any(test, feature = "gallery"))]
    pub fixture_clock: Option<Clock>,
    #[cfg(any(test, feature = "gallery"))]
    pub fixture_zone: Option<chrono_tz::Tz>,
    filters_open: bool,
    detail_page: usize,
    pub rendered: Range<usize>,
    pub fixture: bool,
    notice_epoch: u64,
    announced_epoch: u64,
}
impl History {
    pub fn new(host: Rc<dyn Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextFieldState::new(w, cx).placeholder("Search"));
        let source = cx.new(|cx| {
            Choices::new(
                "Show transcripts",
                ChoiceKind::Dropdown,
                ["Everything", "Spoken", "Files"]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| Choice::new(i.to_string(), *l))
                    .collect(),
                w,
                cx,
            )
        });
        let date = cx.new(|cx| {
            Choices::new(
                "From date",
                ChoiceKind::Dropdown,
                ["Any time", "Today", "Past 7 days", "Past 30 days"]
                    .iter()
                    .enumerate()
                    .map(|(i, l)| Choice::new(i.to_string(), *l))
                    .collect(),
                w,
                cx,
            )
        });
        source.update(cx, |s, cx| s.set_value("0", cx));
        date.update(cx, |s, cx| s.set_value("0", cx));
        let actions =
            cx.new(|cx| Choices::new("More history actions", ChoiceKind::Dropdown, vec![], w, cx));
        actions.update(cx, |a, cx| a.set_trigger_icon(Glyph::More, cx));
        let detail_actions = cx.new(|cx| {
            Choices::new(
                "More transcript actions",
                ChoiceKind::Dropdown,
                vec![
                    Choice::new("markdown", "Copy as Markdown"),
                    Choice::new("pin", "Pin / unpin transcript"),
                    Choice::new("delete", "Delete transcript…"),
                ],
                w,
                cx,
            )
        });
        detail_actions.update(cx, |a, cx| a.set_trigger_icon(Glyph::More, cx));
        let mut s = Self {
            title: "Your dictations",
            detail_actions: detail_actions.clone(),
            copied_key: None,
            embedded: false,
            model: Model::default(),
            host,
            inbox: Inbox::default(),
            authority: Authority::default(),
            subscription: None,
            subscriptions: vec![],
            search: search.clone(),
            source: source.clone(),
            date: date.clone(),
            actions: actions.clone(),
            focus: cx.focus_handle().tab_stop(true),
            filter_focus: cx.focus_handle(),
            back_focus: cx.focus_handle(),
            detail_focus: cx.focus_handle().tab_stop(true),
            detail_scroll: ScrollHandle::new(),
            detail_pages: DetailPages::default(),
            scroll: HistoryScroll::new(),
            list_identity: vec![],
            list_revision: 0,
            row_focus: BTreeMap::new(),
            row_copy_focus: BTreeMap::new(),
            row_reveal: BTreeMap::new(),
            list_end_focus: cx.focus_handle().tab_stop(true),
            #[cfg(any(test, feature = "gallery"))]
            fixture_clock: None,
            #[cfg(any(test, feature = "gallery"))]
            fixture_zone: None,
            filters_open: false,
            detail_page: 0,
            rendered: 0..0,
            fixture: false,
            notice_epoch: 0,
            announced_epoch: 0,
        };
        s.subscriptions
            .push(cx.subscribe(&search, |s, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let mut q = s.model.query.clone();
                    q.text = s.search.read(cx).value().to_string();
                    s.query(q, cx);
                }
            }));
        s.subscriptions
            .push(cx.subscribe(&source, |s, _, e: &SelectionChanged, cx| {
                let mut q = s.model.query.clone();
                q.source = match e.0.as_ref() {
                    "1" => Source::Spoken,
                    "2" => Source::Files,
                    _ => Source::All,
                };
                s.query(q, cx);
            }));
        s.subscriptions
            .push(cx.subscribe(&date, |s, _, e: &SelectionChanged, cx| {
                let mut q = s.model.query.clone();
                q.date = match e.0.as_ref() {
                    "1" => Date::Today,
                    "2" => Date::Week,
                    "3" => Date::Month,
                    _ => Date::All,
                };
                s.query(q, cx);
            }));
        s.subscriptions.push(
            cx.subscribe_in(&actions, w, |s, _, e: &SelectionChanged, w, cx| {
                s.action(&e.0, w, cx)
            }),
        );
        s.subscriptions.push(cx.subscribe_in(
            &detail_actions,
            w,
            |s, _, e: &SelectionChanged, w, cx| {
                let Some(entry) = s
                    .model
                    .entries
                    .iter()
                    .find(|e| Some(e.key) == s.model.detail)
                    .cloned()
                else {
                    return;
                };
                match e.0.as_ref() {
                    "markdown" => s.copy(entry.key, true),
                    "pin"
                        if entry.pinned
                            || s.model.entries.iter().filter(|e| e.pinned).count() < MAX_PINS =>
                    {
                        s.request(RequestKind::Pin { entry })
                    }
                    "delete" => s.delete(DeleteScope::Row(entry.key), w, cx),
                    _ => (),
                }
                cx.notify();
            },
        ));
        s
    }
    pub fn set_embedded(&mut self) {
        self.embedded = true;
    }
    fn date_parts(&self, ms: i64) -> (String, String) {
        let Some(date) = chrono::DateTime::from_timestamp_millis(ms) else {
            return ("Unavailable date".into(), "—".into());
        };
        #[cfg(any(test, feature = "gallery"))]
        if let Some(zone) = self.fixture_zone {
            let date = date.with_timezone(&zone);
            return (
                date.format("%Y-%m-%d").to_string(),
                date.format("%-I:%M %p").to_string(),
            );
        }
        let date = date.with_timezone(&chrono::Local);
        (
            date.format("%Y-%m-%d").to_string(),
            date.format("%-I:%M %p").to_string(),
        )
    }
    fn day_label(&self, ms: i64) -> String {
        let day = self.date_parts(ms).0;
        if day == self.date_parts(self.model.clock.today).0 {
            "Today".into()
        } else if Some(day.as_str())
            == chrono::NaiveDate::parse_from_str(
                &self.date_parts(self.model.clock.today).0,
                "%Y-%m-%d",
            )
            .ok()
            .and_then(|d| d.pred_opt())
            .map(|d| d.to_string())
            .as_deref()
        {
            "Yesterday".into()
        } else {
            chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                .map(|d| d.format("%b %-d, %Y").to_string())
                .unwrap_or(day)
        }
    }
    fn refresh_clock(&mut self) {
        #[cfg(any(test, feature = "gallery"))]
        if let Some(clock) = self.fixture_clock {
            self.model.set_clock(clock);
            return;
        }
        self.model.set_clock(local_clock());
    }
    fn timestamp(&self, ms: i64) -> String {
        #[cfg(any(test, feature = "gallery"))]
        if let Some(zone) = self.fixture_zone {
            return chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .with_timezone(&zone)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string();
        }
        stamp(ms)
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.subscription.is_some() {
            return;
        }
        self.model.attach();
        self.refresh_clock();
        self.inbox = Inbox::default();
        self.authority.sync(&self.model);
        self.subscription = Some(self.host.subscribe(
            murmur_core::events::Consumer::Main,
            self.model.ticket().attachment,
            self.inbox.clone(),
        ));
        self.drain(cx);
        self.request(RequestKind::Read);
        self.prepare(cx);
    }
    fn detach(&mut self) {
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut s) = self.subscription.take() {
            s.stop();
        }
        self.model.detach();
        self.row_focus.clear();
        self.row_copy_focus.clear();
        self.row_reveal.clear();
        self.filters_open = false;
        self.detail_page = 0;
        self.detail_pages = DetailPages::default();
        self.rendered = 0..0;
    }
    /// Clears editor search storage as well as transcript allocations on hide.
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.detach();
        self.source.update(cx, |s, cx| s.set_value("0", cx));
        self.date.update(cx, |s, cx| s.set_value("0", cx));
        self.search.update(cx, |s, cx| s.set_value("", w, cx));
        cx.notify();
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        let attachment = self.model.ticket().attachment;
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(32))
                    .await;
                if !weak
                    .update(cx, |s, cx| {
                        if s.subscription.is_none()
                            || s.model.ticket().attachment != attachment
                            || s.inbox.is_closed()
                        {
                            return false;
                        }
                        let old = s.model.ticket();
                        s.refresh_clock();
                        let armed = s.model.confirmation.is_some();
                        s.model.expire(Instant::now());
                        s.drain(cx);
                        s.authority.sync(&s.model);
                        if old != s.model.ticket() || armed != s.model.confirmation.is_some() {
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
    pub fn receive(&mut self, e: Event, cx: &mut Context<Self>) {
        self.drain(cx);
        self.accept(e, cx);
        self.prepare(cx);
    }
    fn accept(&mut self, e: Event, cx: &mut Context<Self>) -> bool {
        let accepted = self.model.event(e);
        self.authority.sync(&self.model);
        if accepted && self.model.notice.is_some() {
            self.notice_epoch += 1;
            let epoch = self.notice_epoch;
            cx.spawn(async move |weak, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let _ = weak.update(cx, |s, cx| {
                    if s.notice_epoch == epoch {
                        s.model.notice = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        accepted
    }
    pub fn drain(&mut self, cx: &mut Context<Self>) {
        let events = self.inbox.drain();
        if events.is_empty() {
            return;
        }
        let mut retry = false;
        for e in events {
            let stale = matches!(&e,Event::Read{ticket,..} if ticket.attachment==self.model.ticket().attachment);
            if !self.accept(e, cx) && stale {
                retry = true;
            }
        }
        if retry && self.subscription.is_some() {
            self.request(RequestKind::Read);
        }
        self.prepare(cx);
    }
    fn request(&mut self, kind: RequestKind) {
        if self.subscription.is_none() {
            return;
        }
        let ticket = self.model.begin();
        self.authority.sync(&self.model);
        self.host.request(
            Request {
                ticket,
                lease: self.authority.lease(ticket),
                kind,
            },
            self.inbox.clone(),
        );
    }
    fn query(&mut self, q: Query, cx: &mut Context<Self>) {
        self.model.set_query(q);
        self.authority.sync(&self.model);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.prepare(cx);
    }
    #[cfg(any(test, feature = "gallery"))]
    pub fn last_detail_page(&mut self) {
        self.detail_page = usize::MAX;
    }
    #[cfg(any(test, feature = "gallery"))]
    pub fn cache_counters(&self) -> (usize, usize, usize) {
        (
            self.detail_pages.scanned_bytes,
            self.detail_pages.shaped_bytes,
            self.detail_pages.boundaries.len(),
        )
    }
    pub fn prepare(&mut self, cx: &mut Context<Self>) {
        self.model.refresh();
        self.row_focus
            .retain(|key, _| self.model.entries.iter().any(|e| e.key == *key));
        self.row_copy_focus
            .retain(|key, _| self.row_focus.contains_key(key));
        self.row_reveal
            .retain(|key, _| self.row_focus.contains_key(key));
        self.authority.sync(&self.model);
        let count = self.model.matches.len();
        let mut choices = vec![
            Choice::new("menu", "More history actions…"),
            Choice::new("import", "Transcribe audio or video file…"),
        ];
        for (save, verb) in [(false, "Copy"), (true, "Save")] {
            for (i, f) in Format::ALL.iter().enumerate() {
                choices.push(
                    Choice::new(
                        format!("{}-{i}", if save { "save" } else { "copy" }),
                        format!("{verb} {count} shown as {}", f.label()),
                    )
                    .disabled(count == 0),
                );
            }
        }
        if self.model.query.narrowed() {
            choices
                .push(Choice::new("shown", format!("Delete {count} shown…")).disabled(count == 0));
        }
        choices
            .push(Choice::new("clear", "Clear History…").disabled(self.model.entries.is_empty()));
        self.actions.update(cx, |s, cx| {
            s.replace_choices(choices, cx);
            s.set_value("menu", cx);
        });
        cx.notify();
    }
    fn action(&mut self, value: &str, w: &mut Window, cx: &mut Context<Self>) {
        if value == "clear" {
            self.delete(DeleteScope::All, w, cx);
        } else if value == "shown" {
            self.delete(DeleteScope::Shown, w, cx);
        } else if value == "import" {
            self.request(RequestKind::TranscribeFile);
        } else if let Some((verb, index)) = value.split_once('-')
            && let Ok(i) = index.parse::<usize>()
            && let Some(format) = Format::ALL.get(i)
        {
            let entries = self
                .model
                .matches
                .iter()
                .map(|&i| self.model.entries[i].clone())
                .collect();
            self.request(RequestKind::Export {
                entries,
                format: *format,
                save: verb == "save",
            });
        }
        cx.notify();
    }
    fn delete(&mut self, scope: DeleteScope, w: &mut Window, cx: &mut Context<Self>) {
        if self.model.arm(scope.clone(), Instant::now()) {
            let Some(c) = self.model.confirmation.take() else {
                return;
            };
            let entries = self
                .model
                .entries
                .iter()
                .filter(|e| c.keys.contains(&e.key))
                .cloned()
                .collect();
            self.request(RequestKind::Delete {
                entries,
                clear_all: scope == DeleteScope::All,
            });
            w.focus(&self.focus, cx);
        }
        cx.notify();
    }
    fn copy(&mut self, key: u64, markdown: bool) {
        if let Some(entry) = self.model.entries.iter().find(|e| e.key == key).cloned() {
            self.copied_key = Some(key);
            self.request(RequestKind::CopyRow { entry, markdown });
        }
    }
    fn open_detail(&mut self, key: u64, w: &mut Window, cx: &mut Context<Self>) {
        self.model.invalidate();
        self.authority.sync(&self.model);
        self.model.selected = Some(key);
        self.model.detail = Some(key);
        self.detail_page = 0;
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        w.focus(&self.back_focus, cx);
        cx.notify();
    }
    fn back(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.model.invalidate();
        self.authority.sync(&self.model);
        self.model.detail = None;
        if let Some(i) = self
            .model
            .matches
            .iter()
            .position(|&i| Some(self.model.entries[i].key) == self.model.selected)
        {
            self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        }
        w.focus(&self.focus, cx);
        cx.notify();
    }
    fn reset(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.set_value("", w, cx));
        self.source.update(cx, |s, cx| s.set_value("0", cx));
        self.date.update(cx, |s, cx| s.set_value("0", cx));
        self.query(Query::default(), cx);
        w.focus(&self.search.read(cx).focus_handle(cx), cx);
    }
    /// Main-workspace shortcut entry; never focuses a search hidden by detail.
    pub fn focus_search(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.model.detail.is_some() {
            self.back(w, cx);
        }
        w.focus(&self.search.read(cx).focus_handle(cx), cx);
    }
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if e.prefer_character_input {
            return;
        }
        let slash = e.keystroke.key == "/"
            && e.keystroke.modifiers.number_of_modifiers() == 0
            && !e.is_held
            && !self.search.read(cx).focus_handle(cx).is_focused(w);
        if slash || (e.keystroke.modifiers.platform && e.keystroke.key == "f") {
            self.focus_search(w, cx);
        } else if e.keystroke.key == "escape" {
            // Open shared pickers own Escape and their focus/selection restore.
            if [
                &self.source,
                &self.date,
                &self.actions,
                &self.detail_actions,
            ]
            .iter()
            .any(|picker| picker.read(cx).is_open())
            {
                return;
            }
            if self.model.confirmation.take().is_some() {
            } else if self.model.detail.is_some() {
                self.back(w, cx);
            } else if self.filters_open {
                self.filters_open = false;
                w.focus(&self.filter_focus, cx);
            } else {
                self.search.update(cx, |s, cx| s.set_value("", w, cx));
                let mut q = self.model.query.clone();
                q.text.clear();
                self.query(q, cx);
                w.focus(&self.focus, cx);
            }
        } else {
            return;
        }
        w.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
    fn row_action_focus(&mut self, key: u64, cx: &mut Context<Self>) -> FocusHandle {
        self.row_reveal.entry(key).or_default();
        self.row_focus
            .entry(key)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }
    /// Handle the bound Tab action before Root traverses only painted stops.
    /// Returning false lets the outer Root binding own entry/exit traversal.
    fn tab_row_action(&mut self, backwards: bool, w: &mut Window, cx: &mut Context<Self>) -> bool {
        let copy_focused = self
            .row_copy_focus
            .iter()
            .find_map(|(key, focus)| focus.is_focused(w).then_some(*key));
        if let Some(key) = copy_focused
            && !backwards
        {
            let f = self.row_action_focus(key, cx);
            w.focus(&f, cx);
            cx.notify();
            return true;
        }
        let focused = copy_focused.or_else(|| {
            self.row_focus
                .iter()
                .find_map(|(key, focus)| focus.is_focused(w).then_some(*key))
        });
        if backwards
            && copy_focused.is_none()
            && let Some(key) = focused
        {
            let f = self
                .row_copy_focus
                .entry(key)
                .or_insert_with(|| cx.focus_handle())
                .clone();
            w.focus(&f, cx);
            cx.notify();
            return true;
        }
        let position = focused.and_then(|key| {
            self.model
                .matches
                .iter()
                .position(|&i| self.model.entries[i].key == key)
        });
        let next = match (position, backwards) {
            (Some(0), true) => {
                w.focus(&self.focus, cx);
                return true;
            }
            (Some(i), true) => Some(i - 1),
            (Some(i), false) if i + 1 < self.model.matches.len() => Some(i + 1),
            (Some(_), false) => {
                // Skip the reverse-entry boundary without adding a Tab press.
                w.focus(&self.list_end_focus, cx);
                w.focus_next(cx);
                return true;
            }
            (None, false) if self.focus.is_focused(w) && !self.model.matches.is_empty() => Some(0),
            _ => None,
        };
        let Some(position) = next else {
            return false;
        };
        self.focus_row_action(position, w, cx);
        if !backwards {
            let key = self.model.entries[self.model.matches[position]].key;
            let f = self
                .row_copy_focus
                .entry(key)
                .or_insert_with(|| cx.focus_handle())
                .clone();
            w.focus(&f, cx);
        }
        true
    }
    fn focus_row_action(&mut self, position: usize, w: &mut Window, cx: &mut Context<Self>) {
        let key = self.model.entries[self.model.matches[position]].key;
        let focus = self.row_action_focus(key, cx);
        self.scroll
            .scroll_to_item(position, ScrollStrategy::Nearest);
        w.focus(&focus, cx);
        cx.notify();
    }
    fn list_key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if e.prefer_character_input
            || e.keystroke.modifiers.number_of_modifiers() != 0
            || self.model.matches.is_empty()
        {
            return;
        }
        if !self.focus.is_focused(w) {
            // A deferred uniform-list Button can expose the correct AX focus
            // without installing its automatic keyboard-click listener. Route
            // only this owned row action; never copy from a descendant key.
            if matches!(e.keystroke.key.as_str(), "space" | "enter")
                && !e.is_held
                && let Some(key) = self
                    .row_copy_focus
                    .iter()
                    .find_map(|(key, focus)| focus.is_focused(w).then_some(*key))
            {
                self.copy(key, false);
                w.prevent_default();
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if e.keystroke.key == "space"
                && !e.is_held
                && let Some(key) = self
                    .row_focus
                    .iter()
                    .find_map(|(key, focus)| focus.is_focused(w).then_some(*key))
            {
                self.open_detail(key, w, cx);
                w.prevent_default();
                cx.stop_propagation();
            }
            return;
        }
        let mut i = self
            .model
            .matches
            .iter()
            .position(|&i| Some(self.model.entries[i].key) == self.model.selected)
            .unwrap_or(0);
        match e.keystroke.key.as_str() {
            "down" => i = (i + 1).min(self.model.matches.len() - 1),
            "up" => i = i.saturating_sub(1),
            "home" => i = 0,
            "end" => i = self.model.matches.len() - 1,
            "pagedown" => i = (i + 5).min(self.model.matches.len() - 1),
            "pageup" => i = i.saturating_sub(5),
            "enter" | "space" if !e.is_held => {
                self.copy(self.model.entries[self.model.matches[i]].key, false)
            }
            "right" if !e.is_held => {
                self.open_detail(self.model.entries[self.model.matches[i]].key, w, cx)
            }
            _ => return,
        }
        self.model.selected = Some(self.model.entries[self.model.matches[i]].key);
        self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        w.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
    fn rows(
        &mut self,
        range: Range<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.rendered = if self.rendered.is_empty() {
            range.clone()
        } else {
            self.rendered.start.min(range.start)..self.rendered.end.max(range.end)
        };
        let p = Palette::get(cx);

        range
            .map(|position| {
                let key = self.model.entries[self.model.matches[position]].key;
                let action_focus = self.row_action_focus(key, cx);
                let copy_focus = self
                    .row_copy_focus
                    .entry(key)
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                let focused = action_focus.is_focused(w) || copy_focus.is_focused(w);
                let entered = focused && !self.row_reveal[&key];
                self.row_reveal.insert(key, focused);
                if entered {
                    let entity = cx.entity();
                    cx.defer(move |cx| {
                        entity.update(cx, |s, cx| {
                            if s.model
                                .matches
                                .get(position)
                                .is_some_and(|&i| s.model.entries[i].key == key)
                            {
                                s.scroll.scroll_to_item(position, ScrollStrategy::Nearest);
                                cx.notify();
                            }
                        })
                    });
                }
                let entry = &self.model.entries[self.model.matches[position]];
                let label = format!(
                    "Transcript {} of {} from {}. Enter or Space copies; Right opens details.",
                    position + 1,
                    self.model.matches.len(),
                    self.timestamp(entry.timestamp)
                );
                let preview = if entry.text.chars().take(PREVIEW_CHARS + 1).count() <= PREVIEW_CHARS
                {
                    entry.text.clone()
                } else {
                    let prefix: String = entry.text.chars().take(PREVIEW_CHARS).collect();
                    let end = prefix.rfind(char::is_whitespace).unwrap_or(prefix.len());
                    format!("{}…", prefix[..end].trim_end())
                };
                let text = highlighted(&preview, &self.model.query.text, p);
                let day = self.date_parts(entry.timestamp).0;
                let first = position == 0
                    || self
                        .date_parts(self.model.entries[self.model.matches[position - 1]].timestamp)
                        .0
                        != day;
                let last = position + 1 == self.model.matches.len()
                    || self
                        .date_parts(self.model.entries[self.model.matches[position + 1]].timestamp)
                        .0
                        != day;
                let copied = self.copied_key == Some(key) && self.model.notice == Some("Copied");
                let time = self.date_parts(entry.timestamp).1;
                let day_label = self.day_label(entry.timestamp);
                // Copy and detail are siblings. GPUI's automatic keyboard click
                // dispatch must never treat another focused button as card copy.
                let copy = div()
                    .id(("copy", key))
                    .role(Role::Button)
                    .aria_label(label)
                    .aria_value(preview)
                    .when(self.model.selected == Some(key), |d| {
                        d.aria_active_descendant()
                    })
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.model.selected = Some(key);
                        w.focus(&s.focus, cx);
                        s.copy(key, false);
                        cx.notify();
                    }))
                    .on_a11y_action(AccessibleAction::Click, {
                        let entity = cx.entity();
                        move |_, w, cx| {
                            entity.update(cx, |s, cx| {
                                s.model.selected = Some(key);
                                w.focus(&s.focus, cx);
                                s.copy(key, false);
                                cx.notify();
                            })
                        }
                    })
                    .child(
                        div()
                            .w(px(72.))
                            .flex_shrink_0()
                            .font_family(tokens::MONO_FONT)
                            .text_size(tokens::CAPTION_SIZE)
                            .text_color(p.muted)
                            .child(time),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_start()
                            .when(entry.file, |d| {
                                d.child(
                                    div()
                                        .self_start()
                                        .rounded_full()
                                        .px(px(6.))
                                        .text_size(tokens::MICRO_SIZE)
                                        .line_height(tokens::CAPTION_LEADING)
                                        .bg(p.accent.opacity(0.1))
                                        .text_color(p.accent)
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(
                                            crate::icons::icon(crate::icons::Glyph::File)
                                                .size(px(11.)),
                                        )
                                        .child(entry.source_label().to_owned()),
                                )
                            })
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(640.))
                                    .line_clamp(2)
                                    .text_ellipsis()
                                    .child(text),
                            ),
                    );
                div()
                    .id(("row", key))
                    .w_full()
                    .flex()
                    .flex_col()
                    .when(first, |d| {
                        d.child(
                            div()
                                .h(px(if position == 0 { 24. } else { 36. }))
                                .flex()
                                .items_end()
                                .pb(px(6.))
                                .flex_shrink_0()
                                .text_size(tokens::MICRO_SIZE)
                                .text_color(p.muted)
                                .child(day_label),
                        )
                    })
                    .child(
                        div()
                            .id(("card", key))
                            .w_full()
                            .group("history-row")
                            .role(Role::ListItem)
                            .aria_position_in_set(position + 1)
                            .aria_size_of_set(self.model.matches.len())
                            .overflow_hidden()
                            .pl(px(12.))
                            .pr(px(6.))
                            .py(px(10.))
                            .border_l_1()
                            .border_r_1()
                            .border_b_1()
                            .border_color(p.hairline_strong)
                            .when(first, |d| d.border_t_1().rounded_t(tokens::CONTROL_RADIUS))
                            .when(last, |d| d.rounded_b(tokens::CONTROL_RADIUS))
                            .when(entry.pinned, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .h_full()
                                        .w(px(2.))
                                        .bg(p.accent.opacity(0.5)),
                                )
                            })
                            .bg(if copied {
                                p.tint_raised.blend(p.success.opacity(0.08))
                            } else {
                                p.tint_raised
                            })
                            .flex()
                            .items_start()
                            .relative()
                            .child(copy)
                            .child(
                                div()
                                    .absolute()
                                    .right(px(6.))
                                    .top(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .bg(if copied {
                                        p.tint_raised.blend(p.success.opacity(0.08))
                                    } else {
                                        p.tint_raised
                                    })
                                    .opacity(
                                        if copied
                                            || action_focus.is_focused(w)
                                            || copy_focus.is_focused(w)
                                            || (self.model.selected == Some(key)
                                                && self.focus.is_focused(w))
                                        {
                                            1.
                                        } else {
                                            0.
                                        },
                                    )
                                    .group_hover("history-row", |d| d.opacity(1.))
                                    .when(copied, |d| {
                                        d.child(
                                            div()
                                                .text_size(tokens::CAPTION_SIZE)
                                                .text_color(p.muted)
                                                .child("Copied"),
                                        )
                                    })
                                    .child(
                                        Button::new(
                                            ("copy-action", key),
                                            if copied {
                                                format!("Copied transcript {}", position + 1)
                                            } else {
                                                format!("Copy transcript {}", position + 1)
                                            },
                                        )
                                        .icon_only(if copied { Glyph::Check } else { Glyph::Copy })
                                        .intent(ButtonIntent::Ghost)
                                        .size(ButtonSize::ExtraSmall)
                                        .track_focus(&copy_focus)
                                        .on_click(
                                            cx.listener(move |s, _, _, cx| {
                                                cx.stop_propagation();
                                                s.copy(key, false);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            ("detail", key),
                                            format!("Show more for transcript {}", position + 1),
                                        )
                                        .icon_only(Glyph::More)
                                        .intent(ButtonIntent::Ghost)
                                        .size(ButtonSize::ExtraSmall)
                                        .track_focus(&action_focus)
                                        .on_click(
                                            cx.listener(move |s, _, w, cx| {
                                                cx.stop_propagation();
                                                s.open_detail(key, w, cx);
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }
    fn change_detail_page(&mut self, page: usize, cx: &mut Context<Self>) {
        self.detail_page = page;
        self.detail_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    fn detail_key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        let m = e.keystroke.modifiers;
        if e.prefer_character_input || m.platform || m.control || m.alt {
            return;
        }
        let line = px(24.);
        // Leave one line of overlap between viewport pages, even for short lines.
        let viewport = (self.detail_scroll.bounds().size.height - line).max(line);
        let offset = self.detail_scroll.offset().y;
        let bottom = -self.detail_scroll.max_offset().y;
        let next = match e.keystroke.key.as_str() {
            "up" => offset + line,
            "down" => offset - line,
            "pageup" => offset + viewport,
            "pagedown" => offset - viewport,
            "space" if m.shift => offset + viewport,
            "space" => offset - viewport,
            "home" => px(0.),
            "end" => bottom,
            _ => return,
        };
        self.detail_scroll
            .set_offset(point(px(0.), next.clamp(bottom, px(0.))));
        w.prevent_default();
        cx.stop_propagation();
        cx.notify();
    }
    fn detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(entry) = self
            .model
            .entries
            .iter()
            .find(|e| Some(e.key) == self.model.detail)
        else {
            return div().into_any_element();
        };
        let key = entry.key;
        self.detail_actions.update(cx, |menu, cx| {
            menu.replace_choices(
                vec![
                    Choice::new("markdown", "Copy as Markdown"),
                    Choice::new(
                        "pin",
                        if entry.pinned {
                            "Unpin transcript"
                        } else {
                            "Pin transcript"
                        },
                    )
                    .disabled(
                        !entry.pinned
                            && self.model.entries.iter().filter(|e| e.pinned).count() >= MAX_PINS,
                    ),
                    Choice::new("delete", "Delete transcript…"),
                ],
                cx,
            )
        });
        self.detail_pages
            .ensure(entry, self.model.ticket().revision);
        let pages = self.detail_pages.boundaries.len() - 1;
        self.detail_page = self.detail_page.min(pages - 1);
        let page = &entry.text[self.detail_pages.boundaries[self.detail_page]
            ..self.detail_pages.boundaries[self.detail_page + 1]];
        #[cfg(any(test, feature = "gallery"))]
        {
            self.detail_pages.shaped_bytes += page.len();
        }
        let p = Palette::get(cx);
        let mut view = div()
            .id("detail")
            .role(Role::Group)
            .aria_label("Transcript detail")
            .relative()
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(8.))
            .child(
                Button::new("back", "Back to transcripts")
                    .caption("Back")
                    .icon(crate::icons::Glyph::ChevronLeft)
                    .intent(ButtonIntent::Ghost)
                    .track_focus(&self.back_focus)
                    .on_click(cx.listener(|s, _, w, cx| s.back(w, cx))),
            )
            .child(
                div()
                    .font_family(tokens::MONO_FONT)
                    .text_size(tokens::CAPTION_SIZE)
                    .text_color(p.muted)
                    .child(self.timestamp(entry.timestamp)),
            )
            .child(
                div()
                    .id("detail-text")
                    .role(Role::Label)
                    .aria_label(format!(
                        "Transcript text page {} of {pages}",
                        self.detail_page + 1
                    ))
                    .aria_value(page.to_owned())
                    .track_focus(&self.detail_focus)
                    .track_scroll(&self.detail_scroll)
                    .focus_visible(|d| d.border_1().border_color(p.focus))
                    .capture_key_down(cx.listener(Self::detail_key))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(highlighted(page, &self.model.query.text, p)),
            )
            .when(pages > 1, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            Button::new("previous", "Previous text page")
                                .disabled(self.detail_page == 0)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.change_detail_page(s.detail_page.saturating_sub(1), cx);
                                })),
                        )
                        .child(format!("Page {} of {pages}", self.detail_page + 1))
                        .child(
                            Button::new("next", "Next text page")
                                .disabled(self.detail_page + 1 >= pages)
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    s.change_detail_page((s.detail_page + 1).min(pages - 1), cx);
                                })),
                        ),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("copy-full", "Copy full transcript")
                            .icon_only(Glyph::Copy)
                            .intent(ButtonIntent::Ghost)
                            .on_click(cx.listener(move |s, _, _, _| s.copy(key, false))),
                    )
                    .child(self.detail_actions.clone()),
            );
        // Eligibility is the newest STORED entry, independent of sorting/filter.
        if self.model.entries.last().is_some_and(|e| e.key == key) {
            view = view.child(
                Button::new("teach", "Correct & teach…")
                    .intent(ButtonIntent::Ghost)
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(move |s, _, _, _| {
                        if let Some(entry) =
                            s.model.entries.last().filter(|e| e.key == key).cloned()
                        {
                            s.request(RequestKind::CorrectAndTeach { entry });
                        }
                    })),
            );
        }
        view.into_any_element()
    }
}
impl Drop for History {
    fn drop(&mut self) {
        self.detach();
    }
}
impl Render for History {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Root's tab order includes a stable boundary after the virtualized
        // list. Resolve it before building rows, so reverse entry scrolls and
        // focuses the real tail even without a window-activation notification.
        if self.list_end_focus.is_focused(w)
            && self.model.detail.is_none()
            && !self.model.loading
            && let Some(position) = self.model.matches.len().checked_sub(1)
        {
            self.focus_row_action(position, w, cx);
        }
        // Observe blur outside the virtualized rows. A focused row disappearing
        // is not a focus change; another control receiving focus is.
        for (key, state) in &mut self.row_reveal {
            if !self.row_focus[key].is_focused(w)
                && !self
                    .row_copy_focus
                    .get(key)
                    .is_some_and(|f| f.is_focused(w))
            {
                *state = false;
            }
        }
        let p = Palette::get(cx);
        let mut root = div()
            .id("history-root")
            .role(Role::Group)
            .aria_label("Transcription History")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(if self.embedded { 0. } else { 16. }))
            .bg(p.surface)
            .text_color(p.text)
            .font_family(".SystemUIFont")
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .capture_key_down(cx.listener(Self::key));
        if let Some(notice) = self.model.notice {
            if !self.fixture && self.announced_epoch != self.notice_epoch {
                crate::overlay::native::announce(w, notice);
                self.announced_epoch = self.notice_epoch;
            }
            root = root.child(
                div()
                    .when(notice == "Copied", |d| {
                        d.absolute().size_0().overflow_hidden()
                    })
                    .id("notice")
                    .role(Role::Status)
                    .aria_label(notice)
                    .child(notice),
            );
        }
        if let Some(c) = &self.model.confirmation {
            let scope = c.scope.clone();
            let label = format!(
                "Confirm delete {} {}?",
                c.keys.len(),
                if c.keys.len() == 1 {
                    "transcript"
                } else {
                    "transcripts"
                }
            );
            root =
                root.child(
                    div()
                        .id("confirm-delete")
                        .role(Role::Group)
                        .aria_label(label.clone())
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(label)
                        .child("Confirmation expires in four seconds.")
                        .child(Button::new("cancel-delete", "Cancel deletion").on_click(
                            cx.listener(|s, _, _, cx| {
                                s.model.confirmation = None;
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("confirm-delete-action", "Confirm deletion")
                                .intent(ButtonIntent::Danger)
                                .on_click(
                                    cx.listener(move |s, _, w, cx| s.delete(scope.clone(), w, cx)),
                                ),
                        ),
                );
        }
        if self.model.retry_import {
            root=root.child(div().id("migration").role(Role::Status).child("Some older history is waiting for import. Retained entries remain available.")
            .child(Button::new("retry-import","Retry History import").on_click(cx.listener(|s,_,_,_|s.request(RequestKind::RetryImport)))));
        }
        if !self.model.retain_history && !self.model.loading {
            root = root.child(div().id("retention").role(Role::Status).child(
                "Saving new transcripts is off. Existing history stays until you delete it.",
            ));
        }
        if let Some(problem) = self.model.problem {
            root=root.child(div().id("history-error").role(Role::Alert).child(match problem {Problem::Unavailable=>"History could not be refreshed. Try again.",Problem::Invalid=>"History data could not be displayed safely.",Problem::NewerSchema=>"This history was saved by a newer app. Restart with a compatible version."})
            .child(Button::new("retry","Retry History").on_click(cx.listener(|s,_,_,_|s.request(RequestKind::Read)))));
        }
        if self.model.detail.is_some() {
            return root.child(self.detail(cx));
        }
        root = root.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.title),
                )
                .child(
                    div()
                        .relative()
                        .w(px(208.))
                        .child(TextField::new(&self.search, "Search transcripts").search_chrome())
                        .when(self.model.query.text.is_empty(), |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .right(px(8.))
                                    .top(px(5.))
                                    .rounded(px(3.))
                                    .bg(p.surface_container_low)
                                    .px(px(4.))
                                    .text_size(tokens::MICRO_SIZE)
                                    .text_color(p.muted)
                                    .child("/"),
                            )
                        })
                        .when(!self.model.query.text.is_empty(), |d| {
                            d.child(
                                div().absolute().right(px(2.)).top(px(2.)).child(
                                    Button::new("clear-search", "Clear transcript search")
                                        .icon_only(Glyph::Close)
                                        .intent(ButtonIntent::Ghost)
                                        .size(ButtonSize::ExtraSmall)
                                        .on_click(cx.listener(|s, _, w, cx| {
                                            s.search.update(cx, |s, cx| s.set_value("", w, cx));
                                            let mut q = s.model.query.clone();
                                            q.text.clear();
                                            s.query(q, cx);
                                            w.focus(&s.search.read(cx).focus_handle(cx), cx);
                                        })),
                                ),
                            )
                        }),
                )
                .child(
                    Button::new("filter", self.model.query.filter_label())
                        .caption("Filter")
                        .trailing_icon(crate::icons::Glyph::ChevronDown)
                        .icon(Glyph::Filter)
                        .size(ButtonSize::Compact)
                        .toggled(self.filters_open)
                        .track_focus(&self.filter_focus)
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.filters_open = !s.filters_open;
                            cx.notify();
                        })),
                )
                .child(self.actions.clone()),
        );
        if self.filters_open {
            root =
                root.child(
                    div()
                        .id("filters")
                        .role(Role::Group)
                        .aria_label("Transcript filters")
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(self.source.clone())
                        .child(self.date.clone())
                        .child("Pinned only")
                        .child(
                            Toggle::new("pinned", "Pinned only", self.model.query.pinned)
                                .on_change(cx.listener(|s, pinned, _, cx| {
                                    let mut q = s.model.query.clone();
                                    q.pinned = *pinned;
                                    s.query(q, cx);
                                })),
                        )
                        .child(Button::new("clear-filters", "Clear filters").on_click(
                            cx.listener(|s, _, _, cx| {
                                let mut q = s.model.query.clone();
                                q.source = Source::All;
                                q.date = Date::All;
                                q.pinned = false;
                                s.source.update(cx, |s, cx| s.set_value("0", cx));
                                s.date.update(cx, |s, cx| s.set_value("0", cx));
                                s.query(q, cx);
                            }),
                        )),
                );
        }
        if self.model.query.narrowed() {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(format!(
                        "Showing {} of {}",
                        self.model.matches.len(),
                        self.model.entries.len()
                    ))
                    .child(
                        Button::new("show-all", "Show all")
                            .on_click(cx.listener(|s, _, w, cx| s.reset(w, cx))),
                    ),
            );
        }
        if self.model.loading {
            return root.child(Progress::new("Loading History", None));
        }
        if self.model.matches.is_empty() {
            return root.child(div().id("history-empty").role(Role::Label).child(if self.model.entries.is_empty() {"No transcription history yet. Your private, local transcripts will appear here."} else {"No matching transcripts"}));
        }
        let identity: Vec<_> = self
            .model
            .matches
            .iter()
            .map(|&i| self.model.entries[i].key)
            .collect();
        if identity != self.list_identity {
            self.scroll
                .state
                .reset_with_uniform_height(identity.len(), ROW_HEIGHT_HINT);
            self.list_identity = identity;
        } else if self.list_revision != self.model.ticket().revision {
            self.scroll.state.remeasure_items(0..identity.len());
        }
        self.list_revision = self.model.ticket().revision;
        self.rendered = 0..0;
        let list_bounds = self.scroll.bounds.clone();
        root.child(
            div()
                .id("history-list")
                .on_prepaint(move |bounds, _, _| list_bounds.set(bounds))
                .role(Role::List)
                .aria_label(
                    "Transcripts. Arrow keys navigate; Enter or Space copies; Right opens details.",
                )
                .aria_size_of_set(self.model.matches.len())
                .track_focus(&self.focus)
                .flex_1()
                .min_h_0()
                .focus_visible(|d| d.border_1().border_color(p.focus))
                .key_context("HistoryList")
                .on_action(cx.listener(|s, _: &NextRowAction, w, cx| {
                    if !s.tab_row_action(false, w, cx) {
                        cx.propagate();
                    }
                }))
                .on_action(cx.listener(|s, _: &PreviousRowAction, w, cx| {
                    if !s.tab_row_action(true, w, cx) {
                        cx.propagate();
                    }
                }))
                .capture_key_down(cx.listener(Self::list_key))
                .child(
                    {
                        let entity = cx.entity();
                        list(self.scroll.state.clone(), move |position, w, cx| {
                            entity.update(cx, |s, cx| {
                                s.rows(position..position + 1, w, cx).pop().unwrap()
                            })
                        })
                    }
                    .size_full(),
                ),
        )
        .child(
            div()
                .id("history-list-end")
                .track_focus(&self.list_end_focus)
                .absolute()
                .size_0(),
        )
    }
}
fn highlighted(text: &str, query: &str, p: Palette) -> AnyElement {
    let highlights = highlights(text, query).into_iter().map(|range| {
        (
            range,
            HighlightStyle {
                background_color: Some(p.accent.opacity(0.2)),
                color: Some(p.text),
                ..Default::default()
            },
        )
    });
    div()
        .child(StyledText::new(text.to_owned()).with_highlights(highlights))
        .into_any_element()
}
pub fn stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "Unavailable time".into())
}
pub fn local_clock() -> Clock {
    calendar_clock(chrono::Local::now())
}
fn calendar_clock<T: chrono::TimeZone>(now: chrono::DateTime<T>) -> Clock {
    use chrono::Days;
    let today = now.date_naive();
    let midnight = |days| {
        today
            .checked_sub_days(Days::new(days))
            .and_then(|date| calendar_start(&now.timezone(), date))
            .unwrap_or(i64::MIN)
    };
    Clock {
        now: now.timestamp_millis(),
        today: midnight(0),
        week: midnight(6),
        month: midnight(29),
    }
}
/// Earliest instant on this date. Gapped midnight advances to the first valid
/// local second; ambiguous midnight chooses the earlier instant. A wholly skipped
/// calendar date advances to the next representable date, never to wall-clock now.
fn calendar_start<T: chrono::TimeZone>(zone: &T, date: chrono::NaiveDate) -> Option<i64> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    (0..=172_800).find_map(|seconds| {
        let local = midnight.checked_add_signed(chrono::Duration::seconds(seconds))?;
        zone.from_local_datetime(&local)
            .earliest()
            .map(|d| d.timestamp_millis())
    })
}

#[cfg(feature = "production-host")]
impl History {
    pub(crate) fn production_action(
        &mut self,
        action: &str,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            "history-search" => self.focus_search(w, cx),
            "teach-latest" => {
                if let Some(entry) = self.model.entries.last().cloned() {
                    self.request(RequestKind::CorrectAndTeach { entry });
                }
            }
            "history-copy-last" => {
                if let Some(entry) = self.model.entries.last().cloned() {
                    self.request(RequestKind::CopyRow {
                        entry,
                        markdown: false,
                    });
                }
            }
            "history-export" => self.request(RequestKind::Export {
                entries: self.model.entries.clone(),
                format: Format::Markdown,
                save: true,
            }),
            _ => {}
        }
        cx.notify();
    }
}
