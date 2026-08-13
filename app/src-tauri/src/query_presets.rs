//! Provider presets, auth preflight, and vendor login launch for Voice Query (#550).
//!
//! The bridge in `query_flow` stays deliberately generic: one absolute
//! executable, fixed argv, no shell. This module is the data around it. A
//! preset is a static record — where the binary usually lives, which arguments
//! that provider needs for one-shot printing, how to ask it whether the user is
//! signed in, what its "you are not signed in" output looks like, and which
//! environment names it actually reads. Nothing here can spawn anything the
//! generic path could not already spawn.
//!
//! Everything a probe prints stays local. Auth output routinely contains an
//! account email and organisation, so it is returned to the requesting Settings
//! window and never written to telemetry, history, or the event log.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::managed_child::ManagedChild;

const MAIN_WINDOW_LABEL: &str = "main";
const REVIEW_WINDOW_LABEL: &str = "query-review";
/// Enough to show a real auth report or stack trace, small enough that no
/// bounded-output policy is doing guesswork.
pub(crate) const MAX_PROBE_OUTPUT_BYTES: usize = 8 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATION_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct QueryPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    /// Binary name looked for under the standard install prefixes.
    pub binary_name: &'static str,
    /// Home-relative fallbacks probed after the standard prefixes, for
    /// providers that ship their own private install location.
    pub home_relative_paths: &'static [&'static str],
    /// Arguments that put the provider in one-shot, print-the-answer mode.
    pub recommended_arguments: &'static [&'static str],
    /// argv for the auth probe, run through the identical spawn path.
    pub auth_probe_arguments: &'static [&'static str],
    /// argv for the provider's own interactive login, launched in Terminal.
    pub login_arguments: &'static [&'static str],
    /// Lowercase substrings that prove the provider is *not* authenticated.
    pub auth_failure_signatures: &'static [&'static str],
    /// Lowercase substrings that prove the provider *is* authenticated.
    ///
    /// A probe that merely exits 0 proves nothing — an unrecognised
    /// subcommand, a stubbed binary, or a changed CLI all exit 0 with output
    /// this code has never seen. Settings promises that a green check means the
    /// real query will work, so a green check requires a positive signal.
    pub auth_success_signatures: &'static [&'static str],
    /// Environment names this provider actually reads, offered in Settings.
    pub suggested_env_keys: &'static [&'static str],
    /// The exact thing to type in a terminal, shown verbatim in the error.
    pub login_hint: &'static str,
}

/// Signatures every provider shares. A CLI that has lost its credentials tends
/// to say one of these regardless of vendor, and a custom executable gets the
/// same actionable error as a preset one.
const GENERIC_AUTH_FAILURE_SIGNATURES: &[&str] = &[
    "not logged in",
    "not authenticated",
    "please log in",
    "please login",
    "please sign in",
    "authentication_error",
    "authentication failed",
    "invalid api key",
    "invalid_api_key",
    // Deliberately not a bare "unauthorized": remapping fires on failures whose
    // output Murmur did not produce, and a permission or quota error that
    // happens to contain the word is not a sign-in problem.
    "401 unauthorized",
    "http 401",
    "status 401",
    "credentials not found",
    "no credentials",
    "session expired",
    "token expired",
];

