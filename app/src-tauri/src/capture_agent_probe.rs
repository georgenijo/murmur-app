//! Opt-in LaunchAgent recovery spike for issue #407.
//!
//! The production dictation path does not use this module. Signed/notarized
//! builds expose a narrow CLI so LaunchServices can register the experimental
//! per-user agent, hold a microphone lease, and claim content-free recovery
//! evidence after macOS restarts the main application.

use crate::managed_child::{bundled_sibling, ManagedChild};
use serde_json::{Map, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const CAPTURE_AGENT_NAME: &str = "murmur-capture-agent";
const CAPTURE_AGENT_IDENTIFIER: &str = "com.localdictation.capture-agent";
const DEFAULT_OBSERVE: Duration = Duration::from_secs(5);
const MAX_OBSERVE_SECONDS: u64 = 300;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const SERVICE_TRANSITION_TIMEOUT: Duration = Duration::from_secs(15);
const SERVICE_PROCESS_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceCommand {
    Register,
    Refresh,
    Status,
    Unregister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliRequest {
    NotRequested,
    Service(ServiceCommand),
    Probe(Duration),
    SyntheticProbe(Duration),
    SyntheticFaultProbe(Duration),
    Recover,
    RecoverReplayAck,
    Status,
    Invalid,
}

fn parse_cli_request(arguments: impl IntoIterator<Item = String>) -> CliRequest {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => CliRequest::NotRequested,
        [flag, command] if flag == "--capture-agent-service" => match command.as_str() {
            "register" => CliRequest::Service(ServiceCommand::Register),
            "refresh" => CliRequest::Service(ServiceCommand::Refresh),
            "status" => CliRequest::Service(ServiceCommand::Status),
            "unregister" => CliRequest::Service(ServiceCommand::Unregister),
            _ => CliRequest::Invalid,
        },
        [flag] if flag == "--capture-agent-probe" => CliRequest::Probe(DEFAULT_OBSERVE),
        [flag] if flag == "--capture-agent-synthetic-probe" => {
            CliRequest::SyntheticProbe(DEFAULT_OBSERVE)
        }
        [flag] if flag == "--capture-agent-synthetic-fault-probe" => {
            CliRequest::SyntheticFaultProbe(DEFAULT_OBSERVE)
        }
        [flag, duration_flag, seconds]
            if flag == "--capture-agent-probe" && duration_flag == "--observe-seconds" =>
        {
            match seconds.parse::<u64>() {
                Ok(seconds @ 1..=MAX_OBSERVE_SECONDS) => {
                    CliRequest::Probe(Duration::from_secs(seconds))
                }
                _ => CliRequest::Invalid,
            }
        }
        [flag, duration_flag, seconds]
            if flag == "--capture-agent-synthetic-probe"
                && duration_flag == "--observe-seconds" =>
        {
            match seconds.parse::<u64>() {
                Ok(seconds @ 1..=MAX_OBSERVE_SECONDS) => {
                    CliRequest::SyntheticProbe(Duration::from_secs(seconds))
                }
                _ => CliRequest::Invalid,
            }
        }
        [flag, duration_flag, seconds]
            if flag == "--capture-agent-synthetic-fault-probe"
                && duration_flag == "--observe-seconds" =>
        {
            match seconds.parse::<u64>() {
                Ok(seconds @ 1..=MAX_OBSERVE_SECONDS) => {
                    CliRequest::SyntheticFaultProbe(Duration::from_secs(seconds))
                }
                _ => CliRequest::Invalid,
            }
        }
        [flag] if flag == "--capture-agent-recover" => CliRequest::Recover,
        [flag] if flag == "--capture-agent-recover-replay-ack" => CliRequest::RecoverReplayAck,
        [flag] if flag == "--capture-agent-status" => CliRequest::Status,
        [first, ..] if first.starts_with("--capture-agent-") => CliRequest::Invalid,
        _ => CliRequest::NotRequested,
    }
}

fn content_free_error(outcome: &str) -> Value {
    serde_json::json!({
        "schema_version": 1,
        "outcome": outcome,
        "audio_content_retained": false
    })
}

#[cfg(target_os = "macos")]
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

#[cfg(target_os = "macos")]
fn service_status_name(status: isize) -> &'static str {
    match status {
        0 => "not_registered",
        1 => "enabled",
        2 => "requires_approval",
        3 => "not_found",
        _ => "unknown",
    }
}

