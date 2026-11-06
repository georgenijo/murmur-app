//! Serialized durable completions. No window, runtime, clock or host dependency.
//! Only the owner task touches these files; IPC clients never replace snapshots.
pub mod lifecycle;
pub mod producers;
mod stats;
mod storage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub use stats::{Scope, StatisticsEvent, Stats};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
};
use storage::{RootLock, Storage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    Missing,
    Invalid,
    InUse,
    Stopped,
    TooManyFiles,
    Uncertain,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "Completion file missing or quarantined",
            Self::Io => "Completion storage unavailable",
            Self::Invalid => "Invalid completion data",
            Self::InUse => "Completion storage already in use",
            Self::Stopped => "Completion service stopped",
            Self::Uncertain => "File admission outcome uncertain; completion storage unavailable",
            Self::TooManyFiles => "Too many pending file transcriptions",
        })
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
const STATE: &str = "completion-state.json";
const HOLD: &str = "completion-holding.json";
const MIGRATION: &str = "migrations.json";

#[derive(Clone, Copy, Deserialize, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    History,
    Statistics,
    Reports,
}
impl Destination {
    fn file(self) -> &'static str {
        match self {
            Self::History => "history.json",
            Self::Statistics => "stats.json",
            Self::Reports => "benchmark-reports.json",
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: u32,
    v1_imported: bool,
    generation: Option<String>,
    cleared_at: Option<u64>,
    #[serde(default)]
    reset_at: Option<u64>,
    applied_through: u64,
    data: Value,
    #[serde(default)]
    file_receipts: Vec<String>,
}
impl Document {
    fn new(d: Destination) -> Self {
        Self {
            schema: 1,
            file_receipts: vec![],
            v1_imported: false,
            generation: None,
            cleared_at: None,
            reset_at: None,
            applied_through: 0,
            data: match d {
                Destination::Statistics => json!(Stats::default()),
                _ => json!([]),
            },
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    sequence: u64,
    uuid: String,
    event: StatisticsEvent,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    uuid: String,
    admitted: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u32,
    next: u64,
    journal: Vec<Record>,
    #[serde(default)]
    file_admissions: Vec<Admission>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct List {
    generation: String,
    entries: Vec<Value>,
}
impl Default for List {
    fn default() -> Self {
        Self {
            generation: uuid(),
            entries: vec![],
        }
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Deletion {
    id: String,
    digest: String,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Holding {
    schema: u32,
    history: List,
    reports: List,
    deletions: Vec<Deletion>,
    launches: u64,
    #[serde(default)]
    file_receipts: Vec<String>,
}
impl Default for Holding {
    fn default() -> Self {
        Self {
            schema: 1,
            history: List::default(),
            reports: List::default(),
            deletions: vec![],
            launches: 0,
            file_receipts: vec![],
        }
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Migration {
    schema: u32,
    v1: bool,
    #[serde(default)]
    keys_removed: Vec<Destination>,
    #[serde(default)]
    lifecycle: lifecycle::Progress,
}
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyImport {
    pub history: Option<Value>,
    pub statistics: Option<Value>,
    pub reports: Option<Value>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub history: Vec<Value>,
    pub statistics: Stats,
    pub reports: Vec<Value>,
    pub v1_committed: bool,
    pub remove_keys: Vec<String>,
    pub retry_notice: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryMutation {
    Edit { target: Value, text: String },
    Pin { target: Value },
    Delete { targets: Vec<Value> },
}
#[derive(Clone)]
pub enum Command {
    Snapshot,
    Import(LegacyImport),
    KeysRemoved(Vec<Destination>),
    Clear {
        destination: Destination,
        at: u64,
    },
    History(HistoryMutation),
    Report(Value),
    /// Source is app-session UUID + producer id. Reattaching a view never sends
    /// this command. All producers submit at their successful native seam.
    Complete {
        source: String,
        entry: Option<Value>,
        event: Option<StatisticsEvent>,
    },
    /// File-only admission: the staged entry carries a stable completion UUID.
    /// Success acknowledges durable History/holding publication, without statistics.
    AcceptFile {
        entry: Value,
        intent: Arc<producers::FileIntent>,
        active: Vec<String>,
    },
}
type Job = Box<dyn FnOnce(&mut Engine) + Send>;
struct Owner {
    admission: Mutex<bool>,
    send: Option<mpsc::Sender<Job>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        drop(self.send.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Clone)]
pub struct CompletionService {
    owner: Arc<Owner>,
}
impl CompletionService {
    pub fn start(root: PathBuf, webkit_exists: bool) -> Result<Self> {
        let lock = RootLock::acquire(&root)?;
        let (send, receive) = mpsc::channel::<Job>();
        let worker = std::thread::Builder::new()
            .name("completion-service".into())
            .spawn(move || {
                let _lock = lock;
                let mut engine = Engine {
                    storage: Storage::new(root),
                    webkit_exists,
                    seen: HashSet::new(),
                    revision: 0,
                    initialized: false,
                };
                if engine.initialize().is_err() {
                    diagnostic("initialization_failed");
                }
                while let Ok(job) = receive.recv() {
                    job(&mut engine);
                }
            })
            .map_err(|_| Error::Stopped)?;
        Ok(Self {
            owner: Arc::new(Owner {
                admission: Mutex::new(true),
                send: Some(send),
                worker: Some(worker),
            }),
        })
    }
    /// Queue a sequence barrier without blocking the executor or main thread.
    /// Seal is permanent and ordered with submission under the admission lock.
    pub async fn flush(&self) -> Result<()> {
        self.barrier(false).await
    }
    pub async fn seal_and_flush(&self) -> Result<()> {
        self.barrier(true).await
    }
    async fn barrier(&self, seal: bool) -> Result<()> {
        let (send, receive) = futures_channel::oneshot::channel();
        {
            let mut admission = self.owner.admission.lock().map_err(|_| Error::Stopped)?;
            if seal {
                *admission = false;
            }
            self.owner
                .send
                .as_ref()
                .ok_or(Error::Stopped)?
                .send(Box::new(move |engine| {
                    let result = engine.execute(Command::Snapshot).map(|_| ());
                    let _ = send.send(result);
                }))
                .map_err(|_| Error::Stopped)?;
        }
        receive.await.map_err(|_| Error::Stopped)?
    }
    pub fn lifecycle(&self, command: lifecycle::Operation) -> Result<lifecycle::Receipt> {
        let (send, receive) = mpsc::sync_channel(1);
        let admission = self.owner.admission.lock().map_err(|_| Error::Stopped)?;
        if !*admission {
            return Err(Error::Stopped);
        }
        self.owner
            .send
            .as_ref()
            .ok_or(Error::Stopped)?
            .send(Box::new(move |engine| {
                let result = engine
                    .execute(Command::Snapshot)
                    .and_then(|_| engine.lifecycle(command));
                let _ = send.send(result);
            }))
            .map_err(|_| Error::Stopped)?;
        drop(admission);
        receive.recv().map_err(|_| Error::Stopped)?
    }
    pub fn execute(&self, command: Command) -> Result<Snapshot> {
        let (send, receive) = mpsc::sync_channel(1);
        let admission = self.owner.admission.lock().map_err(|_| Error::Stopped)?;
        if !*admission {
            return Err(Error::Stopped);
        }
        self.owner
            .send
            .as_ref()
            .ok_or(Error::Stopped)?
            .send(Box::new(move |engine| {
                let _ = send.send(engine.execute(command));
            }))
            .map_err(|_| Error::Stopped)?;
        drop(admission);
        receive.recv().map_err(|_| Error::Stopped)?
    }
}
fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn diagnostic(code: &'static str) {
    tracing::warn!(target:"completion",code,"completion service diagnostic");
}
struct Engine {
    initialized: bool,
    revision: u64,
    storage: Storage,
    webkit_exists: bool,
    seen: HashSet<String>,
}
impl Engine {
    fn document(&self, d: Destination) -> Result<Option<Document>> {
        let raw = self
            .storage
            .read::<Value>(d.file(), |v| valid_document(d, v))?;
        if raw.is_none() && self.migration()?.v1 {
            return Err(Error::Missing);
        }
        if self.migration()?.v1 && raw.as_ref().is_some_and(|v| v["v1_imported"] != true) {
            return Err(Error::Invalid);
        }
        raw.map(|value| {
            if value.get("schema").is_some() {
                serde_json::from_value(value).map_err(|_| Error::Invalid)
            } else {
                let mut doc = Document::new(d);
                doc.data = value;
                if d == Destination::History {
                    if let Some(entries) = doc.data.as_array_mut() {
                        for v in entries {
                            v["schemaVersion"] = json!(2);
                        }
                    }
                }
                Ok(doc)
            }
        })
        .transpose()
    }
    fn state(&self) -> Result<State> {
        self.storage
            .read(STATE, |v: &State| {
                v.schema == 1
                    && valid_receipts(
                        &v.file_admissions
                            .iter()
                            .map(|a| a.uuid.clone())
                            .collect::<Vec<_>>(),
                    )
                    && v.next > 0
                    && v.journal.windows(2).all(|w| w[0].sequence < w[1].sequence)
                    && v.journal.iter().all(|r| {
                        r.schema == 1
                            && r.sequence > 0
                            && r.sequence < v.next
                            && r.event.valid()
                            && uuid::Uuid::parse_str(&r.uuid).is_ok()
                    })
            })?
            .ok_or(Error::Missing)
    }
    fn holding(&self) -> Result<Holding> {
        self.storage
            .read(HOLD, |v: &Holding| {
                v.schema == 1
                    && valid_receipts(&v.file_receipts)
                    && v.history.entries.iter().all(valid_history)
                    && v.reports.entries.iter().all(valid_report)
            })?
            .ok_or(Error::Missing)
    }
    fn migration(&self) -> Result<Migration> {
        Ok(self
            .storage
            .read(MIGRATION, |v: &Migration| {
                let unique = v
                    .keys_removed
                    .iter()
                    .enumerate()
                    .all(|(i, d)| !v.keys_removed[..i].contains(d));
                v.schema == 1
                    && unique
                    && (!v.lifecycle.v2 || v.v1)
                    && (!v.lifecycle.v3 || v.lifecycle.v2)
                    && (!v.lifecycle.retired || v.lifecycle.retirement_started)
                    && (!v.lifecycle.retirement_started
                        || (v.lifecycle.v3 && v.keys_removed.len() == 3))
            })?
            .unwrap_or_default())
    }
    fn ready(&self, _d: Destination, doc: &Document) -> Result<bool> {
        Ok(doc.v1_imported || self.migration()?.v1)
    }
    fn initialize(&mut self) -> Result<()> {
        self.initialized = false;
        // Published generations may be unknown rename outcomes from a prior run.
        // Establish durability before trusting them or draining any held data.
        self.storage.sync_directory()?;
        let mut holding = match self.holding() {
            Ok(v) => v,
            Err(Error::Missing) => {
                diagnostic("holding_recreated");
                Holding::default()
            }
            Err(e) => return Err(e),
        };
        // Read/quarantine destinations before choosing recovery sequence numbers.
        let stats = self.document(Destination::Statistics)?;
        self.document(Destination::History)?;
        self.document(Destination::Reports)?;
        if self
            .storage
            .read::<State>(STATE, |v| {
                v.schema == 1
                    && valid_receipts(
                        &v.file_admissions
                            .iter()
                            .map(|a| a.uuid.clone())
                            .collect::<Vec<_>>(),
                    )
                    && v.next > 0
                    && v.journal.iter().all(|r| r.schema == 1 && r.event.valid())
            })?
            .is_none()
        {
            diagnostic("state_recreated_pending_loss_unknown");
            self.storage.write(
                STATE,
                &State {
                    schema: 1,
                    next: stats.as_ref().map_or(1, |s| s.applied_through + 1),
                    journal: vec![],
                    file_admissions: vec![],
                },
            )?;
        }
        self.observe_file_receipts(&holding)?;
        self.drop_consumed(&mut holding)?;
        if !self.migration()?.v1 {
            holding.launches += 1;
            tracing::warn!(target:"completion", code="migration_pending", held_history=holding.history.entries.len(), held_reports=holding.reports.entries.len(), pending_statistics=self.state()?.journal.len(), "completion migration pending");
        }
        self.storage.write(HOLD, &holding)?;
        if !self.webkit_exists && !self.migration()?.v1 {
            self.import(LegacyImport::default())?;
            self.storage.write(
                MIGRATION,
                &Migration {
                    schema: 1,
                    v1: true,
                    keys_removed: vec![],
                    lifecycle: Default::default(),
                },
            )?;
        }
        self.recover()?;
        self.initialized = true;
        Ok(())
    }
    fn drop_consumed(&self, h: &mut Holding) -> Result<()> {
        for d in [Destination::History, Destination::Reports] {
            if let Some(doc) = self.document(d)? {
                let list = match d {
                    Destination::History => &mut h.history,
                    _ => &mut h.reports,
                };
                if doc.generation.as_ref() == Some(&list.generation) {
                    *list = List::default();
                    if d == Destination::History {
                        h.deletions.clear();
                    }
                }
            }
        }
        Ok(())
    }
    fn recover(&mut self) -> Result<()> {
        // Unknown outcomes are resolved from disk, never the attempted snapshot.
        // Re-establish directory durability before using any published watermark
        // to compact a journal or drain a consumed holding generation.
        self.storage.sync_directory()?;
        let mut h = self.holding()?;
        self.observe_file_receipts(&h)?;
        let previous = h.clone();
        self.drop_consumed(&mut h)?;
        if h != previous {
            self.storage.write(HOLD, &h)?;
        }
        self.apply_statistics()?;
        Ok(())
    }
    fn apply_statistics(&mut self) -> Result<()> {
        let mut state = self.state()?;
        if state.journal.is_empty() {
            return Ok(());
        }
        let mut doc = self
            .document(Destination::Statistics)?
            .unwrap_or_else(|| Document::new(Destination::Statistics));
        if !self.ready(Destination::Statistics, &doc)? {
            return Ok(());
        }
        let pending: Vec<_> = state
            .journal
            .iter()
            .filter(|r| r.sequence > doc.applied_through)
            .cloned()
            .collect();
        if !pending.is_empty() {
            let mut stats: Stats =
                serde_json::from_value(doc.data.clone()).map_err(|_| Error::Invalid)?;
            for record in pending {
                stats.apply(&record.event);
                doc.applied_through = record.sequence;
            }
            doc.data = json!(stats);
            doc.v1_imported = true;
            self.storage.write(Destination::Statistics.file(), &doc)?;
        }
        state.journal.retain(|r| r.sequence > doc.applied_through);
        self.storage.write(STATE, &state)
    }
    fn snapshot(&self) -> Result<Snapshot> {
        let h = self.holding()?;
        let state = self.state()?;
        let hd = self
            .document(Destination::History)?
            .unwrap_or_else(|| Document::new(Destination::History));
        let sd = self
            .document(Destination::Statistics)?
            .unwrap_or_else(|| Document::new(Destination::Statistics));
        let rd = self
            .document(Destination::Reports)?
            .unwrap_or_else(|| Document::new(Destination::Reports));
        let mut history = hd.data.as_array().cloned().ok_or(Error::Invalid)?;
        history.extend(h.history.entries);
        trim_history(&mut history);
        let mut reports = rd.data.as_array().cloned().ok_or(Error::Invalid)?;
        reports.extend(h.reports.entries);
        trim_reports(&mut reports);
        let mut statistics: Stats =
            serde_json::from_value(sd.data.clone()).map_err(|_| Error::Invalid)?;
        for r in state
            .journal
            .iter()
            .filter(|r| r.sequence > sd.applied_through)
        {
            statistics.apply(&r.event);
        }
        let v1_committed = self.migration()?.v1;
        let mut remove_keys = vec![];
        for (doc, keys) in [
            (&hd, &["dictation-history"][..]),
            (&sd, &["dictation-stats"][..]),
            (
                &rd,
                &["murmur-benchmark-report", "murmur-benchmark-reports"][..],
            ),
        ] {
            if doc.v1_imported {
                remove_keys.extend(keys.iter().map(|s| s.to_string()));
            }
        }
        Ok(Snapshot {
            revision: self.revision,
            history,
            statistics,
            reports,
            v1_committed,
            remove_keys,
            retry_notice: !v1_committed && h.launches >= 3,
        })
    }
    fn execute(&mut self, command: Command) -> Result<Snapshot> {
        self.revision += 1;
        // Initialization retry must not inflate launch counts on every command.
        if !self.initialized
            || matches!(self.state(), Err(Error::Missing))
            || matches!(self.holding(), Err(Error::Missing))
        {
            self.initialize()
                .inspect_err(|_| diagnostic("initialization_failed"))?;
        }
        // Read failures block commands; they never authorize file replacement.
        self.state()?;
        self.holding()?;
        self.recover()?;
        match command {
            Command::Snapshot => {}
            Command::AcceptFile {
                entry,
                intent,
                active,
            } => {
                let id = entry["completionUuid"].as_str().ok_or(Error::Invalid)?;
                if !valid_history(&entry)
                    || uuid::Uuid::parse_str(id).is_err()
                    || !valid_receipts(&active)
                    || !active.iter().any(|v| v == id)
                {
                    return Err(Error::Invalid);
                }
                // Only this app-lifetime producer owns staging. A receipt outlives
                // its row and is retired only after its staged UUID is retired.
                self.compact_file_receipts(&active)?;
                let holding = self.holding()?;
                let doc = self.document(Destination::History)?;
                let mut state = self.state()?;
                let evidence = state.file_admissions.iter().find(|a| a.uuid == id);
                let published = evidence.is_some_and(|a| a.admitted)
                    || holding
                        .file_receipts
                        .iter()
                        .chain(doc.iter().flat_map(|d| &d.file_receipts))
                        .any(|v| v == id);
                if published {
                    intent.admitted();
                } else if intent.attempted() || (evidence.is_some() && !intent.preparing()) {
                    diagnostic("file_admission_uncertain");
                    return Err(Error::Uncertain);
                } else {
                    // Durable write-ahead evidence must precede any content publication.
                    // A failed preparation write cannot have published content.
                    // On retry recover() syncs and rereads its unknown outcome;
                    // live PREPARING permits reuse of that pending UUID record.
                    intent.prepare();
                    if evidence.is_none() {
                        state.file_admissions.push(Admission {
                            uuid: id.into(),
                            admitted: false,
                        });
                    }
                    self.storage.write(STATE, &state)?;
                    self.add_file(entry, &intent)
                        .inspect_err(|_| diagnostic("history_write_failed"))?;
                }
            }
            Command::Import(source) => self.import(source)?,
            Command::KeysRemoved(destinations) => {
                let mut migration = self.migration()?;
                let before = migration.keys_removed.len();
                for destination in destinations {
                    let doc = self
                        .document(destination)?
                        .unwrap_or_else(|| Document::new(destination));
                    if !self.ready(destination, &doc)? {
                        return Err(Error::Invalid);
                    }
                    if !migration.keys_removed.contains(&destination) {
                        migration.keys_removed.push(destination);
                    }
                }
                if before != migration.keys_removed.len() {
                    migration.schema = 1;
                    self.storage.write(MIGRATION, &migration)?;
                }
            }
            Command::Clear { destination, at } => self.clear(destination, at)?,
            Command::History(mutation) => self.mutate_history(mutation)?,
            Command::Report(report) => {
                if !valid_report(&report) {
                    return Err(Error::Invalid);
                }
                self.add_content(Destination::Reports, report)?;
            }
            Command::Complete {
                source,
                entry,
                event,
            } => {
                if entry.as_ref().is_some_and(|v| !valid_history(v))
                    || event.as_ref().is_some_and(|v| !v.valid())
                {
                    return Err(Error::Invalid);
                }
                if !self.seen.contains(&source) {
                    let id = uuid();
                    if let Some(event) = event {
                        let mut state = self.state()?;
                        state.journal.push(Record {
                            schema: 1,
                            sequence: state.next,
                            uuid: id.clone(),
                            event,
                        });
                        state.next += 1;
                        // On an unknown outcome, reread and make durable before deciding
                        // whether this source committed. Never append a second record.
                        if let Err(error) = self.storage.write(STATE, &state) {
                            if self.state()?.journal.iter().any(|r| r.uuid == id) {
                                self.seen.insert(source);
                                self.storage.sync_directory()?;
                            }
                            return Err(error);
                        }
                    }
                    self.seen.insert(source);
                    if let Some(mut entry) = entry {
                        entry["completionUuid"] = json!(id);
                        if self.add_content(Destination::History, entry).is_err() {
                            diagnostic("history_write_failed");
                        }
                    }
                    self.apply_statistics()?;
                }
            }
        }
        self.snapshot()
    }
    fn observe_file_receipts(&mut self, h: &Holding) -> Result<()> {
        let doc = self.document(Destination::History)?;
        let mut state = self.state()?;
        let mut changed = false;
        for admission in &mut state.file_admissions {
            if !admission.admitted
                && h.file_receipts
                    .iter()
                    .chain(doc.iter().flat_map(|d| &d.file_receipts))
                    .any(|id| id == &admission.uuid)
            {
                admission.admitted = true;
                changed = true;
            }
        }
        if changed {
            self.storage.write(STATE, &state)?;
        }
        Ok(())
    }
    fn compact_file_receipts(&mut self, active: &[String]) -> Result<()> {
        let mut state = self.state()?;
        let before = state.file_admissions.len();
        state.file_admissions.retain(|a| active.contains(&a.uuid));
        if before != state.file_admissions.len() {
            self.storage.write(STATE, &state)?;
        }
        let mut h = self.holding()?;
        let before = h.file_receipts.len();
        h.file_receipts.retain(|id| active.contains(id));
        if before != h.file_receipts.len() {
            self.storage.write(HOLD, &h)?;
        }
        if let Some(mut doc) = self.document(Destination::History)? {
            let before = doc.file_receipts.len();
            doc.file_receipts.retain(|id| active.contains(id));
            if before != doc.file_receipts.len() {
                self.storage.write(Destination::History.file(), &doc)?;
            }
        }
        Ok(())
    }
    fn add_file(&mut self, entry: Value, intent: &producers::FileIntent) -> Result<()> {
        let id = entry["completionUuid"]
            .as_str()
            .ok_or(Error::Invalid)?
            .to_owned();
        let mut doc = self
            .document(Destination::History)?
            .unwrap_or_else(|| Document::new(Destination::History));
        if self.ready(Destination::History, &doc)? {
            doc.file_receipts.push(id.clone());
            let entries = doc.data.as_array_mut().ok_or(Error::Invalid)?;
            entries.push(entry);
            trim_history(entries);
            doc.v1_imported = true;
            // Preparation is complete. This CAS begins publication; an earlier
            // cancellation wins even while reads or receipt compaction were busy.
            if !intent.begin_publication() {
                return self.finish_file_publication(&id, intent, Ok(()), false);
            }
            let result = self.storage.write(Destination::History.file(), &doc);
            let renamed = self.storage.renamed;
            intent.end_publication(result.is_ok());
            self.finish_file_publication(&id, intent, result, renamed)
        } else {
            let mut h = self.holding()?;
            h.file_receipts.push(id.clone());
            h.history.entries.push(entry);
            // cap_held can publish original-row eviction, so gate before its
            // first write as well as the final atomic holding publication.
            if !intent.begin_publication() {
                return self.finish_file_publication(&id, intent, Ok(()), false);
            }
            // cap_held may write old History rows, but cannot publish this file.
            // Only the holding write can make this completion's outcome unknown.
            if let Err(error) = self.cap_held(&mut h, Destination::History, &mut doc) {
                intent.end_publication(false);
                return self.finish_file_publication(&id, intent, Err(error), false);
            }
            let result = self.storage.write(HOLD, &h);
            let renamed = self.storage.renamed;
            intent.end_publication(result.is_ok());
            self.finish_file_publication(&id, intent, result, renamed)
        }
    }
    fn finish_file_publication(
        &mut self,
        id: &str,
        intent: &producers::FileIntent,
        result: Result<()>,
        renamed: bool,
    ) -> Result<()> {
        if !renamed {
            // Preserve proven non-publication even if pending-evidence cleanup
            // itself fails. The live intent can safely reconcile and retry it.
            intent.prepare();
        }
        if result.is_ok() || !renamed {
            let mut state = self.state()?;
            if result.is_ok() && renamed {
                if let Some(a) = state.file_admissions.iter_mut().find(|a| a.uuid == id) {
                    a.admitted = true;
                }
            } else {
                // Only proven non-publication permits another attempt. If this
                // cleanup fails, the live preparation phase permits safe retry.
                state.file_admissions.retain(|a| a.uuid != id);
            }
            self.storage.write(STATE, &state)?;
        }
        result
    }
    fn add_content(&mut self, d: Destination, mut value: Value) -> Result<()> {
        if d == Destination::Reports && value.get("completionUuid").is_none() {
            value["completionUuid"] = json!(uuid());
        }
        let mut doc = self.document(d)?.unwrap_or_else(|| Document::new(d));
        if self.ready(d, &doc)? {
            let entries = doc.data.as_array_mut().ok_or(Error::Invalid)?;
            entries.push(value);
            match d {
                Destination::History => trim_history(entries),
                _ => trim_reports(entries),
            };
            doc.v1_imported = true;
            self.storage.write(d.file(), &doc)
        } else {
            let mut h = self.holding()?;
            match d {
                Destination::History => h.history.entries.push(value),
                _ => h.reports.entries.push(value),
            }
            self.cap_held(&mut h, d, &mut doc)?;
            self.storage.write(HOLD, &h)
        }
    }
    fn cap_held(&mut self, h: &mut Holding, d: Destination, doc: &mut Document) -> Result<()> {
        let original = doc.data.as_array_mut().ok_or(Error::Invalid)?;
        let list = match d {
            Destination::History => &mut h.history,
            _ => &mut h.reports,
        };
        if d == Destination::History {
            // Pins are normalized across the combined stored order. Eviction is
            // held-first, then original, preserving the newest 20 pins.
            let mut pins = 20usize;
            for v in list
                .entries
                .iter_mut()
                .rev()
                .chain(original.iter_mut().rev())
            {
                if v["pinned"] == true {
                    v["pinned"] = json!(pins > 0);
                    pins = pins.saturating_sub(1);
                }
            }
            while original.len() + list.entries.len() > 200 {
                if let Some(i) = list.entries.iter().position(|v| v["pinned"] != true) {
                    list.entries.remove(i);
                } else if let Some(i) = original.iter().position(|v| v["pinned"] != true) {
                    let v = original.remove(i);
                    h.deletions.push(deletion(&v));
                } else {
                    break;
                }
            }
        } else {
            trim_reports(&mut list.entries);
            while original.len() + list.entries.len() > 10 {
                let old = list.entries.first().and_then(|v| v["createdAt"].as_str());
                let disk = original.first().and_then(|v| v["createdAt"].as_str());
                if !list.entries.is_empty() && (original.is_empty() || old < disk) {
                    list.entries.remove(0);
                } else {
                    original.remove(0);
                }
            }
        }
        if self.document(d)?.is_some() {
            self.storage.write(d.file(), doc)?;
        }
        Ok(())
    }
    fn clear(&mut self, d: Destination, at: u64) -> Result<()> {
        let mut doc = Document::new(d);
        if d == Destination::History {
            doc.file_receipts = self.document(d)?.map_or_else(Vec::new, |d| d.file_receipts);
        }
        doc.v1_imported = true;
        if d == Destination::Statistics {
            doc.reset_at = Some(at);
        } else {
            doc.cleared_at = Some(at);
        }
        let mut h = self.holding()?;
        match d {
            Destination::History => doc.generation = Some(h.history.generation.clone()),
            Destination::Reports => doc.generation = Some(h.reports.generation.clone()),
            Destination::Statistics => doc.applied_through = self.state()?.next - 1,
        }
        // Sole commit point. The next recovery drops this generation even if
        // the process dies before the following holding/state writes.
        self.storage.write(d.file(), &doc)?;
        self.drop_consumed(&mut h)?;
        self.storage.write(HOLD, &h)?;
        self.apply_statistics()
    }
    fn mutate_history(&mut self, mutation: HistoryMutation) -> Result<()> {
        let mut h = self.holding()?;
        let mut doc = self
            .document(Destination::History)?
            .unwrap_or_else(|| Document::new(Destination::History));
        let original = doc.data.as_array_mut().ok_or(Error::Invalid)?;
        let pin_count = original
            .iter()
            .chain(h.history.entries.iter())
            .filter(|v| v["pinned"] == true)
            .count();
        match mutation {
            HistoryMutation::Delete { targets } => {
                for target in targets {
                    if let Some(i) = h.history.entries.iter().position(|v| *v == target) {
                        h.history.entries.remove(i);
                    } else if let Some(i) = original.iter().position(|v| *v == target) {
                        let v = original.remove(i);
                        h.deletions.push(deletion(&v));
                    }
                }
            }
            HistoryMutation::Edit { target, text } => {
                if let Some(v) = h
                    .history
                    .entries
                    .iter_mut()
                    .chain(original.iter_mut())
                    .find(|v| **v == target)
                {
                    // Keep the pre-upgrade identity with the row, atomically with
                    // the edit. A later deletion must suppress the WebKit original.
                    if v.get("legacyIdentity").is_none() {
                        v["legacyIdentity"] = json!(deletion(v));
                    }
                    v["text"] = json!(text);
                }
            }
            HistoryMutation::Pin { target } => {
                if target["pinned"] != true && pin_count >= 20 {
                    return Err(Error::Invalid);
                }
                if let Some(v) = h
                    .history
                    .entries
                    .iter_mut()
                    .chain(original.iter_mut())
                    .find(|v| **v == target)
                {
                    v["pinned"] = json!(v["pinned"] != true);
                }
            }
        }
        // Persist deletion records before changing originals, so losing the
        // original file later cannot resurrect a deleted legacy entry.
        self.storage.write(HOLD, &h)?;
        if self.document(Destination::History)?.is_some() {
            self.storage.write(Destination::History.file(), &doc)?;
        }
        Ok(())
    }
    fn import(&mut self, source: LegacyImport) -> Result<()> {
        if self.migration()?.v1 {
            return Ok(());
        }
        for (d, legacy) in [
            (Destination::History, source.history),
            (Destination::Statistics, source.statistics),
            (Destination::Reports, source.reports),
        ] {
            let existing = self.document(d)?;
            if existing.as_ref().is_some_and(|v| v.v1_imported) {
                continue;
            }
            let mut doc = existing.clone().unwrap_or_else(|| Document::new(d));
            let mut h = self.holding()?;
            if existing.is_none() {
                if let Some(legacy) = legacy {
                    if !valid_data(d, &legacy) {
                        return Err(Error::Invalid);
                    }
                    doc.data = legacy;
                    if d == Destination::History {
                        let entries = doc.data.as_array_mut().ok_or(Error::Invalid)?;
                        for record in &h.deletions {
                            if let Some(i) = entries
                                .iter()
                                .position(|v| v["id"] == record.id && digest(v) == record.digest)
                            {
                                entries.remove(i);
                            }
                        }
                    }
                }
            }
            match d {
                Destination::Statistics => {
                    let mut stats: Stats =
                        serde_json::from_value(doc.data).map_err(|_| Error::Invalid)?;
                    let watermark = doc.applied_through;
                    for r in self
                        .state()?
                        .journal
                        .iter()
                        .filter(|r| r.sequence > watermark)
                    {
                        stats.apply(&r.event);
                        doc.applied_through = r.sequence;
                    }
                    doc.data = json!(stats);
                }
                Destination::History => {
                    let entries = doc.data.as_array_mut().ok_or(Error::Invalid)?;
                    for mut v in h.history.entries.clone() {
                        if entries
                            .iter()
                            .any(|e| e["id"] == v["id"] && e["text"] == v["text"])
                        {
                            continue;
                        }
                        if entries.iter().any(|e| e["id"] == v["id"]) {
                            v["id"] = json!(uuid());
                        }
                        entries.push(v);
                    }
                    trim_history(entries);
                    doc.generation = Some(h.history.generation.clone());
                }
                Destination::Reports => {
                    let entries = doc.data.as_array_mut().ok_or(Error::Invalid)?;
                    for v in &h.reports.entries {
                        if !entries.iter().any(|e| same_report(e, v)) {
                            entries.push(v.clone());
                        }
                    }
                    trim_reports(entries);
                    doc.generation = Some(h.reports.generation.clone());
                }
            }
            doc.v1_imported = true;
            self.storage.write(d.file(), &doc)?;
            self.drop_consumed(&mut h)?;
            self.storage.write(HOLD, &h)?;
            self.apply_statistics()?;
        }
        self.storage.write(
            MIGRATION,
            &Migration {
                schema: 1,
                v1: true,
                keys_removed: self.migration()?.keys_removed,
                lifecycle: self.migration()?.lifecycle,
            },
        )
    }
}
fn digest(v: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(v["text"].as_str().unwrap_or_default().as_bytes())
    )
}
fn deletion(v: &Value) -> Deletion {
    if let Some(identity) = v.get("legacyIdentity") {
        if let Ok(identity) = serde_json::from_value(identity.clone()) {
            return identity;
        }
    }
    Deletion {
        id: v["id"].as_str().unwrap_or_default().into(),
        digest: digest(v),
    }
}
fn trim_history(entries: &mut Vec<Value>) {
    let mut pins = 20usize;
    for v in entries.iter_mut().rev() {
        v["schemaVersion"] = json!(2);
        if v["pinned"] == true {
            v["pinned"] = json!(pins > 0);
            pins = pins.saturating_sub(1);
        }
    }
    while entries.len() > 200 {
        if let Some(i) = entries.iter().position(|v| v["pinned"] != true) {
            entries.remove(i);
        } else {
            break;
        }
    }
}
fn same_report(a: &Value, b: &Value) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    if let Some(v) = a.as_object_mut() {
        v.remove("completionUuid");
    }
    if let Some(v) = b.as_object_mut() {
        v.remove("completionUuid");
    }
    a == b
}
fn trim_reports(entries: &mut Vec<Value>) {
    entries.sort_by(|a, b| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()));
    entries.dedup_by(|a, b| same_report(a, b));
    if entries.len() > 10 {
        entries.drain(..entries.len() - 10);
    }
}
fn valid_receipts(ids: &[String]) -> bool {
    ids.len() <= producers::MAX_PENDING_FILES
        && ids.iter().all(|id| uuid::Uuid::parse_str(id).is_ok())
        && ids.iter().collect::<HashSet<_>>().len() == ids.len()
}
fn valid_document(d: Destination, v: &Value) -> bool {
    if v.get("schema").is_some() {
        serde_json::from_value::<Document>(v.clone()).is_ok_and(|doc| {
            doc.schema == 1 && valid_receipts(&doc.file_receipts) && valid_data(d, &doc.data)
        })
    } else {
        valid_data(d, v)
    }
}
fn valid_data(d: Destination, v: &Value) -> bool {
    match d {
        Destination::History => v.as_array().is_some_and(|vs| vs.iter().all(valid_history)),
        Destination::Reports => v.as_array().is_some_and(|vs| vs.iter().all(valid_report)),
        Destination::Statistics => {
            serde_json::from_value::<Stats>(v.clone()).is_ok_and(|s| s.valid())
        }
    }
}
fn strings(v: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|k| v[k].is_string())
}
fn numbers(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .all(|k| v[k].as_f64().is_some_and(|n| n.is_finite() && n >= 0.0))
}
fn stages(v: &Value) -> bool {
    v.as_array().is_some_and(|vs| {
        vs.iter().all(|v| {
            strings(v, &["stage"])
                && v["changed"].is_boolean()
                && matches!(
                    v["outcome"].as_str(),
                    Some("applied" | "skipped" | "fallback" | "failed")
                )
        })
    })
}
fn optional(v: &Value, key: &str, valid: impl FnOnce(&Value) -> bool) -> bool {
    v.get(key).is_none_or(valid)
}
fn valid_history(v: &Value) -> bool {
    v.is_object()
        && strings(v, &["id", "text"])
        && numbers(v, &["timestamp", "duration"])
        && optional(v, "schemaVersion", |v| *v == 2)
        && optional(v, "legacyIdentity", |v| {
            serde_json::from_value::<Deletion>(v.clone()).is_ok_and(|d| {
                d.digest.len() == 64 && d.digest.bytes().all(|b| b.is_ascii_hexdigit())
            })
        })
        && optional(v, "pinned", Value::is_boolean)
        && optional(v, "source", |v| {
            matches!(v.as_str(), Some("recording" | "file"))
        })
        && ["rawText", "sourceName", "completionUuid"]
            .iter()
            .all(|k| optional(v, k, Value::is_string))
        && optional(v, "recording", |r| {
            numbers(r, &["recordingId"])
                && strings(r, &["modelId"])
                && stages(&r["stages"])
                && ["modeId", "profileId"]
                    .iter()
                    .all(|k| r[k].is_null() || r[k].is_string())
        })
        && optional(v, "derived", |r| {
            strings(r, &["sourceEntryId", "modeId"])
                && numbers(r, &["createdAt"])
                && stages(&r["stages"])
        })
        && optional(v, "interruption", |r| {
            strings(r, &["reason"]) && numbers(r, &["deliveredSamples", "durationMs"])
        })
        && optional(v, "teachingContext", |r| {
            r.is_null()
                || (r.is_object()
                    && ["appBundleId", "appLabel", "projectRoot"]
                        .iter()
                        .all(|k| optional(r, k, |v| v.is_null() || v.is_string())))
        })
}
fn nullable_strings(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .all(|k| v.get(k).is_some_and(|v| v.is_null() || v.is_string()))
}
fn nullable_numbers(v: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|k| {
        v.get(k)
            .is_some_and(|v| v.is_null() || v.as_f64().is_some_and(|n| n.is_finite()))
    })
}
fn string_array(v: &Value) -> bool {
    v.as_array()
        .is_some_and(|vs| vs.iter().all(Value::is_string))
}
fn valid_report(v: &Value) -> bool {
    let metadata = match v.get("reportVersion") {
        None => ["environment", "corpus", "configuration"]
            .iter()
            .all(|k| v.get(k).is_none()),
        Some(version) if *version == 2 || *version == 3 => {
            let e = &v["environment"];
            let c = &v["corpus"];
            let config = &v["configuration"];
            strings(e, &["os", "architecture"])
                && nullable_strings(e, &["osVersion", "hardwareModel", "chip"])
                && nullable_numbers(e, &["memoryMb"])
                && strings(c, &["language", "provenance", "limitation"])
                && string_array(&c["fixtureIds"])
                && numbers(c, &["fixtureCount", "referenceWords"])
                && (matches!(c["source"].as_str(), Some("bundled" | "personal"))
                    || (*version == 2 && c.get("source").is_none()))
                && numbers(config, &["vadThreshold"])
                && strings(
                    config,
                    &[
                        "executionPath",
                        "transcriptTransformProfile",
                        "percentileMethod",
                    ],
                )
                && string_array(&config["modelRunOrder"])
                && string_array(&config["sharedInitOrder"])
        }
        _ => false,
    };
    v.is_object()
        && metadata
        && strings(v, &["createdAt", "appVersion", "platform"])
        && v["createdAt"]
            .as_str()
            .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok())
        && numbers(v, &["iterations", "sharedInitMs"])
        && matches!(
            v["preset"].as_str(),
            Some("quick" | "standard" | "thorough")
        )
        && v["recommendations"].is_object()
        && nullable_strings(
            &v["recommendations"],
            &["fastest", "mostAccurate", "balanced"],
        )
        && v["results"].as_array().is_some_and(|rs| {
            rs.iter().all(|r| {
                strings(r, &["modelName", "label", "backend", "accelerator"])
                    && optional(r, "downloadSize", Value::is_string)
                    && numbers(r, &["memoryDeltaMb"])
                    && nullable_strings(r, &["error"])
                    && nullable_numbers(
                        r,
                        &[
                            "modelLoadMs",
                            "firstInferenceMs",
                            "warmMedianMs",
                            "warmP95Ms",
                            "realtimeFactor",
                            "wordErrorRate",
                            "normalizedWordErrorRate",
                            "deliveredWordErrorRate",
                            "deliveredNormalizedWordErrorRate",
                        ],
                    )
                    && r["fixtures"].as_array().is_some_and(|fs| {
                        fs.iter().all(|f| {
                            strings(
                                f,
                                &[
                                    "fixtureId",
                                    "label",
                                    "reference",
                                    "transcript",
                                    "deliveredTranscript",
                                ],
                            ) && numbers(
                                f,
                                &[
                                    "audioSeconds",
                                    "warmMedianMs",
                                    "warmP95Ms",
                                    "realtimeFactor",
                                    "wordErrorRate",
                                    "wordErrors",
                                    "referenceWords",
                                    "normalizedWordErrorRate",
                                    "normalizedWordErrors",
                                    "normalizedReferenceWords",
                                    "deliveredWordErrorRate",
                                    "deliveredWordErrors",
                                    "deliveredNormalizedWordErrorRate",
                                    "deliveredNormalizedWordErrors",
                                ],
                            ) && f["deliveredTransformFailed"].is_boolean()
                        })
                    })
            })
        })
}

/// Host ticks the gate, optionally reloads once, then permits held operation.
#[derive(Default)]
pub struct MigrationGate {
    reloaded: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum GateAction {
    Wait,
    ReloadOnce,
    Arm,
    ArmHolding,
}
impl MigrationGate {
    pub fn tick(&mut self, state_ready: bool, committed: bool, elapsed_ms: u64) -> GateAction {
        if !state_ready {
            return GateAction::Wait;
        }
        if committed {
            return GateAction::Arm;
        }
        if elapsed_ms < 5000 {
            return GateAction::Wait;
        }
        if !self.reloaded {
            self.reloaded = true;
            return GateAction::ReloadOnce;
        }
        if elapsed_ms < 10000 {
            GateAction::Wait
        } else {
            GateAction::ArmHolding
        }
    }
}
#[cfg(test)]
mod tests;

/// Shared whole-settings publication path. Migration and journal files are never
/// part of a settings snapshot, even when that snapshot predates an import.
pub fn save_settings(root: &std::path::Path, blob: &str) -> Result<()> {
    let value: Value = serde_json::from_str(blob).map_err(|_| Error::Invalid)?;
    if !value.is_object() || blob.len() > 1024 * 1024 {
        return Err(Error::Invalid);
    }
    Storage::new(root.into()).write_bytes("settings.json", blob.as_bytes())
}

/// Settings/theme commands share the host's writer lock with lifecycle imports.
/// Publication uses the same full-sync discipline as completion destinations.
pub fn save_source(root: &std::path::Path, key: &str, blob: &str) -> Result<String> {
    if blob.len() > crate::state::source_size_limit(key) {
        return Err(Error::Invalid);
    }
    Storage::new(root.into()).reject_future(
        crate::state::file(key).ok_or(Error::Invalid)?,
        if key == "dictation-settings" {
            "settingsVersion"
        } else {
            "version"
        },
        if key == "dictation-settings" { 7 } else { 1 },
    )?;
    let value =
        crate::state::normalize(key, serde_json::from_str(blob).map_err(|_| Error::Invalid)?)?;
    let blob = serde_json::to_string(&value).map_err(|_| Error::Invalid)?;
    let mut storage = Storage::new(root.into());
    recover_source(root, key)?;
    publish_source(&mut storage, key, &value)?;
    Ok(blob)
}

fn source_digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("JSON value"))
    )
}
fn source_revision(key: &str, value: &Value) -> u64 {
    value[if key == "dictation-settings" {
        "ownedFieldsRevision"
    } else {
        "revision"
    }]
    .as_u64()
    .unwrap_or(0)
}
fn source_destination(storage: &Storage, key: &str) -> Result<Option<Value>> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    storage.reject_future(
        file,
        if key == "dictation-settings" {
            "settingsVersion"
        } else {
            "version"
        },
        if key == "dictation-settings" { 7 } else { 1 },
    )?;
    storage.read(file, |v: &Value| {
        crate::state::normalize(key, v.clone()).is_ok()
    })
}

fn source_marker(storage: &Storage, file: &str) -> Result<Option<Value>> {
    match std::fs::read(storage.root.join(format!("{file}.required"))) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Error::Invalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Io),
    }
}
fn source_target_marker(receipt: &Value) -> Value {
    json!({"schema":2,"key":receipt["key"],"storeId":receipt["storeId"],"generation":receipt["generation"],"targetDigest":receipt["targetDigest"],"targetRevision":receipt["targetRevision"]})
}

