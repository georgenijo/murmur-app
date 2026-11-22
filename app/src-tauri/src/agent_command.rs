//! Voice control of coding agents through the local T3 Code CLI.
//!
//! One spoken command becomes one T3 thread in a dedicated dispatch project. The
//! dispatcher agent there resolves the project, picks the machine, launches the
//! work and replies in a sentence or two. When it launched work, its reply ends
//! with a final `FOLLOW_UP: WAIT` line and a second, long-waiting `t3 thread send`
//! reports the outcome.
//!
//! Privacy: the transcript, the agent's reply and `t3` stderr never reach a log.
//! The reply is shown only in the user-facing notification. This module emits no
//! always-on log events (so it needs no event-code registry entry); failures
//! surface only as content-free notifications.
//!
//! The process runner is injectable so the whole flow is unit-tested without a
//! real `t3`.

use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NOTIFICATION_TITLE: &str = "Agent";
const FOLLOW_UP_MARKER: &str = "FOLLOW_UP: WAIT";
const FOLLOW_UP_PROMPT: &str =
    "Wait until the thread you launched finishes, then report its outcome in one or two sentences.";
const TITLE_PREFIX: &str = "Voice: ";
const TITLE_TRANSCRIPT_CHARS: usize = 60;
const NOTIFICATION_BODY_CHARS: usize = 240;
const START_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const FOLLOW_UP_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const STDOUT_LIMIT_BYTES: u64 = 1 << 20;
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const READER_GRACE: Duration = Duration::from_secs(5);

/// Exit code `t3 thread … --wait` uses when the agent is waiting for an answer.
const EXIT_AGENT_WAITING: i32 = 2;

#[derive(Clone, Debug)]
pub struct AgentCommandConfig {
    /// `None` resolves `$HOME/.local/bin/t3`, `/opt/homebrew/bin/t3`, `/usr/local/bin/t3`.
    pub t3_bin: Option<PathBuf>,
    /// Project id or absolute path of the dispatch project.
    pub dispatch_project: String,
    /// Provider for the dispatcher thread, e.g. `claude_cpa`.
    pub provider: String,
    /// Model for the dispatcher thread, e.g. `claude-sonnet-5-5`.
    pub model: String,
}

pub trait AgentNotifier: Send + Sync + 'static {
    fn notify(&self, title: &str, body: &str);
}

/// Runs a voice command on a background thread. Never blocks the caller.
#[allow(dead_code)] // The hotkey/routing change is the caller.
pub fn spawn_dispatch(
    transcript: String,
    config: AgentCommandConfig,
    notifier: Arc<dyn AgentNotifier>,
) {
    if transcript.trim().is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("agent-command".into())
        .spawn(move || run_dispatch(&transcript, &config, &SystemRunner, notifier.as_ref()));
    // A failed thread spawn has no notifier left to use (it moved into the closure);
    // the only cause is resource exhaustion, where the command is lost either way.
    drop(spawned);
}

// ---------------------------------------------------------------------------
// Process runner
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct RunOutput {
    /// Exit code; `-1` when the process ended by signal.
    exit_code: i32,
    stdout: String,
}

#[derive(Debug, PartialEq, Eq)]
enum RunError {
    /// The executable does not exist.
    NotFound,
    /// The executable exists but could not be started.
    Spawn,
    /// The deadline passed; the child was killed.
    Timeout,
}

trait ProcessRunner {
    /// Runs `bin` with `args` (no shell), feeds `stdin`, and captures stdout.
    /// Stderr is discarded. Kills the child when `timeout` passes.
    fn run(
        &self,
        bin: &Path,
        args: &[String],
        stdin: &str,
        timeout: Duration,
    ) -> Result<RunOutput, RunError>;
}

struct SystemRunner;

