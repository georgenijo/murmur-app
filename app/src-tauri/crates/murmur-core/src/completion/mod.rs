//! Serialized durable completions. No window, runtime, clock or host dependency.
//! Only the owner task touches these files; IPC clients never replace snapshots.
mod stats;
mod storage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub use stats::{Scope, StatisticsEvent, Stats};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{mpsc, Arc},
};
use storage::{RootLock, Storage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Io,
    Invalid,
    InUse,
    Stopped,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Io => "Completion storage unavailable",
            Self::Invalid => "Invalid completion data",
            Self::InUse => "Completion storage already in use",
            Self::Stopped => "Completion service stopped",
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
}
impl Document {
    fn new(d: Destination) -> Self {
        Self {
            schema: 1,
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
struct State {
    schema: u32,
    next: u64,
    journal: Vec<Record>,
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
}
impl Default for Holding {
    fn default() -> Self {
        Self {
            schema: 1,
            history: List::default(),
            reports: List::default(),
            deletions: vec![],
            launches: 0,
        }
    }
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    schema: u32,
    v1: bool,
    #[serde(default)]
    keys_removed: Vec<Destination>,
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
}
type Job = Box<dyn FnOnce(&mut Engine) + Send>;
struct Owner {
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
                    storage: Storage {
                        root,
                        #[cfg(test)]
                        fail: None,
                    },
                    webkit_exists,
                    seen: HashSet::new(),
                    revision: 0,
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
                send: Some(send),
                worker: Some(worker),
            }),
        })
    }
    pub fn execute(&self, command: Command) -> Result<Snapshot> {
        let (send, receive) = mpsc::sync_channel(1);
        self.owner
            .send
            .as_ref()
            .ok_or(Error::Stopped)?
            .send(Box::new(move |engine| {
                let _ = send.send(engine.execute(command));
            }))
            .map_err(|_| Error::Stopped)?;
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
            .ok_or(Error::Io)
    }
    fn holding(&self) -> Result<Holding> {
        self.storage
            .read(HOLD, |v: &Holding| {
                v.schema == 1
                    && v.history.entries.iter().all(valid_history)
                    && v.reports.entries.iter().all(valid_report)
            })?
            .ok_or(Error::Io)
    }
    fn migration(&self) -> Result<Migration> {
        Ok(self
            .storage
            .read(MIGRATION, |v: &Migration| v.schema == 1)?
            .unwrap_or_default())
    }
    fn ready(&self, _d: Destination, doc: &Document) -> Result<bool> {
        Ok(doc.v1_imported || self.migration()?.v1)
    }
    fn initialize(&mut self) -> Result<()> {
        // Read/quarantine destinations before choosing recovery sequence numbers.
        let stats = self.document(Destination::Statistics)?;
        self.document(Destination::History)?;
        self.document(Destination::Reports)?;
        if self
            .storage
            .read::<State>(STATE, |v| {
                v.schema == 1
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
                },
            )?;
        }
        let mut holding = match self.holding() {
            Ok(v) => v,
            Err(Error::Io) => {
                diagnostic("holding_recreated");
                Holding::default()
            }
            Err(e) => return Err(e),
        };
        self.drop_consumed(&mut holding)?;
        if !self.migration()?.v1 {
            holding.launches += 1;
            tracing::warn!(target:"completion", code="migration_pending", held_history=holding.history.entries.len(), held_reports=holding.reports.entries.len(), pending_statistics=self.state()?.journal.len(), "completion migration pending");
        }
        self.storage.write(HOLD, &holding)?;
        if !self.webkit_exists && !self.migration()?.v1 {
            self.storage.write(
                MIGRATION,
                &Migration {
                    schema: 1,
                    v1: true,
                    keys_removed: vec![],
                },
            )?;
        }
        self.recover()
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
            if v1_committed || doc.v1_imported {
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
        if self.state().is_err() || self.holding().is_err() {
            self.initialize()?;
        }
        self.recover()?;
        match command {
            Command::Snapshot => {}
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
fn valid_document(d: Destination, v: &Value) -> bool {
    if v.get("schema").is_some() {
        serde_json::from_value::<Document>(v.clone())
            .is_ok_and(|doc| doc.schema == 1 && valid_data(d, &doc.data))
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
