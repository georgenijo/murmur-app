use crate::knowledge_store::*;
use crate::{MutexExt, State};
use std::path::PathBuf;

/// Reservation and publication share this lock, but SQLite never runs under it.
/// A delayed startup read cannot publish over a more recently requested refresh.
#[derive(Default)]
pub(crate) struct KnowledgeRefresh {
    generation: std::sync::Mutex<u64>,
    ready: std::sync::atomic::AtomicBool,
}
impl KnowledgeRefresh {
    pub(crate) fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Acquire)
    }
}

pub(crate) fn refresh_correction_rules(state: &State) -> Result<(), String> {
    refresh_with(&state.app_state, || {
        state.knowledge.enabled_replacement_rules()
    })
}

fn refresh_with(
    app_state: &crate::state::AppState,
    read: impl FnOnce() -> Result<Vec<KnowledgeEntry>, String>,
) -> Result<(), String> {
    let generation = {
        let mut current = app_state.knowledge_refresh.generation.lock_or_recover();
        *current += 1;
        *current
    };
    let entries = read()?;
    let current = app_state.knowledge_refresh.generation.lock_or_recover();
    if *current != generation {
        return Ok(());
    }
    // Hold the dictation lock across entries + matcher publication. Settings
    // rebuilds and recording-start snapshots use this same lock.
    let dictation = app_state.dictation.lock_or_recover();
    *app_state.knowledge_replacements.lock_or_recover() = std::sync::Arc::new(entries);
    crate::commands::recording::rebuild_correction_matcher(app_state, &dictation);
    app_state.bump_settings_revision();
    app_state
        .knowledge_refresh
        .ready
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_knowledge_store_status(state: crate::host::StateRef<'_, State>) -> KnowledgeStoreStatus {
    state.knowledge.status()
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn retry_knowledge_store(state: crate::host::StateRef<'_, State>) -> KnowledgeStoreStatus {
    let status = state.knowledge.retry();
    if status.availability != StoreAvailability::Unavailable {
        if let Err(error) = refresh_correction_rules(&state) {
            tracing::warn!(target: "system", error, "knowledge correction matcher refresh failed");
        }
    }
    status
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn list_knowledge(
    request: KnowledgeListRequest,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeListResponse, String> {
    state.knowledge.list(request)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_knowledge(
    id: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeEntry, String> {
    state.knowledge.get(id.trim())
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn upsert_knowledge(
    draft: KnowledgeDraft,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeEntry, String> {
    if matches!(draft.payload, KnowledgePayload::Transform { .. }) {
        return Err("Saved rewrite instructions are no longer supported.".into());
    }
    if draft.voice_command.is_some() {
        let mut commands = state
            .knowledge
            .all_voice_commands()?
            .into_iter()
            .filter(|entry| draft.id.as_deref() != Some(entry.id.as_str()))
            .map(|entry| {
                let (phrase, replacement, _) = entry.payload.storage_parts();
                crate::state::VoiceCommand {
                    phrase,
                    replacement,
                }
            })
            .collect::<Vec<_>>();
        let (phrase, replacement, _) = draft.payload.storage_parts();
        commands.push(crate::state::VoiceCommand {
            phrase,
            replacement,
        });
        let dictation = state.app_state.dictation.lock_or_recover();
        crate::vocabulary_alias::validate_entries(&dictation.vocabulary_entries, &commands)?;
        drop(dictation);
    }
    let entry = state.knowledge.upsert_manual(draft)?;
    refresh_correction_rules(&state)?;
    Ok(entry)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn set_knowledge_enabled(
    id: String,
    enabled: bool,
    expected_revision: u64,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeEntry, String> {
    let entry = state
        .knowledge
        .set_enabled(id.trim(), enabled, expected_revision)?;
    refresh_correction_rules(&state)?;
    Ok(entry)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn delete_knowledge(
    id: String,
    expected_revision: u64,
    state: crate::host::StateRef<'_, State>,
) -> Result<u64, String> {
    let revision = state.knowledge.delete(id.trim(), expected_revision)?;
    refresh_correction_rules(&state)?;
    Ok(revision)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn resolve_knowledge(
    request: KnowledgeResolveRequest,
    state: crate::host::StateRef<'_, State>,
) -> Result<Option<KnowledgeEntry>, String> {
    state.knowledge.resolve(request)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn preview_voice_command(
    request: VoiceCommandPreviewRequest,
) -> Result<VoiceCommandPreviewResponse, String> {
    let application = crate::voice_commands::preview_voice_command(
        request.draft,
        &request.text,
        request.read_clipboard,
    )?;
    Ok(VoiceCommandPreviewResponse {
        output: application.text,
        matched: application.matched,
        clipboard_required: application.clipboard_required,
        clipboard_read: application.clipboard_read,
    })
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn export_knowledge_to_file(
    path: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<u64, String> {
    state.knowledge.export_to_file(&PathBuf::from(path))
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn inspect_knowledge_import(
    path: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeImportSummary, String> {
    state.knowledge.inspect_import(&PathBuf::from(path))
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn import_knowledge_from_file(
    path: String,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeImportResult, String> {
    let result = state.knowledge.import_from_file(&PathBuf::from(path))?;
    refresh_correction_rules(&state)?;
    Ok(result)
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn delete_all_knowledge(
    expected_revision: u64,
    state: crate::host::StateRef<'_, State>,
) -> Result<u64, String> {
    let revision = state.knowledge.delete_all(expected_revision)?;
    refresh_correction_rules(&state)?;
    Ok(revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    fn rule(replacement: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: "synthetic-rule".into(),
            payload: KnowledgePayload::ReplacementRule {
                source: "synthetic alias".into(),
                replacement: replacement.into(),
            },
            enabled: true,
            scope: KnowledgeScope::Global,
            provenance: KnowledgeProvenance::Manual,
            created_at_ms: 0,
            updated_at_ms: 0,
            revision: 1,
            voice_command: None,
        }
    }

    #[test]
    fn delayed_startup_refresh_cannot_replace_newer_user_matcher() {
        let app = Arc::new(crate::state::AppState::default());
        let startup = app.clone();
        let (entered, reading) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            refresh_with(&startup, || {
                let old = vec![rule("OldValue")];
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(old)
            })
        });
        reading.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!app.knowledge_refresh.ready());
        refresh_with(&app, || Ok(vec![rule("NewValue")])).unwrap();
        let snapshot = app.correction_matcher.lock_or_recover().clone().unwrap();
        let revision = app
            .settings_revision
            .load(std::sync::atomic::Ordering::SeqCst);
        release.send(()).unwrap();
        worker.join().unwrap().unwrap();
        let current = app.correction_matcher.lock_or_recover().clone().unwrap();
        assert!(Arc::ptr_eq(&snapshot, &current));
        assert_eq!(current.select(None).apply("synthetic alias"), "NewValue");
        assert_eq!(
            app.settings_revision
                .load(std::sync::atomic::Ordering::SeqCst),
            revision
        );
    }

    #[test]
    fn recording_admission_waits_for_startup_matcher_publication() {
        let app = Arc::new(crate::state::AppState::default());
        let startup = app.clone();
        let (entered, reading) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            refresh_with(&startup, || {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(vec![rule("StartupValue")])
            })
        });
        reading.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            crate::commands::recording::knowledge_start_blocker(&app)
                .unwrap()
                .kind,
            "knowledge_initializing"
        );
        release.send(()).unwrap();
        worker.join().unwrap().unwrap();
        assert!(crate::commands::recording::knowledge_start_blocker(&app).is_none());
        let snapshot = app.correction_matcher.lock_or_recover().clone().unwrap();
        assert_eq!(
            snapshot.select(None).apply("synthetic alias"),
            "StartupValue"
        );
        refresh_with(&app, || Ok(vec![rule("LaterValue")])).unwrap();
        assert_eq!(
            snapshot.select(None).apply("synthetic alias"),
            "StartupValue"
        );
        let failed = crate::state::AppState::default();
        assert!(refresh_with(&failed, || Err("synthetic read failure".into())).is_err());
        assert!(crate::commands::recording::knowledge_start_blocker(&failed).is_some());
    }
}
