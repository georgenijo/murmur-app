//! Killable Core ML model installation boundary.
//!
//! FluidAudio exposes model setup as one synchronous native call. The Tauri
//! process therefore launches its own already-signed executable in this fixed
//! worker mode, owns the exact child process group, and accepts only a small
//! content-free line protocol. A hard deadline always ends with confirmed
//! process-group termination or quarantined ownership before another Core ML
//! worker can begin.

use crate::managed_child::ManagedChild;
use crate::transcriber::coreml::{
    self, ModelPreparationError, ModelPreparationPhase, COREML_MODEL_NAME,
};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc;
#[cfg(debug_assertions)]
use std::sync::OnceLock;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const WORKER_ARGUMENT: &str = "--coreml-install-worker-v1";
const PROTOCOL_PREFIX: &str = "MRMR_COREML_INSTALL_V1 ";
const MAX_PROTOCOL_LINE_BYTES: usize = 4 * 1024;
const MAX_PROTOCOL_MESSAGES: usize = 32;
const MAX_IGNORED_STDOUT_LINES: usize = 256;
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
pub(crate) const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
static UNCONFIRMED_INSTALLER: LazyLock<Mutex<Option<ManagedChild>>> =
    LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallPhase {
    Preparing,
    Repairing,
    Initializing,
    Validating,
}

