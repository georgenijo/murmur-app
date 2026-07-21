use super::migrations::{self, db_error, LATEST_SCHEMA_VERSION};
use super::types::*;
use chrono::Utc;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension, MAIN_DB};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

const DB_FILE: &str = "knowledge.sqlite3";
const MAX_ENTRIES: u64 = 10_000;
const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRIGGER_CHARS: usize = 256;
const MAX_REPLACEMENT_CHARS: usize = 4_096;
const MAX_SNIPPET_CHARS: usize = 65_536;
const MAX_SCOPE_CHARS: usize = 4_096;
const MAX_ALIASES: usize = 16;
const MAX_BACKUPS: usize = 3;

pub struct KnowledgeRepository {
    root: PathBuf,
    db_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializationOutcome {
    Ready,
    Recovered,
    Reinitialized,
}

impl KnowledgeRepository {
    pub fn initialize(root: PathBuf) -> Result<(Self, InitializationOutcome), String> {
        fs::create_dir_all(root.join("backups")).map_err(|_| storage_error())?;
        fs::create_dir_all(root.join("quarantine")).map_err(|_| storage_error())?;
        let db_path = root.join(DB_FILE);
        let repository = Self { root, db_path };

        let outcome = if repository.db_path.exists() {
            match repository.open_checked() {
                Ok(_) => InitializationOutcome::Ready,
                Err(_) => repository.recover_corrupt_database()?,
            }
        } else {
            InitializationOutcome::Ready
        };

        let mut connection = repository.open_raw()?;
        let previous_version = migrations::schema_version(&connection)?;
        if previous_version > 0 && previous_version < LATEST_SCHEMA_VERSION {
            repository.create_backup(&connection, previous_version)?;
        }
        migrations::migrate(&mut connection)?;
        configure_connection(&connection)?;
        migrations::quick_check(&connection)?;
        migrations::validate_schema(&connection)?;
        Ok((repository, outcome))
    }

    pub fn status(&self, outcome: InitializationOutcome) -> Result<KnowledgeStoreStatus, String> {
        let connection = self.open_checked()?;
        Ok(KnowledgeStoreStatus {
            availability: match outcome {
                InitializationOutcome::Ready => StoreAvailability::Ready,
                InitializationOutcome::Recovered => StoreAvailability::Recovered,
                InitializationOutcome::Reinitialized => StoreAvailability::Reinitialized,
            },
            schema_version: migrations::schema_version(&connection)?,
            record_count: record_count(&connection)?,
            store_revision: store_revision(&connection)?,
            recovery_at_ms: (outcome != InitializationOutcome::Ready).then(now_ms),
            message: match outcome {
                InitializationOutcome::Ready => None,
                InitializationOutcome::Recovered => Some(
                    "Murmur restored personal knowledge from the newest valid local backup. Review the recovered records before relying on them."
                        .to_string(),
                ),
                InitializationOutcome::Reinitialized => Some(
                    "The damaged knowledge database was preserved, but no valid backup was available. Murmur created an empty local store."
                        .to_string(),
                ),
            },
        })
    }

