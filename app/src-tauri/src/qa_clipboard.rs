//! QA WebKit clipboard calls share the native private pasteboard primitives.
//! The plugin is absent from production and also refuses production at dispatch.
#[cfg(any(test, feature = "shutdown-test-support"))]
fn require_isolated(identifier: &str, initialized: Option<&str>) -> Result<(), String> {
    if identifier == crate::identity::PRODUCTION || initialized != Some(identifier) {
        return Err("An initialized isolated clipboard identity is required".into());
    }
    Ok(())
}
#[cfg(feature = "shutdown-test-support")]
#[tauri::command]
fn read_text(app: tauri::AppHandle) -> Result<String, String> {
    require_isolated(
        &app.config().identifier,
        crate::identity::initialized_identifier(),
    )?;
    crate::injector::read_clipboard_text()
}
#[cfg(feature = "shutdown-test-support")]
#[tauri::command]
fn write_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    write_isolated(
        &app.config().identifier,
        crate::identity::initialized_identifier(),
        &text,
    )
}
#[cfg(any(test, feature = "shutdown-test-support"))]
fn write_isolated(identifier: &str, initialized: Option<&str>, text: &str) -> Result<(), String> {
    require_isolated(identifier, initialized)?;
    // Explicitly select the named board after identity validation. This seam is
    // also used by the synthetic native Copy test; it cannot use the general board.
    let name = crate::identity::private_pasteboard_name(initialized)
        .ok_or("A private clipboard identity is required")?;
    crate::injector::private_clipboard_write(&name, text)
}

#[cfg(feature = "shutdown-test-support")]
pub(crate) fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("qa-clipboard")
        .js_init_script_on_all_frames(include_str!("qa_clipboard.js"))
        .invoke_handler(tauri::generate_handler![read_text, write_text])
        .build()
}
#[cfg(test)]
mod tests {
    #[test]
    fn generated_clipboard_acl_matches_build_configuration() {
        let mut context = crate::generated_context();
        let authority = context.runtime_authority_mut();
        for window in ["main", "diagnostics", "overlay", "dictation-preview"] {
            for command in ["read_text", "write_text"] {
                let allowed = authority
                    .resolve_access(
                        &format!("plugin:qa-clipboard|{command}"),
                        window,
                        window,
                        &tauri::ipc::Origin::Local,
                    )
                    .is_some();
                assert_eq!(
                    allowed,
                    cfg!(feature = "shutdown-test-support"),
                    "{window}: {command}"
                );
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn qa_copy_writes_only_to_its_synthetic_private_board() {
        let identifier = format!(
            "com.localdictation.shutdown-test.synthetic-{}",
            uuid::Uuid::new_v4()
        );
        let text = "SYNTHETIC QA COPY";
        super::write_isolated(&identifier, Some(&identifier), text).unwrap();
        let name = crate::identity::private_pasteboard_name(Some(&identifier)).unwrap();
        assert_eq!(
            crate::injector::private_clipboard_read(&name).unwrap(),
            text
        );
        objc2::rc::autoreleasepool(|_| {
            let board = objc2_app_kit::NSPasteboard::pasteboardWithName(
                &objc2_foundation::NSString::from_str(&name),
            );
            board.clearContents();
        });
    }

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