pub(crate) const PRESETS: &[QueryPreset] = &[
    QueryPreset {
        id: "claude",
        label: "Claude Code",
        summary: "Anthropic's CLI. `-p` prints one answer and exits.",
        binary_name: "claude",
        home_relative_paths: &[".claude/local/claude"],
        recommended_arguments: &["-p"],
        auth_probe_arguments: &["auth", "status"],
        login_arguments: &["auth", "login"],
        // `claude auth status` reports `"loggedIn": false` and still exits 0,
        // so the signature — not the exit code — is what decides.
        auth_failure_signatures: &["\"loggedin\": false", "\"loggedin\":false", "run /login"],
        auth_success_signatures: &["\"loggedin\": true", "\"loggedin\":true"],
        suggested_env_keys: &["CLAUDE_CONFIG_DIR"],
        login_hint: "claude auth login",
    },
    QueryPreset {
        id: "codex",
        label: "Codex",
        summary: "OpenAI's CLI. `exec` runs one non-interactive task.",
        binary_name: "codex",
        home_relative_paths: &[".codex/bin/codex"],
        recommended_arguments: &["exec"],
        auth_probe_arguments: &["login", "status"],
        login_arguments: &["login"],
        // Deliberately narrower than a bare "codex login": a signed-in CLI can
        // still mention its own login command (e.g. to switch accounts), and a
        // false "not signed in" is worse than falling back to the generic set.
        auth_failure_signatures: &["run `codex login`", "please run codex login"],
        auth_success_signatures: &["logged in using", "logged in with"],
        suggested_env_keys: &["CODEX_HOME"],
        login_hint: "codex login",
    },
    QueryPreset {
        id: "grok",
        label: "Grok",
        summary: "xAI's CLI. `-p` answers a single prompt and exits.",
        binary_name: "grok",
        home_relative_paths: &[".grok/bin/grok"],
        recommended_arguments: &["-p"],
        // Grok has no auth subcommand; listing models is the cheapest call
        // that only succeeds with working credentials.
        auth_probe_arguments: &["models"],
        login_arguments: &["login"],
        auth_failure_signatures: &["run `grok login`", "sign in to grok"],
        auth_success_signatures: &["you are logged in", "available models"],
        suggested_env_keys: &["GROK_HOME"],
        login_hint: "grok login",
    },
    QueryPreset {
        id: "cursor",
        label: "Cursor Agent",
        summary: "Cursor's CLI. `-p` prints one response for scripts.",
        binary_name: "cursor-agent",
        home_relative_paths: &[".local/bin/cursor-agent", ".cursor/bin/cursor-agent"],
        recommended_arguments: &["-p"],
        auth_probe_arguments: &["status"],
        login_arguments: &["login"],
        auth_failure_signatures: &["run `cursor-agent login`", "not signed in"],
        auth_success_signatures: &["logged in as", "signed in as"],
        suggested_env_keys: &[],
        login_hint: "cursor-agent login",
    },
];

/// Standard absolute install prefixes, probed in order.
const BINARY_PREFIXES: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/homebrew/opt/node/bin",
    "/usr/bin",
];

/// Home-relative prefixes for per-user installs (npm, bun, volta, pipx, …).
const HOME_BINARY_PREFIXES: &[&str] = &[
    ".local/bin",
    "bin",
    ".bun/bin",
    ".npm-global/bin",
    ".volta/bin",
    ".yarn/bin",
];

pub(crate) fn preset(id: &str) -> Option<&'static QueryPreset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// First existing executable for this preset, or `None` when the provider is
/// not installed anywhere Murmur knows to look.
///
/// The host process environment is searched too — a Murmur launched from a
/// shell inherits that shell's `PATH` — but the fixed prefixes are what make
/// discovery work for the normal Finder/Dock launch, where `PATH` is the bare
/// system default.
pub(crate) fn discover(preset: &QueryPreset) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = BINARY_PREFIXES
        .iter()
        .map(|prefix| Path::new(prefix).join(preset.binary_name))
        .collect();
    if let Some(home) = home.as_ref() {
        for prefix in HOME_BINARY_PREFIXES {
            candidates.push(home.join(prefix).join(preset.binary_name));
        }
        for relative in preset.home_relative_paths {
            candidates.push(home.join(relative));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|entry| entry.join(preset.binary_name)));
    }
    candidates
        .into_iter()
        .filter(|candidate| candidate.is_absolute())
        .find(|candidate| is_executable_file(candidate))
        .and_then(|candidate| std::fs::canonicalize(candidate).ok())
}