impl InstallPhase {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Repairing => "repairing",
            Self::Initializing => "initializing",
            Self::Validating => "validating",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkerOutcome {
    Success,
    UnknownModel,
    CacheUnavailable,
    RepairStateUnavailable,
    CacheRepairFailed,
    NativeInitializationFailed,
    ValidationFailed,
}

impl WorkerOutcome {
    fn from_preparation_error(error: ModelPreparationError) -> Self {
        match error {
            ModelPreparationError::UnknownModel => Self::UnknownModel,
            ModelPreparationError::CacheUnavailable => Self::CacheUnavailable,
            ModelPreparationError::RepairStateUnavailable => Self::RepairStateUnavailable,
            ModelPreparationError::RepairFailed => Self::CacheRepairFailed,
            ModelPreparationError::NativeInitializationFailed => Self::NativeInitializationFailed,
            ModelPreparationError::ValidationFailed => Self::ValidationFailed,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::UnknownModel => "unknown_model",
            Self::CacheUnavailable => "cache_unavailable",
            Self::RepairStateUnavailable => "repair_state_unavailable",
            Self::CacheRepairFailed => "cache_repair_failed",
            Self::NativeInitializationFailed => "native_initialization_failed",
            Self::ValidationFailed => "validation_failed",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WorkerMessage {
    Phase {
        phase: InstallPhase,
        #[serde(rename = "repeatedRepair")]
        repeated_repair: bool,
    },
    Terminal {
        outcome: WorkerOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstallReceipt {
    pub(crate) repaired_cache: bool,
    pub(crate) repeated_repair: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InstallFailure {
    pub(crate) code: &'static str,
    pub(crate) repaired_cache: bool,
    pub(crate) repeated_repair: bool,
    pub(crate) termination_confirmed: bool,
}

impl InstallFailure {
    pub(crate) fn user_message(self) -> String {
        let detail = match self.code {
            "installer_timeout" => {
                "Core ML setup stopped after reaching its time limit. No installer was left running."
            }
            "cache_repair_failed" => "Murmur could not safely repair the incomplete Core ML cache.",
            "repair_state_unavailable" => {
                "Murmur could not safely track the incomplete Core ML cache repair."
            }
            "validation_failed" => "Core ML setup finished, but the installed cache did not validate.",
            "termination_unconfirmed" => {
                "Core ML setup stopped, but Murmur could not confirm installer cleanup. Murmur will recheck cleanup before another Core ML attempt."
            }
            "native_initialization_failed" => "Core ML model setup failed in the native installer.",
            "cache_unavailable" => "The Core ML model cache is unavailable.",
            _ => "Core ML model setup failed.",
        };
        let repeated = if self.repeated_repair {
            " This incomplete cache has already required repair on an earlier attempt."
        } else {
            ""
        };
        format!(
            "{detail}{repeated} Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback."
        )
    }
}

enum ReaderEvent {
    Message(WorkerMessage),
    Invalid,
    End,
}

fn valid_phase_transition(previous: Option<InstallPhase>, next: InstallPhase) -> bool {
    matches!(
        (previous, next),
        (None, InstallPhase::Preparing)
            | (Some(InstallPhase::Preparing), InstallPhase::Repairing)
            | (Some(InstallPhase::Preparing), InstallPhase::Initializing)
            | (Some(InstallPhase::Repairing), InstallPhase::Initializing)
            | (Some(InstallPhase::Initializing), InstallPhase::Validating)
    )
}

fn retain_unconfirmed_installer(child: ManagedChild) {
    let mut slot = UNCONFIRMED_INSTALLER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.is_none() {
        *slot = Some(child);
    }
}

fn terminate_or_retain(mut child: ManagedChild) -> bool {
    if child
        .hard_kill_confirmed(Instant::now() + TERMINATION_GRACE)
        .is_some()
    {
        true
    } else {
        retain_unconfirmed_installer(child);
        false
    }
}

fn clear_quarantined_installer() -> bool {
    let mut slot = UNCONFIRMED_INSTALLER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(child) = slot.as_mut() else {
        return true;
    };
    if child
        .hard_kill_confirmed(Instant::now() + TERMINATION_GRACE)
        .is_some()
    {
        slot.take();
        true
    } else {
        false
    }
}

fn emit_worker_message(message: &WorkerMessage) -> Result<(), ()> {
    let encoded = serde_json::to_string(message).map_err(|_| ())?;
    if PROTOCOL_PREFIX.len() + encoded.len() + 1 > MAX_PROTOCOL_LINE_BYTES {
        return Err(());
    }
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "{PROTOCOL_PREFIX}{encoded}").map_err(|_| ())?;
    output.flush().map_err(|_| ())
}

fn read_bounded_protocol_line<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, ()> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().map_err(|_| ())?;
        if buffer.is_empty() {
            return if line.is_empty() { Ok(None) } else { Err(()) };
        }
        if let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
            let consumed = newline + 1;
            if line.len() + consumed > MAX_PROTOCOL_LINE_BYTES {
                return Err(());
            }
            line.extend_from_slice(&buffer[..consumed]);
            reader.consume(consumed);
            return Ok(Some(line));
        }
        if line.len() + buffer.len() > MAX_PROTOCOL_LINE_BYTES {
            return Err(());
        }
        let consumed = buffer.len();
        line.extend_from_slice(buffer);
        reader.consume(consumed);
    }
}

fn start_parent_watchdog() {
    std::thread::spawn(|| {
        let mut input = std::io::stdin();
        let mut byte = [0_u8; 1];
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => {
                    #[cfg(unix)]
                    unsafe {
                        // ManagedChild starts this worker as the leader of a
                        // dedicated process group. If the host disappears
                        // without running its owner drop, terminate the worker
                        // and every native descendant that inherited the group.
                        libc::kill(0, libc::SIGKILL);
                    }
                    std::process::exit(74);
                }
                Ok(_) => {}
            }
        }
    });
}

#[cfg(debug_assertions)]
fn run_debug_scenario(scenario: &str) -> Option<i32> {
    let phase = |phase, repeated_repair| {
        emit_worker_message(&WorkerMessage::Phase {
            phase,
            repeated_repair,
        })
    };
    let terminal = |outcome| emit_worker_message(&WorkerMessage::Terminal { outcome });
    let success = || {
        let _ = phase(InstallPhase::Initializing, false);
        let _ = phase(InstallPhase::Validating, false);
        let _ = terminal(WorkerOutcome::Success);
        Some(0)
    };
    match scenario {
        "hang" => {
            let _ = phase(InstallPhase::Initializing, false);
            loop {
                std::thread::park_timeout(Duration::from_secs(60));
            }
        }
        "failure" => {
            let _ = phase(InstallPhase::Initializing, false);
            let _ = terminal(WorkerOutcome::NativeInitializationFailed);
            Some(21)
        }
        "validation_failure" => {
            let _ = phase(InstallPhase::Initializing, false);
            let _ = phase(InstallPhase::Validating, false);
            let _ = terminal(WorkerOutcome::ValidationFailed);
            Some(22)
        }
        "repeated_repair_failure" => {
            let _ = phase(InstallPhase::Repairing, true);
            let _ = phase(InstallPhase::Initializing, false);
            let _ = terminal(WorkerOutcome::NativeInitializationFailed);
            Some(23)
        }
        "hang_once_then_success" => {
            let token = std::env::var("MURMUR_COREML_INSTALL_SCENARIO_TOKEN").ok()?;
            let marker = std::env::temp_dir().join(format!("murmur-coreml-scenario-{token}"));
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&marker)
            {
                Ok(_) => {
                    let _ = phase(InstallPhase::Initializing, false);
                    loop {
                        std::thread::park_timeout(Duration::from_secs(60));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(marker);
                    success()
                }
                Err(_) => {
                    let _ = terminal(WorkerOutcome::NativeInitializationFailed);
                    Some(24)
                }
            }
        }
        "success" => success(),
        // Exercises the real FluidAudio bridge while allowing the debug-only
        // parent validation seam to force the onboarding gate open.
        "actual" => None,
        _ => None,
    }
}

/// Dispatch the internal worker before Tauri/AppKit startup.
pub fn run_cli_if_requested() -> Option<i32> {
    let mut arguments = std::env::args();
    let _executable = arguments.next();
    if arguments.next().as_deref() != Some(WORKER_ARGUMENT) {
        return None;
    }
    let Some(model_name) = arguments.next() else {
        return Some(64);
    };
    if arguments.next().is_some() || model_name != COREML_MODEL_NAME {
        return Some(64);
    }

    start_parent_watchdog();
    if emit_worker_message(&WorkerMessage::Phase {
        phase: InstallPhase::Preparing,
        repeated_repair: false,
    })
    .is_err()
    {
        return Some(70);
    }

    #[cfg(debug_assertions)]
    if let Ok(scenario) = std::env::var("MURMUR_COREML_INSTALL_SCENARIO") {
        if let Some(exit_code) = run_debug_scenario(&scenario) {
            return Some(exit_code);
        }
    }

    let result = coreml::prepare_model_with_observer(&model_name, |phase| {
        let (phase, repeated_repair) = match phase {
            ModelPreparationPhase::Repairing { repeated } => (InstallPhase::Repairing, repeated),
            ModelPreparationPhase::Initializing => (InstallPhase::Initializing, false),
            ModelPreparationPhase::Validating => (InstallPhase::Validating, false),
        };
        let _ = emit_worker_message(&WorkerMessage::Phase {
            phase,
            repeated_repair,
        });
    });
    let outcome = match result {
        Ok(()) => WorkerOutcome::Success,
        Err(error) => WorkerOutcome::from_preparation_error(error),
    };
    if emit_worker_message(&WorkerMessage::Terminal { outcome }).is_err() {
        return Some(70);
    }
    Some(if outcome == WorkerOutcome::Success {
        0
    } else {
        20
    })
}

fn reader_thread(stdout: std::process::ChildStdout, sender: mpsc::Sender<ReaderEvent>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut messages = 0_usize;
        let mut ignored_lines = 0_usize;
        loop {
            match read_bounded_protocol_line(&mut reader) {
                Ok(None) => {
                    let _ = sender.send(ReaderEvent::End);
                    return;
                }
                Ok(Some(line)) => {
                    let body = &line[..line.len() - 1];
                    let Some(protocol) = body.strip_prefix(PROTOCOL_PREFIX.as_bytes()) else {
                        ignored_lines += 1;
                        if ignored_lines > MAX_IGNORED_STDOUT_LINES {
                            let _ = sender.send(ReaderEvent::Invalid);
                            return;
                        }
                        continue;
                    };
                    messages += 1;
                    if messages > MAX_PROTOCOL_MESSAGES {
                        let _ = sender.send(ReaderEvent::Invalid);
                        return;
                    }
                    match serde_json::from_slice::<WorkerMessage>(protocol) {
                        Ok(message) => {
                            if sender.send(ReaderEvent::Message(message)).is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            let _ = sender.send(ReaderEvent::Invalid);
                            return;
                        }
                    }
                }
                Err(()) => {
                    let _ = sender.send(ReaderEvent::Invalid);
                    return;
                }
            }
        }
    });
}

