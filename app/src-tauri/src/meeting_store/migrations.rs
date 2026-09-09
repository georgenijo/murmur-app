use rusqlite::Connection;

use super::types::MEETING_STORE_SCHEMA_VERSION;

pub(super) fn schema_version(connection: &Connection) -> Result<u32, MeetingDatabaseError> {
    crate::sqlite_support::schema_version(connection).map_err(MeetingDatabaseError::from)
}

pub(super) fn quick_check(connection: &Connection) -> Result<(), MeetingDatabaseError> {
    if crate::sqlite_support::quick_check(connection).map_err(MeetingDatabaseError::from)? {
        Ok(())
    } else {
        Err(MeetingDatabaseError::InvalidData)
    }
}

#[derive(Debug)]
pub(super) enum MeetingDatabaseError {
    Sqlite(rusqlite::Error, &'static str),
    InvalidSchema,
    InvalidData,
    UnsupportedVersion(u32),
    NeedsMigration,
}

impl From<rusqlite::Error> for MeetingDatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error, "The meeting transcript database is unavailable.")
    }
}

impl MeetingDatabaseError {
    pub(super) fn is_corrupt(&self) -> bool {
        match self {
            Self::InvalidData => true,
            Self::Sqlite(
                rusqlite::Error::SqliteFailure(error, _)
                | rusqlite::Error::SqlInputError { error, .. },
                _,
            ) => matches!(
                error.extended_code & 0xff,
                rusqlite::ffi::SQLITE_CORRUPT | rusqlite::ffi::SQLITE_NOTADB
            ),
            _ => false,
        }
    }

    pub(super) fn is_invalid_backup(&self) -> bool {
        match self {
            Self::InvalidSchema
            | Self::InvalidData
            | Self::UnsupportedVersion(_)
            | Self::NeedsMigration => true,
            Self::Sqlite(
                rusqlite::Error::SqliteFailure(error, _)
                | rusqlite::Error::SqlInputError { error, .. },
                _,
            ) => matches!(
                error.extended_code & 0xff,
                rusqlite::ffi::SQLITE_ERROR
                    | rusqlite::ffi::SQLITE_CONSTRAINT
                    | rusqlite::ffi::SQLITE_CORRUPT
                    | rusqlite::ffi::SQLITE_NOTADB
            ),
            _ => false,
        }
    }

    pub(super) fn message(self) -> String {
        match self {
            Self::Sqlite(_, message) => message.to_string(),
            Self::InvalidSchema => {
                "The meeting transcript database schema is incomplete.".to_string()
            }
            Self::InvalidData => {
                "The meeting transcript database failed its integrity check.".to_string()
            }
            Self::UnsupportedVersion(version) => format!(
                "The meeting transcript database uses unsupported schema version {version}."
            ),
            Self::NeedsMigration => {
                "The meeting transcript database requires migration.".to_string()
            }
        }
    }
}

pub(super) fn migrate(connection: &Connection) -> Result<(), MeetingDatabaseError> {
    let version = schema_version(connection)?;
    if version > MEETING_STORE_SCHEMA_VERSION {
        return Err(MeetingDatabaseError::UnsupportedVersion(version));
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
            .map_err(|error| MeetingDatabaseError::Sqlite(error, "Murmur could not create the meeting transcript database."))?;
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
            .map_err(|error| MeetingDatabaseError::Sqlite(error, "Murmur could not migrate meeting summaries."))?;
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
            .map_err(|error| MeetingDatabaseError::Sqlite(error, "Murmur could not migrate meeting reviews."))?;
    }
    if schema_version(connection)? == 3 {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE meeting_remote_speakers (
                   session_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   speaker_id INTEGER NOT NULL CHECK(speaker_id BETWEEN 1 AND 32),
                   label TEXT NOT NULL CHECK(length(CAST(label AS BLOB)) BETWEEN 1 AND 80),
                   PRIMARY KEY(session_id, speaker_id)
                 ) WITHOUT ROWID;
                 ALTER TABLE meeting_segments RENAME TO meeting_segments_v3;
                 DROP INDEX IF EXISTS meeting_segments_session_time_idx;
                 DROP INDEX IF EXISTS meeting_segments_pending_idx;
                 CREATE TABLE meeting_segments (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   session_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   speaker TEXT NOT NULL CHECK(speaker IN ('me','them')),
                   remote_speaker_id INTEGER,
                   sequence INTEGER NOT NULL CHECK(sequence >= 0),
                   start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
                   end_ms INTEGER NOT NULL CHECK(end_ms >= start_ms),
                   status TEXT NOT NULL CHECK(status IN ('pending','final','failed')),
                   text TEXT NOT NULL DEFAULT '',
                   audio_relative_path TEXT,
                   error_code TEXT,
                   UNIQUE(session_id, speaker, sequence),
                   CHECK(remote_speaker_id IS NULL OR
                         (speaker='them' AND remote_speaker_id BETWEEN 1 AND 32)),
                   FOREIGN KEY(session_id, remote_speaker_id)
                     REFERENCES meeting_remote_speakers(session_id, speaker_id)
                 );
                 INSERT INTO meeting_segments(
                   id, session_id, speaker, sequence, start_ms, end_ms, status,
                   text, audio_relative_path, error_code
                 )
                 SELECT id, session_id, speaker, sequence, start_ms, end_ms, status,
                        text, audio_relative_path, error_code
                 FROM meeting_segments_v3;
                 DROP TABLE meeting_segments_v3;
                 CREATE INDEX meeting_segments_session_time_idx ON meeting_segments(session_id, start_ms, id);
                 CREATE INDEX meeting_segments_pending_idx ON meeting_segments(status, id);
                 PRAGMA user_version=4;
                 COMMIT;",
            )
            .map_err(|error| MeetingDatabaseError::Sqlite(error, "Murmur could not migrate remote meeting speakers."))?;
    }
    validate_schema(connection)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<(), MeetingDatabaseError> {
    if schema_version(connection)? != MEETING_STORE_SCHEMA_VERSION {
        return Err(MeetingDatabaseError::NeedsMigration);
    }
    validate_supported_schema(connection)
}

