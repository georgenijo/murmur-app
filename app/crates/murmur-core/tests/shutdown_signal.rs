//! Real thread-targeted signal with a saturated nonblocking pipe; no app/profile.
#[path = "../../murmur-services/src/shutdown/signal.rs"]
mod signal;
#[cfg(unix)]
#[test]
fn full_pipe_signal_preserves_interrupted_errno() {
    use std::sync::atomic::{AtomicI32, Ordering};
    static FD: AtomicI32 = AtomicI32::new(-1);
    extern "C" fn handler(value: libc::c_int) {
        signal::notify(FD.load(Ordering::Relaxed), value);
    }
    unsafe {
        #[cfg(target_os = "macos")]
        let errno = libc::__error();
        #[cfg(not(target_os = "macos"))]
        let errno = libc::__errno_location();
        let mut fds = [-1; 2];
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        assert_ne!(libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK), -1);
        let bytes = [0u8; 4096];
        while libc::write(fds[1], bytes.as_ptr().cast(), bytes.len()) > 0 {}
        assert_eq!(*errno, libc::EAGAIN);
        FD.store(fds[1], Ordering::Relaxed);
        let mut action: libc::sigaction = std::mem::zeroed();
        let mut old: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        assert_eq!(libc::sigaction(libc::SIGUSR1, &action, &mut old), 0);
        *errno = libc::EDOM;
        assert_eq!(libc::pthread_kill(libc::pthread_self(), libc::SIGUSR1), 0);
        let observed = *errno;
        assert_eq!(
            libc::sigaction(libc::SIGUSR1, &old, std::ptr::null_mut()),
            0
        );
        FD.store(-1, Ordering::Relaxed);
        libc::close(fds[0]);
        libc::close(fds[1]);
        assert_eq!(observed, libc::EDOM);
    }
}
