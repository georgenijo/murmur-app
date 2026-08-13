mod repository;
mod types;

pub(crate) use types::*;

use crate::query_provider::QueryProviderId;
use repository::QueryHistoryRepository;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

#[derive(Clone, Default)]
pub(crate) struct QueryHistoryStore {
    inner: Arc<Mutex<QueryHistoryStoreInner>>,
}

#[derive(Default)]
struct QueryHistoryStoreInner {
    repository: Option<QueryHistoryRepository>,
    root: Option<PathBuf>,
    app_handle: Option<tauri::AppHandle>,
    initialization_error: Option<String>,
    generation: u64,
}

impl QueryHistoryStore {
    pub(crate) fn initialize(
        &self,
        root: PathBuf,
        app_handle: Option<tauri::AppHandle>,
    ) -> Result<(), String> {
        let result = QueryHistoryRepository::initialize(root.clone());
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.app_handle = app_handle;
        inner.root = Some(root);
        match result {
            Ok((repository, _outcome)) => {
                inner.generation = repository.clear_epoch().unwrap_or(0);
                inner.repository = Some(repository);
                inner.initialization_error = None;
                Ok(())
            }
            Err(error) => {
                inner.repository = None;
                inner.initialization_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn repository(&self) -> Result<QueryHistoryRepository, String> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.repository.clone().ok_or_else(|| {
            inner.initialization_error.clone().unwrap_or_else(|| {
                "The local Voice Query history store is unavailable.".to_string()
            })
        })
    }

    pub(crate) fn clear_epoch(&self) -> Option<u64> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.repository.as_ref()?;
        Some(inner.generation)
    }

    pub(crate) fn insert_if_epoch(
        &self,
        epoch: u64,
        draft: QueryHistoryDraft,
    ) -> Result<Option<QueryHistoryEntryV1>, String> {
        let (repository, generation) = {
            let inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (inner.repository.clone(), inner.generation)
        };
        if epoch != generation {
            return Ok(None);
        }
        let entry = repository
            .ok_or_else(|| "The local Voice Query history store is unavailable.".to_string())?
            .insert_if_epoch(epoch, draft)?;
        if entry.is_some() {
            self.emit_changed("inserted");
        }
        Ok(entry)
    }

    pub(crate) fn list(
        &self,
        offset: u32,
        limit: u32,
        provider: Option<QueryProviderId>,
    ) -> Result<QueryHistoryPageV1, String> {
        self.repository()?.list(offset, limit, provider)
    }

    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let repository = inner.repository.clone();
        let root = inner.root.clone();
        let next_generation = inner
            .generation
            .checked_add(1)
            .ok_or_else(|| "Voice Query history purge generation is exhausted.".to_string())?;
        if let Some(repository) = repository {
            if let Err(error) = repository.clear_to_epoch(next_generation) {
                if !QueryHistoryRepository::should_reset_after(&error) {
                    return Err(error);
                }
                let root = root.ok_or_else(|| {
                    "The local Voice Query history store is unavailable.".to_string()
                })?;
                let repository = QueryHistoryRepository::reset(root, next_generation)?;
                inner.repository = Some(repository);
                inner.initialization_error = None;
            }
        } else {
            let root = root
                .ok_or_else(|| "The local Voice Query history store is unavailable.".to_string())?;
            let repository = QueryHistoryRepository::reset(root, next_generation)?;
            inner.repository = Some(repository);
            inner.initialization_error = None;
        }
        inner.generation = next_generation;
        drop(inner);
        self.emit_changed("cleared");
        Ok(())
    }

    fn emit_changed(&self, kind: &'static str) {
        let app_handle = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .app_handle
            .clone();
        if let Some(app_handle) = app_handle {
            let _ = app_handle.emit("query-history-changed", serde_json::json!({ "kind": kind }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;

    #[test]
    fn explicit_clear_resets_a_store_replaced_by_a_future_version() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("query-history");
        let store = QueryHistoryStore::default();
        store.initialize(root.clone(), None).unwrap();
        let old_epoch = store.clear_epoch().unwrap();
        let db = root.join("query-history.sqlite3");
        let connection = Connection::open(&db).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        drop(connection);
        fs::write(root.join("quarantine/stale.sqlite3"), b"PRIVATE").unwrap();

        assert!(store.list(0, 10, None).is_err());
        store.clear().unwrap();
        assert_ne!(store.clear_epoch(), Some(old_epoch));
        assert!(store
            .insert_if_epoch(
                old_epoch,
                QueryHistoryDraft {
                    timestamp_ms: 1,
                    provider: QueryProviderId::Claude,
                    question: "old question".to_string(),
                    answer: "old answer".to_string(),
                    tokens: None,
                    duration_ms: 1,
                    error_code: None,
                },
            )
            .unwrap()
            .is_none());
        assert_eq!(store.list(0, 10, None).unwrap().total, 0);
        assert_eq!(fs::read_dir(root.join("quarantine")).unwrap().count(), 0);
        let connection = Connection::open(db).unwrap();
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }
}