fn should_register_service(command: ServiceCommand, status: isize) -> bool {
    status == 0 || (status == 3 && command == ServiceCommand::Register)
}

#[cfg(target_os = "macos")]
fn service_response(outcome: &str, status: isize, exit_code: i32) -> (Value, i32) {
    (
        serde_json::json!({
            "schema_version": 1,
            "outcome": outcome,
            "service_status": service_status_name(status),
            "audio_content_retained": false
        }),
        exit_code,
    )
}

#[cfg(target_os = "macos")]
fn registered_service_pids() -> Result<Vec<u64>, ()> {
    let (status, exit_code) = run_one_shot(&["status"], &["idle", "active", "pending"]);
    if exit_code != 0 {
        return Err(());
    }
    let agent_pid = status
        .get("agent_pid")
        .and_then(Value::as_u64)
        .filter(|pid| (2..=i32::MAX as u64).contains(pid))
        .ok_or(())?;
    let mut pids = vec![agent_pid];
    pids.extend(
        ["worker_pid"]
            .into_iter()
            .filter_map(|key| status.get(key).and_then(Value::as_u64))
            .filter(|pid| (2..=i32::MAX as u64).contains(pid)),
    );
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn wait_for_processes_to_exit(pids: &[u64], timeout: Duration) -> bool {
    if pids.is_empty() {
        return true;
    }
    let deadline = Instant::now() + timeout;
    loop {
        let any_alive = pids.iter().any(|pid| {
            let result = unsafe { libc::kill(*pid as i32, 0) };
            result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        });
        if !any_alive {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
fn run_service_command(command: ServiceCommand) -> (Value, i32) {
    use block2::RcBlock;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    unsafe {
        let Some(mtm) = MainThreadMarker::new() else {
            return (content_free_error("service_unavailable"), 2);
        };
        let application = NSApplication::sharedApplication(mtm);
        application.finishLaunching();

        let Some(class) = AnyClass::get(c"SMAppService") else {
            return (content_free_error("service_unavailable"), 2);
        };
        let plist_name = NSString::from_str("com.localdictation.capture-agent.plist");
        let service: *mut AnyObject = msg_send![class, agentServiceWithPlistName: &*plist_name];
        if service.is_null() {
            return (content_free_error("service_unavailable"), 2);
        }

        let mut status: isize = msg_send![service, status];
        if !matches!(status, 0..=3) {
            return service_response("service_error", status, 2);
        }
        if command == ServiceCommand::Status {
            return service_response(
                if status == 3 {
                    "service_error"
                } else {
                    "service_status"
                },
                status,
                if status == 3 { 2 } else { 0 },
            );
        }

        let observed_pids = if matches!(
            command,
            ServiceCommand::Refresh | ServiceCommand::Unregister
        ) && status == 1
        {
            match registered_service_pids() {
                Ok(pids) => pids,
                Err(()) => return service_response("service_error", status, 2),
            }
        } else {
            Vec::new()
        };

        if matches!(
            command,
            ServiceCommand::Refresh | ServiceCommand::Unregister
        ) && matches!(status, 1 | 2)
        {
            let (sender, receiver) = mpsc::sync_channel(1);
            let completion = RcBlock::new(move |error: *mut AnyObject| {
                let _ = sender.send(error.is_null());
            });
            let _: () = msg_send![service, unregisterWithCompletionHandler: &*completion];
            if receiver.recv_timeout(SERVICE_TRANSITION_TIMEOUT) != Ok(true) {
                status = msg_send![service, status];
                return service_response("service_error", status, 2);
            }
            let status_deadline = Instant::now() + SERVICE_PROCESS_EXIT_TIMEOUT;
            loop {
                status = msg_send![service, status];
                if status == 0 || Instant::now() >= status_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            if status != 0
                || !wait_for_processes_to_exit(&observed_pids, SERVICE_PROCESS_EXIT_TIMEOUT)
            {
                return service_response("service_error", status, 2);
            }
        }

        if command == ServiceCommand::Unregister {
            return service_response(
                if status == 0 {
                    "service_status"
                } else {
                    "service_error"
                },
                status,
                if status == 0 { 0 } else { 2 },
            );
        }

        if should_register_service(command, status) {
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let succeeded: Bool = msg_send![service, registerAndReturnError: &mut error];
            if !succeeded.as_bool() {
                status = msg_send![service, status];
                return service_response("service_error", status, 2);
            }
            let status_deadline = Instant::now() + SERVICE_PROCESS_EXIT_TIMEOUT;
            loop {
                status = msg_send![service, status];
                if matches!(status, 1 | 2) || Instant::now() >= status_deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }

        service_response(
            if status == 1 {
                "service_status"
            } else {
                "service_error"
            },
            status,
            if status == 1 { 0 } else { 2 },
        )
    }
}

#[cfg(not(target_os = "macos"))]
fn run_service_command(_command: ServiceCommand) -> (Value, i32) {
    (content_free_error("service_unavailable"), 2)
}

fn allowed_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "outcome"
            | "service_status"
            | "agent_instance"
            | "worker_termination"
            | "failure"
            | "generation"
            | "agent_pid"
            | "worker_pid"
            | "synthetic_canary_count"
            | "first_callback_ms"
            | "stop_elapsed_ms"
            | "recovery_ttl_ms"
            | "audio_content_retained"
            | "recovered"
            | "agent_survived"
            | "worker_exited"
            | "process_group_empty"
            | "exit_signal"
            | "exact_once"
            | "claim_id"
            | "synthetic_fixture"
            | "synthetic_digest"
            | "synthetic_first_sequence"
            | "synthetic_last_sequence"
            | "synthetic_complete"
    )
}

fn parse_allowlisted_json(line: &str) -> Result<Map<String, Value>, ()> {
    if line.len() > 8_192 || line.contains('\0') {
        return Err(());
    }
    let value: Value = serde_json::from_str(line).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    if object.keys().any(|key| !allowed_key(key))
        || object.get("schema_version").and_then(Value::as_u64) != Some(1)
        || object
            .get("audio_content_retained")
            .and_then(Value::as_bool)
            != Some(false)
        || object.get("outcome").and_then(Value::as_str).is_none()
    {
        return Err(());
    }
    Ok(object.clone())
}

fn has_exact_keys(object: &Map<String, Value>, keys: &[&str]) -> bool {
    object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
}

fn valid_common(object: &Map<String, Value>, outcome: &str) -> bool {
    object.get("schema_version").and_then(Value::as_u64) == Some(1)
        && object.get("outcome").and_then(Value::as_str) == Some(outcome)
        && object
            .get("audio_content_retained")
            .and_then(Value::as_bool)
            == Some(false)
}

fn nonempty_bounded_string(object: &Map<String, Value>, key: &str) -> bool {
    object
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty() && value.len() <= 128)
}

fn valid_worker_termination(object: &Map<String, Value>) -> bool {
    match (
        object.get("worker_termination").and_then(Value::as_str),
        object.get("exit_signal").and_then(Value::as_u64),
    ) {
        (Some("cooperative" | "exited"), Some(0)) => true,
        (Some("hard_kill"), Some(signal)) if signal == libc::SIGKILL as u64 => true,
        _ => false,
    }
}

fn valid_status_response(object: &Map<String, Value>) -> bool {
    let Some(outcome @ ("idle" | "active" | "pending")) =
        object.get("outcome").and_then(Value::as_str)
    else {
        return false;
    };
    has_exact_keys(
        object,
        &[
            "schema_version",
            "outcome",
            "agent_pid",
            "agent_instance",
            "generation",
            "worker_pid",
            "synthetic_canary_count",
            "audio_content_retained",
        ],
    ) && valid_common(object, outcome)
        && object
            .get("agent_pid")
            .and_then(Value::as_u64)
            .is_some_and(|pid| (2..=i32::MAX as u64).contains(&pid))
        && nonempty_bounded_string(object, "agent_instance")
        && object.get("generation").and_then(Value::as_u64).is_some()
        && object.get("worker_pid").and_then(Value::as_u64).is_some()
        && object
            .get("synthetic_canary_count")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 4_096)
}

fn valid_ready_response(object: &Map<String, Value>) -> bool {
    has_exact_keys(
        object,
        &[
            "schema_version",
            "outcome",
            "generation",
            "agent_pid",
            "agent_instance",
            "worker_pid",
            "synthetic_canary_count",
            "first_callback_ms",
            "audio_content_retained",
        ],
    ) && valid_common(object, "ready")
        && nonempty_bounded_string(object, "agent_instance")
        && ["generation", "agent_pid", "worker_pid"].iter().all(|key| {
            object
                .get(*key)
                .and_then(Value::as_u64)
                .is_some_and(|value| value > 0)
        })
        && object
            .get("synthetic_canary_count")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 4_096)
        && object
            .get("first_callback_ms")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 60_000)
}

