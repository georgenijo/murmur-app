//! QA WebKit clipboard calls share the native private pasteboard primitives.
//! The plugin is absent from production and also refuses production at dispatch.
fn require_isolated(identifier: &str, initialized: Option<&str>) -> Result<(), String> {
    if identifier == crate::identity::PRODUCTION || initialized != Some(identifier) {
        return Err("An initialized isolated clipboard identity is required".into());
    }
    Ok(())
}
#[tauri::command]
fn read_text(app: tauri::AppHandle) -> Result<String, String> {
    require_isolated(
        &app.config().identifier,
        crate::identity::initialized_identifier(),
    )?;
    crate::injector::read_clipboard_text()
}
#[tauri::command]
fn write_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    require_isolated(
        &app.config().identifier,
        crate::identity::initialized_identifier(),
    )?;
    crate::injector::write_clipboard_text(&text)
}
pub(crate) fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("qa-clipboard")
        .js_init_script_on_all_frames(include_str!("qa_clipboard.js"))
        .invoke_handler(tauri::generate_handler![read_text, write_text])
        .build()
}
#[cfg(test)]
mod tests {
    #[test]
    fn bridge_never_grants_production_or_unproven_clipboard_access() {
        use super::require_isolated;
        use crate::identity::PRODUCTION;
        assert!(require_isolated(PRODUCTION, Some(PRODUCTION)).is_err());
        for identifier in [
            "com.localdictation.shutdown-test",
            "com.localdictation.bench",
            "future.id",
            "",
        ] {
            assert!(require_isolated(identifier, Some(identifier)).is_ok());
            assert!(require_isolated(identifier, None).is_err());
            assert!(require_isolated(identifier, Some(PRODUCTION)).is_err());
        }
    }
}