/// A receipt proves a transition for this store and source, against the exact
/// prior destination. The identity is durable before any receipt can be written.
/// Hosts hold WRITE_LOCK across recovery, read/merge, restore and publication.
fn publish_source(storage: &mut Storage, key: &str, value: &Value) -> Result<()> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    let identity_file = format!("{file}.identity");
    let identity: String = match storage.read(&identity_file, |v: &String| {
        uuid::Uuid::parse_str(v).is_ok()
    })? {
        Some(v) => v,
        None => {
            let v = uuid::Uuid::new_v4().to_string();
            storage.write(&identity_file, &v)?;
            v
        }
    };
    let base = source_destination(storage, key)?;
    let marker = source_marker(storage, file)?;
    let receipt = json!({
        "schema":2, "key":key, "storeId":identity,
        "generation":uuid::Uuid::new_v4().to_string(),
        "baseDigest":base.as_ref().map(source_digest),
        "baseMarkerDigest":marker.as_ref().map(source_digest),
        "targetDigest":source_digest(value), "targetRevision":source_revision(key, value), "value":value
    });
    storage.write(&format!("{file}.pending"), &receipt)?;
    storage.write(&format!("{file}.required"), &source_target_marker(&receipt))?;
    storage.write(file, value)?;
    std::fs::remove_file(storage.root.join(format!("{file}.pending"))).map_err(|_| Error::Io)?;
    storage.sync_directory()
}