fn valid_stopped_response(object: &Map<String, Value>) -> bool {
    let mut keys = vec![
        "schema_version",
        "outcome",
        "generation",
        "synthetic_canary_count",
        "worker_termination",
        "stop_elapsed_ms",
        "worker_exited",
        "process_group_empty",
        "exit_signal",
        "audio_content_retained",
    ];
    let has_synthetic = object.contains_key("synthetic_fixture");
    if has_synthetic {
        keys.extend([
            "synthetic_fixture",
            "synthetic_digest",
            "synthetic_first_sequence",
            "synthetic_last_sequence",
            "synthetic_complete",
        ]);
    }
    has_exact_keys(object, &keys)
        && valid_common(object, "stopped")
        && object.get("generation").and_then(Value::as_u64).is_some()
        && object
            .get("synthetic_canary_count")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 4_096)
        && valid_worker_termination(object)
        && object
            .get("stop_elapsed_ms")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 5_000)
        && object.get("worker_exited").and_then(Value::as_bool) == Some(true)
        && object.get("process_group_empty").and_then(Value::as_bool) == Some(true)
        && (!has_synthetic
            || (object.get("synthetic_fixture").and_then(Value::as_str) == Some("seq-v1")
                && object.get("synthetic_digest").and_then(Value::as_str)
                    == Some("9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3")
                && object
                    .get("synthetic_first_sequence")
                    .and_then(Value::as_u64)
                    == Some(0)
                && object
                    .get("synthetic_last_sequence")
                    .and_then(Value::as_u64)
                    == Some(63)
                && object.get("synthetic_complete").and_then(Value::as_bool) == Some(true)
                && object.get("synthetic_canary_count").and_then(Value::as_u64) == Some(64)))
}

