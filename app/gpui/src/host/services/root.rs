//! Main window composition. Only the active section holds a presentation lease.
use super::backend::{Backend, Route};
use crate::{Button, TextField, field::TextFieldState, settings::schema::Page, tokens::Palette};
use gpui::{prelude::*, *};
use serde_json::{Value, json};
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Home,
    Meetings,
    Insights,
    Settings,
    Models,
    Knowledge,
    Customize,
    Setup,
}
impl Section {
    fn name(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Meetings => "Meetings",
            Self::Insights => "Insights",
            Self::Settings => "Settings",
            Self::Models => "Models",
            Self::Knowledge => "Knowledge",
            Self::Customize => "Customize",
            Self::Setup => "Setup",
        }
    }
}
/// Editors opened from the Customize hub return to it; from any other page
/// they simply close, matching the shipped labels.
fn back_label(origin: Option<Page>) -> &'static str {
    match origin {
        None | Some(Page::Customize) => "Back to Customize",
        Some(_) => "Close editor",
    }
}
/// What a main-window shortcut does, decided from the visible state alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Nothing,
    Palette,
    Search,
    Settings(Page),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plan {
    close_palette: bool,
    open: Open,
}
fn plan(shortcut: crate::palette::model::Shortcut, palette_open: bool, in_settings: bool) -> Plan {
    use crate::palette::model::Shortcut;
    match shortcut {
        // Command-K toggles the palette.
        Shortcut::Palette => Plan {
            close_palette: palette_open,
            open: if palette_open {
                Open::Nothing
            } else {
                Open::Palette
            },
        },
        Shortcut::Search => Plan {
            close_palette: palette_open,
            open: Open::Search,
        },
        // Opening Settings while Settings is showing changes nothing.
        Shortcut::Settings => Plan {
            close_palette: palette_open,
            open: if in_settings {
                Open::Nothing
            } else {
                Open::Settings(Page::General)
            },
        },
        // Command-L leaves an open palette alone.
        Shortcut::Logs => Plan {
            close_palette: false,
            open: Open::Settings(Page::Performance),
        },
    }
}
struct Teaching {
    entry: murmur_core::history::Entry,
    epoch: u64,
    result: Value,
    busy: bool,
}
pub struct Main {
    backend: Backend,
    section: Section,
    visible: bool,
    ready: bool,
    startup_failed: bool,
    home: Entity<crate::home::Home>,
    settings: Entity<crate::settings::Settings>,
    meetings: Entity<crate::meetings::Meetings>,
    onboarding: Entity<crate::onboarding::Onboarding>,
    insights: Entity<crate::insights::Insights>,
    models: Entity<crate::models::Models>,
    knowledge: Entity<crate::knowledge::Knowledge>,
    modes: Entity<crate::modes::Modes>,
    themes: Entity<crate::themes::Themes>,
    customize: Entity<crate::customize::Customize>,
    palette: Entity<crate::palette::CommandPalette>,
    updater: murmur_core::updater::Snapshot,
    update_open: bool,
    required_update: bool,
    lifetime: Arc<AtomicU64>,
    notes_page: usize,
    teaching: Option<Teaching>,
    corrected: Entity<TextFieldState>,
    teaching_epoch: Arc<AtomicU64>,
    return_section: Option<Section>,
    error: Option<&'static str>,
    dictation_notice: Option<super::dictation_feedback::Notice>,
    feedback_revision: u64,
    subscriptions: Vec<Subscription>,
    pending_history: Option<String>,
    pending_meeting: Option<Option<String>>,
    /// Where a full-page editor was opened from.
    customize_return: Option<Origin>,
}
/// The Settings page an editor was opened from, the control that opened it and
/// that page's own Back stack, so closing the editor restores all three.
struct Origin {
    page: Page,
    opener: Option<String>,
    back: Vec<(Page, String)>,
}
impl Main {
    pub fn new(backend: Backend, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let host = Rc::new(backend.clone());
        let home = cx.new(|cx| crate::home::Home::new(host.clone(), host.clone(), w, cx));
        let settings = cx.new(|cx| crate::settings::Settings::new(host.clone(), w, cx));
        let meetings = cx.new(|cx| crate::meetings::Meetings::new(host.clone(), w, cx));
        let onboarding = cx.new(|cx| crate::onboarding::Onboarding::new(host.clone(), w, cx));
        let insights = cx.new(|cx| crate::insights::Insights::new(host.clone(), w, cx));
        let models = cx.new(|cx| crate::models::Models::new(host.clone(), w, cx));
        let knowledge = cx.new(|cx| crate::knowledge::Knowledge::new(host.clone(), w, cx));
        let modes = cx.new(|cx| crate::modes::Modes::new(host.clone(), w, cx));
        let themes = cx.new(|cx| crate::themes::Themes::new(host.clone(), w, cx));
        settings.update(cx, |s, _| {
            s.embed(Page::Appearance, themes.clone());
            s.embed(Page::Modes, modes.clone());
        });
        themes.update(cx, |s, _| s.embedded = true);
        modes.update(cx, |s, _| s.embedded = true);
        let customize = cx.new(|cx| crate::customize::Customize::new(host.clone(), w, cx));
        insights.update(cx, |s, _| s.embedded = true);
        meetings.update(cx, |s, _| s.embedded = true);
        let palette = cx.new(|cx| crate::palette::CommandPalette::new(host, w, cx));
        let corrected = cx.new(|cx| TextFieldState::new(w, cx));
        let mut s = Self {
            backend,
            section: Section::Home,
            visible: false,
            ready: false,
            startup_failed: false,
            home,
            settings,
            meetings,
            onboarding,
            insights,
            models,
            knowledge,
            modes,
            themes,
            customize,
            palette,
            updater: Default::default(),
            update_open: false,
            required_update: false,
            lifetime: Arc::new(AtomicU64::new(1)),
            notes_page: 0,
            teaching: None,
            corrected,
            teaching_epoch: Default::default(),
            return_section: None,
            error: None,
            dictation_notice: None,
            feedback_revision: 0,
            subscriptions: vec![],
            pending_history: None,
            pending_meeting: None,
            customize_return: None,
        };
        s.subscriptions.push(
            cx.subscribe(&s.settings, |s, _, _: &crate::settings::Done, cx| {
                s.backend.navigate(Route::Home);
                cx.notify();
            }),
        );
        s.subscriptions.push(cx.subscribe_in(
            &s.customize,
            w,
            |s, _, _: &crate::customize::Back, w, cx| {
                // Back returns to the Settings page the editor was opened from,
                // with that page's own Back route, and focuses the control that
                // opened it.
                let Origin { page, opener, back } = s.customize_return.take().unwrap_or(Origin {
                    page: Page::Customize,
                    opener: None,
                    back: vec![],
                });
                s.route(Route::Settings(page), w, cx);
                s.settings.update(cx, |settings, cx| {
                    settings.return_from_editor(opener, back, cx)
                });
            },
        ));
        s.show(w, cx);
        s
    }
    pub fn show(&mut self, _w: &mut Window, cx: &mut Context<Self>) {
        if self.visible {
            return;
        }
        self.visible = true;
        if !self.ready {
            cx.notify();
            return;
        }
        macro_rules! attach {
            ($field:ident) => {
                self.$field.update(cx, |s, cx| {
                    s.attach(cx);
                    s.follow(cx);
                })
            };
        }
        match self.section {
            Section::Home => {
                self.home.update(cx, |s, _| s.workspace = None);
                attach!(home);
            }
            Section::Settings => {
                attach!(settings);
                attach!(themes);
                attach!(modes);
            }
            Section::Meetings => {
                self.home.update(cx, |s, _| {
                    s.workspace = Some((
                        crate::home::model::Destination::Meetings,
                        self.meetings.clone().into(),
                    ))
                });
                attach!(home);
                attach!(meetings);
            }
            Section::Setup => attach!(onboarding),
            Section::Insights => {
                self.home.update(cx, |s, _| {
                    s.workspace = Some((
                        crate::home::model::Destination::Insights,
                        self.insights.clone().into(),
                    ))
                });
                attach!(home);
                attach!(insights);
            }
            Section::Models => attach!(models),
            Section::Knowledge => attach!(knowledge),
            Section::Customize => attach!(customize),
        }
        cx.notify();
    }
    /// Hides the window's content: sections lose their leases and the palette
    /// closes with them.
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.release(w, cx);
        self.palette.update(cx, |s, cx| s.hide(w, cx));
    }
    /// Section teardown alone. Switching sections must not dismiss the command
    /// palette, which belongs to the window rather than to a section.
    fn release(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.pending_history = None;
        self.pending_meeting = None;
        self.lifetime.fetch_add(1, Ordering::SeqCst);
        self.home.update(cx, |s, cx| s.hide(w, cx));
        self.settings.update(cx, |s, cx| s.hide(w, cx));
        self.meetings.update(cx, |s, cx| s.hide(w, cx));
        self.onboarding.update(cx, |s, cx| s.hide(cx));
        self.insights.update(cx, |s, cx| s.hide(cx));
        self.models.update(cx, |s, cx| s.hide(cx));
        self.knowledge.update(cx, |s, cx| s.hide(cx));
        self.modes.update(cx, |s, cx| s.hide(cx));
        self.themes.update(cx, |s, cx| s.hide(cx));
        self.customize.update(cx, |s, cx| s.hide(cx));
        self.close_teaching();
        self.backend.services.main_hidden();
        cx.notify();
    }
    /// Records where a full-page editor was opened from, so Back can return
    /// there. Opening one editor from another keeps the first origin.
    fn remember_editor_origin(&mut self, cx: &mut Context<Self>) {
        if self.section == Section::Settings {
            let page = self.settings.read(cx).model.page;
            let (opener, back) = self.settings.update(cx, |s, _| s.take_editor_return());
            self.customize_return = Some(Origin { page, opener, back });
        }
        let label = back_label(self.customize_return.as_ref().map(|origin| origin.page));
        self.customize.update(cx, |s, _| s.back_label = label);
    }
    /// Switches the section and closes the command palette, as every
    /// navigation except the shortcut that keeps it open does.
    fn select(&mut self, section: Section, w: &mut Window, cx: &mut Context<Self>) {
        self.switch(section, false, w, cx);
    }
    fn switch(
        &mut self,
        section: Section,
        keep_palette: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.section == Section::Customize && section != Section::Customize {
            self.customize_return = None;
        }
        if self.section != section {
            let started = std::time::Instant::now();
            let from = self.section.name();
            let to = section.name();
            let clocks = self.backend.diagnostics.clone();
            let epoch = clocks.epoch(crate::diagnostics::model::Lane::Latency);
            self.release(w, cx);
            if !keep_palette {
                self.palette.update(cx, |s, cx| s.hide(w, cx));
            }
            self.section = section;
            self.show(w, cx);
            let commit = started.elapsed().as_secs_f64() * 1000.;
            w.on_next_frame(move |w, _| {
                let first = started.elapsed().as_secs_f64() * 1000.;
                w.on_next_frame(move |_, _| {
                    clocks.transition(
                        epoch,
                        from,
                        to,
                        commit,
                        first,
                        started.elapsed().as_secs_f64() * 1000.,
                    )
                });
            });
        }
    }
    pub fn tick(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.backend.services.recording_terminal();
        let changed = {
            let mut feedback = self
                .backend
                .events
                .feedback
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            feedback.terminal(terminal);
            if self.feedback_revision == feedback.revision {
                false
            } else {
                self.feedback_revision = feedback.revision;
                self.dictation_notice = feedback.notice;
                true
            }
        };
        if changed {
            if self.dictation_notice.is_some() {
                // A hotkey failure must also be visible when Main was hidden.
                if !super::super::native::visible(w) {
                    super::super::native::show(w, true);
                }
                self.show(w, cx);
            }
            cx.notify();
        }
        if self.section == Section::Home
            && self.home.read(cx).production_history_ready(cx)
            && let Some(action) = self.pending_history.take()
        {
            self.home.update(cx, |home, cx| {
                home.production_history_action(&action, w, cx)
            });
        }
        if self.section == Section::Meetings
            && let Some(view) = self.meetings.read(cx).model.snapshot.as_ref()
            && !self.meetings.read(cx).model.pending
        {
            let _ = view;
            if let Some(action) = self.pending_meeting.take() {
                self.meetings.update(cx, |view, cx| {
                    if let Some(token) = action {
                        view.accept_suggestion(token, cx)
                    } else {
                        view.record(cx)
                    }
                });
            }
        }
        let level = self
            .backend
            .levels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let clock = self.backend.services.recording_clock();
        self.home.update(cx, |home, cx| {
            if home.model.attached
                && let Some(snapshot) = home.model.snapshot.as_mut()
            {
                let mut changed = false;
                if let Some((generation, seconds)) = clock
                    && snapshot.recording_generation == Some(generation)
                    && snapshot.recording_seconds != seconds
                {
                    snapshot.recording_seconds = seconds;
                    changed = true;
                }
                if let Some((generation, level)) = level
                    && snapshot.recording_generation == Some(generation)
                    && self.backend.services.recording_is_current(generation)
                {
                    snapshot.audio_level = level;
                    changed = true;
                }
                if changed {
                    cx.notify();
                }
            }
        });
        let meter = self
            .backend
            .preview
            .meter
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let owned = self.backend.preview.id.load(Ordering::SeqCst);
        if let Some((id, rms, peak)) = meter
            && owned == id
            && id > 0
        {
            self.settings.update(cx, |settings, cx| {
                if settings.model.attached
                    && settings.model.page == Page::Recording
                    && let Some(snapshot) = settings.model.snapshot.as_mut()
                {
                    snapshot.runtime.meter = Some((rms, peak));
                    cx.notify();
                }
            });
        }
    }
    /// Main-window shortcuts that are not bound actions (Control forms, Command-F
    /// and Command-L). Pages that own a key handle it first and stop it;
    /// everything else reaches here, wherever the focus is inside the window.
    fn key(&mut self, e: &KeyDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        if !self.shortcuts_active() {
            return;
        }
        let Some(shortcut) = crate::palette::main_shortcut(e, w) else {
            return;
        };
        self.apply(shortcut, w, cx);
        w.prevent_default();
        cx.stop_propagation();
    }
    /// The shipped shortcuts are inactive during setup, while a dialog is open
    /// and before the profile is ready.
    fn shortcuts_active(&self) -> bool {
        self.ready && self.section != Section::Setup && self.teaching.is_none() && !self.update_open
    }
    /// A bound shortcut action. GPUI runs bound actions before key listeners and
    /// an action stops the key, so Command-K and Command-, reach the policy here.
    fn shortcut(
        &mut self,
        shortcut: crate::palette::model::Shortcut,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcuts_active() {
            self.apply(shortcut, w, cx);
        }
    }
    /// Carries out the shortcut policy against the visible state.
    fn apply(
        &mut self,
        shortcut: crate::palette::model::Shortcut,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let palette_open = self.palette.read(cx).open;
        let plan = plan(shortcut, palette_open, self.section == Section::Settings);
        if plan.close_palette {
            self.palette.update(cx, |p, cx| p.hide(w, cx));
        }
        match plan.open {
            Open::Nothing => {}
            Open::Palette => self.route(Route::Palette, w, cx),
            Open::Search => self.route(Route::HistoryAction("history-search".into()), w, cx),
            Open::Settings(page) => self.open_settings(page, !plan.close_palette, w, cx),
        }
        cx.notify();
    }
    /// Shows a Settings page. An open palette stays open, with its query, when
    /// asked to.
    fn open_settings(
        &mut self,
        page: Page,
        keep_palette: bool,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.switch(Section::Settings, keep_palette, w, cx);
        self.settings
            .update(cx, |s, cx| s.navigate(page, None, w, cx));
        self.keep_palette_focus(w, cx);
    }
    fn keep_palette_focus(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.palette.read(cx).open {
            self.palette.update(cx, |p, cx| p.refocus(w, cx));
        }
    }
    pub fn route(&mut self, route: Route, w: &mut Window, cx: &mut Context<Self>) {
        if !self.ready
            && !matches!(
                route,
                Route::BootstrapRetry
                    | Route::BootstrapReady(_)
                    | Route::BootstrapFailed
                    | Route::Sources(_)
                    | Route::Notice(_)
                    | Route::Update(_)
                    | Route::Diagnostics
            )
        {
            return;
        }
        match route {
            Route::BootstrapRetry => {
                self.startup_failed = false;
                super::application::bootstrap(self.backend.clone());
            }
            Route::BootstrapFailed => {
                self.startup_failed = true;
                if self.ready {
                    self.error = Some("Legacy import remains preserved for retry.");
                }
            }
            Route::BootstrapReady(setup) => {
                if self.ready {
                    cx.notify();
                    return;
                }
                let visible = self.visible;
                self.ready = true;
                self.startup_failed = false;
                self.section = if setup { Section::Setup } else { Section::Home };
                self.visible = false;
                if visible {
                    self.show(w, cx);
                }
            }

            Route::Home => self.select(Section::Home, w, cx),
            Route::Meetings => self.select(Section::Meetings, w, cx),
            Route::MeetingToggle => {
                self.select(Section::Meetings, w, cx);
                self.pending_meeting = Some(None);
            }
            Route::MeetingSuggested(token) => {
                self.select(Section::Meetings, w, cx);
                self.pending_meeting = Some(Some(token));
            }
            Route::Insights => self.select(Section::Insights, w, cx),
            Route::Models => self.select(Section::Models, w, cx),
            Route::Knowledge => self.select(Section::Knowledge, w, cx),
            Route::Modes => {
                self.select(Section::Settings, w, cx);
                self.settings
                    .update(cx, |s, cx| s.navigate(Page::Modes, None, w, cx));
            }
            Route::Themes => {
                self.select(Section::Settings, w, cx);
                self.settings
                    .update(cx, |s, cx| s.navigate(Page::Appearance, None, w, cx));
            }
            Route::Customize => {
                self.select(Section::Settings, w, cx);
                self.settings
                    .update(cx, |s, cx| s.navigate(Page::Customize, None, w, cx));
            }
            Route::CustomizePage(page) => {
                self.remember_editor_origin(cx);
                self.select(Section::Customize, w, cx);
                self.customize.update(cx, |s, cx| s.set_page(page, w, cx));
            }
            Route::BindApp(mode) => {
                self.remember_editor_origin(cx);
                self.select(Section::Customize, w, cx);
                self.customize.update(cx, |s, cx| s.bind_app(mode, w, cx));
            }
            Route::Settings(page) => match page {
                Page::Diagnostics => self.backend.navigate(Route::Diagnostics),
                _ => self.open_settings(page, false, w, cx),
            },
            // Settings from the app menu or Command-, : a no-op while Settings is
            // showing, General otherwise, and an open palette closes first.
            Route::OpenSettings => self.apply(crate::palette::model::Shortcut::Settings, w, cx),
            // A control named by a hint: reuse Settings' disclosure, reveal and
            // focus once the page has its snapshot.
            Route::SettingsTarget(page, target) => {
                self.switch(Section::Settings, false, w, cx);
                self.settings
                    .update(cx, |s, cx| s.navigate_to_control(page, target, w, cx));
            }
            Route::Setup | Route::SetupReplay => {
                let replay = matches!(route, Route::SetupReplay);
                self.onboarding.update(cx, |s, _| s.replay = replay);
                self.select(Section::Setup, w, cx);
            }
            Route::Palette => {
                self.palette.update(cx, |s, cx| {
                    s.attach(cx);
                    s.follow(cx);
                    s.show(w, cx);
                });
                super::discovery::complete(&self.backend, "command_palette");
            }
            Route::Notice(message) => {
                self.error = Some(message);
            }
            Route::About => {
                let b = self.backend.clone();
                let version = b.services.version();
                self.backend.spawn(async move{let _=b.os.panel(json!({"kind":"alert","title":"Murmur","message":format!("Version {version}\nPrivate, local dictation and meeting transcription.\nMIT license · github.com/georgenijo/murmur-app"),"confirm":"OK"}),Arc::new(||true)).await;});
            }
            Route::Updates => {
                self.update_open = true;
                let b = self.backend.clone();
                self.backend.spawn(async move {
                    if let Ok(v) = b.call("updater_snapshot", json!({})).await
                        && let Ok(snapshot) = serde_json::from_value(v)
                    {
                        b.navigate(Route::Update(snapshot));
                    }
                });
            }
            Route::Update(snapshot) => {
                if snapshot.revision < self.updater.revision {
                    return;
                }
                use murmur_core::updater::Status;
                match &snapshot.update_status {
                    Status::Available { is_forced, .. }
                    | Status::Ready { is_forced, .. }
                    | Status::Error { is_forced, .. } => self.required_update = *is_forced,
                    Status::Idle | Status::UpToDate => self.required_update = false,
                    _ => {}
                }
                self.update_open |=
                    snapshot.is_update_dialog_open || snapshot.completed_update.is_some();
                self.updater = snapshot;
            }
            Route::Sources(sources) => {
                if let Some(doc) = sources.get("murmur-appearance").filter(|v| v.is_object()) {
                    crate::themes::appearance::install(doc.clone(), cx);
                }
            }
            Route::Teach(entry) => {
                self.close_teaching();
                self.return_section = Some(self.section);
                let epoch = self.teaching_epoch.fetch_add(1, Ordering::SeqCst) + 1;
                self.corrected
                    .update(cx, |s, cx| s.set_value(entry.text.clone(), w, cx));
                self.teaching = Some(Teaching {
                    entry,
                    epoch,
                    result: Value::Null,
                    busy: false,
                });
                w.focus(&self.corrected.read(cx).focus_handle(cx), cx);
            }
            Route::TeachingResult { epoch, result } => {
                if let Some(teaching) = self.teaching.as_mut().filter(|t| t.epoch == epoch) {
                    teaching.busy = false;
                    teaching.result = result;
                }
            }
            Route::HistoryAction(action) => {
                self.select(Section::Home, w, cx);
                self.pending_history = Some(action);
            }
            Route::Diagnostics | Route::Calibration { .. } => {}
        }
        cx.notify();
    }
    fn close_teaching(&mut self) {
        self.teaching_epoch.fetch_add(1, Ordering::SeqCst);
        if let Some(teaching) = self.teaching.take()
            && let Some(id) = teaching.result["proposalId"].as_u64()
        {
            let b = self.backend.clone();
            self.backend.spawn(async move {
                let _ = b
                    .call(
                        "discard_learned_correction_proposal",
                        json!({"proposalId":id}),
                    )
                    .await;
            });
        }
    }
    fn review_teaching(&mut self, cx: &mut Context<Self>) {
        let Some(teaching) = self.teaching.as_mut().filter(|t| !t.busy) else {
            return;
        };
        teaching.busy = true;
        let entry = teaching.entry.clone();
        let epoch = teaching.epoch;
        let corrected = self.corrected.read(cx).value().to_string();
        let state = self.teaching_epoch.clone();
        let b = self.backend.clone();
        self.backend.spawn(async move{
            let current=||state.load(Ordering::SeqCst)==epoch;
            if !current(){return;}
            let result=async {
                let (revision,snapshot)=b.services.history_snapshot().await?;
                if !current()||!snapshot.history.contains(&entry.target()){return Err("History changed".to_owned());}
                b.services.mutate_history(revision,murmur_core::completion::Command::History(murmur_core::completion::HistoryMutation::Edit{target:entry.target(),text:corrected.clone()}))?;
                if !current(){return Err("Presentation changed".into());}
                b.call("propose_learned_correction",json!({"request":{"originalText":entry.text,"correctedText":corrected,"teachingContext":entry.target().get("teachingContext")}})).await
            }.await;
            let result=result.unwrap_or_else(|_|json!({"kind":"unsafe","reason":"The correction could not be reviewed. Reopen the current History entry and retry."}));
            if current(){b.navigate(Route::TeachingResult{epoch,result});}
            else if let Some(id)=result["proposalId"].as_u64(){let _=b.call("discard_learned_correction_proposal",json!({"proposalId":id})).await;}
        });
    }
    fn update_action(&self, kind: &'static str) {
        let b = self.backend.clone();
        self.backend.spawn(async move {
            let action = if kind == "check" {
                json!({"kind":kind,"background":false})
            } else {
                json!({"kind":kind})
            };
            let _ = b.call("updater_action", json!({"action":action})).await;
        });
    }
    fn body(&self) -> AnyView {
        match self.section {
            Section::Home => self.home.clone().into(),
            Section::Meetings | Section::Insights => self.home.clone().into(),
            Section::Settings => self.settings.clone().into(),
            Section::Setup => self.onboarding.clone().into(),
            Section::Models => self.models.clone().into(),
            Section::Knowledge => self.knowledge.clone().into(),
            Section::Customize => self.customize.clone().into(),
        }
    }
}
actions!(
    native_workspace,
    [CommandPalette, HomePage, MeetingsPage, Back]
);
/// The main window's key bindings. The host binds exactly these.
pub fn bindings() -> [KeyBinding; 4] {
    [
        KeyBinding::new("cmd-k", CommandPalette, Some("MurmurWorkspace")),
        KeyBinding::new("cmd-1", HomePage, Some("MurmurWorkspace")),
        KeyBinding::new("cmd-2", MeetingsPage, Some("MurmurWorkspace")),
        KeyBinding::new("escape", Back, Some("MurmurWorkspace")),
    ]
}
impl Render for Main {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.ready {
            return div().size_full().flex().flex_col().items_center().justify_center().gap_4()
                .child(if self.startup_failed {"Murmur could not finish loading your local data. Your previous data is preserved."} else {"Loading your local data…"})
                .when(self.startup_failed, |d| d.child(Button::new("startup-retry", "Retry").on_click(cx.listener(|s,_,_,cx| {s.startup_failed=false;super::application::bootstrap(s.backend.clone());cx.notify();}))))
                .into_any_element();
        }