/// Explicit user-selected recovery, serialized by the host's WRITE_LOCK. Never
/// overwrite a readable destination, even if pending recovery itself fails.
/// Replacing the pending receipt is the restore commit point: before it, recovery
/// stays blocked; after it, the exact backup replays. Old evidence is retained.
pub fn restore_source(root: &std::path::Path, key: &str, value: &Value) -> Result<()> {
    restore_source_with_storage(&mut Storage::new(root.into()), key, value)
}
fn restore_source_with_storage(storage: &mut Storage, key: &str, value: &Value) -> Result<()> {
    if !complete_source_document(key, value) {
        return Err(Error::Invalid);
    }
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    std::fs::create_dir_all(&storage.root).map_err(|_| Error::Io)?;
    storage.sync_directory()?;
    if source_destination(storage, key)?.is_some() {
        return Err(Error::Invalid);
    }
    if storage.root.join(format!("{file}.pending")).exists() {
        storage.quarantine(&format!("{file}.pending"))?;
    }
    // Required is durable before replacing a failed first generation, so a
    // crash before the new pending receipt cannot become successful absence.
    storage.write(&format!("{file}.required"), &true)?;
    publish_source(storage, key, value)
}

/// Full normalized source documents only. The legacy microphone migration may
/// advance its flag on the next normalization; that boolean is source metadata.
pub fn complete_source_document(key: &str, value: &Value) -> bool {
    crate::state::normalize(key, value.clone()).is_ok_and(|mut normalized| {
        if key == "dictation-settings" && value["microphoneIdMigrationComplete"].is_boolean() {
            normalized["microphoneIdMigrationComplete"] =
                value["microphoneIdMigrationComplete"].clone();
        }
        normalized == *value
    })
}

