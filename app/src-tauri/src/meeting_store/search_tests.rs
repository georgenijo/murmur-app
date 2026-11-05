//! Transcript search and upgrade compatibility with legacy derived rows.
use super::*;
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

#[test]
fn legacy_derived_rows_are_ignored_and_label_updates_preserve_them() {
    let (_root, repository) = repository();
    session(&repository, "legacy", "Synthetic transcript evidence");
    let connection = repository.open_checked().unwrap();
    connection.execute("INSERT INTO meeting_artifacts(session_id, artifact_json, created_at_ms, runtime_ms, peak_rss_mb, revision) VALUES('legacy','invalid legacy json',1,1,1,1)", []).unwrap();
    connection.execute("INSERT INTO meeting_reviews(session_id,revision,based_on_artifact_revision,me_label,them_label,review_json,updated_at_ms) VALUES('legacy',1,1,'Me','Them','invalid legacy review',1)", []).unwrap();
    connection.execute("INSERT INTO meeting_review_fts(session_id,field,text) VALUES('legacy','summary','hiddenquasar')", []).unwrap();
    let before = fs::read(repository.root().join("meetings.sqlite3")).unwrap();
    let workspace = repository.workspace("legacy").unwrap();
    assert_eq!(workspace.segments.len(), 1);
    assert!(workspace.generated.is_none());
    assert!(workspace.active_document.is_none());
    assert_eq!(
        repository
            .list_sessions(Some("hiddenquasar"), 0, 10)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        repository
            .list_sessions(Some("evidence"), 0, 10)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        before,
        fs::read(repository.root().join("meetings.sqlite3")).unwrap()
    );
    let updated = repository
        .save_review(SaveMeetingReviewRequest {
            session_id: "legacy".into(),
            expected_review_revision: Some(1),
            base: ReviewEditBase::LabelsOnly,
            labels: MeetingSpeakerLabels {
                me: "Owner".into(),
                them: "Guests".into(),
            },
            document: None,
            new_claims: Vec::new(),
        })
        .unwrap();
    assert_eq!(updated.labels.them, "Guests");
    assert_eq!(updated.segments[0].speaker, MeetingSpeaker::Them);
    assert_eq!(
        connection
            .query_row(
                "SELECT review_json FROM meeting_reviews WHERE session_id='legacy'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "invalid legacy review"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT artifact_json FROM meeting_artifacts WHERE session_id='legacy'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "invalid legacy json"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM meeting_review_fts WHERE session_id='legacy'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let exported = crate::meeting_review::render_export(
        &updated,
        crate::meeting_review::MeetingReviewExportFormat::Text,
    )
    .unwrap();
    assert!(exported.contains("Synthetic transcript evidence"));
    assert!(exported.contains("Guests"));
    assert!(!exported.contains("Summary"));
}
