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

/// The only parent variables a user CLI ever inherits. Declared pairs (#550)
/// are layered underneath this list, never over it.
pub const USER_CLI_ENVIRONMENT_ALLOWLIST: [&str; 8] = [
    "HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME",
];

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
    /// shims and credential stores. Arbitrary parent variables (including API
    /// keys) are deliberately not forwarded. `USER`/`LOGNAME` carry no secret
    /// (the username is already visible in `HOME`) and are required on macOS:
    /// Claude Code derives its Keychain credential account name from `USER`
    /// and resolves to a nonexistent "unknown" account without it, reporting
    /// "Not logged in" even when the user is signed in.
    ///
    /// `declared_environment` carries the explicit name/value pairs the user
    /// added in Settings (#550). They are applied *before* the inherited
    /// allowlist so a declared pair can never shadow `HOME` or any other
    /// allowlist key even if the caller's validation were bypassed.
    ///
    /// stderr is piped rather than discarded: a provider CLI reports "not
    /// logged in", quota, and network failures there, and that tail is what
    /// makes an otherwise blank failure diagnosable.
    pub fn spawn_user_cli(
        executable: &Path,
        arguments: &[String],
        declared_environment: &[(String, String)],
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout, ChildStderr)> {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for (key, value) in declared_environment {
            command.env(key, value);
        }
        for key in USER_CLI_ENVIRONMENT_ALLOWLIST {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        Self::spawn_piped_command(command)
    }

    fn spawn_owned_group(mut command: Command) -> std::io::Result<Child> {
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
        command.spawn()
    }

    fn missing_pipe(name: &str) -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            format!("missing helper {name}"),
        )
    }

    fn spawn_command(command: Command) -> std::io::Result<(Self, ChildStdin, ChildStdout)> {
        let mut child = Self::spawn_owned_group(command)?;
        let stdin = child.stdin.take().ok_or_else(|| Self::missing_pipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Self::missing_pipe("stdout"))?;
        Ok((Self::from_spawned_child(child), stdin, stdout))
    }

    fn spawn_piped_command(
        command: Command,
    ) -> std::io::Result<(Self, ChildStdin, ChildStdout, ChildStderr)> {
        let mut child = Self::spawn_owned_group(command)?;
        let stdin = child.stdin.take().ok_or_else(|| Self::missing_pipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Self::missing_pipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Self::missing_pipe("stderr"))?;
        Ok((Self::from_spawned_child(child), stdin, stdout, stderr))
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
        let (mut child, stdin, mut stdout, stderr) =
            ManagedChild::spawn_user_cli(Path::new("/usr/bin/printf"), &arguments, &[]).unwrap();
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

    fn user_cli_environment(declared: &[(String, String)]) -> Vec<(String, String)> {
        let (mut child, stdin, mut stdout, stderr) =
            ManagedChild::spawn_user_cli(Path::new("/usr/bin/env"), &[], declared).unwrap();
        drop((stdin, stderr));
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        child
            .wait_for_exit(Instant::now() + Duration::from_secs(1))
            .expect("env must exit cleanly");
        output
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn user_cli_environment_contains_only_explicit_allowlist() {
        for (key, _) in user_cli_environment(&[]) {
            assert!(
                USER_CLI_ENVIRONMENT_ALLOWLIST.contains(&key.as_str()),
                "unexpected inherited environment key: {key}"
            );
        }
    }

    #[test]
    fn declared_variables_extend_the_allowlist_but_can_never_shadow_it() {
        // `CLAUDE_CONFIG_DIR` is exactly the kind of pair Settings declares.
        // `HOME` is the pair the validator refuses; even if one reached this
        // far, the inherited allowlist is applied last and still wins.
        let inherited_home = std::env::var("HOME").unwrap();
        let declared = vec![
            ("CLAUDE_CONFIG_DIR".to_string(), "/tmp/murmur-cfg".to_string()),
            ("HOME".to_string(), "/tmp/hijacked".to_string()),
        ];
        let environment = user_cli_environment(&declared);
        assert!(environment
            .iter()
            .any(|(key, value)| key == "CLAUDE_CONFIG_DIR" && value == "/tmp/murmur-cfg"));
        assert!(environment
            .iter()
            .any(|(key, value)| key == "HOME" && *value == inherited_home));
    }

    #[test]
    fn user_cli_stderr_is_captured_rather_than_discarded() {
        let arguments = vec!["-c".to_string(), "echo not logged in 1>&2".to_string()];
        let (mut child, stdin, stdout, mut stderr) =
            ManagedChild::spawn_user_cli(Path::new("/bin/sh"), &arguments, &[]).unwrap();
        drop((stdin, stdout));
        let mut captured = String::new();
        stderr.read_to_string(&mut captured).unwrap();
        child
            .wait_for_exit(Instant::now() + Duration::from_secs(2))
            .expect("sh must exit cleanly");
        assert_eq!(captured.trim(), "not logged in");
    }

    #[test]
    fn user_cli_hard_kill_confirms_descendant_process_group_is_empty() {
        let arguments = vec!["-c".to_string(), "sleep 30 & wait".to_string()];
        let (mut child, stdin, stdout, stderr) =
            ManagedChild::spawn_user_cli(Path::new("/bin/sh"), &arguments, &[]).unwrap();
        drop((stdin, stdout, stderr));
        let termination = child
            .hard_kill_confirmed(Instant::now() + Duration::from_secs(2))
            .expect("owned child and its descendant must be confirmed stopped");
        assert!(termination.process_group_empty);
    }
}
