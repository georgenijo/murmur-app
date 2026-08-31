use rusqlite::Connection;

use super::types::MEETING_STORE_SCHEMA_VERSION;

pub(super) fn schema_version(connection: &Connection) -> Result<u32, String> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| "The meeting transcript database is unavailable.".to_string())
}

pub(super) fn quick_check(connection: &Connection) -> Result<(), String> {
    let status: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| "The meeting transcript database is unavailable.".to_string())?;
    if status == "ok" {
        Ok(())
    } else {
        Err("The meeting transcript database failed its integrity check.".to_string())
    }
}

pub(super) fn migrate(connection: &Connection) -> Result<(), String> {
    let version = schema_version(connection)?;
    if version > MEETING_STORE_SCHEMA_VERSION {
        return Err(format!(
            "This meeting database uses schema version {version}, which is newer than this Murmur build supports."
        ));
    }
    if version == 0 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE meeting_sessions (
                   id TEXT PRIMARY KEY NOT NULL,
                   started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0),
                   ended_at_ms INTEGER CHECK(ended_at_ms IS NULL OR ended_at_ms >= started_at_ms),
                   status TEXT NOT NULL CHECK(status IN ('active','complete','interrupted','failed')),
                   model_name TEXT NOT NULL,
                   language TEXT NOT NULL,
                   smart_punctuation INTEGER NOT NULL CHECK(smart_punctuation IN (0,1)),
                   retain_audio INTEGER NOT NULL CHECK(retain_audio IN (0,1)),
                   error_code TEXT
                 );
                 CREATE TABLE meeting_segments (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   session_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   speaker TEXT NOT NULL CHECK(speaker IN ('me','them')),
                   sequence INTEGER NOT NULL CHECK(sequence >= 0),
                   start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
                   end_ms INTEGER NOT NULL CHECK(end_ms >= start_ms),
                   status TEXT NOT NULL CHECK(status IN ('pending','final','failed')),
                   text TEXT NOT NULL DEFAULT '',
                   audio_relative_path TEXT,
                   error_code TEXT,
                   UNIQUE(session_id, speaker, sequence)
                 );
                 CREATE INDEX meeting_sessions_started_idx ON meeting_sessions(started_at_ms DESC);
                 CREATE INDEX meeting_segments_session_time_idx ON meeting_segments(session_id, start_ms, id);
                 CREATE INDEX meeting_segments_pending_idx ON meeting_segments(status, id);
                 CREATE VIRTUAL TABLE meeting_segments_fts USING fts5(
                   segment_id UNINDEXED,
                   session_id UNINDEXED,
                   text,
                   tokenize='unicode61 remove_diacritics 2'
                 );
                 PRAGMA user_version=1;
                 COMMIT;",
            )
            .map_err(|_| "Murmur could not create the meeting transcript database.".to_string())?;
    }
    if schema_version(connection)? == 1 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE meeting_artifacts (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   artifact_json TEXT NOT NULL,
                   created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                   runtime_ms INTEGER NOT NULL CHECK(runtime_ms >= 0),
                   peak_rss_mb INTEGER NOT NULL CHECK(peak_rss_mb >= 0)
                 );
                 PRAGMA user_version=2;
                 COMMIT;",
            )
            .map_err(|_| "Murmur could not migrate meeting summaries.".to_string())?;
    }
    if schema_version(connection)? == 2 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 ALTER TABLE meeting_artifacts ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0);
                 CREATE TABLE meeting_reviews (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   revision INTEGER NOT NULL CHECK(revision > 0),
                   based_on_artifact_revision INTEGER CHECK(based_on_artifact_revision IS NULL OR based_on_artifact_revision > 0),
                   me_label TEXT NOT NULL,
                   them_label TEXT NOT NULL,
                   review_json TEXT,
                   updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0),
                   CHECK((review_json IS NULL AND based_on_artifact_revision IS NULL) OR
                         (review_json IS NOT NULL AND based_on_artifact_revision IS NOT NULL))
                 );
                 PRAGMA user_version=3;
                 COMMIT;",
            )
            .map_err(|_| "Murmur could not migrate meeting reviews.".to_string())?;
    }
    validate_schema(connection)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), String> {
    for table in [
        "meeting_sessions",
        "meeting_segments",
        "meeting_segments_fts",
        "meeting_artifacts",
        "meeting_reviews",
    ] {
        let exists: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?)",
                [table],
                |row| row.get(0),
            )
            .map_err(|_| "The meeting transcript database is unavailable.".to_string())?;
        if exists != 1 {
            return Err("The meeting transcript database schema is incomplete.".to_string());
        }
    }
    for (table, required) in [
        ("meeting_artifacts", &["revision"][..]),
        (
            "meeting_reviews",
            &[
                "session_id",
                "revision",
                "based_on_artifact_revision",
                "me_label",
                "them_label",
                "review_json",
                "updated_at_ms",
            ][..],
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|_| "The meeting transcript database is unavailable.".to_string())?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| "The meeting transcript database is unavailable.".to_string())?
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(|_| "The meeting transcript database is unavailable.".to_string())?;
        if required.iter().any(|column| !columns.contains(*column)) {
            return Err("The meeting transcript database schema is incomplete.".to_string());
        }
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_list(meeting_reviews)")
        .map_err(|_| "The meeting transcript database is unavailable.".to_string())?;
    let cascades_to_session = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(2)?, row.get::<_, String>(6)?))
        })
        .map_err(|_| "The meeting transcript database is unavailable.".to_string())?
        .any(|row| matches!(row, Ok((table, action)) if table == "meeting_sessions" && action.eq_ignore_ascii_case("CASCADE")));
    if !cascades_to_session {
        return Err("The meeting transcript database schema is incomplete.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_artifacts_migrate_without_becoming_user_reviews() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE meeting_sessions (
                   id TEXT PRIMARY KEY NOT NULL, started_at_ms INTEGER NOT NULL,
                   ended_at_ms INTEGER, status TEXT NOT NULL, model_name TEXT NOT NULL,
                   language TEXT NOT NULL, smart_punctuation INTEGER NOT NULL,
                   retain_audio INTEGER NOT NULL, error_code TEXT
                 );
                 CREATE TABLE meeting_segments (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   session_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   speaker TEXT NOT NULL, sequence INTEGER NOT NULL, start_ms INTEGER NOT NULL,
                   end_ms INTEGER NOT NULL, status TEXT NOT NULL, text TEXT NOT NULL DEFAULT '',
                   audio_relative_path TEXT, error_code TEXT,
                   UNIQUE(session_id, speaker, sequence)
                 );
                 CREATE VIRTUAL TABLE meeting_segments_fts USING fts5(segment_id UNINDEXED, session_id UNINDEXED, text);
                 CREATE TABLE meeting_artifacts (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   artifact_json TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
                   runtime_ms INTEGER NOT NULL, peak_rss_mb INTEGER NOT NULL
                 );
                 INSERT INTO meeting_sessions VALUES('meeting',1,2,'complete','base.en','en',1,0,NULL);
                 INSERT INTO meeting_artifacts VALUES('meeting','{}',3,4,5);
                 PRAGMA user_version=2;",
            )
            .unwrap();

        migrate(&connection).unwrap();

        assert_eq!(schema_version(&connection).unwrap(), 3);
        assert_eq!(
            connection
                .query_row(
                    "SELECT revision FROM meeting_artifacts WHERE session_id='meeting'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM meeting_reviews", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
