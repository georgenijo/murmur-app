//! The dictation status and the small closed vocabularies the other modules
//! share: error codes, terminal outcomes, trigger and cancel reasons (C2.1, C3).

/// `DictationState.status` (`state.rs` enum 12-19). M1 in C1.2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Status {
    /// No recording.
    #[default]
    Idle,
    /// Capture start requested, not yet ready.
    Starting,
    /// Capture is live.
    Recording,
    /// Capture is being torn down (supervisor `Recovering`).
    Recovering,
    /// A pipeline owns the recording.
    Processing,
}

impl Status {
    /// The literal `state` string of every reply and of `recording-status-changed`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Recording => "recording",
            Self::Recovering => "recovering",
            Self::Processing => "processing",
        }
    }

    /// Every status, in declaration order.
    pub const ALL: [Status; 5] = [
        Self::Idle,
        Self::Starting,
        Self::Recording,
        Self::Recovering,
        Self::Processing,
    ];
}

/// Mirror of `DictationErrorCode` (`dictation_telemetry.rs`): the `TERM` cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum ErrCode {
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

/// Mirror of `DictationTerminalOutcome` (`dictation_telemetry.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum TerminalOutcome {
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

/// Why a supervisor `Recovering` happened (`AudioCancelReason`, C3.6 R209).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum AudioCancelReason {
    User,
    DeviceChanged,
    SystemSleep,
    SystemWake,
    RuntimeFailure,
    HardDeadline,
}

/// Who asked for a stop (`stop_native_recording_for(Some(id), Other)`, 4853-4871).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StopTrigger {
    /// The user's stop gesture or command.
    User,
    /// The self-dispatched auto-stop (`RS`) and any other trigger.
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_are_the_reply_literals() {
        let got: Vec<_> = Status::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            got,
            ["idle", "starting", "recording", "recovering", "processing"]
        );
        assert_eq!(Status::default(), Status::Idle);
    }
}
