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
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(1);

pub(crate) const USER_CLI_BASE_ENVIRONMENT: [&str; 8] = [
    "HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME",
];

pub(crate) fn apply_user_cli_base_environment(command: &mut Command) {
    command.env_clear();
    for key in USER_CLI_BASE_ENVIRONMENT {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
}

pub(crate) fn user_cli_base_environment() -> Vec<(String, std::ffi::OsString)> {
    USER_CLI_BASE_ENVIRONMENT
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key.to_string(), value)))
        .collect()
}

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

        Self::spawn_command(command)
    }

    /// Spawn an explicitly configured user CLI without a shell. `arguments`
    /// are passed straight to `Command::args`; callers append any untrusted
    /// content as its own element and never build a command string.
    ///
    /// The child receives only the small environment needed by common CLI
    /// shims and credential stores plus the two explicitly declared config-dir
    /// additions. Arbitrary parent variables (including API keys) are
    /// deliberately not forwarded, and callers cannot override a base key.
    /// `USER`/`LOGNAME` carry no secret (the username is already visible in
    /// `HOME`) and are required on macOS: Claude Code derives its Keychain
    /// credential account name from `USER` and resolves to a nonexistent
    /// "unknown" account without it, reporting "Not logged in" even when the
    /// user is signed in.
    pub fn spawn_user_cli(
        executable: &Path,
        arguments: &[String],
        declared_environment: &[(String, String)],
        working_directory: &Path,
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout, ChildStderr)> {
        const DECLARED_ENVIRONMENT: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CODEX_HOME"];
        let mut seen = std::collections::HashSet::new();
        for (key, value) in declared_environment {
            if !DECLARED_ENVIRONMENT.contains(&key.as_str())
                || USER_CLI_BASE_ENVIRONMENT.contains(&key.as_str())
                || !seen.insert(key.as_str())
                || key.contains(['\0', '='])
                || value.contains('\0')
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid declared user CLI environment",
                ));
            }
        }
        if !working_directory.is_absolute() || !working_directory.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid user CLI working directory",
            ));
        }
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir(working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        apply_user_cli_base_environment(&mut command);
        for (key, value) in declared_environment {
            command.env(key, value);
        }
        Self::spawn_user_cli_command(command)
    }

    fn spawn_user_cli_command(
        mut command: Command,
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout, ChildStderr)> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
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
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing user CLI stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing user CLI stdout")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "missing user CLI stderr")
        })?;
        Ok((Self::from_spawned_child(child), stdin, stdout, stderr))
    }

    fn spawn_command(mut command: Command) -> std::io::Result<(Self, ChildStdin, ChildStdout)> {
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
        if !self.termination_armed {
            return self
                .child
                .try_wait()
                .ok()
                .flatten()
                .map(|status| self.confirmed(status));
        }
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
        // Once normal exit and an empty process group have been confirmed, do
        // not signal the numeric PGID again: the OS may eventually reuse it.
        if !self.termination_armed {
            return self.wait_for_exit(deadline);
        }
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

// Callers deliberately collapse every lookup failure to their own
// content-free error; exposing filesystem detail here would weaken that seam.
#[allow(clippy::result_unit_err)]
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
    use std::io::Read;
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
        let repeated = child
            .hard_kill_confirmed(Instant::now() + Duration::from_secs(1))
            .expect("an already confirmed child remains confirmed without signalling");
        assert_eq!(repeated.exit_code, Some(0));
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

    #[test]
    fn user_cli_receives_metacharacters_as_one_literal_argument() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("shell-interpolation-must-not-run");
        let question = format!("what?; $(touch {}) && echo unsafe", marker.display());
        let arguments = vec!["%s".to_string(), question.clone()];
        let (mut child, stdin, mut stdout, stderr) = ManagedChild::spawn_user_cli(
            Path::new("/usr/bin/printf"),
            &arguments,
            &[],
            directory.path(),
        )
        .unwrap();
        drop((stdin, stderr));
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        let termination = child
            .wait_for_exit(Instant::now() + Duration::from_secs(1))
            .expect("printf must exit cleanly");
        assert_eq!(termination.exit_code, Some(0));
        assert_eq!(output, question);
        assert!(
            !marker.exists(),
            "question content must never be shell-evaluated"
        );
    }

    #[test]
    fn user_cli_environment_contains_only_explicit_allowlist() {
        let directory = tempfile::tempdir().unwrap();
        let (mut child, stdin, mut stdout, stderr) =
            ManagedChild::spawn_user_cli(Path::new("/usr/bin/env"), &[], &[], directory.path())
                .unwrap();
        drop((stdin, stderr));
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        child
            .wait_for_exit(Instant::now() + Duration::from_secs(1))
            .expect("env must exit cleanly");
        let allowed = [
            "HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME",
        ];
        for line in output.lines() {
            let key = line.split_once('=').map(|(key, _)| key).unwrap_or(line);
            assert!(
                allowed.contains(&key),
                "unexpected inherited environment key: {key}"
            );
        }
    }

    #[test]
    fn user_cli_accepts_only_explicit_config_directory_additions() {
        let directory = tempfile::tempdir().unwrap();
        let additions = vec![("CODEX_HOME".to_string(), "/tmp/codex-home".to_string())];
        let (mut child, stdin, mut stdout, stderr) = ManagedChild::spawn_user_cli(
            Path::new("/usr/bin/env"),
            &[],
            &additions,
            directory.path(),
        )
        .unwrap();
        drop((stdin, stderr));
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        child
            .wait_for_exit(Instant::now() + Duration::from_secs(1))
            .expect("env must exit cleanly");
        assert!(output
            .lines()
            .any(|line| line == "CODEX_HOME=/tmp/codex-home"));

        for key in [
            "HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME",
        ] {
            let rejected = vec![(key.to_string(), "/tmp/override".to_string())];
            assert!(ManagedChild::spawn_user_cli(
                Path::new("/usr/bin/env"),
                &[],
                &rejected,
                directory.path(),
            )
            .is_err());
        }
        let secret = vec![("ANTHROPIC_API_KEY".to_string(), "secret".to_string())];
        assert!(ManagedChild::spawn_user_cli(
            Path::new("/usr/bin/env"),
            &[],
            &secret,
            directory.path(),
        )
        .is_err());
    }

    #[test]
    fn user_cli_hard_kill_confirms_descendant_process_group_is_empty() {
        let directory = tempfile::tempdir().unwrap();
        let arguments = vec!["-c".to_string(), "sleep 30 & wait".to_string()];
        let (mut child, stdin, stdout, stderr) =
            ManagedChild::spawn_user_cli(Path::new("/bin/sh"), &arguments, &[], directory.path())
                .unwrap();
        drop((stdin, stdout, stderr));
        let termination = child
            .hard_kill_confirmed(Instant::now() + Duration::from_secs(2))
            .expect("owned child and its descendant must be confirmed stopped");
        assert!(termination.process_group_empty);
    }
}