        let p = Palette::get(cx);
        let mut root = div()
            .id("native-workspace")
            .key_context("MurmurWorkspace")
            .size_full()
            .relative()
            .bg(p.surface)
            .text_color(p.text)
            .on_drop(cx.listener(|s, paths: &ExternalPaths, _, cx| {
                let paths: Vec<_> = paths
                    .paths()
                    .iter()
                    .take(64)
                    .filter_map(|p| p.to_str().map(|s| json!(s)))
                    .collect();
                if paths.is_empty() {
                    return;
                }
                let lifetime = s.lifetime.clone();
                let epoch = lifetime.load(Ordering::SeqCst);
                let current: super::os::Live =
                    Arc::new(move || lifetime.load(Ordering::SeqCst) == epoch);
                let b = s.backend.clone();
                s.backend.spawn(async move {
                    let _ = b.transcribe_paths(paths, current).await;
                });
                cx.notify();
            }))
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|s, _: &CommandPalette, w, cx| {
                s.shortcut(crate::palette::model::Shortcut::Palette, w, cx)
            }))
            .on_action(cx.listener(|s, _: &HomePage, w, cx| s.select(Section::Home, w, cx)))
            .on_action(cx.listener(|s, _: &MeetingsPage, w, cx| s.select(Section::Meetings, w, cx)))
            .on_action(cx.listener(|s, _: &Back, w, cx| {
                if s.palette.read(cx).open {
                    // Escape closes the palette first and goes nowhere else.
                    s.palette.update(cx, |p, cx| p.hide(w, cx));
                } else if s.teaching.is_some() {
                    s.close_teaching();
                    s.home.update(cx, |s, cx| s.destination_closed(w, cx));
                } else if s.update_open {
                    if !s.required_update && !update_required(&s.updater.update_status) {
                        s.update_open = false;
                        s.update_action("dismiss");
                    }
                } else {
                    s.select(Section::Home, w, cx);
                }
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .size_full()
                    .child(div().flex_1().min_w_0().h_full().child(self.body())),
            )
            .when(self.palette.read(cx).open, |root| {
                root.child(div().absolute().inset_0().child(self.palette.clone()))
            });
        if let Some(notice) = self.dictation_notice {
            use super::dictation_feedback::Recovery;
            let label = match notice.recovery {
                Recovery::MicrophoneSettings => "Open Microphone Settings…",
                Recovery::ChooseMicrophone => "Choose microphone…",
                Recovery::Reinstall => "Open Murmur downloads…",
                Recovery::Restart => "Restart Murmur",
                Recovery::Retry => "Recording settings…",
            };
            root = root.child(div()
                .id("dictation-failure-banner")
                .role(Role::Alert)
                .aria_label("Dictation needs attention")
                .absolute().bottom_4().left_4().right_4().rounded_lg()
                .p_4().bg(p.surface).border_1().border_color(p.border)
                .flex().flex_wrap().gap_3()
                .child(gpui::text!(notice.message))
                .child(Button::new("dictation-recovery", label).on_click(cx.listener(move |s, _, w, cx| {
                    match notice.recovery {
                        Recovery::ChooseMicrophone | Recovery::Retry => s.route(Route::Settings(Page::Recording), w, cx),
                        recovery => {
                            let b = s.backend.clone();
                            s.backend.spawn(async move {
                                let result = match recovery {
                                    Recovery::MicrophoneSettings => b.call("open_system_preferences", json!({})).await.map(|_| ()),
                                    Recovery::Restart => { b.services.shutdown(murmur_core::shutdown::Request::Restart); Ok(()) },
                                    Recovery::Reinstall => b.os.open("https://github.com/georgenijo/murmur-app/releases/latest".into(), false, Arc::new(|| true)).await,
                                    _ => Ok(()),
                                };
                                if result.is_err() { b.navigate(Route::Notice("Could not open recovery. Use System Settings or restart Murmur manually.")); }
                            });
                        }
                    }
                })))
                .child(Button::new("dismiss-dictation-failure", "Dismiss").on_click(cx.listener(|s, _, _, cx| {
                    s.backend.events.feedback.lock().unwrap_or_else(|p| p.into_inner()).dismiss();
                    s.dictation_notice = None;
                    cx.notify();
                }))));
        }
        if let Some(error) = self.error {
            root = root.child(gpui::text!(error));
        }
        if self.update_open {
            use murmur_core::updater::Status;
            let status = &self.updater.update_status;
            let (title, message, primary, action, forced) = match status {
                Status::Idle => (
                    "Updates".into(),
                    "Check for a newer Murmur release.".into(),
                    Some("Check now"),
                    "check",
                    false,
                ),
                Status::Checking => (
                    "Checking for updates…".into(),
                    String::new(),
                    None,
                    "",
                    false,
                ),
                Status::UpToDate => (
                    "Murmur is up to date".into(),
                    String::new(),
                    Some("Check again"),
                    "check",
                    false,
                ),
                Status::Available {
                    version,
                    notes,
                    is_forced,
                } => (
                    format!("Murmur {version} is available"),
                    notes.clone(),
                    Some("Download update"),
                    "download",
                    *is_forced,
                ),
                Status::Preparing { version } => (
                    format!("Preparing Murmur {version}…"),
                    String::new(),
                    None,
                    "",
                    false,
                ),
                Status::Downloading { version, progress } => (
                    format!("Downloading Murmur {version}…"),
                    progress.map(|v| format!("{v:.0}%")).unwrap_or_default(),
                    None,
                    "",
                    false,
                ),
                Status::Ready { version, is_forced } => (
                    format!("Murmur {version} is ready"),
                    "Restart Murmur to install this update.".into(),
                    Some("Restart and install"),
                    "install",
                    *is_forced,
                ),
                Status::Restarting { .. } => (
                    "Finishing before restart…".into(),
                    "Active work is being drained safely.".into(),
                    None,
                    "",
                    true,
                ),
                Status::Error {
                    message,
                    recovery,
                    stage,
                    is_forced,
                    ..
                } => (
                    "Update could not finish".into(),
                    message.clone(),
                    if recovery.as_deref() == Some("reinstall") {
                        None
                    } else {
                        Some(if stage == "restart" {
                            "Retry restart"
                        } else {
                            "Try again"
                        })
                    },
                    match stage.as_str() {
                        "restart" => "install",
                        "install" => "download",
                        _ => "check",
                    },
                    *is_forced,
                ),
            };
            let forced = forced || self.required_update;
            let busy = matches!(
                status,
                Status::Preparing { .. }
                    | Status::Downloading { .. }
                    | Status::Restarting { .. }
                    | Status::Checking
            );
            let (title, message) = if let Some(completed) = &self.updater.completed_update {
                (
                    format!("Updated to Murmur {}", completed.version),
                    completed.notes.clone(),
                )
            } else {
                (title, message)
            };
            let page = message
                .chars()
                .skip(self.notes_page * 4096)
                .take(4096)
                .collect::<String>();
            let close = (!forced && !busy).then(|| {
                Button::new("updater-close", "Close update dialog")
                    .icon_only(crate::icons::Glyph::Close)
                    .intent(crate::ButtonIntent::Ghost)
                    .size(crate::ButtonSize::Compact)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.update_open = false;
                        s.update_action(if s.updater.completed_update.is_some() {
                            "dismissCompleted"
                        } else {
                            "dismiss"
                        });
                        cx.notify();
                    }))
            });
            let (heading, caption) = match status {
                Status::Available { version, .. } => {
                    ("Update available".to_string(), format!("Version {version}"))
                }
                _ => (title, String::new()),
            };
            let mut modal = crate::update_dialog::card(p)
                .child(crate::update_dialog::header(heading, caption, close, p))
                .child(crate::update_dialog::notes().child({
                    let backend = self.backend.clone();
                    gpui_component::text::TextView::markdown("native-release-notes", page)
                        .markdown_extensions(super::notes::extensions())
                        .on_link_click(move |url, _, _, _| {
                            if super::notes::allowed_link(url) {
                                let b = backend.clone();
                                let url = url.to_string();
                                backend.spawn(async move {
                                    let _ = b.os.open(url, false, Arc::new(|| true)).await;
                                });
                            }
                        })
                }));
            if message.chars().count() > (self.notes_page + 1) * 4096 {
                modal = modal.child(Button::new("more-notes", "More release notes").on_click(
                    cx.listener(|s, _, _, cx| {
                        s.notes_page += 1;
                        cx.notify();
                    }),
                ));
            }
            if self.notes_page > 0 {
                modal = modal.child(
                    Button::new("previous-notes", "Previous release notes").on_click(cx.listener(
                        |s, _, _, cx| {
                            s.notes_page = s.notes_page.saturating_sub(1);
                            cx.notify();
                        },
                    )),
                );
            }
            if matches!(status,Status::Error{stage,..} if stage=="check") {
                modal = modal.child(Button::new("release-page", "Open releases page").on_click(
                    cx.listener(|s, _, _, _| {
                        let b = s.backend.clone();
                        s.backend.spawn(async move {
                            let _ =
                                b.os.open(
                                    "https://github.com/georgenijo/murmur-app/releases".into(),
                                    false,
                                    Arc::new(|| true),
                                )
                                .await;
                        });
                    }),
                ));
            }
            if forced {
                modal = modal.child(Button::new("updater-quit", "Quit Murmur").on_click(
                    cx.listener(|s, _, _, _| {
                        s.backend
                            .services
                            .shutdown(murmur_core::shutdown::Request::Quit)
                    }),
                ));
            }
            let mut footer = crate::update_dialog::footer(p);
            if let Some(label) = primary {
                footer = footer.child(
                    div().flex_1().child(
                        Button::new("updater-primary", label)
                            .pill()
                            .height(px(38.))
                            .full_width()
                            .intent(crate::ButtonIntent::Primary)
                            .on_click(cx.listener(move |s, _, _, _| s.update_action(action))),
                    ),
                );
            }
            if !forced && !busy {
                footer = footer.child(
                    div().flex_1().child(
                        Button::new("updater-later", "Close")
                            .caption("Later")
                            .pill()
                            .height(px(38.))
                            .full_width()
                            .intent(crate::ButtonIntent::Secondary)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.update_open = false;
                                s.update_action(if s.updater.completed_update.is_some() {
                                    "dismissCompleted"
                                } else {
                                    "dismiss"
                                });
                                cx.notify();
                            })),
                    ),
                );
            }
            modal = modal.child(footer);
            root = root.child(crate::update_dialog::scrim().child(modal));
        }
        if let Some(teaching) = &self.teaching {
            let proposal_id = teaching.result["proposalId"].as_u64();
            let mut modal = div()
                .flex()
                .flex_col()
                .gap_3()
                .p_5()
                .w(px(600.))
                .bg(p.raised)
                .border_1()
                .border_color(p.border)
                .child(
                    div()
                        .text_size(px(20.))
                        .child(gpui::text!("Correct & Teach")),
                )
                .child(
                    TextField::new(&self.corrected, "Corrected dictation")
                        .readonly(teaching.busy || proposal_id.is_some()),
                );
            if let Some(id) = proposal_id {
                let label = format!(
                    "Remember “{}” as “{}”",
                    teaching.result["source"].as_str().unwrap_or_default(),
                    teaching.result["replacement"].as_str().unwrap_or_default()
                );
                modal = modal.child(gpui::text!(label));
                for (index, option) in teaching.result["scopeOptions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(3)
                    .enumerate()
                {
                    let scope = option["scope"].clone();
                    let label = option["label"]
                        .as_str()
                        .unwrap_or("Save correction")
                        .to_owned();
                    modal=modal.child(Button::new(format!("teach-confirm-{index}"),label).disabled(teaching.busy).on_click(cx.listener(move|s,_,_,cx|{
                        let Some(teaching)=s.teaching.as_mut().filter(|t|!t.busy) else{return;};
                        let epoch=teaching.epoch;
                        let accepted=s.backend.services.accept_correction(id,scope.clone());
                        teaching.busy=true;
                        let b=s.backend.clone();
                        s.backend.spawn(async move{
                            let success=match accepted {Ok(job)=>job.await.is_ok_and(|r|r.is_ok()),Err(_)=>false};
                            b.navigate(Route::TeachingResult{epoch,result:if success{json!({"kind":"saved","reason":"The correction is saved."})}else{json!({"kind":"unsafe","reason":"The correction could not be saved. Review it again and retry."})}});
                        });cx.notify();
                    })));
                }
            } else {
                modal = modal.child(
                    Button::new("teach-review", "Save correction and review rule")
                        .disabled(teaching.busy)
                        .on_click(cx.listener(|s, _, _, cx| s.review_teaching(cx))),
                );
            }
            if let Some(reason) = teaching.result["reason"].as_str() {
                modal = modal.child(gpui::text!(reason.to_owned()));
            }
            modal = modal.child(Button::new("teach-cancel", "Close").on_click(cx.listener(
                |s, _, w, cx| {
                    s.close_teaching();
                    s.home.update(cx, |h, cx| h.destination_closed(w, cx));
                    cx.notify();
                },
            )));
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(p.surface.opacity(0.8))
                    .child(modal),
            );
        }
        root.into_any_element()
    }
}

