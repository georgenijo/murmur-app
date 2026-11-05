//! Remove retired private stores now that their deletion surfaces are gone.
//! Never scan directories, read content, or follow a symlink.

#[cfg(unix)]
pub(crate) fn cleanup(app_data: &std::path::Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    fn open_dir(parent: Option<&OwnedFd>, name: &std::ffi::CStr) -> std::io::Result<OwnedFd> {
        let fd = unsafe {
            libc::openat(
                parent.map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    fn open_root(path: &Path) -> std::io::Result<OwnedFd> {
        if !path.is_absolute() {
            return Err(std::io::Error::other(
                "expected absolute app data directory",
            ));
        }
        let mut dir = open_dir(None, c"/")?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    dir = open_dir(Some(&dir), &CString::new(name.as_bytes())?)?;
                }
                _ => return Err(std::io::Error::other("invalid app data directory")),
            }
        }
        Ok(dir)
    }

    let root = match open_root(app_data) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut failed = false;
    for (directory, file) in [
        (c"query-history", "query-history.sqlite3"),
        (c"assistant", "conversations-v1.json"),
    ] {
        // Each directory stays pinned even if its name is replaced during cleanup.
        let dir = match open_dir(Some(&root), directory) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let name = CString::new(format!("{file}{suffix}"))?;
            // unlinkat removes an entry (including a symlink itself), never its target.
            // Directories are refused because AT_REMOVEDIR is deliberately absent.
            if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
            {
                failed = true;
            }
        }
    }
    if failed {
        Err(std::io::Error::other("legacy conversation cleanup failed"))
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
pub(crate) fn cleanup(_app_data: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::cleanup;
    use std::{fs, os::unix::fs::symlink};

    fn root(temp: &tempfile::TempDir) -> std::path::PathBuf {
        temp.path().canonicalize().unwrap()
    }

    #[test]
    fn removes_exact_stores_and_journals_preserving_all_other_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = root(&temp);
        let stores = [
            ("query-history", "query-history.sqlite3"),
            ("assistant", "conversations-v1.json"),
        ];
        for (dir, file) in stores {
            fs::create_dir(root.join(dir)).unwrap();
            for suffix in ["", "-wal", "-shm", "-journal"] {
                fs::write(
                    root.join(dir).join(format!("{file}{suffix}")),
                    "private fixture",
                )
                .unwrap();
            }
            for keep in [
                "keep.txt",
                "other.sqlite3",
                ".unrecognized.tmp",
                "conversations-v2.json",
            ] {
                fs::write(root.join(dir).join(keep), "keep").unwrap();
            }
        }
        for dir in ["meetings", "knowledge", "diagnostics"] {
            fs::create_dir(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("query-history.sqlite3"), "keep").unwrap();
        }
        fs::write(root.join("conversations-v1.json"), "keep").unwrap();
        cleanup(&root).unwrap();
        cleanup(&root).unwrap(); // idempotent, including absent journals
        for (dir, file) in stores {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                assert!(!root.join(dir).join(format!("{file}{suffix}")).exists());
            }
            for keep in [
                "keep.txt",
                "other.sqlite3",
                ".unrecognized.tmp",
                "conversations-v2.json",
            ] {
                assert_eq!(
                    fs::read_to_string(root.join(dir).join(keep)).unwrap(),
                    "keep"
                );
            }
        }
        for dir in ["meetings", "knowledge", "diagnostics"] {
            assert!(root.join(dir).join("query-history.sqlite3").exists());
        }
        assert!(root.join("conversations-v1.json").exists());
    }

    #[test]
    fn removes_file_symlinks_without_touching_targets_or_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = root(&temp);
        fs::write(root.join("outside"), "keep").unwrap();
        for (dir, file) in [
            ("query-history", "query-history.sqlite3"),
            ("assistant", "conversations-v1.json"),
        ] {
            fs::create_dir(root.join(dir)).unwrap();
            symlink(root.join("outside"), root.join(dir).join(file)).unwrap();
            fs::create_dir(root.join(dir).join(format!("{file}-wal"))).unwrap();
        }
        assert!(cleanup(&root).is_err());
        assert_eq!(fs::read_to_string(root.join("outside")).unwrap(), "keep");
        for (dir, file) in [
            ("query-history", "query-history.sqlite3"),
            ("assistant", "conversations-v1.json"),
        ] {
            assert!(fs::symlink_metadata(root.join(dir).join(file)).is_err());
            assert!(root.join(dir).join(format!("{file}-wal")).is_dir());
        }
    }

    #[test]
    fn refuses_symlinked_store_root_and_ancestor_but_continues_other_store() {
        let temp = tempfile::tempdir().unwrap();
        let root = root(&temp);
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("query-history.sqlite3"), "keep").unwrap();
        symlink(outside.path(), root.join("query-history")).unwrap();
        fs::create_dir(root.join("assistant")).unwrap();
        fs::write(
            root.join("assistant/conversations-v1.json"),
            "private fixture",
        )
        .unwrap();
        assert!(cleanup(&root).is_err());
        assert!(outside.path().join("query-history.sqlite3").exists());
        assert!(!root.join("assistant/conversations-v1.json").exists());
        let link = root.join("linked-root");
        symlink(outside.path(), &link).unwrap();
        assert!(cleanup(&link).is_err());
        fs::create_dir(outside.path().join("app")).unwrap();
        assert!(cleanup(&link.join("app")).is_err());
        assert!(cleanup(&root.join("missing")).is_ok());
    }
}