/// True when `output` carries proof that the provider is not signed in.
///
/// Matching is substring-on-lowercase against the preset's own phrasing plus a
/// shared generic set, so a custom executable still gets the actionable
/// "sign in" error instead of a bare non-zero exit.
pub(crate) fn indicates_auth_failure(preset: Option<&QueryPreset>, output: &str) -> bool {
    if output.is_empty() {
        return false;
    }
    let haystack = output.to_lowercase();
    preset
        .map(|preset| preset.auth_failure_signatures)
        .unwrap_or(&[])
        .iter()
        .chain(GENERIC_AUTH_FAILURE_SIGNATURES.iter())
        .any(|signature| haystack.contains(signature))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthVerdict {
    Authenticated,
    NotAuthenticated,
    /// The probe ran but its result is not conclusive. The raw output is shown
    /// rather than guessed at — an unknown verdict must never read as a
    /// confident "you are signed in".
    Unknown,
}

/// Decide a verdict from what the probe actually produced.
///
/// Failure signatures decide first (a signed-out `claude auth status` still
/// exits 0). A clean exit alone is then *not* enough for a preset that declares
/// what success looks like: without that positive signal the result is
/// `Unknown`, which shows the raw output instead of a green check the run has
/// not earned.
pub(crate) fn verdict_for(
    preset: Option<&QueryPreset>,
    exit_code: Option<i32>,
    output: &str,
) -> AuthVerdict {
    if indicates_auth_failure(preset, output) {
        return AuthVerdict::NotAuthenticated;
    }
    if exit_code != Some(0) {
        return AuthVerdict::Unknown;
    }
    let expectations = preset
        .map(|preset| preset.auth_success_signatures)
        .unwrap_or(&[]);
    if expectations.is_empty() {
        return AuthVerdict::Authenticated;
    }
    let haystack = output.to_lowercase();
    if expectations
        .iter()
        .any(|signature| haystack.contains(signature))
    {
        AuthVerdict::Authenticated
    } else {
        AuthVerdict::Unknown
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryPresetInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub binary_name: &'static str,
    pub recommended_arguments: Vec<String>,
    pub suggested_env_keys: Vec<String>,
    pub login_hint: &'static str,
    /// Absolute path Murmur found for this provider, if it is installed.
    pub discovered_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryAuthProbeReport {
    pub verdict: AuthVerdict,
    pub exit_code: Option<i32>,
    /// Bounded, local-only merged stdout+stderr. Settings shows it verbatim.
    pub output: String,
    pub truncated: bool,
    pub duration_ms: u64,
    pub login_hint: Option<&'static str>,
}

fn read_bounded(mut source: impl std::io::Read) -> (Vec<u8>, bool) {
    let mut collected = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut truncated = false;
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let room = MAX_PROBE_OUTPUT_BYTES.saturating_sub(collected.len());
                if room == 0 {
                    truncated = true;
                    // Keep draining so the child is never blocked on a full pipe.
                    continue;
                }
                let taken = count.min(room);
                truncated |= taken < count;
                collected.extend_from_slice(&buffer[..taken]);
            }
        }
    }
    (collected, truncated)
}

/// Run one preset's auth probe through the identical `spawn_user_cli` path the
/// query itself uses — same cleared environment, same declared pairs, same
/// process-group ownership — so a probe that passes proves the real thing will.
fn run_probe(
    executable: &Path,
    arguments: &[String],
    declared_environment: &[(String, String)],
) -> Result<(Option<i32>, String, bool), String> {
    let (mut child, stdin, stdout, stderr) =
        ManagedChild::spawn_user_cli(executable, arguments, declared_environment)
            .map_err(|_| "The configured CLI could not be started.".to_string())?;
    drop(stdin);

    let stdout_reader = std::thread::spawn(move || read_bounded(stdout));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr));

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("The provider check timed out and was stopped.".to_string());
            }
            Ok(None) => std::thread::sleep(PROBE_POLL_INTERVAL),
            Err(_) => {
                let _ = child.hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("The provider check could not be completed.".to_string());
            }
        }
    };
    // A wrapper CLI can exit while a descendant still inherits its stdout and
    // stderr. Confirm the whole owned process group is gone — killing it if it
    // is not — *before* joining the readers, exactly as the query path does:
    // joining first would block this probe on that leaked pipe forever, with no
    // timeout left to rescue it.
    if child
        .wait_for_exit(Instant::now() + TERMINATION_DEADLINE)
        .is_none()
    {
        let _ = child.hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
    }
    let (out_bytes, out_truncated) = stdout_reader.join().unwrap_or_default();
    let (err_bytes, err_truncated) = stderr_reader.join().unwrap_or_default();

    let mut output = String::from_utf8_lossy(&out_bytes).into_owned();
    let stderr_text = String::from_utf8_lossy(&err_bytes);
    if !stderr_text.trim().is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr_text);
    }
    Ok((
        exit_code,
        output.trim().to_string(),
        out_truncated || err_truncated,
    ))
}

