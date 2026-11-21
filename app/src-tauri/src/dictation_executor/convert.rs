//! The shell's mapping between the core crate's closed vocabularies and the
//! app's own enums (`dictation_telemetry`, `audio_lifecycle`).
//!
//! The crate mirrors these enums so it needs no app dependency (`status.rs`).
//! Every mapping here is an exhaustive `match` generated from one variant list,
//! so a variant added on either side fails to compile until both agree, and the
//! round-trip test pins the names.

use crate::audio_lifecycle::AudioCancelReason;
use crate::dictation_telemetry::{DictationErrorCode, DictationTerminalOutcome};
use murmur_dictation_core::status::{
    AudioCancelReason as CoreCancelReason, ErrCode, TerminalOutcome,
};

macro_rules! mirror {
    ($app:ident <-> $core:ident, $(#[$to_core_attr:meta])* $to_core:ident, $from_core:ident, $all:ident: $($variant:ident),+ $(,)?) => {
        /// The core vocabulary's name for an app value.
        $(#[$to_core_attr])*
        pub(crate) fn $to_core(value: $app) -> $core {
            match value {
                $($app::$variant => $core::$variant,)+
            }
        }

        /// The app value for a core vocabulary name.
        pub(crate) fn $from_core(value: $core) -> $app {
            match value {
                $($core::$variant => $app::$variant,)+
            }
        }

        #[cfg(test)]
        pub(crate) const $all: &[$core] = &[$($core::$variant),+];
    };
}

mirror!(DictationErrorCode <-> ErrCode, error_code_to_core, error_code_from_core, ALL_ERROR_CODES:
    None, EmptyAudio, VadNoSpeech, EmptyOutput, CoremlVadRetryExhausted, BelowMinimumDuration,
    CancelledStarting, CancelledRecording, CancelledProcessing, MissingContext, StaleOwner,
    StopFinalizationFailed, TranscriptionFailed, RuntimeFailure, DeviceChanged, SystemSleep,
    SystemWake, PermissionDenied, DeviceUnavailable, HostUnavailable, InvalidInput,
    ResourceExhausted, StreamInvalidated, UnsupportedConfig, BackendError, ProtocolError,
    FirstBufferTimeout, InitializationTimeout, PermissionPromptTimeout, TerminationUnconfirmed,
    WorkerPanicked, SignatureInvalid);

// `terminal_outcome_to_core` is read only by tests; production maps core to app only.
mirror!(DictationTerminalOutcome <-> TerminalOutcome, #[cfg_attr(not(test), allow(dead_code))] terminal_outcome_to_core, terminal_outcome_from_core, ALL_TERMINAL_OUTCOMES:
    Success, NoSpeech, TooShort, UserCancelledStarting, UserCancelledRecording,
    UserCancelledProcessing, CaptureInitFailure, RuntimeInterruption, StopFailure,
    PipelineFailure, Superseded);

mirror!(AudioCancelReason <-> CoreCancelReason, cancel_reason_to_core, cancel_reason_from_core, ALL_CANCEL_REASONS:
    User, DeviceChanged, SystemSleep, SystemWake, RuntimeFailure, HardDeadline);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_round_trips_by_name() {
        assert_eq!(ALL_ERROR_CODES.len(), 32);
        for &code in ALL_ERROR_CODES {
            let app = error_code_from_core(code);
            assert_eq!(error_code_to_core(app), code);
            assert_eq!(format!("{app:?}"), format!("{code:?}"));
        }
    }

    #[test]
    fn every_terminal_outcome_and_cancel_reason_round_trips_by_name() {
        for &outcome in ALL_TERMINAL_OUTCOMES {
            let app = terminal_outcome_from_core(outcome);
            assert_eq!(terminal_outcome_to_core(app), outcome);
            assert_eq!(format!("{app:?}"), format!("{outcome:?}"));
        }
        for &reason in ALL_CANCEL_REASONS {
            let app = cancel_reason_from_core(reason);
            assert_eq!(cancel_reason_to_core(app), reason);
            assert_eq!(format!("{app:?}"), format!("{reason:?}"));
        }
    }
}
