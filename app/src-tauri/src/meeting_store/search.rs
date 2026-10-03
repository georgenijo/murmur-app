//! Private, derived search rows. Call only within the source-write transaction.
use rusqlite::{params, Connection, OptionalExtension};

use crate::meeting_review::MeetingReviewDocumentV1;

pub(super) const MATCHES_CTE: &str =
    "WITH matches AS (
       SELECT session_id, 'transcript' AS field FROM meeting_segments_fts WHERE meeting_segments_fts MATCH ?1
       UNION SELECT session_id, 'title' FROM meeting_sessions_fts WHERE meeting_sessions_fts MATCH ?1
       UNION SELECT session_id, field FROM meeting_review_fts WHERE meeting_review_fts MATCH ?1
     )";

pub(super) fn refresh_document_index(connection: &Connection, id: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM meeting_review_fts WHERE session_id=?", [id])?;
    // A labels-only review has no document and must still use the generated draft.
    let reviewed = connection
        .query_row(
            "SELECT review_json FROM meeting_reviews WHERE session_id=?",
            [id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let allowed = {
        let mut statement = connection.prepare(
            "SELECT id FROM meeting_segments WHERE session_id=? AND status='final' AND trim(text)!=''",
        )?;
        let ids = statement
            .query_map([id], |row| row.get::<_, i64>(0))?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        ids
    };
    let document = if let Some(json) = reviewed {
        serde_json::from_str::<MeetingReviewDocumentV1>(&json)
            .ok()
            .filter(|document| {
                let mut document = document.clone();
                crate::meeting_review::validate_document(&mut document, &allowed)
            })
    } else {
        let generated = connection
            .query_row(
                "SELECT artifact_json FROM meeting_artifacts WHERE session_id=?",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        generated.and_then(|json| {
            let mut artifact = serde_json::from_str(&json).ok()?;
            crate::meeting_artifact::validate_artifact(&mut artifact, &allowed)
                .then(|| crate::meeting_review::document_from_artifact(&artifact))
        })
    };
    // Invalid persisted content is unavailable in the workspace too. Do not
    // index malformed JSON or fall back to a superseded generated document.
    let Some(document) = document else {
        return Ok(());
    };
    let mut insert = connection
        .prepare("INSERT INTO meeting_review_fts(session_id, field, text) VALUES(?,?,?)")?;
    insert.execute(params![id, "summary", document.summary.text])?;
    for item in document.decisions {
        insert.execute(params![id, "decision", item.text])?;
    }
    for item in document.action_items {
        let text = format!(
            "{} {} {}",
            item.text,
            item.owner.unwrap_or_default(),
            item.due_date.unwrap_or_default()
        );
        insert.execute(params![id, "action_item", text])?;
    }
    for item in document.open_questions {
        insert.execute(params![id, "open_question", item.text])?;
    }
    Ok(())
}
