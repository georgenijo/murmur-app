//! Reusable direct-child ownership primitives for bundled Murmur helpers.
//!
//! The executable path and environment are host-owned. Each helper becomes the
//! leader of a new process group in the parent's session without daemonizing.
//! Forced termination targets
//! the group derived from the exact owned PID, then waits for that PID and proves
//! no member of the owned process group remains.

use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedTermination {
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub process_group_empty: bool,
}

pub struct ManagedChild {
    child: Child,
    pid: u32,
}

impl ManagedChild {
    pub fn spawn(
        executable: &Path,
        test_environment: &[(String, String)],
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout)> {
        let mut command = Command::new(executable);
        command
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
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing helper stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing helper stdout")
        })?;
        Ok((Self { child, pid }, stdin, stdout))
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
                    return self
                        .confirm_process_group_empty(deadline)
                        .then_some(termination);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
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
            // exact child and also removes any fault-injected descendants.
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
            std::thread::sleep(Duration::from_millis(5));
        }
        #[cfg(not(unix))]
        {
            let _ = deadline;
            true
        }
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            // Always target the owned group: the direct child may already have
            // exited while a descendant remains alive in the same group.
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        #[cfg(not(unix))]
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
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
