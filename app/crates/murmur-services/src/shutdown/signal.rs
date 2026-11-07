#[cfg(unix)]
pub(crate) fn notify(fd: libc::c_int, signal: libc::c_int) {
    unsafe {
        #[cfg(target_os = "macos")]
        let errno = libc::__error();
        #[cfg(not(target_os = "macos"))]
        let errno = libc::__errno_location();
        let saved = *errno;
        if fd >= 0 {
            let byte = signal as u8;
            loop {
                if libc::write(fd, (&byte as *const u8).cast(), 1) >= 0 || *errno != libc::EINTR {
                    break;
                }
            }
            // EAGAIN means a notification is already pending; never block.
        }
        *errno = saved;
    }
}
