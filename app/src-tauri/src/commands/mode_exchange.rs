use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;

const MAX_MODE_IMPORT_BYTES: usize = 256 * 1024;

fn require_main_window(label: &str) -> Result<(), String> {
    if label != "main" {
        return Err("Mode imports are only available from the main window.".to_string());
    }
    Ok(())
}

fn read_modes_file_at(path: &Path) -> Result<String, String> {
    let invalid = || "Choose a regular JSON Mode file with an absolute path.".to_string();
    let unreadable = || "Murmur could not read the Mode file.".to_string();
    let oversized = || "Mode files must be 256 KiB or smaller.".to_string();
    if !path.is_absolute()
        || !path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err(invalid());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| unreadable())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    if metadata.len() > MAX_MODE_IMPORT_BYTES as u64 {
        return Err(oversized());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| unreadable())?;
    if !file.metadata().map_err(|_| unreadable())?.is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MODE_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() > MAX_MODE_IMPORT_BYTES {
        return Err(oversized());
    }
    String::from_utf8(bytes).map_err(|_| "Mode files must be valid UTF-8.".to_string())
}

#[tauri::command]
pub fn read_modes_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    require_main_window(window.label())?;
    read_modes_file_at(Path::new(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_import_requires_main_window() {
        assert!(require_main_window("main").is_ok());
        for label in ["overlay", "log-viewer", "transform-review", "", "main-copy"] {
            assert!(require_main_window(label).is_err());
        }
    }

    #[test]
    fn reads_bounded_utf8_json_and_rejects_invalid_paths_without_disclosing_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private-mode-name.json");
        fs::write(&path, "é".repeat(MAX_MODE_IMPORT_BYTES / 2)).unwrap();
        assert_eq!(read_modes_file_at(&path).unwrap().len(), MAX_MODE_IMPORT_BYTES);
        fs::write(&path, vec![b'a'; MAX_MODE_IMPORT_BYTES + 1]).unwrap();
        assert_eq!(read_modes_file_at(&path).unwrap_err(), "Mode files must be 256 KiB or smaller.");
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(read_modes_file_at(&path).unwrap_err(), "Mode files must be valid UTF-8.");
        for invalid in [Path::new("relative.json"), temp.path(), Path::new("/missing/private.json"), Path::new("/private.txt")] {
            let error = read_modes_file_at(invalid).unwrap_err();
            assert!(!error.contains("private"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_special_files() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.json");
        let link = temp.path().join("link.json");
        fs::write(&target, "{}").unwrap();
        symlink(&target, &link).unwrap();
        assert!(read_modes_file_at(&link).is_err());
        let socket = temp.path().join("socket.json");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(read_modes_file_at(&socket).is_err());
    }
}
