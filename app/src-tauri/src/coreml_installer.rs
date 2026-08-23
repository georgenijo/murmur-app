//! Killable Core ML model installation boundary.
//!
//! FluidAudio exposes model setup as one synchronous native call. The Tauri
//! process therefore launches its own already-signed executable in this fixed
//! worker mode, owns the exact child process group, and accepts only a small
//! content-free line protocol. A hard deadline always ends with confirmed
//! process-group termination before retry or fallback can begin.

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
use std::time::{Duration, Instant};

const WORKER_ARGUMENT: &str = "--coreml-install-worker-v1";
const MAX_PROTOCOL_LINE_BYTES: usize = 4 * 1024;
const MAX_PROTOCOL_MESSAGES: usize = 32;
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
pub(crate) const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
                "Core ML setup stopped, but Murmur could not confirm installer cleanup. Restart Murmur before retrying."
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

fn emit_worker_message(message: &WorkerMessage) -> Result<(), ()> {
    let encoded = serde_json::to_string(message).map_err(|_| ())?;
    if encoded.len() > MAX_PROTOCOL_LINE_BYTES {
        return Err(());
    }
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "{encoded}").map_err(|_| ())?;
    output.flush().map_err(|_| ())
}

fn start_parent_watchdog() {
    std::thread::spawn(|| {
        let mut input = std::io::stdin();
        let mut byte = [0_u8; 1];
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => std::process::exit(74),
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
            let _ = phase(InstallPhase::Initializing, true);
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
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(ReaderEvent::End);
                    return;
                }
                Ok(_) => {
                    messages += 1;
                    if messages > MAX_PROTOCOL_MESSAGES
                        || line.len() > MAX_PROTOCOL_LINE_BYTES
                        || !line.ends_with('\n')
                    {
                        let _ = sender.send(ReaderEvent::Invalid);
                        return;
                    }
                    match serde_json::from_str::<WorkerMessage>(line.trim_end()) {
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
                Err(_) => {
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
    loop {
        let now = Instant::now();
        if now >= deadline {
            let confirmed = child
                .hard_kill_confirmed(Instant::now() + TERMINATION_GRACE)
                .is_some();
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
                repaired_cache |= phase == InstallPhase::Repairing;
                repeated_repair |= repeated;
                on_progress(phase, repeated);
            }
            Ok(ReaderEvent::Message(WorkerMessage::Terminal { outcome })) if terminal.is_none() => {
                terminal = Some(outcome);
            }
            Ok(ReaderEvent::End) => {
                let termination = child.wait_for_exit(Instant::now() + TERMINATION_GRACE);
                let Some(termination) = termination else {
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
                let confirmed = child
                    .hard_kill_confirmed(Instant::now() + TERMINATION_GRACE)
                    .is_some();
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
    let mut environment = Vec::new();
    #[cfg(debug_assertions)]
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

    #[test]
    fn successful_worker_requires_terminal_and_clean_exit() {
        let script = r#"printf '%s\n' '{"type":"phase","phase":"initializing","repeatedRepair":false}' '{"type":"phase","phase":"validating","repeatedRepair":false}' '{"type":"terminal","outcome":"success"}'"#;
        let (result, phases) = run_shell(script, Duration::from_secs(1));
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
                (InstallPhase::Initializing, false),
                (InstallPhase::Validating, false),
            ]
        );
    }

    #[test]
    fn explicit_failure_preserves_repeated_repair_evidence() {
        let script = r#"printf '%s\n' '{"type":"phase","phase":"repairing","repeatedRepair":true}' '{"type":"terminal","outcome":"native_initialization_failed"}'; exit 20"#;
        let (result, _) = run_shell(script, Duration::from_secs(1));
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
        let script = r#"printf '%s\n' '{"type":"phase","phase":"initializing","repeatedRepair":false}'; sleep 30 & wait"#;
        let started = Instant::now();
        let (result, phases) = run_shell(script, Duration::from_millis(75));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "installer_timeout");
        assert!(failure.termination_confirmed);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(phases, vec![(InstallPhase::Initializing, false)]);
    }

    #[test]
    fn malformed_protocol_fails_closed_and_reaps_worker() {
        let (result, _) = run_shell("printf 'not-json\\n'; sleep 30", Duration::from_secs(1));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "protocol_error");
        assert!(failure.termination_confirmed);
    }

    #[test]
    fn terminal_success_with_nonzero_exit_is_not_accepted() {
        let script = r#"printf '%s\n' '{"type":"terminal","outcome":"success"}'; exit 9"#;
        let (result, _) = run_shell(script, Duration::from_secs(1));
        let failure = result.unwrap_err();
        assert_eq!(failure.code, "protocol_error");
        assert!(failure.termination_confirmed);
    }
}
