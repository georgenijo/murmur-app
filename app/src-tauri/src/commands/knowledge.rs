use crate::knowledge_store::*;
use crate::MutexExt;
use crate::State;
use std::path::PathBuf;

#[tauri::command]
pub fn get_knowledge_store_status(state: tauri::State<'_, State>) -> KnowledgeStoreStatus {
    state.knowledge.status()
}

#[tauri::command]
pub fn retry_knowledge_store(state: tauri::State<'_, State>) -> KnowledgeStoreStatus {
    state.knowledge.retry()
}

#[tauri::command]
pub fn list_knowledge(
    request: KnowledgeListRequest,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeListResponse, String> {
    state.knowledge.list(request)
}

#[tauri::command]
pub fn get_knowledge(id: String, state: tauri::State<'_, State>) -> Result<KnowledgeEntry, String> {
    state.knowledge.get(id.trim())
}

#[tauri::command]
pub fn upsert_knowledge(
    draft: KnowledgeDraft,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeEntry, String> {
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
    state.knowledge.upsert_manual(draft)
}

#[tauri::command]
pub fn set_knowledge_enabled(
    id: String,
    enabled: bool,
    expected_revision: u64,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeEntry, String> {
    state
        .knowledge
        .set_enabled(id.trim(), enabled, expected_revision)
}

#[tauri::command]
pub fn delete_knowledge(
    id: String,
    expected_revision: u64,
    state: tauri::State<'_, State>,
) -> Result<u64, String> {
    state.knowledge.delete(id.trim(), expected_revision)
}

#[tauri::command]
pub fn resolve_knowledge(
    request: KnowledgeResolveRequest,
    state: tauri::State<'_, State>,
) -> Result<Option<KnowledgeEntry>, String> {
    state.knowledge.resolve(request)
}

#[tauri::command]
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

#[tauri::command]
pub fn export_knowledge_to_file(
    path: String,
    state: tauri::State<'_, State>,
) -> Result<u64, String> {
    state.knowledge.export_to_file(&PathBuf::from(path))
}

#[tauri::command]
pub fn inspect_knowledge_import(
    path: String,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeImportSummary, String> {
    state.knowledge.inspect_import(&PathBuf::from(path))
}

#[tauri::command]
pub fn import_knowledge_from_file(
    path: String,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeImportResult, String> {
    state.knowledge.import_from_file(&PathBuf::from(path))
}

#[tauri::command]
pub fn delete_all_knowledge(
    expected_revision: u64,
    state: tauri::State<'_, State>,
) -> Result<u64, String> {
    state.knowledge.delete_all(expected_revision)
}
