mod migrations;
mod repository;
mod types;

pub(crate) use repository::normalize_key;
pub use repository::{InitializationOutcome, KnowledgeRepository};
pub use types::*;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct KnowledgeStore {
    inner: Arc<Mutex<KnowledgeStoreInner>>,
}

#[derive(Default)]
struct KnowledgeStoreInner {
    root: Option<PathBuf>,
    repository: Option<KnowledgeRepository>,
    outcome: Option<InitializationOutcome>,
    status: KnowledgeStoreStatus,
}

impl KnowledgeStore {
    pub fn initialize(&self, root: PathBuf) -> KnowledgeStoreStatus {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.root = Some(root.clone());
        match KnowledgeRepository::initialize(root) {
            Ok((repository, outcome)) => match repository.status(outcome) {
                Ok(status) => {
                    inner.repository = Some(repository);
                    inner.outcome = Some(outcome);
                    inner.status = status.clone();
                    status
                }
                Err(error) => unavailable(&mut inner, error),
            },
            Err(error) => unavailable(&mut inner, error),
        }
    }

    pub fn retry(&self) -> KnowledgeStoreStatus {
        let root = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .root
            .clone();
        match root {
            Some(root) => self.initialize(root),
            None => self.status(),
        }
    }

    pub fn status(&self) -> KnowledgeStoreStatus {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
            .clone()
    }

    fn with_repository<T>(
        &self,
        action: impl FnOnce(&KnowledgeRepository) -> Result<T, String>,
    ) -> Result<T, String> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let repository = inner.repository.as_ref().ok_or_else(|| {
            inner
                .status
                .message
                .clone()
                .unwrap_or_else(|| "The local knowledge store is unavailable.".to_string())
        })?;
        action(repository)
    }

    pub fn list(&self, request: KnowledgeListRequest) -> Result<KnowledgeListResponse, String> {
        self.with_repository(|repository| repository.list(request))
    }

    pub fn get(&self, id: &str) -> Result<KnowledgeEntry, String> {
        self.with_repository(|repository| repository.get(id))
    }

    pub fn upsert_manual(&self, draft: KnowledgeDraft) -> Result<KnowledgeEntry, String> {
        let entry = self.with_repository(|repository| repository.upsert_manual(draft))?;
        self.refresh_status();
        Ok(entry)
    }

    pub fn create_learned_replacement(
        &self,
        source: String,
        replacement: String,
        scope: KnowledgeScope,
    ) -> Result<KnowledgeEntry, String> {
        let entry = self.with_repository(|repository| {
            repository.create_learned_replacement(source, replacement, scope)
        })?;
        self.refresh_status();
        Ok(entry)
    }

    pub fn enabled_replacement_rules(&self) -> Result<Vec<KnowledgeEntry>, String> {
        self.with_repository(KnowledgeRepository::enabled_replacement_rules)
    }

    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<KnowledgeEntry, String> {
        let entry = self
            .with_repository(|repository| repository.set_enabled(id, enabled, expected_revision))?;
        self.refresh_status();
        Ok(entry)
    }

    pub fn delete(&self, id: &str, expected_revision: u64) -> Result<u64, String> {
        let revision =
            self.with_repository(|repository| repository.delete(id, expected_revision))?;
        self.refresh_status();
        Ok(revision)
    }

    pub fn resolve(
        &self,
        request: KnowledgeResolveRequest,
    ) -> Result<Option<KnowledgeEntry>, String> {
        self.with_repository(|repository| repository.resolve(request))
    }

    pub fn voice_commands_for_context(
        &self,
        bundle_id: Option<&str>,
    ) -> Result<Vec<KnowledgeEntry>, String> {
        self.with_repository(|repository| repository.voice_commands_for_context(bundle_id))
    }

    pub fn all_voice_commands(&self) -> Result<Vec<KnowledgeEntry>, String> {
        self.with_repository(KnowledgeRepository::all_voice_commands)
    }

    pub fn migrate_legacy_voice_commands(
        &self,
        commands: &[(String, String)],
    ) -> Result<u64, String> {
        let inserted =
            self.with_repository(|repository| repository.migrate_legacy_voice_commands(commands))?;
        self.refresh_status();
        Ok(inserted)
    }

    pub fn export_to_file(&self, path: &Path) -> Result<u64, String> {
        self.with_repository(|repository| repository.export_to_file(path))
    }

    pub fn inspect_import(&self, path: &Path) -> Result<KnowledgeImportSummary, String> {
        self.with_repository(|repository| repository.inspect_import(path))
    }

    pub fn import_from_file(&self, path: &Path) -> Result<KnowledgeImportResult, String> {
        let result = self.with_repository(|repository| repository.import_from_file(path))?;
        self.refresh_status();
        Ok(result)
    }

    pub fn delete_all(&self, expected_revision: u64) -> Result<u64, String> {
        let revision =
            self.with_repository(|repository| repository.delete_all(expected_revision))?;
        self.refresh_status();
        Ok(revision)
    }

    fn refresh_status(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let (Some(repository), Some(outcome)) = (inner.repository.as_ref(), inner.outcome) {
            if let Ok(mut status) = repository.status(outcome) {
                if inner.status.recovery_at_ms.is_some() {
                    status.recovery_at_ms = inner.status.recovery_at_ms;
                }
                inner.status = status;
            }
        }
    }
}

