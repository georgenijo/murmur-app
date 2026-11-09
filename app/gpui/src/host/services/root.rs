//! Main window composition. Only the active section holds a presentation lease.
use super::backend::{Backend, Route};
use super::teaching::{self, Review, Settled, Step, Teaching};
use crate::{Button, TextField, settings::schema::Page, tokens::Palette};
use gpui::{prelude::*, *};
use gpui_component::input::Textarea;
use serde_json::json;
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
    fields: teaching::Fields,
    teaching_epoch: Arc<AtomicU64>,
    return_section: Option<Section>,
    error: Option<&'static str>,
    dictation_notice: Option<super::dictation_feedback::Notice>,
    feedback_revision: u64,
    subscriptions: Vec<Subscription>,
    pending_history: Option<String>,
    pending_meeting: Option<Option<String>>,
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
        let fields = teaching::Fields::new(w, cx);
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
            fields,
            teaching_epoch: Default::default(),
            return_section: None,
            error: None,
            dictation_notice: None,
            feedback_revision: 0,
            subscriptions: vec![],
            pending_history: None,
            pending_meeting: None,
        };
        s.subscriptions.push(
            cx.subscribe(&s.settings, |s, _, _: &crate::settings::Done, cx| {
                s.backend.navigate(Route::Home);
                cx.notify();
            }),
        );
        s.subscriptions.push(
            cx.subscribe(&s.customize, |s, _, _: &crate::customize::Back, cx| {
                let backend = s.backend.clone();
                backend.navigate(Route::Modes);
                cx.notify();
            }),
        );
        // Review buttons follow the typed text and the heard-term selection.
        for field in [s.fields.corrected.clone(), s.fields.heard.clone()] {
            s.subscriptions
                .push(cx.observe(&field, |_, _, cx| cx.notify()));
        }
        for field in [s.fields.source.clone(), s.fields.replacement.clone()] {
            s.subscriptions
                .push(cx.observe(&field, |_, _, cx| cx.notify()));
        }
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
    pub fn hide(&mut self, w: &mut Window, cx: &mut Context<Self>) {
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
        self.palette.update(cx, |s, cx| s.hide(w, cx));
        self.close_teaching();
        self.backend.services.main_hidden();
        cx.notify();
    }
    fn select(&mut self, section: Section, w: &mut Window, cx: &mut Context<Self>) {
        if self.section != section {
            let started = std::time::Instant::now();
            let from = self.section.name();
            let to = section.name();
            let clocks = self.backend.diagnostics.clone();
            let epoch = clocks.epoch(crate::diagnostics::model::Lane::Latency);
            self.hide(w, cx);
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
                self.select(Section::Customize, w, cx);
                self.customize.update(cx, |s, cx| s.set_page(page, w, cx));
            }
            Route::BindApp(mode) => {
                self.select(Section::Customize, w, cx);
                self.customize.update(cx, |s, cx| s.bind_app(mode, w, cx));
            }
            Route::Settings(page) => match page {
                Page::Diagnostics => self.backend.navigate(Route::Diagnostics),
                _ => {
                    self.select(Section::Settings, w, cx);
                    self.settings
                        .update(cx, |s, cx| s.navigate(page, None, w, cx));
                }
            },
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
                self.fields.seed(&entry, w, cx);
                self.teaching = Some(Teaching::new(entry, epoch));
                w.focus(&self.fields.corrected.read(cx).focus_handle(cx), cx);
            }
            Route::TeachingResult { epoch, result } => {
                let mut close = false;
                // The typed correction stays in its field through every step,
                // so a "Save correction only" retry writes exactly that text.
                if let Some(teaching) = self.teaching.as_mut().filter(|t| t.epoch == epoch) {
                    close = teaching.settle(result);
                }
                if close {
                    self.close_teaching();
                    self.home.update(cx, |h, cx| h.destination_closed(w, cx));
                } else {
                    self.focus_teaching(w, cx);
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
        if let Some(id) = self.teaching.take().and_then(|t| t.proposal_id()) {
            let b = self.backend.clone();
            self.backend
                .spawn(async move { teaching::discard(&b, id).await });
        }
    }
    fn focus_teaching(&self, w: &mut Window, cx: &mut Context<Self>) {
        let handle = match self.teaching.as_ref().map(|t| t.step) {
            Some(Step::Edit) => self.fields.corrected.read(cx).focus_handle(cx),
            Some(Step::SpecificEdit) => self.fields.source.read(cx).focus_handle(cx),
            _ => return,
        };
        w.focus(&handle, cx);
    }
    /// Run one service operation for the open dialog. The result returns through
    /// the route queue and is ignored if the dialog was closed or replaced.
    fn run_teaching<Fut>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(Backend, super::os::Live, murmur_core::history::Entry) -> Fut,
    ) where
        Fut: std::future::Future<Output = Settled> + Send + 'static,
    {
        let Some(teaching) = self.teaching.as_mut().filter(|t| !t.busy) else {
            return;
        };
        teaching.busy = true;
        teaching.error = None;
        let epoch = teaching.epoch;
        let entry = teaching.entry.clone();
        let state = self.teaching_epoch.clone();
        let live: super::os::Live = Arc::new(move || state.load(Ordering::SeqCst) == epoch);
        let b = self.backend.clone();
        let job = work(b.clone(), live, entry);
        self.backend.spawn(async move {
            let result = job.await;
            b.navigate(Route::TeachingResult { epoch, result });
        });
        cx.notify();
    }
    /// The exact History text a save writes: always the typed correction, never
    /// the rule preview. Blank or unchanged text never saves.
    fn teaching_text(&self, cx: &App) -> Option<String> {
        let teaching = self.teaching.as_ref()?;
        let text = self.fields.corrected.read(cx).value().to_string();
        (!text.trim().is_empty() && text != teaching.entry.text).then_some(text)
    }
    fn review_automatic(&mut self, cx: &mut Context<Self>) {
        let corrected = self.fields.corrected.read(cx).value().to_string();
        if !self
            .teaching
            .as_ref()
            .is_some_and(|t| t.can_review(&corrected))
        {
            return;
        }
        self.run_teaching(cx, move |b, live, entry| async move {
            teaching::review(&b, &live, &entry, Review::Automatic { corrected }).await
        });
    }
    fn review_specific(&mut self, cx: &mut Context<Self>) {
        let source = self.fields.source.read(cx).value().to_string();
        let replacement = self.fields.replacement.read(cx).value().to_string();
        if !self
            .teaching
            .as_ref()
            .is_some_and(|t| t.can_review_specific(&source, &replacement))
        {
            return;
        }
        self.run_teaching(cx, move |b, live, entry| async move {
            let review = Review::Specific {
                source,
                replacement,
            };
            teaching::review(&b, &live, &entry, review).await
        });
    }
    fn save_teaching_only(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.teaching_text(cx) else {
            return;
        };
        let proposal = self.teaching.as_ref().and_then(|t| t.proposal_id());
        self.run_teaching(cx, move |b, live, entry| async move {
            teaching::save_only(&b, &live, &entry, text, proposal).await
        });
    }
    fn remember_teaching(&mut self, cx: &mut Context<Self>) {
        // The typed correction travels with the rule; an unchanged or blank one
        // saves the rule alone and leaves History untouched.
        let text = self.fields.corrected.read(cx).value().to_string();
        let Some((proposal, scope)) = self
            .teaching
            .as_ref()
            .and_then(|t| Some((t.proposal_id()?, t.selected_scope()?.scope.clone())))
        else {
            return;
        };
        self.run_teaching(cx, move |b, live, entry| async move {
            teaching::remember(&b, &live, &entry, text, proposal, scope).await
        });
    }
    fn teaching_back(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(teaching) = self.teaching.as_mut().filter(|t| !t.busy) else {
            return;
        };
        if let Some(id) = teaching.back() {
            let b = self.backend.clone();
            self.backend
                .spawn(async move { teaching::discard(&b, id).await });
        }
        self.focus_teaching(w, cx);
        cx.notify();
    }
    fn teach_specific(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(teaching) = self.teaching.as_mut().filter(|t| !t.busy) else {
            return;
        };
        let (source, replacement, discard) = teaching.open_specific();
        if let Some(id) = discard {
            let b = self.backend.clone();
            self.backend
                .spawn(async move { teaching::discard(&b, id).await });
        }
        for (field, text) in [
            (&self.fields.source, source),
            (&self.fields.replacement, replacement),
        ] {
            field.update(cx, |s, cx| s.set_value(text, w, cx));
        }
        self.focus_teaching(w, cx);
        cx.notify();
    }
    fn use_heard_selection(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if self.fields.use_selection(w, cx) {
            cx.notify();
        }
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
impl Render for Main {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|s, _: &CommandPalette, w, cx| s.route(Route::Palette, w, cx)))
            .on_action(cx.listener(|s, _: &HomePage, w, cx| s.select(Section::Home, w, cx)))
            .on_action(cx.listener(|s, _: &MeetingsPage, w, cx| s.select(Section::Meetings, w, cx)))
            .on_action(cx.listener(|s, _: &Back, w, cx| {
                if s.teaching.is_some() {
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
        if self.teaching.is_some() {
            let modal = self.teaching_modal(p, w.viewport_size().height, cx);
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

fn teach_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Text {
    Text::new(id.into(), text.into())
}

fn teach_example(p: Palette, id: &'static str, label: &'static str, text: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .border_1()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(teach_text(format!("{id}-label"), label)),
        )
        .child(
            // Long examples scroll inside their own box so the dialog's actions
            // always stay on screen.
            div()
                .id(format!("{id}-scroll"))
                .max_h(px(120.))
                .overflow_y_scroll()
                .child(teach_text(id, text)),
        )
}

impl Main {
    /// Step-based Correct & Teach dialog. Rendering reads state only; every
    /// button forwards to a method that goes through `teaching::*`.
    fn teaching_modal(&self, p: Palette, viewport: Pixels, cx: &mut Context<Self>) -> Div {
        let Some(t) = self.teaching.as_ref() else {
            return div();
        };
        let busy = t.busy;
        let corrected = self.fields.corrected.read(cx).value().to_string();
        let source = self.fields.source.read(cx).value().to_string();
        let replacement = self.fields.replacement.read(cx).value().to_string();
        let selection = !self.fields.selected_term(cx).is_empty();
        let can_save = !busy && self.teaching_text(cx).is_some();
        let can_remember = !busy && t.proposal().is_some();
        let primary = crate::ButtonIntent::Primary;
        let mut modal = teach_body();
        let mut buttons = div().flex().flex_row().justify_end().gap_2();
        match t.step {
            Step::Edit => {
                modal = modal
                    .child(teach_text(
                        "teach-intro",
                        "Edit this local history entry. Murmur will not remember a rule unless you review it and press Remember correction.",
                    ))
                    .child(
                        Textarea::new(&self.fields.corrected)
                            .aria_label("Corrected dictation")
                            .disabled(busy)
                            .h(px(140.)),
                    );
                buttons = buttons
                    .child(Button::new("teach-cancel", "Cancel").on_click(cx.listener(
                        |s, _, w, cx| {
                            s.close_teaching();
                            s.home.update(cx, |h, cx| h.destination_closed(w, cx));
                            cx.notify();
                        },
                    )))
                    .child(
                        Button::new("teach-specific", "Teach specific term")
                            .disabled(busy)
                            .on_click(cx.listener(|s, _, w, cx| s.teach_specific(w, cx))),
                    )
                    .child(
                        Button::new(
                            "teach-review",
                            if busy {
                                "Reviewing…"
                            } else {
                                "Review correction"
                            },
                        )
                        .intent(primary)
                        .disabled(!t.can_review(&corrected))
                        .on_click(cx.listener(|s, _, _, cx| s.review_automatic(cx))),
                    );
            }
            Step::AutomaticReview | Step::SpecificReview => {
                let specific = t.step == Step::SpecificReview;
                if let Some(proposal) = t.proposal() {
                    modal = modal
                        .child(teach_text(
                            "teach-review-title",
                            "Review learned correction",
                        ))
                        .child(teach_text(
                            "teach-rule",
                            format!(
                                "Remember “{}” as “{}” ({} in this example)",
                                proposal.source, proposal.replacement, proposal.occurrences
                            ),
                        ))
                        .child(teach_example(
                            p,
                            "teach-before",
                            "Before",
                            proposal.before.clone(),
                        ))
                        .child(teach_example(
                            p,
                            "teach-after",
                            "After",
                            proposal.after.clone(),
                        ));
                    // The rule preview and the History edit are separate: show
                    // the typed correction that Save/Remember will write.
                    if let Some(typed) = self
                        .teaching_text(cx)
                        .filter(|typed| *typed != proposal.after)
                    {
                        modal = modal.child(teach_example(
                            p,
                            "teach-history",
                            "Saved to History",
                            typed,
                        ));
                    }
                    modal =
                        modal.child(teach_text("teach-scope-label", "Where should this apply?"));
                    let mut scopes = div().flex().flex_col().gap_1();
                    for (index, option) in proposal.scopes.iter().enumerate() {
                        scopes = scopes.child(
                            Button::new(format!("teach-scope-{index}"), option.caption())
                                .toggled(index == t.scope)
                                .disabled(busy)
                                .on_click(cx.listener(move |s, _, _, cx| {
                                    if let Some(t) = s.teaching.as_mut().filter(|t| !t.busy) {
                                        t.select_scope(index);
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    modal = modal.child(scopes);
                } else {
                    let reason = match &t.outcome {
                        Some(teaching::Outcome::Unsafe(reason)) => reason.clone(),
                        _ => teaching::REVIEW_FAILED.to_owned(),
                    };
                    modal = modal
                        .child(teach_text("teach-review-title", "No rule can be learned"))
                        .child(teach_text("teach-unsafe", reason))
                        .child(teach_example(
                            p,
                            "teach-before",
                            "Before",
                            t.entry.text.clone(),
                        ))
                        .child(teach_example(p, "teach-after", "After", corrected.clone()));
                }
                buttons = buttons
                    .child(
                        Button::new("teach-back", "Back")
                            .disabled(busy)
                            .on_click(cx.listener(|s, _, w, cx| s.teaching_back(w, cx))),
                    )
                    .when(!specific, |b| {
                        b.child(
                            Button::new("teach-specific", "Teach specific term")
                                .disabled(busy)
                                .on_click(cx.listener(|s, _, w, cx| s.teach_specific(w, cx))),
                        )
                    })
                    .child(
                        Button::new("teach-save-only", "Save correction only")
                            .disabled(!can_save)
                            .on_click(cx.listener(|s, _, _, cx| s.save_teaching_only(cx))),
                    );
                if t.proposal().is_some() {
                    buttons = buttons.child(
                        Button::new(
                            "teach-remember",
                            if busy {
                                "Saving…"
                            } else {
                                "Remember correction"
                            },
                        )
                        .intent(primary)
                        .disabled(!can_remember)
                        .on_click(cx.listener(|s, _, _, cx| s.remember_teaching(cx))),
                    );
                }
            }
            Step::SpecificEdit => {
                modal = modal
                    .child(teach_text("teach-review-title", "Teach specific term"))
                    .child(
                        Textarea::new(&self.fields.heard)
                            .aria_label("Heard transcript for term selection")
                            .readonly(true)
                            .h(px(110.)),
                    )
                    .child(
                        Button::new("teach-use-selection", "Use selected text")
                            .disabled(busy || !selection)
                            .on_click(cx.listener(|s, _, w, cx| s.use_heard_selection(w, cx))),
                    )
                    .child(TextField::new(&self.fields.source, "Exact heard term").disabled(busy))
                    .child(
                        TextField::new(&self.fields.replacement, "Exact written replacement")
                            .disabled(busy),
                    )
                    .child(teach_text(
                        "teach-hint",
                        "Each side is limited to eight tokens and 256 characters. Heard text must match a whole term in this example.",
                    ));
                buttons = buttons
                    .child(
                        Button::new("teach-back", "Back")
                            .disabled(busy)
                            .on_click(cx.listener(|s, _, w, cx| s.teaching_back(w, cx))),
                    )
                    .child(
                        Button::new("teach-save-only", "Save correction only")
                            .disabled(!can_save)
                            .on_click(cx.listener(|s, _, _, cx| s.save_teaching_only(cx))),
                    )
                    .child(
                        Button::new(
                            "teach-review-specific",
                            if busy {
                                "Reviewing…"
                            } else {
                                "Review specific term"
                            },
                        )
                        .intent(primary)
                        .disabled(!t.can_review_specific(&source, &replacement))
                        .on_click(cx.listener(|s, _, _, cx| s.review_specific(cx))),
                    );
            }
            Step::HistoryRetry => {
                modal = modal
                    .child(teach_example(
                        p,
                        "teach-before",
                        "Before",
                        t.entry.text.clone(),
                    ))
                    .child(teach_example(p, "teach-after", "After", corrected.clone()));
                buttons = buttons
                    .child(Button::new("teach-cancel", "Close").on_click(cx.listener(
                        |s, _, w, cx| {
                            s.close_teaching();
                            s.home.update(cx, |h, cx| h.destination_closed(w, cx));
                            cx.notify();
                        },
                    )))
                    .child(
                        Button::new("teach-save-only", "Save correction only")
                            .intent(primary)
                            .disabled(!can_save)
                            .on_click(cx.listener(|s, _, _, cx| s.save_teaching_only(cx))),
                    );
            }
        }
        let error = t.error.as_ref().map(|error| {
            div()
                .text_color(p.danger)
                .child(teach_text("teach-error", error.clone()))
        });
        if !matches!(t.step, Step::Edit | Step::HistoryRetry) {
            buttons = buttons.child(Button::new("teach-cancel", "Cancel").on_click(cx.listener(
                |s, _, w, cx| {
                    s.close_teaching();
                    s.home.update(cx, |h, cx| h.destination_closed(w, cx));
                    cx.notify();
                },
            )));
        }
        teach_dialog(p, viewport, modal, error, buttons)
    }
}

/// The scrolling content region of the dialog.
fn teach_body() -> Stateful<Div> {
    div()
        .id("teach-body")
        .flex()
        .flex_col()
        .gap_3()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
}

/// Dialog frame bounded by the window. Only `body` scrolls; the title, the
/// error and the actions stay outside it so they are always reachable.
fn teach_dialog(
    p: Palette,
    viewport: Pixels,
    body: Stateful<Div>,
    error: Option<Div>,
    buttons: Div,
) -> Div {
    div()
        .key_context("CorrectAndTeach")
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .w(px(600.))
        .max_h((viewport - px(48.)).max(px(240.)))
        .bg(p.raised)
        .border_1()
        .border_color(p.border)
        .child(
            div()
                .text_size(px(20.))
                .child(teach_text("teach-title", "Correct & Teach")),
        )
        .child(body)
        .children(error.map(|error| error.flex_shrink_0()))
        .child(buttons.flex_shrink_0())
}

// Reads painted bounds through the accessibility test transport.
#[cfg(all(test, feature = "a11y-tests"))]
mod tests {
    use super::*;

    /// A review with far more content than the window can show.
    struct LongReview {
        viewport: Pixels,
    }
    impl Render for LongReview {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let p = Palette::get(cx);
            let long = "synthetic line ".repeat(400);
            let mut body = teach_body().child(teach_example(p, "teach-before", "Before", long));
            for line in 0..60 {
                body = body.child(teach_text(
                    format!("synthetic-line-{line}"),
                    format!("Synthetic scope option {line}"),
                ));
            }
            let buttons = div()
                .flex()
                .flex_row()
                .child(Button::new("teach-cancel", "Cancel").on_click(|_, _, _| {}))
                .child(Button::new("teach-remember", "Remember correction").on_click(|_, _, _| {}));
            let error = Some(div().child(teach_text("teach-error", "Synthetic failure.")));
            teach_dialog(p, self.viewport, body, error, buttons)
        }
    }

    #[gpui::test]
    fn a_long_review_scrolls_and_keeps_the_actions_inside_the_window(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let viewport = px(400.);
        let (_, cx) = cx.add_window_view(|w, cx| {
            let content = cx.new(|_| LongReview { viewport });
            gpui_base::Root::new(content, w, cx)
        });
        let window = cx.update(|w, _| w.window_handle());
        cx.activate_accessibility(window);
        cx.run_until_parked();
        cx.update(|w, cx| w.draw(cx).clear(cx));
        let tree = cx.accessibility_tree(window);
        let scale = cx.update(|w, _| w.scale_factor()) as f64;
        for label in ["Cancel", "Remember correction"] {
            let node = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map(|(_, n)| n)
                .expect("action is rendered");
            let bounds = node.bounds().expect("painted action has bounds");
            assert!(
                bounds.y1 / scale <= f64::from(f32::from(viewport)),
                "{label} ends at {} beyond the {} window",
                bounds.y1 / scale,
                f32::from(viewport)
            );
        }
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
