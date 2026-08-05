//! Reusable direct-child ownership primitives for bundled Murmur helpers.
//!
//! The executable path and environment are host-owned. Each helper becomes the
//! leader of a new process group in the parent's session without daemonizing.
//! Forced termination targets
//! the group derived from the exact owned PID, then waits for that PID and proves
//! no member of the owned process group remains. This covers descendants that
//! inherit the group; it does not claim control over a process that deliberately
//! escapes with `setsid`/`setpgid`. The separately signed sandboxed capture helper
//! is therefore forbidden from exposing any process-spawn surface.

use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedTermination {
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub process_group_empty: bool,
}

pub struct ManagedChild {
    child: Child,
    pid: u32,
    termination_armed: bool,
    #[cfg(test)]
    drop_fallback_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ManagedChild {
    fn from_spawned_child(child: Child) -> Self {
        let pid = child.id();
        Self {
            child,
            pid,
            termination_armed: true,
            #[cfg(test)]
            drop_fallback_count: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    pub fn spawn(
        executable: &Path,
        test_environment: &[(String, String)],
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout)> {
        Self::spawn_with_arguments(executable, &[], test_environment)
    }

    pub fn spawn_with_arguments(
        executable: &Path,
        arguments: &[&str],
        test_environment: &[(String, String)],
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout)> {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        #[cfg(any(debug_assertions, feature = "llm-test-support"))]
        for (key, value) in test_environment {
            command.env(key, value);
        }
        #[cfg(not(any(debug_assertions, feature = "llm-test-support")))]
        let _ = test_environment;

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    // A dedicated process group enables descendant cleanup
                    // without setsid/fork/chdir daemonization that could weaken
                    // the macOS responsible-code chain used for TCC attribution.
                    if libc::setpgid(0, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let max_fd = libc::getdtablesize();
                    let mut fd = 3;
                    while fd < max_fd {
                        libc::close(fd);
                        fd += 1;
                    }
                    Ok(())
                });
            }
        }

        let mut child = command.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing helper stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing helper stdout")
        })?;
        Ok((Self::from_spawned_child(child), stdin, stdout))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub fn wait_for_exit(&mut self, deadline: Instant) -> Option<ConfirmedTermination> {
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    let termination = self.confirmed(status);
                    if self.confirm_process_group_empty(deadline) {
                        self.termination_armed = false;
                        return Some(termination);
                    }
                    return None;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(EXIT_POLL_INTERVAL);
                }
                Ok(None) | Err(_) => return None,
            }
        }
    }

    pub fn hard_kill_confirmed(&mut self, deadline: Instant) -> Option<ConfirmedTermination> {
        #[cfg(unix)]
        unsafe {
            // The direct child is the process-group leader created by setpgid().
            // A negative PID therefore targets only the group owned by this
            // exact child and any fault-injected descendants that inherit it.
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();

        self.wait_for_exit(deadline)
    }

    fn confirmed(&self, status: std::process::ExitStatus) -> ConfirmedTermination {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        ConfirmedTermination {
            exit_code: status.code(),
            #[cfg(unix)]
            exit_signal: status.signal(),
            #[cfg(not(unix))]
            exit_signal: None,
            process_group_empty: true,
        }
    }

    fn confirm_process_group_empty(&self, deadline: Instant) -> bool {
        #[cfg(unix)]
        loop {
            let result = unsafe { libc::kill(-(self.pid as i32), 0) };
            if result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return true;
            }
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(EXIT_POLL_INTERVAL);
        }
        #[cfg(not(unix))]
        {
            let _ = deadline;
            true
        }
    }

    #[cfg(test)]
    fn drop_fallback_observer(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        std::sync::Arc::clone(&self.drop_fallback_count)
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.termination_armed {
            #[cfg(test)]
            self.drop_fallback_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = self.hard_kill_confirmed(Instant::now() + Duration::from_millis(250));
        }
    }
}

pub fn bundled_sibling(name: &str) -> Result<PathBuf, ()> {
    let executable = std::env::current_exe().map_err(|_| ())?;
    let directory = executable.parent().ok_or(())?;
    let candidate = directory.join(name);
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|_| ())?;
    (metadata.is_file() && !metadata.file_type().is_symlink())
        .then_some(candidate)
        .ok_or(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn confirmed_normal_exit_disarms_drop_without_resignalling() {
        let (mut child, stdin, stdout) =
            ManagedChild::spawn(Path::new("/usr/bin/true"), &[]).unwrap();
        drop((stdin, stdout));
        let observer = child.drop_fallback_observer();
        let termination = child
            .wait_for_exit(Instant::now() + Duration::from_secs(1))
            .expect("normal exit and empty process group must be confirmed");
        assert_eq!(termination.exit_code, Some(0));
        assert!(!child.termination_armed);
        drop(child);
        assert_eq!(observer.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn confirmed_hard_kill_disarms_drop_without_resignalling() {
        let (mut child, stdin, stdout) =
            ManagedChild::spawn(Path::new("/usr/bin/yes"), &[]).unwrap();
        drop((stdin, stdout));
        let observer = child.drop_fallback_observer();
        let termination = child
            .hard_kill_confirmed(Instant::now() + Duration::from_secs(1))
            .expect("hard kill and empty process group must be confirmed");
        assert_eq!(termination.exit_signal, Some(libc::SIGKILL));
        assert!(!child.termination_armed);
        drop(child);
        assert_eq!(observer.load(Ordering::SeqCst), 0);
    }
}
