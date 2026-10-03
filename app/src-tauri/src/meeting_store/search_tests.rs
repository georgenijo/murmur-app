//! Search lifecycle coverage for the repository's active meeting document.
//! Synthetic transcripts and review content only.

use super::*;
use crate::meeting_artifact::{
    MeetingActionItem, MeetingArtifactV1, SourcedMeetingText, MEETING_ARTIFACT_SCHEMA,
};
use crate::meeting_review::{
    EditableReviewAction, EditableReviewDocument, EditableReviewText, MeetingSpeakerLabels,
    RestoreMeetingReviewRequest, ReviewEditBase, SaveMeetingReviewRequest,
};
use std::fs;
use tempfile::TempDir;

fn repository() -> (TempDir, MeetingRepository) {
    let root = TempDir::new().unwrap();
    let (repository, _) =
        MeetingRepository::initialize(root.path().canonicalize().unwrap()).unwrap();
    (root, repository)
}

fn session(repository: &MeetingRepository, id: &str, transcript: &str) -> i64 {
    repository
        .create_session(id, "base.en", "en", true, false)
        .unwrap();
    let relative = format!("audio/{id}/them-0.wav");
    let audio = repository.root().join(&relative);
    fs::create_dir_all(audio.parent().unwrap()).unwrap();
    fs::write(&audio, b"synthetic").unwrap();
    let segment = repository
        .insert_pending_segment(id, MeetingSpeaker::Them, 0, 0, 500, &relative)
        .unwrap();
    repository
        .finalize_segment(segment, transcript, false)
        .unwrap();
    fs::remove_file(audio).unwrap();
    segment
}

fn artifact(source: i64) -> MeetingArtifactV1 {
    MeetingArtifactV1 {
        schema: MEETING_ARTIFACT_SCHEMA.into(),
        summary: SourcedMeetingText {
            text: "summary quasar".into(),
            source_segment_ids: vec![source],
        },
        decisions: vec![SourcedMeetingText {
            text: "decision nebula".into(),
            source_segment_ids: vec![source],
        }],
        action_items: vec![MeetingActionItem {
            text: "action orion".into(),
            owner: None,
            due_date: None,
            source_segment_ids: vec![source],
        }],
        open_questions: vec![SourcedMeetingText {
            text: "question pulsar".into(),
            source_segment_ids: vec![source],
        }],
    }
}

fn fields(repository: &MeetingRepository, query: &str, id: &str) -> Vec<MeetingSearchField> {
    repository
        .list_sessions(Some(query), 0, 10)
        .unwrap()
        .search_matches
        .get(id)
        .cloned()
        .unwrap_or_default()
}

fn edit_from_generated(
    repository: &MeetingRepository,
    id: &str,
    decision_text: &str,
) -> EditableReviewDocument {
    let generated = repository
        .workspace(id)
        .unwrap()
        .generated
        .unwrap()
        .document;
    EditableReviewDocument {
        summary: EditableReviewText {
            key: generated.summary.key,
            text: generated.summary.text,
        },
        decisions: generated
            .decisions
            .iter()
            .map(|item| EditableReviewText {
                key: item.key.clone(),
                text: decision_text.into(),
            })
            .collect(),
        action_items: generated
            .action_items
            .iter()
            .map(|item| EditableReviewAction {
                key: item.key.clone(),
                text: item.text.clone(),
                owner: item.owner.clone(),
                due_date: item.due_date.clone(),
            })
            .collect(),
        open_questions: generated
            .open_questions
            .iter()
            .map(|item| EditableReviewText {
                key: item.key.clone(),
                text: item.text.clone(),
            })
            .collect(),
    }
}

#[test]
fn generated_artifact_indexes_each_review_field_and_reports_field_labels() {
    let (_root, repository) = repository();
    let source = session(&repository, "generated", "ordinary transcript words");
    repository
        .save_artifact("generated", &artifact(source), 1, 1)
        .unwrap();
    repository
        .save_metadata(crate::meeting_store::SaveMeetingMetadataRequest {
            session_id: "generated".into(),
            title: Some("Title zephyr".into()),
            attendees: vec![],
        })
        .unwrap();

    for (query, expected) in [
        ("quasar", MeetingSearchField::Summary),
        ("nebula", MeetingSearchField::Decision),
        ("orion", MeetingSearchField::ActionItem),
        ("pulsar", MeetingSearchField::OpenQuestion),
        ("ordinary", MeetingSearchField::Transcript),
        ("zephyr", MeetingSearchField::Title),
    ] {
        assert_eq!(
            fields(&repository, query, "generated"),
            vec![expected],
            "query {query}"
        );
    }
}

#[test]
fn labels_only_review_keeps_generated_fallback_searchable() {
    let (_root, repository) = repository();
    let source = session(&repository, "labels-only", "transcript cobalt");
    repository
        .save_artifact("labels-only", &artifact(source), 1, 1)
        .unwrap();
    repository
        .save_review(SaveMeetingReviewRequest {
            session_id: "labels-only".into(),
            expected_review_revision: None,
            base: ReviewEditBase::LabelsOnly,
            labels: MeetingSpeakerLabels {
                me: "Host".into(),
                them: "Guests".into(),
            },
            document: None,
        })
        .unwrap();

    assert_eq!(
        fields(&repository, "quasar", "labels-only"),
        vec![MeetingSearchField::Summary]
    );
    assert_eq!(
        fields(&repository, "cobalt", "labels-only"),
        vec![MeetingSearchField::Transcript]
    );
}

