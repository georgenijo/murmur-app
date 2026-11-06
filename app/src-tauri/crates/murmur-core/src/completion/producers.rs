//! Production completion admission shared by native hosts and headless effects.
use super::{Command, CompletionService, Error, Result, Scope, Snapshot, StatisticsEvent};
use serde_json::Value;
use std::{collections::HashMap, sync::Mutex};

// Bound transcript-bearing operations even if a client disappears before accepting.
const MAX_PENDING_FILES: usize = 200;

pub struct Producers {
    service: CompletionService,
    session: String,
    files: Mutex<HashMap<String, FileState>>,
}
enum FileState {
    Running,
    Cancelled,
    Ready(Option<Value>),
}
impl Producers {
    pub fn new(service: CompletionService, session: String) -> Self {
        Self {
            service,
            session,
            files: Mutex::new(HashMap::new()),
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
                files.insert(operation.into(), FileState::Running);
                Ok(())
            }
        }
    }
    pub fn cancel_file(&self, operation: &str) {
        // Also handles cancellation delivered before begin_file. No transcript
        // survives cancellation. Acceptance and cancellation share this lock.
        self.files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(operation.into(), FileState::Cancelled);
    }
    pub fn finish_file(&self, operation: &str, entry: Option<Value>, retain: bool) {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        if matches!(files.get(operation), Some(FileState::Running)) {
            files.insert(operation.into(), FileState::Ready(entry.filter(|_| retain)));
        }
    }
    /// The client accepts only after its cancellation check. A late cancellation
    /// before acceptance still wins, including after native transcription ends.
    pub fn accept_file(&self, operation: &str) -> Result<Option<Snapshot>> {
        let mut files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        let snapshot = match files.get(operation) {
            Some(FileState::Ready(Some(entry))) => {
                Some(self.submit(format!("file:{operation}"), Some(entry.clone()), None)?)
            }
            Some(FileState::Ready(None)) => None,
            _ => return Ok(None),
        };
        // Submission errors leave the result staged. The shared lock makes a
        // later retry and cancellation mutually exclusive, just like first acceptance.
        files.remove(operation);
        Ok(snapshot)
    }
    pub fn discard_file(&self, operation: &str) {
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
