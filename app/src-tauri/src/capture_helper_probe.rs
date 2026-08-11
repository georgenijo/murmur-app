//! Opt-in Phase-0 capture-helper supervisor for issue #407.
//!
//! This is not wired into dictation. It exists only behind an explicit CLI
//! probe entry point so signed/notarized bundles can exercise direct-child TCC
//! attribution and hard-kill behavior without retaining or transporting PCM.

use crate::managed_child::{bundled_sibling, ConfirmedTermination, ManagedChild};
use murmur_capture_helper_protocol::{
    read_frame, valid_helper_message, write_frame, CapturePhase, FailureCode, FrameError,
    HelperMessage, HostMessage, PROTOCOL_NAME, PROTOCOL_VERSION,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CAPTURE_HELPER_NAME: &str = "murmur-capture-helper";
pub const CAPTURE_HELPER_IDENTIFIER: &str = "com.localdictation.capture-helper";
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_OBSERVE: Duration = Duration::from_secs(5);
const MAX_OBSERVE_SECONDS: u64 = 300;
const CANCEL_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct TestCaptureProbeConfig {
    pub helper_path: PathBuf,
    pub scenario_environment: Vec<(String, String)>,
    pub permission_status_sequence: Vec<String>,
    pub handshake_timeout: Duration,
    pub observe_for: Duration,
    pub cancel_grace: Duration,
    pub spawned_signal: Option<mpsc::Sender<u32>>,
}

enum SpawnPlan {
    Production {
        observe_for: Duration,
    },
    #[cfg(any(debug_assertions, feature = "llm-test-support"))]
    Test(TestCaptureProbeConfig),
}

impl SpawnPlan {
    fn helper_path(&self) -> Result<PathBuf, CaptureProbeError> {
        match self {
            Self::Production { .. } => {
                bundled_sibling(CAPTURE_HELPER_NAME).map_err(|_| CaptureProbeError::SpawnFailed)
            }
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => Ok(config.helper_path.clone()),
        }
    }

    fn scenario_environment(&self) -> &[(String, String)] {
        match self {
            Self::Production { .. } => &[],
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => &config.scenario_environment,
        }
    }

    fn handshake_timeout(&self) -> Duration {
        match self {
            Self::Production { .. } => DEFAULT_HANDSHAKE_TIMEOUT,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.handshake_timeout,
        }
    }

    fn observe_for(&self) -> Duration {
        match self {
            Self::Production { observe_for } => *observe_for,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.observe_for,
        }
    }

    fn cancel_grace(&self) -> Duration {
        match self {
            Self::Production { .. } => CANCEL_GRACE,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.cancel_grace,
        }
    }

    fn microphone_permission_status(&self, _test_index: &mut usize) -> String {
        match self {
            Self::Production { .. } => {
                crate::commands::permissions::check_microphone_permission_status()
            }
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => {
                let status = config
                    .permission_status_sequence
                    .get(*_test_index)
                    .cloned()
                    .or_else(|| config.permission_status_sequence.last().cloned())
                    .unwrap_or_else(|| "granted".to_string());
                *_test_index = _test_index.saturating_add(1);
                status
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureProbeError {
    Busy,
    SpawnFailed,
    SignatureInvalid,
    HandshakeTimeout,
    Protocol,
    HelperFailed(FailureCode),
}

impl CaptureProbeError {
    fn as_str(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::SpawnFailed => "spawn_failed",
            Self::SignatureInvalid => "signature_invalid",
            Self::HandshakeTimeout => "handshake_timeout",
            Self::Protocol => "protocol",
            Self::HelperFailed(FailureCode::PermissionDenied) => "permission_denied",
            Self::HelperFailed(FailureCode::NoInputDevice) => "no_input_device",
            Self::HelperFailed(FailureCode::EnumerationFailed) => "enumeration_failed",
            Self::HelperFailed(FailureCode::ConfigurationFailed) => "configuration_failed",
            Self::HelperFailed(FailureCode::StreamOpenFailed) => "stream_open_failed",
            Self::HelperFailed(FailureCode::StreamStartFailed) => "stream_start_failed",
            Self::HelperFailed(FailureCode::StreamError) => "stream_error",
            Self::HelperFailed(FailureCode::CallbackStalled) => "callback_stalled",
            Self::HelperFailed(FailureCode::InvalidMessage) => "invalid_message",
            Self::HelperFailed(FailureCode::UnsupportedOs) => "unsupported_os",
            Self::HelperFailed(FailureCode::SystemAudioUnavailable) => "system_audio_unavailable",
            Self::HelperFailed(FailureCode::Internal) => "internal",
        }
    }
}

enum HelperEvent {
    Frame(HelperMessage),
    Exited,
    Protocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperProtocolState {
    AwaitEnumeration,
    AwaitStreamOpen,
    AwaitReady,
    AwaitAwaitingFirstCallback,
    AwaitFirstCallback,
    AwaitActive,
    Active,
    Failed,
    Stopping,
    Stopped,
}

enum AcceptedFrame {
    Phase(CapturePhase),
    Ready,
    FirstCallback(u64),
    Health,
    Failure(FailureCode),
    Stopped,
}

struct HelperProtocol {
    state: HelperProtocolState,
    cancel_sent: bool,
}

impl HelperProtocol {
    fn new() -> Self {
        Self {
            state: HelperProtocolState::AwaitEnumeration,
            cancel_sent: false,
        }
    }

    fn awaiting_first_callback(&self) -> bool {
        self.state == HelperProtocolState::AwaitFirstCallback
    }

    fn begin_cancel(&mut self) {
        self.cancel_sent = true;
    }

    fn accept(&mut self, frame: HelperMessage, expected_nonce: &str) -> Result<AcceptedFrame, ()> {
        if !valid_helper_message(&frame, expected_nonce) {
            return Err(());
        }
        match frame {
            HelperMessage::Phase { phase, .. } => {
                let next = match (self.state, phase) {
                    (HelperProtocolState::AwaitEnumeration, CapturePhase::Enumeration) => {
                        HelperProtocolState::AwaitStreamOpen
                    }
                    (HelperProtocolState::AwaitStreamOpen, CapturePhase::StreamOpen) => {
                        HelperProtocolState::AwaitReady
                    }
                    (
                        HelperProtocolState::AwaitAwaitingFirstCallback,
                        CapturePhase::AwaitingFirstCallback,
                    ) => HelperProtocolState::AwaitFirstCallback,
                    (HelperProtocolState::AwaitActive, CapturePhase::Active) => {
                        HelperProtocolState::Active
                    }
                    (state, CapturePhase::Stopping)
                        if self.cancel_sent
                            && !matches!(
                                state,
                                HelperProtocolState::Failed
                                    | HelperProtocolState::Stopping
                                    | HelperProtocolState::Stopped
                            ) =>
                    {
                        HelperProtocolState::Stopping
                    }
                    _ => return Err(()),
                };
                self.state = next;
                Ok(AcceptedFrame::Phase(phase))
            }
            HelperMessage::Ready { .. } if self.state == HelperProtocolState::AwaitReady => {
                self.state = HelperProtocolState::AwaitAwaitingFirstCallback;
                Ok(AcceptedFrame::Ready)
            }
            HelperMessage::FirstCallback {
                callback_latency_ms,
                ..
            } if self.state == HelperProtocolState::AwaitFirstCallback => {
                self.state = HelperProtocolState::AwaitActive;
                Ok(AcceptedFrame::FirstCallback(callback_latency_ms))
            }
            HelperMessage::CallbackHealth {
                callback_count_bucket,
                ..
            } if self.state == HelperProtocolState::Active
                && matches!(
                    callback_count_bucket.as_str(),
                    "0" | "le10" | "le100" | "le1k" | "gt1k"
                ) =>
            {
                Ok(AcceptedFrame::Health)
            }
            HelperMessage::Failure { code, .. }
                if !matches!(
                    self.state,
                    HelperProtocolState::Failed
                        | HelperProtocolState::Stopping
                        | HelperProtocolState::Stopped
                ) =>
            {
                self.state = HelperProtocolState::Failed;
                Ok(AcceptedFrame::Failure(code))
            }
            HelperMessage::Stopped { .. }
                if self.cancel_sent && self.state == HelperProtocolState::Stopping =>
            {
                self.state = HelperProtocolState::Stopped;
                Ok(AcceptedFrame::Stopped)
            }
            _ => Err(()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CaptureProbeEvidence {
    pub schema_version: u8,
    pub outcome: &'static str,
    pub last_phase: Option<&'static str>,
    pub helper_pid: u32,
    pub first_callback_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub termination: &'static str,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub process_group_empty: bool,
    pub audio_content_retained: bool,
}

pub struct CaptureProbeSupervisor {
    plan: SpawnPlan,
    ownership: Mutex<Option<ManagedChild>>,
}

impl Default for CaptureProbeSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureProbeSupervisor {
    pub fn new() -> Self {
        Self::with_observe_for(DEFAULT_OBSERVE)
    }

    fn with_observe_for(observe_for: Duration) -> Self {
        Self {
            plan: SpawnPlan::Production { observe_for },
            ownership: Mutex::new(None),
        }
    }

    #[cfg(any(debug_assertions, feature = "llm-test-support"))]
    pub fn for_test(config: TestCaptureProbeConfig) -> Self {
        Self {
            plan: SpawnPlan::Test(config),
            ownership: Mutex::new(None),
        }
    }

    pub fn run(&self) -> Result<CaptureProbeEvidence, CaptureProbeError> {
        let mut ownership = self
            .ownership
            .try_lock()
            .map_err(|_| CaptureProbeError::Busy)?;
        if let Some(stale) = ownership.as_mut() {
            if stale
                .hard_kill_confirmed(Instant::now() + Duration::from_millis(250))
                .is_none()
            {
                return Err(CaptureProbeError::Busy);
            }
            ownership.take();
        }
        let started = Instant::now();
        let mut permission_status_index = 0;
        let initial_permission_status = self
            .plan
            .microphone_permission_status(&mut permission_status_index);
        if initial_permission_status != "granted" {
            return Err(CaptureProbeError::HelperFailed(
                FailureCode::PermissionDenied,
            ));
        }
        let helper_path = self.plan.helper_path()?;
        if matches!(self.plan, SpawnPlan::Production { .. }) && !cfg!(debug_assertions) {
            crate::code_signing::validate_bundled_helper(&helper_path, CAPTURE_HELPER_IDENTIFIER)
                .map_err(|_| CaptureProbeError::SignatureInvalid)?;
        }
        let (child, mut stdin, stdout) =
            ManagedChild::spawn(&helper_path, self.plan.scenario_environment())
                .map_err(|_| CaptureProbeError::SpawnFailed)?;
        let pid = child.pid();
        *ownership = Some(child);
        #[cfg(any(debug_assertions, feature = "llm-test-support"))]
        if let SpawnPlan::Test(config) = &self.plan {
            if let Some(signal) = &config.spawned_signal {
                let _ = signal.send(pid);
            }
        }
        let nonce = unique_nonce();
        let hello = HostMessage::Hello {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.clone(),
        };
        if write_frame(&mut stdin, &hello).is_err() {
            let terminated = ownership.as_mut().and_then(|child| {
                child.hard_kill_confirmed(Instant::now() + Duration::from_millis(250))
            });
            if terminated.is_some() {
                ownership.take();
                return Err(CaptureProbeError::Protocol);
            }
            return Err(CaptureProbeError::Busy);
        }

        let (events_tx, events_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stdout = stdout;
            loop {
                match read_frame::<HelperMessage>(&mut stdout) {
                    Ok(frame) => {
                        if events_tx.send(HelperEvent::Frame(frame)).is_err() {
                            return;
                        }
                    }
                    Err(FrameError::IncompleteHeader) => {
                        let _ = events_tx.send(HelperEvent::Exited);
                        return;
                    }
                    Err(_) => {
                        let _ = events_tx.send(HelperEvent::Protocol);
                        return;
                    }
                }
            }
        });

        let handshake_deadline = Instant::now() + self.plan.handshake_timeout();
        let mut observe_deadline = None;
        let mut last_phase = None;
        let mut first_callback_ms = None;
        let mut active_seen = false;
        let mut timed_out_waiting_for_callback = false;
        let mut protocol = HelperProtocol::new();
        let mut result = Ok(());
        loop {
            if active_seen {
                let permission_status = self
                    .plan
                    .microphone_permission_status(&mut permission_status_index);
                if permission_status != "granted" {
                    result = Err(CaptureProbeError::HelperFailed(
                        FailureCode::PermissionDenied,
                    ));
                    break;
                }
            }
            let phase_deadline = observe_deadline.unwrap_or(handshake_deadline);
            if Instant::now() >= phase_deadline {
                if observe_deadline.is_none() {
                    if protocol.awaiting_first_callback() {
                        timed_out_waiting_for_callback = true;
                    } else {
                        result = Err(CaptureProbeError::HandshakeTimeout);
                    }
                }
                if !active_seen && first_callback_ms.is_some() {
                    result = Err(CaptureProbeError::HandshakeTimeout);
                }
                break;
            }
            match events_rx
                .recv_timeout((phase_deadline - Instant::now()).min(Duration::from_millis(50)))
            {
                Ok(HelperEvent::Frame(frame)) => match protocol.accept(frame, &nonce) {
                    Ok(AcceptedFrame::Phase(phase)) => {
                        last_phase = Some(phase);
                        if phase == CapturePhase::Active {
                            active_seen = true;
                            observe_deadline = Some(Instant::now() + self.plan.observe_for());
                        }
                    }
                    Ok(AcceptedFrame::Ready | AcceptedFrame::Health) => {}
                    Ok(AcceptedFrame::FirstCallback(callback_latency_ms)) => {
                        first_callback_ms = Some(callback_latency_ms)
                    }
                    Ok(AcceptedFrame::Failure(code)) => {
                        result = Err(CaptureProbeError::HelperFailed(code));
                        break;
                    }
                    Ok(AcceptedFrame::Stopped) | Err(()) => {
                        result = Err(CaptureProbeError::Protocol);
                        break;
                    }
                },
                Ok(HelperEvent::Exited) | Err(RecvTimeoutError::Disconnected) => {
                    result = Err(CaptureProbeError::Protocol);
                    break;
                }
                Ok(HelperEvent::Protocol) => {
                    result = Err(CaptureProbeError::Protocol);
                    break;
                }
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }

        let preserve_revocation = matches!(
            result,
            Err(CaptureProbeError::HelperFailed(
                FailureCode::PermissionDenied
            ))
        );
        let cancel = HostMessage::Cancel {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.clone(),
        };
        let _ = write_frame(&mut stdin, &cancel);
        protocol.begin_cancel();
        let settle_deadline = Instant::now() + self.plan.cancel_grace();
        let mut stopped = false;
        while Instant::now() < settle_deadline {
            match events_rx
                .recv_timeout((settle_deadline - Instant::now()).min(Duration::from_millis(25)))
            {
                Ok(HelperEvent::Frame(frame)) => match protocol.accept(frame, &nonce) {
                    Ok(AcceptedFrame::Phase(phase)) => last_phase = Some(phase),
                    Ok(AcceptedFrame::Stopped) => {
                        stopped = true;
                        break;
                    }
                    Ok(
                        AcceptedFrame::Ready
                        | AcceptedFrame::FirstCallback(_)
                        | AcceptedFrame::Health,
                    ) => {}
                    Ok(AcceptedFrame::Failure(code)) => {
                        if !preserve_revocation {
                            result = Err(CaptureProbeError::HelperFailed(code));
                        }
                        break;
                    }
                    Err(()) => {
                        if !preserve_revocation {
                            result = Err(CaptureProbeError::Protocol);
                        }
                        break;
                    }
                },
                Ok(HelperEvent::Exited) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(HelperEvent::Protocol) => {
                    if !preserve_revocation {
                        result = Err(CaptureProbeError::Protocol);
                    }
                    break;
                }
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }

        let child = ownership
            .as_mut()
            .expect("owned capture helper remains present until confirmed exit");
        let (termination_kind, termination) = if stopped {
            match child.wait_for_exit(settle_deadline) {
                Some(termination) => ("cooperative", termination),
                None => (
                    "hard_kill",
                    child
                        .hard_kill_confirmed(Instant::now() + Duration::from_millis(500))
                        .ok_or(CaptureProbeError::Busy)?,
                ),
            }
        } else if let Ok(Some(_)) = child.try_wait() {
            (
                "exited",
                child
                    .wait_for_exit(Instant::now() + Duration::from_millis(250))
                    .or_else(|| {
                        child.hard_kill_confirmed(Instant::now() + Duration::from_millis(500))
                    })
                    .ok_or(CaptureProbeError::Busy)?,
            )
        } else {
            (
                "hard_kill",
                child
                    .hard_kill_confirmed(Instant::now() + Duration::from_millis(500))
                    .ok_or(CaptureProbeError::Busy)?,
            )
        };
        ownership.take();

        let outcome = match result {
            Ok(()) if timed_out_waiting_for_callback => "no_first_callback",
            Ok(()) if active_seen && first_callback_ms.is_some() => "ok",
            Ok(()) => CaptureProbeError::HandshakeTimeout.as_str(),
            Err(error) => error.as_str(),
        };
        let evidence = evidence(
            outcome,
            last_phase,
            pid,
            first_callback_ms,
            started,
            termination_kind,
            termination,
        );
        Ok(evidence)
    }
}

fn phase_name(phase: CapturePhase) -> &'static str {
    match phase {
        CapturePhase::Enumeration => "enumeration",
        CapturePhase::StreamOpen => "stream_open",
        CapturePhase::AwaitingFirstCallback => "awaiting_first_callback",
        CapturePhase::Active => "active",
        CapturePhase::Stopping => "stopping",
    }
}

fn evidence(
    outcome: &'static str,
    phase: Option<CapturePhase>,
    helper_pid: u32,
    first_callback_ms: Option<u64>,
    started: Instant,
    termination: &'static str,
    confirmed: ConfirmedTermination,
) -> CaptureProbeEvidence {
    CaptureProbeEvidence {
        schema_version: 1,
        outcome,
        last_phase: phase.map(phase_name),
        helper_pid,
        first_callback_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        termination,
        exit_code: confirmed.exit_code,
        exit_signal: confirmed.exit_signal,
        process_group_empty: confirmed.process_group_empty,
        audio_content_retained: false,
    }
}

fn unique_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("capture-{}-{nanos}", std::process::id())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliRequest {
    NotRequested,
    Probe(Duration),
    Invalid,
}

fn parse_cli_request(arguments: impl IntoIterator<Item = String>) -> CliRequest {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => CliRequest::NotRequested,
        [probe] if probe == "--capture-helper-probe" => CliRequest::Probe(DEFAULT_OBSERVE),
        [probe, flag, seconds]
            if probe == "--capture-helper-probe" && flag == "--observe-seconds" =>
        {
            match seconds.parse::<u64>() {
                Ok(seconds @ 1..=MAX_OBSERVE_SECONDS) => {
                    CliRequest::Probe(Duration::from_secs(seconds))
                }
                _ => CliRequest::Invalid,
            }
        }
        [first, ..] if first == "--capture-helper-probe" => CliRequest::Invalid,
        _ => CliRequest::NotRequested,
    }
}

pub fn run_cli_if_requested() -> Option<i32> {
    let request = parse_cli_request(std::env::args().skip(1));
    let observe_for = match request {
        CliRequest::NotRequested => return None,
        CliRequest::Probe(observe_for) => observe_for,
        CliRequest::Invalid => {
            println!(
                "{}",
                serde_json::json!({
                    "schema_version": 1,
                    "outcome": "invalid_arguments",
                    "audio_content_retained": false
                })
            );
            return Some(64);
        }
    };
    let supervisor = CaptureProbeSupervisor::with_observe_for(observe_for);
    match supervisor.run() {
        Ok(evidence) => match serde_json::to_string(&evidence) {
            Ok(json) => {
                println!("{json}");
                Some(if evidence.outcome == "ok" { 0 } else { 2 })
            }
            Err(_) => Some(70),
        },
        Err(error) => {
            let fallback = serde_json::json!({
                "schema_version": 1,
                "outcome": error.as_str(),
                "audio_content_retained": false
            });
            println!("{fallback}");
            Some(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_error_labels_are_content_free() {
        assert_eq!(
            CaptureProbeError::HandshakeTimeout.as_str(),
            "handshake_timeout"
        );
        assert_eq!(
            CaptureProbeError::HelperFailed(FailureCode::CallbackStalled).as_str(),
            "callback_stalled"
        );
    }

    #[test]
    fn cli_observation_duration_is_explicit_strict_and_bounded() {
        assert_eq!(
            parse_cli_request(["--capture-helper-probe".to_string()]),
            CliRequest::Probe(DEFAULT_OBSERVE)
        );
        assert_eq!(
            parse_cli_request([
                "--capture-helper-probe".to_string(),
                "--observe-seconds".to_string(),
                "120".to_string(),
            ]),
            CliRequest::Probe(Duration::from_secs(120))
        );
        for arguments in [
            vec![
                "--capture-helper-probe".to_string(),
                "--observe-seconds".to_string(),
                "0".to_string(),
            ],
            vec![
                "--capture-helper-probe".to_string(),
                "--observe-seconds".to_string(),
                "301".to_string(),
            ],
            vec![
                "--capture-helper-probe".to_string(),
                "--observe-seconds".to_string(),
                "1.5".to_string(),
            ],
            vec!["--capture-helper-probe".to_string(), "extra".to_string()],
        ] {
            assert_eq!(parse_cli_request(arguments), CliRequest::Invalid);
        }
        assert_eq!(
            parse_cli_request(["--other".to_string()]),
            CliRequest::NotRequested
        );
    }
}
