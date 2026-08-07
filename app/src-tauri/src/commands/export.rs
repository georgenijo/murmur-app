//! Narrow, validated sink for user-initiated "Save as…" exports.
//!
//! The frontend renders the payload (transcript history today) and the user
//! picks the destination through the native save dialog, so this command must
//! not become a general-purpose "write any file anywhere" primitive. Every
//! request is checked against an extension allow-list and a size ceiling, and
//! the write is atomic (temp file in the same directory, then rename) so a
//! failure part-way through never leaves a truncated export behind.

use std::path::{Path, PathBuf};

/// Extensions this command is willing to produce. Anything else is refused —
/// an export is a document, never an executable or a config file.
const ALLOWED_EXTENSIONS: [&str; 3] = ["json", "md", "txt"];

/// Hard ceiling on one export payload. Transcript history is capped well below
/// this; the bound exists so a malformed caller cannot write an unbounded blob.
const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;

/// Reject anything that isn't an absolute path to a writable document with an
/// allow-listed extension inside an existing directory.
pub(crate) fn validate_export_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Export path must be absolute".to_string());
    }
    if path.is_dir() {
        return Err("Export path is a directory".to_string());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Export path has no file name".to_string())?;
    if name.starts_with('.') {
        return Err("Export file name must not start with a dot".to_string());
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| "Export file name must have an extension".to_string())?;
    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "Unsupported export type '.{extension}' (expected .json, .md or .txt)"
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Export path has no parent directory".to_string())?;
    if !parent.is_dir() {
        return Err("Export folder does not exist".to_string());
    }
    Ok(())
}

/// Temp sibling used for the atomic publish. Kept in the destination directory
/// so the rename stays on one filesystem.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export");
    path.with_file_name(format!(".{name}.murmur-tmp"))
}

/// Validate, then write `contents` to `path` atomically. Returns bytes written.
pub(crate) fn write_text_export(path: &Path, contents: &str) -> Result<u64, String> {
    validate_export_path(path)?;
    if contents.len() > MAX_EXPORT_BYTES {
        return Err(format!(
            "Export is too large ({} bytes, limit {MAX_EXPORT_BYTES})",
            contents.len()
        ));
    }

    let temp = temp_path_for(path);
    if let Err(e) = std::fs::write(&temp, contents) {
        // A partial write (ENOSPC, permissions) must not leave a hidden temp
        // sibling behind in the folder the user picked.
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to write export: {e}"));
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Failed to publish export: {e}"));
    }
    Ok(contents.len() as u64)
}

/// Write a user-authored text export to a path the user chose in the native
/// save dialog. Returns the number of bytes written.
#[tauri::command]
pub fn save_text_export(path: String, contents: String) -> Result<u64, String> {
    let bytes = write_text_export(Path::new(&path), &contents)?;
    // Content-free: how much was exported, never what.
    tracing::info!(target: "pipeline", bytes, "text export written");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("murmur_export_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_an_allowed_export() {
        let dir = temp_dir("write");
        let path = dir.join("murmur-history-2026-07-27-1432.md");
        let bytes = write_text_export(&path, "# hi\n").unwrap();
        assert_eq!(bytes, 5);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# hi\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrites_an_existing_export_and_leaves_no_temp_file() {
        let dir = temp_dir("overwrite");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "stale contents that are longer").unwrap();
        write_text_export(&path, "fresh\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["notes.txt".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn accepts_each_allowed_extension_case_insensitively() {
        let dir = temp_dir("extensions");
        for name in ["a.md", "b.TXT", "c.Json"] {
            let path = dir.join(name);
            assert!(
                write_text_export(&path, "x").is_ok(),
                "{name} should be allowed"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_unlisted_extensions() {
        let dir = temp_dir("bad_ext");
        for name in ["script.sh", "app.command", "config.toml", "archive.tar.gz"] {
            let error = write_text_export(&dir.join(name), "x").unwrap_err();
            assert!(error.contains("Unsupported export type"), "{name}: {error}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_a_missing_extension() {
        let dir = temp_dir("no_ext");
        let error = write_text_export(&dir.join("history"), "x").unwrap_err();
        assert!(error.contains("must have an extension"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_dotfiles() {
        let dir = temp_dir("dotfile");
        let error = write_text_export(&dir.join(".zshrc.txt"), "x").unwrap_err();
        assert!(error.contains("must not start with a dot"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_relative_paths() {
        let error = write_text_export(Path::new("history.md"), "x").unwrap_err();
        assert!(error.contains("must be absolute"), "{error}");
    }

    #[test]
    fn rejects_a_directory_target() {
        let dir = temp_dir("dir_target");
        let nested = dir.join("bundle.md");
        std::fs::create_dir_all(&nested).unwrap();
        let error = write_text_export(&nested, "x").unwrap_err();
        assert!(error.contains("is a directory"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_a_missing_parent_directory() {
        let dir = temp_dir("missing_parent");
        let error = write_text_export(&dir.join("nope").join("history.md"), "x").unwrap_err();
        assert!(error.contains("folder does not exist"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_payloads_over_the_ceiling() {
        let dir = temp_dir("too_big");
        let path = dir.join("history.txt");
        let huge = "a".repeat(MAX_EXPORT_BYTES + 1);
        let error = write_text_export(&path, &huge).unwrap_err();
        assert!(error.contains("too large"), "{error}");
        assert!(
            !path.exists(),
            "nothing should be written when the payload is refused"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file_behind() {
        // The temp sibling path is occupied by a directory, so `fs::write`
        // itself fails rather than the rename.
        let dir = temp_dir("write_failure");
        let path = dir.join("history.md");
        std::fs::create_dir_all(temp_path_for(&path)).unwrap();
        assert!(write_text_export(&path, "x").is_err());
        assert!(!path.exists());
        // The pre-existing blocker is left alone; nothing new is created.
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), 1, "unexpected leftovers: {entries:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn temp_path_stays_in_the_destination_directory() {
        let path = Path::new("/tmp/murmur/history.md");
        let temp = temp_path_for(path);
        assert_eq!(temp.parent(), path.parent());
        assert_ne!(temp.file_name(), path.file_name());
    }
}