fn supervise_process<F>(
    executable: &Path,
    arguments: &[&str],
    environment: &[(String, String)],
    timeout: Duration,
    mut on_progress: F,
) -> Result<InstallReceipt, InstallFailure>
where
    F: FnMut(InstallPhase, bool),
{
    if !clear_quarantined_installer() {
        return Err(InstallFailure {
            code: "termination_unconfirmed",
            repaired_cache: false,
            repeated_repair: false,
            termination_confirmed: false,
        });
    }
    let (mut child, child_stdin, stdout) =
        ManagedChild::spawn_with_arguments(executable, arguments, environment).map_err(|_| {
            InstallFailure {
                code: "spawn_failed",
                repaired_cache: false,
                repeated_repair: false,
                termination_confirmed: true,
            }
        })?;
    let _child_stdin = child_stdin;
    let (sender, receiver) = mpsc::channel();
    reader_thread(stdout, sender);

    let deadline = Instant::now() + timeout;
    let mut terminal = None;
    let mut repaired_cache = false;
    let mut repeated_repair = false;
    let mut last_phase = None;
    loop {
        let now = Instant::now();
        if now >= deadline {
            let confirmed = terminate_or_retain(child);
            return Err(InstallFailure {
                code: if confirmed {
                    "installer_timeout"
                } else {
                    "termination_unconfirmed"
                },
                repaired_cache,
                repeated_repair,
                termination_confirmed: confirmed,
            });
        }

        let wait = deadline
            .saturating_duration_since(now)
            .min(Duration::from_millis(50));
        match receiver.recv_timeout(wait) {
            Ok(ReaderEvent::Message(WorkerMessage::Phase {
                phase,
                repeated_repair: repeated,
            })) if terminal.is_none() => {
                if (repeated && phase != InstallPhase::Repairing)
                    || !valid_phase_transition(last_phase, phase)
                {
                    let confirmed = terminate_or_retain(child);
                    return Err(InstallFailure {
                        code: if confirmed {
                            "protocol_error"
                        } else {
                            "termination_unconfirmed"
                        },
                        repaired_cache,
                        repeated_repair,
                        termination_confirmed: confirmed,
                    });
                }
                last_phase = Some(phase);
                repaired_cache |= phase == InstallPhase::Repairing;
                repeated_repair |= repeated;
                on_progress(phase, repeated);
            }
            Ok(ReaderEvent::Message(WorkerMessage::Terminal { outcome })) if terminal.is_none() => {
                if outcome == WorkerOutcome::Success && last_phase != Some(InstallPhase::Validating)
                {
                    let confirmed = terminate_or_retain(child);
                    return Err(InstallFailure {
                        code: if confirmed {
                            "protocol_error"
                        } else {
                            "termination_unconfirmed"
                        },
                        repaired_cache,
                        repeated_repair,
                        termination_confirmed: confirmed,
                    });
                }
                terminal = Some(outcome);
            }
            Ok(ReaderEvent::End) => {
                let termination = child.wait_for_exit(Instant::now() + TERMINATION_GRACE);
                let Some(termination) = termination else {
                    retain_unconfirmed_installer(child);
                    return Err(InstallFailure {
                        code: "termination_unconfirmed",
                        repaired_cache,
                        repeated_repair,
                        termination_confirmed: false,
                    });
                };
                let Some(outcome) = terminal else {
                    return Err(InstallFailure {
                        code: "worker_exited_early",
                        repaired_cache,
                        repeated_repair,
                        termination_confirmed: true,
                    });
                };
                if outcome == WorkerOutcome::Success {
                    if termination.exit_code == Some(0) {
                        return Ok(InstallReceipt {
                            repaired_cache,
                            repeated_repair,
                        });
                    }
                    return Err(InstallFailure {
                        code: "protocol_error",
                        repaired_cache,
                        repeated_repair,
                        termination_confirmed: true,
                    });
                }
                return Err(InstallFailure {
                    code: outcome.code(),
                    repaired_cache,
                    repeated_repair,
                    termination_confirmed: true,
                });
            }
            Ok(ReaderEvent::Invalid)
            | Ok(ReaderEvent::Message(_))
            | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let confirmed = terminate_or_retain(child);
                return Err(InstallFailure {
                    code: if confirmed {
                        "protocol_error"
                    } else {
                        "termination_unconfirmed"
                    },
                    repaired_cache,
                    repeated_repair,
                    termination_confirmed: confirmed,
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

pub(crate) fn configured_install_timeout() -> Duration {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("MURMUR_COREML_INSTALL_TIMEOUT_MS") {
        if let Ok(milliseconds) = value.parse::<u64>() {
            return Duration::from_millis(milliseconds.clamp(50, 10 * 60 * 1000));
        }
    }
    DEFAULT_INSTALL_TIMEOUT
}

pub(crate) fn install<F>(
    model_name: &str,
    timeout: Duration,
    on_progress: F,
) -> Result<InstallReceipt, InstallFailure>
where
    F: FnMut(InstallPhase, bool),
{
    let executable = std::env::current_exe().map_err(|_| InstallFailure {
        code: "spawn_failed",
        repaired_cache: false,
        repeated_repair: false,
        termination_confirmed: true,
    })?;
    let arguments = [WORKER_ARGUMENT, model_name];
    #[cfg(debug_assertions)]
    let environment = {
        let mut environment = Vec::new();
        if let Ok(scenario) = std::env::var("MURMUR_COREML_INSTALL_SCENARIO") {
            environment.push(("MURMUR_COREML_INSTALL_SCENARIO".to_string(), scenario));
            static SCENARIO_TOKEN: OnceLock<String> = OnceLock::new();
            environment.push((
                "MURMUR_COREML_INSTALL_SCENARIO_TOKEN".to_string(),
                SCENARIO_TOKEN
                    .get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
                    .clone(),
            ));
        }
        environment
    };
    #[cfg(not(debug_assertions))]
    let environment: Vec<(String, String)> = Vec::new();
    supervise_process(&executable, &arguments, &environment, timeout, on_progress)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn run_shell(
        script: &str,
        timeout: Duration,
    ) -> (
        Result<InstallReceipt, InstallFailure>,
        Vec<(InstallPhase, bool)>,
    ) {
        let mut phases = Vec::new();
        let result = supervise_process(
            Path::new("/bin/sh"),
            &["-c", script],
            &[],
            timeout,
            |phase, repeated| phases.push((phase, repeated)),
        );
        (result, phases)
    }

    fn protocol_script(messages: &[&str], suffix: &str) -> String {
        let framed = messages
            .iter()
            .map(|message| format!("'{PROTOCOL_PREFIX}{message}'"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("printf '%s\\n' {framed}; {suffix}")
    }

    #[test]
    fn successful_worker_requires_terminal_and_clean_exit() {
        let script = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"initializing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"validating","repeatedRepair":false}"#,
                r#"{"type":"terminal","outcome":"success"}"#,
            ],
            "exit 0",
        );
        let (result, phases) = run_shell(&script, Duration::from_secs(1));
        assert_eq!(
            result,
            Ok(InstallReceipt {
                repaired_cache: false,
                repeated_repair: false,
            })
        );
        assert_eq!(
            phases,
            vec![
                (InstallPhase::Preparing, false),
                (InstallPhase::Initializing, false),
                (InstallPhase::Validating, false),
            ]
        );
    }

    #[test]
    fn explicit_failure_preserves_repeated_repair_evidence() {
        let script = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"repairing","repeatedRepair":true}"#,
                r#"{"type":"terminal","outcome":"native_initialization_failed"}"#,
            ],
            "exit 20",
        );
        let (result, _) = run_shell(&script, Duration::from_secs(1));
        assert_eq!(
            result,
            Err(InstallFailure {
                code: "native_initialization_failed",
                repaired_cache: true,
                repeated_repair: true,
                termination_confirmed: true,
            })
        );
    }

    #[test]
    fn hanging_worker_and_descendant_are_killed_before_timeout_returns() {
        let script = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"initializing","repeatedRepair":false}"#,
            ],
            "sleep 30 & wait",
        );
        let started = Instant::now();
        let (result, phases) = run_shell(&script, Duration::from_millis(75));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "installer_timeout");
        assert!(failure.termination_confirmed);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            phases,
            vec![
                (InstallPhase::Preparing, false),
                (InstallPhase::Initializing, false),
            ]
        );
    }

    #[test]
    fn malformed_protocol_fails_closed_and_reaps_worker() {
        let script = format!("printf '{PROTOCOL_PREFIX}not-json\\n'; sleep 30");
        let (result, _) = run_shell(&script, Duration::from_secs(1));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "protocol_error");
        assert!(failure.termination_confirmed);
    }

    #[test]
    fn bounded_native_stdout_noise_is_ignored() {
        let protocol = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"initializing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"validating","repeatedRepair":false}"#,
                r#"{"type":"terminal","outcome":"success"}"#,
            ],
            "exit 0",
        );
        let script = format!("printf 'ASR init notice\\n'; {protocol}");
        let (result, _) = run_shell(&script, Duration::from_secs(1));
        assert!(result.is_ok());
    }

    #[test]
    fn oversized_protocol_line_is_rejected_before_unbounded_allocation() {
        let script = "/usr/bin/yes x | /usr/bin/head -c 5000; printf '\\n'; /bin/sleep 30";
        let (result, _) = run_shell(script, Duration::from_secs(1));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "protocol_error");
        assert!(failure.termination_confirmed);
    }

    #[test]
    fn terminal_success_with_nonzero_exit_is_not_accepted() {
        let script = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"initializing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"validating","repeatedRepair":false}"#,
                r#"{"type":"terminal","outcome":"success"}"#,
            ],
            "exit 9",
        );
        let (result, _) = run_shell(&script, Duration::from_secs(1));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "protocol_error");
        assert!(failure.termination_confirmed);
    }

    #[test]
    fn out_of_order_or_duplicate_phases_fail_closed() {
        for script in [
            protocol_script(
                &[r#"{"type":"phase","phase":"validating","repeatedRepair":false}"#],
                "sleep 30",
            ),
            protocol_script(
                &[
                    r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                    r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                ],
                "sleep 30",
            ),
            protocol_script(
                &[r#"{"type":"phase","phase":"preparing","repeatedRepair":true}"#],
                "sleep 30",
            ),
        ] {
            let (result, _) = run_shell(&script, Duration::from_secs(1));
            let failure = result.unwrap_err();
            assert_eq!(failure.code, "protocol_error");
            assert!(failure.termination_confirmed);
        }
    }

    #[test]
    fn quarantined_owner_is_confirmed_gone_before_a_new_worker_spawns() {
        let (child, stdin, stdout) = ManagedChild::spawn(Path::new("/usr/bin/yes"), &[]).unwrap();
        drop((stdin, stdout));
        retain_unconfirmed_installer(child);

        let script = protocol_script(
            &[
                r#"{"type":"phase","phase":"preparing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"initializing","repeatedRepair":false}"#,
                r#"{"type":"phase","phase":"validating","repeatedRepair":false}"#,
                r#"{"type":"terminal","outcome":"success"}"#,
            ],
            "exit 0",
        );
        let (result, _) = run_shell(&script, Duration::from_secs(1));

        assert!(result.is_ok());
        assert!(UNCONFIRMED_INSTALLER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
    }
}
