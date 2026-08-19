use crate::meeting_store::MeetingSegment;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MEETING_ARTIFACT_SCHEMA: &str = "murmur.meeting-artifact.v1";
const MAX_ITEMS: usize = 200;
const MAX_TEXT_BYTES: usize = 16_384;
const CHUNK_CHARS: usize = 12_000;
const CHUNK_SEGMENTS: usize = 50;
const MAX_SOURCE_IDS: usize = 200;

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

fn valid_text(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES && !text.contains('\0')
}

fn valid_sources(ids: &[i64], allowed: &HashSet<i64>) -> bool {
    !ids.is_empty()
        && ids.len() <= MAX_SOURCE_IDS
        && ids.iter().all(|id| *id > 0 && allowed.contains(id))
}

pub fn validate_artifact(artifact: &mut MeetingArtifactV1, allowed: &HashSet<i64>) -> bool {
    if artifact.schema != MEETING_ARTIFACT_SCHEMA
        || !valid_text(&artifact.summary.text)
        || !valid_sources(&artifact.summary.source_segment_ids, allowed)
        || artifact.decisions.len() > MAX_ITEMS
        || artifact.action_items.len() > MAX_ITEMS
        || artifact.open_questions.len() > MAX_ITEMS
    {
        return false;
    }
    artifact.summary.text = artifact.summary.text.trim().to_string();
    artifact.summary.source_segment_ids.sort_unstable();
    artifact.summary.source_segment_ids.dedup();
    for item in artifact
        .decisions
        .iter_mut()
        .chain(artifact.open_questions.iter_mut())
    {
        if !valid_text(&item.text) || !valid_sources(&item.source_segment_ids, allowed) {
            return false;
        }
        item.text = item.text.trim().to_string();
        item.source_segment_ids.sort_unstable();
        item.source_segment_ids.dedup();
    }
    for item in &mut artifact.action_items {
        if !valid_text(&item.text) || !valid_sources(&item.source_segment_ids, allowed) {
            return false;
        }
        if item
            .owner
            .as_ref()
            .is_some_and(|owner| owner.len() > 256 || owner.contains('\0'))
            || item.due_date.as_ref().is_some_and(|date| {
                date.len() != 10
                    || !date.bytes().enumerate().all(|(index, byte)| {
                        if index == 4 || index == 7 {
                            byte == b'-'
                        } else {
                            byte.is_ascii_digit()
                        }
                    })
            })
        {
            return false;
        }
        item.text = item.text.trim().to_string();
        item.owner = item.owner.take().and_then(|owner| {
            let owner = owner.trim().to_string();
            (!owner.is_empty()).then_some(owner)
        });
        item.source_segment_ids.sort_unstable();
        item.source_segment_ids.dedup();
    }
    true
}

pub fn parse_artifact(output: &str, allowed: &HashSet<i64>) -> Option<MeetingArtifactV1> {
    let output = output.trim();
    let start = output.find('{')?;
    let candidate = output.get(start..)?;
    let mut values = serde_json::Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
    let mut value = values.next()?.ok()?;
    for field in ["decisions", "openQuestions"] {
        if let Some(items) = value
            .get_mut(field)
            .and_then(serde_json::Value::as_array_mut)
        {
            for item in items {
                if let Some(object) = item.as_object_mut() {
                    for optional in ["owner", "dueDate"] {
                        if object.get(optional).is_some_and(serde_json::Value::is_null) {
                            object.remove(optional);
                        }
                    }
                }
            }
        }
    }
    let mut artifact: MeetingArtifactV1 = serde_json::from_value(value).ok()?;
    validate_artifact(&mut artifact, allowed).then_some(artifact)
}

pub fn chunk_segments(segments: &[MeetingSegment]) -> Vec<Vec<MeetingSegment>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut chars = 0usize;
    for segment in segments
        .iter()
        .filter(|segment| !segment.text.trim().is_empty())
    {
        let size = segment.text.len().min(MAX_TEXT_BYTES);
        if !current.is_empty()
            && (current.len() >= CHUNK_SEGMENTS || chars.saturating_add(size) > CHUNK_CHARS)
        {
            chunks.push(std::mem::take(&mut current));
            chars = 0;
        }
        current.push(segment.clone());
        chars = chars.saturating_add(size);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn dedupe<T>(items: Vec<T>, text: impl Fn(&T) -> &str) -> Vec<T> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(text(item).to_lowercase()))
        .take(MAX_ITEMS)
        .collect()
}

pub fn merge_artifacts(artifacts: Vec<MeetingArtifactV1>) -> Option<MeetingArtifactV1> {
    let first = artifacts.first()?;
    let mut summary_ids = artifacts
        .iter()
        .flat_map(|artifact| artifact.summary.source_segment_ids.iter().copied())
        .collect::<Vec<_>>();
    summary_ids.sort_unstable();
    summary_ids.dedup();
    summary_ids.truncate(MAX_SOURCE_IDS);
    Some(MeetingArtifactV1 {
        schema: first.schema.clone(),
        summary: SourcedMeetingText {
            text: artifacts
                .iter()
                .map(|artifact| artifact.summary.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_TEXT_BYTES)
                .collect(),
            source_segment_ids: summary_ids,
        },
        decisions: dedupe(
            artifacts.iter().flat_map(|a| a.decisions.clone()).collect(),
            |x| &x.text,
        ),
        action_items: dedupe(
            artifacts
                .iter()
                .flat_map(|a| a.action_items.clone())
                .collect(),
            |x| &x.text,
        ),
        open_questions: dedupe(
            artifacts
                .into_iter()
                .flat_map(|a| a.open_questions)
                .collect(),
            |x| &x.text,
        ),
    })
}

pub fn render_chunk_input(segments: &[MeetingSegment]) -> String {
    segments
        .iter()
        .map(|segment| {
            format!(
                "[segment {}] {}: {}",
                segment.id,
                segment.speaker.as_db(),
                segment.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub const SUMMARY_INSTRUCTION: &str = r#"Return one line of JSON and stop after the final }. Use this exact shape: {"schema":"murmur.meeting-artifact.v1","summary":{"text":"...","sourceSegmentIds":[1]},"decisions":[],"actionItems":[{"text":"...","owner":null,"dueDate":null,"sourceSegmentIds":[1]}],"openQuestions":[]}. Use at most 80 words total and at most 3 items per array. Use only supplied segment IDs. Every claim needs supporting IDs. Never guess owners or dates; use null. No markdown, commentary, repetition, or extra keys."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_unknown_sources_and_accepts_exact_json() {
        let allowed = HashSet::from([1]);
        let json = r#"{"schema":"murmur.meeting-artifact.v1","summary":{"text":"Done","sourceSegmentIds":[1]},"decisions":[],"actionItems":[],"openQuestions":[]}"#;
        assert!(parse_artifact(json, &allowed).is_some());
        assert!(parse_artifact(&format!("```json\n{json}\n```"), &allowed).is_some());
        let compatible = json.replace(
            r#""openQuestions":[]"#,
            r#""openQuestions":[{"text":"Pending","owner":null,"dueDate":null,"sourceSegmentIds":[1]}]"#,
        );
        assert!(parse_artifact(&compatible, &allowed).is_some());
        assert!(parse_artifact(
            &compatible.replace("\"owner\":null", "\"owner\":\"guess\""),
            &allowed
        )
        .is_none());
        assert!(parse_artifact(&json.replace("[1]", "[2]"), &allowed).is_none());
    }
}
