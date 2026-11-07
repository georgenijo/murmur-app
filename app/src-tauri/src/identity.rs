//! Legacy global stores retain production locations only for the production ID.
//! All other bundle IDs get an injective, path-safe namespace, including future IDs.
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
pub(crate) const PRODUCTION: &str = "com.localdictation";
static IDENTIFIER: OnceLock<String> = OnceLock::new();
#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
static NATIVE_SUPPORT: OnceLock<PathBuf> = OnceLock::new();
#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
pub(crate) fn initialize_native(identifier: &str, home: &Path) -> Result<(), String> {
    if !home.is_absolute() {
        return Err("Native home path unavailable".into());
    }
    let support = home.join("Library/Application Support");
    if NATIVE_SUPPORT.get().is_some_and(|old| *old != support)
        || initialized_identifier().is_some_and(|old| old != identifier)
    {
        return Err("Native identity already initialized".into());
    }
    let _ = NATIVE_SUPPORT.set(support);
    initialize(identifier);
    Ok(())
}
#[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
pub(crate) fn native_support() -> Option<&'static Path> {
    NATIVE_SUPPORT.get().map(PathBuf::as_path)
}
pub(crate) fn initialize(identifier: &str) {
    let _ = IDENTIFIER.set(identifier.to_owned());
}
pub(crate) fn initialized_identifier() -> Option<&'static str> {
    IDENTIFIER.get().map(String::as_str)
}
/// Only an explicitly initialized production process may reach shared OS input.
pub(crate) fn external_input_allowed() -> bool {
    input_allowed_for(initialized_identifier())
}
fn input_allowed_for(identifier: Option<&str>) -> bool {
    identifier == Some(PRODUCTION)
}
pub(crate) fn private_pasteboard_name(identifier: Option<&str>) -> Option<String> {
    if input_allowed_for(identifier) {
        return None;
    }
    let encoded: String = identifier
        .unwrap_or("uninitialized")
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // The option discriminator also separates None from Some("uninitialized").
    Some(format!(
        "com.localdictation.private-pasteboard.{}.{encoded}",
        if identifier.is_some() { "id" } else { "absent" }
    ))
}
pub(crate) fn identifier() -> &'static str {
    IDENTIFIER.get().map(String::as_str).unwrap_or(PRODUCTION)
}
pub(crate) fn internal() -> bool {
    identifier() != PRODUCTION
}
pub(crate) fn namespace(root: &Path, identifier: &str) -> PathBuf {
    if identifier == PRODUCTION {
        return root.to_owned();
    }
    let encoded: String = identifier
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    root.join("murmur-isolated").join(encoded)
}
pub(crate) fn data_dir() -> Option<PathBuf> {
    #[cfg(all(feature = "native-host", not(feature = "tauri-host")))]
    return native_support().map(|root| namespace(root, identifier()));
    #[cfg(feature = "tauri-host")]
    dirs::data_dir().map(|root| namespace(&root, identifier()))
}
pub(crate) fn temp_dir() -> PathBuf {
    namespace(&std::env::temp_dir(), identifier())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_and_input_routing_fails_closed_without_touching_pasteboards() {
        assert!(input_allowed_for(Some(PRODUCTION)));
        assert_eq!(private_pasteboard_name(Some(PRODUCTION)), None);
        let mut names = std::collections::HashSet::new();
        for id in [
            None,
            Some("uninitialized"),
            Some(""),
            Some("com.localdictation.shutdown-test"),
            Some("com.localdictation.bench"),
            Some("com.localdictation.refactor-test"),
            Some("future.id"),
            Some("../com.localdictation"),
        ] {
            assert!(!input_allowed_for(id));
            let name = private_pasteboard_name(id).unwrap();
            assert!(!name.contains('/'));
            assert!(names.insert(name));
        }
    }
    #[test]
    fn every_nonproduction_identity_is_disjoint_from_production_and_each_other() {
        let root = Path::new("/synthetic");
        let ids = [
            "com.localdictation.bench",
            "com.localdictation.refactor-test",
            "com.localdictation.shutdown-test",
            "future.id",
            "../com.localdictation",
            "",
        ];
        let mut paths = std::collections::HashSet::new();
        for id in ids {
            let isolated = namespace(root, id);
            for suffix in [
                "local-dictation/logs",
                "local-dictation/models",
                "FluidAudio/Models",
                "Murmur Benchmark Corpus/v1",
                "pending",
                "cache",
                "socket",
            ] {
                assert_ne!(isolated.join(suffix), root.join(suffix));
                assert!(!isolated
                    .join(suffix)
                    .starts_with(root.join("local-dictation")));
            }
            assert!(paths.insert(isolated));
        }
    }
}
