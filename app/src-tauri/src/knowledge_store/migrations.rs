use rusqlite::{Connection, Transaction};

pub const LATEST_SCHEMA_VERSION: u32 = 2;

pub fn migrate(connection: &mut Connection) -> Result<(), String> {
    let current = schema_version(connection)?;
    if current > LATEST_SCHEMA_VERSION {
        return Err(format!(
            "This knowledge database uses schema version {current}, which is newer than this Murmur build supports."
        ));
    }
    for next in (current + 1)..=LATEST_SCHEMA_VERSION {
        let transaction = connection.transaction().map_err(db_error)?;
        apply(&transaction, next)?;
        transaction
            .pragma_update(None, "user_version", next)
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
    }
    Ok(())
}

pub fn schema_version(connection: &Connection) -> Result<u32, String> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)
}

fn apply(transaction: &Transaction<'_>, version: u32) -> Result<(), String> {
    match version {
        1 => transaction
            .execute_batch(
                r#"
                CREATE TABLE knowledge_meta (
                    key TEXT PRIMARY KEY NOT NULL,
                    value INTEGER NOT NULL
                );
                INSERT INTO knowledge_meta(key, value) VALUES ('store_revision', 0);

                CREATE TABLE knowledge_entries (
                    id TEXT PRIMARY KEY NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('replacement_rule', 'vocabulary_term', 'snippet')),
                    trigger_text TEXT NOT NULL,
                    normalized_trigger TEXT NOT NULL,
                    content_text TEXT NOT NULL,
                    aliases_json TEXT NOT NULL DEFAULT '[]',
                    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                    scope_kind TEXT NOT NULL CHECK(scope_kind IN ('global', 'app', 'project')),
                    app_bundle_id TEXT,
                    project_root TEXT,
                    provenance TEXT NOT NULL CHECK(provenance IN ('manual', 'code_scan', 'learned_correction', 'import')),
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    CHECK(
                        (scope_kind = 'global' AND app_bundle_id IS NULL AND project_root IS NULL)
                        OR (scope_kind = 'app' AND app_bundle_id IS NOT NULL AND project_root IS NULL)
                        OR (scope_kind = 'project' AND app_bundle_id IS NOT NULL AND project_root IS NOT NULL)
                    )
                );
                "#,
            )
            .map_err(db_error),
        2 => transaction
            .execute_batch(
                r#"
                CREATE INDEX knowledge_entries_listing
                    ON knowledge_entries(updated_at_ms DESC, id ASC);
                CREATE INDEX knowledge_entries_resolution
                    ON knowledge_entries(kind, enabled, normalized_trigger, scope_kind, app_bundle_id, project_root);
                CREATE INDEX knowledge_entries_scope
                    ON knowledge_entries(scope_kind, app_bundle_id, project_root);
                CREATE VIRTUAL TABLE knowledge_fts USING fts5(
                    id UNINDEXED,
                    trigger_text,
                    content_text,
                    aliases_text,
                    tokenize = 'unicode61 remove_diacritics 2'
                );
                INSERT INTO knowledge_fts(id, trigger_text, content_text, aliases_text)
                    SELECT id, trigger_text, content_text,
                           replace(replace(aliases_json, '[', ''), ']', '')
                    FROM knowledge_entries;
                "#,
            )
            .map_err(db_error),
        _ => Err("Knowledge migration sequence is incomplete.".to_string()),
    }
}

pub fn quick_check(connection: &Connection) -> Result<(), String> {
    let result: String = connection
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(db_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err("The local knowledge database failed its integrity check.".to_string())
    }
}

pub fn validate_core_schema(connection: &Connection) -> Result<(), String> {
    connection
        .prepare("SELECT key, value FROM knowledge_meta LIMIT 0")
        .and_then(|_| {
            connection.prepare(
                "SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision FROM knowledge_entries LIMIT 0",
            )
        })
        .map(|_| ())
        .map_err(db_error)
}

pub fn validate_schema(connection: &Connection) -> Result<(), String> {
    validate_core_schema(connection)?;
    connection
        .prepare("SELECT id FROM knowledge_fts LIMIT 0")
        .map(|_| ())
        .map_err(db_error)
}

pub(crate) fn db_error(error: rusqlite::Error) -> String {
    match error {
        rusqlite::Error::QueryReturnedNoRows => {
            "The requested knowledge record was not found.".to_string()
        }
        _ => "The local knowledge database operation failed.".to_string(),
    }
}
