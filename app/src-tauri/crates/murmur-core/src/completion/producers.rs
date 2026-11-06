//! Production completion admission shared by native hosts and headless effects.
use super::{Command, CompletionService, Error, Result, Scope, Snapshot, StatisticsEvent};
use serde_json::Value;
use std::{
    collections::HashMap,
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
    intents: Mutex<HashMap<String, Arc<FileIntent>>>,
    #[cfg(test)]
    cancel_recorded: Mutex<Option<std::sync::mpsc::Sender<()>>>,
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
    fn cancel(&self) {
        self.0.fetch_or(Self::CANCELLED, Ordering::SeqCst);
    }
    pub(super) fn begin_publication(&self) -> bool {
        self.0
            .compare_exchange(0, Self::PUBLISHING, Ordering::SeqCst, Ordering::SeqCst)
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
            intents: Mutex::new(HashMap::new()),
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
                if files
                    .values()
                    .filter(|state| !matches!(state, FileState::Cancelled))
                    .count()
                    >= MAX_PENDING_FILES
                {
                    return Err(Error::TooManyFiles);
                }
                if self.intent(operation).0.load(Ordering::SeqCst) != 0 {
                    return Err(Error::Stopped);
                }
                files.insert(operation.into(), FileState::Running);
                Ok(())
            }
        }
    }
    fn intent(&self, operation: &str) -> Arc<FileIntent> {
        self.intents
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(operation.into())
            .or_default()
            .clone()
    }
    pub fn cancel_file(&self, operation: &str) {
        // Release the small registry lock before taking files. The actor only
        // uses atomics, so files -> owner queue can never invert this ordering.
        self.intent(operation).cancel();
        #[cfg(test)]
        if let Some(send) = self.cancel_recorded.lock().unwrap().take() {
            send.send(()).unwrap();
        }
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(operation.into(), FileState::Cancelled);
    }
    pub fn finish_file(&self, operation: &str, entry: Option<Value>, retain: bool) {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
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
        let snapshot = match files.get(operation) {
            Some(FileState::Ready(Some(entry))) => Some(
                self.service.execute(Command::AcceptFile {
                    entry: entry.clone(),
                    intent: self.intent(operation),
                    active: files
                        .values()
                        .filter_map(|state| match state {
                            FileState::Ready(Some(v)) => {
                                v["completionUuid"].as_str().map(str::to_owned)
                            }
                            _ => None,
                        })
                        .collect(),
                })?,
            ),
            Some(FileState::Ready(None)) => None,
            _ => return Ok(None),
        };
        // Errors retain staging; cancellation intent already reached the actor
        // even if cancel_file is still waiting to erase its content here.
        files.remove(operation);
        if self.intent(operation).cancelled_before_admission() {
            Ok(None)
        } else {
            Ok(snapshot)
        }
    }
    pub fn discard_file(&self, operation: &str) {
        self.intent(operation).cancel();
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(operation);
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
            Some(json!({"id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
            true,
        );
        p
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
                        for n in 0..200 {
                            p.dictation(n, json!({"id":format!("synthetic-{n}"),"text":"synthetic","timestamp":n+2,"duration":1}), true, None, "2026-11-05".into()).unwrap();
                        }
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
                Some(json!({"id":operation,"text":"synthetic","timestamp":n+2,"duration":1})),
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
                    Some(json!({"id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
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
                Some(json!({"id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
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
                Some(json!({"id":"synthetic","text":"synthetic file","timestamp":1,"duration":2})),
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
