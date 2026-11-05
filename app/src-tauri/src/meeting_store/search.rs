//! Private, derived search rows. Call only within the source-write transaction.
use rusqlite::Connection;

pub(super) const MATCHES_CTE: &str =
    "WITH matches AS (
       SELECT session_id, 'transcript' AS field FROM meeting_segments_fts WHERE meeting_segments_fts MATCH ?1
       UNION SELECT session_id, 'title' FROM meeting_sessions_fts WHERE meeting_sessions_fts MATCH ?1
     )";

// Compatibility entrypoints for the existing schema. Legacy derived rows stay
// untouched and MATCHES_CTE searches only transcript and title.
#[cfg(test)]
pub(super) fn refresh_document_index(_connection: &Connection, _id: &str) -> rusqlite::Result<()> {
    Ok(())
}

pub(super) fn index_document(_connection: &Connection, _id: &str) -> rusqlite::Result<()> {
    Ok(())
}
