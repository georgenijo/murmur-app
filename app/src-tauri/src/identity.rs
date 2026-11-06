//! Legacy global stores retain production locations only for the production ID.
//! All other bundle IDs get an injective, path-safe namespace, including future IDs.
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
pub(crate) const PRODUCTION: &str = "com.localdictation";
static IDENTIFIER: OnceLock<String> = OnceLock::new();
pub(crate) fn initialize(identifier: &str) {
    let _ = IDENTIFIER.set(identifier.to_owned());
}
pub(crate) fn initialized_identifier() -> Option<&'static str> {
    IDENTIFIER.get().map(String::as_str)
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
    dirs::data_dir().map(|root| namespace(&root, identifier()))
}
pub(crate) fn temp_dir() -> PathBuf {
    namespace(&std::env::temp_dir(), identifier())
}
#[cfg(test)]
mod tests {
    use super::*;
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
