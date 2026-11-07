//! QA WebKit clipboard calls share the native private pasteboard primitives.
//! The plugin is absent from production and also refuses production at dispatch.
#[cfg(test)]
fn require_isolated(identifier: &str, initialized: Option<&str>) -> Result<(), String> {
    if identifier == crate::identity::PRODUCTION || initialized != Some(identifier) {
        return Err("An initialized isolated clipboard identity is required".into());
    }
    Ok(())
}

#[cfg(test)]
fn write_isolated(identifier: &str, initialized: Option<&str>, text: &str) -> Result<(), String> {
    require_isolated(identifier, initialized)?;
    // Explicitly select the named board after identity validation. This seam is
    // also used by the synthetic native Copy test; it cannot use the general board.
    let name = crate::identity::private_pasteboard_name(initialized)
        .ok_or("A private clipboard identity is required")?;
    crate::injector::private_clipboard_write(&name, text)
}

#[cfg(test)]
mod tests {

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