fn unavailable(inner: &mut KnowledgeStoreInner, error: String) -> KnowledgeStoreStatus {
    inner.repository = None;
    inner.outcome = None;
    inner.status = KnowledgeStoreStatus {
        availability: StoreAvailability::Unavailable,
        message: Some(error),
        ..KnowledgeStoreStatus::default()
    };
    inner.status.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn store() -> (TempDir, KnowledgeStore) {
        let temp = tempfile::tempdir().unwrap();
        let store = KnowledgeStore::default();
        let status = store.initialize(temp.path().join("knowledge"));
        assert_eq!(status.availability, StoreAvailability::Ready);
        (temp, store)
    }

    fn replacement(source: &str, replacement: &str, scope: KnowledgeScope) -> KnowledgeDraft {
        KnowledgeDraft {
            id: None,
            expected_revision: None,
            payload: KnowledgePayload::ReplacementRule {
                source: source.to_string(),
                replacement: replacement.to_string(),
            },
            enabled: true,
            scope,
            voice_command: None,
        }
    }

    fn voice_command(
        phrase: &str,
        content: &str,
        command_type: VoiceCommandKind,
        scope: KnowledgeScope,
    ) -> KnowledgeDraft {
        KnowledgeDraft {
            id: None,
            expected_revision: None,
            payload: match command_type {
                VoiceCommandKind::TextReplacement => KnowledgePayload::ReplacementRule {
                    source: phrase.to_string(),
                    replacement: content.to_string(),
                },
                VoiceCommandKind::Snippet => KnowledgePayload::Snippet {
                    trigger: phrase.to_string(),
                    body: content.to_string(),
                },
            },
            enabled: true,
            scope,
            voice_command: Some(VoiceCommandMetadata {
                command_type,
                allow_clipboard_read: false,
            }),
        }
    }

    #[test]
    fn persists_crud_search_and_enabled_state_across_reopen() {
        let (temp, store) = store();
        let created = store
            .upsert_manual(replacement(
                "George Neo",
                "George Nijo",
                KnowledgeScope::Global,
            ))
            .unwrap();
        assert_eq!(created.revision, 1);
        let page = store
            .list(KnowledgeListRequest {
                query: Some("Nijo".to_string()),
                ..KnowledgeListRequest::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);

        let disabled = store
            .set_enabled(&created.id, false, created.revision)
            .unwrap();
        assert!(!disabled.enabled);
        assert!(store
            .set_enabled(&created.id, true, created.revision)
            .is_err());

        let reopened = KnowledgeStore::default();
        let status = reopened.initialize(temp.path().join("knowledge"));
        assert_eq!(status.record_count, 1);
        assert!(!reopened.get(&created.id).unwrap().enabled);
        assert!(reopened.delete(&created.id, disabled.revision).is_ok());
        assert_eq!(reopened.status().record_count, 0);
    }

    #[test]
    fn learned_replacements_require_an_explicit_create_and_keep_provenance() {
        let (_temp, store) = store();
        let learned = store
            .create_learned_replacement(
                "use recording state".to_string(),
                "useRecordingState".to_string(),
                KnowledgeScope::Global,
            )
            .unwrap();
        assert_eq!(learned.provenance, KnowledgeProvenance::LearnedCorrection);
        assert_eq!(
            store.enabled_replacement_rules().unwrap(),
            vec![learned.clone()]
        );

        let duplicate = store
            .create_learned_replacement(
                " Use   Recording State ".to_string(),
                "useRecordingState".to_string(),
                KnowledgeScope::Global,
            )
            .unwrap();
        assert_eq!(duplicate.id, learned.id);
        assert!(store
            .create_learned_replacement(
                "use recording state".to_string(),
                "differentValue".to_string(),
                KnowledgeScope::Global,
            )
            .unwrap_err()
            .contains("already uses"));
    }

    #[test]
    fn voice_command_replacements_never_enter_smart_correction_or_masquerade_as_learned() {
        let (_temp, store) = store();
        let command = store
            .upsert_manual(voice_command(
                "my signature",
                "Regards, George",
                VoiceCommandKind::TextReplacement,
                KnowledgeScope::Global,
            ))
            .unwrap();
        assert!(command.voice_command.is_some());
        assert!(store.enabled_replacement_rules().unwrap().is_empty());

        let error = store
            .create_learned_replacement(
                "my signature".to_string(),
                "Regards, George".to_string(),
                KnowledgeScope::Global,
            )
            .unwrap_err();
        assert!(error.contains("Voice Command"));
    }

    #[test]
    fn resolves_scope_then_provenance_then_time_then_id() {
        let (_temp, store) = store();
        let global = store
            .upsert_manual(replacement("use hook", "global", KnowledgeScope::Global))
            .unwrap();
        let app = store
            .upsert_manual(replacement(
                "use hook",
                "app",
                KnowledgeScope::App {
                    bundle_id: "com.editor".to_string(),
                },
            ))
            .unwrap();
        let project = store
            .upsert_manual(replacement(
                "use hook",
                "project",
                KnowledgeScope::Project {
                    bundle_id: "com.editor".to_string(),
                    root: "/project".to_string(),
                },
            ))
            .unwrap();

        let resolved = store
            .resolve(KnowledgeResolveRequest {
                kind: KnowledgeKind::ReplacementRule,
                trigger: "Use   Hook".to_string(),
                bundle_id: Some("com.editor".to_string()),
                project_root: Some("/project".to_string()),
            })
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id, project.id);

        let app_resolved = store
            .resolve(KnowledgeResolveRequest {
                kind: KnowledgeKind::ReplacementRule,
                trigger: "use hook".to_string(),
                bundle_id: Some("com.editor".to_string()),
                project_root: None,
            })
            .unwrap()
            .unwrap();
        assert_eq!(app_resolved.id, app.id);
        assert_ne!(app_resolved.id, global.id);
    }

    #[test]
    fn export_import_and_delete_all_are_atomic_and_bounded() {
        let (temp, store) = store();
        let created = store
            .upsert_manual(replacement("alpha", "beta", KnowledgeScope::Global))
            .unwrap();
        let export = temp.path().join("knowledge.json");
        assert_eq!(store.export_to_file(&export).unwrap(), 1);
        let before_delete = store.status();
        store.delete_all(before_delete.store_revision).unwrap();
        assert_eq!(store.status().record_count, 0);
        let preview = store.inspect_import(&export).unwrap();
        assert_eq!(preview.new, 1);
        let imported = store.import_from_file(&export).unwrap();
        assert_eq!(imported.imported, 1);
        assert_eq!(
            store.get(&created.id).unwrap().provenance,
            KnowledgeProvenance::Import
        );
        assert_eq!(store.import_from_file(&export).unwrap().duplicates, 1);
    }

    #[test]
    fn voice_command_conflicts_are_scope_aware_and_builtins_are_reserved() {
        let (_temp, store) = store();
        store
            .upsert_manual(voice_command(
                "my signature",
                "Global",
                VoiceCommandKind::TextReplacement,
                KnowledgeScope::Global,
            ))
            .unwrap();
        assert!(store
            .upsert_manual(voice_command(
                "MY   SIGNATURE",
                "Duplicate",
                VoiceCommandKind::Snippet,
                KnowledgeScope::Global,
            ))
            .unwrap_err()
            .contains("already exists"));
        store
            .upsert_manual(voice_command(
                "my signature",
                "Mail only",
                VoiceCommandKind::Snippet,
                KnowledgeScope::App {
                    bundle_id: "com.apple.mail".to_string(),
                },
            ))
            .unwrap();
        assert!(store
            .upsert_manual(voice_command(
                "new line",
                "reserved",
                VoiceCommandKind::TextReplacement,
                KnowledgeScope::Global,
            ))
            .unwrap_err()
            .contains("built-in"));

        let mail = crate::voice_commands::commands_from_knowledge(
            store
                .voice_commands_for_context(Some("com.apple.mail"))
                .unwrap(),
        );
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].content, "Mail only");
        assert!(mail[0].app_scoped);
        let notes = crate::voice_commands::commands_from_knowledge(
            store
                .voice_commands_for_context(Some("com.apple.Notes"))
                .unwrap(),
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].content, "Global");
    }

    #[test]
    fn legacy_voice_command_migration_is_idempotent_and_preserves_literal_pairs() {
        let (_temp, store) = store();
        let legacy = vec![
            ("first command".to_string(), "One".to_string()),
            ("remove phrase".to_string(), "".to_string()),
            ("new line".to_string(), "never ran".to_string()),
        ];
        assert_eq!(store.migrate_legacy_voice_commands(&legacy).unwrap(), 3);
        assert_eq!(store.migrate_legacy_voice_commands(&legacy).unwrap(), 0);
        let entries = store.voice_commands_for_context(None).unwrap();
        assert_eq!(entries.len(), 2, "built-in collision migrates disabled");
        assert_eq!(entries[0].id, "legacy-voice-command-00000000");
        assert_eq!(entries[1].id, "legacy-voice-command-00000001");
        assert!(matches!(
            &entries[1].payload,
            KnowledgePayload::ReplacementRule { replacement, .. } if replacement.is_empty()
        ));
        assert_eq!(store.status().record_count, 3);
    }

    #[test]
    fn legacy_voice_command_migration_never_silently_drops_existing_pairs() {
        let (temp, store) = store();
        store
            .upsert_manual(replacement("seed", "record", KnowledgeScope::Global))
            .unwrap();
        let export = temp.path().join("collision.json");
        store.export_to_file(&export).unwrap();
        let mut bundle: KnowledgeExport =
            serde_json::from_slice(&std::fs::read(&export).unwrap()).unwrap();
        bundle.entries[0].id = "legacy-voice-command-00000000".to_string();
        std::fs::write(&export, serde_json::to_vec_pretty(&bundle).unwrap()).unwrap();
        let revision = store.status().store_revision;
        store.delete_all(revision).unwrap();
        store.import_from_file(&export).unwrap();

        let long_replacement = "r".repeat(4_097);
        let long_phrase = "p".repeat(257);
        let legacy = vec![
            ("collision command".to_string(), long_replacement.clone()),
            (long_phrase.clone(), "still migrated".to_string()),
        ];
        assert_eq!(store.migrate_legacy_voice_commands(&legacy).unwrap(), 2);
        let entries = store.all_voice_commands().unwrap();
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].id, "legacy-voice-command-00000000");
        assert!(matches!(
            &entries[0].payload,
            KnowledgePayload::ReplacementRule { source, replacement }
                if source == "collision command" && replacement == &long_replacement
        ));
        assert!(matches!(
            &entries[1].payload,
            KnowledgePayload::ReplacementRule { source, replacement }
                if source == &long_phrase && replacement == "still migrated"
        ));
        assert_eq!(store.status().record_count, 3);
    }

    #[test]
    fn migrates_v1_with_backup_and_recovers_only_inside_store_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("knowledge");
        std::fs::create_dir_all(root.join("backups")).unwrap();
        std::fs::create_dir_all(root.join("quarantine")).unwrap();
        let db = root.join("knowledge.sqlite3");
        let mut connection = Connection::open(&db).unwrap();
        migrations::migrate_to_for_test(&mut connection, 1).unwrap();
        drop(connection);

        let store = KnowledgeStore::default();
        let status = store.initialize(root.clone());
        assert_eq!(status.schema_version, 4);
        assert_eq!(
            std::fs::read_dir(root.join("backups"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("sqlite3")
                })
                .count(),
            1
        );

        let preserved = store
            .upsert_manual(replacement(
                "recover this",
                "still present",
                KnowledgeScope::Global,
            ))
            .unwrap();

        let valid_backup = root
            .join("backups")
            .join("knowledge-v2-9999999999999.sqlite3");
        Connection::open(&db)
            .unwrap()
            .backup(rusqlite::MAIN_DB, &valid_backup, None)
            .unwrap();
        std::fs::write(
            root.join("backups")
                .join("knowledge-v2-9999999999999.sqlite3-wal"),
            b"",
        )
        .unwrap();
        std::fs::write(&db, b"not a sqlite database").unwrap();
        let recovered = KnowledgeStore::default();
        let recovered_status = recovered.initialize(root.clone());
        assert_eq!(recovered_status.availability, StoreAvailability::Recovered);
        assert_eq!(recovered_status.record_count, 1);
        assert_eq!(
            recovered.get(&preserved.id).unwrap().payload,
            preserved.payload
        );
        assert!(std::fs::read_dir(root.join("quarantine")).unwrap().count() >= 1);
    }

    #[test]
    fn unavailable_status_does_not_echo_private_paths_or_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-secret");
        std::fs::write(&root, b"blocking file").unwrap();
        let store = KnowledgeStore::default();
        let status = store.initialize(root.clone());
        assert_eq!(status.availability, StoreAvailability::Unavailable);
        let message = status.message.unwrap();
        assert!(!message.contains("private-secret"));
        assert!(!message.contains(temp.path().to_string_lossy().as_ref()));
    }

    fn transform_draft(name: &str, instruction: &str) -> KnowledgeDraft {
        KnowledgeDraft {
            id: None,
            expected_revision: None,
            payload: KnowledgePayload::Transform {
                name: name.to_string(),
                instruction: instruction.to_string(),
            },
            enabled: true,
            scope: KnowledgeScope::Global,
            voice_command: None,
        }
    }

    #[test]
    fn transform_instructions_over_the_protocol_byte_limit_are_rejected() {
        let (_temp, store) = store();
        let oversized = "x".repeat(5_000);
        let error = store
            .upsert_manual(transform_draft("too long", &oversized))
            .unwrap_err();
        assert!(
            error.contains("4096") || error.to_lowercase().contains("byte"),
            "expected a byte-limit message, got: {error}"
        );

        // A right-at-the-limit instruction (in bytes) still saves fine.
        let at_limit = "y".repeat(murmur_local_llm_protocol::MAX_INSTRUCTION_BYTES);
        assert!(store
            .upsert_manual(transform_draft("right at limit", &at_limit))
            .is_ok());
    }

    #[test]
    fn saving_an_exact_duplicate_transform_name_is_rejected_but_editing_self_is_allowed() {
        let (_temp, store) = store();
        let first = store
            .upsert_manual(transform_draft("Meeting Notes", "Rewrite as bullet notes."))
            .unwrap();

        // Same normalized name (case + whitespace-insensitive) is rejected.
        let error = store
            .upsert_manual(transform_draft("meeting   notes", "Something else."))
            .unwrap_err();
        assert!(error.contains("already exists"));

        // Punctuation-only differences also collide (shares the preset normalize()).
        let error = store
            .upsert_manual(transform_draft("Meeting Notes!", "Something else."))
            .unwrap_err();
        assert!(error.contains("already exists"));

        // Editing the same record with its own (unchanged) name is fine.
        let mut draft = transform_draft("Meeting Notes", "Updated instruction.");
        draft.id = Some(first.id.clone());
        draft.expected_revision = Some(first.revision);
        assert!(store.upsert_manual(draft).is_ok());

        // A distinct name is unaffected.
        assert!(store
            .upsert_manual(transform_draft("standup summary", "Summarize as a standup update."))
            .is_ok());
    }

    #[test]
    fn transform_entries_export_import_round_trip_at_current_version() {
        let (temp, store) = store();
        let transform = store
            .upsert_manual(transform_draft(
                "meeting notes",
                "Rewrite as concise meeting notes with action items.",
            ))
            .unwrap();

        let export = temp.path().join("transform-export.json");
        store.export_to_file(&export).unwrap();
        let bundle: KnowledgeExport =
            serde_json::from_slice(&std::fs::read(&export).unwrap()).unwrap();
        assert_eq!(bundle.version, EXPORT_VERSION);
        assert_eq!(bundle.version, 3, "store convention bumped for #312 round 2");
        assert!(bundle
            .entries
            .iter()
            .any(|entry| matches!(entry.payload, KnowledgePayload::Transform { .. })));

        let revision = store.status().store_revision;
        store.delete_all(revision).unwrap();
        assert_eq!(store.status().record_count, 0);

        let imported = store.import_from_file(&export).unwrap();
        assert_eq!(imported.imported, 1);
        let restored = store.get(&transform.id).unwrap();
        assert_eq!(restored.payload, transform.payload);
        assert_eq!(restored.provenance, KnowledgeProvenance::Import);
    }

    #[test]
    fn import_rejects_a_newer_export_version_with_the_clean_message_not_a_serde_error() {
        let (temp, store) = store();
        let bundle = KnowledgeExport {
            format: EXPORT_FORMAT.to_string(),
            version: 4,
            exported_at_ms: 0,
            entries: Vec::new(),
        };
        let path = temp.path().join("future-version.json");
        std::fs::write(&path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        let error = store.import_from_file(&path).unwrap_err();
        assert_eq!(
            error,
            "The selected knowledge export format is not supported by this Murmur build."
        );
    }

    #[test]
    fn import_still_accepts_legacy_version_1_and_2_bundles() {
        let (temp, store) = store();
        let mut bundle = KnowledgeExport {
            format: EXPORT_FORMAT.to_string(),
            version: 1,
            exported_at_ms: 0,
            entries: vec![KnowledgeEntry {
                id: "legacy-entry-00000001".to_string(),
                payload: KnowledgePayload::ReplacementRule {
                    source: "hello".to_string(),
                    replacement: "hi".to_string(),
                },
                enabled: true,
                scope: KnowledgeScope::Global,
                provenance: KnowledgeProvenance::Manual,
                created_at_ms: 1,
                updated_at_ms: 1,
                revision: 1,
                voice_command: None,
            }],
        };
        let v1_path = temp.path().join("v1-export.json");
        std::fs::write(&v1_path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert_eq!(store.import_from_file(&v1_path).unwrap().imported, 1);

        bundle.version = 2;
        bundle.entries[0].id = "legacy-entry-00000002".to_string();
        // Distinct payload so this isn't treated as a semantic duplicate of
        // the v1 entry already imported above (import dedupes by payload +
        // scope + enabled + voice_command, independent of ID).
        bundle.entries[0].payload = KnowledgePayload::ReplacementRule {
            source: "goodbye".to_string(),
            replacement: "bye".to_string(),
        };
        let v2_path = temp.path().join("v2-export.json");
        std::fs::write(&v2_path, serde_json::to_vec(&bundle).unwrap()).unwrap();
        assert_eq!(store.import_from_file(&v2_path).unwrap().imported, 1);
    }

    /// Data-carrying v3 -> v4 migration test (issue #312 round 2 D1 fix #4).
    /// Migration 4 rebuilds `knowledge_entries` from scratch (SQLite cannot
    /// ALTER a CHECK constraint) to widen the `kind` set for
    /// `KnowledgeKind::Transform`. Seed one row of every kind that already
    /// existed at v3 — including voice-command columns populated — then
    /// migrate and assert every row/field survives losslessly and every
    /// index (from v2 and v3) is recreated on the rebuilt table.
    #[test]
    fn schema_v3_to_v4_migration_is_lossless_and_recreates_every_index() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("knowledge");
        std::fs::create_dir_all(root.join("backups")).unwrap();
        std::fs::create_dir_all(root.join("quarantine")).unwrap();
        let db = root.join("knowledge.sqlite3");
        let mut connection = Connection::open(&db).unwrap();
        migrations::migrate_to_for_test(&mut connection, 3).unwrap();

        let now = 1_700_000_000_000_i64;
        connection
            .execute(
                "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES ('replacement-1', 'replacement_rule', 'use hook', 'use hook', 'useHook', '[]', 1, 'global', NULL, NULL, 'manual', ?1, ?1, 1, NULL, 0)",
                rusqlite::params![now],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES ('vocab-1', 'vocabulary_term', 'Tori', 'tori', '', '[\"Tory\",\"Tori\"]', 1, 'app', 'com.editor', NULL, 'manual', ?1, ?1, 1, NULL, 0)",
                rusqlite::params![now],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES ('snippet-1', 'snippet', 'sign off', 'sign off', 'Regards, George', '[]', 1, 'project', 'com.editor', '/project', 'manual', ?1, ?1, 1, NULL, 0)",
                rusqlite::params![now],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO knowledge_entries(id, kind, trigger_text, normalized_trigger, content_text, aliases_json, enabled, scope_kind, app_bundle_id, project_root, provenance, created_at_ms, updated_at_ms, revision, voice_command_kind, voice_command_clipboard) VALUES ('voice-1', 'replacement_rule', 'my signature', 'my signature', 'Regards, George', '[]', 1, 'global', NULL, NULL, 'manual', ?1, ?1, 1, 'text_replacement', 1)",
                rusqlite::params![now],
            )
            .unwrap();
        drop(connection);

        let store = KnowledgeStore::default();
        let status = store.initialize(root.clone());
        assert_eq!(status.availability, StoreAvailability::Ready);
        assert_eq!(status.schema_version, 4);
        assert_eq!(status.record_count, 4);

        let replacement = store.get("replacement-1").unwrap();
        assert_eq!(
            replacement.payload,
            KnowledgePayload::ReplacementRule {
                source: "use hook".to_string(),
                replacement: "useHook".to_string(),
            }
        );
        assert_eq!(replacement.scope, KnowledgeScope::Global);
        assert_eq!(replacement.created_at_ms, now);
        assert_eq!(replacement.updated_at_ms, now);
        assert!(replacement.voice_command.is_none());

        let vocab = store.get("vocab-1").unwrap();
        assert_eq!(
            vocab.payload,
            KnowledgePayload::VocabularyTerm {
                written: "Tori".to_string(),
                aliases: vec!["Tory".to_string(), "Tori".to_string()],
            }
        );
        assert_eq!(
            vocab.scope,
            KnowledgeScope::App {
                bundle_id: "com.editor".to_string()
            }
        );

        let snippet = store.get("snippet-1").unwrap();
        assert_eq!(
            snippet.payload,
            KnowledgePayload::Snippet {
                trigger: "sign off".to_string(),
                body: "Regards, George".to_string(),
            }
        );
        assert_eq!(
            snippet.scope,
            KnowledgeScope::Project {
                bundle_id: "com.editor".to_string(),
                root: "/project".to_string(),
            }
        );

        let voice = store.get("voice-1").unwrap();
        assert_eq!(
            voice.voice_command,
            Some(VoiceCommandMetadata {
                command_type: VoiceCommandKind::TextReplacement,
                allow_clipboard_read: true,
            })
        );

        // Migration 4 rebuilds knowledge_entries (DROP + rename), so every
        // index defined at v2 and v3 must be recreated, not just the new
        // columns/CHECK constraint.
        let connection = Connection::open(&db).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='knowledge_entries' ORDER BY name",
            )
            .unwrap();
        let indexes: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for expected in [
            "knowledge_entries_listing",
            "knowledge_entries_resolution",
            "knowledge_entries_scope",
            "knowledge_entries_voice_commands",
        ] {
            assert!(
                indexes.iter().any(|name| name == expected),
                "missing index {expected}, have: {indexes:?}"
            );
        }
    }
}
