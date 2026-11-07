//! Host-neutral presentation contracts. Services retain recording, storage,
//! redaction and permission ownership. No model here implements Debug/Serialize.
use super::reports::{ImportError, MAX_IMPORTS, Report, Source};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_EVENTS: usize = 500;
pub const MAX_RUNS: usize = 200;
pub const MAX_SAMPLES: usize = 600;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Events,
    Runs,
    Performance,
    Latency,
    Compare,
    Dictation,
}
impl Tab {
    pub const ALL: [Self; 6] = [
        Self::Events,
        Self::Runs,
        Self::Performance,
        Self::Latency,
        Self::Compare,
        Self::Dictation,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Events => "Events",
            Self::Runs => "Runs",
            Self::Performance => "Performance",
            Self::Latency => "Latency",
            Self::Compare => "Compare",
            Self::Dictation => "Dictation",
        }
    }
    pub fn from_route(s: &str) -> Option<Self> {
        match s {
            "events" => Some(Self::Events),
            "runs" => Some(Self::Runs),
            "performance" => Some(Self::Performance),
            "latency" => Some(Self::Latency),
            "reports" => Some(Self::Compare),
            "dictation" => Some(Self::Dictation),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stream {
    Pipeline,
    Audio,
    Keyboard,
    Transform,
    Meeting,
    Query,
    System,
}
impl Stream {
    pub const ALL: [Self; 7] = [
        Self::Pipeline,
        Self::Audio,
        Self::Keyboard,
        Self::Transform,
        Self::Meeting,
        Self::Query,
        Self::System,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline",
            Self::Audio => "audio",
            Self::Keyboard => "keyboard",
            Self::Transform => "transform",
            Self::Meeting => "meeting",
            Self::Query => "query",
            Self::System => "system",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrelationField {
    Run,
    Recording,
    FileRun,
    TransformPass,
    QueryPass,
}
impl CorrelationField {
    pub const ALL: [Self; 5] = [
        Self::Run,
        Self::Recording,
        Self::FileRun,
        Self::TransformPass,
        Self::QueryPass,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Run => "Run ID",
            Self::Recording => "Recording ID",
            Self::FileRun => "File run ID",
            Self::TransformPass => "Transform pass ID",
            Self::QueryPass => "Query pass ID",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Run => "run_id",
            Self::Recording => "recording_id",
            Self::FileRun => "file_run_id",
            Self::TransformPass => "transform_pass_id",
            Self::QueryPass => "query_pass_id",
        }
    }
}
/// Construct only from the existing sanitized telemetry sink, never tracing a
/// partial transcript into a new sink. Bounds reject, rather than truncate JSON.
#[derive(Clone)]
pub struct EventRow {
    pub sequence: u64,
    pub timestamp: String,
    pub stream: Stream,
    pub level: Level,
    pub summary: String,
    pub data: serde_json::Value,
}
impl EventRow {
    pub fn valid(&self) -> bool {
        self.sequence > 0
            && self.timestamp.len() <= 128
            && self.summary.len() <= 8192
            && self.data.is_object()
            && self.data.to_string().len() <= 16384
    }
    pub fn label(&self) -> String {
        format!(
            "{} [{}] {} {}",
            self.timestamp,
            self.stream.label(),
            self.level.label().to_uppercase(),
            self.summary
        )
    }
    pub fn copy_line(&self) -> String {
        format!(
            "{}{}",
            self.label(),
            if self.data.as_object().is_none_or(|v| v.is_empty()) {
                String::new()
            } else {
                format!(" {}", self.data)
            }
        )
    }
}
#[derive(Clone)]
pub struct Filters {
    pub streams: BTreeSet<Stream>,
    pub levels: BTreeSet<Level>,
    pub field: CorrelationField,
    pub correlation: String,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            streams: [
                Stream::Pipeline,
                Stream::Audio,
                Stream::Transform,
                Stream::Query,
                Stream::System,
            ]
            .into(),
            levels: [Level::Info, Level::Warn, Level::Error].into(),
            field: CorrelationField::Recording,
            correlation: String::new(),
        }
    }
}
impl Filters {
    pub fn matches(&self, e: &EventRow) -> bool {
        if !self.streams.contains(&e.stream) || !self.levels.contains(&e.level) {
            return false;
        }
        let query = self.correlation.trim();
        if query.is_empty() {
            return true;
        }
        let value = &e.data[self.field.key()];
        if self.field == CorrelationField::Run {
            return value.as_str() == Some(query);
        }
        query
            .as_bytes()
            .first()
            .is_some_and(|c| (b'1'..=b'9').contains(c))
            && query.bytes().all(|b| b.is_ascii_digit())
            && query
                .parse::<u64>()
                .ok()
                .filter(|n| *n <= 9_007_199_254_740_991)
                .is_some_and(|n| value.as_u64() == Some(n))
    }
}
#[derive(Clone)]
pub enum Measurement {
    Measured(f64),
    NotApplicable,
    Unavailable(&'static str),
}
impl Measurement {
    pub fn value(&self) -> Option<f64> {
        match self {
            Self::Measured(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }
    pub fn display(&self, unit: &str) -> String {
        match self {
            Self::Measured(v) if v.is_finite() => format!("{v:.2} {unit}"),
            Self::NotApplicable => "Not applicable".into(),
            Self::Unavailable(reason) => format!("Unavailable · {reason}"),
            _ => "Unavailable".into(),
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Dictation,
    FileTranscription,
    SelectedTextTransform,
    VoiceQuery,
}
impl RunKind {
    pub const ALL: [Self; 4] = [
        Self::Dictation,
        Self::FileTranscription,
        Self::SelectedTextTransform,
        Self::VoiceQuery,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Dictation => "Dictation",
            Self::FileTranscription => "File transcription",
            Self::SelectedTextTransform => "Selected text transform",
            Self::VoiceQuery => "Voice Query (retired)",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    NoSpeech,
    Cancelled,
    TimedOut,
    Failed,
    Interrupted,
}
impl Outcome {
    pub const ALL: [Self; 6] = [
        Self::Success,
        Self::NoSpeech,
        Self::Cancelled,
        Self::TimedOut,
        Self::Failed,
        Self::Interrupted,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::NoSpeech => "No speech",
            Self::Cancelled => "Cancelled",
            Self::TimedOut => "Timed out",
            Self::Failed => "Failed",
            Self::Interrupted => "Interrupted",
        }
    }
}
#[derive(Clone)]
pub struct Stage {
    pub name: String,
    pub outcome: String,
    pub duration: Measurement,
}
#[derive(Clone)]
pub struct ResourceRange {
    pub label: String,
    pub unit: &'static str,
    pub start: Measurement,
    pub average: Measurement,
    pub peak: Measurement,
    pub end: Measurement,
}
/// Presentation projection of PerformanceRunV1. `facts` are content-free DTO
/// fields (cohort/startup/input/runtime/follow-up); no transcripts or file paths.
#[derive(Clone)]
pub struct Run {
    pub id: String,
    pub kind: RunKind,
    pub finished_ms: u64,
    pub outcome: Outcome,
    pub correlation: (CorrelationField, String),
    pub runtime: String,
    pub input: String,
    pub latency: Measurement,
    pub rate: Measurement,
    pub rate_unit: &'static str,
    pub resources: Vec<ResourceRange>,
    pub stages: Vec<Stage>,
    pub facts: Vec<(String, String)>,
}
impl Run {
    fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 128
            && self.runtime.len() <= 1024
            && self.input.len() <= 512
            && self.stages.len() <= 64
            && self.resources.len() <= 8
            && self.resources.iter().all(|r| r.label.len() <= 256)
            && self.correlation.1.len() <= 128
            && self.facts.len() <= 128
            && self
                .facts
                .iter()
                .all(|(k, v)| k.len() <= 128 && v.len() <= 2048)
            && self
                .stages
                .iter()
                .all(|s| s.name.len() <= 128 && s.outcome.len() <= 128)
    }
}
#[derive(Clone)]
pub struct Sample {
    pub observed_ms: u64,
    pub host_cpu: Measurement,
    pub main_cpu: Measurement,
    pub sidecar_cpu: Measurement,
    pub rss: Measurement,
    pub rust_heap: Measurement,
    pub native_heap: Measurement,
    pub sidecar_rss: Measurement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    None,
    Retry,
    FreeDisk,
    CheckPermissions,
    Reinitialize,
    Restart,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StoreProblem {
    CorruptIntegrity,
    InvalidRecord,
    SchemaMigration,
}
#[derive(Clone)]
pub struct Health {
    pub available: bool,
    pub recommended: Recovery,
    pub problem: Option<StoreProblem>,
    pub skipped: u64,
    pub recent_write_failed: bool,
    pub recovered: bool,
}
impl Health {
    pub fn guidance(&self) -> &'static str {
        if self.available {
            if self.skipped > 0 {
                "Diagnostics runs were skipped. Dictation continued normally."
            } else if self.recent_write_failed {
                "Recent diagnostics data was not saved. Dictation continued normally."
            } else if self.recovered {
                "Diagnostics store recovered. The unreadable store was quarantined."
            } else {
                "Diagnostics store available. Content-free runs and samples stay local."
            }
        } else {
            match self.recommended {
                Recovery::FreeDisk => {
                    "Free local disk space, then retry. Dictation remains available."
                }
                Recovery::CheckPermissions => {
                    "Check local store write permissions, then retry. Dictation remains available."
                }
                Recovery::Reinitialize => match self.problem {
                    Some(StoreProblem::CorruptIntegrity) => {
                        "Corrupt diagnostics store. Reinitialize only after confirmation; the old store is quarantined."
                    }
                    Some(StoreProblem::InvalidRecord) => {
                        "Invalid diagnostics data was rejected. Reinitialize only after confirmation; the old store is quarantined."
                    }
                    Some(StoreProblem::SchemaMigration) => {
                        "Diagnostics schema migration failed. Reinitialize only after confirmation; the old store is quarantined."
                    }
                    None => {
                        "Unreadable diagnostics store. Reinitialize only after confirmation; the old store is quarantined."
                    }
                },
                Recovery::Restart => {
                    "Quit and reopen Murmur. A newer or incompatible store cannot be reinitialized here."
                }
                _ => "Diagnostics store unavailable. Dictation remains available.",
            }
        }
    }
    pub fn can_recover(&self) -> bool {
        !self.available && !matches!(self.recommended, Recovery::None | Recovery::Restart)
    }
}
#[derive(Clone)]
pub struct Latency {
    pub build: String,
    pub from: String,
    pub to: String,
    pub count: u64,
    pub first_frame: Measurement,
    pub commit: Measurement,
    pub painted: Measurement,
    pub p95: Measurement,
}
/// Exact private text is intentionally separate and has no formatter/serializer.
/// It may only enter the selected private review surface, never events or copy.
#[derive(Clone)]
pub struct PrivateCapture {
    pub id: String,
    pub recording_id: u64,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub uploaded: bool,
    pub sections: Vec<(String, String)>,
}
impl PrivateCapture {
    fn valid(&self) -> bool {
        self.id.len() <= 128
            && self.sections.len() <= 16
            && self
                .sections
                .iter()
                .all(|(k, v)| k.len() <= 128 && v.len() <= 8192)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    Events,
    Performance,
    Health,
    Reports,
    Latency,
    Captures,
    Route,
}
impl Lane {
    pub const ALL: [Self; 7] = [
        Self::Events,
        Self::Performance,
        Self::Health,
        Self::Reports,
        Self::Latency,
        Self::Captures,
        Self::Route,
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub attachment: u64,
    pub lane: Lane,
    pub epoch: u64,
    pub store_epoch: u64,
}
#[derive(Default)]
struct Clock {
    revision: u64,
    epoch: u64,
}
pub enum Payload {
    Events(Vec<EventRow>),
    Event(EventRow),
    Performance {
        runs: Vec<Run>,
        samples: Vec<Sample>,
    },
    Run(Run),
    Sample(Sample),
    Health(Health),
    Reports(Vec<Report>),
    Latency(Vec<Latency>),
    Captures(Vec<PrivateCapture>),
    CaptureDetail(PrivateCapture),
    Route(Tab),
    Failed,
}
impl Payload {
    fn lane(&self) -> Option<Lane> {
        match self {
            Self::Events(_) | Self::Event(_) => Some(Lane::Events),
            Self::Performance { .. } | Self::Run(_) | Self::Sample(_) => Some(Lane::Performance),
            Self::Health(_) => Some(Lane::Health),
            Self::Reports(_) => Some(Lane::Reports),
            Self::Latency(_) => Some(Lane::Latency),
            Self::Captures(_) | Self::CaptureDetail(_) => Some(Lane::Captures),
            Self::Route(_) => Some(Lane::Route),
            Self::Failed => None,
        }
    }
}
pub enum Event {
    Read {
        ticket: Ticket,
        revision: u64,
        payload: Payload,
    },
    Notification {
        attachment: u64,
        lane: Lane,
        revision: u64,
        store_epoch: u64,
        payload: Payload,
    },
    /// Service-owned clear epoch prevents queued pre-clear completions from
    /// repopulating after either this client or another named consumer clears.
    Cleared {
        attachment: u64,
        lane: Lane,
        revision: u64,
        store_epoch: u64,
    },
    Import {
        ticket: Ticket,
        result: Result<Option<Report>, ImportError>,
    },
    Action {
        ticket: Ticket,
        success: bool,
    },
}
pub struct Model {
    attachment: u64,
    attached: bool,
    clocks: BTreeMap<Lane, Clock>,
    event_epoch: u64,
    performance_epoch: u64,
    pub tab: Tab,
    pub filters: Filters,
    pub events: Vec<EventRow>,
    pub runs: Vec<Run>,
    pub samples: Vec<Sample>,
    pub health: Option<Health>,
    pub reports: Vec<Report>,
    pub latency: Vec<Latency>,
    pub captures: Vec<PrivateCapture>,
    pub capture_detail: Option<PrivateCapture>,
    pub loading: BTreeSet<Lane>,
    pub errors: BTreeSet<Lane>,
    pub import_error: Option<ImportError>,
    pub notice: Option<&'static str>,
    pub selected_run: Option<String>,
    pub selected_capture: Option<String>,
    pub baseline: usize,
    pub candidate: usize,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            attachment: 0,
            attached: false,
            clocks: BTreeMap::new(),
            event_epoch: 0,
            performance_epoch: 0,
            tab: Tab::Events,
            filters: Filters::default(),
            events: vec![],
            runs: vec![],
            samples: vec![],
            health: None,
            reports: vec![],
            latency: vec![],
            captures: vec![],
            capture_detail: None,
            loading: BTreeSet::new(),
            errors: BTreeSet::new(),
            import_error: None,
            notice: None,
            selected_run: None,
            selected_capture: None,
            baseline: 0,
            candidate: 1,
        }
    }
}
impl Model {
    pub fn attach(&mut self) -> u64 {
        self.attachment += 1;
        self.attached = true;
        self.attachment
    }
    pub fn detach(&mut self) {
        self.attached = false;
        for c in self.clocks.values_mut() {
            c.epoch += 1;
        }
        self.captures.clear();
        self.capture_detail = None;
        self.selected_capture = None;
        self.loading.clear();
    }
    pub fn ticket(&self, lane: Lane) -> Ticket {
        Ticket {
            attachment: self.attachment,
            lane,
            epoch: self.clocks.get(&lane).map_or(0, |c| c.epoch),
            store_epoch: self.domain_epoch(lane),
        }
    }
    pub fn begin_read(&mut self, lane: Lane) -> Ticket {
        self.clocks.entry(lane).or_default().epoch += 1;
        self.loading.insert(lane);
        self.errors.remove(&lane);
        self.ticket(lane)
    }
    pub fn current(&self, t: Ticket) -> bool {
        self.attached && t == self.ticket(t.lane)
    }
    pub fn filtered_events(&self) -> Vec<&EventRow> {
        self.events
            .iter()
            .filter(|e| self.filters.matches(e))
            .collect()
    }
    pub fn copy_events(&self) -> String {
        self.filtered_events()
            .into_iter()
            .map(EventRow::copy_line)
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn clear_imports(&mut self) {
        self.clocks.entry(Lane::Reports).or_default().epoch += 1;
        self.reports.retain(|r| r.source == Source::Local);
        self.baseline = 0;
        self.candidate = 1;
        self.import_error = None;
        self.notice =
            Some("Session imports cleared. Source files and saved reports are unchanged.");
    }
    pub fn event(&mut self, e: Event) -> bool {
        match e {
            Event::Read {
                ticket,
                revision,
                payload,
            } => {
                if !self.current(ticket)
                    || payload.lane().is_some_and(|l| l != ticket.lane)
                    || revision == 0
                    || revision <= self.clocks.entry(ticket.lane).or_default().revision
                {
                    return false;
                }
                self.accept(ticket.lane, revision, payload);
            }
            Event::Notification {
                attachment,
                lane,
                revision,
                store_epoch,
                payload,
            } => {
                if !self.attached
                    || attachment != self.attachment
                    || store_epoch < self.domain_epoch(lane)
                    || payload.lane().is_some_and(|l| l != lane)
                    || revision == 0
                    || revision <= self.clocks.entry(lane).or_default().revision
                {
                    return false;
                }
                if store_epoch > self.domain_epoch(lane) {
                    self.reset_store(lane, store_epoch);
                }
                self.clocks.entry(lane).or_default().epoch += 1;
                self.accept(lane, revision, payload);
            }
            Event::Cleared {
                attachment,
                lane,
                revision,
                store_epoch,
            } => {
                if !self.attached
                    || attachment != self.attachment
                    || !matches!(lane, Lane::Events | Lane::Performance)
                    || store_epoch <= self.domain_epoch(lane)
                    || revision == 0
                    || revision <= self.clocks.entry(lane).or_default().revision
                {
                    return false;
                }
                self.reset_store(lane, store_epoch);
                self.clocks.entry(lane).or_default().revision = revision;
                self.notice = Some("Diagnostics data cleared.");
            }
            Event::Import { ticket, result } => {
                if !self.current(ticket) || ticket.lane != Lane::Reports {
                    return false;
                }
                self.loading.remove(&Lane::Reports);
                match result {
                    Ok(Some(report)) => {
                        if report.source != Source::Imported {
                            self.import_error = Some(ImportError::SchemaMismatch);
                        } else if self
                            .reports
                            .iter()
                            .filter(|r| r.source == Source::Imported)
                            .count()
                            >= MAX_IMPORTS
                        {
                            self.import_error = Some(ImportError::SessionLimit);
                        } else {
                            self.reports.push(report);
                            self.candidate = self.reports.len() - 1;
                            self.import_error = None;
                            self.notice =
                                Some("Imported into this session. No saved runs were changed.");
                        }
                    }
                    Ok(None) => {}
                    Err(e) => self.import_error = Some(e),
                }
                // One result per operation, including cancellation or failure.
                self.clocks.entry(Lane::Reports).or_default().epoch += 1;
            }
            Event::Action { ticket, success } => {
                if !self.current(ticket) {
                    return false;
                }
                self.loading.remove(&ticket.lane);
                if success {
                    self.notice = Some("Action completed.");
                } else {
                    self.errors.insert(ticket.lane);
                }
                self.clocks.entry(ticket.lane).or_default().epoch += 1;
            }
        }
        true
    }
    fn domain_epoch(&self, lane: Lane) -> u64 {
        match lane {
            Lane::Events => self.event_epoch,
            Lane::Performance | Lane::Health => self.performance_epoch,
            _ => 0,
        }
    }
    fn reset_store(&mut self, lane: Lane, epoch: u64) {
        match lane {
            Lane::Events => {
                self.event_epoch = epoch;
                self.events.clear();
            }
            Lane::Performance | Lane::Health => {
                self.performance_epoch = epoch;
                self.runs.clear();
                self.samples.clear();
                self.selected_run = None;
            }
            _ => return,
        }
        for affected in if lane == Lane::Events {
            vec![Lane::Events]
        } else {
            vec![Lane::Performance, Lane::Health]
        } {
            self.clocks.entry(affected).or_default().epoch += 1;
            self.loading.remove(&affected);
        }
    }
    fn accept(&mut self, lane: Lane, revision: u64, payload: Payload) {
        self.clocks.entry(lane).or_default().revision = revision;
        self.loading.remove(&lane);
        self.errors.remove(&lane);
        match payload {
            Payload::Events(events) => {
                self.events.clear();
                for e in events
                    .into_iter()
                    .rev()
                    .take(MAX_EVENTS)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                {
                    if e.valid() {
                        self.events.push(e);
                    }
                }
            }
            Payload::Event(e) => {
                if e.valid() && !self.events.iter().any(|x| x.sequence == e.sequence) {
                    self.events.push(e);
                    self.events.sort_by_key(|e| e.sequence);
                    if self.events.len() > MAX_EVENTS {
                        self.events.remove(0);
                    }
                }
            }
            Payload::Performance { runs, samples } => {
                self.runs.clear();
                self.samples.clear();
                for r in runs.into_iter().take(MAX_RUNS) {
                    self.merge_run(r);
                }
                for s in samples.into_iter().rev().take(MAX_SAMPLES) {
                    self.merge_sample(s);
                }
            }
            Payload::Run(r) => self.merge_run(r),
            Payload::Sample(s) => self.merge_sample(s),
            Payload::Health(h) => self.health = Some(h),
            Payload::Reports(reports) => {
                self.reports.retain(|r| r.source == Source::Imported);
                self.reports.splice(
                    0..0,
                    reports
                        .into_iter()
                        .filter(|r| r.source == Source::Local)
                        .take(10),
                );
            }
            Payload::Latency(v) => self.latency = v.into_iter().take(MAX_SAMPLES).collect(),
            Payload::Captures(v) => {
                self.captures = v
                    .into_iter()
                    .filter(PrivateCapture::valid)
                    .take(20)
                    .map(|mut c| {
                        c.sections.clear();
                        c
                    })
                    .collect();
                if self
                    .selected_capture
                    .as_ref()
                    .is_some_and(|id| !self.captures.iter().any(|c| &c.id == id))
                {
                    self.selected_capture = None;
                    self.capture_detail = None;
                }
            }
            Payload::CaptureDetail(c) => {
                if c.valid() && self.selected_capture.as_ref() == Some(&c.id) {
                    self.capture_detail = Some(c);
                }
            }
            Payload::Route(tab) => self.tab = tab,
            Payload::Failed => {
                self.errors.insert(lane);
            }
        }
        if self
            .selected_run
            .as_ref()
            .is_some_and(|id| !self.runs.iter().any(|r| &r.id == id))
        {
            self.selected_run = None;
        }
    }
    fn merge_run(&mut self, r: Run) {
        if !r.valid() {
            return;
        }
        self.runs.retain(|x| x.id != r.id);
        self.runs.push(r);
        self.runs
            .sort_by(|a, b| b.finished_ms.cmp(&a.finished_ms).then(b.id.cmp(&a.id)));
        self.runs.truncate(MAX_RUNS);
    }
    fn merge_sample(&mut self, s: Sample) {
        self.samples.retain(|x| x.observed_ms != s.observed_ms);
        self.samples.push(s);
        self.samples.sort_by_key(|s| s.observed_ms);
        if self.samples.len() > MAX_SAMPLES {
            self.samples.remove(0);
        }
    }
}
