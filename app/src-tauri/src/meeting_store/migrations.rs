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
    validate_schema(connection)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), String> {
    for table in [
        "meeting_sessions",
        "meeting_segments",
        "meeting_segments_fts",
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
    Ok(())
}
