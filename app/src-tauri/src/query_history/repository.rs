use super::types::*;
use crate::query_provider::QueryProviderId;
use rusqlite::{params, Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DB_FILE: &str = "query-history.sqlite3";
const LATEST_DB_SCHEMA_VERSION: u32 = 1;
const MAX_ENTRIES: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitializationOutcome {
    Ready,
    Reinitialized,
}

#[derive(Debug, Clone)]
pub(crate) struct QueryHistoryRepository {
    root: PathBuf,
    db_path: PathBuf,
}

impl QueryHistoryRepository {
    pub(crate) fn initialize(root: PathBuf) -> Result<(Self, InitializationOutcome), String> {
        ensure_private_directory(&root)?;
        ensure_private_directory(&root.join("quarantine"))?;
        let repository = Self {
            db_path: root.join(DB_FILE),
            root,
        };
        let existed = repository.db_path.exists();
        let first_attempt = repository.initialize_database();
        let outcome = match first_attempt {
            Ok(()) => InitializationOutcome::Ready,
            Err(error) if existed && is_future_version_error(&error) => return Err(error),
            Err(_) if existed => {
                repository.quarantine_corrupt_database()?;
                repository.initialize_database()?;
                InitializationOutcome::Reinitialized
            }
            Err(error) => return Err(error),
        };
        set_private_file_permissions(&repository.db_path)?;
        Ok((repository, outcome))
    }

    fn initialize_database(&self) -> Result<(), String> {
        if self.db_path.exists() {
            let version = read_schema_version(&self.db_path)?;
            if version > LATEST_DB_SCHEMA_VERSION {
                return Err(future_version_error(version));
            }
        }
        let mut connection = self.open_raw()?;
        configure_connection(&connection)?;
        migrate(&mut connection)?;
        quick_check(&connection)?;
        validate_schema(&connection)
    }

    fn open_raw(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|_| storage_error())
    }

    fn open_checked(&self) -> Result<Connection, String> {
        // Inspect the version read-only before connection pragmas such as WAL
        // can modify a database written by a newer Murmur build.
        let version = read_schema_version(&self.db_path)?;
        if version > LATEST_DB_SCHEMA_VERSION {
            return Err(future_version_error(version));
        }
        let connection = self.open_raw()?;
        configure_connection(&connection)?;
        quick_check(&connection)?;
        if version == LATEST_DB_SCHEMA_VERSION {
            validate_schema(&connection)?;
        }
        Ok(connection)
    }

    pub(crate) fn clear_epoch(&self) -> Result<u64, String> {
        let connection = self.open_checked()?;
        clear_epoch(&connection)
    }

    pub(crate) fn insert_if_epoch(
        &self,
        expected_epoch: u64,
        draft: QueryHistoryDraft,
    ) -> Result<Option<QueryHistoryEntryV1>, String> {
        validate_draft(&draft)?;
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        if clear_epoch(&transaction)? != expected_epoch {
            return Ok(None);
        }
        let id: String = transaction
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
            .map_err(db_error)?;
        let entry = QueryHistoryEntryV1 {
            schema_version: QUERY_HISTORY_SCHEMA_VERSION,
            id,
            timestamp_ms: draft.timestamp_ms,
            provider: draft.provider,
            question: draft.question,
            answer: draft.answer,
            tokens: draft.tokens,
            duration_ms: draft.duration_ms,
            error_code: draft.error_code,
        };
        let tokens_json = entry
            .tokens
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| invalid_record())?;
        transaction
            .execute(
                "INSERT INTO query_history(
                    id, record_version, timestamp_ms, provider, question, answer,
                    tokens_json, duration_ms, error_code
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    entry.id,
                    QUERY_HISTORY_SCHEMA_VERSION,
                    entry.timestamp_ms,
                    entry.provider.as_str(),
                    entry.question,
                    entry.answer,
                    tokens_json,
                    to_i64(entry.duration_ms)?,
                    entry.error_code,
                ],
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM query_history WHERE id NOT IN (
                    SELECT id FROM query_history
                    ORDER BY timestamp_ms DESC, rowid DESC LIMIT ?
                 )",
                [MAX_ENTRIES],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(Some(entry))
    }

    pub(crate) fn list(
        &self,
        offset: u32,
        limit: u32,
        provider: Option<QueryProviderId>,
    ) -> Result<QueryHistoryPageV1, String> {
        let connection = self.open_checked()?;
        let limit = limit.clamp(1, MAX_QUERY_HISTORY_PAGE_SIZE);
        let total: u32 = match provider {
            Some(provider) => connection
                .query_row(
                    "SELECT COUNT(*) FROM query_history WHERE provider = ?",
                    [provider.as_str()],
                    |row| row.get::<_, u32>(0),
                )
                .map_err(db_error)?,
            None => connection
                .query_row("SELECT COUNT(*) FROM query_history", [], |row| row.get(0))
                .map_err(db_error)?,
        };
        let offset = offset.min(total);
        let mut statement = connection
            .prepare(
                "SELECT id, record_version, timestamp_ms, provider, question, answer,
                        tokens_json, duration_ms, error_code
                 FROM query_history
                 WHERE (? IS NULL OR provider = ?)
                 ORDER BY timestamp_ms DESC, rowid DESC LIMIT ? OFFSET ?",
            )
            .map_err(db_error)?;
        let provider = provider.map(|value| value.as_str());
        let rows = statement
            .query_map(params![provider, provider, limit, offset], |row| {
                let record_version = row.get::<_, u32>(1)?;
                if record_version != QUERY_HISTORY_SCHEMA_VERSION {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let provider = parse_provider(&row.get::<_, String>(3)?)?;
                let tokens_json = row.get::<_, Option<String>>(6)?;
                let tokens = tokens_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(QueryHistoryEntryV1 {
                    schema_version: record_version,
                    id: row.get(0)?,
                    timestamp_ms: row.get(2)?,
                    provider,
                    question: row.get(4)?,
                    answer: row.get(5)?,
                    tokens,
                    duration_ms: to_u64(row.get(7)?)?,
                    error_code: row.get(8)?,
                })
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let has_more = offset.saturating_add(rows.len().try_into().unwrap_or(u32::MAX)) < total;
        Ok(QueryHistoryPageV1 {
            schema_version: QUERY_HISTORY_SCHEMA_VERSION,
            entries: rows,
            total,
            offset,
            has_more,
        })
    }

    #[cfg(test)]
    pub(crate) fn clear(&self) -> Result<(), String> {
        let next_epoch = self
            .clear_epoch()?
            .checked_add(1)
            .ok_or_else(|| storage_error())?;
        self.clear_to_epoch(next_epoch)
    }

    pub(crate) fn clear_to_epoch(&self, next_epoch: u64) -> Result<(), String> {
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        transaction
            .execute(
                "UPDATE query_history_meta SET value = ? WHERE key = 'clear_epoch'",
                [to_i64(next_epoch)?],
            )
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM query_history", [])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        checkpoint_truncate(&connection)?;
        connection.execute_batch("VACUUM;").map_err(db_error)?;
        // VACUUM itself writes in WAL mode. A successful purge is not reported
        // until those new frames are durably checkpointed and the WAL has
        // been verified empty.
        checkpoint_truncate(&connection)?;
        remove_files_in(&self.root.join("quarantine"))?;
        Ok(())
    }

    pub(crate) fn should_reset_after(error: &str) -> bool {
        is_future_version_error(error) || error == invalid_record()
    }

    /// Explicit purge is the only operation allowed to replace a store from a
    /// newer build. Reads and automatic recovery remain fail-closed.
    pub(crate) fn reset(root: PathBuf, clear_epoch: u64) -> Result<Self, String> {
        ensure_private_directory(&root)?;
        let quarantine = root.join("quarantine");
        ensure_private_directory(&quarantine)?;
        // Fail before touching the live database if old quarantined content
        // cannot be purged.
        remove_files_in(&quarantine)?;
        let db_path = root.join(DB_FILE);
        remove_sidecars(&db_path)?;
        if db_path.exists() {
            fs::remove_file(&db_path).map_err(|_| storage_error())?;
        }
        let (repository, _) = Self::initialize(root)?;
        repository.set_clear_epoch(clear_epoch)?;
        Ok(repository)
    }

    fn set_clear_epoch(&self, epoch: u64) -> Result<(), String> {
        let connection = self.open_checked()?;
        connection
            .execute(
                "UPDATE query_history_meta SET value=? WHERE key='clear_epoch'",
                [to_i64(epoch)?],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn quarantine_corrupt_database(&self) -> Result<InitializationOutcome, String> {
        let path = self
            .root
            .join("quarantine")
            .join(format!("query-history-corrupt-{}.sqlite3", now_ms()));
        fs::rename(&self.db_path, path).map_err(|_| storage_error())?;
        remove_sidecars(&self.db_path)?;
        Ok(InitializationOutcome::Reinitialized)
    }
}

fn validate_draft(draft: &QueryHistoryDraft) -> Result<(), String> {
    const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    if draft.question.is_empty()
        || draft.question.len() > 32 * 1024
        || draft.answer.len() > 256 * 1024
        || draft.question.contains('\0')
        || draft.answer.contains('\0')
        || draft.timestamp_ms < 0
        || draft.timestamp_ms as u64 > JS_MAX_SAFE_INTEGER
        || draft.duration_ms > JS_MAX_SAFE_INTEGER
        || draft
            .error_code
            .as_deref()
            .is_some_and(|code| code.len() > 64 || !valid_error_code(code))
    {
        return Err(invalid_record());
    }
    if draft.tokens.is_some_and(|tokens| {
        [
            tokens.input_tokens,
            tokens.output_tokens,
            tokens.reasoning_output_tokens,
            tokens.cached_input_tokens,
            tokens.cache_creation_input_tokens,
        ]
        .into_iter()
        .any(|value| value > JS_MAX_SAFE_INTEGER)
    }) {
        return Err(invalid_record());
    }
    Ok(())
}

fn valid_error_code(code: &str) -> bool {
    matches!(
        code,
        "not_configured"
            | "invalid_executable"
            | "invalid_arguments"
            | "invalid_timeout"
            | "invalid_environment"
            | "environment_unavailable"
            | "busy"
            | "audio_start_failed"
            | "audio_not_ready"
            | "audio_capture_failed"
            | "audio_recovering"
            | "audio_recovery_stalled"
            | "no_speech"
            | "transcription_failed"
            | "empty_query"
            | "query_too_large"
            | "spawn_failed"
            | "process_failed"
            | "termination_unconfirmed"
            | "timed_out"
            | "output_too_large"
            | "provider_not_authenticated"
            | "provider_error"
            | "exit_nonzero"
            | "empty_answer"
            | "clipboard_superseded"
            | "clipboard_unavailable"
            | "cancelled"
    )
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_provider(value: &str) -> rusqlite::Result<QueryProviderId> {
    match value {
        "claude" => Ok(QueryProviderId::Claude),
        "codex" => Ok(QueryProviderId::Codex),
        "grok" => Ok(QueryProviderId::Grok),
        "cursor" => Ok(QueryProviderId::Cursor),
        "custom" => Ok(QueryProviderId::Custom),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA secure_delete=ON;
             PRAGMA busy_timeout=2000;",
        )
        .map_err(db_error)
}

fn migrate(connection: &mut Connection) -> Result<(), String> {
    let current = schema_version(connection)?;
    if current > LATEST_DB_SCHEMA_VERSION {
        return Err(future_version_error(current));
    }
    if current == 0 {
        let transaction = connection.transaction().map_err(db_error)?;
        transaction
            .execute_batch(
                "CREATE TABLE query_history_meta(
                    key TEXT PRIMARY KEY NOT NULL,
                    value INTEGER NOT NULL CHECK(value >= 0)
                 );
                 INSERT INTO query_history_meta(key, value) VALUES ('clear_epoch', 0);
                 CREATE TABLE query_history(
                    id TEXT PRIMARY KEY NOT NULL,
                    record_version INTEGER NOT NULL,
                    timestamp_ms INTEGER NOT NULL CHECK(timestamp_ms >= 0),
                    provider TEXT NOT NULL CHECK(provider IN ('claude','codex','grok','cursor','custom')),
                    question TEXT NOT NULL,
                    answer TEXT NOT NULL,
                    tokens_json TEXT,
                    duration_ms INTEGER NOT NULL CHECK(duration_ms >= 0),
                    error_code TEXT
                 );
                 CREATE INDEX query_history_newest ON query_history(timestamp_ms DESC, id DESC);
                 CREATE INDEX query_history_provider_newest ON query_history(provider, timestamp_ms DESC, id DESC);",
            )
            .map_err(db_error)?;
        transaction
            .pragma_update(None, "user_version", LATEST_DB_SCHEMA_VERSION)
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<(), String> {
    let allowed_objects = [
        ("table", "query_history_meta"),
        ("table", "query_history"),
        ("index", "sqlite_autoindex_query_history_meta_1"),
        ("index", "sqlite_autoindex_query_history_1"),
        ("index", "query_history_newest"),
        ("index", "query_history_provider_newest"),
    ];
    let mut objects = connection
        .prepare(
            "SELECT type, name FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' OR name LIKE 'sqlite_autoindex_query_history%'",
        )
        .map_err(db_error)?;
    let objects = objects
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    if objects.len() != allowed_objects.len()
        || objects.iter().any(|(kind, name)| {
            !allowed_objects
                .iter()
                .any(|(allowed_kind, allowed_name)| kind == allowed_kind && name == allowed_name)
        })
    {
        return Err(invalid_record());
    }

    for (table, expected_columns) in [
        (
            "query_history_meta",
            &[("key", "TEXT", true, 1_u32), ("value", "INTEGER", true, 0)][..],
        ),
        (
            "query_history",
            &[
                ("id", "TEXT", true, 1),
                ("record_version", "INTEGER", true, 0),
                ("timestamp_ms", "INTEGER", true, 0),
                ("provider", "TEXT", true, 0),
                ("question", "TEXT", true, 0),
                ("answer", "TEXT", true, 0),
                ("tokens_json", "TEXT", false, 0),
                ("duration_ms", "INTEGER", true, 0),
                ("error_code", "TEXT", false, 0),
            ][..],
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)? == 1,
                    row.get::<_, u32>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        if columns.len() != expected_columns.len()
            || columns
                .iter()
                .zip(expected_columns.iter())
                .any(|(actual, expected)| {
                    actual.0 != expected.0
                        || !actual.1.eq_ignore_ascii_case(expected.1)
                        || actual.2 != expected.2
                        || actual.3 != expected.3
                })
        {
            return Err(invalid_record());
        }
    }
    for (index, expected_columns) in [
        (
            "query_history_newest",
            &[("timestamp_ms", true), ("id", true)][..],
        ),
        (
            "query_history_provider_newest",
            &[("provider", false), ("timestamp_ms", true), ("id", true)][..],
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA index_xinfo({index})"))
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, u32>(3)? == 1,
                    row.get::<_, u32>(5)? == 1,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
            .into_iter()
            .filter_map(|(name, descending, key)| key.then(|| name.map(|name| (name, descending))))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid_record)?;
        if columns.len() != expected_columns.len()
            || columns.iter().zip(expected_columns.iter()).any(
                |((name, descending), (expected_name, expected_descending))| {
                    name != expected_name || descending != expected_descending
                },
            )
        {
            return Err(invalid_record());
        }
    }
    let epoch_rows: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM query_history_meta WHERE key='clear_epoch' AND value >= 0",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if epoch_rows != 1 {
        return Err(invalid_record());
    }
    let row_count: u32 = connection
        .query_row("SELECT COUNT(*) FROM query_history", [], |row| row.get(0))
        .map_err(db_error)?;
    if row_count > MAX_ENTRIES {
        return Err(invalid_record());
    }
    let mut rows = connection
        .prepare(
            "SELECT id, record_version, timestamp_ms, provider, question, answer,
                    tokens_json, duration_ms, error_code FROM query_history",
        )
        .map_err(db_error)?;
    let rows = rows
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(db_error)?;
    for row in rows {
        let (
            id,
            version,
            timestamp_ms,
            provider,
            question,
            answer,
            tokens,
            duration_ms,
            error_code,
        ) = row.map_err(db_error)?;
        if !valid_id(&id)
            || version != QUERY_HISTORY_SCHEMA_VERSION
            || timestamp_ms < 0
            || parse_provider(&provider).is_err()
            || u64::try_from(duration_ms).is_err()
        {
            return Err(invalid_record());
        }
        let tokens = tokens
            .map(|json| serde_json::from_str::<QueryHistoryTokenCountsV1>(&json))
            .transpose()
            .map_err(|_| invalid_record())?;
        validate_draft(&QueryHistoryDraft {
            timestamp_ms,
            provider: parse_provider(&provider).map_err(|_| invalid_record())?,
            question,
            answer,
            tokens,
            duration_ms: u64::try_from(duration_ms).map_err(|_| invalid_record())?,
            error_code,
        })?;
    }
    Ok(())
}

fn read_schema_version(path: &Path) -> Result<u32, String> {
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(db_error)?;
    schema_version(&connection)
}

fn schema_version(connection: &Connection) -> Result<u32, String> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)
}

fn quick_check(connection: &Connection) -> Result<(), String> {
    let status: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(db_error)?;
    (status == "ok").then_some(()).ok_or_else(invalid_record)
}

fn clear_epoch(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row(
            "SELECT value FROM query_history_meta WHERE key='clear_epoch'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(db_error)?
        .try_into()
        .map_err(|_| invalid_record())
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| storage_error())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| storage_error())?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| storage_error())?;
    }
    Ok(())
}