pub(super) fn validate_supported_schema(
    connection: &Connection,
) -> Result<(), MeetingDatabaseError> {
    let version = schema_version(connection)?;
    if !(1..=MEETING_STORE_SCHEMA_VERSION).contains(&version) {
        return Err(MeetingDatabaseError::UnsupportedVersion(version));
    }
    let tables: &[(&str, u32, &[&str])] = &[
        (
            "meeting_sessions",
            1,
            &[
                "id",
                "started_at_ms",
                "ended_at_ms",
                "status",
                "model_name",
                "language",
                "smart_punctuation",
                "retain_audio",
                "error_code",
            ],
        ),
        (
            "meeting_segments",
            1,
            &[
                "id",
                "session_id",
                "speaker",
                "sequence",
                "start_ms",
                "end_ms",
                "status",
                "text",
                "audio_relative_path",
                "error_code",
            ],
        ),
        (
            "meeting_segments_fts",
            1,
            &["segment_id", "session_id", "text"],
        ),
        (
            "meeting_artifacts",
            2,
            &[
                "session_id",
                "artifact_json",
                "created_at_ms",
                "runtime_ms",
                "peak_rss_mb",
            ],
        ),
        (
            "meeting_reviews",
            3,
            &[
                "session_id",
                "revision",
                "based_on_artifact_revision",
                "me_label",
                "them_label",
                "review_json",
                "updated_at_ms",
            ],
        ),
        (
            "meeting_remote_speakers",
            4,
            &["session_id", "speaker_id", "label"],
        ),
    ];
    for (table, introduced, columns) in tables {
        if version < *introduced {
            continue;
        }
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
                [table],
                |row| row.get(0),
            )
            .map_err(MeetingDatabaseError::from)?;
        if !exists {
            return Err(schema_error());
        }
        require_columns(connection, table, columns)?;
    }
    require_session_cascade(connection, "meeting_segments")?;
    if version >= 2 {
        require_session_cascade(connection, "meeting_artifacts")?;
    }
    if version >= 3 {
        require_columns(connection, "meeting_artifacts", &["revision"])?;
        require_session_cascade(connection, "meeting_reviews")?;
    }
    if version >= 4 {
        require_columns(connection, "meeting_segments", &["remote_speaker_id"])?;
        require_session_cascade(connection, "meeting_remote_speakers")?;
        let mut statement = connection
            .prepare("PRAGMA foreign_key_list(meeting_segments)")
            .map_err(MeetingDatabaseError::from)?;
        let remote_speaker_columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(MeetingDatabaseError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(MeetingDatabaseError::from)?
            .into_iter()
            .filter(|(table, _, _)| table == "meeting_remote_speakers")
            .map(|(_, from, to)| (from, to))
            .collect::<std::collections::HashSet<_>>();
        if remote_speaker_columns
            != std::collections::HashSet::from([
                ("session_id".to_string(), "session_id".to_string()),
                ("remote_speaker_id".to_string(), "speaker_id".to_string()),
            ])
        {
            return Err(schema_error());
        }
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(MeetingDatabaseError::from)?;
    if statement
        .query([])
        .map_err(MeetingDatabaseError::from)?
        .next()
        .map_err(MeetingDatabaseError::from)?
        .is_some()
    {
        return Err(MeetingDatabaseError::InvalidData);
    }
    Ok(())
}

fn schema_error() -> MeetingDatabaseError {
    MeetingDatabaseError::InvalidSchema
}

fn require_columns(
    connection: &Connection,
    table: &str,
    required: &[&str],
) -> Result<(), MeetingDatabaseError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(MeetingDatabaseError::from)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(MeetingDatabaseError::from)?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(MeetingDatabaseError::from)?;
    if required.iter().any(|column| !columns.contains(*column)) {
        return Err(schema_error());
    }
    Ok(())
}