fn valid_recovery_payload(object: &Map<String, Value>, outcome: &str) -> bool {
    let mut keys = vec![
        "schema_version",
        "outcome",
        "generation",
        "agent_pid",
        "agent_instance",
        "worker_pid",
        "synthetic_canary_count",
        "first_callback_ms",
        "worker_termination",
        "stop_elapsed_ms",
        "recovery_ttl_ms",
        "agent_survived",
        "worker_exited",
        "process_group_empty",
        "exit_signal",
        "audio_content_retained",
        "claim_id",
        "recovered",
        "exact_once",
    ];
    let has_synthetic = object.contains_key("synthetic_fixture");
    if has_synthetic {
        keys.extend([
            "synthetic_fixture",
            "synthetic_digest",
            "synthetic_first_sequence",
            "synthetic_last_sequence",
            "synthetic_complete",
        ]);
    }
    has_exact_keys(object, &keys)
        && valid_common(object, outcome)
        && ["generation", "agent_pid", "worker_pid"].iter().all(|key| {
            object
                .get(*key)
                .and_then(Value::as_u64)
                .is_some_and(|value| value > 0)
        })
        && nonempty_bounded_string(object, "agent_instance")
        && nonempty_bounded_string(object, "claim_id")
        && object
            .get("synthetic_canary_count")
            .and_then(Value::as_u64)
            .is_some_and(|value| (1..=4_096).contains(&value))
        && object
            .get("first_callback_ms")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 60_000)
        && valid_worker_termination(object)
        && object
            .get("stop_elapsed_ms")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 5_000)
        && object
            .get("recovery_ttl_ms")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= 30_000)
        && object.get("agent_survived").and_then(Value::as_bool) == Some(true)
        && object.get("worker_exited").and_then(Value::as_bool) == Some(true)
        && object.get("process_group_empty").and_then(Value::as_bool) == Some(true)
        && object.get("recovered").and_then(Value::as_bool) == Some(outcome == "recovery_acked")
        && object.get("exact_once").and_then(Value::as_bool) == Some(outcome == "recovery_acked")
        && (!has_synthetic
            || (object.get("synthetic_fixture").and_then(Value::as_str) == Some("seq-v1")
                && object.get("synthetic_digest").and_then(Value::as_str)
                    == Some("9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3")
                && object
                    .get("synthetic_first_sequence")
                    .and_then(Value::as_u64)
                    == Some(0)
                && object
                    .get("synthetic_last_sequence")
                    .and_then(Value::as_u64)
                    == Some(63)
                && object.get("synthetic_complete").and_then(Value::as_bool) == Some(true)
                && object.get("synthetic_canary_count").and_then(Value::as_u64) == Some(64)))
}