impl ProcessRunner for SystemRunner {
    fn run(
        &self,
        bin: &Path,
        args: &[String],
        stdin: &str,
        timeout: Duration,
    ) -> Result<RunOutput, RunError> {
        let home = home_dir();
        let mut command = Command::new(bin);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("PATH", launch_path(home.as_deref(), bin))
            .current_dir(home.as_deref().unwrap_or_else(|| Path::new("/")));
        if let Some(home) = &home {
            command.env("HOME", home);
        }
        #[cfg(unix)]
        {
            // Own process group so a timeout kills the whole tree (t3 may be a node wrapper).
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RunError::NotFound
            } else {
                RunError::Spawn
            }
        })?;
        let pid = child.id();

        if let Some(mut pipe) = child.stdin.take() {
            let input = stdin.to_owned();
            std::thread::spawn(move || {
                // A broken pipe means the child exited early; its exit code tells the story.
                let _ = pipe.write_all(input.as_bytes());
            });
        }

        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
        if let Some(mut pipe) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut captured = Vec::new();
                let _ = (&mut pipe)
                    .take(STDOUT_LIMIT_BYTES)
                    .read_to_end(&mut captured);
                // Keep draining so a chatty child is never blocked on a full pipe.
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
                let _ = stdout_tx.send(captured);
            });
        }

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    kill_group(pid);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunError::Timeout);
                }
                Ok(None) => std::thread::sleep(POLL_INTERVAL),
                Err(_) => {
                    kill_group(pid);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunError::Spawn);
                }
            }
        };

        // A descendant that outlives t3 may hold the pipe open; do not wait on it forever.
        let captured = stdout_rx.recv_timeout(READER_GRACE).unwrap_or_default();
        Ok(RunOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&captured).into_owned(),
        })
    }
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(pid) {
            // SAFETY: plain syscall; the pid is our own just-spawned process group leader.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .or_else(dirs::home_dir)
}

/// Launch Services starts apps with a minimal PATH; `t3` is usually a node script that
/// needs `node` from a user-level location.
fn launch_path(home: Option<&Path>, bin: &Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = bin.parent().filter(|parent| parent.is_absolute()) {
        dirs.push(parent.to_path_buf());
    }
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
    }
    for fixed in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        dirs.push(PathBuf::from(fixed));
    }
    if let Some(inherited) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&inherited));
    }
    let mut seen = Vec::new();
    dirs.retain(|dir| {
        if seen.contains(dir) {
            false
        } else {
            seen.push(dir.clone());
            true
        }
    });
    std::env::join_paths(dirs).unwrap_or_default()
}

fn t3_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = home {
        candidates.push(home.join(".local/bin/t3"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/t3"));
    candidates.push(PathBuf::from("/usr/local/bin/t3"));
    candidates
}

/// An explicit path is trusted as is (the runner reports a missing file); otherwise the
/// first candidate that exists wins.
fn resolve_t3(
    configured: Option<&Path>,
    home: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    match configured {
        Some(path) => Some(path.to_path_buf()),
        None => t3_candidates(home).into_iter().find(|path| exists(path)),
    }
}

// ---------------------------------------------------------------------------
// Output parsing
// ---------------------------------------------------------------------------

/// What `t3 thread start|send --json --wait` prints: one JSON line,
/// `{"threadId","runId","status","reply","error"}`.
#[derive(Debug, PartialEq, Eq)]
struct T3Reply {
    thread_id: Option<String>,
    reply: String,
    /// `status` was present and not `completed`, or `error` was non-null.
    failed: bool,
}

fn parse_t3_output(stdout: &str) -> Option<T3Reply> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok().or_else(|| {
        // Tolerate a stray line ahead of the JSON.
        stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with('{'))
            .and_then(|line| serde_json::from_str(line).ok())
    })?;
    let object = value.as_object()?;
    let text = |key: &str| object.get(key).and_then(|v| v.as_str());
    let status_failed = text("status").is_some_and(|status| status != "completed");
    let error_present = object.get("error").is_some_and(|error| !error.is_null());
    Some(T3Reply {
        thread_id: text("threadId")
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        reply: text("reply").unwrap_or_default().trim().to_owned(),
        failed: status_failed || error_present,
    })
}

// ---------------------------------------------------------------------------
// Reply text
// ---------------------------------------------------------------------------

fn is_follow_up_marker(line: &str) -> bool {
    line.trim() == FOLLOW_UP_MARKER
}

/// The final non-empty line is exactly the marker.
fn wants_follow_up(reply: &str) -> bool {
    reply
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(is_follow_up_marker)
}

/// The reply with its trailing marker line (and surrounding blank lines) removed.
fn strip_follow_up(reply: &str) -> String {
    let mut lines: Vec<&str> = reply.lines().collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.last().is_some_and(|line| is_follow_up_marker(line)) {
        lines.pop();
    }
    lines.join("\n").trim().to_owned()
}

