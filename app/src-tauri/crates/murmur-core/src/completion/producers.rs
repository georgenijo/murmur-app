//! Production completion admission shared by native hosts and headless effects.
use super::{Command, CompletionService, Error, Result, Scope, Snapshot, StatisticsEvent};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};

// Bound transcript-bearing operations even if a client disappears before accepting.
pub(super) const MAX_PENDING_FILES: usize = 200;

pub struct Producers {
    service: CompletionService,
    session: String,
    files: Mutex<HashMap<String, FileState>>,
    intents: Mutex<IntentRegistry>,
    #[cfg(test)]
    cancel_recorded: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}
#[derive(Default)]
struct IntentRegistry {
    active: HashMap<String, Arc<FileIntent>>,
    retired: VecDeque<String>,
}
impl IntentRegistry {
    #[cfg(test)]
    fn len(&self) -> usize {
        self.active.len() + self.retired.len()
    }
    fn retire(&mut self, operation: &str) {
        self.active.remove(operation);
        if !self.retired.iter().any(|id| id == operation) {
            self.retired.push_back(operation.into());
        }
        while self.retired.len() > MAX_PENDING_FILES {
            self.retired.pop_front();
        }
    }
}
enum FileState {
    Running,
    Cancelled,
    Ready(Option<Value>),
}
// Independent of the transcript-bearing files mutex. Cancellation can mark a
// queued acceptance without waiting for the owner. Bits preserve late intent
// across a failed in-progress publication; a durable receipt always wins later.
#[derive(Default)]
pub struct FileIntent(AtomicU8);
impl FileIntent {
    const CANCELLED: u8 = 1;
    const PUBLISHING: u8 = 2;
    const ADMITTED: u8 = 4;
    const ATTEMPTED: u8 = 8;
    pub(super) fn attempted(&self) -> bool {
        self.0.load(Ordering::SeqCst) & Self::ATTEMPTED != 0
    }
    pub(super) fn mark_attempted(&self) {
        self.0.fetch_or(Self::ATTEMPTED, Ordering::SeqCst);
    }
    pub(super) fn clear_attempted(&self) {
        self.0.fetch_and(!Self::ATTEMPTED, Ordering::SeqCst);
    }
    fn cancel(&self) {
        self.0.fetch_or(Self::CANCELLED, Ordering::SeqCst);
    }
    pub(super) fn begin_publication(&self) -> bool {
        self.0
            .compare_exchange(
                Self::ATTEMPTED,
                Self::ATTEMPTED | Self::PUBLISHING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }
    pub(super) fn admitted(&self) {
        self.0.fetch_or(Self::ADMITTED, Ordering::SeqCst);
    }
    pub(super) fn end_publication(&self, success: bool) {
        if success {
            self.admitted();
        }
        self.0.fetch_and(!Self::PUBLISHING, Ordering::SeqCst);
    }
    fn cancelled_before_admission(&self) -> bool {
        self.0.load(Ordering::SeqCst) & (Self::CANCELLED | Self::ADMITTED) == Self::CANCELLED
    }
}
impl Producers {
    pub fn new(service: CompletionService, session: String) -> Self {
        Self {
            service,
            session,
            files: Mutex::new(HashMap::new()),
            intents: Mutex::new(IntentRegistry::default()),
            #[cfg(test)]
            cancel_recorded: Mutex::new(None),
        }
    }
    fn submit(
        &self,
        key: String,
        entry: Option<Value>,
        event: Option<StatisticsEvent>,
    ) -> Result<Snapshot> {
        self.service.execute(Command::Complete {
            source: format!("{}:{key}", self.session),
            entry,
            event,
        })
    }
    /// Retention is the immutable configuration resolved at recording start.
    pub fn dictation(
        &self,
        generation: u64,
        entry: Value,
        retain: bool,
        mode: Option<String>,
        day: String,
    ) -> Result<Snapshot> {
        let event = StatisticsEvent::Dictation {
            words: entry["text"]
                .as_str()
                .unwrap_or_default()
                .split_whitespace()
                .count() as u64,
            seconds: entry["duration"].as_f64().ok_or(Error::Invalid)?,
            mode,
            day,
        };
        self.submit(
            format!("dictation:{generation}"),
            retain.then_some(entry),
            Some(event),
        )
    }
    pub fn meeting(&self, generation: u64, seconds: f64, month: String) -> Result<Snapshot> {
        self.submit(
            format!("meeting:{generation}"),
            None,
            Some(StatisticsEvent::Meeting { seconds, month }),
        )
    }
    pub fn correction_proposed(&self, proposal: u64) -> Result<Snapshot> {
        self.submit(
            format!("proposed:{proposal}"),
            None,
            Some(StatisticsEvent::CorrectionProposed),
        )
    }
    pub fn correction_taught(&self, proposal: u64, scope: Scope) -> Result<Snapshot> {
        self.submit(
            format!("taught:{proposal}"),
            None,
            Some(StatisticsEvent::CorrectionTaught { scope }),
        )
    }
    pub fn paste_last(&self, delivery: &str) -> Result<Snapshot> {
        self.submit(
            format!("paste:{delivery}"),
            None,
            Some(StatisticsEvent::PasteLast),
        )
    }
    pub fn begin_file(&self, operation: &str) -> Result<()> {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        match files.get(operation) {
            Some(FileState::Cancelled) => Err(Error::Stopped),
            Some(_) => Err(Error::InUse),
            None => {
                let mut intents = self.intents.lock().unwrap_or_else(|p| p.into_inner());
                if intents.retired.iter().any(|id| id == operation) {
                    return Err(Error::Stopped);
                }
                if files.len() >= MAX_PENDING_FILES {
                    return Err(Error::TooManyFiles);
                }
                intents
                    .active
                    .insert(operation.into(), Arc::new(FileIntent::default()));
                files.insert(operation.into(), FileState::Running);
                Ok(())
            }
        }
    }
    fn intent(&self, operation: &str) -> Arc<FileIntent> {
        self.intents
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .active[operation]
            .clone()
    }
    fn retire(&self, operation: &str) {
        self.intents
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retire(operation);
    }
    pub fn cancel_file(&self, operation: &str) {
        // Never wait for files or the owner on the IPC dispatch thread. Running
        // ownership pins its intent until finish/discard; queued accepts hold Arc.
        {
            let mut intents = self.intents.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(intent) = intents.active.get(operation) {
                intent.cancel();
            } else {
                intents.retire(operation);
            }
        }
        #[cfg(test)]
        if let Some(send) = self.cancel_recorded.lock().unwrap().take() {
            send.send(()).unwrap();
        }
        if let Ok(mut files) = self.files.try_lock() {
            match files.get(operation) {
                Some(FileState::Running | FileState::Cancelled) => {
                    files.insert(operation.into(), FileState::Cancelled);
                }
                Some(FileState::Ready(_)) => {
                    files.remove(operation);
                    self.retire(operation);
                }
                None => {}
            }
        }
    }
    pub fn finish_file(&self, operation: &str, entry: Option<Value>, retain: bool) {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        if files.contains_key(operation) && self.intent(operation).cancelled_before_admission() {
            files.remove(operation);
            self.retire(operation);
            return;
        }
        if matches!(files.get(operation), Some(FileState::Running)) {
            let entry = entry.filter(|_| retain).map(|mut entry| {
                // Retain this identity across every admission retry, including
                // an unknown publication outcome. It contains no source content.
                if let Some(object) = entry.as_object_mut() {
                    object.insert("completionUuid".into(), serde_json::json!(super::uuid()));
                }
                entry
            });
            files.insert(operation.into(), FileState::Ready(entry));
        }
    }
    /// The client accepts only after its cancellation check. A late cancellation
    /// before acceptance still wins, including after native transcription ends.
    pub fn accept_file(&self, operation: &str) -> Result<Option<Snapshot>> {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        if !matches!(files.get(operation), Some(FileState::Ready(_))) {
            return Ok(None);
        }
        let intent = self.intent(operation);
        let result = match files.get(operation) {
            Some(FileState::Ready(Some(entry))) => self
                .service
                .execute(Command::AcceptFile {
                    entry: entry.clone(),
                    intent: intent.clone(),
                    active: files
                        .values()
                        .filter_map(|state| match state {
                            FileState::Ready(Some(v)) => {
                                v["completionUuid"].as_str().map(str::to_owned)
                            }
                            _ => None,
                        })
                        .collect(),
                })
                .map(Some),
            Some(FileState::Ready(None)) => Ok(None),
            _ => return Ok(None),
        };
        if result.is_ok() || intent.cancelled_before_admission() {
            files.remove(operation);
            self.retire(operation);
        }
        if intent.cancelled_before_admission() {
            result.map(|_| None)
        } else {
            result
        }
    }
    pub fn discard_file(&self, operation: &str) {
        self.cancel_file(operation);
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(operation);
        self.retire(operation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    // Configure the real owner's storage without replacing the production adapter.
    fn storage(
        service: &CompletionService,
        f: impl FnOnce(&mut super::super::storage::Storage) + Send + 'static,
    ) {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        service
            .owner
            .send
            .as_ref()
            .unwrap()
            .send(Box::new(move |engine| {
                f(&mut engine.storage);
                send.send(()).unwrap();
            }))
            .unwrap();
        receive.recv().unwrap();
    }
    fn staged_file(service: &CompletionService) -> Producers {
        service.execute(Command::Snapshot).unwrap();
        let p = Producers::new(service.clone(), "synthetic".into());
        p.begin_file("operation").unwrap();
        p.finish_file(
            "operation",
            Some(json!({"schemaVersion":2,"pinned":false,"source":"file","id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
            true,
        );
        p
    }
    fn lose(root: &std::path::Path, file: &str, how: u8) {
        let path = root.join(file);
        match how {
            0 => {
                let _ = std::fs::remove_file(path);
            }
            1 => std::fs::write(path, b"broken synthetic JSON").unwrap(),
            _ => std::fs::write(path, br#"{"schema":999}"#).unwrap(),
        }
    }

    #[test]
    fn file_unknown_publication_loss_matrix_never_duplicates_or_resurrects() {
        for webkit in [false, true] {
            for file in [
                super::super::HOLD,
                super::super::STATE,
                super::super::MIGRATION,
                "history.json",
                "both-evidence-stores",
            ] {
                for how in 0..3 {
                    for removed in [false, true] {
                        let dir = tempfile::tempdir().unwrap();
                        let service = CompletionService::start(dir.path().into(), webkit).unwrap();
                        let mut p = staged_file(&service);
                        drop(service);
                        publication_fault(&p, webkit, super::super::storage::Boundary::AfterRename);
                        if removed {
                            p.service
                                .execute(Command::Import(super::super::LegacyImport::default()))
                                .unwrap();
                            let snapshot = p.service.execute(Command::Snapshot).unwrap();
                            assert_eq!(snapshot.history.len(), 1);
                            p.service
                                .execute(Command::History(super::super::HistoryMutation::Delete {
                                    targets: snapshot.history,
                                }))
                                .unwrap();
                            assert!(p
                                .service
                                .execute(Command::Snapshot)
                                .unwrap()
                                .history
                                .is_empty());
                        }
                        if file == "both-evidence-stores" {
                            lose(dir.path(), super::super::STATE, how);
                            lose(dir.path(), super::super::HOLD, how);
                            lose(
                                dir.path(),
                                if webkit && !removed {
                                    super::super::HOLD
                                } else {
                                    "history.json"
                                },
                                how,
                            );
                        } else {
                            lose(dir.path(), file, how);
                        }
                        restart(&mut p, dir.path(), webkit);
                        let before = p.service.execute(Command::Snapshot).unwrap();
                        let result = p.accept_file("operation");
                        if (file == "both-evidence-stores")
                            || (webkit && !removed && file == super::super::HOLD)
                            || (!webkit && !removed && file == "history.json")
                        {
                            assert!(result.is_err(), "lost evidence must remain uncertain: {webkit}/{file}/{how}/{removed}");
                        }
                        assert_eq!(
                            p.service.execute(Command::Snapshot).unwrap().history,
                            before.history,
                            "retry after {file}/{how}/{removed}"
                        );
                        p.cancel_file("operation");
                        assert!(p.accept_file("operation").unwrap().is_none());
                        p.service
                            .execute(Command::Import(super::super::LegacyImport::default()))
                            .unwrap();
                        let after = p.service.execute(Command::Snapshot).unwrap();
                        assert_eq!(after.history, before.history);
                        assert_no_statistics(&after);
                        restart(&mut p, dir.path(), false);
                        assert_eq!(
                            p.service.execute(Command::Snapshot).unwrap().history,
                            before.history
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn file_uncertainty_isolated_from_all_other_producers() {
        let dir = tempfile::tempdir().unwrap();
        let service = CompletionService::start(dir.path().into(), true).unwrap();
        let p = staged_file(&service);
        publication_fault(&p, true, super::super::storage::Boundary::AfterRename);
        lose(dir.path(), super::super::HOLD, 0);
        assert!(p.accept_file("operation").is_err());
        p.dictation(1, json!({"schemaVersion":2,"id":"dictation","text":"synthetic","timestamp":2,"duration":1}), true, None, "2026-11-05".into()).unwrap();
        p.meeting(1, 60., "2026-11".into()).unwrap();
        p.correction_proposed(1).unwrap();
        p.correction_taught(1, Scope::Global).unwrap();
        p.paste_last("synthetic").unwrap();
        let before = service.execute(Command::Snapshot).unwrap();
        assert_eq!(before.history.len(), 1);
        assert_eq!(before.statistics.total_recordings, 1);
        assert!(p.accept_file("operation").is_err());
        assert_eq!(
            service.execute(Command::Snapshot).unwrap().history,
            before.history
        );
    }

    #[test]
    fn file_intent_ownership_and_tombstones_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let service = CompletionService::start(dir.path().into(), false).unwrap();
        let p = Producers::new(service, "synthetic".into());
        // A cancelled native operation keeps ownership until its finish arrives,
        // even while older completed tombstones leave the bounded window.
        p.begin_file("still-running").unwrap();
        p.cancel_file("still-running");
        for n in 0..1000 {
            let operation = n.to_string();
            p.begin_file(&operation).unwrap();
            p.finish_file(&operation, None, false);
            if n % 2 == 0 {
                p.accept_file(&operation).unwrap();
            } else {
                p.cancel_file(&operation);
            }
            assert!(p.intents.lock().unwrap().len() <= MAX_PENDING_FILES + 1);
        }
        assert!(p.begin_file("still-running").is_err());
        p.finish_file("still-running", Some(json!({"text":"synthetic"})), true);
        assert!(p.accept_file("still-running").unwrap().is_none());
        for n in 0..1000 {
            p.cancel_file(&format!("never-started-{n}"));
        }
        assert!(p.intents.lock().unwrap().len() <= MAX_PENDING_FILES);
        assert!(p.files.lock().unwrap().is_empty());
    }
    fn assert_no_statistics(snapshot: &Snapshot) {
        assert_eq!(
            serde_json::to_value(&snapshot.statistics).unwrap(),
            serde_json::to_value(super::super::Stats::default()).unwrap()
        );
    }

    // Persist the staged UUID across a service restart in tests. Production drops
    // all staged content at process exit; this stronger check also models a lost
    // acceptance response and a reconnecting caller retrying its same UUID.
    fn restart(p: &mut Producers, root: &std::path::Path, webkit: bool) {
        let placeholder = tempfile::tempdir().unwrap();
        let old = std::mem::replace(
            &mut p.service,
            CompletionService::start(placeholder.path().into(), false).unwrap(),
        );
        drop(old);
        p.service = CompletionService::start(root.into(), webkit).unwrap();
    }
    fn publication_fault(p: &Producers, webkit: bool, at: super::super::storage::Boundary) {
        let file = if webkit {
            super::super::HOLD
        } else {
            "history.json"
        };
        storage(&p.service, move |s| s.fail = Some((file.into(), at)));
        assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
    }
    fn removed_then_retry(mutation: u8, relaunch: bool) {
        for webkit in [false, true] {
            for at in [
                super::super::storage::Boundary::AfterRename,
                super::super::storage::Boundary::DirectorySync,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let service = CompletionService::start(dir.path().into(), webkit).unwrap();
                let mut p = staged_file(&service);
                drop(service);
                publication_fault(&p, webkit, at);
                let snapshot = p.service.execute(Command::Snapshot).unwrap();
                assert_eq!(snapshot.history.len(), 1);
                let target = snapshot.history[0].clone();
                match mutation {
                    0 => {
                        p.service
                            .execute(Command::History(super::super::HistoryMutation::Delete {
                                targets: vec![target],
                            }))
                            .unwrap();
                    }
                    1 => {
                        p.service
                            .execute(Command::Clear {
                                destination: super::super::Destination::History,
                                at: 2,
                            })
                            .unwrap();
                    }
                    2 => {
                        // Seed a native-shaped full store in one fixture write;
                        // the next real producer must perform the actual eviction.
                        storage(&p.service, move |s| {
                            let file = if webkit {
                                super::super::HOLD
                            } else {
                                "history.json"
                            };
                            let mut raw = s.read::<Value>(file, |_| true).unwrap().unwrap();
                            let entries = if webkit {
                                &mut raw["history"]["entries"]
                            } else {
                                &mut raw["data"]
                            };
                            entries.as_array_mut().unwrap().extend((0..199).map(|n|
                                json!({"schemaVersion":2,"pinned":false,"source":"file","id":format!("synthetic-{n}"),"text":"synthetic","timestamp":n+2,"duration":1})
                            ));
                            s.write(file, &raw).unwrap();
                        });
                        let full = p.service.execute(Command::Snapshot).unwrap();
                        assert_eq!(full.history.len(), 200);
                        assert!(full.history.iter().any(|v| v["id"] == "synthetic"));
                        p.dictation(200, json!({"schemaVersion":2,"pinned":false,"source":"recording","id":"evicting-dictation","text":"synthetic","timestamp":201,"duration":1}), true, None, "2026-11-05".into()).unwrap();
                    }
                    3 => {
                        p.service
                            .execute(Command::History(super::super::HistoryMutation::Edit {
                                target,
                                text: "synthetic edited".into(),
                            }))
                            .unwrap();
                    }
                    _ => unreachable!(),
                }
                let before = p.service.execute(Command::Snapshot).unwrap();
                match mutation {
                    0 | 1 => assert!(before.history.is_empty(), "mutation must remove the row"),
                    2 => {
                        assert_eq!(before.history.len(), 200);
                        assert!(before.history.iter().all(|v| v["id"] != "synthetic"));
                    }
                    3 => assert_eq!(before.history[0]["text"], "synthetic edited"),
                    _ => unreachable!(),
                }
                if relaunch {
                    restart(&mut p, dir.path(), webkit);
                }
                let after = p.accept_file("operation").unwrap().unwrap();
                assert_eq!(
                    after.history, before.history,
                    "retry changed a removed/edited row"
                );
                assert_eq!(
                    serde_json::to_value(&after.statistics).unwrap(),
                    serde_json::to_value(&before.statistics).unwrap()
                );
                assert!(p.accept_file("operation").unwrap().is_none());
                if webkit {
                    p.service
                        .execute(Command::Import(super::super::LegacyImport::default()))
                        .unwrap();
                }
                restart(&mut p, dir.path(), false);
                assert_eq!(
                    p.service.execute(Command::Snapshot).unwrap().history,
                    before.history
                );
            }
        }
    }
    #[test]
    fn file_receipt_delete_then_retry_never_resurrects() {
        removed_then_retry(0, false);
    }
    #[test]
    fn file_receipt_clear_then_retry_never_resurrects() {
        removed_then_retry(1, false);
    }
    #[test]
    fn file_receipt_cap_evict_then_retry_never_resurrects() {
        removed_then_retry(2, false);
    }
    #[test]
    fn file_receipt_relaunch_then_retry_never_resurrects() {
        removed_then_retry(0, true);
    }
    #[test]
    fn file_receipt_edit_then_retry_keeps_edit() {
        removed_then_retry(3, true);
    }

    #[test]
    fn file_queued_retries_cancel_before_publication_wins() {
        for webkit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), webkit).unwrap();
            let p = staged_file(&service);
            publication_fault(&p, webkit, super::super::storage::Boundary::BeforeRename);
            let (blocked, reached) = std::sync::mpsc::channel();
            let (resume, release) = std::sync::mpsc::channel();
            service
                .owner
                .send
                .as_ref()
                .unwrap()
                .send(Box::new(move |_| {
                    blocked.send(()).unwrap();
                    release.recv().unwrap();
                }))
                .unwrap();
            reached.recv().unwrap();
            let (cancelled, observed) = std::sync::mpsc::channel();
            *p.cancel_recorded.lock().unwrap() = Some(cancelled);
            std::thread::scope(|scope| {
                let first = scope.spawn(|| p.accept_file("operation"));
                // Wait for acceptance to hold files while the owner is blocked.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while p.files.try_lock().is_ok() {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
                let second = scope.spawn(|| p.accept_file("operation"));
                let cancel = scope.spawn(|| p.cancel_file("operation"));
                observed
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                resume.send(()).unwrap();
                assert!(first.join().unwrap().unwrap().is_none());
                assert!(second.join().unwrap().unwrap().is_none());
                cancel.join().unwrap();
            });
            assert!(service
                .execute(Command::Snapshot)
                .unwrap()
                .history
                .is_empty());
            assert!(p.accept_file("operation").unwrap().is_none());
            assert_no_statistics(&service.execute(Command::Snapshot).unwrap());
        }
    }

    #[test]
    fn file_cancel_after_durable_publish_keeps_history() {
        for webkit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), webkit).unwrap();
            let mut p = staged_file(&service);
            drop(service);
            // Lost response after rename; Snapshot makes the publication durable.
            publication_fault(&p, webkit, super::super::storage::Boundary::AfterRename);
            let before = p.service.execute(Command::Snapshot).unwrap();
            p.cancel_file("operation");
            assert!(p.accept_file("operation").unwrap().is_none());
            restart(&mut p, dir.path(), webkit);
            assert_eq!(
                p.service.execute(Command::Snapshot).unwrap().history,
                before.history
            );
            assert_no_statistics(&before);
        }
    }

    #[test]
    fn file_cancel_during_publication_preserves_only_published_outcomes() {
        for webkit in [false, true] {
            for fail in [
                None,
                Some(super::super::storage::Boundary::BeforeRename),
                Some(super::super::storage::Boundary::AfterRename),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let service = CompletionService::start(dir.path().into(), webkit).unwrap();
                let p = staged_file(&service);
                let (paused, reached) = std::sync::mpsc::channel();
                let (resume, release) = std::sync::mpsc::channel();
                storage(&service, move |s| {
                    let file = if webkit {
                        super::super::HOLD
                    } else {
                        "history.json"
                    };
                    s.pause = Some((
                        file.into(),
                        super::super::storage::Boundary::BeforeRename,
                        paused,
                        release,
                    ));
                    s.fail = fail.map(|at| (file.into(), at));
                });
                let (cancelled, observed) = std::sync::mpsc::channel();
                *p.cancel_recorded.lock().unwrap() = Some(cancelled);
                std::thread::scope(|scope| {
                    let accept = scope.spawn(|| p.accept_file("operation"));
                    reached
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    let cancel = scope.spawn(|| p.cancel_file("operation"));
                    observed
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    resume.send(()).unwrap();
                    assert_eq!(accept.join().unwrap().is_err(), fail.is_some());
                    cancel.join().unwrap();
                });
                assert!(p.accept_file("operation").unwrap().is_none());
                let snapshot = service.execute(Command::Snapshot).unwrap();
                assert_eq!(
                    snapshot.history.len(),
                    usize::from(fail != Some(super::super::storage::Boundary::BeforeRename))
                );
                assert_no_statistics(&snapshot);
            }
        }
    }

    #[test]
    fn file_receipts_compact_only_after_staging_retires() {
        let dir = tempfile::tempdir().unwrap();
        let service = CompletionService::start(dir.path().into(), false).unwrap();
        let p = staged_file(&service);
        publication_fault(&p, false, super::super::storage::Boundary::AfterRename);
        let id = p
            .files
            .lock()
            .unwrap()
            .get("operation")
            .and_then(|s| match s {
                FileState::Ready(Some(v)) => v["completionUuid"].as_str(),
                _ => None,
            })
            .unwrap()
            .to_owned();
        for n in 0..205 {
            let operation = n.to_string();
            p.begin_file(&operation).unwrap();
            p.finish_file(
                &operation,
                Some(json!({"schemaVersion":2,"pinned":false,"source":"file","id":operation,"text":"synthetic","timestamp":n+2,"duration":1})),
                true,
            );
            p.accept_file(&operation).unwrap();
            let raw: Value =
                serde_json::from_slice(&std::fs::read(dir.path().join("history.json")).unwrap())
                    .unwrap();
            let receipts = raw["file_receipts"].as_array().unwrap();
            assert!(receipts.contains(&json!(id)));
            assert!(receipts.len() <= 3);
            assert!(receipts
                .iter()
                .all(|v| uuid::Uuid::parse_str(v.as_str().unwrap()).is_ok()));
        }
        let before = service.execute(Command::Snapshot).unwrap();
        p.accept_file("operation").unwrap();
        assert_eq!(
            service.execute(Command::Snapshot).unwrap().history,
            before.history
        );
    }

    #[test]
    fn file_history_publication_failure_retains_result_and_retries_once() {
        for webkit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), webkit).unwrap();
            let p = staged_file(&service);
            let file = if webkit {
                super::super::HOLD
            } else {
                "history.json"
            };
            storage(&service, move |s| {
                s.fail = Some((file.into(), super::super::storage::Boundary::BeforeRename))
            });
            assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
            assert!(matches!(
                p.files.lock().unwrap().get("operation"),
                Some(FileState::Ready(Some(_)))
            ));
            assert!(service
                .execute(Command::Snapshot)
                .unwrap()
                .history
                .is_empty());
            let snapshot = p.accept_file("operation").unwrap().unwrap();
            assert_eq!(snapshot.history.len(), 1);
            assert_no_statistics(&snapshot);
            assert!(p.accept_file("operation").unwrap().is_none());
            if webkit {
                service
                    .execute(Command::Import(super::super::LegacyImport::default()))
                    .unwrap();
            }
            drop(p);
            drop(service);
            let service = CompletionService::start(dir.path().into(), false).unwrap();
            let snapshot = service.execute(Command::Snapshot).unwrap();
            assert_eq!(snapshot.history.len(), 1);
            assert_no_statistics(&snapshot);
        }
    }

    #[test]
    fn file_history_unknown_publication_rereads_before_retry_without_duplicates() {
        for webkit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), webkit).unwrap();
            let p = staged_file(&service);
            let file = if webkit {
                super::super::HOLD
            } else {
                "history.json"
            };
            storage(&service, move |s| {
                s.fail = Some((file.into(), super::super::storage::Boundary::AfterRename))
            });
            assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
            // A failed directory sync cannot acknowledge the unknown publication.
            storage(&service, |s| s.fail_sync.set(true));
            assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
            assert!(matches!(
                p.files.lock().unwrap().get("operation"),
                Some(FileState::Ready(Some(_)))
            ));
            storage(&service, |s| s.trace.borrow_mut().clear());
            let snapshot = p.accept_file("operation").unwrap().unwrap();
            assert_eq!(snapshot.history.len(), 1);
            assert_no_statistics(&snapshot);
            assert!(p.accept_file("operation").unwrap().is_none());
            storage(&service, move |s| {
                let trace = s.trace.borrow();
                assert_eq!(trace.first().map(String::as_str), Some("directory_sync"));
                assert!(!trace
                    .iter()
                    .any(|step| step == &format!("{file}:BeforeRename")));
            });
            if webkit {
                service
                    .execute(Command::Import(super::super::LegacyImport::default()))
                    .unwrap();
            }
            drop(p);
            drop(service);
            let service = CompletionService::start(dir.path().into(), false).unwrap();
            let snapshot = service.execute(Command::Snapshot).unwrap();
            assert_eq!(snapshot.history.len(), 1);
            assert_no_statistics(&snapshot);
        }
    }

