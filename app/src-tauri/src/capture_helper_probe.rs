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
const CANCEL_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct TestCaptureProbeConfig {
    pub helper_path: PathBuf,
    pub scenario_environment: Vec<(String, String)>,
    pub handshake_timeout: Duration,
    pub observe_for: Duration,
    pub cancel_grace: Duration,
}

enum SpawnPlan {
    Production,
    #[cfg(any(debug_assertions, feature = "llm-test-support"))]
    Test(TestCaptureProbeConfig),
}

impl SpawnPlan {
    fn helper_path(&self) -> Result<PathBuf, CaptureProbeError> {
        match self {
            Self::Production => {
                bundled_sibling(CAPTURE_HELPER_NAME).map_err(|_| CaptureProbeError::SpawnFailed)
            }
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => Ok(config.helper_path.clone()),
        }
    }

    fn scenario_environment(&self) -> &[(String, String)] {
        match self {
            Self::Production => &[],
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => &config.scenario_environment,
        }
    }

    fn handshake_timeout(&self) -> Duration {
        match self {
            Self::Production => DEFAULT_HANDSHAKE_TIMEOUT,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.handshake_timeout,
        }
    }

    fn observe_for(&self) -> Duration {
        match self {
            Self::Production => DEFAULT_OBSERVE,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.observe_for,
        }
    }

    fn cancel_grace(&self) -> Duration {
        match self {
            Self::Production => CANCEL_GRACE,
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            Self::Test(config) => config.cancel_grace,
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
            Self::HelperFailed(FailureCode::Internal) => "internal",
        }
    }
}

enum HelperEvent {
    Frame(HelperMessage),
    Exited,
    Protocol,
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

impl CaptureProbeSupervisor {
    pub fn new() -> Self {
        Self {
            plan: SpawnPlan::Production,
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
        let helper_path = self.plan.helper_path()?;
        if matches!(self.plan, SpawnPlan::Production) && !cfg!(debug_assertions) {
            crate::code_signing::validate_bundled_helper(&helper_path, CAPTURE_HELPER_IDENTIFIER)
                .map_err(|_| CaptureProbeError::SignatureInvalid)?;
        }
        let (child, mut stdin, stdout) =
            ManagedChild::spawn(&helper_path, self.plan.scenario_environment())
                .map_err(|_| CaptureProbeError::SpawnFailed)?;
        let pid = child.pid();
        *ownership = Some(child);
        let nonce = unique_nonce();
        let hello = HostMessage::Hello {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.clone(),
        };
        if write_frame(&mut stdin, &hello).is_err() {
            if let Some(child) = ownership.as_mut() {
                let _ = child.hard_kill_confirmed(Instant::now() + Duration::from_millis(250));
            }
            return Err(CaptureProbeError::Protocol);
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
        let observe_deadline = Instant::now() + self.plan.observe_for();
        let mut last_phase = None;
        let mut first_callback_ms = None;
        let mut ready = false;
        let mut result = Ok(());
        while Instant::now() < observe_deadline {
            let phase_deadline = if ready {
                observe_deadline
            } else {
                handshake_deadline.min(observe_deadline)
            };
            if Instant::now() >= phase_deadline {
                if !ready {
                    result = Err(CaptureProbeError::HandshakeTimeout);
                }
                break;
            }
            match events_rx
                .recv_timeout((phase_deadline - Instant::now()).min(Duration::from_millis(50)))
            {
                Ok(HelperEvent::Frame(frame)) => {
                    if !valid_helper_message(&frame, &nonce) {
                        result = Err(CaptureProbeError::Protocol);
                        break;
                    }
                    match frame {
                        HelperMessage::Phase { phase, .. } => last_phase = Some(phase),
                        HelperMessage::Ready { .. } => ready = true,
                        HelperMessage::FirstCallback {
                            callback_latency_ms,
                            ..
                        } => first_callback_ms = Some(callback_latency_ms),
                        HelperMessage::CallbackHealth { .. } => {}
                        HelperMessage::Failure { code, .. } => {
                            result = Err(CaptureProbeError::HelperFailed(code));
                            break;
                        }
                        HelperMessage::Stopped { .. } => {
                            result = Err(CaptureProbeError::Protocol);
                            break;
                        }
                    }
                }
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

        let cancel = HostMessage::Cancel {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.clone(),
        };
        let _ = write_frame(&mut stdin, &cancel);
        let settle_deadline = Instant::now() + self.plan.cancel_grace();
        let mut stopped = false;
        while Instant::now() < settle_deadline {
            match events_rx
                .recv_timeout((settle_deadline - Instant::now()).min(Duration::from_millis(25)))
            {
                Ok(HelperEvent::Frame(frame)) if valid_helper_message(&frame, &nonce) => {
                    match frame {
                        HelperMessage::Phase { phase, .. } => last_phase = Some(phase),
                        HelperMessage::Stopped { .. } => {
                            stopped = true;
                            break;
                        }
                        _ => {}
                    }
                }
                Ok(HelperEvent::Exited) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(_) => {}
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
            Ok(()) if ready && first_callback_ms.is_some() => "ok",
            Ok(()) if ready => "no_first_callback",
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

pub fn run_cli_if_requested() -> Option<i32> {
    let mut arguments = std::env::args();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some("--capture-helper-probe") || arguments.next().is_some() {
        return None;
    }
    let supervisor = CaptureProbeSupervisor::new();
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
}
