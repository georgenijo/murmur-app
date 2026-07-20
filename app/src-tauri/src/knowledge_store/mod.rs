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
        assert_eq!(status.schema_version, 3);
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
}