/// Quarantine a rejected source under the host's existing WRITE_LOCK, using the
/// same durable history guard and monotonic evidence sequence as core recovery.
pub fn quarantine_source(root: &std::path::Path, key: &str) -> Result<()> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    Storage::new(root.into()).quarantine(file)
}

/// Bound per-source evidence and abandoned publication siblings. Host readers
/// also call this after quarantining a destination under WRITE_LOCK.
pub fn prune_source_artifacts(root: &std::path::Path, key: &str) -> Result<()> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    let storage = Storage::new(root.into());
    for name in [
        file.to_string(),
        format!("{file}.pending"),
        format!("{file}.required"),
        format!("{file}.identity"),
    ] {
        storage.prune_artifacts(&name)?;
        storage.prune_artifacts(&format!("{name}.quarantine_sequence"))?;
    }
    storage.prune_artifacts(&format!("{file}.required-history"))?;
    Ok(())
}

/// Resume only a proven interrupted transition. Corrupt receipts never veto a
/// readable destination; conflicting receipts surface once and preserve data.
pub fn recover_source(root: &std::path::Path, key: &str) -> Result<()> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    let pending = format!("{file}.pending");
    std::fs::create_dir_all(root).map_err(|_| Error::Io)?;
    let mut storage = Storage::new(root.into());
    storage.sync_directory()?;
    // Establish whether the destination is usable before treating any evidence
    // as resolved. A corrupt destination is quarantined and pins its evidence.
    let destination = source_destination(&storage, key)?;
    prune_source_artifacts(root, key)?;
    let existed = root.join(&pending).exists();
    let receipt: Option<Value> = storage.read(&pending, |v: &Value| {
        let Some(receipt_key) = v["key"]
            .as_str()
            .filter(|k| crate::state::file(k).is_some())
        else {
            return false;
        };
        v["schema"] == 2
            && v["storeId"]
                .as_str()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            && v["generation"]
                .as_str()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            && (v.get("baseDigest") == Some(&Value::Null)
                || v["baseDigest"]
                    .as_str()
                    .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())))
            && (v.get("baseMarkerDigest") == Some(&Value::Null)
                || v["baseMarkerDigest"]
                    .as_str()
                    .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())))
            && complete_source_document(receipt_key, &v["value"])
            && v["targetDigest"] == source_digest(&v["value"])
            && v["targetRevision"] == source_revision(receipt_key, &v["value"])
    })?;
    if let Some(receipt) = receipt {
        let identity: Option<String> = storage
            .read(&format!("{file}.identity"), |v: &String| {
                uuid::Uuid::parse_str(v).is_ok()
            })?;
        let belongs = receipt["key"] == key && identity.as_deref() == receipt["storeId"].as_str();
        let matches_base = destination.as_ref().is_none_or(|v| {
            let digest = source_digest(v);
            receipt["baseDigest"] == digest || receipt["targetDigest"] == digest
        });
        let marker = source_marker(&storage, file)?;
        let marker_digest = marker.as_ref().map(source_digest);
        let matches_generation = receipt["baseMarkerDigest"] == json!(marker_digest)
            || marker.as_ref() == Some(&source_target_marker(&receipt));
        if !belongs || !matches_base || !matches_generation {
            storage.quarantine(&pending)?;
            return Err(Error::Invalid);
        }
        storage.write(&format!("{file}.required"), &source_target_marker(&receipt))?;
        if destination
            .as_ref()
            .is_none_or(|v| source_digest(v) != receipt["targetDigest"])
        {
            storage.write(file, &receipt["value"])?;
        }
        std::fs::remove_file(root.join(&pending)).map_err(|_| Error::Io)?;
        storage.sync_directory()?;
    } else if destination.is_none() && (existed || storage.quarantined(&pending)?) {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// A previously published or quarantined source cannot become successful absence.
pub fn source_required(root: &std::path::Path, key: &str) -> Result<bool> {
    let file = crate::state::file(key).ok_or(Error::Invalid)?;
    recover_source(root, key)?;
    let storage = Storage::new(root.into());
    if root.join(format!("{file}.required")).exists()
        || root.join(format!("{file}.required-history")).exists()
        || storage.quarantined(file)?
    {
        return Ok(true);
    }
    storage.reject_future("ui-state.json", "schema", 1)?;
    let existed = root.join("ui-state.json").exists();
    let ui: Option<serde_json::Value> = storage.read("ui-state.json", |v: &Value| {
        v["schema"] == 1
            && v["v2_imported"].is_boolean()
            && v["destinations"].as_array().is_some_and(|a| {
                a.iter()
                    .all(|k| k.as_str().is_some_and(|k| crate::state::file(k).is_some()))
            })
    })?;
    let migration: Option<Migration> = storage.read(MIGRATION, |v: &Migration| v.schema == 1)?;
    let expected_ui = migration.is_some_and(|m| m.lifecycle.v2);
    if ui.is_none() && (existed || expected_ui || storage.quarantined("ui-state.json")?) {
        return Err(Error::Invalid);
    }
    Ok(ui.is_some_and(|v| {
        v["destinations"]
            .as_array()
            .is_some_and(|a| a.contains(&json!(key)))
    }))
}

#[cfg(test)]
mod source_recovery_tests;
/// Private single-owner snapshot publication. The immutable pending document is
/// synced before the destination; replay runs before admitting another writer.
pub fn publish_snapshot<T: Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let root = path.parent().ok_or(Error::Invalid)?;
    std::fs::create_dir_all(root).map_err(|_| Error::Io)?;
    let file = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(Error::Invalid)?;
    let pending = format!("{file}.pending");
    let mut storage = Storage::new(root.into());
    storage.write(&pending, &json!({"schema":1,"value":value}))?;
    recover_snapshot(path)
}
pub fn recover_snapshot(path: &std::path::Path) -> Result<()> {
    let root = path.parent().ok_or(Error::Invalid)?;
    let file = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(Error::Invalid)?;
    let pending = format!("{file}.pending");
    let mut storage = Storage::new(root.into());
    storage.sync_directory()?;
    if let Some(receipt) =
        storage.read::<Value>(&pending, |v| v["schema"] == 1 && v.get("value").is_some())?
    {
        storage.write(file, &receipt["value"])?;
        std::fs::remove_file(root.join(pending)).map_err(|_| Error::Io)?;
        storage.sync_directory()?;
    }
    Ok(())
}
