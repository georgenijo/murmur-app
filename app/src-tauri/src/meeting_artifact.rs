//! Legacy wire types retained for older databases and export compatibility.
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub const MEETING_ARTIFACT_SCHEMA: &str = "murmur.meeting-artifact.v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourcedMeetingText {
    pub text: String,
    pub source_segment_ids: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingActionItem {
    pub text: String,
    pub owner: Option<String>,
    pub due_date: Option<String>,
    pub source_segment_ids: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingArtifactV1 {
    pub schema: String,
    pub summary: SourcedMeetingText,
    pub decisions: Vec<SourcedMeetingText>,
    pub action_items: Vec<MeetingActionItem>,
    pub open_questions: Vec<SourcedMeetingText>,
}