fn valid_terminal_recovery_response(object: &Map<String, Value>) -> bool {
    match object.get("outcome").and_then(Value::as_str) {
        Some("none" | "settling" | "claim_busy") => {
            has_exact_keys(
                object,
                &[
                    "schema_version",
                    "outcome",
                    "recovered",
                    "audio_content_retained",
                ],
            ) && object.get("recovered").and_then(Value::as_bool) == Some(false)
        }
        Some("expired") => {
            has_exact_keys(
                object,
                &[
                    "schema_version",
                    "outcome",
                    "generation",
                    "recovered",
                    "audio_content_retained",
                ],
            ) && object.get("generation").and_then(Value::as_u64).is_some()
                && object.get("recovered").and_then(Value::as_bool) == Some(false)
        }
        Some("already_acked") => {
            has_exact_keys(
                object,
                &[
                    "schema_version",
                    "outcome",
                    "generation",
                    "recovered",
                    "exact_once",
                    "audio_content_retained",
                ],
            ) && object.get("generation").and_then(Value::as_u64).is_some()
                && object.get("recovered").and_then(Value::as_bool) == Some(false)
                && object.get("exact_once").and_then(Value::as_bool) == Some(true)
        }
        Some("recovery_offer") => valid_recovery_payload(object, "recovery_offer"),
        Some("recovery_acked") => valid_recovery_payload(object, "recovery_acked"),
        _ => false,
    }
}

fn agent_path() -> Result<std::path::PathBuf, ()> {
    let path = bundled_sibling(CAPTURE_AGENT_NAME)?;
    if !cfg!(debug_assertions) {
        crate::code_signing::validate_bundled_helper(&path, CAPTURE_AGENT_IDENTIFIER)?;
    }
    Ok(path)
}

struct AgentProcess {
    child: ManagedChild,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<String>,
}