fn cap_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut capped: String = text.chars().take(limit.saturating_sub(1)).collect();
    capped.push('…');
    capped
}

fn thread_title(transcript: &str) -> String {
    let flat = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title = format!(
        "{TITLE_PREFIX}{}",
        flat.chars()
            .take(TITLE_TRANSCRIPT_CHARS)
            .collect::<String>()
    );
    if flat.chars().count() > TITLE_TRANSCRIPT_CHARS {
        title.push('…');
    }
    title
}

// ---------------------------------------------------------------------------
// Flow
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    NotInstalled,
    CouldNotStart,
    TimedOut,
    Errored,
    BadOutput,
}

impl Failure {
    fn message(&self) -> &'static str {
        match self {
            Failure::NotInstalled => "Agent command failed: T3 isn't installed.",
            Failure::CouldNotStart => "Agent command failed: T3 couldn't be started.",
            Failure::TimedOut => "Agent command failed: T3 took too long.",
            Failure::Errored => "Agent command failed: T3 reported an error.",
            Failure::BadOutput => "Agent command failed: T3 sent an unexpected reply.",
        }
    }
}

enum Outcome {
    Completed {
        thread_id: String,
        reply: String,
    },
    /// Exit 2: the agent is waiting for an answer; `reply` is its question.
    Question {
        reply: String,
    },
}

fn call_t3(
    runner: &dyn ProcessRunner,
    bin: &Path,
    args: &[String],
    stdin: &str,
    timeout: Duration,
) -> Result<Outcome, Failure> {
    let output = runner
        .run(bin, args, stdin, timeout)
        .map_err(|error| match error {
            RunError::NotFound => Failure::NotInstalled,
            RunError::Spawn => Failure::CouldNotStart,
            RunError::Timeout => Failure::TimedOut,
        })?;
    match output.exit_code {
        0 => {
            let parsed = parse_t3_output(&output.stdout).ok_or(Failure::BadOutput)?;
            if parsed.failed {
                return Err(Failure::Errored);
            }
            let thread_id = parsed.thread_id.ok_or(Failure::BadOutput)?;
            Ok(Outcome::Completed {
                thread_id,
                reply: parsed.reply,
            })
        }
        EXIT_AGENT_WAITING => Ok(Outcome::Question {
            reply: parse_t3_output(&output.stdout)
                .map(|parsed| parsed.reply)
                .unwrap_or_default(),
        }),
        _ => Err(Failure::Errored),
    }
}

fn notify_reply(notifier: &dyn AgentNotifier, reply: &str, empty_fallback: &str) {
    let body = strip_follow_up(reply);
    let body = if body.is_empty() {
        empty_fallback
    } else {
        &body
    };
    notifier.notify(
        NOTIFICATION_TITLE,
        &cap_chars(body, NOTIFICATION_BODY_CHARS),
    );
}

fn notify_question(notifier: &dyn AgentNotifier, reply: &str) {
    notify_reply(notifier, reply, "The agent needs an answer in T3.");
}

