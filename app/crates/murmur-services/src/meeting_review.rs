use crate::meeting_store::{
    MeetingSegment, MeetingSegmentStatus, MeetingSession, MeetingSpeaker, RemoteSpeakerLabel,
};
use serde::{Deserialize, Serialize};

mod subtitles;

pub const MEETING_REVIEW_EXPORT_SCHEMA: &str = "murmur.meeting-review-export.v2";
const MAX_LABEL_BYTES: usize = 80;
const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingSpeakerLabels {
    pub me: String,
    pub them: String,
}

impl Default for MeetingSpeakerLabels {
    fn default() -> Self {
        Self {
            me: "Me".into(),
            them: "Them".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewText {
    pub key: String,
    pub text: String,
    pub source_segment_ids: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewAction {
    pub key: String,
    pub text: String,
    pub owner: Option<String>,
    pub due_date: Option<String>,
    pub source_segment_ids: Vec<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingReviewDocumentV1 {
    pub schema: String,
    pub summary: ReviewText,
    pub decisions: Vec<ReviewText>,
    pub action_items: Vec<ReviewAction>,
    pub open_questions: Vec<ReviewText>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedMeetingReview {
    pub revision: u64,
    pub document: MeetingReviewDocumentV1,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedMeetingReview {
    pub revision: u64,
    pub based_on_generated_revision: Option<u64>,
    pub document: Option<MeetingReviewDocumentV1>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActiveReviewOrigin {
    Generated,
    Reviewed,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingWorkspace {
    pub session: MeetingSession,
    pub segments: Vec<MeetingSegment>,
    pub labels: MeetingSpeakerLabels,
    pub remote_speakers: Vec<RemoteSpeakerLabel>,
    pub generated: Option<GeneratedMeetingReview>,
    pub review: Option<SavedMeetingReview>,
    pub active_document: Option<MeetingReviewDocumentV1>,
    pub active_origin: Option<ActiveReviewOrigin>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditableReviewText {
    pub key: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditableReviewAction {
    pub key: String,
    pub text: String,
    pub owner: Option<String>,
    pub due_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditableReviewDocument {
    pub summary: EditableReviewText,
    pub decisions: Vec<EditableReviewText>,
    pub action_items: Vec<EditableReviewAction>,
    pub open_questions: Vec<EditableReviewText>,
}

// Creation has no client key. Only saved keys may be used by the edit DTOs above.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NewReviewClaim {
    Decision {
        text: String,
        #[serde(rename = "sourceSegmentIds")]
        source_segment_ids: Vec<i64>,
    },
    ActionItem {
        text: String,
        owner: Option<String>,
        #[serde(rename = "dueDate")]
        due_date: Option<String>,
        #[serde(rename = "sourceSegmentIds")]
        source_segment_ids: Vec<i64>,
    },
    OpenQuestion {
        text: String,
        #[serde(rename = "sourceSegmentIds")]
        source_segment_ids: Vec<i64>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewEditBase {
    LabelsOnly,
    Generated {
        #[serde(rename = "generatedRevision")]
        generated_revision: u64,
    },
    Review {
        #[serde(rename = "reviewRevision")]
        review_revision: u64,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveMeetingReviewRequest {
    pub session_id: String,
    pub expected_review_revision: Option<u64>,
    pub base: ReviewEditBase,
    pub labels: MeetingSpeakerLabels,
    pub document: Option<EditableReviewDocument>,
    #[serde(default)]
    pub new_claims: Vec<NewReviewClaim>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingReviewExportFormat {
    Markdown,
    Text,
    Json,
    Srt,
    Vtt,
}

impl MeetingReviewExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Json => "json",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

fn valid_user_text(value: &str, maximum: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(|character| {
            character == '\0'
                || character.is_control()
                || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        }))
    .then(|| value.to_string())
}

pub fn validate_labels(labels: MeetingSpeakerLabels) -> Result<MeetingSpeakerLabels, String> {
    let me = valid_user_text(&labels.me, MAX_LABEL_BYTES).ok_or_else(|| {
        "Enter a shorter Me speaker label without control characters.".to_string()
    })?;
    let them = valid_user_text(&labels.them, MAX_LABEL_BYTES).ok_or_else(|| {
        "Enter a shorter Them speaker label without control characters.".to_string()
    })?;
    if me.to_lowercase() == them.to_lowercase() {
        return Err("Use different labels for Me and Them.".into());
    }
    Ok(MeetingSpeakerLabels { me, them })
}

pub fn validate_remote_speaker_label(label: &str) -> Result<String, String> {
    valid_user_text(label, MAX_LABEL_BYTES).ok_or_else(|| {
        "Enter a shorter remote speaker label without control characters.".to_string()
    })
}

fn speaker_label<'a>(workspace: &'a MeetingWorkspace, segment: &MeetingSegment) -> &'a str {
    match segment.speaker {
        MeetingSpeaker::Me => &workspace.labels.me,
        MeetingSpeaker::Them => segment
            .remote_speaker_id
            .and_then(|speaker_id| {
                workspace
                    .remote_speakers
                    .iter()
                    .find(|speaker| speaker.speaker_id == speaker_id)
            })
            .map(|speaker| speaker.label.as_str())
            .unwrap_or(&workspace.labels.them),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportMeetingSegment<'a> {
    #[serde(flatten)]
    segment: &'a MeetingSegment,
    display_label: &'a str,
}

fn timestamp(ms: u64) -> String {
    let seconds = ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub fn render_export(
    workspace: &MeetingWorkspace,
    format: MeetingReviewExportFormat,
) -> Result<String, String> {
    match format {
        MeetingReviewExportFormat::Srt => {
            return subtitles::render(workspace, subtitles::Format::Srt);
        }
        MeetingReviewExportFormat::Vtt => {
            return subtitles::render(workspace, subtitles::Format::Vtt);
        }
        MeetingReviewExportFormat::Markdown
        | MeetingReviewExportFormat::Text
        | MeetingReviewExportFormat::Json => {}
    }
    if format == MeetingReviewExportFormat::Json {
        let transcript = workspace
            .segments
            .iter()
            .map(|segment| ExportMeetingSegment {
                segment,
                display_label: speaker_label(workspace, segment),
            })
            .collect::<Vec<_>>();
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "schema": MEETING_REVIEW_EXPORT_SCHEMA,
            "session": workspace.session,
            "labels": workspace.labels,
            "remoteSpeakers": workspace.remote_speakers,
            "activeOrigin": null,
            "review": null,
            "transcript": transcript,
        }))
        .map(|json| format!("{json}\n"))
        .map_err(|_| "The meeting review could not be exported.".to_string())?;
        return bounded_export(output);
    }
    let markdown = format == MeetingReviewExportFormat::Markdown;
    let mut output = if markdown {
        "# Meeting review\n\n".to_string()
    } else {
        "MEETING REVIEW\n\n".to_string()
    };
    output.push_str(&format!(
        "Title: {}\n",
        workspace.session.title.as_deref().unwrap_or("None")
    ));
    if workspace.session.attendees.is_empty() {
        output.push_str("Attendees: None\n");
    } else {
        output.push_str("Attendees:\n");
        for attendee in &workspace.session.attendees {
            output.push_str(&format!("- {attendee}\n"));
        }
    }
    output.push('\n');
    output.push_str(&format!(
        "Me: {}\nThem: {}\n\n",
        workspace.labels.me, workspace.labels.them
    ));
    for speaker in &workspace.remote_speakers {
        output.push_str(&format!(
            "Speaker {}: {}\n",
            speaker.speaker_id, speaker.label
        ));
    }
    if !workspace.remote_speakers.is_empty() {
        output.push('\n');
    }
    output.push_str(if markdown {
        "## Transcript\n\n"
    } else {
        "TRANSCRIPT\n\n"
    });
    for segment in &workspace.segments {
        let label = speaker_label(workspace, segment);
        let status = match segment.status {
            MeetingSegmentStatus::Final => segment.text.as_str(),
            MeetingSegmentStatus::Pending => "[pending]",
            MeetingSegmentStatus::Failed => "[transcription failed]",
        };
        if markdown {
            output.push_str(&format!("<a id=\"segment-{}\"></a>\n", segment.id));
        }
        output.push_str(&format!(
            "[{}] {} [{}]: {}\n",
            timestamp(segment.start_ms),
            label,
            segment.speaker.as_db(),
            status
        ));
    }
    bounded_export(output)
}

fn bounded_export(output: String) -> Result<String, String> {
    if output.len() > MAX_EXPORT_BYTES {
        Err("The meeting review is too large to export.".into())
    } else {
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_base_wire_shape_is_explicit_and_labels_fail_closed() {
        let base: ReviewEditBase = serde_json::from_value(serde_json::json!({
            "kind": "generated",
            "generatedRevision": 7
        }))
        .unwrap();
        assert!(matches!(
            base,
            ReviewEditBase::Generated {
                generated_revision: 7
            }
        ));
        assert!(validate_labels(MeetingSpeakerLabels {
            me: "Same".into(),
            them: "same".into(),
        })
        .is_err());
        assert!(validate_labels(MeetingSpeakerLabels {
            me: "Me\nElse".into(),
            them: "Them".into(),
        })
        .is_err());
    }

    #[test]
    fn every_export_path_uses_the_same_hard_size_bound() {
        assert_eq!(
            bounded_export("a".repeat(MAX_EXPORT_BYTES)).unwrap().len(),
            MAX_EXPORT_BYTES
        );
        assert_eq!(
            bounded_export("a".repeat(MAX_EXPORT_BYTES + 1)).unwrap_err(),
            "The meeting review is too large to export."
        );
    }

    #[test]
    fn exports_resolve_remote_names_and_keep_channel_fallbacks() {
        let segment = |id: i64,
                       speaker: MeetingSpeaker,
                       remote_speaker_id: Option<u32>,
                       start_ms: u64,
                       text: &str| MeetingSegment {
            id,
            session_id: "meeting".into(),
            speaker,
            remote_speaker_id,
            sequence: id as u64,
            start_ms,
            end_ms: start_ms + 500,
            status: MeetingSegmentStatus::Final,
            text: text.into(),
            audio_available: false,
            error_code: None,
        };
        let workspace = MeetingWorkspace {
            session: MeetingSession {
                id: "meeting".into(),
                title: Some("Quarterly planning".into()),
                title_source: Some(crate::meeting_store::MeetingTitleSource::Manual),
                attendees: vec!["Casey".into(), "Morgan".into()],
                started_at_ms: 1,
                ended_at_ms: Some(2),
                status: crate::meeting_store::MeetingSessionStatus::Complete,
                model_name: "base.en".into(),
                language: "en".into(),
                smart_punctuation: true,
                retain_audio: false,
                duration_ms: 1,
                segment_count: 3,
                preview: "Mic".into(),
                error_code: None,
            },
            segments: vec![
                segment(1, MeetingSpeaker::Me, None, 0, "Mic"),
                segment(2, MeetingSpeaker::Them, Some(1), 1_000, "Remote"),
                segment(3, MeetingSpeaker::Them, None, 2_000, "Uncertain"),
            ],
            labels: MeetingSpeakerLabels {
                me: "George".into(),
                them: "Team".into(),
            },
            remote_speakers: vec![RemoteSpeakerLabel {
                speaker_id: 1,
                label: "Casey".into(),
            }],
            generated: None,
            review: None,
            active_document: None,
            active_origin: None,
        };

        let text = render_export(&workspace, MeetingReviewExportFormat::Text).unwrap();
        assert!(text.contains("Title: Quarterly planning"));
        assert!(text.contains("Attendees:\n- Casey\n- Morgan"));
        assert!(text.contains("[0:00] George [me]: Mic"));
        assert!(text.contains("[0:01] Casey [them]: Remote"));
        assert!(text.contains("[0:02] Team [them]: Uncertain"));
        let markdown = render_export(&workspace, MeetingReviewExportFormat::Markdown).unwrap();
        assert!(markdown.contains("Title: Quarterly planning"));
        assert!(markdown.contains("Attendees:\n- Casey\n- Morgan"));
        let json = render_export(&workspace, MeetingReviewExportFormat::Json).unwrap();
        assert!(json.contains(MEETING_REVIEW_EXPORT_SCHEMA));
        assert!(json.contains("\"title\": \"Quarterly planning\""));
        assert!(json.contains("\"attendees\": ["));
        assert!(json.contains("\"displayLabel\": \"Casey\""));
        assert!(json.contains("\"remoteSpeakers\""));
    }
}
