//! Stable, privacy-safe lifecycle telemetry for accepted live dictations.

use crate::MutexExt;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

const MAX_TRACKED_ATTEMPTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DictationTerminalOutcome {
    Success,
    NoSpeech,
    TooShort,
    UserCancelledStarting,
    UserCancelledRecording,
    UserCancelledProcessing,
    CaptureInitFailure,
    RuntimeInterruption,
    StopFailure,
    PipelineFailure,
    Superseded,
}

impl DictationTerminalOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoSpeech => "no_speech",
            Self::TooShort => "too_short",
            Self::UserCancelledStarting => "user_cancelled_starting",
            Self::UserCancelledRecording => "user_cancelled_recording",
            Self::UserCancelledProcessing => "user_cancelled_processing",
            Self::CaptureInitFailure => "capture_init_failure",
            Self::RuntimeInterruption => "runtime_interruption",
            Self::StopFailure => "stop_failure",
            Self::PipelineFailure => "pipeline_failure",
            Self::Superseded => "superseded",
        }
    }

    fn is_failure(self) -> bool {
        matches!(
            self,
            Self::CaptureInitFailure | Self::StopFailure | Self::PipelineFailure
        )
    }

    fn is_warning(self) -> bool {
        matches!(self, Self::RuntimeInterruption | Self::Superseded)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DictationErrorCode {
    None,
    EmptyAudio,
    VadNoSpeech,
    EmptyOutput,
    CoremlVadRetryExhausted,
    BelowMinimumDuration,
    CancelledStarting,
    CancelledRecording,
    CancelledProcessing,
    MissingContext,
    StaleOwner,
    StopFinalizationFailed,
    TranscriptionFailed,
    RuntimeFailure,
    DeviceChanged,
    SystemSleep,
    SystemWake,
    PermissionDenied,
    DeviceUnavailable,
    HostUnavailable,
    InvalidInput,
    ResourceExhausted,
    StreamInvalidated,
    UnsupportedConfig,
    BackendError,
    ProtocolError,
    FirstBufferTimeout,
    InitializationTimeout,
    PermissionPromptTimeout,
    TerminationUnconfirmed,
    WorkerPanicked,
    SignatureInvalid,
}

impl DictationErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::EmptyAudio => "empty_audio",
            Self::VadNoSpeech => "vad_no_speech",
            Self::EmptyOutput => "empty_output",
            Self::CoremlVadRetryExhausted => "coreml_vad_retry_exhausted",
            Self::BelowMinimumDuration => "below_minimum_duration",
            Self::CancelledStarting => "cancelled_starting",
            Self::CancelledRecording => "cancelled_recording",
            Self::CancelledProcessing => "cancelled_processing",
            Self::MissingContext => "missing_context",
            Self::StaleOwner => "stale_owner",
            Self::StopFinalizationFailed => "stop_finalization_failed",
            Self::TranscriptionFailed => "transcription_failed",
            Self::RuntimeFailure => "runtime_failure",
            Self::DeviceChanged => "device_changed",
            Self::SystemSleep => "system_sleep",
            Self::SystemWake => "system_wake",
            Self::PermissionDenied => "permission_denied",
            Self::DeviceUnavailable => "device_unavailable",
            Self::HostUnavailable => "host_unavailable",
            Self::InvalidInput => "invalid_input",
            Self::ResourceExhausted => "resource_exhausted",
            Self::StreamInvalidated => "stream_invalidated",
            Self::UnsupportedConfig => "unsupported_config",
            Self::BackendError => "backend_error",
            Self::ProtocolError => "protocol_error",
            Self::FirstBufferTimeout => "first_buffer_timeout",
            Self::InitializationTimeout => "initialization_timeout",
            Self::PermissionPromptTimeout => "permission_prompt_timeout",
            Self::TerminationUnconfirmed => "termination_unconfirmed",
            Self::WorkerPanicked => "worker_panicked",
            Self::SignatureInvalid => "signature_invalid",
        }
    }

    pub(crate) fn from_audio_failure(kind: crate::audio::AudioFailureKind) -> Self {
        use crate::audio::AudioFailureKind;
        match kind {
            AudioFailureKind::PermissionDenied => Self::PermissionDenied,
            AudioFailureKind::DeviceUnavailable => Self::DeviceUnavailable,
            AudioFailureKind::HostUnavailable => Self::HostUnavailable,
            AudioFailureKind::InvalidInput => Self::InvalidInput,
            AudioFailureKind::ResourceExhausted => Self::ResourceExhausted,
            AudioFailureKind::StreamInvalidated => Self::StreamInvalidated,
            AudioFailureKind::UnsupportedConfig => Self::UnsupportedConfig,
            AudioFailureKind::BackendError => Self::BackendError,
            AudioFailureKind::ProtocolError => Self::ProtocolError,
            AudioFailureKind::FirstBufferTimeout => Self::FirstBufferTimeout,
            AudioFailureKind::InitializationTimeout => Self::InitializationTimeout,
            AudioFailureKind::PermissionPromptTimeout => Self::PermissionPromptTimeout,
            AudioFailureKind::TerminationUnconfirmed => Self::TerminationUnconfirmed,
            AudioFailureKind::WorkerPanicked => Self::WorkerPanicked,
            AudioFailureKind::SignatureInvalid => Self::SignatureInvalid,
        }
    }

    pub(crate) fn from_interruption_reason(reason: &str) -> Self {
        match reason {
            "permission_denied" => Self::PermissionDenied,
            "device_unavailable" => Self::DeviceUnavailable,
            "host_unavailable" => Self::HostUnavailable,
            "invalid_input" => Self::InvalidInput,
            "resource_exhausted" => Self::ResourceExhausted,
            "stream_invalidated" => Self::StreamInvalidated,
            "unsupported_config" => Self::UnsupportedConfig,
            "backend_error" => Self::BackendError,
            "protocol_error" => Self::ProtocolError,
            "first_buffer_timeout" => Self::FirstBufferTimeout,
            "initialization_timeout" => Self::InitializationTimeout,
            "permission_prompt_timeout" => Self::PermissionPromptTimeout,
            "termination_unconfirmed" => Self::TerminationUnconfirmed,
            "worker_panicked" => Self::WorkerPanicked,
            "signature_invalid" => Self::SignatureInvalid,
            _ => Self::RuntimeFailure,
        }
    }
}

