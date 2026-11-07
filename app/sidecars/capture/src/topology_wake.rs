//! Self-pipe wakeup. The registered Core Audio listener receives a process-lifetime
//! instance: neither its client pointer nor descriptors can be retired/reused while
//! a callback is in flight, even if listener removal fails during process teardown.
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub(super) struct TopologyWake {
    read: OwnedFd,
    write: OwnedFd,
    changed: AtomicBool,
}

impl TopologyWake {
    pub(super) fn new() -> io::Result<Self> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let wake = Self {
            read: unsafe { OwnedFd::from_raw_fd(fds[0]) },
            write: unsafe { OwnedFd::from_raw_fd(fds[1]) },
            changed: AtomicBool::new(false),
        };
        for fd in [&wake.read, &wake.write] {
            if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == -1
                || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(wake)
    }

    /// Callback boundary: coalescing atomic + nonblocking one-byte syscall only.
    pub(super) fn invalidate(&self) {
        if !self.changed.swap(true, Ordering::AcqRel) {
            self.notify();
        }
    }

    pub(super) fn notify(&self) {
        let byte = [1u8];
        loop {
            let result = unsafe { libc::write(self.write.as_raw_fd(), byte.as_ptr().cast(), 1) };
            if result >= 0 || unsafe { *libc::__error() } != libc::EINTR {
                // EAGAIN means the pipe already holds a wake. Both descriptors
                // remain open for the entire registered callback lifetime.
                return;
            }
        }
    }

    pub(super) fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }

    /// Deadline is used by deterministic tests; productive idle waiting is infinite.
    pub(super) fn wait(&self, deadline: Option<Instant>) -> io::Result<bool> {
        loop {
            let timeout = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Ok(false);
                    }
                    remaining
                        .as_millis()
                        .saturating_add(1)
                        .min(i32::MAX as u128) as i32
                }
                None => -1,
            };
            let mut fd = libc::pollfd {
                fd: self.read.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut fd, 1, timeout) };
            if result == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(error);
            }
            if result == 0 {
                continue;
            }
            if fd.revents & libc::POLLIN == 0 {
                return Err(io::Error::other("topology wake pipe disconnected"));
            }
            let mut bytes = [0u8; 256];
            loop {
                let count = unsafe {
                    libc::read(
                        self.read.as_raw_fd(),
                        bytes.as_mut_ptr().cast(),
                        bytes.len(),
                    )
                };
                if count > 0 {
                    continue;
                }
                let error = io::Error::last_os_error();
                if count < 0 && error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                if count < 0 && error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                return Err(io::Error::other("topology wake pipe read failed"));
            }
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[test]
    fn coalesces_invalidations_and_retains_wake_before_wait() {
        let wake = TopologyWake::new().unwrap();
        for _ in 0..10_000 {
            wake.invalidate();
        }
        assert!(wake
            .wait(Some(Instant::now() + Duration::from_secs(1)))
            .unwrap());
        assert!(wake.take_changed());
        assert!(!wake.take_changed());
        assert!(!wake
            .wait(Some(Instant::now() + Duration::from_millis(10)))
            .unwrap());
        wake.invalidate();
        assert!(wake
            .wait(Some(Instant::now() + Duration::from_secs(1)))
            .unwrap());
        assert!(wake.take_changed());
    }

    #[test]
    fn control_shutdown_and_eof_wake_without_topology_change() {
        for message in [Some(()), None] {
            let wake = Arc::new(TopologyWake::new().unwrap());
            let worker = wake.clone();
            let (tx, rx) = mpsc::channel();
            let join = std::thread::spawn(move || {
                tx.send(message).unwrap();
                worker.notify();
            });
            assert!(wake
                .wait(Some(Instant::now() + Duration::from_secs(1)))
                .unwrap());
            assert_eq!(rx.try_recv().unwrap(), message);
            assert!(!wake.take_changed());
            join.join().unwrap();
        }
    }

    #[test]
    fn invalidation_during_drain_remains_observable() {
        let wake = Arc::new(TopologyWake::new().unwrap());
        let worker = wake.clone();
        let (tx, rx) = mpsc::sync_channel(0);
        let (ack_tx, ack_rx) = mpsc::sync_channel(0);
        let join = std::thread::spawn(move || {
            for _ in 0..500 {
                rx.recv().unwrap();
                worker.invalidate();
                ack_tx.send(()).unwrap();
            }
        });
        for _ in 0..500 {
            tx.send(()).unwrap();
            assert!(wake
                .wait(Some(Instant::now() + Duration::from_secs(1)))
                .unwrap());
            ack_rx.recv().unwrap();
            assert!(wake.take_changed());
        }
        join.join().unwrap();
    }
}