impl AgentProcess {
    fn spawn(arguments: &[&str]) -> Result<Self, ()> {
        let path = agent_path()?;
        let (child, stdin, stdout) =
            ManagedChild::spawn_with_arguments(&path, arguments, &[]).map_err(|_| ())?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                let read = match (&mut reader).take(8_193).read_line(&mut line) {
                    Ok(read) => read,
                    Err(_) => break,
                };
                if read == 0 {
                    break;
                }
                if read > 8_192 || !line.ends_with('\n') {
                    let _ = sender.send(String::new());
                    break;
                }
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
        })
    }

    fn receive(&self, timeout: Duration) -> Result<Map<String, Value>, ()> {
        let line = self.lines.recv_timeout(timeout).map_err(|_| ())?;
        parse_allowlisted_json(&line)
    }

    fn finish(mut self) -> i32 {
        drop(self.stdin);
        let deadline = Instant::now() + EXIT_TIMEOUT;
        let termination = self.child.wait_for_exit(deadline).or_else(|| {
            self.child
                .hard_kill_confirmed(Instant::now() + EXIT_TIMEOUT)
        });
        termination.and_then(|value| value.exit_code).unwrap_or(2)
    }
}

fn run_one_shot(arguments: &[&str], allowed_outcomes: &[&str]) -> (Value, i32) {
    let process = match AgentProcess::spawn(arguments) {
        Ok(process) => process,
        Err(()) => return (content_free_error("agent_spawn_failed"), 2),
    };
    let response = match process.receive(RESPONSE_TIMEOUT) {
        Ok(response)
            if response
                .get("outcome")
                .and_then(Value::as_str)
                .is_some_and(|outcome| allowed_outcomes.contains(&outcome))
                && valid_status_response(&response) =>
        {
            Value::Object(response)
        }
        _ => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    };
    let exit_code = process.finish();
    (response, exit_code)
}

