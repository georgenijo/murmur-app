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
            new_claims: Vec::new(),
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
            new_claims: Vec::new(),
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
            new_claims: Vec::new(),
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

#[test]
fn failed_index_insert_rolls_back_generated_content_and_previous_index_rows() {
    let (_root, repository) = repository();
    let source = session(&repository, "atomic-index", "ordinary evidence");
    repository
        .save_artifact("atomic-index", &artifact(source), 1, 1)
        .unwrap();
    let connection = repository.open_checked().unwrap();
    let before: (String, i64) = connection
        .query_row(
            "SELECT artifact_json, revision FROM meeting_artifacts WHERE session_id='atomic-index'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    // Same schema columns, but a deterministic insertion failure after DELETE.
    connection.execute_batch(
        "DROP TABLE meeting_review_fts;
         CREATE TABLE meeting_review_fts(session_id TEXT, field TEXT CHECK(field!='summary'), text TEXT);
         INSERT INTO meeting_review_fts VALUES('atomic-index','decision','previous index row');",
    ).unwrap();
    let mut replacement = artifact(source);
    replacement.summary.text = "replacement-only".into();
    let error = repository
        .save_artifact("atomic-index", &replacement, 1, 1)
        .unwrap_err();
    assert!(!error.contains("replacement-only"));
    assert_eq!(connection.query_row(
        "SELECT artifact_json, revision FROM meeting_artifacts WHERE session_id='atomic-index'", [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    ).unwrap(), before);
    assert_eq!(
        connection
            .query_row("SELECT text FROM meeting_review_fts", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "previous index row"
    );
}

fn new_claim_request(
    repository: &MeetingRepository,
    id: &str,
    sources: Vec<i64>,
) -> SaveMeetingReviewRequest {
    SaveMeetingReviewRequest {
        session_id: id.into(),
        expected_review_revision: None,
        base: ReviewEditBase::Generated {
            generated_revision: 1,
        },
        labels: MeetingSpeakerLabels::default(),
        document: Some(edit_from_generated(repository, id, "decision nebula")),
        new_claims: vec![crate::meeting_review::NewReviewClaim::ActionItem {
            text: "Verify zircon prototype".into(),
            owner: Some("Fixture owner".into()),
            due_date: Some("2026-10-04".into()),
            source_segment_ids: sources,
        }],
    }
}

fn editable_saved(
    document: &crate::meeting_review::MeetingReviewDocumentV1,
) -> EditableReviewDocument {
    EditableReviewDocument {
        summary: EditableReviewText {
            key: document.summary.key.clone(),
            text: document.summary.text.clone(),
        },
        decisions: document
            .decisions
            .iter()
            .map(|i| EditableReviewText {
                key: i.key.clone(),
                text: i.text.clone(),
            })
            .collect(),
        action_items: document
            .action_items
            .iter()
            .map(|i| EditableReviewAction {
                key: i.key.clone(),
                text: i.text.clone(),
                owner: i.owner.clone(),
                due_date: i.due_date.clone(),
            })
            .collect(),
        open_questions: document
            .open_questions
            .iter()
            .map(|i| EditableReviewText {
                key: i.key.clone(),
                text: i.text.clone(),
            })
            .collect(),
    }
}

#[test]
fn new_claims_persist_exact_sources_rehydrate_and_export_and_search_all_kinds() {
    use crate::meeting_review::{render_export, MeetingReviewExportFormat, NewReviewClaim};
    let (root, repository) = repository();
    let first = session(&repository, "new-claims", "Synthetic first passage");
    let relative = "audio/new-claims/me-0.wav";
    fs::write(repository.root().join(relative), b"synthetic").unwrap();
    let second = repository
        .insert_pending_segment("new-claims", MeetingSpeaker::Me, 0, 1_000, 1_500, relative)
        .unwrap();
    repository
        .finalize_segment(second, "Synthetic selected second passage", false)
        .unwrap();
    let relative = "audio/new-claims/me-1.wav";
    fs::write(repository.root().join(relative), b"synthetic").unwrap();
    let third = repository
        .insert_pending_segment("new-claims", MeetingSpeaker::Me, 1, 2_000, 2_500, relative)
        .unwrap();
    repository
        .finalize_segment(third, "Synthetic unselected passage", false)
        .unwrap();
    repository
        .finish_session("new-claims", MeetingSessionStatus::Complete, None)
        .unwrap();
    repository
        .save_artifact("new-claims", &artifact(first), 0, 0)
        .unwrap();
    let before = repository.workspace("new-claims").unwrap().segments;
    let mut request = new_claim_request(&repository, "new-claims", vec![second, first]);
    request.new_claims.push(NewReviewClaim::Decision {
        text: "Choose emerald prototype".into(),
        source_segment_ids: vec![second],
    });
    request.new_claims.push(NewReviewClaim::OpenQuestion {
        text: "When is heliotrope ready?".into(),
        source_segment_ids: vec![first, second],
    });
    let retry = request.clone();
    let saved = repository.save_review(request).unwrap();
    let document = saved.active_document.as_ref().unwrap();
    let action = document.action_items.last().unwrap();
    assert_eq!(action.source_segment_ids, vec![first, second]);
    assert_eq!(
        document.decisions.last().unwrap().source_segment_ids,
        vec![second]
    );
    assert_eq!(
        document.open_questions.last().unwrap().source_segment_ids,
        vec![first, second]
    );
    assert!(action.key.starts_with("claim:"));
    assert!(!action.key.contains("new-claims"));
    assert_ne!(action.key, document.decisions.last().unwrap().key);
    assert_eq!(
        serde_json::to_value(&saved.segments).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(repository
        .save_review(retry)
        .unwrap_err()
        .contains("changed"));
    for (word, field) in [
        ("zircon", MeetingSearchField::ActionItem),
        ("emerald", MeetingSearchField::Decision),
        ("heliotrope", MeetingSearchField::OpenQuestion),
    ] {
        assert_eq!(fields(&repository, word, "new-claims"), vec![field]);
    }
    let json: serde_json::Value =
        serde_json::from_str(&render_export(&saved, MeetingReviewExportFormat::Json).unwrap())
            .unwrap();
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(serialized.contains("Verify zircon prototype"));
    // Every format includes the selected references alongside the new claim.
    for format in [
        MeetingReviewExportFormat::Markdown,
        MeetingReviewExportFormat::Text,
    ] {
        let output = render_export(&saved, format).unwrap();
        let claim_line = output
            .lines()
            .find(|l| l.contains("Verify zircon prototype"))
            .unwrap();
        assert!(claim_line.contains(&format!("segments: {first}, {second}")));
        assert!(!claim_line.contains(&format!(", {third}")));
    }
    assert_eq!(
        json["review"]["actionItems"][1]["sourceSegmentIds"],
        serde_json::json!([first, second])
    );
    let (reopened, _) = MeetingRepository::initialize(root.path().canonicalize().unwrap()).unwrap();
    let reloaded = reopened.workspace("new-claims").unwrap();
    assert_eq!(reloaded.active_document.as_ref().unwrap(), document);
    let mut edit = editable_saved(document);
    edit.action_items.last_mut().unwrap().text = "Verify updated zircon prototype".into();
    let edited = reopened
        .save_review(SaveMeetingReviewRequest {
            session_id: "new-claims".into(),
            expected_review_revision: Some(1),
            base: ReviewEditBase::Review { review_revision: 1 },
            labels: reloaded.labels,
            document: Some(edit),
            new_claims: vec![],
        })
        .unwrap();
    let updated = edited.active_document.unwrap().action_items.pop().unwrap();
    assert_eq!(updated.key, action.key);
    assert_eq!(updated.source_segment_ids, vec![first, second]);
    assert_eq!(
        fields(&reopened, "updated zircon", "new-claims"),
        vec![MeetingSearchField::ActionItem]
    );
}

#[test]
fn new_claims_reject_empty_foreign_cross_session_pending_and_blank_sources_atomically() {
    let (_root, repository) = repository();
    let source = session(&repository, "claim-validation", "Synthetic source");
    let foreign = session(&repository, "other-meeting", "Synthetic foreign source");
    repository
        .save_artifact("claim-validation", &artifact(source), 0, 0)
        .unwrap();
    fs::write(
        repository.root().join("audio/claim-validation/me-0.wav"),
        b"synthetic",
    )
    .unwrap();
    let pending = repository
        .insert_pending_segment(
            "claim-validation",
            MeetingSpeaker::Me,
            0,
            1_000,
            1_500,
            "audio/claim-validation/me-0.wav",
        )
        .unwrap();
    fs::write(
        repository.root().join("audio/claim-validation/me-1.wav"),
        b"synthetic",
    )
    .unwrap();
    let blank = repository
        .insert_pending_segment(
            "claim-validation",
            MeetingSpeaker::Me,
            1,
            2_000,
            2_500,
            "audio/claim-validation/me-1.wav",
        )
        .unwrap();
    repository.finalize_segment(blank, " ", false).unwrap();
    for ids in [
        vec![],
        vec![999999],
        vec![foreign],
        vec![source, foreign],
        vec![pending],
        vec![blank],
    ] {
        let mut request = new_claim_request(&repository, "claim-validation", ids);
        request.labels.me = "Changed label".into();
        assert!(repository
            .save_review(request)
            .unwrap_err()
            .contains("source"));
        let workspace = repository.workspace("claim-validation").unwrap();
        assert!(workspace.review.is_none());
        assert_eq!(workspace.labels, MeetingSpeakerLabels::default());
        assert!(fields(&repository, "zircon", "claim-validation").is_empty());
    }
    let mut request = new_claim_request(&repository, "claim-validation", vec![source]);
    request.base = ReviewEditBase::LabelsOnly;
    request.document = None;
    assert!(repository
        .save_review(request)
        .unwrap_err()
        .contains("editable"));
}

#[test]
fn new_claim_keys_cannot_be_forged_rebound_duplicated_or_moved_between_sections() {
    let (_root, repository) = repository();
    let source = session(&repository, "opaque-claims", "Synthetic source");
    repository
        .save_artifact("opaque-claims", &artifact(source), 0, 0)
        .unwrap();
    let saved = repository
        .save_review(new_claim_request(
            &repository,
            "opaque-claims",
            vec![source],
        ))
        .unwrap();
    let document = saved.active_document.unwrap();
    let action = document.action_items.last().unwrap();
    let base_request = SaveMeetingReviewRequest {
        session_id: "opaque-claims".into(),
        expected_review_revision: Some(1),
        base: ReviewEditBase::Review { review_revision: 1 },
        labels: saved.labels,
        document: Some(editable_saved(&document)),
        new_claims: vec![],
    };
    for variant in 0..3 {
        let mut request = base_request.clone();
        let edit = request.document.as_mut().unwrap();
        match variant {
            0 => edit.action_items.last_mut().unwrap().key = "claim:forged".into(),
            1 => edit
                .action_items
                .push(edit.action_items.last().unwrap().clone()),
            _ => edit.decisions.push(EditableReviewText {
                key: action.key.clone(),
                text: "Moved claim".into(),
            }),
        }
        assert!(repository
            .save_review(request)
            .unwrap_err()
            .contains("changed"));
        assert_eq!(
            repository
                .workspace("opaque-claims")
                .unwrap()
                .review
                .unwrap()
                .revision,
            1
        );
    }
    assert!(serde_json::from_value::<crate::meeting_review::NewReviewClaim>(serde_json::json!({
        "kind":"action_item", "key":action.key, "text":"Forged mapping", "owner":null, "dueDate":null, "sourceSegmentIds":[source]
    })).is_err());
    assert!(serde_json::from_value::<EditableReviewAction>(serde_json::json!({
        "key":action.key, "text":"Forged mapping", "owner":null, "dueDate":null, "sourceSegmentIds":[source]
    })).is_err());
    let mut stale = base_request.clone();
    stale.expected_review_revision = None;
    stale.new_claims = new_claim_request(&repository, "opaque-claims", vec![source]).new_claims;
    assert!(repository
        .save_review(stale)
        .unwrap_err()
        .contains("changed"));
    let mut stale = new_claim_request(&repository, "opaque-claims", vec![source]);
    let source2 = session(&repository, "different-session", "Other evidence");
    repository
        .save_artifact("different-session", &artifact(source2), 0, 0)
        .unwrap();
    stale.session_id = "different-session".into();
    assert!(repository.save_review(stale).is_err());
}

#[test]
fn new_claim_creation_rejects_a_stale_generated_base_and_keeps_draft_and_index() {
    let (_root, repository) = repository();
    let source = session(&repository, "stale-new-claim", "Synthetic source");
    repository
        .save_artifact("stale-new-claim", &artifact(source), 0, 0)
        .unwrap();
    let request = new_claim_request(&repository, "stale-new-claim", vec![source]);
    let mut regenerated = artifact(source);
    regenerated.summary.text = "Fresh generated ivory".into();
    repository
        .save_artifact("stale-new-claim", &regenerated, 0, 0)
        .unwrap();
    assert!(repository
        .save_review(request)
        .unwrap_err()
        .contains("generated draft changed"));
    let workspace = repository.workspace("stale-new-claim").unwrap();
    assert!(workspace.review.is_none());
    assert_eq!(workspace.generated.unwrap().revision, 2);
    assert_eq!(
        fields(&repository, "ivory", "stale-new-claim"),
        vec![MeetingSearchField::Summary]
    );
    assert!(fields(&repository, "zircon", "stale-new-claim").is_empty());
}