/// Escape a string for embedding in an AppleScript string literal.
fn applescript_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Quote one argument for a POSIX shell using single quotes.
///
/// `do script` hands Terminal a command line, so this is the one place in the
/// voice-query path where a string is built for a shell. Only the validated
/// executable path and the preset's own static argv ever reach it, and both are
/// quoted here rather than trusted.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn login_command_line(executable: &Path, arguments: &[&str]) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| "That executable path cannot be opened in Terminal.".to_string())?;
    if executable.chars().any(|character| character.is_control()) {
        return Err("That executable path cannot be opened in Terminal.".to_string());
    }
    let mut line = shell_quote(executable);
    for argument in arguments {
        line.push(' ');
        line.push_str(&shell_quote(argument));
    }
    Ok(line)
}

/// Launch the provider's own login flow in Terminal.
///
/// Tier 1 of the sign-in design: Murmur never handles the credential, prompts
/// for it, or proxies it. It opens the vendor's interactive login where the
/// user can see exactly what they are approving, and the caller re-probes
/// afterwards to confirm.
#[cfg(target_os = "macos")]
fn launch_login_in_terminal(executable: &Path, arguments: &[&str]) -> Result<(), String> {
    let command_line = login_command_line(executable, arguments)?;
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        applescript_literal(&command_line)
    );
    let output = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|_| "Murmur could not open Terminal.".to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err("Murmur could not open Terminal.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn launch_login_in_terminal(executable: &Path, arguments: &[&str]) -> Result<(), String> {
    let _ = login_command_line(executable, arguments)?;
    Err("Opening a provider login is only supported on macOS.".to_string())
}

/// True when something latency- or process-sensitive is already running.
///
/// Mirrors the gate `start_query_capture` applies, minus the checks that only
/// make sense for capture: the probe never touches the microphone, but it does
/// start a provider CLI that can be a heavy inference runtime.
fn is_pipeline_busy(state: &crate::State) -> bool {
    use crate::MutexExt;
    use std::sync::atomic::Ordering;

    #[cfg(feature = "internal-benchmark")]
    let corpus_busy = state.corpus.is_active();
    #[cfg(not(feature = "internal-benchmark"))]
    let corpus_busy = false;

    state.query.status().blocks_pipeline()
        || state.app_state.dictation.lock_or_recover().status != crate::state::DictationStatus::Idle
        || state.app_state.file_transcribing.load(Ordering::SeqCst)
        || state.benchmark.is_running()
        || state.app_state.transform_status().blocks_recording()
        || state.transform_runtime.is_transform_busy()
        || corpus_busy
}

fn require_configuration_window(label: &str) -> Result<(), String> {
    if label == MAIN_WINDOW_LABEL || label == REVIEW_WINDOW_LABEL {
        Ok(())
    } else {
        Err("Voice Query provider actions are not available from this window.".to_string())
    }
}

#[tauri::command]
pub(crate) fn list_query_presets(
    window: tauri::WebviewWindow,
) -> Result<Vec<QueryPresetInfo>, String> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err("Voice Query presets are only available from the main window.".to_string());
    }
    Ok(PRESETS
        .iter()
        .map(|preset| QueryPresetInfo {
            id: preset.id,
            label: preset.label,
            summary: preset.summary,
            binary_name: preset.binary_name,
            recommended_arguments: preset
                .recommended_arguments
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            suggested_env_keys: preset
                .suggested_env_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            login_hint: preset.login_hint,
            discovered_path: discover(preset).map(|path| path.to_string_lossy().into_owned()),
        })
        .collect())
}

