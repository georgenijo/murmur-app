//! Standalone native Diagnostics. Production host integration stays with Phase 2.
#[cfg(any(test, feature = "gallery"))]
pub mod fixtures;
pub mod host;
pub mod inbox;
pub mod model;
pub mod reports;
#[cfg(test)]
mod tests;
use crate::{
    Button, ButtonIntent, Choice, ChoiceKind, Choices, SelectionChanged, TextField,
    data_components::*,
    field::{InputEvent, TextFieldState},
    tokens::{self, Palette},
};
use gpui::{prelude::*, *};
use host::{Authority, Host, Request, RequestKind};
use inbox::Inbox;
use model::*;
use std::{collections::BTreeSet, rc::Rc};
#[derive(Clone)]
enum Confirm {
    Events,
    Performance,
    Latency,
    Reinitialize,
    Delete(String),
    Upload(String),
}
impl Confirm {
    fn label(&self) -> &'static str {
        match self {
            Self::Events => "Clear events?",
            Self::Performance => "Clear local Performance runs and resource samples?",
            Self::Latency => "Clear local UI latency samples?",
            Self::Reinitialize => "Reinitialize the local diagnostics store?",
            Self::Delete(_) => "Delete this private local capture?",
            Self::Upload(_) => "Upload this reviewed private capture?",
        }
    }
    fn detail(&self) -> &'static str {
        match self {
            Self::Events => "Clears Events only. Performance runs and saved reports remain.",
            Self::Performance => {
                "Clears content-free runs and resource samples only. Events, transcription history, settings and saved reports remain."
            }
            Self::Latency => "Clears local latency samples. No report file is changed.",
            Self::Reinitialize => {
                "Quarantine the unreadable diagnostics database and start a new one. Performance runs and samples are removed. Transcription history, settings, logs and benchmark reports remain."
            }
            Self::Delete(_) => {
                "Permanently remove this private local capture. This cannot be undone."
            }
            Self::Upload(_) => {
                "Send exact reviewed transcript text to the restricted Murmur diagnostics receiver, retained there for at most seven days. The local copy remains until deleted."
            }
        }
    }
}
pub struct Diagnostics {
    pub model: Model,
    host: Rc<dyn Host>,
    inbox: Inbox,
    subscription: Option<Box<dyn host::Subscription>>,
    authority: Authority,
    pub tabs: Entity<Tabs>,
    pub events: Entity<DetailList>,
    charts: Vec<Entity<Chart>>,
    stage_chart: Entity<Chart>,
    correlation: Entity<TextFieldState>,
    field: Entity<Choices>,
    kind: Entity<Choices>,
    outcome: Entity<Choices>,
    baseline: Entity<Choices>,
    candidate: Entity<Choices>,
    build: Entity<Choices>,
    kind_filter: Option<RunKind>,
    outcome_filter: Option<Outcome>,
    build_filter: Option<String>,
    subscriptions: Vec<gpui::Subscription>,
    confirm: Option<Confirm>,
    confirm_ticket: Option<Ticket>,
    confirm_focus: FocusHandle,
    cancel_focus: FocusHandle,
    restore_focus: Option<FocusHandle>,
    report_titles: Vec<String>,
    pub fixture: bool,
}
impl Diagnostics {
    pub fn new(host: Rc<dyn Host>, w: &mut Window, cx: &mut Context<Self>) -> Self {
        let tabs = cx.new(|cx| {
            Tabs::new(
                "Diagnostics views",
                Tab::ALL.iter().map(|t| t.label().into()).collect(),
                cx,
            )
        });
        let events = cx.new(|cx| DetailList::new("Diagnostic events", 100, cx));
        let correlation = cx.new(|cx| TextFieldState::new(w, cx).placeholder("All"));
        let mut choices = |label: &'static str, items: Vec<Choice>, cx: &mut Context<Self>| {
            cx.new(|cx| Choices::new(label, ChoiceKind::Dropdown, items, w, cx))
        };
        let field = choices(
            "Correlation field",
            CorrelationField::ALL
                .iter()
                .enumerate()
                .map(|(i, f)| Choice::new(i.to_string(), f.label()))
                .collect(),
            cx,
        );
        field.update(cx, |s, cx| s.set_value("1", cx));
        let kind = choices(
            "Run kind",
            std::iter::once(Choice::new("all", "All kinds"))
                .chain(
                    RunKind::ALL
                        .iter()
                        .enumerate()
                        .map(|(i, k)| Choice::new(i.to_string(), k.label())),
                )
                .collect(),
            cx,
        );
        let outcome = choices(
            "Run outcome",
            std::iter::once(Choice::new("all", "All outcomes"))
                .chain(
                    Outcome::ALL
                        .iter()
                        .enumerate()
                        .map(|(i, k)| Choice::new(i.to_string(), k.label())),
                )
                .collect(),
            cx,
        );
        kind.update(cx, |s, cx| s.set_value("all", cx));
        outcome.update(cx, |s, cx| s.set_value("all", cx));
        let baseline = choices("Baseline report", vec![], cx);
        let candidate = choices("Candidate report", vec![], cx);
        let build = choices("UI latency build", vec![], cx);
        let charts = [
            ("CPU · host", "%"),
            ("CPU · main process", "%"),
            ("CPU · sidecar", "%"),
            ("RSS · main process", "MB"),
            ("Rust heap · main process", "MB"),
            ("Native heap · main process", "MB"),
            ("RSS · sidecar", "MB"),
        ]
        .into_iter()
        .map(|(label, unit)| cx.new(|cx| Chart::new(label, unit, w, cx)))
        .collect();
        let stage_chart = cx.new(|cx| Chart::new("Stage duration waterfall", "ms", w, cx));
        let mut s = Self {
            model: Model::default(),
            host,
            inbox: Inbox::default(),
            subscription: None,
            authority: Authority::default(),
            tabs: tabs.clone(),
            events,
            charts,
            stage_chart,
            correlation: correlation.clone(),
            field: field.clone(),
            kind: kind.clone(),
            outcome: outcome.clone(),
            baseline: baseline.clone(),
            candidate: candidate.clone(),
            build: build.clone(),
            kind_filter: None,
            outcome_filter: None,
            build_filter: None,
            subscriptions: vec![],
            confirm: None,
            confirm_ticket: None,
            confirm_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            restore_focus: None,
            report_titles: vec![],
            fixture: false,
        };
        s.subscriptions
            .push(cx.subscribe(&tabs, |s, _, e: &TabChanged, cx| {
                s.model.tab = Tab::ALL[e.0];
                s.model.selected_run = None;
                s.model.selected_capture = None;
                s.model.capture_detail = None;
                s.confirm = None;
                s.sync(cx);
                cx.notify();
            }));
        s.subscriptions
            .push(cx.subscribe(&correlation, |s, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    s.model.filters.correlation = s.correlation.read(cx).value().to_string();
                    s.sync(cx);
                    cx.notify();
                }
            }));
        s.subscriptions
            .push(cx.subscribe(&field, |s, _, e: &SelectionChanged, cx| {
                if let Ok(i) = e.0.parse::<usize>() {
                    s.model.filters.field = CorrelationField::ALL[i];
                    s.sync(cx);
                    cx.notify();
                }
            }));
        s.subscriptions
            .push(cx.subscribe(&kind, |s, _, e: &SelectionChanged, cx| {
                s.kind_filter = e.0.parse::<usize>().ok().map(|i| RunKind::ALL[i]);
                cx.notify();
            }));
        s.subscriptions
            .push(cx.subscribe(&outcome, |s, _, e: &SelectionChanged, cx| {
                s.outcome_filter = e.0.parse::<usize>().ok().map(|i| Outcome::ALL[i]);
                cx.notify();
            }));
        s.subscriptions
            .push(cx.subscribe(&baseline, |s, _, e: &SelectionChanged, cx| {
                s.model.baseline = e.0.parse().unwrap_or(0);
                cx.notify();
            }));
        s.subscriptions
            .push(cx.subscribe(&candidate, |s, _, e: &SelectionChanged, cx| {
                s.model.candidate = e.0.parse().unwrap_or(1);
                cx.notify();
            }));
        s.subscriptions
            .push(cx.subscribe(&build, |s, _, e: &SelectionChanged, cx| {
                s.build_filter = Some(e.0.to_string());
                cx.notify();
            }));
        s
    }
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        if self.subscription.is_some() {
            return;
        }
        let attachment = self.model.attach();
        self.inbox = Inbox::default();
        self.authority.sync(&self.model);
        // Subscribe before *any* initial reads, drain queued notifications, then
        // capture hydration epochs. Reopening obtains a new terminal inbox.
        self.subscription = Some(self.host.subscribe(
            murmur_core::events::Consumer::Diagnostics,
            attachment,
            self.inbox.clone(),
        ));
        self.drain(cx);
        for lane in Lane::ALL {
            self.request(lane, RequestKind::Read);
        }
        self.sync(cx);
    }
    pub fn detach(&mut self) {
        self.authority.revoke();
        self.inbox.close();
        if let Some(mut subscription) = self.subscription.take() {
            subscription.stop();
        }
        self.model.detach();
        self.confirm = None;
        self.confirm_ticket = None;
    }
    pub fn follow(&self, cx: &mut Context<Self>) {
        let attachment = self.model.ticket(Lane::Events).attachment;
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let keep = weak
                    .update(cx, |s, cx| {
                        if s.model.ticket(Lane::Events).attachment != attachment
                            || s.subscription.is_none()
                        {
                            return false;
                        }
                        if s.inbox.is_closed() {
                            false
                        } else {
                            if s.inbox.pending() {
                                s.drain(cx);
                                s.sync(cx);
                                cx.notify();
                            }
                            true
                        }
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    /// Refresh derived controls after a host fixture changes presentation state.
    pub fn prepare(&mut self, cx: &mut Context<Self>) {
        self.sync(cx);
        cx.notify();
    }
    pub fn receive(&mut self, event: Event, cx: &mut Context<Self>) {
        self.drain(cx);
        self.model.event(event);
        self.authority.sync(&self.model);
        self.sync(cx);
        cx.notify();
    }
    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut retry = BTreeSet::new();
        for event in self.inbox.drain() {
            let read = if let Event::Read { ticket, .. } = &event {
                Some(*ticket)
            } else {
                None
            };
            if !self.model.event(event)
                && let Some(t) = read
                && self.model.ticket(t.lane).attachment == t.attachment
            {
                retry.insert(t.lane);
            }
        }
        self.authority.sync(&self.model);
        for lane in retry {
            self.request(lane, RequestKind::Read);
        }
        cx.notify();
    }
    fn request(&mut self, lane: Lane, kind: RequestKind) {
        let ticket = self.model.begin_read(lane);
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
    fn sync(&mut self, cx: &mut Context<Self>) {
        let rows = self
            .model
            .filtered_events()
            .into_iter()
            .map(|e| DetailRow {
                id: e.sequence,
                label: e.label().into(),
                detail: if e.data.as_object().is_some_and(|v| v.is_empty()) {
                    SharedString::default()
                } else {
                    serde_json::to_string_pretty(&e.data)
                        .unwrap_or_default()
                        .into()
                },
            })
            .collect();
        self.events.update(cx, |s, cx| s.set_rows(rows, cx));
        for (i, chart) in self.charts.iter().enumerate() {
            let points = self
                .model
                .samples
                .iter()
                .map(|s| ChartPoint {
                    label: s.observed_ms.to_string().into(),
                    value: match i {
                        0 => s.host_cpu.value(),
                        1 => s.main_cpu.value(),
                        2 => s.sidecar_cpu.value(),
                        3 => s.rss.value().map(|v| v / 1048576.),
                        4 => s.rust_heap.value().map(|v| v / 1048576.),
                        5 => s.native_heap.value().map(|v| v / 1048576.),
                        _ => s.sidecar_rss.value().map(|v| v / 1048576.),
                    },
                })
                .collect();
            chart.update(cx, |s, cx| s.set_points(points, cx));
        }
        if let Some(r) = self
            .model
            .selected_run
            .as_ref()
            .and_then(|id| self.model.runs.iter().find(|r| &r.id == id))
        {
            let points = r
                .stages
                .iter()
                .map(|s| ChartPoint {
                    label: format!("{} · {}", s.name, s.outcome).into(),
                    value: s.duration.value(),
                })
                .collect();
            self.stage_chart
                .update(cx, |s, cx| s.set_points(points, cx));
        }
        let titles: Vec<_> = self
            .model
            .reports
            .iter()
            .enumerate()
            .map(|(i, r)| format!("{} · {}", i + 1, r.title()))
            .collect();
        if titles != self.report_titles {
            // Recreate only when membership changes. Ordinary refreshes preserve
            // picker focus and selected report identities.
            self.report_titles = titles;
            let values: Vec<_> = self
                .report_titles
                .iter()
                .enumerate()
                .map(|(i, t)| Choice::new(i.to_string(), t.clone()))
                .collect();
            self.baseline
                .update(cx, |s, cx| s.replace_choices(values.clone(), cx));
            self.candidate
                .update(cx, |s, cx| s.replace_choices(values, cx));
            self.baseline.update(cx, |s, cx| {
                s.set_value(&self.model.baseline.to_string(), cx)
            });
            self.candidate.update(cx, |s, cx| {
                s.set_value(&self.model.candidate.to_string(), cx)
            });
        }
        let builds: BTreeSet<_> = self.model.latency.iter().map(|l| l.build.clone()).collect();
        self.build.update(cx, |s, cx| {
            s.replace_choices(
                builds
                    .into_iter()
                    .map(|b| Choice::new(b.clone(), b))
                    .collect(),
                cx,
            )
        });
        self.tabs.update(cx, |s, cx| {
            s.set_selected(
                Tab::ALL.iter().position(|t| *t == self.model.tab).unwrap(),
                cx,
            )
        });
    }
    fn open_confirm(
        &mut self,
        confirm: Confirm,
        lane: Lane,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_focus = w.focused(cx);
        self.confirm_ticket = Some(self.model.ticket(lane));
        self.confirm = Some(confirm);
        w.focus(&self.cancel_focus, cx);
        cx.notify();
    }
    fn dismiss(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        self.confirm_ticket = None;
        if let Some(focus) = self.restore_focus.take() {
            w.focus(&focus, cx);
        }
        cx.notify();
    }
    fn confirm(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.drain(cx);
        let Some(ticket) = self.confirm_ticket else {
            return;
        };
        if !self.model.current(ticket) {
            self.model.notice =
                Some("Diagnostics changed. Review the current data before confirming again.");
            self.dismiss(w, cx);
            return;
        }
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        let kind = match confirm {
            Confirm::Events => RequestKind::ClearEvents,
            Confirm::Performance => RequestKind::ClearPerformance,
            Confirm::Latency => RequestKind::ClearLatency,
            Confirm::Reinitialize => {
                if !self
                    .model
                    .health
                    .as_ref()
                    .is_some_and(|h| h.can_recover() && h.recommended == Recovery::Reinitialize)
                {
                    self.dismiss(w, cx);
                    return;
                }
                RequestKind::Recover {
                    allow_reinitialize: true,
                }
            }
            Confirm::Delete(capture_id) => RequestKind::DeleteCapture { capture_id },
            Confirm::Upload(capture_id) => RequestKind::UploadCapture { capture_id },
        };
        self.request(ticket.lane, kind);
        self.dismiss(w, cx);
    }
    fn toolbar(&self) -> Div {
        div().flex().flex_wrap().items_center().gap(px(8.))
    }
    fn table(
        &self,
        id: &'static str,
        label: &str,
        headers: &[&str],
        rows: Vec<Vec<String>>,
    ) -> DataTable {
        DataTable::new(
            id,
            label.to_owned(),
            headers,
            rows.into_iter()
                .map(|r| r.into_iter().map(Into::into).collect())
                .collect(),
        )
    }
    fn health(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut d = div()
            .id("store-health")
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(tokens::CARD_RADIUS)
            .border_1()
            .border_color(Palette::get(cx).border);
        if self.model.loading.contains(&Lane::Health) && self.model.health.is_none() {
            return d
                .child(Progress::new("Checking diagnostics store", None))
                .into_any_element();
        }
        if self.model.errors.contains(&Lane::Health) || self.model.health.is_none() {
            return d
                .role(Role::Alert)
                .child("Diagnostics health could not be verified. Dictation remains available.")
                .child(
                    Button::new("health-refresh", "Refresh store health").on_click(
                        cx.listener(|s, _, _, _| s.request(Lane::Health, RequestKind::Read)),
                    ),
                )
                .into_any_element();
        }
        let h = self.model.health.as_ref().unwrap();
        d = d
            .role(if h.available {
                Role::Status
            } else {
                Role::Alert
            })
            .child(h.guidance());
        if h.skipped > 0 {
            d = d.child(format!("{} diagnostics runs skipped", h.skipped));
        }
        if h.can_recover() {
            d = d.child(
                Button::new(
                    "recover",
                    if h.recommended == Recovery::Reinitialize {
                        "Reinitialize store…"
                    } else {
                        "Retry store"
                    },
                )
                .disabled(self.model.loading.contains(&Lane::Health))
                .on_click(cx.listener(|s, _, w, cx| {
                    if s.model
                        .health
                        .as_ref()
                        .is_some_and(|h| h.recommended == Recovery::Reinitialize)
                    {
                        s.open_confirm(Confirm::Reinitialize, Lane::Health, w, cx);
                    } else {
                        s.request(
                            Lane::Health,
                            RequestKind::Recover {
                                allow_reinitialize: false,
                            },
                        );
                    }
                })),
            );
        }
        d.into_any_element()
    }
    fn event_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut filters = self.toolbar();
        for stream in Stream::ALL {
            filters = filters.child(
                Button::new(
                    ("stream", stream as usize),
                    format!(
                        "{} {}",
                        if self.model.filters.streams.contains(&stream) {
                            "✓"
                        } else {
                            "○"
                        },
                        stream.label()
                    ),
                )
                .toggled(self.model.filters.streams.contains(&stream))
                .size(crate::ButtonSize::Compact)
                .on_click(cx.listener(move |s, _, _, cx| {
                    if !s.model.filters.streams.remove(&stream) {
                        s.model.filters.streams.insert(stream);
                    }
                    s.sync(cx);
                    cx.notify();
                })),
            );
        }
        let mut levels = self.toolbar();
        for level in Level::ALL {
            levels = levels.child(
                Button::new(
                    ("level", level as usize),
                    format!(
                        "{} {}",
                        if self.model.filters.levels.contains(&level) {
                            "✓"
                        } else {
                            "○"
                        },
                        level.label()
                    ),
                )
                .toggled(self.model.filters.levels.contains(&level))
                .size(crate::ButtonSize::Compact)
                .on_click(cx.listener(move |s, _, _, cx| {
                    if !s.model.filters.levels.remove(&level) {
                        s.model.filters.levels.insert(level);
                    }
                    s.sync(cx);
                    cx.notify();
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .flex_1()
            .gap(px(8.))
            .child(
                self.toolbar()
                    .child(
                        Button::new("copy-events", "Copy filtered events")
                            .disabled(self.model.filtered_events().is_empty())
                            .on_click(cx.listener(|s, _, _, _| {
                                s.request(
                                    Lane::Events,
                                    RequestKind::CopyEvents(s.model.copy_events()),
                                )
                            })),
                    )
                    .child(
                        Button::new("clear-events", "Clear events…").on_click(cx.listener(
                            |s, _, w, cx| s.open_confirm(Confirm::Events, Lane::Events, w, cx),
                        )),
                    ),
            )
            .child(filters)
            .child(levels)
            .child(
                self.toolbar()
                    .child(self.field.clone())
                    .child(div().w(px(240.)).child(TextField::new(
                        &self.correlation,
                        format!("Filter by {}", self.model.filters.field.label()),
                    ))),
            )
            .when(self.model.loading.contains(&Lane::Events), |d| {
                d.child(Progress::new("Loading events", None))
            })
            .when(self.model.events.is_empty(), |d| {
                d.child("No events to display")
            })
            .child(self.events.clone())
            .into_any_element()
    }
    fn runs_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(r) = self
            .model
            .selected_run
            .as_ref()
            .and_then(|id| self.model.runs.iter().find(|r| &r.id == id))
        {
            return div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    Button::new("back-runs", "Back to runs").on_click(cx.listener(
                        |s, _, _, cx| {
                            s.model.selected_run = None;
                            cx.notify();
                        },
                    )),
                )
                .child(format!(
                    "{} · {} · {}",
                    r.kind.label(),
                    r.id,
                    r.outcome.label()
                ))
                .child(self.stage_chart.clone())
                .child(
                    self.table(
                        "stage-table",
                        "Run stage timings",
                        &["Stage", "Outcome", "Duration"],
                        r.stages
                            .iter()
                            .map(|s| {
                                vec![s.name.clone(), s.outcome.clone(), s.duration.display("ms")]
                            })
                            .collect(),
                    ),
                )
                .child(
                    self.table(
                        "resources",
                        "Run resource ranges",
                        &["Resource", "Start", "Average", "Peak", "End"],
                        r.resources
                            .iter()
                            .map(|v| {
                                vec![
                                    v.label.clone(),
                                    v.start.display(v.unit),
                                    v.average.display(v.unit),
                                    v.peak.display(v.unit),
                                    v.end.display(v.unit),
                                ]
                            })
                            .collect(),
                    ),
                )
                .child(
                    self.table(
                        "run-facts",
                        "Runtime, input, cohort, capture and follow-up facts",
                        &["Fact", "Value"],
                        r.facts
                            .iter()
                            .map(|(k, v)| vec![k.clone(), v.clone()])
                            .collect(),
                    ),
                )
                .child(
                    Button::new("show-events", "Show correlated events").on_click(cx.listener(
                        |s, _, w, cx| {
                            if let Some(r) = s
                                .model
                                .selected_run
                                .as_ref()
                                .and_then(|id| s.model.runs.iter().find(|r| &r.id == id))
                            {
                                s.model.filters.field = r.correlation.0;
                                s.model.filters.correlation = r.correlation.1.clone();
                                s.model.filters.streams = Stream::ALL.into();
                                s.model.filters.levels = Level::ALL.into();
                                s.model.tab = Tab::Events;
                                s.field.update(cx, |f, cx| {
                                    f.set_value(
                                        &CorrelationField::ALL
                                            .iter()
                                            .position(|f| *f == s.model.filters.field)
                                            .unwrap()
                                            .to_string(),
                                        cx,
                                    )
                                });
                                s.correlation.update(cx, |f, cx| {
                                    f.set_value(s.model.filters.correlation.clone(), w, cx)
                                });
                                s.sync(cx);
                                cx.notify();
                            }
                        },
                    )),
                )
                .into_any_element();
        }
        let runs: Vec<_> = self
            .model
            .runs
            .iter()
            .filter(|r| {
                self.kind_filter.is_none_or(|k| k == r.kind)
                    && self.outcome_filter.is_none_or(|o| o == r.outcome)
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.health(cx))
            .child(
                self.toolbar()
                    .child(self.kind.clone())
                    .child(self.outcome.clone())
                    .child(Button::new("refresh-runs", "Refresh runs").on_click(
                        cx.listener(|s, _, _, _| s.request(Lane::Performance, RequestKind::Read)),
                    ))
                    .child(
                        Button::new("clear-performance", "Clear performance data…").on_click(
                            cx.listener(|s, _, w, cx| {
                                s.open_confirm(Confirm::Performance, Lane::Performance, w, cx)
                            }),
                        ),
                    ),
            )
            .child("Newest 200 local, content-free performance records")
            .when(self.model.loading.contains(&Lane::Performance), |d| {
                d.child(Progress::new("Loading performance runs", None))
            })
            .when(runs.is_empty(), |d| d.child("No runs match these filters"))
            .child(
                self.table(
                    "runs-table",
                    "Performance runs",
                    &[
                        "Run",
                        "Runtime",
                        "Outcome",
                        "Input",
                        "Latency",
                        "Rate",
                        "Peak resources",
                    ],
                    runs.iter()
                        .map(|r| {
                            vec![
                                format!("{} · {}", r.id, r.kind.label()),
                                r.runtime.clone(),
                                r.outcome.label().into(),
                                r.input.clone(),
                                r.latency.display("ms"),
                                r.rate.display(r.rate_unit),
                                r.resources
                                    .iter()
                                    .map(|v| format!("{} {}", v.label, v.peak.display(v.unit)))
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            ]
                        })
                        .collect(),
                )
                .with_row_actions(
                    runs.into_iter()
                        .map(|r| {
                            let id = r.id.clone();
                            Button::new(
                                SharedString::from(format!("run-{id}")),
                                format!("View details for {} · {}", r.kind.label(), id),
                            )
                            .caption("Details")
                            .on_click(cx.listener(
                                move |s, _, _, cx| {
                                    s.model.selected_run = Some(id.clone());
                                    s.sync(cx);
                                    cx.notify();
                                },
                            ))
                        })
                        .collect(),
                ),
            )
            .into_any_element()
    }
    fn performance_view(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.health(cx))
            .child(
                Button::new("refresh-performance", "Refresh performance").on_click(cx.listener(
                    |s, _, _, _| {
                        s.request(Lane::Performance, RequestKind::Read);
                        s.request(Lane::Health, RequestKind::Read);
                    },
                )),
            )
            .child("Content-free resources · missing observations remain unavailable")
            .when(self.model.loading.contains(&Lane::Performance), |d| {
                d.child(Progress::new("Loading resource samples", None))
            })
            .children(self.charts.iter().cloned())
            .into_any_element()
    }
    fn compare_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut d =
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child("Report comparison · explicit JSON imports stay in this Diagnostics session")
                .child(
                    self.toolbar()
                        .child(
                            Button::new("import", "Import JSON")
                                .disabled(
                                    self.model
                                        .reports
                                        .iter()
                                        .filter(|r| r.source == reports::Source::Imported)
                                        .count()
                                        >= reports::MAX_IMPORTS,
                                )
                                .on_click(cx.listener(|s, _, _, _| {
                                    s.request(Lane::Reports, RequestKind::ImportReport)
                                })),
                        )
                        .child(Button::new("clear-imports", "Clear imports").on_click(
                            cx.listener(|s, _, _, cx| {
                                s.model.clear_imports();
                                s.authority.sync(&s.model);
                                s.sync(cx);
                                cx.notify();
                            }),
                        )),
                )
                .child(
                    self.toolbar()
                        .child(self.baseline.clone())
                        .child(self.candidate.clone()),
                );
        if let Some(e) = self.model.import_error {
            d = d.child(div().id("error").role(Role::Alert).child(e.message()));
        }
        for (title, index) in [
            ("Baseline", self.model.baseline),
            ("Candidate", self.model.candidate),
        ] {
            if let Some(r) = self.model.reports.get(index) {
                d = d
                    .child(format!("{title}: {}", r.title()))
                    .child(r.privacy_notice())
                    .child(self.table(
                        if title == "Baseline" {
                            "baseline-meta"
                        } else {
                            "candidate-meta"
                        },
                        title,
                        &["Field", "Value"],
                        r.metadata().into_iter().map(|(k, v)| vec![k, v]).collect(),
                    ));
            }
        }
        if self.model.baseline == self.model.candidate {
            return d
                .child("Choose two different reports to compare.")
                .into_any_element();
        }
        if let (Some(a), Some(b)) = (
            self.model.reports.get(self.model.baseline),
            self.model.reports.get(self.model.candidate),
        ) {
            let c = reports::compare(a, b);
            d = d.child(
                div()
                    .id("notice")
                    .role(Role::Status)
                    .child(format!("Compatibility gate · {}", c.status())),
            );
            for f in &c.findings {
                d = d.child(
                    div()
                        .id(f.code)
                        .role(if f.blocker { Role::Alert } else { Role::Status })
                        .child(format!(
                            "{} · {}: {}",
                            if f.blocker { "Blocker" } else { "Warning" },
                            f.code,
                            f.message
                        )),
                );
            }
            if c.blocked() {
                d=d.child("Deltas and recommendations are unavailable while compatibility blockers remain.");
            } else {
                d = d.child(
                    self.table(
                        "report-metrics",
                        "Comparison deltas",
                        &[
                            "Metric / scope",
                            "Baseline",
                            "Candidate",
                            "Absolute Δ",
                            "Percentage Δ",
                        ],
                        c.metrics
                            .iter()
                            .take(500)
                            .map(|m| {
                                vec![
                                    format!("{} · {}", m.scope, m.label),
                                    format!("{:.2} {}", m.baseline, m.unit),
                                    format!("{:.2} {}", m.candidate, m.unit),
                                    format!("{:+.2} {}", m.delta, m.unit),
                                    m.percent
                                        .map(|p| format!("{p:+.2}%"))
                                        .unwrap_or_else(|| "Unavailable · zero baseline".into()),
                                ]
                            })
                            .collect(),
                    ),
                );
                if c.metrics.len() > 500 {
                    d = d.child(format!(
                        "Showing first 500 of {} bounded metrics",
                        c.metrics.len()
                    ));
                }
                if let Some(r) = c.recommendations {
                    d = d.child(format!("Recommendation eligibility: eligible · {r}"));
                } else {
                    d=d.child("Recommendation eligibility: no ranking or recommendation from this comparison.");
                }
            }
        } else {
            d=d.child("Import two diagnostic reports, or choose saved Performance Lab reports. No reports are run automatically.");
        }
        d.into_any_element()
    }
    fn latency_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let filtered: Vec<_> = self
            .model
            .latency
            .iter()
            .filter(|l| self.build_filter.as_ref().is_none_or(|b| b == &l.build))
            .collect();
        div().flex().flex_col().gap(px(12.)).child("UI latency map · content-free observations by build")
            .child(self.toolbar().child(self.build.clone()).child(Button::new("copy-latency","Copy latency JSON").disabled(filtered.is_empty()).on_click(cx.listener(|s,_,_,_|{
                let values:Vec<_>=s.model.latency.iter().filter(|l|s.build_filter.as_ref().is_none_or(|b|b==&l.build)).map(|l|serde_json::json!({"build":l.build,"from":l.from,"to":l.to,"count":l.count,"firstFrameMs":l.first_frame.value(),"commitMs":l.commit.value(),"paintedMs":l.painted.value(),"p95Ms":l.p95.value()})).collect();
                s.request(Lane::Latency,RequestKind::CopyLatency(serde_json::json!({"schemaVersion":1,"edges":values}).to_string()));
            }))).child(Button::new("clear-latency","Clear latency samples…").on_click(cx.listener(|s,_,w,cx|s.open_confirm(Confirm::Latency,Lane::Latency,w,cx)))))
            .child(self.table("latency-table","UI latency edges",&["Build / route","Samples","First frame","Commit","Paint proxy","p95"],filtered.iter().map(|l|vec![format!("{} · {} → {}",l.build,l.from,l.to),l.count.to_string(),l.first_frame.display("ms"),l.commit.display("ms"),l.painted.display("ms"),l.p95.display("ms")]).collect()))
            .when(filtered.is_empty(),|d|d.child("No latency samples for this build")).into_any_element()
    }
    fn dictation_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut d=div().flex().flex_col().gap(px(12.)).child("Private dictation captures · arm or disarm from the main Murmur window")
            .child("One-shot consent expires after ten minutes. Captures remain local for at most seven days. Upload requires separate review and confirmation.")
            .child(Button::new("refresh-captures","Refresh private captures").on_click(cx.listener(|s,_,_,_|s.request(Lane::Captures,RequestKind::Read))));
        if let Some(c) = self.model.capture_detail.as_ref() {
            d = d.child(
                Button::new("back-captures", "Back to private captures").on_click(cx.listener(
                    |s, _, _, cx| {
                        s.model.selected_capture = None;
                        s.model.capture_detail = None;
                        cx.notify();
                    },
                )),
            );
            for (label, text) in &c.sections {
                d = d.child(
                    div()
                        .id(SharedString::from(label.clone()))
                        .role(Role::Group)
                        .aria_label(label.clone())
                        .child(label.clone())
                        .child(
                            div()
                                .id("private-text")
                                .role(Role::Label)
                                .aria_value(text.clone())
                                .font_family("Menlo")
                                .p(px(12.))
                                .bg(Palette::get(cx).raised)
                                .child(text.clone()),
                        ),
                );
            }
            let id = c.id.clone();
            d = d.child(
                Button::new("upload-capture", "Upload reviewed private capture…").on_click(
                    cx.listener(move |s, _, w, cx| {
                        s.open_confirm(Confirm::Upload(id.clone()), Lane::Captures, w, cx)
                    }),
                ),
            );
        } else {
            for c in &self.model.captures {
                let id = c.id.clone();
                d = d.child(
                    Button::new(
                        SharedString::from(format!("capture-{id}")),
                        format!(
                            "Review capture · recording {} · created {} · expires {}{}",
                            c.recording_id,
                            c.created_ms,
                            c.expires_ms,
                            if c.uploaded { " · Uploaded" } else { "" }
                        ),
                    )
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.model.selected_capture = Some(id.clone());
                        s.model.capture_detail = None;
                        s.request(
                            Lane::Captures,
                            RequestKind::ReadCapture {
                                capture_id: id.clone(),
                            },
                        );
                        cx.notify();
                    })),
                );
                let id = c.id.clone();
                d = d.child(
                    Button::new(
                        SharedString::from(format!("delete-{id}")),
                        format!("Delete local capture for recording {}…", c.recording_id),
                    )
                    .on_click(cx.listener(move |s, _, w, cx| {
                        s.open_confirm(Confirm::Delete(id.clone()), Lane::Captures, w, cx)
                    })),
                );
            }
        }
        if self.model.captures.is_empty() {
            d = d.child("No private captures");
        }
        d.into_any_element()
    }
}
impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.detach();
    }
}
impl Render for Diagnostics {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.fixture {
            crate::preferences::refresh(cx);
        }
        let p = Palette::get(cx);
        let panel = match self.model.tab {
            Tab::Events => self.event_view(cx),
            Tab::Runs => self.runs_view(cx),
            Tab::Performance => self.performance_view(cx),
            Tab::Compare => self.compare_view(cx),
            Tab::Latency => self.latency_view(cx),
            Tab::Dictation => self.dictation_view(cx),
        };
        let mut root = div()
            .id("diagnostics-root")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .bg(p.surface)
            .text_color(p.text)
            .font_family(".SystemUIFont")
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING);
        if let Some(confirm) = self.confirm.clone() {
            // Only the dialog is mounted in the modal frame; background controls
            // leave the AX tree and Tab order. Escape and Cancel restore focus.
            root = root.child(
                div()
                    .id("confirmation")
                    .role(Role::AlertDialog)
                    .aria_label(confirm.label())
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(24.))
                    .border_1()
                    .border_color(p.border)
                    .rounded(tokens::CARD_RADIUS)
                    .capture_key_down(cx.listener(|s, e: &KeyDownEvent, w, cx| {
                        if e.keystroke.key == "escape" {
                            w.prevent_default();
                            cx.stop_propagation();
                            s.dismiss(w, cx);
                        } else if e.keystroke.key == "tab" {
                            w.prevent_default();
                            cx.stop_propagation();
                            if s.cancel_focus.is_focused(w) {
                                w.focus(&s.confirm_focus, cx);
                            } else {
                                w.focus(&s.cancel_focus, cx);
                            }
                        }
                    }))
                    .child(confirm.label())
                    .child(confirm.detail())
                    .child(
                        self.toolbar()
                            .child(
                                Button::new("cancel", "Cancel")
                                    .track_focus(&self.cancel_focus)
                                    .on_click(cx.listener(|s, _, w, cx| s.dismiss(w, cx))),
                            )
                            .child(
                                Button::new("confirm", "Confirm")
                                    .intent(ButtonIntent::Danger)
                                    .track_focus(&self.confirm_focus)
                                    .on_click(cx.listener(|s, _, w, cx| s.confirm(w, cx))),
                            ),
                    ),
            );
            return root;
        }
        root = root.child(self.tabs.clone());
        if let Some(notice) = self.model.notice {
            root = root.child(div().id("notice").role(Role::Status).child(notice));
        }
        if self.model.errors.contains(&match self.model.tab {
            Tab::Events => Lane::Events,
            Tab::Runs | Tab::Performance => Lane::Performance,
            Tab::Latency => Lane::Latency,
            Tab::Compare => Lane::Reports,
            Tab::Dictation => Lane::Captures,
        }) {
            root=root.child(div().id("error").role(Role::Alert).child("Diagnostics could not be refreshed. Retry to reconnect; existing data remains visible.").child(Button::new("retry","Retry diagnostics").on_click(cx.listener(|s,_,_,_|{for l in Lane::ALL{s.request(l,RequestKind::Read);}}))));
        }
        root.child(
            div()
                .id("diagnostics-panel")
                .role(Role::TabPanel)
                .aria_label(self.model.tab.label())
                .flex_1()
                .min_h_0()
                .when(self.model.tab != Tab::Events, |d| d.overflow_y_scroll())
                .flex()
                .flex_col()
                .child(panel),
        )
    }
}
