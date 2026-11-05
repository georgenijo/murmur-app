//! Remove retired private stores now that their deletion surfaces are gone.
//! Scan only the legacy quarantine directory; never read content or follow a symlink.

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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn clean_quarantine(dir: &OwnedFd) -> std::io::Result<()> {
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let duplicate = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { libc::close(duplicate) };
            return Err(error);
        }
        let stream = Directory(stream);
        let mut failed = false;
        loop {
            #[cfg(target_os = "macos")]
            let errno = unsafe { libc::__error() };
            #[cfg(target_os = "linux")]
            let errno = unsafe { libc::__errno_location() };
            unsafe { *errno = 0 };
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if unsafe { *errno } != 0 {
                    failed = true;
                }
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
            let bytes = name.to_bytes();
            let base = [b"-wal".as_slice(), b"-shm", b"-journal"]
                .iter()
                .find_map(|suffix| bytes.strip_suffix(*suffix))
                .unwrap_or(bytes);
            let timestamp = base
                .strip_prefix(b"query-history-corrupt-")
                .and_then(|s| s.strip_suffix(b".sqlite3"));
            if timestamp.is_some_and(|s| !s.is_empty() && s.iter().all(u8::is_ascii_digit))
                && unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
            {
                failed = true;
            }
        }
        if failed {
            Err(std::io::Error::other("quarantine cleanup failed"))
        } else {
            Ok(())
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn clean_quarantine(_dir: &OwnedFd) -> std::io::Result<()> {
        Err(std::io::Error::other("quarantine cleanup unsupported"))
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
        if directory == c"query-history" {
            match open_dir(Some(&dir), c"quarantine") {
                Ok(quarantine) => {
                    if clean_quarantine(&quarantine).is_err() {
                        failed = true;
                    }
                    // Remove only an empty directory. Unrelated entries remain untouched.
                    if unsafe {
                        libc::unlinkat(dir.as_raw_fd(), c"quarantine".as_ptr(), libc::AT_REMOVEDIR)
                    } != 0
                    {
                        let error = std::io::Error::last_os_error();
                        if error.kind() != std::io::ErrorKind::NotFound
                            && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
                        {
                            failed = true;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => failed = true,
            }
        }
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
    fn removes_quarantined_stores_and_preserves_unrelated_entries_and_symlink_targets() {
        let temp = tempfile::tempdir().unwrap();
        let root = root(&temp);
        let quarantine = root.join("query-history/quarantine");
        fs::create_dir_all(&quarantine).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("target"), "keep").unwrap();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            fs::write(
                quarantine.join(format!("query-history-corrupt-123.sqlite3{suffix}")),
                "private",
            )
            .unwrap();
        }
        symlink(
            outside.path().join("target"),
            quarantine.join("query-history-corrupt-456.sqlite3-wal"),
        )
        .unwrap();
        for name in [
            "keep.txt",
            "query-history-corrupt-.sqlite3",
            "query-history-corrupt-abc.sqlite3",
            "query-history-corrupt-123.sqlite3.extra",
        ] {
            fs::write(quarantine.join(name), "keep").unwrap();
        }
        cleanup(&root).unwrap();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            assert!(fs::symlink_metadata(
                quarantine.join(format!("query-history-corrupt-123.sqlite3{suffix}"))
            )
            .is_err());
        }
        assert!(
            fs::symlink_metadata(quarantine.join("query-history-corrupt-456.sqlite3-wal")).is_err()
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("target")).unwrap(),
            "keep"
        );
        for name in [
            "keep.txt",
            "query-history-corrupt-.sqlite3",
            "query-history-corrupt-abc.sqlite3",
            "query-history-corrupt-123.sqlite3.extra",
        ] {
            fs::remove_file(quarantine.join(name)).unwrap();
        }
        cleanup(&root).unwrap();
        assert!(!quarantine.exists());
    }

    #[test]
    fn refuses_symlinked_quarantine_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = root(&temp);
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.join("query-history")).unwrap();
        fs::write(
            outside.path().join("query-history-corrupt-123.sqlite3"),
            "keep",
        )
        .unwrap();
        symlink(outside.path(), root.join("query-history/quarantine")).unwrap();
        assert!(cleanup(&root).is_err());
        assert!(outside
            .path()
            .join("query-history-corrupt-123.sqlite3")
            .exists());
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