fn start_args(config: &AgentCommandConfig, title: &str) -> Vec<String> {
    [
        "thread",
        "start",
        "--json",
        "--wait",
        "--provider",
        &config.provider,
        "--model",
        &config.model,
        "--title",
        title,
        &config.dispatch_project,
        "-",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn send_args(thread_id: &str) -> Vec<String> {
    ["thread", "send", "--json", "--wait", thread_id, "-"]
        .map(str::to_owned)
        .to_vec()
}

fn run_dispatch(
    transcript: &str,
    config: &AgentCommandConfig,
    runner: &dyn ProcessRunner,
    notifier: &dyn AgentNotifier,
) {
    let transcript = transcript.trim();
    if transcript.is_empty() {
        return;
    }
    let home = home_dir();
    let Some(bin) = resolve_t3(config.t3_bin.as_deref(), home.as_deref(), |path| {
        path.is_file()
    }) else {
        notifier.notify(NOTIFICATION_TITLE, Failure::NotInstalled.message());
        return;
    };

    let started = call_t3(
        runner,
        &bin,
        &start_args(config, &thread_title(transcript)),
        transcript,
        START_TIMEOUT,
    );
    let (thread_id, reply) = match started {
        Ok(Outcome::Completed { thread_id, reply }) => (thread_id, reply),
        Ok(Outcome::Question { reply }) => return notify_question(notifier, &reply),
        Err(failure) => return notifier.notify(NOTIFICATION_TITLE, failure.message()),
    };
    notify_reply(notifier, &reply, "Done.");
    if !wants_follow_up(&reply) {
        return;
    }

    match call_t3(
        runner,
        &bin,
        &send_args(&thread_id),
        FOLLOW_UP_PROMPT,
        FOLLOW_UP_TIMEOUT,
    ) {
        Ok(Outcome::Completed { reply, .. }) => {
            notify_reply(notifier, &reply, "The thread finished.")
        }
        Ok(Outcome::Question { reply }) => notify_question(notifier, &reply),
        Err(failure) => notifier.notify(NOTIFICATION_TITLE, failure.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Call {
        bin: PathBuf,
        args: Vec<String>,
        stdin: String,
        timeout: Duration,
    }

    struct FakeRunner {
        calls: Mutex<Vec<Call>>,
        responses: Mutex<VecDeque<Result<RunOutput, RunError>>>,
    }

    impl FakeRunner {
        fn new(responses: Vec<Result<RunOutput, RunError>>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            }
        }
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            bin: &Path,
            args: &[String],
            stdin: &str,
            timeout: Duration,
        ) -> Result<RunOutput, RunError> {
            self.calls.lock().unwrap().push(Call {
                bin: bin.to_path_buf(),
                args: args.to_vec(),
                stdin: stdin.to_owned(),
                timeout,
            });
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra t3 call")
        }
    }

    #[derive(Default)]
    struct RecordingNotifier(Mutex<Vec<(String, String)>>);

    impl AgentNotifier for RecordingNotifier {
        fn notify(&self, title: &str, body: &str) {
            self.0
                .lock()
                .unwrap()
                .push((title.to_owned(), body.to_owned()));
        }
    }

    impl RecordingNotifier {
        fn bodies(&self) -> Vec<String> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .map(|(_, body)| body.clone())
                .collect()
        }
    }

    fn config() -> AgentCommandConfig {
        AgentCommandConfig {
            t3_bin: Some(PathBuf::from("/fake/t3")),
            dispatch_project: "/work/agent-dispatch".into(),
            provider: "claude_cpa".into(),
            model: "claude-sonnet-5-5".into(),
        }
    }

    fn json(reply: &str) -> String {
        serde_json::json!({
            "threadId": "mcp:abc",
            "runId": "run:1",
            "status": "completed",
            "reply": reply,
            "error": null,
        })
        .to_string()
    }

    fn ok(stdout: String) -> Result<RunOutput, RunError> {
        Ok(RunOutput {
            exit_code: 0,
            stdout,
        })
    }

    fn run(
        responses: Vec<Result<RunOutput, RunError>>,
        transcript: &str,
    ) -> (FakeRunner, RecordingNotifier) {
        let runner = FakeRunner::new(responses);
        let notifier = RecordingNotifier::default();
        run_dispatch(transcript, &config(), &runner, &notifier);
        (runner, notifier)
    }

    #[test]
    fn success_notifies_reply_and_passes_exact_arguments() {
        let (runner, notifier) = run(
            vec![ok(json("Looked it up: two threads running."))],
            "  what's running?  ",
        );
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].bin, PathBuf::from("/fake/t3"));
        assert_eq!(
            calls[0].args,
            [
                "thread",
                "start",
                "--json",
                "--wait",
                "--provider",
                "claude_cpa",
                "--model",
                "claude-sonnet-5-5",
                "--title",
                "Voice: what's running?",
                "/work/agent-dispatch",
                "-",
            ]
        );
        assert_eq!(calls[0].stdin, "what's running?");
        assert_eq!(calls[0].timeout, START_TIMEOUT);
        assert_eq!(notifier.bodies(), ["Looked it up: two threads running."]);
        assert_eq!(notifier.0.lock().unwrap()[0].0, "Agent");
    }

    #[test]
    fn follow_up_marker_triggers_wait_and_second_notification() {
        let (runner, notifier) = run(
            vec![
                ok(json("Started on Volta: fix the map fade.\nFOLLOW_UP: WAIT")),
                ok(json("The map fade fix finished and tests pass.")),
            ],
            "launch a thread on Volta to fix the map fade",
        );
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1].args,
            ["thread", "send", "--json", "--wait", "mcp:abc", "-"]
        );
        assert_eq!(calls[1].stdin, FOLLOW_UP_PROMPT);
        assert_eq!(calls[1].timeout, FOLLOW_UP_TIMEOUT);
        assert_eq!(
            notifier.bodies(),
            [
                "Started on Volta: fix the map fade.",
                "The map fade fix finished and tests pass."
            ]
        );
    }

    #[test]
    fn marker_must_be_the_last_non_empty_line() {
        assert!(wants_follow_up("ok\nFOLLOW_UP: WAIT\n\n"));
        assert!(wants_follow_up("ok\r\nFOLLOW_UP: WAIT\r\n"));
        assert!(!wants_follow_up("FOLLOW_UP: WAIT\nactually, no"));
        assert!(!wants_follow_up("I will FOLLOW_UP: WAIT"));
        assert!(!wants_follow_up(""));
        let (runner, notifier) = run(vec![ok(json("Nothing launched."))], "hello");
        assert_eq!(runner.calls().len(), 1);
        assert_eq!(notifier.bodies().len(), 1);
    }

    #[test]
    fn exit_two_shows_reply_as_question_without_follow_up() {
        let (runner, notifier) = run(
            vec![Ok(RunOutput {
                exit_code: 2,
                stdout: json("Which machine should I use?\nFOLLOW_UP: WAIT"),
            })],
            "fix the fade",
        );
        assert_eq!(runner.calls().len(), 1);
        assert_eq!(notifier.bodies(), ["Which machine should I use?"]);
    }

    #[test]
    fn exit_two_with_unparseable_output_uses_generic_question() {
        let (_, notifier) = run(
            vec![Ok(RunOutput {
                exit_code: 2,
                stdout: "not json".into(),
            })],
            "fix the fade",
        );
        assert_eq!(notifier.bodies(), ["The agent needs an answer in T3."]);
    }

    #[test]
    fn timeout_notifies_content_free_failure() {
        let (runner, notifier) = run(vec![Err(RunError::Timeout)], "fix the fade");
        assert_eq!(runner.calls().len(), 1);
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 took too long."]
        );
    }

    #[test]
    fn follow_up_timeout_keeps_the_first_notification() {
        let (_, notifier) = run(
            vec![
                ok(json("Started.\nFOLLOW_UP: WAIT")),
                Err(RunError::Timeout),
            ],
            "go",
        );
        assert_eq!(
            notifier.bodies(),
            ["Started.", "Agent command failed: T3 took too long."]
        );
    }

    #[test]
    fn missing_binary_notifies_not_installed() {
        let (_, notifier) = run(vec![Err(RunError::NotFound)], "fix the fade");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 isn't installed."]
        );
        let (_, notifier) = run(vec![Err(RunError::Spawn)], "fix the fade");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 couldn't be started."]
        );
    }

    #[test]
    fn resolver_returns_none_when_nothing_exists() {
        // No configured path and (on this test host) possibly no candidate: only assert
        // the pure resolver, which is what decides it.
        assert_eq!(resolve_t3(None, Some(Path::new("/h")), |_| false), None);
    }

    #[test]
    fn empty_transcript_does_nothing() {
        let (runner, notifier) = run(vec![], "  \n\t ");
        assert!(runner.calls().is_empty());
        assert!(notifier.bodies().is_empty());
    }

    #[test]
    fn spawn_dispatch_returns_immediately_for_empty_transcript() {
        struct Panics;
        impl AgentNotifier for Panics {
            fn notify(&self, _: &str, _: &str) {
                panic!("must not notify");
            }
        }
        spawn_dispatch(" ".into(), config(), Arc::new(Panics));
    }

    #[test]
    fn nonzero_exit_failed_status_and_bad_output_are_failures() {
        let (_, notifier) = run(
            vec![Ok(RunOutput {
                exit_code: 1,
                stdout: json("secret reply"),
            })],
            "x",
        );
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 reported an error."]
        );

        let failed_status =
            serde_json::json!({"threadId":"t","status":"failed","reply":"secret","error":null})
                .to_string();
        let (_, notifier) = run(vec![ok(failed_status)], "x");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 reported an error."]
        );

        let with_error = serde_json::json!({"threadId":"t","status":"completed","reply":"secret","error":"boom"}).to_string();
        let (_, notifier) = run(vec![ok(with_error)], "x");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 reported an error."]
        );

        let (_, notifier) = run(vec![ok("garbage".into())], "x");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 sent an unexpected reply."]
        );

        let no_thread = serde_json::json!({"status":"completed","reply":"hi"}).to_string();
        let (_, notifier) = run(vec![ok(no_thread)], "x");
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 sent an unexpected reply."]
        );
    }

    #[test]
    fn real_t3_rejection_is_exit_one_with_empty_stdout() {
        // Observed from the real CLI for an unknown provider or model: exit 1, nothing on
        // stdout, the reason on stderr (which this module discards).
        let (runner, notifier) = run(
            vec![Ok(RunOutput {
                exit_code: 1,
                stdout: String::new(),
            })],
            "fix the fade",
        );
        assert_eq!(runner.calls().len(), 1);
        assert_eq!(
            notifier.bodies(),
            ["Agent command failed: T3 reported an error."]
        );
    }

    #[test]
    fn parser_reads_real_shape_and_tolerates_leading_noise() {
        let real = r#"{"threadId":"mcp:006ba3e9","runId":"run:thread:1","status":"completed","reply":"Started.\nFOLLOW_UP: WAIT","error":null}"#;
        assert_eq!(
            parse_t3_output(real),
            Some(T3Reply {
                thread_id: Some("mcp:006ba3e9".into()),
                reply: "Started.\nFOLLOW_UP: WAIT".into(),
                failed: false,
            })
        );
        let noisy = format!("warning: update available\n{real}\n");
        assert_eq!(parse_t3_output(&noisy), parse_t3_output(real));
        assert_eq!(parse_t3_output("[1,2]"), None);
        assert_eq!(parse_t3_output(""), None);
    }

    #[test]
    fn title_and_body_caps_are_char_boundary_safe() {
        let transcript = "é".repeat(100);
        let title = thread_title(&transcript);
        assert_eq!(title.chars().count(), TITLE_PREFIX.chars().count() + 60 + 1);
        assert!(title.starts_with("Voice: éé") && title.ends_with('…'));
        assert_eq!(thread_title("  short \n one "), "Voice: short one");

        let capped = cap_chars(&"🙂".repeat(300), NOTIFICATION_BODY_CHARS);
        assert_eq!(capped.chars().count(), NOTIFICATION_BODY_CHARS);
        assert!(capped.ends_with('…'));
        assert_eq!(cap_chars("short", NOTIFICATION_BODY_CHARS), "short");
    }

    #[test]
    fn notification_body_is_capped_and_marker_stripped() {
        let long = format!("{}\nFOLLOW_UP: WAIT", "word ".repeat(100));
        let (_, notifier) = run(vec![ok(json(&long)), ok(json("done"))], "go");
        let first = &notifier.bodies()[0];
        assert!(first.chars().count() <= NOTIFICATION_BODY_CHARS);
        assert!(!first.contains("FOLLOW_UP"));
        assert_eq!(strip_follow_up("a\n\nFOLLOW_UP: WAIT\n"), "a");
        assert_eq!(strip_follow_up("FOLLOW_UP: WAIT"), "");
    }

    #[test]
    fn resolver_prefers_configured_then_first_existing_candidate() {
        let configured = Path::new("/custom/t3");
        assert_eq!(
            resolve_t3(Some(configured), Some(Path::new("/h")), |_| false),
            Some(configured.to_path_buf())
        );
        let home = Path::new("/Users/x");
        assert_eq!(
            resolve_t3(None, Some(home), |p| p == Path::new("/opt/homebrew/bin/t3")),
            Some(PathBuf::from("/opt/homebrew/bin/t3"))
        );
        assert_eq!(
            resolve_t3(None, Some(home), |_| true),
            Some(PathBuf::from("/Users/x/.local/bin/t3"))
        );
    }

    #[test]
    fn launch_path_includes_user_level_dirs_without_duplicates() {
        let path = launch_path(
            Some(Path::new("/Users/x")),
            Path::new("/Users/x/.local/bin/t3"),
        );
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[0], PathBuf::from("/Users/x/.local/bin"));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert_eq!(
            dirs.iter().filter(|d| **d == Path::new("/usr/bin")).count(),
            1
        );
    }

    /// Live check against the real `t3` CLI and the running T3 server. Not part of CI.
    ///
    /// Env: `MURMUR_E2E_TRANSCRIPT` (default "what's running?"), `MURMUR_E2E_PROJECT`
    /// (default the dispatch project), `MURMUR_E2E_WAIT_SECS` (default 900).
    /// Run: `cargo test --lib real_t3_round_trip -- --ignored --nocapture --test-threads=1`
    #[test]
    #[ignore = "needs a running T3 server and the real t3 CLI"]
    fn real_t3_round_trip() {
        let transcript =
            std::env::var("MURMUR_E2E_TRANSCRIPT").unwrap_or_else(|_| "what's running?".to_owned());
        let project = std::env::var("MURMUR_E2E_PROJECT")
            .unwrap_or_else(|_| "/Users/george-mac-mini/Documents/code/agent-dispatch".to_owned());
        let wait = Duration::from_secs(
            std::env::var("MURMUR_E2E_WAIT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(900),
        );
        let config = AgentCommandConfig {
            t3_bin: None,
            dispatch_project: project,
            provider: "claude_cpa".into(),
            model: "claude-sonnet-5-5".into(),
        };

        struct PrintingNotifier {
            seen: Arc<Mutex<Vec<(String, String)>>>,
            started: Instant,
        }
        impl AgentNotifier for PrintingNotifier {
            fn notify(&self, title: &str, body: &str) {
                println!(
                    "[{:>6.1}s] NOTIFY title={title:?} body={body:?}",
                    self.started.elapsed().as_secs_f32()
                );
                self.seen
                    .lock()
                    .unwrap()
                    .push((title.to_owned(), body.to_owned()));
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let started = Instant::now();
        println!(
            "HOME={:?} PATH={:?} transcript={transcript:?}",
            std::env::var("HOME").ok(),
            std::env::var("PATH").ok()
        );
        spawn_dispatch(
            transcript,
            config,
            Arc::new(PrintingNotifier {
                seen: Arc::clone(&seen),
                started,
            }),
        );

        // The dispatcher thread is detached, so wait on the recorded notifications:
        // until the expected count arrives, then a short grace period for extras.
        let expected = std::env::var("MURMUR_E2E_EXPECT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let deadline = Instant::now() + wait;
        while seen.lock().unwrap().len() < expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::thread::sleep(Duration::from_secs(5));
        assert!(
            seen.lock().unwrap().len() >= expected,
            "expected {expected} notification(s) within {wait:?}"
        );
        println!(
            "TOTAL {:.1}s, {} notification(s)",
            started.elapsed().as_secs_f32(),
            seen.lock().unwrap().len()
        );
    }

    #[cfg(unix)]
    mod system_runner {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        fn script(dir: &Path, body: &str) -> PathBuf {
            let path = dir.join("t3");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        #[test]
        fn feeds_stdin_and_captures_stdout_and_exit_code() {
            let dir = tempfile::tempdir().unwrap();
            let bin = script(dir.path(), "cat; echo \" args:$#\"; exit 2");
            let out = SystemRunner
                .run(
                    &bin,
                    &["a".into(), "b".into()],
                    "hello",
                    Duration::from_secs(10),
                )
                .unwrap();
            assert_eq!(out.exit_code, 2);
            assert_eq!(out.stdout, "hello args:2\n");
        }

        #[test]
        fn timeout_kills_the_child() {
            let dir = tempfile::tempdir().unwrap();
            let bin = script(dir.path(), "sleep 30");
            let started = Instant::now();
            let result = SystemRunner.run(&bin, &[], "", Duration::from_millis(200));
            assert_eq!(result, Err(RunError::Timeout));
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        #[test]
        fn missing_executable_is_not_found() {
            let result = SystemRunner.run(
                Path::new("/definitely/not/here/t3"),
                &[],
                "",
                Duration::from_secs(1),
            );
            assert_eq!(result, Err(RunError::NotFound));
        }
    }
}
