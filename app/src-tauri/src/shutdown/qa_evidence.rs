//! Feature-only, content-free terminal evidence. No telemetry or IPC dependency.
use murmur_core::shutdown::{
    Action, Coordinator, Event, Outcome, Request, State, Step, WriteOutcome,
};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const QA_ID: &str = "com.localdictation.shutdown-test";

/// One wait shared by the queued callback and the permitted runtime event.
/// Neither filesystem work, diagnostics nor an evidence mutex runs on main.
pub(crate) const TERMINAL_EVIDENCE_BUDGET: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum AttemptOutcome {
    InProgress,
    CompletedWithinBound,
    TimedOut,
    IoFailure,
    Panicked,
    Unavailable,
}

#[derive(Default)]
pub(crate) struct TerminalAttempt {
    started: AtomicBool,
    outcome: AtomicU8,
}

fn contain<T>(work: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(result) => Some(result),
        Err(payload) => {
            // Even dropping a custom panic payload can panic. Evidence must
            // never propagate that second unwind into the terminal caller.
            std::mem::forget(payload);
            None
        }
    }
}
impl TerminalAttempt {
    pub const fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            outcome: AtomicU8::new(0),
        }
    }

    pub fn run(&self, write: impl FnOnce() -> io::Result<()> + Send + 'static) -> AttemptOutcome {
        if self.started.swap(true, Ordering::AcqRel) {
            return self.outcome();
        }
        let deadline = Instant::now() + TERMINAL_EVIDENCE_BUDGET;
        // Also contain a panic during thread setup; callers additionally contain
        // evidence preparation so no evidence panic can skip the native action.
        let result = contain(|| {
            let (done, receive) = mpsc::channel();
            let (report, reported) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("shutdown-qa-evidence".into())
                .spawn(move || {
                    let result = match contain(write) {
                        Some(Ok(())) => AttemptOutcome::CompletedWithinBound,
                        Some(Err(_)) => AttemptOutcome::IoFailure,
                        None => AttemptOutcome::Panicked,
                    };
                    let _ = done.send((result, Instant::now()));
                    // The waiting caller records the actual bounded-wait result.
                    // Diagnostics can themselves block or fail without delaying
                    // termination. Never print paths, payloads or panic messages.
                    if let Ok(outcome) = reported.recv() {
                        let _ = contain(|| {
                            diagnostic(&mut io::stderr(), outcome);
                        });
                    }
                });
            let outcome = if spawned.is_err() {
                AttemptOutcome::Unavailable
            } else {
                match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok((outcome, completed)) if completed <= deadline => outcome,
                    Ok(_) => AttemptOutcome::TimedOut,
                    Err(mpsc::RecvTimeoutError::Timeout) => AttemptOutcome::TimedOut,
                    Err(mpsc::RecvTimeoutError::Disconnected) => AttemptOutcome::Panicked,
                }
            };
            self.outcome.store(outcome as u8, Ordering::Release);
            let _ = report.send(outcome);
            outcome
        })
        .unwrap_or(AttemptOutcome::Panicked);
        self.outcome.store(result as u8, Ordering::Release);
        result
    }

    fn outcome(&self) -> AttemptOutcome {
        match self.outcome.load(Ordering::Acquire) {
            1 => AttemptOutcome::CompletedWithinBound,
            2 => AttemptOutcome::TimedOut,
            3 => AttemptOutcome::IoFailure,
            4 => AttemptOutcome::Panicked,
            5 => AttemptOutcome::Unavailable,
            _ => AttemptOutcome::InProgress,
        }
    }
}

fn diagnostic(output: &mut impl Write, outcome: AttemptOutcome) {
    let message: &[u8] = match outcome {
        AttemptOutcome::CompletedWithinBound => {
            b"shutdown QA evidence: completed_within_bound=true\n"
        }
        AttemptOutcome::TimedOut => {
            b"shutdown QA evidence: completed_within_bound=false timed_out\n"
        }
        AttemptOutcome::IoFailure => {
            b"shutdown QA evidence: completed_within_bound=false receipt_io_failure\n"
        }
        AttemptOutcome::Panicked => {
            b"shutdown QA evidence: completed_within_bound=false evidence_panicked\n"
        }
        AttemptOutcome::Unavailable => {
            b"shutdown QA evidence: completed_within_bound=false worker_unavailable\n"
        }
        AttemptOutcome::InProgress => return,
    };
    let _ = output.write_all(message);
}