    #[test]
    fn file_history_cancel_after_publication_failure_never_admits_on_retry() {
        for webkit in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), webkit).unwrap();
            let p = staged_file(&service);
            let file = if webkit {
                super::super::HOLD
            } else {
                "history.json"
            };
            storage(&service, move |s| {
                s.fail = Some((file.into(), super::super::storage::Boundary::BeforeRename))
            });
            assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
            p.cancel_file("operation");
            assert!(p.accept_file("operation").unwrap().is_none());
            assert!(matches!(p.begin_file("operation"), Err(Error::Stopped)));
            if webkit {
                service
                    .execute(Command::Import(super::super::LegacyImport::default()))
                    .unwrap();
            }
            drop(p);
            drop(service);
            let service = CompletionService::start(dir.path().into(), false).unwrap();
            let snapshot = service.execute(Command::Snapshot).unwrap();
            assert!(snapshot.history.is_empty());
            assert_no_statistics(&snapshot);
        }
    }

    #[test]
    fn file_acceptance_storage_failure_retains_result_for_retry_or_cancellation() {
        for file in ["completion-holding.json", "completion-state.json"] {
            for cancel in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let service = CompletionService::start(dir.path().into(), false).unwrap();
                service.execute(Command::Snapshot).unwrap();
                let p = Producers::new(service.clone(), "synthetic".into());
                p.begin_file("operation").unwrap();
                p.finish_file(
                    "operation",
                    Some(json!({"schemaVersion":2,"pinned":false,"source":"file","id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
                    true,
                );
                // A directory at the file path is unreadable even under root.
                let path = dir.path().join(file);
                let saved = std::fs::read(&path).unwrap();
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
                assert!(matches!(p.accept_file("operation"), Err(Error::Io)));
                assert!(matches!(
                    p.files.lock().unwrap().get("operation"),
                    Some(FileState::Ready(Some(_)))
                ));
                if cancel {
                    p.cancel_file("operation");
                }
                std::fs::remove_dir(&path).unwrap();
                std::fs::write(&path, saved).unwrap();
                assert_eq!(p.accept_file("operation").unwrap().is_some(), !cancel);
                assert!(p.accept_file("operation").unwrap().is_none());
                if cancel {
                    assert!(matches!(p.begin_file("operation"), Err(Error::Stopped)));
                }
                let snapshot = service.execute(Command::Snapshot).unwrap();
                assert_eq!(snapshot.history.len(), usize::from(!cancel));
                assert_eq!(
                    serde_json::to_value(snapshot.statistics).unwrap(),
                    serde_json::to_value(super::super::Stats::default()).unwrap()
                );
            }
        }
    }

    #[test]
    fn file_staging_is_bounded_and_cancellation_releases_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let service = CompletionService::start(dir.path().into(), false).unwrap();
        let p = Producers::new(service, "synthetic".into());
        for n in 0..MAX_PENDING_FILES {
            let operation = n.to_string();
            p.begin_file(&operation).unwrap();
            p.finish_file(&operation, Some(json!({"text":"synthetic"})), true);
        }
        assert!(matches!(p.begin_file("overflow"), Err(Error::TooManyFiles)));
        p.cancel_file("0");
        p.begin_file("overflow").unwrap();
        // A cancelled identity still cannot be restarted while its tombstone remains.
        assert!(matches!(p.begin_file("0"), Err(Error::Stopped)));
        p.discard_file("overflow");
        p.begin_file("replacement").unwrap();
    }

    #[test]
    fn file_cancellation_before_during_after_transcription_and_after_acceptance() {
        for phase in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            let service = CompletionService::start(dir.path().into(), false).unwrap();
            let p = Producers::new(service.clone(), "synthetic".into());
            if phase == 0 {
                p.cancel_file("operation");
                assert!(p.begin_file("operation").is_err());
            } else {
                p.begin_file("operation").unwrap();
            }
            if phase == 1 {
                p.cancel_file("operation");
            }
            p.finish_file(
                "operation",
                Some(json!({"schemaVersion":2,"pinned":false,"source":"file","id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
                true,
            );
            if phase == 2 {
                p.cancel_file("operation");
            }
            p.accept_file("operation").unwrap();
            if phase == 3 {
                p.cancel_file("operation");
            }
            p.accept_file("operation").unwrap();
            let snapshot = service.execute(Command::Snapshot).unwrap();
            assert_eq!(snapshot.history.len(), usize::from(phase == 3));
            assert_eq!(snapshot.statistics.total_recordings, 0);
        }
    }
    #[test]
    fn file_cancellation_racing_transcription_finish_never_enters_history() {
        let dir = tempfile::tempdir().unwrap();
        let service = CompletionService::start(dir.path().into(), false).unwrap();
        let p = Producers::new(service.clone(), "synthetic".into());
        p.begin_file("operation").unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                p.cancel_file("operation");
            });
            barrier.wait();
            p.finish_file(
                "operation",
                Some(json!({"schemaVersion":2,"pinned":false,"source":"file","id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
                true,
            );
        });
        p.accept_file("operation").unwrap();
        assert!(service
            .execute(Command::Snapshot)
            .unwrap()
            .history
            .is_empty());
    }
}