    pub fn list(&self, request: KnowledgeListRequest) -> Result<KnowledgeListResponse, String> {
        let connection = self.open_checked()?;
        let limit = request
            .limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let offset = request.offset.unwrap_or(0);
        let mut joins = String::new();
        let mut where_parts = Vec::new();
        let mut values = Vec::<Value>::new();

        if let Some(query) = request
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            joins.push_str(" JOIN knowledge_fts ON knowledge_fts.id = e.id ");
            where_parts.push("knowledge_fts MATCH ?".to_string());
            values.push(Value::Text(fts_query(query)?));
        }
        if let Some(kind) = request.kind {
            where_parts.push("e.kind = ?".to_string());
            values.push(Value::Text(kind.as_str().to_string()));
        }
        if let Some(enabled) = request.enabled {
            where_parts.push("e.enabled = ?".to_string());
            values.push(Value::Integer(i64::from(enabled)));
        }
        if let Some(scope) = request
            .scope_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !matches!(scope, "global" | "app" | "project") {
                return Err("Knowledge scope filter is invalid.".to_string());
            }
            where_parts.push("e.scope_kind = ?".to_string());
            values.push(Value::Text(scope.to_string()));
        }
        if let Some(voice_command) = request.voice_command {
            where_parts.push(if voice_command {
                "e.voice_command_kind IS NOT NULL".to_string()
            } else {
                "e.voice_command_kind IS NULL".to_string()
            });
        }

        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_parts.join(" AND "))
        };
        let count_sql = format!("SELECT COUNT(*) FROM knowledge_entries e {joins}{where_clause}");
        let total: u64 = connection
            .query_row(&count_sql, params_from_iter(values.iter()), |row| {
                row.get::<_, i64>(0)
            })
            .map_err(db_error)?
            .try_into()
            .map_err(|_| "Knowledge record count is invalid.".to_string())?;

        let sql = format!(
            "SELECT e.id, e.kind, e.trigger_text, e.content_text, e.aliases_json, e.enabled, \
             e.scope_kind, e.app_bundle_id, e.project_root, e.provenance, e.created_at_ms, \
             e.updated_at_ms, e.revision, e.voice_command_kind, e.voice_command_clipboard \
             FROM knowledge_entries e {joins}{where_clause} \
             ORDER BY e.updated_at_ms DESC, e.id ASC LIMIT ? OFFSET ?"
        );
        let mut page_values = values;
        page_values.push(Value::Integer(limit.into()));
        page_values.push(Value::Integer(offset.into()));
        let mut statement = connection.prepare(&sql).map_err(db_error)?;
        let entries = statement
            .query_map(params_from_iter(page_values.iter()), row_to_entry)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let consumed = offset.saturating_add(entries.len() as u32);
        Ok(KnowledgeListResponse {
            entries,
            total,
            next_offset: (u64::from(consumed) < total).then_some(consumed),
            store_revision: store_revision(&connection)?,
        })
    }

    pub fn get(&self, id: &str) -> Result<KnowledgeEntry, String> {
        validate_id(id)?;
        let connection = self.open_checked()?;
        connection
            .query_row(
                "SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries WHERE id = ?",
                [id],
                row_to_entry,
            )
            .map_err(db_error)
    }

    pub fn upsert_manual(&self, draft: KnowledgeDraft) -> Result<KnowledgeEntry, String> {
        validate_payload(&draft.payload)?;
        validate_scope(&draft.scope)?;
        validate_voice_command(&draft.payload, &draft.scope, draft.voice_command.as_ref())?;
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        if draft.voice_command.is_some() {
            validate_voice_command_conflicts_tx(
                &transaction,
                &draft.payload.storage_parts().0,
                &draft.scope,
                draft.id.as_deref(),
            )?;
        }
        if let KnowledgePayload::Transform { name, .. } = &draft.payload {
            validate_transform_name_conflict_tx(&transaction, name, draft.id.as_deref())?;
        }
        let timestamp = now_ms();
        let (trigger, content, aliases) = draft.payload.storage_parts();
        let aliases_json = serde_json::to_string(&aliases).map_err(|_| validation_error())?;
        let normalized = normalize_key(&trigger);
        let voice_command_kind = draft
            .voice_command
            .as_ref()
            .map(|voice| voice.command_type.as_str());
        let voice_command_clipboard = draft
            .voice_command
            .as_ref()
            .is_some_and(|voice| voice.allow_clipboard_read);
        let id = match draft.id.as_deref() {
            Some(id) => {
                validate_id(id)?;
                let expected = draft.expected_revision.ok_or_else(|| {
                    "Editing knowledge requires the current record revision.".to_string()
                })?;
                let changed = transaction
                    .execute(
                        "UPDATE knowledge_entries SET kind=?, trigger_text=?, normalized_trigger=?, content_text=?, aliases_json=?, enabled=?, scope_kind=?, app_bundle_id=?, project_root=?, updated_at_ms=?, revision=revision+1, voice_command_kind=?, voice_command_clipboard=? WHERE id=? AND revision=?",
                        params![
                            draft.payload.kind().as_str(), trigger, normalized, content, aliases_json,
                            draft.enabled, draft.scope.kind(), draft.scope.bundle_id(), draft.scope.root(),
                            timestamp, voice_command_kind, voice_command_clipboard, id, revision_to_i64(expected)?,
                        ],
                    )
                    .map_err(db_error)?;
                if changed == 0 {
                    return Err(
                        "This knowledge record changed in another window. Refresh and try again."
                            .to_string(),
                    );
                }
                id.to_string()
            }
            None => {
                if record_count_tx(&transaction)? >= MAX_ENTRIES {
                    return Err(
                        "The personal knowledge store has reached its 10,000-record limit."
                            .to_string(),
                    );
                }
                let id: String = transaction
                    .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
                    .map_err(db_error)?;
                transaction
                    .execute(
                        "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'manual', ?, ?, 1, ?, ?)",
                        params![
                            id, draft.payload.kind().as_str(), trigger, normalized, content, aliases_json,
                            draft.enabled, draft.scope.kind(), draft.scope.bundle_id(), draft.scope.root(),
                            timestamp, timestamp, voice_command_kind, voice_command_clipboard,
                        ],
                    )
                    .map_err(db_error)?;
                id
            }
        };
        refresh_fts(&transaction, &id)?;
        bump_store_revision(&transaction)?;
        transaction.commit().map_err(db_error)?;
        self.get(&id)
    }

    pub fn create_learned_replacement(
        &self,
        source: String,
        replacement: String,
        scope: KnowledgeScope,
    ) -> Result<KnowledgeEntry, String> {
        let payload = KnowledgePayload::ReplacementRule {
            source: source.trim().to_string(),
            replacement: replacement.trim().to_string(),
        };
        validate_payload(&payload)?;
        validate_scope(&scope)?;

        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        for existing in entries_with_kind(&transaction, KnowledgeKind::ReplacementRule)? {
            let KnowledgePayload::ReplacementRule {
                source: existing_source,
                replacement: existing_replacement,
            } = &existing.payload
            else {
                continue;
            };
            if normalize_key(existing_source) != normalize_key(&source) || existing.scope != scope {
                continue;
            }
            if existing.voice_command.is_some() {
                return Err(
                    "A Voice Command already uses this phrase and scope. Review it in Voice Commands before teaching another correction."
                        .to_string(),
                );
            }
            if existing.enabled && existing_replacement == replacement.trim() {
                return Ok(existing);
            }
            return Err(
                "A replacement rule already uses this phrase and scope. Review or edit it in Knowledge before teaching another."
                    .to_string(),
            );
        }
        if record_count_tx(&transaction)? >= MAX_ENTRIES {
            return Err(
                "The personal knowledge store has reached its 10,000-record limit.".to_string(),
            );
        }
        let timestamp = now_ms();
        let id: String = transaction
            .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision) VALUES (?, 'replacement_rule', ?, ?, ?, '[]', 1, ?, ?, ?, 'learned_correction', ?, ?, 1)",
                params![
                    id,
                    source.trim(),
                    normalize_key(&source),
                    replacement.trim(),
                    scope.kind(),
                    scope.bundle_id(),
                    scope.root(),
                    timestamp,
                    timestamp,
                ],
            )
            .map_err(db_error)?;
        refresh_fts(&transaction, &id)?;
        bump_store_revision(&transaction)?;
        transaction.commit().map_err(db_error)?;
        self.get(&id)
    }

    pub fn enabled_replacement_rules(&self) -> Result<Vec<KnowledgeEntry>, String> {
        let connection = self.open_checked()?;
        let mut entries = entries_with_kind(&connection, KnowledgeKind::ReplacementRule)?
            .into_iter()
            .filter(|entry| entry.enabled && entry.voice_command.is_none())
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| compare_precedence(right, left));
        Ok(entries)
    }

    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<KnowledgeEntry, String> {
        validate_id(id)?;
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE knowledge_entries SET enabled=?, updated_at_ms=?, revision=revision+1 WHERE id=? AND revision=?",
                params![enabled, now_ms(), id, revision_to_i64(expected_revision)?],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(
                "This knowledge record changed in another window. Refresh and try again."
                    .to_string(),
            );
        }
        bump_store_revision(&transaction)?;
        transaction.commit().map_err(db_error)?;
        self.get(id)
    }

    pub fn delete(&self, id: &str, expected_revision: u64) -> Result<u64, String> {
        validate_id(id)?;
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let changed = transaction
            .execute(
                "DELETE FROM knowledge_entries WHERE id=? AND revision=?",
                params![id, revision_to_i64(expected_revision)?],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(
                "This knowledge record changed in another window. Refresh and try again."
                    .to_string(),
            );
        }
        transaction
            .execute("DELETE FROM knowledge_fts WHERE id=?", [id])
            .map_err(db_error)?;
        let revision = bump_store_revision(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(revision)
    }

    pub fn resolve(
        &self,
        request: KnowledgeResolveRequest,
    ) -> Result<Option<KnowledgeEntry>, String> {
        let key = normalize_key(&request.trigger);
        if key.is_empty() {
            return Err("Knowledge lookup requires a trigger.".to_string());
        }
        let connection = self.open_checked()?;
        let mut matches = entries_with_kind(&connection, request.kind)?
            .into_iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| payload_matches(&entry.payload, &key))
            .filter(|entry| {
                scope_matches(
                    &entry.scope,
                    request.bundle_id.as_deref(),
                    request.project_root.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| compare_precedence(right, left));
        Ok(matches.into_iter().next())
    }

    pub fn voice_commands_for_context(
        &self,
        bundle_id: Option<&str>,
    ) -> Result<Vec<KnowledgeEntry>, String> {
        let connection = self.open_checked()?;
        let mut statement = connection
            .prepare(
                "SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard \
                 FROM knowledge_entries WHERE enabled=1 AND voice_command_kind IS NOT NULL \
                 AND (scope_kind='global' OR (scope_kind='app' AND app_bundle_id=?)) \
                 ORDER BY created_at_ms ASC, id ASC",
            )
            .map_err(db_error)?;
        let entries = statement
            .query_map([bundle_id], row_to_entry)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(entries)
    }

    pub fn all_voice_commands(&self) -> Result<Vec<KnowledgeEntry>, String> {
        let connection = self.open_checked()?;
        let mut statement = connection
            .prepare(
                "SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard \
                 FROM knowledge_entries WHERE voice_command_kind IS NOT NULL \
                 ORDER BY created_at_ms ASC, id ASC",
            )
            .map_err(db_error)?;
        let entries = statement
            .query_map([], row_to_entry)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(entries)
    }

    pub fn migrate_legacy_voice_commands(
        &self,
        commands: &[(String, String)],
    ) -> Result<u64, String> {
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let migrated: i64 = transaction
            .query_row(
                "SELECT value FROM knowledge_meta WHERE key='legacy_voice_commands_migrated'",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if migrated != 0 {
            return Ok(0);
        }
        let timestamp = now_ms();
        let mut inserted = 0_u64;
        for (index, (phrase, replacement)) in commands.iter().enumerate() {
            let phrase = phrase.trim();
            if phrase.is_empty() {
                continue;
            }
            // Legacy settings never imposed the repository editor's newer
            // trigger/content limits. Grandfather those local pairs so an
            // upgrade cannot silently change behavior, while keeping the
            // stricter bounds for every newly created or imported command.
            let base_id = format!("legacy-voice-command-{index:08}");
            let mut id = base_id.clone();
            let mut collision = 0_u32;
            while entry_by_id_tx(&transaction, &id)?.is_some() {
                collision = collision.saturating_add(1);
                id = format!("{base_id}-migrated-{collision:04}");
            }
            let enabled = !crate::voice_commands::is_builtin_phrase(&normalize_key(phrase));
            transaction
                .execute(
                    "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) \
                     VALUES (?, 'replacement_rule', ?, ?, ?, '[]', ?, 'global', NULL, NULL, 'manual', ?, ?, 1, 'text_replacement', 0)",
                    params![id, phrase, normalize_key(phrase), replacement, enabled, timestamp, timestamp],
                )
                .map_err(db_error)?;
            refresh_fts(&transaction, &id)?;
            inserted += 1;
        }
        transaction
            .execute(
                "UPDATE knowledge_meta SET value=1 WHERE key='legacy_voice_commands_migrated'",
                [],
            )
            .map_err(db_error)?;
        if inserted > 0 {
            bump_store_revision(&transaction)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(inserted)
    }

    pub fn export_to_file(&self, path: &Path) -> Result<u64, String> {
        let entries = self.all_entries()?;
        let bundle = KnowledgeExport {
            format: EXPORT_FORMAT.to_string(),
            version: EXPORT_VERSION,
            exported_at_ms: now_ms(),
            entries,
        };
        let bytes = serde_json::to_vec_pretty(&bundle)
            .map_err(|_| "Knowledge export could not be serialized.".to_string())?;
        let parent = path
            .parent()
            .ok_or_else(|| "Choose a valid export destination.".to_string())?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Choose a valid export destination.".to_string())?;
        let temp = parent.join(format!(".{file_name}.tmp"));
        fs::write(&temp, &bytes)
            .map_err(|_| "Murmur could not write the knowledge export.".to_string())?;
        fs::rename(&temp, path).map_err(|_| {
            let _ = fs::remove_file(&temp);
            "Murmur could not publish the knowledge export.".to_string()
        })?;
        Ok(bundle.entries.len() as u64)
    }

    pub fn inspect_import(&self, path: &Path) -> Result<KnowledgeImportSummary, String> {
        let bundle = read_import(path)?;
        self.summarize_import(&bundle)
    }

    pub fn import_from_file(&self, path: &Path) -> Result<KnowledgeImportResult, String> {
        let bundle = read_import(path)?;
        let summary = self.summarize_import(&bundle)?;
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let mut imported = 0_u64;
        for entry in bundle.entries {
            if semantic_duplicate_tx(&transaction, &entry)? {
                continue;
            }
            if let Some(existing) = entry_by_id_tx(&transaction, &entry.id)? {
                if !semantic_equal(&existing, &entry) {
                    return Err(
                        "The import contains an ID that conflicts with different local knowledge."
                            .to_string(),
                    );
                }
                continue;
            }
            if entry.voice_command.is_some() {
                validate_voice_command_conflicts_tx(
                    &transaction,
                    &entry.payload.storage_parts().0,
                    &entry.scope,
                    None,
                )?;
            }
            insert_imported(&transaction, entry)?;
            imported += 1;
        }
        if imported > 0 {
            bump_store_revision(&transaction)?;
        }
        let revision = store_revision_tx(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(KnowledgeImportResult {
            imported,
            duplicates: summary.duplicates,
            store_revision: revision,
        })
    }

    pub fn delete_all(&self, expected_revision: u64) -> Result<u64, String> {
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        if store_revision_tx(&transaction)? != expected_revision {
            return Err("The knowledge store changed. Review the latest records before deleting everything.".to_string());
        }
        transaction
            .execute("DELETE FROM knowledge_entries", [])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM knowledge_fts", [])
            .map_err(db_error)?;
        let revision = bump_store_revision(&transaction)?;
        transaction.commit().map_err(db_error)?;
        connection
            .pragma_update(None, "secure_delete", "ON")
            .map_err(db_error)?;
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
            .map_err(db_error)?;
        remove_files_in(&self.root.join("backups"))?;
        remove_files_in(&self.root.join("quarantine"))?;
        Ok(revision)
    }

    fn summarize_import(&self, bundle: &KnowledgeExport) -> Result<KnowledgeImportSummary, String> {
        let connection = self.open_checked()?;
        let mut summary = KnowledgeImportSummary {
            total: bundle.entries.len() as u64,
            ..KnowledgeImportSummary::default()
        };
        for entry in &bundle.entries {
            if semantic_duplicate(&connection, entry)? {
                summary.duplicates += 1;
            } else {
                if let Some(existing) = entry_by_id(&connection, &entry.id)? {
                    if !semantic_equal(&existing, entry) {
                        return Err("The import contains an ID that conflicts with different local knowledge.".to_string());
                    }
                }
                if trigger_conflict(&connection, entry)? {
                    summary.conflicts += 1;
                }
                summary.new += 1;
            }
        }
        if record_count(&connection)?.saturating_add(summary.new) > MAX_ENTRIES {
            return Err("The import would exceed the 10,000-record knowledge limit.".to_string());
        }
        Ok(summary)
    }

    fn all_entries(&self) -> Result<Vec<KnowledgeEntry>, String> {
        let connection = self.open_checked()?;
        let mut statement = connection
            .prepare("SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries ORDER BY kind ASC, normalized_trigger ASC, scope_kind ASC, id ASC")
            .map_err(db_error)?;
        let entries = statement
            .query_map([], row_to_entry)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(entries)
    }

    fn open_raw(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|_| storage_error())
    }

    fn open_checked(&self) -> Result<Connection, String> {
        let connection = self.open_raw()?;
        configure_connection(&connection)?;
        migrations::quick_check(&connection)?;
        let version = migrations::schema_version(&connection)?;
        if version > LATEST_SCHEMA_VERSION {
            return Err(format!(
                "This knowledge database uses schema version {version}, which is newer than this Murmur build supports."
            ));
        }
        if version == LATEST_SCHEMA_VERSION {
            migrations::validate_schema(&connection)?;
        }
        Ok(connection)
    }

    fn create_backup(&self, source: &Connection, version: u32) -> Result<(), String> {
        let name = format!("knowledge-v{version}-{}.sqlite3", now_ms());
        let path = self.root.join("backups").join(name);
        source.backup(MAIN_DB, &path, None).map_err(db_error)?;
        let check = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| storage_error())?;
        migrations::quick_check(&check)?;
        retain_newest(&self.root.join("backups"), MAX_BACKUPS)?;
        Ok(())
    }

    fn recover_corrupt_database(&self) -> Result<InitializationOutcome, String> {
        let quarantine = self
            .root
            .join("quarantine")
            .join(format!("knowledge-corrupt-{}.sqlite3", now_ms()));
        fs::rename(&self.db_path, quarantine).map_err(|_| storage_error())?;
        remove_sidecars(&self.db_path);

        let mut backups = backup_files_newest_first(&self.root.join("backups"))?;
        for backup in backups.drain(..) {
            let valid = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| storage_error())
                .and_then(|connection| {
                    migrations::quick_check(&connection)?;
                    let version = migrations::schema_version(&connection)?;
                    if version == 0 || version > LATEST_SCHEMA_VERSION {
                        return Err(
                            "The backup schema is not supported by this Murmur build.".to_string()
                        );
                    }
                    if version < LATEST_SCHEMA_VERSION {
                        migrations::validate_core_schema(&connection)?;
                    } else {
                        migrations::validate_schema(&connection)?;
                    }
                    Ok(())
                })
                .is_ok();
            if !valid {
                continue;
            }
            fs::copy(&backup, &self.db_path).map_err(|_| storage_error())?;
            return Ok(InitializationOutcome::Recovered);
        }
        Ok(InitializationOutcome::Reinitialized)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(db_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(db_error)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeEntry> {
    let kind: String = row.get(1)?;
    let trigger: String = row.get(2)?;
    let content: String = row.get(3)?;
    let aliases_json: String = row.get(4)?;
    let aliases: Vec<String> = serde_json::from_str(&aliases_json).unwrap_or_default();
    let scope_kind: String = row.get(6)?;
    let bundle_id: Option<String> = row.get(7)?;
    let project_root: Option<String> = row.get(8)?;
    let payload = match kind.as_str() {
        "replacement_rule" => KnowledgePayload::ReplacementRule {
            source: trigger,
            replacement: content,
        },
        "vocabulary_term" => KnowledgePayload::VocabularyTerm {
            written: trigger,
            aliases,
        },
        "snippet" => KnowledgePayload::Snippet {
            trigger,
            body: content,
        },
        "transform" => KnowledgePayload::Transform {
            name: trigger,
            instruction: content,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let scope = match scope_kind.as_str() {
        "global" => KnowledgeScope::Global,
        "app" => KnowledgeScope::App {
            bundle_id: bundle_id.ok_or(rusqlite::Error::InvalidQuery)?,
        },
        "project" => KnowledgeScope::Project {
            bundle_id: bundle_id.ok_or(rusqlite::Error::InvalidQuery)?,
            root: project_root.ok_or(rusqlite::Error::InvalidQuery)?,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let provenance: String = row.get(9)?;
    let voice_command_kind: Option<String> = row.get(13)?;
    let voice_command = match voice_command_kind {
        Some(kind) => Some(VoiceCommandMetadata {
            command_type: VoiceCommandKind::parse(&kind)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
            allow_clipboard_read: row.get(14)?,
        }),
        None => None,
    };
    Ok(KnowledgeEntry {
        id: row.get(0)?,
        payload,
        enabled: row.get(5)?,
        scope,
        provenance: KnowledgeProvenance::parse(&provenance)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
        revision: u64::try_from(row.get::<_, i64>(12)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        voice_command,
    })
}

fn validate_payload(payload: &KnowledgePayload) -> Result<(), String> {
    let (trigger, content, aliases) = payload.storage_parts();
    if trigger.trim().is_empty() || trigger.chars().count() > MAX_TRIGGER_CHARS {
        return Err(
            "Knowledge triggers and written terms must be between 1 and 256 characters."
                .to_string(),
        );
    }
    match payload {
        KnowledgePayload::ReplacementRule { .. } => {
            if content.chars().count() > MAX_REPLACEMENT_CHARS {
                return Err("Replacement text must be between 1 and 4,096 characters.".to_string());
            }
        }
        KnowledgePayload::VocabularyTerm { .. } => {
            if aliases.len() > MAX_ALIASES {
                return Err("Vocabulary terms support at most 16 aliases.".to_string());
            }
            let mut normalized = std::collections::HashSet::new();
            for alias in aliases {
                let key = normalize_key(&alias);
                if key.is_empty() || alias.chars().count() > MAX_TRIGGER_CHARS {
                    return Err(
                        "Vocabulary aliases must be between 1 and 256 characters.".to_string()
                    );
                }
                if key == normalize_key(&trigger) || !normalized.insert(key) {
                    return Err(
                        "Vocabulary aliases must be unique and different from the written term."
                            .to_string(),
                    );
                }
            }
        }
        KnowledgePayload::Snippet { .. } => {
            if content.is_empty() || content.chars().count() > MAX_SNIPPET_CHARS {
                return Err("Snippet bodies must be between 1 and 65,536 characters.".to_string());
            }
        }
        KnowledgePayload::Transform { .. } => {
            if content.is_empty() {
                return Err(
                    "Transform instructions must be 1 to 4,096 bytes.".to_string(),
                );
            }
            // The local-LLM sidecar protocol caps instruction bytes at
            // MAX_INSTRUCTION_BYTES (4096); enforce the same bound (bytes,
            // not chars) here so an oversized saved transform is rejected at
            // save time instead of saving fine and always failing later with
            // an opaque invalid_request (issue #312 round 2 D1 fix #3).
            if content.len() > murmur_local_llm_protocol::MAX_INSTRUCTION_BYTES {
                return Err(format!(
                    "Transform instructions must be at most {} bytes.",
                    murmur_local_llm_protocol::MAX_INSTRUCTION_BYTES
                ));
            }
        }
    }
    Ok(())
}

fn validate_scope(scope: &KnowledgeScope) -> Result<(), String> {
    let values = [scope.bundle_id(), scope.root()];
    if values
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty() || value.chars().count() > MAX_SCOPE_CHARS)
    {
        return Err(
            "Knowledge scope identifiers must be between 1 and 4,096 characters.".to_string(),
        );
    }
    Ok(())
}

fn validate_voice_command(
    payload: &KnowledgePayload,
    scope: &KnowledgeScope,
    voice_command: Option<&VoiceCommandMetadata>,
) -> Result<(), String> {
    let Some(voice_command) = voice_command else {
        if matches!(payload, KnowledgePayload::ReplacementRule { replacement, .. } if replacement.trim().is_empty())
        {
            return Err("Replacement text must be between 1 and 4,096 characters.".to_string());
        }
        return Ok(());
    };
    if matches!(scope, KnowledgeScope::Project { .. }) {
        return Err("Voice commands support global or per-app scope.".to_string());
    }
    match (voice_command.command_type, payload) {
        (VoiceCommandKind::TextReplacement, KnowledgePayload::ReplacementRule { .. }) => {}
        (VoiceCommandKind::Snippet, KnowledgePayload::Snippet { body, .. }) => {
            crate::voice_commands::validate_snippet_template(
                body,
                voice_command.allow_clipboard_read,
            )?;
        }
        _ => {
            return Err(
                "Voice command type does not match its stored knowledge payload.".to_string(),
            );
        }
    }
    if voice_command.allow_clipboard_read && voice_command.command_type != VoiceCommandKind::Snippet
    {
        return Err("Only snippet commands can request clipboard access.".to_string());
    }
    Ok(())
}

fn validate_voice_command_conflicts_tx(
    transaction: &rusqlite::Transaction<'_>,
    trigger: &str,
    scope: &KnowledgeScope,
    editing_id: Option<&str>,
) -> Result<(), String> {
    let normalized = normalize_key(trigger);
    if crate::voice_commands::is_builtin_phrase(&normalized) {
        return Err("That phrase is reserved by a built-in Voice Command.".to_string());
    }
    let mut statement = transaction
        .prepare(
            "SELECT id, scope_kind, app_bundle_id FROM knowledge_entries \
             WHERE voice_command_kind IS NOT NULL AND normalized_trigger=?",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map([normalized], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(db_error)?;
    for row in rows {
        let (id, scope_kind, bundle_id) = row.map_err(db_error)?;
        if editing_id == Some(id.as_str()) {
            continue;
        }
        let same_scope = match scope {
            KnowledgeScope::Global => scope_kind == "global",
            KnowledgeScope::App {
                bundle_id: required,
            } => scope_kind == "app" && bundle_id.as_deref() == Some(required.as_str()),
            KnowledgeScope::Project { .. } => false,
        };
        if same_scope {
            return Err(
                "A Voice Command with that phrase already exists in this scope.".to_string(),
            );
        }
    }
    Ok(())
}

/// Reject an exact duplicate saved-transform name (issue #312 round 2 D1
/// fix #5). Uses the same normalization as preset matching and
/// `transform_flow::resolve_saved_transform` (case- and punctuation-
/// insensitive) so two names that would be indistinguishable when spoken
/// cannot both be saved. Preset-name/alias shadowing is intentionally NOT
/// rejected here — it is only a UI-level warning, since a saved transform
/// with that name is still valid data (it is simply unreachable by voice
/// until renamed or the preset changes).
fn validate_transform_name_conflict_tx(
    transaction: &rusqlite::Transaction<'_>,
    name: &str,
    editing_id: Option<&str>,
) -> Result<(), String> {
    let normalized = crate::transform_presets::normalize(name);
    if normalized.is_empty() {
        return Ok(());
    }
    for existing in entries_with_kind(transaction, KnowledgeKind::Transform)? {
        if editing_id == Some(existing.id.as_str()) {
            continue;
        }
        if let KnowledgePayload::Transform {
            name: existing_name,
            ..
        } = &existing.payload
        {
            if crate::transform_presets::normalize(existing_name) == normalized {
                return Err("A saved transform with that name already exists.".to_string());
            }
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        Err("Knowledge record ID is invalid.".to_string())
    } else {
        Ok(())
    }
}

pub fn normalize_key(value: &str) -> String {
    value
        .split_whitespace()
        .flat_map(|word| {
            word.chars()
                .flat_map(char::to_lowercase)
                .chain(std::iter::once(' '))
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn fts_query(query: &str) -> Result<String, String> {
    if query.chars().count() > 256 {
        return Err("Knowledge search is limited to 256 characters.".to_string());
    }
    let terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err("Knowledge search needs at least one letter or number.".to_string());
    }
    Ok(terms.join(" AND "))
}

fn refresh_fts(transaction: &rusqlite::Transaction<'_>, id: &str) -> Result<(), String> {
    transaction
        .execute("DELETE FROM knowledge_fts WHERE id=?", [id])
        .map_err(db_error)?;
    transaction
        .execute(
            "INSERT INTO knowledge_fts(id, trigger_text, content_text, aliases_text) SELECT id, trigger_text, content_text, replace(replace(aliases_json, '[', ''), ']', '') FROM knowledge_entries WHERE id=?",
            [id],
        )
        .map_err(db_error)?;
    Ok(())
}

fn bump_store_revision(transaction: &rusqlite::Transaction<'_>) -> Result<u64, String> {
    transaction
        .execute(
            "UPDATE knowledge_meta SET value=value+1 WHERE key='store_revision'",
            [],
        )
        .map_err(db_error)?;
    store_revision_tx(transaction)
}

fn store_revision(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row(
            "SELECT value FROM knowledge_meta WHERE key='store_revision'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(db_error)?
        .try_into()
        .map_err(|_| "Knowledge store revision is invalid.".to_string())
}

fn store_revision_tx(transaction: &rusqlite::Transaction<'_>) -> Result<u64, String> {
    transaction
        .query_row(
            "SELECT value FROM knowledge_meta WHERE key='store_revision'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(db_error)?
        .try_into()
        .map_err(|_| "Knowledge store revision is invalid.".to_string())
}

fn record_count(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row("SELECT COUNT(*) FROM knowledge_entries", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(db_error)?
        .try_into()
        .map_err(|_| "Knowledge record count is invalid.".to_string())
}

fn record_count_tx(transaction: &rusqlite::Transaction<'_>) -> Result<u64, String> {
    transaction
        .query_row("SELECT COUNT(*) FROM knowledge_entries", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(db_error)?
        .try_into()
        .map_err(|_| "Knowledge record count is invalid.".to_string())
}

fn revision_to_i64(revision: u64) -> Result<i64, String> {
    revision
        .try_into()
        .map_err(|_| "Knowledge record revision is invalid.".to_string())
}

fn payload_matches(payload: &KnowledgePayload, normalized: &str) -> bool {
    let (trigger, _, aliases) = payload.storage_parts();
    normalize_key(&trigger) == normalized
        || aliases
            .iter()
            .any(|alias| normalize_key(alias) == normalized)
}

fn scope_matches(
    scope: &KnowledgeScope,
    bundle_id: Option<&str>,
    project_root: Option<&str>,
) -> bool {
    match scope {
        KnowledgeScope::Global => true,
        KnowledgeScope::App {
            bundle_id: required,
        } => bundle_id == Some(required.as_str()),
        KnowledgeScope::Project {
            bundle_id: required,
            root,
        } => bundle_id == Some(required.as_str()) && project_root == Some(root.as_str()),
    }
}

fn compare_precedence(left: &KnowledgeEntry, right: &KnowledgeEntry) -> Ordering {
    left.scope
        .specificity()
        .cmp(&right.scope.specificity())
        .then(
            left.provenance
                .precedence()
                .cmp(&right.provenance.precedence()),
        )
        .then(left.updated_at_ms.cmp(&right.updated_at_ms))
        .then_with(|| right.id.cmp(&left.id))
}

fn read_import(path: &Path) -> Result<KnowledgeExport, String> {
    let metadata = fs::metadata(path)
        .map_err(|_| "Murmur could not read the selected knowledge import.".to_string())?;
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err("Knowledge imports are limited to 8 MiB.".to_string());
    }
    let bytes = fs::read(path)
        .map_err(|_| "Murmur could not read the selected knowledge import.".to_string())?;
    let bundle: KnowledgeExport = serde_json::from_slice(&bytes)
        .map_err(|_| "The selected file is not a valid Murmur knowledge export.".to_string())?;
    if bundle.format != EXPORT_FORMAT || !matches!(bundle.version, 1 | 2 | 3) {
        return Err(
            "The selected knowledge export format is not supported by this Murmur build."
                .to_string(),
        );
    }
    if bundle.entries.len() as u64 > MAX_ENTRIES {
        return Err("Knowledge imports support at most 10,000 records.".to_string());
    }
    let mut ids = std::collections::HashSet::new();
    for entry in &bundle.entries {
        validate_id(&entry.id)?;
        validate_payload(&entry.payload)?;
        validate_scope(&entry.scope)?;
        validate_voice_command(&entry.payload, &entry.scope, entry.voice_command.as_ref())?;
        if !ids.insert(entry.id.clone()) {
            return Err("The import contains duplicate record IDs.".to_string());
        }
    }
    Ok(bundle)
}

fn semantic_equal(left: &KnowledgeEntry, right: &KnowledgeEntry) -> bool {
    left.payload == right.payload
        && left.scope == right.scope
        && left.enabled == right.enabled
        && left.voice_command == right.voice_command
}

fn semantic_duplicate(connection: &Connection, entry: &KnowledgeEntry) -> Result<bool, String> {
    let candidates = entries_with_kind(connection, entry.payload.kind())?;
    Ok(candidates
        .iter()
        .any(|candidate| semantic_equal(candidate, entry)))
}

fn semantic_duplicate_tx(
    transaction: &rusqlite::Transaction<'_>,
    entry: &KnowledgeEntry,
) -> Result<bool, String> {
    let mut statement = transaction.prepare("SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries WHERE kind=?").map_err(db_error)?;
    let candidates = statement
        .query_map([entry.payload.kind().as_str()], row_to_entry)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(candidates
        .iter()
        .any(|candidate| semantic_equal(candidate, entry)))
}

fn trigger_conflict(connection: &Connection, entry: &KnowledgeEntry) -> Result<bool, String> {
    let key = normalize_key(&entry.payload.storage_parts().0);
    Ok(entries_with_kind(connection, entry.payload.kind())?
        .iter()
        .any(|candidate| {
            payload_matches(&candidate.payload, &key)
                && candidate.scope == entry.scope
                && !semantic_equal(candidate, entry)
        }))
}

fn entries_with_kind(
    connection: &Connection,
    kind: KnowledgeKind,
) -> Result<Vec<KnowledgeEntry>, String> {
    let mut statement = connection.prepare("SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries WHERE kind=?").map_err(db_error)?;
    let entries = statement
        .query_map([kind.as_str()], row_to_entry)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(entries)
}

fn entry_by_id(connection: &Connection, id: &str) -> Result<Option<KnowledgeEntry>, String> {
    connection.query_row("SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries WHERE id=?", [id], row_to_entry).optional().map_err(db_error)
}

fn entry_by_id_tx(
    transaction: &rusqlite::Transaction<'_>,
    id: &str,
) -> Result<Option<KnowledgeEntry>, String> {
    transaction.query_row("SELECT id, kind, trigger_text, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard FROM knowledge_entries WHERE id=?", [id], row_to_entry).optional().map_err(db_error)
}

fn insert_imported(
    transaction: &rusqlite::Transaction<'_>,
    mut entry: KnowledgeEntry,
) -> Result<(), String> {
    entry.provenance = KnowledgeProvenance::Import;
    let (trigger, content, aliases) = entry.payload.storage_parts();
    let aliases_json = serde_json::to_string(&aliases).map_err(|_| validation_error())?;
    transaction.execute(
        "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'import', ?, ?, 1, ?, ?)",
        params![entry.id, entry.payload.kind().as_str(), trigger, normalize_key(&trigger), content, aliases_json, entry.enabled, entry.scope.kind(), entry.scope.bundle_id(), entry.scope.root(), entry.created_at_ms, entry.updated_at_ms, entry.voice_command.as_ref().map(|voice| voice.command_type.as_str()), entry.voice_command.as_ref().is_some_and(|voice| voice.allow_clipboard_read)],
    ).map_err(db_error)?;
    refresh_fts(transaction, &entry.id)
}

fn files_newest_first(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = fs::read_dir(directory)
        .map_err(|_| storage_error())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    Ok(files)
}

fn backup_files_newest_first(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = files_newest_first(directory)?
        .into_iter()
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sqlite3"))
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    Ok(files)
}

fn retain_newest(directory: &Path, keep: usize) -> Result<(), String> {
    for path in backup_files_newest_first(directory)?.into_iter().skip(keep) {
        fs::remove_file(&path).map_err(|_| storage_error())?;
        remove_sidecars(&path);
    }
    Ok(())
}

fn remove_files_in(directory: &Path) -> Result<(), String> {
    for path in files_newest_first(directory)? {
        fs::remove_file(path).map_err(|_| storage_error())?;
    }
    Ok(())
}

fn remove_sidecars(db_path: &Path) {
    let path = db_path.to_string_lossy();
    let _ = fs::remove_file(format!("{path}-wal"));
    let _ = fs::remove_file(format!("{path}-shm"));
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn storage_error() -> String {
    "Murmur could not access the local personal knowledge store.".to_string()
}

fn validation_error() -> String {
    "Knowledge record validation failed.".to_string()
}
