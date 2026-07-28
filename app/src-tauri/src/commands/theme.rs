use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAIN_WINDOW_LABEL: &str = "main";
const MAX_THEME_FILE_BYTES: usize = 64 * 1024;
const TEMP_FILE_ATTEMPTS: usize = 32;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn require_main_window(label: &str) -> Result<(), String> {
    if label == MAIN_WINDOW_LABEL {
        Ok(())
    } else {
        Err("Theme file exchange is only available from the main window.".to_string())
    }
}

fn validated_file_path(path: &str, invalid_message: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err(invalid_message.to_string());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(invalid_message.to_string());
    }
    Ok(path)
}

fn open_regular_file(path: &Path) -> Result<File, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "Murmur could not read the theme file.".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Choose a valid theme file.".to_string());
    }
    if metadata.len() > MAX_THEME_FILE_BYTES as u64 {
        return Err("Theme files must be 64 KiB or smaller.".to_string());
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "Murmur could not read the theme file.".to_string())?;
    if !file
        .metadata()
        .map_err(|_| "Murmur could not read the theme file.".to_string())?
        .is_file()
    {
        return Err("Choose a valid theme file.".to_string());
    }
    Ok(file)
}

fn read_theme_file_at(path: &Path) -> Result<String, String> {
    let file = open_regular_file(path)?;
    let mut bytes = Vec::with_capacity(MAX_THEME_FILE_BYTES.min(4096));
    file.take(MAX_THEME_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Murmur could not read the theme file.".to_string())?;
    if bytes.len() > MAX_THEME_FILE_BYTES {
        return Err("Theme files must be 64 KiB or smaller.".to_string());
    }
    String::from_utf8(bytes).map_err(|_| "Theme files must be valid UTF-8.".to_string())
}

fn validate_export_target(path: &Path) -> Result<Option<fs::Permissions>, String> {
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| "Choose a valid theme export destination.".to_string())?;
    if parent.as_os_str().is_empty() {
        return Err("Choose a valid theme export destination.".to_string());
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("Choose a valid theme export destination.".to_string())
        }
        Ok(metadata) => Ok(Some(metadata.permissions())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Murmur could not write the theme export.".to_string()),
    }
}

fn create_sibling_temp(
    path: &Path,
    target_permissions: Option<fs::Permissions>,
) -> Result<(PathBuf, File), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Choose a valid theme export destination.".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Choose a valid theme export destination.".to_string())?;

    for _ in 0..TEMP_FILE_ATTEMPTS {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(
            ".{file_name}.murmur-theme.{}.{sequence}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temp_path) {
            Ok(file) => {
                if let Some(permissions) = target_permissions.clone() {
                    if fs::set_permissions(&temp_path, permissions).is_err() {
                        drop(file);
                        let _ = fs::remove_file(&temp_path);
                        return Err("Murmur could not write the theme export.".to_string());
                    }
                }
                return Ok((temp_path, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Murmur could not write the theme export.".to_string()),
        }
    }

    Err("Murmur could not write the theme export.".to_string())
}

fn write_theme_file_at_with_io<F, S>(
    path: &Path,
    contents: &str,
    publish: F,
    sync_parent: S,
) -> Result<(), String>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
    S: FnOnce(&Path) -> io::Result<()>,
{
    if contents.len() > MAX_THEME_FILE_BYTES {
        return Err("Theme exports must be 64 KiB or smaller.".to_string());
    }
    let target_permissions = validate_export_target(path)?;

    let (temp_path, mut temp_file) = create_sibling_temp(path, target_permissions)?;
    let write_result = temp_file
        .write_all(contents.as_bytes())
        .and_then(|_| temp_file.sync_all());
    drop(temp_file);
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err("Murmur could not write the theme export.".to_string());
    }

    if publish(&temp_path, path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err("Murmur could not publish the theme export.".to_string());
    }
    let parent = path
        .parent()
        .expect("validated theme export path always has a parent");
    if sync_parent(parent).is_err() {
        return Err(
            "Theme export was written, but Murmur could not finish syncing its folder.".to_string(),
        );
    }
    Ok(())
}

fn sync_export_parent(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
        Ok(())
    }
}

fn write_theme_file_at(path: &Path, contents: &str) -> Result<(), String> {
    write_theme_file_at_with_io(
        path,
        contents,
        |from, to| fs::rename(from, to),
        sync_export_parent,
    )
}

#[tauri::command]
pub fn read_theme_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    require_main_window(window.label())?;
    let path = validated_file_path(&path, "Choose a valid theme file.")?;
    read_theme_file_at(&path)
}