/// The terminal continuation runs once even if evidence preparation panics.
pub(crate) fn with_terminal_evidence<T>(
    evidence: impl FnOnce(),
    terminal: impl FnOnce() -> T,
) -> T {
    let _ = contain(evidence);
    terminal()
}

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
        self.write_terminal_with_sync(action, counts, File::sync_all)
    }

    fn write_terminal_with_sync(
        &mut self,
        action: Action,
        counts: Counts,
        mut sync: impl FnMut(&File) -> io::Result<()>,
    ) -> io::Result<bool> {
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
        sync(&File::open(
            self.directory.parent().ok_or(io::ErrorKind::InvalidInput)?,
        )?)?;
        let path = self
            .directory
            .join(format!("{}-{}.json", self.pid, self.process_start_id));
        atomic_publish(&path, &bytes, || Ok(()), &mut sync)?;
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
    sync: &mut impl FnMut(&File) -> io::Result<()>,
) -> io::Result<()> {
    let staged = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let result = (|| {
        file.write_all(bytes)?;
        sync(&file)?;
        before_publish()?;
        fs::hard_link(&staged, path)?;
        sync(&File::open(
            path.parent().ok_or(io::ErrorKind::InvalidInput)?,
        )?)
    })();
    drop(file);
    let cleanup = fs::remove_file(&staged);
    result?;
    cleanup?;
    sync(&File::open(
        path.parent().ok_or(io::ErrorKind::InvalidInput)?,
    )?)
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
    fn successful_bounded_write_is_recorded_once_with_content_free_status() {
        let root = scratch();
        let mut evidence =
            Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        let attempt = TerminalAttempt::new();
        let outcome = attempt.run(move || {
            evidence
                .write_terminal(Action::Quit, Counts::default())
                .map(|_| ())
        });
        assert_eq!(outcome, AttemptOutcome::CompletedWithinBound);
        assert_eq!(
            attempt.run(|| panic!("duplicate evidence attempt")),
            outcome
        );
        let mut output = Vec::new();
        diagnostic(&mut output, outcome);
        assert_eq!(
            output,
            b"shutdown QA evidence: completed_within_bound=true\n"
        );
        assert_eq!(
            fs::read_dir(root.path().join("shutdown-evidence"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn slow_fsync_cannot_hold_terminal_action_or_start_a_second_attempt() {
        let root = scratch();
        let mut evidence =
            Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        let attempt = TerminalAttempt::new();
        let (finished, drained) = mpsc::channel();
        let start = Instant::now();
        let mut terminal_count = 0;
        with_terminal_evidence(
            || {
                assert_eq!(
                    attempt.run(move || {
                        let mut sync_count = 0;
                        let result = evidence.write_terminal_with_sync(
                            Action::Quit,
                            Counts::default(),
                            |file| {
                                sync_count += 1;
                                std::thread::sleep(Duration::from_secs(3));
                                file.sync_all()
                            },
                        );
                        let _ = finished.send(sync_count);
                        result.map(|_| ())
                    }),
                    AttemptOutcome::TimedOut
                );
            },
            || {
                terminal_count += 1;
                assert!(start.elapsed() <= Duration::from_millis(250));
            },
        );
        // A runtime event arriving after the queued callback consumes no second
        // budget and cannot try to lock or join the blocked evidence writer.
        assert_eq!(
            attempt.run(|| panic!("duplicate evidence attempt")),
            AttemptOutcome::TimedOut
        );
        assert!(start.elapsed() <= Duration::from_millis(250));
        assert_eq!(terminal_count, 1);
        assert_eq!(drained.recv_timeout(Duration::from_secs(15)).unwrap(), 4);
        // A late disk completion cannot rewrite the bounded-wait result.
        assert_eq!(attempt.outcome(), AttemptOutcome::TimedOut);
    }

    #[test]
    fn evidence_panics_still_run_the_terminal_continuation_once() {
        for preparation_panics in [false, true] {
            let attempt = TerminalAttempt::new();
            let mut terminal_count = 0;
            with_terminal_evidence(
                || {
                    if preparation_panics {
                        panic!("synthetic evidence preparation panic");
                    }
                    assert_eq!(
                        attempt.run(|| panic!("synthetic evidence worker panic")),
                        AttemptOutcome::Panicked
                    );
                },
                || terminal_count += 1,
            );
            assert_eq!(terminal_count, 1);
            if !preparation_panics {
                assert_eq!(attempt.outcome(), AttemptOutcome::Panicked);
            }
        }
    }

    #[test]
    fn panicking_payload_destructor_cannot_skip_terminal_action() {
        struct Payload;
        impl Drop for Payload {
            fn drop(&mut self) {
                panic!("synthetic panic payload destructor");
            }
        }
        let mut terminal_count = 0;
        with_terminal_evidence(|| std::panic::panic_any(Payload), || terminal_count += 1);
        assert_eq!(terminal_count, 1);
        assert_eq!(
            TerminalAttempt::new().run(|| std::panic::panic_any(Payload)),
            AttemptOutcome::Panicked
        );
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_destination_and_broken_stderr_cannot_skip_terminal_action() {
        const CHILD: &str = "MURMUR_QA_BROKEN_STDERR_CHILD";
        if std::env::var_os(CHILD).is_none() {
            use std::{
                os::{fd::OwnedFd, unix::net::UnixStream},
                process::{Command, Stdio},
            };
            let (reader, writer) = UnixStream::pair().unwrap();
            drop(reader);
            let fd: OwnedFd = writer.into();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "shutdown::qa_evidence::tests::unwritable_destination_and_broken_stderr_cannot_skip_terminal_action", "--nocapture"])
                .env(CHILD, "1")
                .stderr(Stdio::from(fd))
                .output()
                .unwrap();
            assert!(output.status.success(), "child failed: {:?}", output.status);
            assert!(String::from_utf8_lossy(&output.stdout).contains("terminal-count=1"));
            return;
        }
        // A file in place of the destination directory fails even as root.
        assert!(io::stderr()
            .write_all(b"synthetic broken-stderr probe\n")
            .is_err());
        let root = scratch();
        fs::write(root.path().join("shutdown-evidence"), b"synthetic blocker").unwrap();
        let mut evidence =
            Evidence::new(QA_ID, Some(QA_ID), || Some(root.path().to_owned())).unwrap();
        let attempt = TerminalAttempt::new();
        let mut terminal_count = 0;
        with_terminal_evidence(
            || {
                let outcome = attempt.run(move || {
                    evidence
                        .write_terminal(Action::Quit, Counts::default())
                        .map(|_| ())
                });
                assert_eq!(outcome, AttemptOutcome::IoFailure);
                // Exercise the exact diagnostic against the child's real broken
                // stderr, without changing the parent test runner's descriptors.
                diagnostic(&mut io::stderr(), outcome);
            },
            || terminal_count += 1,
        );
        assert_eq!(terminal_count, 1);
        assert_eq!(attempt.outcome(), AttemptOutcome::IoFailure);
        assert!(catch_unwind(|| diagnostic(&mut io::stderr(), AttemptOutcome::IoFailure)).is_ok());
        println!("terminal-count={terminal_count}");
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
        let failure = atomic_publish(
            &path,
            bytes,
            || {
                assert!(!path.exists());
                assert_eq!(fs::read(path.with_extension("pending")).unwrap(), bytes);
                Err(io::ErrorKind::Interrupted.into())
            },
            &mut File::sync_all,
        );
        assert!(failure.is_err());
        assert!(!path.exists());
        assert!(!path.with_extension("pending").exists());
        atomic_publish(
            &path,
            bytes,
            || {
                assert!(!path.exists());
                Ok(())
            },
            &mut File::sync_all,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            atomic_publish(&path, b"different", || Ok(()), &mut File::sync_all)
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
