use serde::{Deserialize, Serialize};

pub const MEETING_STORE_SCHEMA_VERSION: u32 = 2;
pub const MAX_MEETING_PAGE_SIZE: u32 = 100;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetingStoreAvailability {
    #[default]
    Unavailable,
    Available,
    Recovered,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingStoreStatus {
    pub availability: MeetingStoreAvailability,
    pub schema_version: u32,
    pub session_count: u64,
    pub pending_segment_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MeetingSpeaker {
    Me,
    Them,
}

impl MeetingSpeaker {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Them => "them",
        }
    }

    pub(crate) fn from_db(value: &str) -> rusqlite::Result<Self> {
        match value {
            "me" => Ok(Self::Me),
            "them" => Ok(Self::Them),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetingSessionStatus {
    Active,
    Complete,
    Interrupted,
    Failed,
}

impl MeetingSessionStatus {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Complete => "complete",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_db(value: &str) -> rusqlite::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "complete" => Ok(Self::Complete),
            "interrupted" => Ok(Self::Interrupted),
            "failed" => Ok(Self::Failed),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetingSegmentStatus {
    Pending,
    Final,
    Failed,
}

impl MeetingSegmentStatus {
    pub(crate) fn from_db(value: &str) -> rusqlite::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "final" => Ok(Self::Final),
            "failed" => Ok(Self::Failed),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSession {
    pub id: String,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub status: MeetingSessionStatus,
    pub model_name: String,
    pub language: String,
    pub smart_punctuation: bool,
    pub retain_audio: bool,
    pub duration_ms: u64,
    pub segment_count: u64,
    pub preview: String,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSegment {
    pub id: i64,
    pub session_id: String,
    pub speaker: MeetingSpeaker,
    pub sequence: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: MeetingSegmentStatus,
    pub text: String,
    pub audio_available: bool,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingMeetingSegment {
    pub id: i64,
    pub session_id: String,
    pub speaker: MeetingSpeaker,
    pub sequence: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub audio_relative_path: String,
    pub model_name: String,
    pub language: String,
    pub smart_punctuation: bool,
    pub retain_audio: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPage {
    pub sessions: Vec<MeetingSession>,
    pub total: u64,
    pub offset: u64,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub session: MeetingSession,
    pub segments: Vec<MeetingSegment>,
    pub artifact: Option<crate::meeting_artifact::MeetingArtifactV1>,
}