#[derive(Debug, Default)]
struct AttemptState {
    terminal: Option<(DictationTerminalOutcome, DictationErrorCode)>,
}

#[derive(Debug, Default)]
struct LifecycleState {
    attempts: HashMap<u64, AttemptState>,
    order: VecDeque<u64>,
}

/// Process-local guard for the producer's exactly-once terminal contract.
/// Recording IDs restart with the process, so no state is persisted.
#[derive(Debug, Default)]
pub(crate) struct DictationTelemetry {
    inner: Mutex<LifecycleState>,
}

impl DictationTelemetry {
    pub(crate) fn accepted(&self, recording_id: u64) {
        let mut state = self.inner.lock_or_recover();
        if state.attempts.contains_key(&recording_id) {
            return;
        }
        while state.order.len() >= MAX_TRACKED_ATTEMPTS {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.attempts.remove(&oldest);
        }
        state.order.push_back(recording_id);
        state.attempts.insert(recording_id, AttemptState::default());
    }

    fn claim_terminal(
        &self,
        recording_id: u64,
        outcome: DictationTerminalOutcome,
        error_code: DictationErrorCode,
    ) -> bool {
        let mut state = self.inner.lock_or_recover();
        let Some(attempt) = state.attempts.get_mut(&recording_id) else {
            return false;
        };
        if attempt.terminal.is_some() {
            return false;
        }
        attempt.terminal = Some((outcome, error_code));
        true
    }

