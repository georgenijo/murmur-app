#[cfg(target_os = "macos")]
const NOTCHPILL_BUNDLE_ID: &str = "com.local.notchpill";

/// Whether NotchPill is registered with macOS Launch Services.
///
/// Looking up the bundle identifier catches supported `/Applications` installs
/// as well as user-local or renamed app bundles. The setting is macOS-only, so
/// other platforms keep the same command surface and report it unavailable.
pub(crate) fn notchpill_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::NSString;

        let bundle_id = NSString::from_str(NOTCHPILL_BUNDLE_ID);
        NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(&bundle_id)
            .is_some()
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[tauri::command]
pub fn is_notchpill_installed() -> bool {
    notchpill_installed()
}