#[test]
fn review_overrides_draft_reindex_hides_old_generated_content_and_restore_activates_new_draft() {
    let (_root, repository) = repository();
    let source = session(&repository, "active-review", "transcript amber");
    repository
        .save_artifact("active-review", &artifact(source), 1, 1)
        .unwrap();
    repository
        .save_review(SaveMeetingReviewRequest {
            session_id: "active-review".into(),
            expected_review_revision: None,
            base: ReviewEditBase::Generated {
                generated_revision: 1,
            },
            labels: MeetingSpeakerLabels::default(),
            document: Some(edit_from_generated(
                &repository,
                "active-review",
                "reviewed jade",
            )),
        })
        .unwrap();
    assert_eq!(
        fields(&repository, "jade", "active-review"),
        vec![MeetingSearchField::Decision]
    );
    assert!(fields(&repository, "nebula", "active-review").is_empty());

    let mut replacement = artifact(source);
    replacement.summary.text = "replacement garnet".into();
    replacement.decisions[0].text = "replacement sapphire".into();
    repository
        .save_artifact("active-review", &replacement, 2, 2)
        .unwrap();
    assert!(
        fields(&repository, "sapphire", "active-review").is_empty(),
        "a saved review remains active after draft regeneration"
    );
    assert_eq!(
        fields(&repository, "jade", "active-review"),
        vec![MeetingSearchField::Decision]
    );

    repository
        .restore_review_from_generated(RestoreMeetingReviewRequest {
            session_id: "active-review".into(),
            generated_revision: 2,
            expected_review_revision: Some(1),
        })
        .unwrap();
    assert_eq!(
        fields(&repository, "sapphire", "active-review"),
        vec![MeetingSearchField::Decision]
    );
    assert!(fields(&repository, "jade", "active-review").is_empty());
}

#[test]
fn edited_claim_removal_and_page_totals_are_stable_and_deduplicated() {
    let (_root, repository) = repository();
    let first = session(&repository, "page-first", "transcript sharedword");
    let second = session(&repository, "page-second", "transcript sharedword");
    repository
        .save_artifact("page-first", &artifact(first), 1, 1)
        .unwrap();
    repository
        .save_artifact("page-second", &artifact(second), 1, 1)
        .unwrap();
    let mut document = edit_from_generated(&repository, "page-first", "decision withdrawn");
    document.summary.text = "sharedword summary also matches".into();
    repository
        .save_review(SaveMeetingReviewRequest {
            session_id: "page-first".into(),
            expected_review_revision: None,
            base: ReviewEditBase::Generated {
                generated_revision: 1,
            },
            labels: MeetingSpeakerLabels::default(),
            document: Some(document),
        })
        .unwrap();

    let first_page = repository.list_sessions(Some("sharedword"), 0, 1).unwrap();
    let next_page = repository.list_sessions(Some("sharedword"), 1, 1).unwrap();
    assert_eq!(first_page.total, 2);
    assert_eq!(next_page.total, 2);
    assert_eq!(first_page.sessions.len(), 1);
    assert_eq!(next_page.sessions.len(), 1);
    assert_ne!(first_page.sessions[0].id, next_page.sessions[0].id);
    assert_eq!(first_page.search_matches.len(), 1);
    assert_eq!(next_page.search_matches.len(), 1);
    assert_eq!(
        fields(&repository, "withdrawn", "page-first"),
        vec![MeetingSearchField::Decision]
    );
    assert!(
        fields(&repository, "nebula", "page-first").is_empty(),
        "an edited-out generated claim must disappear"
    );
}

#[test]
fn literal_unicode_queries_and_session_deletion_clean_search_results() {
    let (_root, repository) = repository();
    let first = session(&repository, "unicode-first", "transcript café 猫");
    let second = session(&repository, "unicode-second", "unrelated transcript");
    repository
        .save_artifact("unicode-first", &artifact(first), 1, 1)
        .unwrap();
    repository
        .save_artifact("unicode-second", &artifact(second), 1, 1)
        .unwrap();
    let mut multilingual = artifact(first);
    multilingual.summary.text = "café 猫 literal * query".into();
    repository
        .save_artifact("unicode-first", &multilingual, 2, 2)
        .unwrap();

    assert_eq!(
        fields(&repository, "café", "unicode-first"),
        vec![MeetingSearchField::Summary, MeetingSearchField::Transcript]
    );
    assert_eq!(
        fields(&repository, "猫", "unicode-first"),
        vec![MeetingSearchField::Summary, MeetingSearchField::Transcript]
    );
    // Punctuation is quoted by the FTS query builder; it must not become an operator.
    assert_eq!(
        fields(&repository, "literal*", "unicode-first"),
        vec![MeetingSearchField::Summary]
    );
    repository.delete_session("unicode-first").unwrap();
    assert!(repository
        .list_sessions(Some("café"), 0, 10)
        .unwrap()
        .sessions
        .is_empty());
    assert!(repository
        .list_sessions(Some("café"), 0, 10)
        .unwrap()
        .search_matches
        .is_empty());
    assert_eq!(
        repository
            .list_sessions(Some("unrelated"), 0, 10)
            .unwrap()
            .total,
        1
    );

    repository.delete_all().unwrap();
    assert!(repository
        .list_sessions(None, 0, 10)
        .unwrap()
        .sessions
        .is_empty());
    assert!(repository
        .list_sessions(Some("unrelated"), 0, 10)
        .unwrap()
        .search_matches
        .is_empty());
}