    #[cfg(test)]
    pub(crate) fn terminal_for_test(
        &self,
        recording_id: u64,
    ) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
        self.inner
            .lock_or_recover()
            .attempts
            .get(&recording_id)
            .and_then(|attempt| attempt.terminal)
    }

    pub(crate) fn emit_terminal(
        &self,
        recording_id: u64,
        outcome: DictationTerminalOutcome,
        error_code: DictationErrorCode,
    ) -> bool {
        self.emit_terminal_with_output(recording_id, outcome, error_code, 0)
    }

    pub(crate) fn emit_terminal_with_output(
        &self,
        recording_id: u64,
        outcome: DictationTerminalOutcome,
        error_code: DictationErrorCode,
        char_count: u64,
    ) -> bool {
        if !self.claim_terminal(recording_id, outcome, error_code) {
            return false;
        }
        if outcome.is_failure() {
            tracing::error!(
                target: "pipeline",
                event_code = "pipeline.dictation_terminal",
                recording_id,
                outcome = outcome.as_str(),
                error_code = error_code.as_str(),
                char_count,
                "dictation reached terminal outcome"
            );
        } else if outcome.is_warning() {
            tracing::warn!(
                target: "pipeline",
                event_code = "pipeline.dictation_terminal",
                recording_id,
                outcome = outcome.as_str(),
                error_code = error_code.as_str(),
                char_count,
                "dictation reached terminal outcome"
            );
        } else {
            tracing::info!(
                target: "pipeline",
                event_code = "pipeline.dictation_terminal",
                recording_id,
                outcome = outcome.as_str(),
                error_code = error_code.as_str(),
                char_count,
                "dictation reached terminal outcome"
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_can_only_be_claimed_once_for_an_accepted_attempt() {
        let telemetry = DictationTelemetry::default();
        assert!(!telemetry.claim_terminal(
            7,
            DictationTerminalOutcome::Success,
            DictationErrorCode::None
        ));
        telemetry.accepted(7);
        assert!(telemetry.claim_terminal(
            7,
            DictationTerminalOutcome::Success,
            DictationErrorCode::None
        ));
        assert!(!telemetry.claim_terminal(
            7,
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::TranscriptionFailed
        ));
        assert_eq!(
            telemetry.terminal_for_test(7),
            Some((DictationTerminalOutcome::Success, DictationErrorCode::None))
        );
    }

    #[test]
    fn stale_terminal_cannot_consume_a_newer_attempt() {
        let telemetry = DictationTelemetry::default();
        telemetry.accepted(10);
        telemetry.accepted(11);
        assert!(telemetry.claim_terminal(
            10,
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        ));
        assert!(telemetry.claim_terminal(
            11,
            DictationTerminalOutcome::Success,
            DictationErrorCode::None
        ));
    }

    #[test]
    fn interruption_reasons_collapse_to_a_bounded_vocabulary() {
        assert_eq!(
            DictationErrorCode::from_interruption_reason("stream_invalidated"),
            DictationErrorCode::StreamInvalidated
        );
        assert_eq!(
            DictationErrorCode::from_interruption_reason("private raw error"),
            DictationErrorCode::RuntimeFailure
        );
    }

    #[test]
    fn terminal_vocabulary_covers_every_required_path() {
        let paths = [
            (DictationTerminalOutcome::Success, DictationErrorCode::None),
            (
                DictationTerminalOutcome::NoSpeech,
                DictationErrorCode::VadNoSpeech,
            ),
            (
                DictationTerminalOutcome::TooShort,
                DictationErrorCode::BelowMinimumDuration,
            ),
            (
                DictationTerminalOutcome::UserCancelledStarting,
                DictationErrorCode::CancelledStarting,
            ),
            (
                DictationTerminalOutcome::UserCancelledRecording,
                DictationErrorCode::CancelledRecording,
            ),
            (
                DictationTerminalOutcome::UserCancelledProcessing,
                DictationErrorCode::CancelledProcessing,
            ),
            (
                DictationTerminalOutcome::CaptureInitFailure,
                DictationErrorCode::InitializationTimeout,
            ),
            (
                DictationTerminalOutcome::RuntimeInterruption,
                DictationErrorCode::StreamInvalidated,
            ),
            (
                DictationTerminalOutcome::StopFailure,
                DictationErrorCode::StopFinalizationFailed,
            ),
            (
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::TranscriptionFailed,
            ),
            (
                DictationTerminalOutcome::Superseded,
                DictationErrorCode::StaleOwner,
            ),
        ];
        let telemetry = DictationTelemetry::default();
        for (index, (outcome, error_code)) in paths.into_iter().enumerate() {
            let recording_id = index as u64 + 1;
            telemetry.accepted(recording_id);
            assert!(telemetry.emit_terminal(recording_id, outcome, error_code));
            assert_eq!(
                telemetry.terminal_for_test(recording_id),
                Some((outcome, error_code))
            );
            assert!(!telemetry.emit_terminal(
                recording_id,
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::RuntimeFailure,
            ));
        }
        assert_eq!(
            paths.map(|(outcome, _)| outcome.as_str()),
            [
                "success",
                "no_speech",
                "too_short",
                "user_cancelled_starting",
                "user_cancelled_recording",
                "user_cancelled_processing",
                "capture_init_failure",
                "runtime_interruption",
                "stop_failure",
                "pipeline_failure",
                "superseded",
            ]
        );
    }
}