fn remove_sidecars(path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        match fs::remove_file(&sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(storage_error()),
        }
        if sidecar.exists() {
            return Err(storage_error());
        }
    }
    Ok(())
}

fn remove_files_in(path: &Path) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|_| storage_error())? {
        let path = entry.map_err(|_| storage_error())?.path();
        if path.is_file() {
            fs::remove_file(path).map_err(|_| storage_error())?;
        }
    }
    Ok(())
}

fn checkpoint_truncate(connection: &Connection) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let (busy, log_frames): (u32, u32) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(db_error)?;
        if busy == 0 && log_frames == 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Voice Query history purge could not securely finish.".to_string());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn is_future_version_error(error: &str) -> bool {
    error.starts_with("This Voice Query history database uses schema version")
}

fn future_version_error(version: u32) -> String {
    format!(
        "This Voice Query history database uses schema version {version}, which is newer than this Murmur build supports."
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| invalid_record())
}

fn to_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn storage_error() -> String {
    "The local Voice Query history store is unavailable.".to_string()
}

fn invalid_record() -> String {
    "The local Voice Query history store contains an invalid record.".to_string()
}

fn db_error(_: rusqlite::Error) -> String {
    storage_error()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repository() -> (tempfile::TempDir, QueryHistoryRepository) {
        let temp = tempfile::tempdir().unwrap();
        let (repository, _) =
            QueryHistoryRepository::initialize(temp.path().join("query-history")).unwrap();
        (temp, repository)
    }

    fn draft(id: u64) -> QueryHistoryDraft {
        QueryHistoryDraft {
            timestamp_ms: id as i64,
            provider: QueryProviderId::Claude,
            question: format!("question-{id}"),
            answer: format!("answer-{id}"),
            tokens: Some(QueryHistoryTokenCountsV1 {
                input_tokens: id,
                output_tokens: 2,
                reasoning_output_tokens: 3,
                cached_input_tokens: 4,
                cache_creation_input_tokens: 5,
            }),
            duration_ms: id,
            error_code: None,
        }
    }

    #[test]
    fn creates_private_versioned_store_and_round_trips_entries() {
        let (temp, repository) = repository();
        let epoch = repository.clear_epoch().unwrap();
        let inserted = repository
            .insert_if_epoch(epoch, draft(1))
            .unwrap()
            .unwrap();
        let page = repository.list(0, 10, None).unwrap();
        assert_eq!(page.entries, vec![inserted]);
        assert_eq!(page.total, 1);
        assert!(!page.has_more);
        let connection = Connection::open(temp.path().join("query-history").join(DB_FILE)).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(temp.path().join("query-history").join(DB_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn retention_provider_filter_and_pagination_are_bounded() {
        let (_temp, repository) = repository();
        let epoch = repository.clear_epoch().unwrap();
        for id in 1..=205 {
            let mut item = draft(id);
            if id == 205 {
                item.provider = QueryProviderId::Codex;
            }
            repository.insert_if_epoch(epoch, item).unwrap();
        }
        let first = repository.list(0, 1000, None).unwrap();
        assert_eq!(first.total, 200);
        assert_eq!(first.entries.len(), 100);
        assert!(first.has_more);
        assert_eq!(first.entries[0].question, "question-205");
        let codex = repository
            .list(0, 10, Some(QueryProviderId::Codex))
            .unwrap();
        assert_eq!(codex.total, 1);
        assert_eq!(codex.entries[0].provider, QueryProviderId::Codex);
    }

    #[test]
    fn purge_epoch_prevents_an_in_flight_pass_from_reinserting() {
        let (temp, repository) = repository();
        let old_epoch = repository.clear_epoch().unwrap();
        repository.insert_if_epoch(old_epoch, draft(1)).unwrap();
        fs::write(
            temp.path().join("query-history/quarantine/stale.sqlite3"),
            b"private sentinel",
        )
        .unwrap();
        repository.clear().unwrap();
        assert!(repository
            .insert_if_epoch(old_epoch, draft(2))
            .unwrap()
            .is_none());
        assert_eq!(repository.list(0, 10, None).unwrap().total, 0);
        assert_eq!(
            fs::read_dir(temp.path().join("query-history/quarantine"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn future_version_is_preserved_and_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("query-history");
        fs::create_dir_all(root.join("quarantine")).unwrap();
        let db = root.join(DB_FILE);
        let connection = Connection::open(&db).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        assert!(QueryHistoryRepository::initialize(root.clone()).is_err());
        let connection = Connection::open(db).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), 99);
        drop(connection);
        let repository = QueryHistoryRepository::reset(root, 1).unwrap();
        assert_eq!(repository.list(0, 10, None).unwrap().total, 0);
    }

    #[test]
    fn corrupt_database_is_quarantined_without_exposing_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("query-history");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(DB_FILE), b"PRIVATE QUESTION SENTINEL").unwrap();
        let (_, outcome) = QueryHistoryRepository::initialize(root.clone()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Reinitialized);
        assert_eq!(fs::read_dir(root.join("quarantine")).unwrap().count(), 1);
    }

    #[test]
    fn malformed_supported_schemas_are_quarantined_and_reinitialized() {
        for version in [0_u32, 1] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join(format!("query-history-{version}"));
            fs::create_dir_all(&root).unwrap();
            let connection = Connection::open(root.join(DB_FILE)).unwrap();
            connection
                .execute("CREATE TABLE query_history(id TEXT)", [])
                .unwrap();
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            drop(connection);

            let (repository, outcome) = QueryHistoryRepository::initialize(root.clone()).unwrap();
            assert_eq!(outcome, InitializationOutcome::Reinitialized);
            assert_eq!(repository.list(0, 10, None).unwrap().total, 0);
            assert_eq!(fs::read_dir(root.join("quarantine")).unwrap().count(), 1);
        }
    }

    #[test]
    fn v1_schema_allowlist_rejects_extra_columns_wrong_affinity_and_token_fields() {
        enum Mutation {
            ExtraColumn,
            WrongAffinity,
            ExtraTokenField,
            InvalidFrontendNumbers,
        }
        for (index, mutation) in [
            Mutation::ExtraColumn,
            Mutation::WrongAffinity,
            Mutation::ExtraTokenField,
            Mutation::InvalidFrontendNumbers,
        ]
        .into_iter()
        .enumerate()
        {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join(format!("query-history-shape-{index}"));
            let (repository, _) = QueryHistoryRepository::initialize(root.clone()).unwrap();
            drop(repository);
            let connection = Connection::open(root.join(DB_FILE)).unwrap();
            match mutation {
                Mutation::ExtraColumn => {
                    connection
                        .execute("ALTER TABLE query_history ADD COLUMN context TEXT", [])
                        .unwrap();
                }
                Mutation::WrongAffinity => {
                    connection
                        .execute_batch(
                            "DROP INDEX query_history_newest;
                             DROP INDEX query_history_provider_newest;
                             ALTER TABLE query_history RENAME TO query_history_old;
                             CREATE TABLE query_history(
                                id TEXT PRIMARY KEY NOT NULL,
                                record_version INTEGER NOT NULL,
                                timestamp_ms TEXT NOT NULL,
                                provider TEXT NOT NULL,
                                question TEXT NOT NULL,
                                answer TEXT NOT NULL,
                                tokens_json TEXT,
                                duration_ms INTEGER NOT NULL,
                                error_code TEXT
                             );
                             DROP TABLE query_history_old;
                             CREATE INDEX query_history_newest
                                ON query_history(timestamp_ms DESC, id DESC);
                             CREATE INDEX query_history_provider_newest
                                ON query_history(provider, timestamp_ms DESC, id DESC);",
                        )
                        .unwrap();
                }
                Mutation::ExtraTokenField => {
                    connection
                        .execute(
                            "INSERT INTO query_history(
                                id, record_version, timestamp_ms, provider, question, answer,
                                tokens_json, duration_ms, error_code
                             ) VALUES ('bad', 1, 1, 'claude', 'question', 'answer',
                                '{\"inputTokens\":1,\"outputTokens\":2,\"reasoningOutputTokens\":0,\"cachedInputTokens\":0,\"cacheCreationInputTokens\":0,\"costUsd\":99,\"detail\":\"private\"}',
                                1, NULL)",
                            [],
                        )
                        .unwrap();
                }
                Mutation::InvalidFrontendNumbers => {
                    connection
                        .execute(
                            "INSERT INTO query_history(
                                id, record_version, timestamp_ms, provider, question, answer,
                                tokens_json, duration_ms, error_code
                             ) VALUES ('NOT_LOWER_HEX', 1, ?, 'claude', 'question', 'answer',
                                NULL, ?, NULL)",
                            params![i64::MAX, i64::MAX],
                        )
                        .unwrap();
                }
            }
            drop(connection);
            let (repository, outcome) = QueryHistoryRepository::initialize(root.clone()).unwrap();
            assert_eq!(outcome, InitializationOutcome::Reinitialized);
            assert_eq!(repository.list(0, 10, None).unwrap().total, 0);
            assert_eq!(fs::read_dir(root.join("quarantine")).unwrap().count(), 1);
        }
    }

    #[test]
    fn list_clamps_offset_and_token_counts_must_be_js_safe() {
        let (_temp, repository) = repository();
        let epoch = repository.clear_epoch().unwrap();
        repository.insert_if_epoch(epoch, draft(1)).unwrap();
        let page = repository.list(99, 10, None).unwrap();
        assert_eq!(page.offset, 1);
        assert!(page.entries.is_empty());
        assert!(!page.has_more);

        let mut unsafe_tokens = draft(2);
        unsafe_tokens.tokens.as_mut().unwrap().input_tokens = 9_007_199_254_740_992;
        assert!(repository
            .insert_if_epoch(epoch, unsafe_tokens)
            .unwrap_err()
            .contains("invalid record"));
        let mut unknown_error = draft(3);
        unknown_error.error_code = Some("future_private_detail".to_string());
        assert!(repository
            .insert_if_epoch(epoch, unknown_error)
            .unwrap_err()
            .contains("invalid record"));
    }

    #[test]
    fn initialization_quarantines_history_over_the_retention_bound() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("query-history-overflow");
        let (repository, _) = QueryHistoryRepository::initialize(root.clone()).unwrap();
        drop(repository);
        let mut connection = Connection::open(root.join(DB_FILE)).unwrap();
        let transaction = connection.transaction().unwrap();
        for id in 0..=MAX_ENTRIES {
            transaction
                .execute(
                    "INSERT INTO query_history(
                        id, record_version, timestamp_ms, provider, question, answer,
                        tokens_json, duration_ms, error_code
                     ) VALUES (?, 1, ?, 'claude', 'question', 'answer', NULL, 1, NULL)",
                    params![format!("{id:032x}"), id],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        drop(connection);
        let (repository, outcome) = QueryHistoryRepository::initialize(root).unwrap();
        assert_eq!(outcome, InitializationOutcome::Reinitialized);
        assert_eq!(repository.list(0, 10, None).unwrap().total, 0);
    }

    #[test]
    fn purge_waits_for_readers_and_removes_content_from_database_and_wal() {
        let (temp, repository) = repository();
        let epoch = repository.clear_epoch().unwrap();
        let mut private = draft(1);
        private.question = "PRIVATE_QUERY_PURGE_SENTINEL_42".to_string();
        private.answer = "PRIVATE_ANSWER_PURGE_SENTINEL_73".to_string();
        repository.insert_if_epoch(epoch, private).unwrap();

        let reader = repository.open_checked().unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let _: String = reader
            .query_row("SELECT question FROM query_history LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let clearing = repository.clone();
        let clear_thread = std::thread::spawn(move || clearing.clear());
        std::thread::sleep(Duration::from_millis(50));
        reader.execute_batch("ROLLBACK").unwrap();
        drop(reader);
        clear_thread.join().unwrap().unwrap();

        for path in [
            temp.path().join("query-history").join(DB_FILE),
            temp.path()
                .join("query-history")
                .join(format!("{DB_FILE}-wal")),
        ] {
            if path.exists() {
                let bytes = fs::read(path).unwrap();
                assert!(!bytes
                    .windows(b"PRIVATE_QUERY_PURGE_SENTINEL_42".len())
                    .any(|window| window == b"PRIVATE_QUERY_PURGE_SENTINEL_42"));
                assert!(!bytes
                    .windows(b"PRIVATE_ANSWER_PURGE_SENTINEL_73".len())
                    .any(|window| window == b"PRIVATE_ANSWER_PURGE_SENTINEL_73"));
            }
        }
    }

    #[test]
    fn reset_fails_if_a_private_sqlite_sidecar_cannot_be_removed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("query-history-sidecar-failure");
        let (repository, _) = QueryHistoryRepository::initialize(root.clone()).unwrap();
        drop(repository);
        let wal = root.join(format!("{DB_FILE}-wal"));
        if wal.exists() {
            fs::remove_file(&wal).unwrap();
        }
        fs::create_dir(&wal).unwrap();
        assert!(QueryHistoryRepository::reset(root.clone(), 9).is_err());
        assert!(root.join(DB_FILE).exists());
        assert!(wal.exists());
    }
}