/// Preflight the configured command without recording anything.
///
/// Settings calls this when Voice Query is enabled and whenever the executable
/// changes, so a missing or non-executable path is reported at configuration
/// time instead of at the first keypress, mid-question.
#[tauri::command]
pub(crate) fn validate_query_command(
    window: tauri::WebviewWindow,
    command: crate::query_flow::QueryCommandConfig,
) -> Result<(), String> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err("Voice Query preflight is only available from the main window.".to_string());
    }
    crate::query_flow::validate_command(command, Vec::new())
        .map(|_| ())
        .map_err(|error_code| error_code.to_string())
}

#[tauri::command]
pub(crate) async fn probe_query_provider_auth(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    preset_id: Option<String>,
    command: crate::query_flow::QueryCommandConfig,
) -> Result<QueryAuthProbeReport, String> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err(
            "Voice Query provider checks are only available from the main window.".to_string(),
        );
    }
    // The probe spawns the same user CLI a query does, so it takes the same
    // exclusivity. Without this, pressing Test mid-question would run a second
    // provider process against the pipeline a real pass is already using.
    if is_pipeline_busy(&state) {
        return Err(
            "Murmur is recording or running another local task. Try the check again in a moment."
                .to_string(),
        );
    }
    let preset = preset_id.as_deref().and_then(preset);
    let Some(preset) = preset else {
        return Err(
            "Choose a provider preset to check sign-in status. A custom command has no known check."
                .to_string(),
        );
    };
    let declared = crate::query_env::spawn_pairs(&app);
    let validated = crate::query_flow::validate_command(command, declared.clone())
        .map_err(|error_code| error_code.to_string())?;
    let arguments: Vec<String> = preset
        .auth_probe_arguments
        .iter()
        .map(|argument| (*argument).to_string())
        .collect();

    let started = Instant::now();
    let executable = validated.executable_path().to_path_buf();
    let (exit_code, output, truncated) =
        tokio::task::spawn_blocking(move || run_probe(&executable, &arguments, &declared))
            .await
            .map_err(|_| "The provider check could not be completed.".to_string())??;
    let verdict = verdict_for(Some(preset), exit_code, &output);
    tracing::info!(
        target: "query",
        event_code = "query.auth_probe",
        preset = preset.id,
        verdict = ?verdict,
        exit_code,
        "voice-query provider check completed"
    );
    Ok(QueryAuthProbeReport {
        verdict,
        exit_code,
        output,
        truncated,
        duration_ms: started.elapsed().as_millis() as u64,
        login_hint: (verdict != AuthVerdict::Authenticated).then_some(preset.login_hint),
    })
}

/// Settings' "Sign in…" button: launch the chosen preset's own login.
///
/// Main window only. This is the one entry point that takes an executable path
/// from its caller, so it stays with the window the user configures in; the
/// popover uses the pass-scoped command below and names no path itself.
#[tauri::command]
pub(crate) fn launch_query_provider_login(
    window: tauri::WebviewWindow,
    preset_id: String,
    command: crate::query_flow::QueryCommandConfig,
) -> Result<(), String> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err("Voice Query sign-in is only available from the main window.".to_string());
    }
    let preset =
        preset(&preset_id).ok_or_else(|| "That provider is not recognised.".to_string())?;
    let validated = crate::query_flow::validate_command(command, Vec::new())
        .map_err(|_| "The configured CLI executable is missing or cannot be run.".to_string())?;
    launch_login_in_terminal(validated.executable_path(), preset.login_arguments)
}