fn update_required(status: &murmur_core::updater::Status) -> bool {
    use murmur_core::updater::Status;
    matches!(
        status,
        Status::Available {
            is_forced: true,
            ..
        } | Status::Ready {
            is_forced: true,
            ..
        } | Status::Error {
            is_forced: true,
            ..
        } | Status::Preparing { .. }
            | Status::Downloading { .. }
            | Status::Restarting { .. }
    )
}
#[cfg(test)]
mod shortcut_tests {
    use super::*;
    use crate::palette::model::Shortcut;
    // `use super::*` also brings in `gpui::test`, which would shadow the
    // built-in `#[test]` and recurse forever when it expands.
    use std::prelude::v1::test;
    #[test]
    fn command_k_toggles_and_the_others_close_the_palette_first() {
        let closed = plan(Shortcut::Palette, false, false);
        assert_eq!((closed.close_palette, closed.open), (false, Open::Palette));
        let open = plan(Shortcut::Palette, true, false);
        assert_eq!((open.close_palette, open.open), (true, Open::Nothing));
        let search = plan(Shortcut::Search, true, true);
        assert_eq!((search.close_palette, search.open), (true, Open::Search));
        let settings = plan(Shortcut::Settings, true, false);
        assert_eq!(
            (settings.close_palette, settings.open),
            (true, Open::Settings(Page::General))
        );
    }
    #[test]
    fn settings_is_a_no_op_while_showing_and_performance_keeps_the_palette() {
        assert_eq!(plan(Shortcut::Settings, false, true).open, Open::Nothing);
        let logs = plan(Shortcut::Logs, true, false);
        assert_eq!(
            (logs.close_palette, logs.open),
            (false, Open::Settings(Page::Performance))
        );
    }
    #[test]
    fn settings_shortcut_closes_the_palette_and_stays_on_settings() {
        let plan = plan(Shortcut::Settings, true, true);
        assert_eq!((plan.close_palette, plan.open), (true, Open::Nothing));
    }
    /// Stands in for the composed window, which needs a running host. It
    /// carries the production bindings and the production policy.
    struct Keys {
        focus: FocusHandle,
        palette_open: bool,
        listener_runs: u32,
    }
    impl Render for Keys {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context("MurmurWorkspace")
                .track_focus(&self.focus)
                .size_full()
                .on_key_down(cx.listener(|s, _: &KeyDownEvent, _, _| s.listener_runs += 1))
                .on_action(cx.listener(|s, _: &CommandPalette, _, _| {
                    s.palette_open =
                        plan(Shortcut::Palette, s.palette_open, false).open == Open::Palette;
                }))
        }
    }
    #[gpui::test]
    fn the_bound_command_k_action_carries_the_toggle_and_the_listener_never_sees_it(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| cx.bind_keys(bindings()));
        let (view, cx) = cx.add_window_view(|_, cx| Keys {
            focus: cx.focus_handle(),
            palette_open: false,
            listener_runs: 0,
        });
        let focus = view.update(cx, |v, _| v.focus.clone());
        cx.update(|w, cx| w.focus(&focus, cx));
        cx.run_until_parked();
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let state =
            |cx: &mut VisualTestContext| view.update(cx, |v, _| (v.palette_open, v.listener_runs));
        cx.simulate_keystrokes("cmd-k");
        assert_eq!(state(cx), (true, 0));
        cx.simulate_keystrokes("cmd-k");
        assert_eq!(state(cx), (false, 0));
        // Unbound shortcuts such as Command-F reach the key listener instead.
        cx.simulate_keystrokes("cmd-f");
        assert_eq!(state(cx), (false, 1));
    }
    #[test]
    fn editors_name_where_back_leads() {
        assert_eq!(back_label(None), "Back to Customize");
        assert_eq!(back_label(Some(Page::Customize)), "Back to Customize");
        for page in [Page::Text, Page::Modes, Page::General] {
            assert_eq!(back_label(Some(page)), "Close editor");
        }
    }
}