fn require_session_cascade(
    connection: &Connection,
    table: &str,
) -> Result<(), MeetingDatabaseError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .map_err(MeetingDatabaseError::from)?;
    let keys = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(MeetingDatabaseError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(MeetingDatabaseError::from)?;
    if !keys.iter().any(|(table, from, to, action)| {
        table == "meeting_sessions"
            && from == "session_id"
            && to == "id"
            && action.eq_ignore_ascii_case("CASCADE")
    }) {
        return Err(schema_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_sqlite_failures_never_classify_as_invalid_backup_data() {
        for code in [
            rusqlite::ffi::SQLITE_IOERR_READ,
            rusqlite::ffi::SQLITE_NOMEM,
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_LOCKED_SHAREDCACHE,
            rusqlite::ffi::SQLITE_FULL,
            rusqlite::ffi::SQLITE_INTERRUPT,
            rusqlite::ffi::SQLITE_CANTOPEN,
        ] {
            let error = MeetingDatabaseError::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                Some("private diagnostic detail".into()),
            ));
            assert!(!error.is_invalid_backup(), "SQLite code {code}");
            assert!(!error.is_corrupt(), "SQLite code {code}");
            assert!(!error.message().contains("private diagnostic"));
        }
    }

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

        assert_eq!(schema_version(&connection).unwrap(), 4);
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

    #[test]
    fn v3_evidence_and_reviews_survive_remote_speaker_migration() {
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
                 CREATE INDEX meeting_sessions_started_idx ON meeting_sessions(started_at_ms DESC);
                 CREATE INDEX meeting_segments_session_time_idx ON meeting_segments(session_id, start_ms, id);
                 CREATE INDEX meeting_segments_pending_idx ON meeting_segments(status, id);
                 CREATE VIRTUAL TABLE meeting_segments_fts USING fts5(segment_id UNINDEXED, session_id UNINDEXED, text);
                 CREATE TABLE meeting_artifacts (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   artifact_json TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
                   runtime_ms INTEGER NOT NULL, peak_rss_mb INTEGER NOT NULL,
                   revision INTEGER NOT NULL DEFAULT 1
                 );
                 CREATE TABLE meeting_reviews (
                   session_id TEXT PRIMARY KEY NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
                   revision INTEGER NOT NULL, based_on_artifact_revision INTEGER,
                   me_label TEXT NOT NULL, them_label TEXT NOT NULL,
                   review_json TEXT, updated_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO meeting_sessions VALUES('meeting',1,2,'complete','base.en','en',1,0,NULL);
                 INSERT INTO meeting_segments VALUES(41,'meeting','them',0,10,20,'final','preserved evidence',NULL,NULL);
                 INSERT INTO meeting_segments_fts VALUES(41,'meeting','preserved evidence');
                 INSERT INTO meeting_reviews VALUES('meeting',7,NULL,'George','Team',NULL,3);
                 PRAGMA user_version=3;",
            )
            .unwrap();

        migrate(&connection).unwrap();

        assert_eq!(schema_version(&connection).unwrap(), 4);
        assert_eq!(
            connection
                .query_row(
                    "SELECT id, text, remote_speaker_id FROM meeting_segments WHERE session_id='meeting'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<i64>>(2)?)),
                )
                .unwrap(),
            (41, "preserved evidence".to_string(), None)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT revision, me_label, them_label FROM meeting_reviews WHERE session_id='meeting'",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .unwrap(),
            (7, "George".to_string(), "Team".to_string())
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT text FROM meeting_segments_fts WHERE segment_id=41",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "preserved evidence"
        );
    }
}
