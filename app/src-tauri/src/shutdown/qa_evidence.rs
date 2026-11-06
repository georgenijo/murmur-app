//! Feature-only, content-free terminal evidence. No telemetry or IPC dependency.
use murmur_core::shutdown::{
    Action, Coordinator, Event, Outcome, Request, State, Step, WriteOutcome,
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const QA_ID: &str = "com.localdictation.shutdown-test";

#[derive(Default)]
pub(crate) struct DispatcherCounts {
    pub app_context: AtomicUsize,
    pub tauri_dispatch: AtomicUsize,
    pub cf_heartbeat: AtomicUsize,
}
impl DispatcherCounts {
    pub fn snapshot(&self) -> Counts {
        Counts {
            app_context: self.app_context.load(Ordering::Acquire),
            tauri_dispatch: self.tauri_dispatch.load(Ordering::Acquire),
            cf_heartbeat: self.cf_heartbeat.load(Ordering::Acquire),
        }
    }
}
#[derive(Clone, Copy, Default, Serialize)]
pub(crate) struct Counts {
    pub app_context: usize,
    pub tauri_dispatch: usize,
    pub cf_heartbeat: usize,
}
impl Counts {
    pub fn total(self) -> usize {
        self.app_context + self.tauri_dispatch + self.cf_heartbeat
    }
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Admission,
    Capture,
    Helpers,
    Writers,
    Terminate,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PhaseOutcome {
    Completed,
    PreservedForReplay,
    Failed,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum FailureCode {
    None,
    OwnerFailed,
    DeadlineExpired,
    UnpreservedFailure,
}
#[derive(Clone, Copy, Serialize)]
struct PhaseReceipt {
    phase: Phase,
    outcome: PhaseOutcome,
    failure_code: FailureCode,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalAction {
    Quit,
    Restart,
}
#[derive(Serialize)]
struct TerminalReceipt<'a> {
    schema_version: u8,
    pid: u32,
    process_start_id: &'a str,
    phases: Vec<PhaseReceipt>,
    dispatcher_callbacks: Counts,
    dispatcher_callbacks_total: usize,
    dispatcher_callbacks_expected: usize,
    terminal_action: TerminalAction,
    terminal_action_count: u8,
    replacement_requested: bool,
}

/// All payload fields are typed counts, booleans, generated identity or closed
/// code enums. This object never receives owner errors or application content.
pub(crate) struct Evidence {
    directory: PathBuf,
    pid: u32,
    process_start_id: String,
    phases: Vec<PhaseReceipt>,
    replacement_requested: bool,
    written: bool,
}
impl Evidence {
    pub fn new(
        identifier: &str,
        initialized: Option<&str>,
        data_dir: impl FnOnce() -> Option<PathBuf>,
    ) -> Option<Self> {
        // Refuse before resolving or touching any directory, even in a feature
        // build accidentally run with the production configuration.
        if identifier != QA_ID || initialized != Some(QA_ID) {
            return None;
        }
        Some(Self {
            directory: data_dir()?.join("shutdown-evidence"),
            pid: std::process::id(),
            process_start_id: uuid::Uuid::new_v4().to_string(),
            phases: Vec::with_capacity(5),
            replacement_requested: false,
            written: false,
        })
    }
    pub fn observe(
        &mut self,
        before: State,
        core: &Coordinator,
        event: Option<Event>,
        request: Option<Request>,
        now: u64,
    ) {
        if !matches!(before, State::TerminationIssued | State::Exited)
            && request == Some(Request::Restart)
        {
            self.replacement_requested = true;
        }
        // Called under the same reducer lock for request, tick and completion.
        // A request can itself expire multiple phases. Late owner completions
        // must never replace the deadline codes already recorded here.
        let index = |state| match state {
            State::Running | State::ClosingAdmission => 0,
            State::DrainingCapture => 1,
            State::ConfirmingHelpers => 2,
            State::FlushingWriters => 3,
            State::ReadyToTerminate | State::TerminationIssued | State::Exited => 4,
        };
        for phase_index in index(before)..index(core.state) {
            let (phase, step) = [
                (Phase::Admission, Step::Admission),
                (Phase::Capture, Step::Capture),
                (Phase::Helpers, Step::Helpers),
                (Phase::Writers, Step::Writers),
            ][phase_index];
            let accepted =
                event.filter(|event| event.effect.step == step && now <= event.effect.deadline_ms);
            let (outcome, failure_code) = match accepted.map(|event| event.outcome) {
                Some(
                    Outcome::Admission(true) | Outcome::Capture(true) | Outcome::Helpers(true),
                )
                | Some(Outcome::Writers(WriteOutcome::Committed)) => {
                    (PhaseOutcome::Completed, FailureCode::None)
                }
                Some(Outcome::Writers(WriteOutcome::PreservedForReplay)) => {
                    (PhaseOutcome::PreservedForReplay, FailureCode::None)
                }
                Some(Outcome::Writers(WriteOutcome::UnpreservedFailure)) => {
                    (PhaseOutcome::Failed, FailureCode::UnpreservedFailure)
                }
                Some(_) => (PhaseOutcome::Failed, FailureCode::OwnerFailed),
                None => (PhaseOutcome::Failed, FailureCode::DeadlineExpired),
            };
            self.phases.push(PhaseReceipt {
                phase,
                outcome,
                failure_code,
            });
        }
    }
    pub fn write_terminal(&mut self, action: Action, counts: Counts) -> io::Result<bool> {
        if self.written {
            return Ok(false);
        }
        let mut phases = self.phases.clone();
        phases.push(PhaseReceipt {
            phase: Phase::Terminate,
            outcome: PhaseOutcome::Completed,
            failure_code: FailureCode::None,
        });
        let receipt = TerminalReceipt {
            schema_version: 1,
            pid: self.pid,
            process_start_id: &self.process_start_id,
            phases,
            dispatcher_callbacks: counts,
            dispatcher_callbacks_total: counts.total(),
            dispatcher_callbacks_expected: 195,
            terminal_action: match action {
                Action::Quit => TerminalAction::Quit,
                Action::Restart => TerminalAction::Restart,
            },
            terminal_action_count: 1,
            replacement_requested: self.replacement_requested,
        };
        let bytes = serde_json::to_vec(&receipt)?;
        fs::create_dir_all(&self.directory)?;
        // Persist the directory itself when first created, too.
        File::open(self.directory.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()?;
        let path = self
            .directory
            .join(format!("{}-{}.json", self.pid, self.process_start_id));
        atomic_publish(&path, &bytes, || Ok(()))?;
        self.written = true;
        Ok(true)
    }
}

/// Publish an immutable name atomically: a reader sees either no JSON or the
/// complete synced file. Hard-link publication refuses to clobber an existing
/// per-process receipt, including after an uncertain directory-sync failure.
fn atomic_publish(
    path: &Path,
    bytes: &[u8],
    before_publish: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let staged = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        before_publish()?;
        fs::hard_link(&staged, path)?;
        File::open(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()
    })();
    drop(file);
    let cleanup = fs::remove_file(&staged);
    result?;
    cleanup?;
    File::open(path.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scratch() -> tempfile::TempDir {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/murmur-shutdown-evidence-tests");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    #[test]
    fn identity_gate_precedes_all_directory_access() {
        for identifier in ["com.localdictation", "", "future.id", QA_ID] {
            for initialized in [None, Some("com.localdictation"), Some(QA_ID)] {
                let allowed = identifier == QA_ID && initialized == Some(QA_ID);
                let mut resolved = false;
                let evidence = Evidence::new(identifier, initialized, || {
                    resolved = true;
                    Some(PathBuf::new())
                });
                assert_eq!(resolved, allowed);
                assert_eq!(evidence.is_some(), allowed);
            }
        }
    }
    #[test]
    fn phase_deadlines_and_late_completions_do_not_erase_failure_codes() {
        let root = scratch();
        let mut evidence =
            Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        let mut core = Coordinator::default();
        let before = core.state;
        let admission = core.request(Request::Restart, 0)[0];
        evidence.observe(before, &core, None, Some(Request::Restart), 0);
        // A mixed request can expire every outstanding phase in one reducer call.
        let before = core.state;
        core.request(Request::Quit, 10_001);
        evidence.observe(before, &core, None, Some(Request::Quit), 10_001);
        let late = Event {
            effect: admission,
            outcome: Outcome::Admission(true),
        };
        let before = core.state;
        assert!(core.advance(late, 10_002).is_empty());
        evidence.observe(before, &core, Some(late), None, 10_002);
        assert_eq!(evidence.phases.len(), 4);
        for phase in &evidence.phases {
            assert!(matches!(phase.failure_code, FailureCode::DeadlineExpired));
        }
        assert!(evidence.replacement_requested);
        assert_eq!(core.issue(), Some(Action::Quit));
    }
    #[test]
    fn owner_failure_and_preserved_writes_have_distinct_codes() {
        for writes in [
            WriteOutcome::PreservedForReplay,
            WriteOutcome::UnpreservedFailure,
        ] {
            let root = scratch();
            let mut evidence =
                Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
            let mut core = Coordinator::default();
            let mut effect = core.request(Request::Quit, 0)[0];
            for outcome in [
                Outcome::Admission(false),
                Outcome::Capture(true),
                Outcome::Helpers(false),
                Outcome::Writers(writes),
            ] {
                let before = core.state;
                let event = Event { effect, outcome };
                effect = core.advance(event, 1)[0];
                evidence.observe(before, &core, Some(event), None, 1);
            }
            let phases = serde_json::to_value(&evidence.phases).unwrap();
            assert_eq!(phases[0]["failure_code"], "owner_failed");
            assert_eq!(phases[1]["failure_code"], "none");
            assert_eq!(phases[2]["failure_code"], "owner_failed");
            if writes == WriteOutcome::PreservedForReplay {
                assert_eq!(phases[3]["outcome"], "preserved_for_replay");
                assert_eq!(phases[3]["failure_code"], "none");
            } else {
                assert_eq!(phases[3]["outcome"], "failed");
                assert_eq!(phases[3]["failure_code"], "unpreserved_failure");
            }
        }
    }
    #[test]
    fn atomic_publication_never_exposes_partial_json_or_clobbers_receipt() {
        let root = scratch();
        let path = root.path().join("synthetic.json");
        let bytes = br#"{"count":195}"#;
        let failure = atomic_publish(&path, bytes, || {
            assert!(!path.exists());
            assert_eq!(fs::read(path.with_extension("pending")).unwrap(), bytes);
            Err(io::ErrorKind::Interrupted.into())
        });
        assert!(failure.is_err());
        assert!(!path.exists());
        assert!(!path.with_extension("pending").exists());
        atomic_publish(&path, bytes, || {
            assert!(!path.exists());
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            atomic_publish(&path, b"different", || Ok(()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn one_receipt_per_process_with_closed_content_schema() {
        let root = scratch();
        let mut evidence =
            Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        let start = evidence.process_start_id.clone();
        let counts = Counts {
            app_context: 65,
            tauri_dispatch: 65,
            cf_heartbeat: 65,
        };
        assert!(evidence.write_terminal(Action::Quit, counts).unwrap());
        assert!(!evidence
            .write_terminal(Action::Restart, Counts::default())
            .unwrap());
        let files: Vec<_> = fs::read_dir(root.path().join("shutdown-evidence"))
            .unwrap()
            .collect();
        assert_eq!(files.len(), 1);
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(files[0].as_ref().unwrap().path()).unwrap()).unwrap();
        let mut keys: Vec<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "dispatcher_callbacks",
                "dispatcher_callbacks_expected",
                "dispatcher_callbacks_total",
                "phases",
                "pid",
                "process_start_id",
                "replacement_requested",
                "schema_version",
                "terminal_action",
                "terminal_action_count"
            ]
        );
        assert_eq!(value["process_start_id"], start);
        assert_eq!(value["terminal_action"], "quit");
        assert_eq!(value["terminal_action_count"], 1);
        assert_eq!(value["dispatcher_callbacks_total"], 195);
        assert_eq!(
            value["dispatcher_callbacks"],
            serde_json::json!({"app_context":65,"tauri_dispatch":65,"cf_heartbeat":65})
        );
        assert_eq!(
            value["phases"][0],
            serde_json::json!({"phase":"terminate","outcome":"completed","failure_code":"none"})
        );
        // A second process has an independent start identity and receipt.
        let mut next = Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        assert_ne!(next.process_start_id, start);
        next.write_terminal(Action::Quit, counts).unwrap();
        assert_eq!(
            fs::read_dir(root.path().join("shutdown-evidence"))
                .unwrap()
                .count(),
            2
        );
    }
}
