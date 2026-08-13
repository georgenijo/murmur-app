use crate::query_history::{
    QueryHistoryPageV1, DEFAULT_QUERY_HISTORY_PAGE_SIZE, MAX_QUERY_HISTORY_PAGE_SIZE,
};
use crate::query_provider::QueryProviderId;
use crate::State;

fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    (window.label() == "main")
        .then_some(())
        .ok_or_else(|| "Voice Query history is only available in the main window.".to_string())
}

#[tauri::command]
pub fn list_query_history(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
    offset: Option<u32>,
    limit: Option<u32>,
    provider: Option<QueryProviderId>,
) -> Result<QueryHistoryPageV1, String> {
    require_main_window(&window)?;
    state.query_history.list(
        offset.unwrap_or(0),
        limit
            .unwrap_or(DEFAULT_QUERY_HISTORY_PAGE_SIZE)
            .clamp(1, MAX_QUERY_HISTORY_PAGE_SIZE),
        provider,
    )
}

#[tauri::command]
pub fn clear_query_history(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    require_main_window(&window)?;
    state.query_history.clear()
}