#[tauri::command]
pub fn write_theme_file(
    window: tauri::WebviewWindow,
    path: String,
    contents: String,
) -> Result<(), String> {
    require_main_window(window.label())?;
    let path = validated_file_path(&path, "Choose a valid theme export destination.")?;
    write_theme_file_at(&path, &contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sibling_temp_files(path: &Path) -> Vec<PathBuf> {
        let parent = path.parent().unwrap();
        let prefix = format!(
            ".{}.murmur-theme.",
            path.file_name().unwrap().to_string_lossy()
        );
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
            })
            .collect()
    }

    #[test]
    fn theme_file_exchange_is_strictly_scoped_to_main() {
        assert!(require_main_window(MAIN_WINDOW_LABEL).is_ok());
        for label in ["log-viewer", "transform-review", "overlay", "", "main-copy"] {
            assert!(
                require_main_window(label).is_err(),
                "unexpected theme file access for {label:?}"
            );
        }
        assert!(validated_file_path("", "invalid").is_err());
        assert!(validated_file_path("relative.json", "invalid").is_err());
    }

    #[test]
    fn read_accepts_exactly_64_kib_and_rejects_one_more_byte() {
        let temp = tempfile::tempdir().unwrap();
        let exact = temp.path().join("exact.json");
        let oversized = temp.path().join("oversized.json");
        fs::write(&exact, vec![b'a'; MAX_THEME_FILE_BYTES]).unwrap();
        fs::write(&oversized, vec![b'a'; MAX_THEME_FILE_BYTES + 1]).unwrap();

        assert_eq!(
            read_theme_file_at(&exact).unwrap().len(),
            MAX_THEME_FILE_BYTES
        );
        assert_eq!(
            read_theme_file_at(&oversized).unwrap_err(),
            "Theme files must be 64 KiB or smaller."
        );
    }

    #[test]
    fn read_rejects_invalid_utf8_directories_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let invalid = temp.path().join("invalid.json");
        fs::write(&invalid, [0xff, 0xfe]).unwrap();
        assert_eq!(
            read_theme_file_at(&invalid).unwrap_err(),
            "Theme files must be valid UTF-8."
        );
        assert_eq!(
            read_theme_file_at(temp.path()).unwrap_err(),
            "Choose a valid theme file."
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp.path().join("target.json");
            let link = temp.path().join("link.json");
            fs::write(&target, "{}").unwrap();
            symlink(&target, &link).unwrap();
            assert_eq!(
                read_theme_file_at(&link).unwrap_err(),
                "Choose a valid theme file."
            );
        }
    }

    #[test]
    fn write_is_byte_bounded_and_atomically_replaces_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("theme.json");
        fs::write(&path, "old").unwrap();
        let exact = "é".repeat(MAX_THEME_FILE_BYTES / 2);

        write_theme_file_at(&path, &exact).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), exact);
        assert!(sibling_temp_files(&path).is_empty());
        let oversized = format!("{exact}a");
        assert_eq!(
            write_theme_file_at(&path, &oversized).unwrap_err(),
            "Theme exports must be 64 KiB or smaller."
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), exact);
    }

    #[test]
    fn failed_publish_preserves_existing_target_and_cleans_temp() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("theme.json");
        fs::write(&path, "original").unwrap();

        let error = write_theme_file_at_with_io(
            &path,
            "replacement",
            |_, _| Err(io::Error::new(io::ErrorKind::PermissionDenied, "test")),
            |_| panic!("directory sync must not run after failed publish"),
        )
        .unwrap_err();

        assert_eq!(error, "Murmur could not publish the theme export.");
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert!(sibling_temp_files(&path).is_empty());
    }

    #[test]
    fn directory_sync_runs_after_publish_and_has_an_explicit_failure_contract() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("theme.json");
        fs::write(&path, "original").unwrap();
        let mut sync_called = false;

        let error = write_theme_file_at_with_io(
            &path,
            "replacement",
            |from, to| fs::rename(from, to),
            |parent| {
                sync_called = true;
                assert_eq!(parent, temp.path());
                Err(io::Error::new(io::ErrorKind::Other, "test"))
            },
        )
        .unwrap_err();

        assert!(sync_called);
        assert_eq!(
            error,
            "Theme export was written, but Murmur could not finish syncing its folder."
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        assert!(sibling_temp_files(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn write_uses_0600_for_new_files_and_preserves_existing_mode() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let new_path = temp.path().join("new.json");
        write_theme_file_at(&new_path, "{}").unwrap();
        assert_eq!(
            fs::metadata(&new_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let existing_path = temp.path().join("existing.json");
        fs::write(&existing_path, "original").unwrap();
        fs::set_permissions(&existing_path, fs::Permissions::from_mode(0o640)).unwrap();
        write_theme_file_at(&existing_path, "replacement").unwrap();
        assert_eq!(
            fs::metadata(&existing_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn write_rejects_symlinks_and_special_targets() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            write_theme_file_at(temp.path(), "{}").unwrap_err(),
            "Choose a valid theme export destination."
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp.path().join("target.json");
            let link = temp.path().join("link.json");
            fs::write(&target, "original").unwrap();
            symlink(&target, &link).unwrap();
            assert_eq!(
                write_theme_file_at(&link, "{}").unwrap_err(),
                "Choose a valid theme export destination."
            );
            assert_eq!(fs::read_to_string(&target).unwrap(), "original");
        }
    }
}
