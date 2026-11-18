//! Typed command replies (appendix C2.1 `Reply`, C3 "Reply" columns).
//!
//! The core holds no `serde`; the shell maps a [`Reply`] to today's JSON with
//! [`ReplyType::as_str`] and [`Status::as_str`], so no caller-visible literal
//! changes.

use crate::ids::RecordingId;
use crate::status::Status;

/// The `type` literal of a typed JSON reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum ReplyType {
    AppDisabled,
    BusyRestarting,
    BusyMeeting,
    BusyTranscribingFile,
    BusyRecordingCorpus,
    BusyBenchmarking,
    AlreadyStarting,
    AlreadyRecording,
    AudioRecovering,
    AlreadyProcessing,
    RecordingSuperseded,
    NotRecording,
    RecordingCancelled,
    RecordingStarting,
}

impl ReplyType {
    /// The literal `type` string (`recording.rs` replies; C3).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppDisabled => "app_disabled",
            Self::BusyRestarting => "busy_restarting",
            Self::BusyMeeting => "busy_meeting",
            Self::BusyTranscribingFile => "busy_transcribing_file",
            Self::BusyRecordingCorpus => "busy_recording_corpus",
            Self::BusyBenchmarking => "busy_benchmarking",
            Self::AlreadyStarting => "already_starting",
            Self::AlreadyRecording => "already_recording",
            Self::AudioRecovering => "audio_recovering",
            Self::AlreadyProcessing => "already_processing",
            Self::RecordingSuperseded => "recording_superseded",
            Self::NotRecording => "not_recording",
            Self::RecordingCancelled => "recording_cancelled",
            Self::RecordingStarting => "recording_starting",
        }
    }
}

/// Which legacy `process_audio` refusal (R301.1-.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessAudioRefusal {
    /// R301.1.
    MeetingActive,
    /// R301.2.
    FileTranscription,
    /// R301.3.
    Benchmark,
    /// R301.4.
    LiveDictation,
}

impl ProcessAudioRefusal {
    /// The literal `Err` text.
    pub const fn message(self) -> &'static str {
        match self {
            Self::MeetingActive => "Cannot process audio while a meeting transcript is active.",
            Self::FileTranscription => {
                "Cannot process audio while a file transcription is in progress."
            }
            Self::Benchmark => "Cannot process audio while a benchmark is in progress.",
            Self::LiveDictation => "Cannot process audio while live dictation is active.",
        }
    }
}

/// An `Err(String)` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrText {
    /// The text comes from a shell-owned result (capture start/stop/cancel
    /// error, pipeline error): the core only says "reply with that error".
    Shell,
    /// R202.3: `Missing dictation context for recording {rid}`.
    MissingContext(RecordingId),
    /// R101.2: `Smart Auto cannot be combined with a fixed microphone.`
    SmartAutoWithFixedDevice,
    /// R101.14: the smart-auto resolution error, verbatim. Like [`ErrText::Shell`],
    /// the text is the shell's (the core only sees a typed `SmartAutoFailure`), so
    /// the shell replies with the error string it kept.
    SmartAuto,
    /// R301.1-.4.
    ProcessAudio(ProcessAudioRefusal),
}

impl ErrText {
    /// The literal text, where the core knows it (`None` for [`ErrText::Shell`] and
    /// [`ErrText::SmartAuto`], whose text the shell owns).
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Shell | Self::SmartAuto => None,
            Self::MissingContext(id) => {
                Some(format!("Missing dictation context for recording {}", id.0))
            }
            Self::SmartAutoWithFixedDevice => {
                Some("Smart Auto cannot be combined with a fixed microphone.".to_string())
            }
            Self::ProcessAudio(refusal) => Some(refusal.message().to_string()),
        }
    }
}

/// The text slot of a `transcription` reply. The text itself is transcript
/// content and never enters the core (CLAUDE.md privacy rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptText {
    /// `""` (no speech / too short / interrupted).
    Empty,
    /// The pipeline's text, held by the shell.
    FromPipeline,
}

/// Typed form of every command reply (C2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `{"type": ty, "state": state[, "recordingId": id]}`.
    Typed {
        /// `type` literal.
        ty: ReplyType,
        /// `state` literal.
        state: Status,
        /// `recordingId`, only on `recording_starting` (R104.7).
        recording_id: Option<RecordingId>,
    },
    /// `{"type":"transcription","text":T,"state":state}`.
    Transcription {
        /// Text slot.
        text: TranscriptText,
        /// `state` literal.
        state: Status,
    },
    /// `Ok(())` (cancel).
    Ok,
    /// `Err(text)`.
    Err(ErrText),
}

impl Reply {
    /// `{"type": ty, "state": state}`.
    pub const fn typed(ty: ReplyType, state: Status) -> Self {
        Self::Typed {
            ty,
            state,
            recording_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_type_literals_match_the_code() {
        let all = [
            (ReplyType::AppDisabled, "app_disabled"),
            (ReplyType::BusyRestarting, "busy_restarting"),
            (ReplyType::BusyMeeting, "busy_meeting"),
            (ReplyType::BusyTranscribingFile, "busy_transcribing_file"),
            (ReplyType::BusyRecordingCorpus, "busy_recording_corpus"),
            (ReplyType::BusyBenchmarking, "busy_benchmarking"),
            (ReplyType::AlreadyStarting, "already_starting"),
            (ReplyType::AlreadyRecording, "already_recording"),
            (ReplyType::AudioRecovering, "audio_recovering"),
            (ReplyType::AlreadyProcessing, "already_processing"),
            (ReplyType::RecordingSuperseded, "recording_superseded"),
            (ReplyType::NotRecording, "not_recording"),
            (ReplyType::RecordingCancelled, "recording_cancelled"),
            (ReplyType::RecordingStarting, "recording_starting"),
        ];
        for (ty, literal) in all {
            assert_eq!(ty.as_str(), literal);
        }
    }

    #[test]
    fn err_text_literals() {
        assert_eq!(
            ErrText::MissingContext(RecordingId(7)).message().as_deref(),
            Some("Missing dictation context for recording 7")
        );
        assert_eq!(ErrText::Shell.message(), None);
        assert_eq!(
            ErrText::ProcessAudio(ProcessAudioRefusal::LiveDictation)
                .message()
                .as_deref(),
            Some("Cannot process audio while live dictation is active.")
        );
    }
}
