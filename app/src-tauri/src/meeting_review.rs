use crate::meeting_artifact::{MeetingArtifactV1, SourcedMeetingText};
use crate::meeting_store::{MeetingSegment, MeetingSegmentStatus, MeetingSession, MeetingSpeaker};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const MEETING_REVIEW_SCHEMA: &str = "murmur.meeting-review.v1";
pub const MEETING_REVIEW_EXPORT_SCHEMA: &str = "murmur.meeting-review-export.v1";
const MAX_LABEL_BYTES: usize = 80;
const MAX_TEXT_BYTES: usize = 16_384;
const MAX_ITEMS: usize = 200;
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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreMeetingReviewRequest {
    pub session_id: String,
    pub generated_revision: u64,
    pub expected_review_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingReviewExportFormat {
    Markdown,
    Text,
    Json,
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

fn valid_review_text(value: &str, maximum: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(|character| {
            character == '\0'
                || (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
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

fn review_text(key: String, item: &SourcedMeetingText) -> ReviewText {
    ReviewText {
        key,
        text: item.text.clone(),
        source_segment_ids: item.source_segment_ids.clone(),
    }
}

pub fn document_from_artifact(artifact: &MeetingArtifactV1) -> MeetingReviewDocumentV1 {
    MeetingReviewDocumentV1 {
        schema: MEETING_REVIEW_SCHEMA.into(),
        summary: review_text("summary".into(), &artifact.summary),
        decisions: artifact
            .decisions
            .iter()
            .enumerate()
            .map(|(index, item)| review_text(format!("decision:{index}"), item))
            .collect(),
        action_items: artifact
            .action_items
            .iter()
            .enumerate()
            .map(|(index, item)| ReviewAction {
                key: format!("action:{index}"),
                text: item.text.clone(),
                owner: item.owner.clone(),
                due_date: item.due_date.clone(),
                source_segment_ids: item.source_segment_ids.clone(),
            })
            .collect(),
        open_questions: artifact
            .open_questions
            .iter()
            .enumerate()
            .map(|(index, item)| review_text(format!("question:{index}"), item))
            .collect(),
    }
}

fn valid_sources(ids: &mut Vec<i64>, allowed: &HashSet<i64>) -> bool {
    if ids.is_empty() || ids.len() > MAX_ITEMS || !ids.iter().all(|id| allowed.contains(id)) {
        return false;
    }
    ids.sort_unstable();
    ids.dedup();
    true
}

pub fn validate_document(document: &mut MeetingReviewDocumentV1, allowed: &HashSet<i64>) -> bool {
    if document.schema != MEETING_REVIEW_SCHEMA
        || document.decisions.len() > MAX_ITEMS
        || document.action_items.len() > MAX_ITEMS
        || document.open_questions.len() > MAX_ITEMS
    {
        return false;
    }
    let mut keys = HashSet::new();
    let validate_text = |item: &mut ReviewText, keys: &mut HashSet<String>| {
        let Some(text) = valid_review_text(&item.text, MAX_TEXT_BYTES) else {
            return false;
        };
        if item.key.is_empty()
            || item.key.len() > 128
            || !keys.insert(item.key.clone())
            || !valid_sources(&mut item.source_segment_ids, allowed)
        {
            return false;
        }
        item.text = text;
        true
    };
    if !validate_text(&mut document.summary, &mut keys)
        || document
            .decisions
            .iter_mut()
            .chain(document.open_questions.iter_mut())
            .any(|item| !validate_text(item, &mut keys))
    {
        return false;
    }
    for item in &mut document.action_items {
        let Some(text) = valid_review_text(&item.text, MAX_TEXT_BYTES) else {
            return false;
        };
        if item.key.is_empty()
            || item.key.len() > 128
            || !keys.insert(item.key.clone())
            || !valid_sources(&mut item.source_segment_ids, allowed)
            || item
                .owner
                .as_ref()
                .is_some_and(|owner| valid_user_text(owner, 256).is_none())
            || item.due_date.as_ref().is_some_and(|date| {
                date.len() != 10
                    || !date.bytes().enumerate().all(|(index, byte)| {
                        matches!(index, 4 | 7)
                            .then_some(byte == b'-')
                            .unwrap_or(byte.is_ascii_digit())
                    })
            })
        {
            return false;
        }
        item.text = text;
        item.owner = item
            .owner
            .take()
            .map(|owner| valid_user_text(&owner, 256).expect("owner validated above"));
    }
    true
}

fn rebuild_text(
    inputs: Vec<EditableReviewText>,
    base: &[ReviewText],
) -> Result<Vec<ReviewText>, String> {
    let by_key = base
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    inputs
        .into_iter()
        .map(|input| {
            let source = by_key
                .get(input.key.as_str())
                .filter(|_| seen.insert(input.key.clone()))
                .ok_or_else(|| {
                    "The review changed from the version being edited. Reload it and try again."
                        .to_string()
                })?;
            let text = valid_review_text(&input.text, MAX_TEXT_BYTES)
                .ok_or_else(|| "Review text must be non-empty and bounded.".to_string())?;
            Ok(ReviewText {
                key: input.key,
                text,
                source_segment_ids: source.source_segment_ids.clone(),
            })
        })
        .collect()
}

pub fn apply_edit(
    base: &MeetingReviewDocumentV1,
    edit: EditableReviewDocument,
    allowed: &HashSet<i64>,
) -> Result<MeetingReviewDocumentV1, String> {
    if edit.summary.key != base.summary.key {
        return Err(
            "The review changed from the version being edited. Reload it and try again.".into(),
        );
    }
    let summary_text = valid_review_text(&edit.summary.text, MAX_TEXT_BYTES)
        .ok_or_else(|| "Review text must be non-empty and bounded.".to_string())?;
    let action_base = base
        .action_items
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut action_keys = HashSet::new();
    let mut actions = Vec::with_capacity(edit.action_items.len());
    for input in edit.action_items {
        let source = action_base
            .get(input.key.as_str())
            .filter(|_| action_keys.insert(input.key.clone()))
            .ok_or_else(|| {
                "The review changed from the version being edited. Reload it and try again."
                    .to_string()
            })?;
        let owner = input
            .owner
            .map(|owner| {
                valid_user_text(&owner, 256)
                    .ok_or_else(|| "Action owners must be bounded text.".to_string())
            })
            .transpose()?;
        actions.push(ReviewAction {
            key: input.key,
            text: valid_review_text(&input.text, MAX_TEXT_BYTES)
                .ok_or_else(|| "Review text must be non-empty and bounded.".to_string())?,
            owner,
            due_date: input.due_date,
            source_segment_ids: source.source_segment_ids.clone(),
        });
    }
    let mut document = MeetingReviewDocumentV1 {
        schema: MEETING_REVIEW_SCHEMA.into(),
        summary: ReviewText {
            key: edit.summary.key,
            text: summary_text,
            source_segment_ids: base.summary.source_segment_ids.clone(),
        },
        decisions: rebuild_text(edit.decisions, &base.decisions)?,
        action_items: actions,
        open_questions: rebuild_text(edit.open_questions, &base.open_questions)?,
    };
    validate_document(&mut document, allowed)
        .then_some(document)
        .ok_or_else(|| "The reviewed meeting is invalid.".to_string())
}

fn speaker_label(labels: &MeetingSpeakerLabels, speaker: MeetingSpeaker) -> &str {
    match speaker {
        MeetingSpeaker::Me => &labels.me,
        MeetingSpeaker::Them => &labels.them,
    }
}

fn timestamp(ms: u64) -> String {
    let seconds = ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn sources(item: &ReviewText) -> String {
    item.source_segment_ids
        .iter()
        .map(|id| format!("segment {id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn render_export(
    workspace: &MeetingWorkspace,
    format: MeetingReviewExportFormat,
) -> Result<String, String> {
    if format == MeetingReviewExportFormat::Json {
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "schema": MEETING_REVIEW_EXPORT_SCHEMA,
            "session": workspace.session,
            "labels": workspace.labels,
            "activeOrigin": workspace.active_origin,
            "review": workspace.active_document,
            "transcript": workspace.segments,
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
        "Me: {}\nThem: {}\n\n",
        workspace.labels.me, workspace.labels.them
    ));
    if let Some(document) = workspace.active_document.as_ref() {
        let heading = |title: &str| {
            if markdown {
                format!("## {title}\n")
            } else {
                format!("{}\n", title.to_uppercase())
            }
        };
        output.push_str(&heading("Summary"));
        output.push_str(&format!(
            "{}\nSources: {}\n\n",
            document.summary.text,
            sources(&document.summary)
        ));
        for (title, items) in [
            ("Decisions", &document.decisions),
            ("Open questions", &document.open_questions),
        ] {
            output.push_str(&heading(title));
            if items.is_empty() {
                output.push_str("None recorded.\n");
            }
            for item in items {
                output.push_str(&format!("- {} ({})\n", item.text, sources(item)));
            }
            output.push('\n');
        }
        output.push_str(&heading("Action items"));
        if document.action_items.is_empty() {
            output.push_str("None recorded.\n");
        }
        for item in &document.action_items {
            output.push_str(&format!(
                "- {} — Owner: {}; Due: {} (segments: {})\n",
                item.text,
                item.owner.as_deref().unwrap_or("Unknown"),
                item.due_date.as_deref().unwrap_or("Unknown"),
                item.source_segment_ids
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        output.push('\n');
    } else {
        output.push_str(if markdown {
            "## Review\nNo review generated.\n\n"
        } else {
            "REVIEW\nNo review generated.\n\n"
        });
    }
    output.push_str(if markdown {
        "## Transcript\n\n"
    } else {
        "TRANSCRIPT\n\n"
    });
    for segment in &workspace.segments {
        let label = speaker_label(&workspace.labels, segment.speaker);
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
    fn edit_rehydrates_sources_and_rejects_unknown_keys() {
        let artifact = MeetingArtifactV1 {
            schema: crate::meeting_artifact::MEETING_ARTIFACT_SCHEMA.into(),
            summary: SourcedMeetingText {
                text: "Summary".into(),
                source_segment_ids: vec![1],
            },
            decisions: vec![SourcedMeetingText {
                text: "Decision".into(),
                source_segment_ids: vec![2],
            }],
            action_items: Vec::new(),
            open_questions: Vec::new(),
        };
        let base = document_from_artifact(&artifact);
        let edit = EditableReviewDocument {
            summary: EditableReviewText {
                key: "summary".into(),
                text: "Edited summary".into(),
            },
            decisions: vec![EditableReviewText {
                key: "decision:0".into(),
                text: "Edited decision".into(),
            }],
            action_items: Vec::new(),
            open_questions: Vec::new(),
        };
        let saved = apply_edit(&base, edit, &HashSet::from([1, 2])).unwrap();
        assert_eq!(saved.decisions[0].source_segment_ids, vec![2]);
        let forged = EditableReviewDocument {
            summary: EditableReviewText {
                key: "summary".into(),
                text: "Summary".into(),
            },
            decisions: vec![EditableReviewText {
                key: "invented".into(),
                text: "No".into(),
            }],
            action_items: Vec::new(),
            open_questions: Vec::new(),
        };
        assert!(apply_edit(&base, forged, &HashSet::from([1, 2])).is_err());
    }

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
    fn review_text_preserves_multiline_edits_but_rejects_hidden_controls() {
        assert_eq!(
            valid_review_text("First line\nSecond line", MAX_TEXT_BYTES).as_deref(),
            Some("First line\nSecond line")
        );
        assert!(valid_review_text("visible\u{0007}hidden", MAX_TEXT_BYTES).is_none());
    }
}