/// The answer popover's "Sign in…" button: launch the login for the exact
/// command the failed pass used, without the popover naming any path itself.
#[tauri::command]
pub(crate) fn launch_query_pass_login(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::State>,
    query_pass_id: u64,
) -> Result<(), String> {
    require_configuration_window(window.label())?;
    let (executable, login_arguments) = state
        .query
        .login_target(query_pass_id)
        .ok_or_else(|| "That query is no longer available.".to_string())?;
    launch_login_in_terminal(&executable, login_arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_declares_a_usable_probe_and_login() {
        for preset in PRESETS {
            assert!(!preset.auth_probe_arguments.is_empty(), "{}", preset.id);
            assert!(!preset.login_arguments.is_empty(), "{}", preset.id);
            assert!(
                preset.login_hint.contains(preset.binary_name),
                "{}",
                preset.id
            );
            assert!(
                preset
                    .auth_failure_signatures
                    .iter()
                    .all(|signature| signature.to_lowercase() == *signature),
                "{} signatures must be lowercase to match",
                preset.id
            );
        }
        assert!(preset("claude").is_some());
        assert!(preset("custom").is_none());
    }

    #[test]
    fn a_signed_out_claude_is_recognised_even_though_it_exits_zero() {
        // The incident this issue came from: `claude auth status` succeeds and
        // reports the logged-out state in its payload.
        let signed_out = r#"{"loggedIn": false, "authMethod": null}"#;
        assert_eq!(
            verdict_for(preset("claude"), Some(0), signed_out),
            AuthVerdict::NotAuthenticated
        );
        let signed_in = r#"{"loggedIn": true, "authMethod": "claude.ai"}"#;
        assert_eq!(
            verdict_for(preset("claude"), Some(0), signed_in),
            AuthVerdict::Authenticated
        );
    }

    #[test]
    fn a_signed_in_provider_that_mentions_its_own_login_is_not_read_as_signed_out() {
        // Reporting a signed-in provider as signed out sends the user through a
        // pointless login, so a signature must match a refusal — not any
        // sentence that happens to name the login command.
        assert_eq!(
            verdict_for(
                preset("codex"),
                Some(0),
                "Logged in using ChatGPT. Use codex login to switch accounts."
            ),
            AuthVerdict::Authenticated
        );
        assert_eq!(
            verdict_for(
                preset("grok"),
                Some(0),
                "You are logged in with grok.com. Run grok logout first, then grok login again."
            ),
            AuthVerdict::Authenticated
        );
        // The refusals themselves are still recognised.
        assert_eq!(
            verdict_for(
                preset("codex"),
                Some(1),
                "Not logged in. Run `codex login`."
            ),
            AuthVerdict::NotAuthenticated
        );
    }

    #[test]
    fn a_clean_exit_alone_is_never_reported_as_signed_in() {
        // Settings promises a green check means the real query will work. A
        // probe that exits 0 with output this code has never seen — a renamed
        // subcommand, a shim, a stubbed binary — has not shown that.
        for preset_id in ["claude", "codex", "grok", "cursor"] {
            assert_eq!(
                verdict_for(preset(preset_id), Some(0), ""),
                AuthVerdict::Unknown,
                "{preset_id} must not go green on silence"
            );
            assert_eq!(
                verdict_for(preset(preset_id), Some(0), "usage: see --help"),
                AuthVerdict::Unknown,
                "{preset_id} must not go green on unrecognised output"
            );
        }
        // Each preset's real signed-in output still reads as authenticated.
        for (preset_id, output) in [
            ("claude", r#"{"loggedIn": true, "authMethod": "claude.ai"}"#),
            ("codex", "Logged in using ChatGPT"),
            ("grok", "You are logged in with grok.com."),
            ("cursor", "✓ Logged in as someone@example.com"),
        ] {
            assert_eq!(
                verdict_for(preset(preset_id), Some(0), output),
                AuthVerdict::Authenticated,
                "{preset_id} must recognise its own success output"
            );
        }
        // A custom executable declares no success wording, so a clean exit is
        // all there is to go on and remains the answer.
        assert_eq!(
            verdict_for(None, Some(0), "anything"),
            AuthVerdict::Authenticated
        );
    }

    #[test]
    fn a_bare_unauthorized_mention_is_not_treated_as_a_sign_in_failure() {
        // Remapping fires on failures whose output Murmur did not write; a
        // permission or quota error that merely contains the word must keep its
        // own error code rather than send the user to a pointless login.
        assert!(!indicates_auth_failure(
            None,
            "error: unauthorized to write to that repository"
        ));
        assert!(indicates_auth_failure(None, "HTTP 401 Unauthorized"));
        assert!(indicates_auth_failure(
            None,
            "request failed with status 401"
        ));
    }

    #[test]
    fn a_non_zero_exit_without_a_signature_stays_unknown() {
        assert_eq!(
            verdict_for(preset("codex"), Some(1), "network unreachable"),
            AuthVerdict::Unknown
        );
        assert_eq!(verdict_for(preset("codex"), None, ""), AuthVerdict::Unknown);
    }

    #[test]
    fn generic_signatures_cover_a_custom_executable_with_no_preset() {
        assert!(indicates_auth_failure(None, "Error: Not logged in"));
        assert!(indicates_auth_failure(None, "HTTP 401 Unauthorized"));
        assert!(indicates_auth_failure(
            None,
            "invalid API key · fix external"
        ));
        assert!(!indicates_auth_failure(None, "The answer is 42."));
        assert!(!indicates_auth_failure(None, ""));
    }

    #[test]
    fn a_login_command_line_is_quoted_rather_than_interpolated() {
        let line = login_command_line(
            Path::new("/Users/some one/bin/it's here/claude"),
            &["auth", "login"],
        )
        .unwrap();
        assert_eq!(
            line,
            r#"'/Users/some one/bin/it'\''s here/claude' 'auth' 'login'"#
        );
        assert!(login_command_line(Path::new("/tmp/bad\nname"), &[]).is_err());
    }

    #[test]
    fn applescript_literals_escape_quotes_and_backslashes() {
        assert_eq!(applescript_literal(r#"say "hi"\"#), r#"say \"hi\"\\"#);
    }

    #[test]
    fn probe_output_is_bounded_and_marked_when_truncated() {
        let (bytes, truncated) = read_bounded(std::io::Cursor::new(vec![
            b'a';
            MAX_PROBE_OUTPUT_BYTES
                + 512
        ]));
        assert_eq!(bytes.len(), MAX_PROBE_OUTPUT_BYTES);
        assert!(truncated);

        let (bytes, truncated) = read_bounded(std::io::Cursor::new(b"short".to_vec()));
        assert_eq!(bytes, b"short");
        assert!(!truncated);
    }

    #[cfg(unix)]
    #[test]
    fn discovery_finds_an_executable_under_a_home_relative_path() {
        use std::os::unix::fs::PermissionsExt;

        let preset = QueryPreset {
            id: "test",
            label: "Test",
            summary: "",
            binary_name: "murmur-discovery-probe",
            home_relative_paths: &["murmur-discovery-test/bin/tool"],
            recommended_arguments: &[],
            auth_probe_arguments: &["status"],
            login_arguments: &["login"],
            auth_failure_signatures: &[],
            auth_success_signatures: &[],
            suggested_env_keys: &[],
            login_hint: "murmur-discovery-probe login",
        };
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        let directory = home.join("murmur-discovery-test/bin");
        std::fs::create_dir_all(&directory).unwrap();
        let tool = directory.join("tool");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            discover(&preset),
            Some(std::fs::canonicalize(&tool).unwrap())
        );

        // A path that exists but is not executable is not a discovery.
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(discover(&preset), None);
        std::fs::remove_dir_all(home.join("murmur-discovery-test")).unwrap();
    }

    #[test]
    fn a_probe_returns_even_when_a_descendant_outlives_the_command() {
        // `sh` exits immediately while the background sleep keeps the inherited
        // stdout open. Joining the readers before confirming the owned group is
        // empty would hang here forever instead of returning a verdict.
        let arguments = vec![
            "-c".to_string(),
            "echo not logged in; sleep 30 &".to_string(),
        ];
        let (exit_code, output, _) = run_probe(Path::new("/bin/sh"), &arguments, &[]).unwrap();
        assert_eq!(exit_code, Some(0));
        assert!(output.contains("not logged in"), "{output}");
    }

    #[test]
    fn a_probe_reports_the_exit_code_and_merges_both_streams() {
        let arguments = vec![
            "-c".to_string(),
            "echo standard; echo not logged in 1>&2; exit 3".to_string(),
        ];
        let (exit_code, output, truncated) =
            run_probe(Path::new("/bin/sh"), &arguments, &[]).unwrap();
        assert_eq!(exit_code, Some(3));
        assert!(output.contains("standard"), "{output}");
        assert!(output.contains("not logged in"), "{output}");
        assert!(!truncated);
        assert_eq!(
            verdict_for(None, exit_code, &output),
            AuthVerdict::NotAuthenticated
        );
    }
}