fn run_probe(observe_for: Duration, synthetic: bool, fault: bool) -> (Value, i32) {
    if !synthetic && crate::commands::permissions::check_microphone_permission_status() != "granted"
    {
        return (content_free_error("permission_denied"), 2);
    }
    let arguments = if fault {
        &["lease-synthetic-fault"][..]
    } else if synthetic {
        &["lease-synthetic"][..]
    } else {
        &["lease"][..]
    };
    let mut process = match AgentProcess::spawn(arguments) {
        Ok(process) => process,
        Err(()) => return (content_free_error("agent_spawn_failed"), 2),
    };
    let ready = match process.receive(RESPONSE_TIMEOUT) {
        Ok(response) => response,
        Err(()) => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    };
    if !valid_ready_response(&ready) {
        let exit_code = process.finish();
        return (Value::Object(ready), exit_code.max(2));
    }

    let deadline = Instant::now() + observe_for;
    while Instant::now() < deadline {
        if !synthetic
            && crate::commands::permissions::check_microphone_permission_status() != "granted"
        {
            // Closing the lease client connection is the interruption signal.
            let _ = process.finish();
            return (content_free_error("permission_revoked"), 2);
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    if process.stdin.write_all(b"stop\n").is_err() || process.stdin.flush().is_err() {
        let _ = process.finish();
        return (content_free_error("agent_stop_failed"), 2);
    }
    let stopped = match process.receive(RESPONSE_TIMEOUT) {
        Ok(response) => response,
        Err(()) => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    };
    let exit_code = process.finish();
    if !valid_stopped_response(&stopped) {
        return (Value::Object(stopped), exit_code.max(2));
    }

    let mut combined = ready;
    combined.insert("outcome".to_string(), Value::String("ok".to_string()));
    for (key, value) in stopped {
        if key != "outcome" {
            combined.insert(key, value);
        }
    }
    (Value::Object(combined), exit_code)
}

fn immutable_recovery_payload(object: &Map<String, Value>) -> Map<String, Value> {
    let mut payload = object.clone();
    for key in ["outcome", "recovered", "exact_once", "recovery_ttl_ms"] {
        payload.remove(key);
    }
    payload
}

fn run_recover(replay_acknowledgement: bool) -> (Value, i32) {
    let arguments = if replay_acknowledgement {
        ["recover-replay-ack"].as_slice()
    } else {
        ["recover"].as_slice()
    };
    let mut process = match AgentProcess::spawn(arguments) {
        Ok(process) => process,
        Err(()) => return (content_free_error("agent_spawn_failed"), 2),
    };
    let offer = match process.receive(RESPONSE_TIMEOUT) {
        Ok(response) if valid_terminal_recovery_response(&response) => response,
        Ok(_) | Err(()) => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    };
    match offer.get("outcome").and_then(Value::as_str) {
        Some("none" | "settling" | "expired" | "already_acked") => {
            let exit_code = process.finish();
            return (Value::Object(offer), exit_code);
        }
        Some("recovery_acked") => {
            let exit_code = process.finish();
            return (Value::Object(offer), exit_code);
        }
        Some("recovery_offer") => {}
        _ => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    }
    let Some(claim_id) = offer
        .get("claim_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
    else {
        let _ = process.finish();
        return (content_free_error("agent_protocol_failed"), 2);
    };
    if writeln!(process.stdin, "ack:{claim_id}").is_err() || process.stdin.flush().is_err() {
        let _ = process.finish();
        return (content_free_error("agent_protocol_failed"), 2);
    }
    let mut acknowledged = match process.receive(RESPONSE_TIMEOUT) {
        Ok(response)
            if valid_recovery_payload(&response, "recovery_acked")
                && response.get("claim_id").and_then(Value::as_str) == Some(claim_id)
                && immutable_recovery_payload(&response) == immutable_recovery_payload(&offer) =>
        {
            response
        }
        _ => {
            let _ = process.finish();
            return (content_free_error("agent_protocol_failed"), 2);
        }
    };
    let exit_code = process.finish();
    if replay_acknowledgement {
        acknowledged.insert("ack_replay_verified".to_string(), Value::Bool(true));
    }
    (Value::Object(acknowledged), exit_code)
}

pub fn run_cli_if_requested() -> Option<i32> {
    let (response, exit_code) = match parse_cli_request(std::env::args().skip(1)) {
        CliRequest::NotRequested => return None,
        CliRequest::Invalid => (content_free_error("invalid_arguments"), 64),
        CliRequest::Service(command) => run_service_command(command),
        CliRequest::Recover => run_recover(false),
        CliRequest::RecoverReplayAck => run_recover(true),
        CliRequest::Status => run_one_shot(&["status"], &["idle", "active", "pending"]),
        CliRequest::Probe(observe_for) => run_probe(observe_for, false, false),
        CliRequest::SyntheticProbe(observe_for) => run_probe(observe_for, true, false),
        CliRequest::SyntheticFaultProbe(observe_for) => run_probe(observe_for, true, true),
    };
    println!("{response}");
    Some(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_is_explicit_strict_and_bounded() {
        assert_eq!(
            parse_cli_request(["--capture-agent-probe".to_string()]),
            CliRequest::Probe(DEFAULT_OBSERVE)
        );
        assert_eq!(
            parse_cli_request([
                "--capture-agent-probe".to_string(),
                "--observe-seconds".to_string(),
                "120".to_string(),
            ]),
            CliRequest::Probe(Duration::from_secs(120))
        );
        assert_eq!(
            parse_cli_request([
                "--capture-agent-service".to_string(),
                "register".to_string(),
            ]),
            CliRequest::Service(ServiceCommand::Register)
        );
        assert_eq!(
            parse_cli_request(["--capture-agent-service".to_string(), "refresh".to_string(),]),
            CliRequest::Service(ServiceCommand::Refresh)
        );
        assert_eq!(
            parse_cli_request(["--capture-agent-recover".to_string()]),
            CliRequest::Recover
        );
        assert_eq!(
            parse_cli_request(["--capture-agent-recover-replay-ack".to_string()]),
            CliRequest::RecoverReplayAck
        );
        assert_eq!(
            parse_cli_request(["--capture-agent-synthetic-fault-probe".to_string()]),
            CliRequest::SyntheticFaultProbe(DEFAULT_OBSERVE)
        );
        for arguments in [
            vec![
                "--capture-agent-probe".to_string(),
                "--observe-seconds".to_string(),
                "0".to_string(),
            ],
            vec![
                "--capture-agent-probe".to_string(),
                "--observe-seconds".to_string(),
                "301".to_string(),
            ],
            vec!["--capture-agent-service".to_string(), "enable".to_string()],
            vec!["--capture-agent-status".to_string(), "extra".to_string()],
        ] {
            assert_eq!(parse_cli_request(arguments), CliRequest::Invalid);
        }
        assert_eq!(
            parse_cli_request(["--unrelated".to_string()]),
            CliRequest::NotRequested
        );
    }

    #[test]
    fn only_explicit_register_may_recover_from_not_found() {
        assert!(should_register_service(ServiceCommand::Register, 0));
        assert!(should_register_service(ServiceCommand::Register, 3));
        assert!(should_register_service(ServiceCommand::Refresh, 0));
        assert!(!should_register_service(ServiceCommand::Refresh, 3));
        assert!(!should_register_service(ServiceCommand::Unregister, 3));
        assert!(!should_register_service(ServiceCommand::Status, 3));
    }

    #[test]
    fn evidence_allowlist_rejects_content_and_unknown_fields() {
        assert!(parse_allowlisted_json(
            r#"{"schema_version":1,"outcome":"idle","audio_content_retained":false,"agent_pid":42}"#
        )
        .is_ok());
        for rejected in [
            r#"{"schema_version":1,"outcome":"idle","audio_content_retained":true}"#,
            r#"{"schema_version":1,"outcome":"idle","audio_content_retained":false,"pcm":[1]}"#,
            r#"{"schema_version":1,"outcome":"idle","audio_content_retained":false,"path":"/tmp/x"}"#,
        ] {
            assert!(parse_allowlisted_json(rejected).is_err());
        }
    }

    #[test]
    fn response_contracts_are_exact_and_cross_field_checked() {
        let ready = serde_json::json!({
            "schema_version": 1,
            "outcome": "ready",
            "generation": 7,
            "agent_pid": 101,
            "agent_instance": "boot",
            "worker_pid": 102,
            "synthetic_canary_count": 64,
            "first_callback_ms": 0,
            "audio_content_retained": false
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(valid_ready_response(&ready));
        let mut mutated = ready.clone();
        mutated.insert("pcm".into(), serde_json::json!([1]));
        assert!(!valid_ready_response(&mutated));

        let stopped = serde_json::json!({
            "schema_version": 1,
            "outcome": "stopped",
            "generation": 7,
            "synthetic_canary_count": 64,
            "worker_termination": "hard_kill",
            "stop_elapsed_ms": 260,
            "worker_exited": true,
            "process_group_empty": true,
            "exit_signal": 9,
            "audio_content_retained": false
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(valid_stopped_response(&stopped));
        let mut contradictory = stopped.clone();
        contradictory.insert("exit_signal".into(), Value::from(0));
        assert!(!valid_stopped_response(&contradictory));
        let mut unconfirmed = stopped;
        unconfirmed.insert("process_group_empty".into(), Value::Bool(false));
        assert!(!valid_stopped_response(&unconfirmed));

        let offer = serde_json::json!({
            "schema_version": 1,
            "outcome": "recovery_offer",
            "generation": 7,
            "agent_pid": 101,
            "agent_instance": "boot",
            "worker_pid": 102,
            "synthetic_canary_count": 64,
            "first_callback_ms": 0,
            "worker_termination": "hard_kill",
            "stop_elapsed_ms": 260,
            "recovery_ttl_ms": 29_000,
            "agent_survived": true,
            "worker_exited": true,
            "process_group_empty": true,
            "exit_signal": 9,
            "audio_content_retained": false,
            "claim_id": "claim",
            "recovered": false,
            "exact_once": false,
            "synthetic_fixture": "seq-v1",
            "synthetic_digest":
                "9fda676f94adbf56e31e91462c702dcda9fcf989eece435876a28778782abfd3",
            "synthetic_first_sequence": 0,
            "synthetic_last_sequence": 63,
            "synthetic_complete": true
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(valid_recovery_payload(&offer, "recovery_offer"));
        let mut incomplete = offer;
        incomplete.insert("synthetic_last_sequence".into(), Value::from(62));
        assert!(!valid_recovery_payload(&incomplete, "recovery_offer"));
    }
}
