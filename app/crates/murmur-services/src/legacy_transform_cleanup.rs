//! Remove retired transform captures, which no longer have a deletion UI.

#[cfg(unix)]
pub(crate) fn cleanup(app_data: &std::path::Path) -> std::io::Result<()> {
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let root = CString::new(app_data.as_os_str().as_bytes())?;
    // Open every component relative to its parent, refusing symlinks. Keeping
    // the final directory open also prevents a rename from redirecting deletes.
    let fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    for component in [c"diagnostics", c"transforms", c"transform-captures"] {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            return if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            };
        }
        dir = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    let scan_fd = unsafe { libc::dup(dir.as_raw_fd()) };
    if scan_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stream = unsafe { libc::fdopendir(scan_fd) };
    if stream.is_null() {
        unsafe {
            libc::close(scan_fd);
        }
        return Err(std::io::Error::last_os_error());
    }
    let mut failed = false;
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if name.to_bytes().ends_with(b".json")
            && unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0
        {
            failed = true;
        }
    }
    unsafe {
        libc::closedir(stream);
    }
    if failed {
        Err(std::io::Error::other("legacy capture cleanup failed"))
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

    #[test]
    fn removes_only_legacy_json_entries_without_following_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let captures = root
            .path()
            .join("diagnostics/transforms/transform-captures");
        let outside = root.path().join("outside");
        fs::create_dir_all(&captures).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(captures.join("capture.json"), "private").unwrap();
        fs::write(captures.join("keep.txt"), "keep").unwrap();
        fs::write(outside.join("keep.json"), "keep").unwrap();
        symlink(outside.join("keep.json"), captures.join("link.json")).unwrap();
        cleanup(root.path()).unwrap();
        assert!(!captures.join("capture.json").exists());
        assert!(!captures.join("link.json").exists());
        assert!(captures.join("keep.txt").exists());
        assert!(outside.join("keep.json").exists());
        fs::remove_dir_all(root.path().join("diagnostics/transforms")).unwrap();
        symlink(&outside, root.path().join("diagnostics/transforms")).unwrap();
        assert!(cleanup(root.path()).is_err());
        assert!(outside.join("keep.json").exists());
    }
}
