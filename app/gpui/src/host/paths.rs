//! Path calculation only. Construction never probes or opens a user directory.
use murmur_core::context::{ContextError, Paths};
use std::path::{Path, PathBuf};
pub const PRODUCTION_ID: &str = "com.localdictation";
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataPaths {
    pub identifier: String,
    pub support: PathBuf,
    pub app: PathBuf,
    pub legacy: PathBuf,
}
impl DataPaths {
    pub fn new(home: &Path, identifier: &str) -> Result<Self, ContextError> {
        // Bundle identifiers are a single path component. Never accept an empty
        // identity and accidentally fall back to the production namespace.
        if !home.is_absolute()
            || identifier.is_empty()
            || identifier.len() > 255
            || !identifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || identifier.starts_with('.')
            || identifier.ends_with('.')
            || identifier.contains("..")
        {
            return Err(ContextError::PathUnavailable);
        }
        let support = home.join("Library/Application Support");
        let legacy = if identifier == PRODUCTION_ID {
            support.clone()
        } else {
            let encoded: String = identifier.bytes().map(|b| format!("{b:02x}")).collect();
            support.join("murmur-isolated").join(encoded)
        };
        Ok(Self {
            identifier: identifier.into(),
            app: support.join(identifier),
            support,
            legacy,
        })
    }
    pub fn production(&self) -> bool {
        self.identifier == PRODUCTION_ID
    }
}
impl Paths for DataPaths {
    fn app_data_dir(&self) -> Result<PathBuf, ContextError> {
        Ok(self.app.clone())
    }
    fn models_dir(&self) -> Option<PathBuf> {
        Some(self.legacy.join("local-dictation/models"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tauri_path_parity_is_calculation_only() {
        let home = Path::new("/synthetic-home");
        for id in [
            PRODUCTION_ID,
            "com.localdictation.dev",
            "com.localdictation.bench",
        ] {
            let p = DataPaths::new(home, id).unwrap();
            assert_eq!(p.app, home.join("Library/Application Support").join(id));
            assert_eq!(p.app_data_dir().unwrap(), p.app);
            if id == PRODUCTION_ID {
                assert_eq!(p.legacy, p.support);
            } else {
                assert!(p.legacy.starts_with(p.support.join("murmur-isolated")));
            }
        }
    }
    #[test]
    fn unknown_id_is_isolated_and_bad_id_fails_closed() {
        assert!(
            !DataPaths::new(Path::new("/synthetic"), "future.id")
                .unwrap()
                .production()
        );
        for id in [
            "",
            "..",
            "../com.localdictation",
            "/com.localdictation",
            "a/b",
            "a\\b",
        ] {
            assert!(DataPaths::new(Path::new("/synthetic"), id).is_err());
        }
    }
}
