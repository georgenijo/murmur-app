//! Provider metadata and preflight support for Voice Query (#550).
//!
//! Presets are inert data. Every process still goes through
//! `ManagedChild::spawn_user_cli`, which starts one exact executable without a
//! shell and with a cleared, fail-closed environment.

use crate::managed_child::{
    apply_user_cli_base_environment, user_cli_base_environment, ManagedChild,
    USER_CLI_BASE_ENVIRONMENT,
};
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

pub(crate) const MAX_STDERR_BYTES: usize = 16 * 1024;
const MAX_PROBE_STDOUT_BYTES: usize = 16 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const TERMINATION_DEADLINE: Duration = Duration::from_secs(2);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ENVIRONMENT_FILE: &str = "query-environment.json";
const ENVIRONMENT_VERSION: u32 = 1;
const MAX_ENV_VALUE_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryProviderId {
    Claude,
    Codex,
    Grok,
    Cursor,
    #[default]
    Custom,
}

impl QueryProviderId {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Cursor => "cursor",
            Self::Custom => "custom",
        }
    }
}

struct ProviderPresetData {
    id: QueryProviderId,
    label: &'static str,
    command_name: &'static str,
    discovery_paths: &'static [&'static str],
    recommended_arguments: &'static [&'static str],
    auth_probe_arguments: &'static [&'static str],
    auth_failure_signatures: &'static [&'static str],
    sign_in_arguments: &'static [&'static str],
    sign_in_fix: Option<&'static str>,
    permitted_environment_variables: &'static [&'static str],
}

const CLAUDE: ProviderPresetData = ProviderPresetData {
    id: QueryProviderId::Claude,
    label: "Claude",
    command_name: "claude",
    discovery_paths: &[
        "~/.local/bin/claude",
        "/opt/homebrew/bin/claude",
        "/usr/local/bin/claude",
    ],
    recommended_arguments: &[
        "--print",
        "--verbose",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
    ],
    auth_probe_arguments: &["auth", "status"],
    auth_failure_signatures: &[
        "not logged in",
        "\"loggedin\": false",
        "\"logged_in\": false",
        "not authenticated",
        "please run /login",
        "authentication required",
    ],
    // Claude's interactive slash command is the repair users already know,
    // and matches the actionable guidance required by the issue.
    sign_in_arguments: &["/login"],
    sign_in_fix: Some("Run claude /login in Terminal."),
    permitted_environment_variables: &["CLAUDE_CONFIG_DIR"],
};

const CODEX: ProviderPresetData = ProviderPresetData {
    id: QueryProviderId::Codex,
    label: "Codex",
    command_name: "codex",
    discovery_paths: &[
        "/opt/homebrew/bin/codex",
        "/usr/local/bin/codex",
        "~/.local/bin/codex",
        "~/.npm-global/bin/codex",
    ],
    recommended_arguments: &[
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--ephemeral",
        "--color",
        "never",
    ],
    auth_probe_arguments: &["login", "status"],
    auth_failure_signatures: &[
        "not logged in",
        "not authenticated",
        "login required",
        "authentication required",
    ],
    sign_in_arguments: &["login"],
    sign_in_fix: Some("Run codex login in Terminal."),
    permitted_environment_variables: &["CODEX_HOME"],
};

const GROK: ProviderPresetData = ProviderPresetData {
    id: QueryProviderId::Grok,
    label: "Grok",
    command_name: "grok",
    discovery_paths: &[
        "/opt/homebrew/bin/grok",
        "/usr/local/bin/grok",
        "~/.local/bin/grok",
    ],
    recommended_arguments: &["-p"],
    // Grok does not currently expose a status command. Listing models is its
    // documented, read-only authenticated operation and exits before a query.
    auth_probe_arguments: &["models"],
    auth_failure_signatures: &[
        "not authenticated",
        "authentication required",
        "sign in to grok",
        "grok login",
        "xai_api_key",
    ],
    sign_in_arguments: &["login"],
    sign_in_fix: Some("Run grok login in Terminal."),
    permitted_environment_variables: &[],
};

const CURSOR: ProviderPresetData = ProviderPresetData {
    id: QueryProviderId::Cursor,
    label: "Cursor",
    command_name: "cursor-agent",
    discovery_paths: &[
        "~/.local/bin/cursor-agent",
        "/opt/homebrew/bin/cursor-agent",
        "/usr/local/bin/cursor-agent",
    ],
    recommended_arguments: &["--print", "--mode", "ask", "--single-turn"],
    auth_probe_arguments: &["status"],
    auth_failure_signatures: &[
        "not authenticated",
        "unauthenticated",
        "authentication token file is missing",
        "authentication is invalid",
    ],
    sign_in_arguments: &["login"],
    sign_in_fix: Some("Run cursor-agent login in Terminal."),
    permitted_environment_variables: &[],
};

const CUSTOM: ProviderPresetData = ProviderPresetData {
    id: QueryProviderId::Custom,
    label: "Custom",
    command_name: "",
    discovery_paths: &[],
    recommended_arguments: &[],
    auth_probe_arguments: &[],
    auth_failure_signatures: &[],
    sign_in_arguments: &[],
    sign_in_fix: None,
    // These are directory selectors, not credential values. Custom remains
    // bounded to the same two explicit additions as the built-in presets.
    permitted_environment_variables: &["CLAUDE_CONFIG_DIR", "CODEX_HOME"],
};

const PRESETS: [&ProviderPresetData; 5] = [&CLAUDE, &CODEX, &GROK, &CURSOR, &CUSTOM];

fn preset(id: QueryProviderId) -> &'static ProviderPresetData {
    match id {
        QueryProviderId::Claude => &CLAUDE,
        QueryProviderId::Codex => &CODEX,
        QueryProviderId::Grok => &GROK,
        QueryProviderId::Cursor => &CURSOR,
        QueryProviderId::Custom => &CUSTOM,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryProviderPreset {
    id: QueryProviderId,
    label: &'static str,
    discovery_paths: Vec<String>,
    discovered_executable: Option<String>,
    recommended_arguments: Vec<&'static str>,
    auth_probe_arguments: Vec<&'static str>,
    auth_failure_signatures: Vec<&'static str>,
    sign_in_arguments: Vec<&'static str>,
    sign_in_fix: Option<&'static str>,
    permitted_environment_variables: Vec<&'static str>,
}

fn expand_home(path: &str) -> Option<PathBuf> {
    if let Some(suffix) = path.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(suffix))
    } else {
        Some(PathBuf::from(path))
    }
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

fn discovery_candidates(data: &ProviderPresetData) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = data
        .discovery_paths
        .iter()
        .filter_map(|path| expand_home(path))
        .collect();
    if !data.command_name.is_empty() {
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(data.command_name)));
        }
    }
    let mut seen = HashSet::new();
    candidates.retain(|path| seen.insert(path.clone()));
    candidates
}

pub(crate) fn provider_presets() -> Vec<QueryProviderPreset> {
    PRESETS
        .iter()
        .map(|data| {
            let candidates = discovery_candidates(data);
            QueryProviderPreset {
                id: data.id,
                label: data.label,
                discovery_paths: candidates
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
                discovered_executable: candidates
                    .iter()
                    .find(|path| is_executable_file(path))
                    .map(|path| path.to_string_lossy().into_owned()),
                recommended_arguments: data.recommended_arguments.to_vec(),
                auth_probe_arguments: data.auth_probe_arguments.to_vec(),
                auth_failure_signatures: data.auth_failure_signatures.to_vec(),
                sign_in_arguments: data.sign_in_arguments.to_vec(),
                sign_in_fix: data.sign_in_fix,
                permitted_environment_variables: data.permitted_environment_variables.to_vec(),
            }
        })
        .collect()
}

pub(crate) fn auth_fix(provider: QueryProviderId) -> Option<&'static str> {
    preset(provider).sign_in_fix
}

pub(crate) fn is_auth_failure(provider: QueryProviderId, stdout: &str, stderr: &str) -> bool {
    let data = preset(provider);
    if data.auth_failure_signatures.is_empty() {
        return false;
    }
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    data.auth_failure_signatures
        .iter()
        .any(|signature| combined.contains(signature))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryEnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryEnvironmentStore {
    version: u32,
    providers: BTreeMap<String, Vec<QueryEnvironmentVariable>>,
}

static ENVIRONMENT_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn environment_path(dir: &Path) -> PathBuf {
    dir.join(ENVIRONMENT_FILE)
}

fn environment_temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(".{ENVIRONMENT_FILE}.murmur-tmp"))
}

fn empty_environment_store() -> QueryEnvironmentStore {
    QueryEnvironmentStore {
        version: ENVIRONMENT_VERSION,
        providers: BTreeMap::new(),
    }
}

fn validate_environment(
    provider: QueryProviderId,
    variables: &[QueryEnvironmentVariable],
) -> Result<(), &'static str> {
    let permitted = preset(provider).permitted_environment_variables;
    if variables.len() > permitted.len() {
        return Err("invalid_environment");
    }
    let mut seen = HashSet::new();
    for variable in variables {
        if !permitted.contains(&variable.name.as_str())
            || !seen.insert(variable.name.as_str())
            || variable.value.is_empty()
            || variable.value.len() > MAX_ENV_VALUE_BYTES
            || variable.value.contains('\0')
            || !Path::new(&variable.value).is_absolute()
        {
            return Err("invalid_environment");
        }
    }
    Ok(())
}

fn read_environment_store(dir: &Path) -> Result<QueryEnvironmentStore, &'static str> {
    let path = environment_path(dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(empty_environment_store())
        }
        Err(_) => return Err("environment_unavailable"),
    };
    if bytes.len() > 32 * 1024 {
        return Err("invalid_environment");
    }
    let store: QueryEnvironmentStore =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_environment")?;
    if store.version != ENVIRONMENT_VERSION {
        return Err("invalid_environment");
    }
    for (provider_id, variables) in &store.providers {
        let provider = match provider_id.as_str() {
            "claude" => QueryProviderId::Claude,
            "codex" => QueryProviderId::Codex,
            "grok" => QueryProviderId::Grok,
            "cursor" => QueryProviderId::Cursor,
            "custom" => QueryProviderId::Custom,
            _ => return Err("invalid_environment"),
        };
        validate_environment(provider, variables)?;
    }
    Ok(store)
}

fn write_environment_store(dir: &Path, store: &QueryEnvironmentStore) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(store).map_err(|_| "environment_unavailable")?;
    if bytes.len() > 32 * 1024 {
        return Err("invalid_environment");
    }
    std::fs::create_dir_all(dir).map_err(|_| "environment_unavailable")?;
    let path = environment_path(dir);
    let temp = environment_temp_path(dir);
    if std::fs::write(&temp, bytes).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err("environment_unavailable");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err("environment_unavailable");
        }
    }
    if std::fs::rename(&temp, &path).is_err() {
        let _ = std::fs::remove_file(&temp);
        return Err("environment_unavailable");
    }
    Ok(())
}

fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, &'static str> {
    app.path()
        .app_data_dir()
        .map_err(|_| "environment_unavailable")
}

pub(crate) fn load_environment(
    app: &tauri::AppHandle,
    provider: QueryProviderId,
) -> Result<Vec<QueryEnvironmentVariable>, &'static str> {
    let _guard = ENVIRONMENT_WRITE_LOCK.lock_or_recover();
    let store = read_environment_store(&app_data_dir(app)?)?;
    let variables = store
        .providers
        .get(provider.as_str())
        .cloned()
        .unwrap_or_default();
    validate_environment(provider, &variables)?;
    Ok(variables)
}

pub(crate) fn configured_environment_names(
    app: &tauri::AppHandle,
    provider: QueryProviderId,
) -> Result<Vec<String>, &'static str> {
    Ok(load_environment(app, provider)?
        .into_iter()
        .map(|variable| variable.name)
        .collect())
}

fn apply_environment_update(
    store: &mut QueryEnvironmentStore,
    provider: QueryProviderId,
    variables: Vec<QueryEnvironmentVariable>,
) {
    if variables.is_empty() {
        store.providers.remove(provider.as_str());
        return;
    }
    let saved = store
        .providers
        .entry(provider.as_str().to_string())
        .or_default();
    for variable in variables {
        if let Some(existing) = saved
            .iter_mut()
            .find(|existing| existing.name == variable.name)
        {
            *existing = variable;
        } else {
            saved.push(variable);
        }
    }
    saved.sort_by(|left, right| left.name.cmp(&right.name));
}

pub(crate) fn save_environment(
    app: &tauri::AppHandle,
    provider: QueryProviderId,
    variables: Vec<QueryEnvironmentVariable>,
) -> Result<(), &'static str> {
    validate_environment(provider, &variables)?;
    let _guard = ENVIRONMENT_WRITE_LOCK.lock_or_recover();
    let dir = app_data_dir(app)?;
    save_environment_in_dir(&dir, provider, variables)
}

fn save_environment_in_dir(
    dir: &Path,
    provider: QueryProviderId,
    variables: Vec<QueryEnvironmentVariable>,
) -> Result<(), &'static str> {
    validate_environment(provider, &variables)?;
    let mut store = match read_environment_store(dir) {
        Ok(store) => store,
        // Explicit Clear is the recovery path for a corrupt or future-version
        // file. Its contents cannot be trusted enough to preserve selectively,
        // so replace the whole store with a valid empty one atomically.
        Err("invalid_environment") if variables.is_empty() => empty_environment_store(),
        Err(error) => return Err(error),
    };
    apply_environment_update(&mut store, provider, variables);
    store.version = ENVIRONMENT_VERSION;
    write_environment_store(dir, &store)
}

#[derive(Default)]
struct TailBuffer {
    bytes: Vec<u8>,
    capacity: usize,
    truncated: bool,
}

impl TailBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ..Self::default()
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.capacity {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&bytes[bytes.len() - self.capacity..]);
            self.truncated = true;
            return;
        }
        let excess = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.capacity);
        if excess > 0 {
            self.bytes.drain(..excess);
            self.truncated = true;
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn text(&self) -> String {
        sanitize_output(&String::from_utf8_lossy(&self.bytes))
    }
}

pub(crate) fn sanitize_output(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character == '\n'
                || *character == '\r'
                || *character == '\t'
                || !character.is_control()
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Debug)]
struct BoundedCommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

#[cfg(unix)]
pub(crate) fn set_pipe_nonblocking<T: std::os::fd::AsRawFd>(pipe: &T) -> std::io::Result<()> {
    let descriptor = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_pipe_nonblocking<T>(_pipe: &T) -> std::io::Result<()> {
    Ok(())
}

fn read_bounded_pipe<R: Read>(mut pipe: R, tail: Arc<Mutex<TailBuffer>>, stop: Arc<AtomicBool>) {
    let mut buffer = [0_u8; 4096];
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => tail.lock_or_recover().push(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(CHILD_POLL_INTERVAL);
            }
            Err(_) => break,
        }
    }
}

fn run_bounded_command_with_timeout(
    executable: &Path,
    arguments: &[String],
    environment: &[QueryEnvironmentVariable],
    timeout: Duration,
) -> Result<BoundedCommandOutput, &'static str> {
    let environment: Vec<(String, String)> = environment
        .iter()
        .map(|variable| (variable.name.clone(), variable.value.clone()))
        .collect();
    let (mut child, stdin, stdout, stderr) =
        ManagedChild::spawn_user_cli(executable, arguments, &environment)
            .map_err(|_| "spawn_failed")?;
    drop(stdin);
    if set_pipe_nonblocking(&stdout).is_err() || set_pipe_nonblocking(&stderr).is_err() {
        let _ = child.hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
        return Err("process_failed");
    }

    // Readers write directly into bounded tails. Nonblocking pipe reads plus
    // the stop flag make teardown bounded even if a hostile descendant leaves
    // the owned process group while retaining stdout or stderr.
    let stdout_tail = Arc::new(Mutex::new(TailBuffer::new(MAX_PROBE_STDOUT_BYTES)));
    let stderr_tail = Arc::new(Mutex::new(TailBuffer::new(MAX_STDERR_BYTES)));
    let stop = Arc::new(AtomicBool::new(false));
    let stdout_reader = {
        let tail = Arc::clone(&stdout_tail);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || read_bounded_pipe(stdout, tail, stop))
    };
    let stderr_reader = {
        let tail = Arc::clone(&stderr_tail);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || read_bounded_pipe(stderr, tail, stop))
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        if Instant::now() >= deadline {
            let confirmed = child
                .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                .is_some();
            stop.store(true, Ordering::Release);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(if confirmed {
                "timed_out"
            } else {
                "termination_unconfirmed"
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if child
                    .wait_for_exit(Instant::now() + TERMINATION_DEADLINE)
                    .is_none()
                {
                    stop.store(true, Ordering::Release);
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err("termination_unconfirmed");
                }
                break status;
            }
            Ok(None) => std::thread::sleep(CHILD_POLL_INTERVAL),
            Err(_) => {
                let confirmed = child
                    .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                    .is_some();
                stop.store(true, Ordering::Release);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(if confirmed {
                    "process_failed"
                } else {
                    "termination_unconfirmed"
                });
            }
        }
    };
    let reader_drain_deadline = Instant::now() + Duration::from_millis(250);
    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
        && Instant::now() < reader_drain_deadline
    {
        std::thread::sleep(Duration::from_millis(1));
    }
    stop.store(true, Ordering::Release);
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    let stdout_tail = stdout_tail.lock_or_recover();
    let stderr_tail = stderr_tail.lock_or_recover();
    Ok(BoundedCommandOutput {
        success: status.success(),
        stdout: stdout_tail.text(),
        stderr: stderr_tail.text(),
        stdout_truncated: stdout_tail.truncated,
        stderr_truncated: stderr_tail.truncated,
    })
}

fn run_bounded_command(
    executable: &Path,
    arguments: &[String],
    environment: &[QueryEnvironmentVariable],
) -> Result<BoundedCommandOutput, &'static str> {
    run_bounded_command_with_timeout(executable, arguments, environment, PROBE_TIMEOUT)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryProviderTestResult {
    ok: bool,
    authenticated: Option<bool>,
    error_code: Option<&'static str>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
    sign_in_fix: Option<&'static str>,
}

impl QueryProviderTestResult {
    pub(crate) fn authenticated(&self) -> bool {
        self.authenticated == Some(true)
    }
}

pub(crate) fn run_auth_probe(
    provider: QueryProviderId,
    executable: &Path,
    environment: &[QueryEnvironmentVariable],
) -> QueryProviderTestResult {
    let data = preset(provider);
    if data.auth_probe_arguments.is_empty() {
        return QueryProviderTestResult {
            ok: true,
            authenticated: None,
            error_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            sign_in_fix: None,
        };
    }
    let arguments: Vec<String> = data
        .auth_probe_arguments
        .iter()
        .map(|argument| (*argument).to_string())
        .collect();
    match run_bounded_command(executable, &arguments, environment) {
        Ok(output) => {
            let auth_failure = is_auth_failure(provider, &output.stdout, &output.stderr);
            let ok = output.success && !auth_failure;
            QueryProviderTestResult {
                ok,
                authenticated: Some(ok),
                error_code: (!ok).then_some(if auth_failure {
                    "provider_not_authenticated"
                } else {
                    "probe_failed"
                }),
                stdout: output.stdout,
                stderr: output.stderr,
                stdout_truncated: output.stdout_truncated,
                stderr_truncated: output.stderr_truncated,
                sign_in_fix: auth_failure.then_some(data.sign_in_fix).flatten(),
            }
        }
        Err(error_code) => QueryProviderTestResult {
            ok: false,
            authenticated: Some(false),
            error_code: Some(error_code),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            sign_in_fix: None,
        },
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn terminal_sign_in_command(
    executable: &Path,
    arguments: &[&str],
    environment: &[QueryEnvironmentVariable],
    base_environment: &[(String, String)],
) -> String {
    let mut elements = vec![shell_quote("/usr/bin/env"), shell_quote("-i")];
    elements.extend(
        base_environment
            .iter()
            .filter(|(name, _)| USER_CLI_BASE_ENVIRONMENT.contains(&name.as_str()))
            .map(|(name, value)| shell_quote(&format!("{name}={value}"))),
    );
    elements.extend(
        environment
            .iter()
            .map(|variable| shell_quote(&format!("{}={}", variable.name, variable.value))),
    );
    elements.push(shell_quote(&executable.to_string_lossy()));
    elements.extend(arguments.iter().map(|argument| shell_quote(argument)));
    elements.join(" ")
}

/// Launch the provider-owned interactive sign-in inside Terminal. This is a
/// separate, explicit repair action; the query and auth-probe paths remain
/// direct process spawns and never pass through Terminal or a shell.
pub(crate) fn launch_sign_in(
    provider: QueryProviderId,
    executable: &Path,
    environment: &[QueryEnvironmentVariable],
) -> Result<(), &'static str> {
    let data = preset(provider);
    if data.sign_in_arguments.is_empty() {
        return Err("sign_in_unavailable");
    }
    validate_environment(provider, environment)?;
    let base_environment = user_cli_base_environment()
        .into_iter()
        .map(|(name, value)| {
            value
                .into_string()
                .map(|value| (name, value))
                .map_err(|_| "sign_in_failed")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let command = terminal_sign_in_command(
        executable,
        data.sign_in_arguments,
        environment,
        &base_environment,
    );
    // Keep user-controlled paths out of the AppleScript program itself. The
    // generated command is one osascript argv value and `do script` receives
    // it as data, so quotes or newlines cannot inject AppleScript statements.
    let script = "on run argv\nset signInCommand to item 1 of argv\ntell application \"Terminal\"\nactivate\ndo script signInCommand\nend tell\nend run";
    let mut osascript = std::process::Command::new("/usr/bin/osascript");
    osascript.args(["-e", script, &command]);
    apply_user_cli_base_environment(&mut osascript);
    let status = osascript.status().map_err(|_| "sign_in_failed")?;
    status.success().then_some(()).ok_or("sign_in_failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murmur_query_environment_test_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn presets_keep_provider_commands_and_auth_repairs_as_data() {
        let presets = provider_presets();
        assert_eq!(presets.len(), 5);
        let claude = presets
            .iter()
            .find(|preset| preset.id == QueryProviderId::Claude)
            .unwrap();
        assert_eq!(
            claude.recommended_arguments,
            vec![
                "--print",
                "--verbose",
                "--output-format",
                "stream-json",
                "--include-partial-messages"
            ]
        );
        assert_eq!(
            claude
                .recommended_arguments
                .iter()
                .filter(|argument| **argument == "--verbose")
                .count(),
            1
        );
        assert_eq!(claude.auth_probe_arguments, vec!["auth", "status"]);
        assert_eq!(claude.sign_in_fix, Some("Run claude /login in Terminal."));
        assert_eq!(
            claude.permitted_environment_variables,
            vec!["CLAUDE_CONFIG_DIR"]
        );
        let codex = presets
            .iter()
            .find(|preset| preset.id == QueryProviderId::Codex)
            .unwrap();
        assert_eq!(codex.recommended_arguments[0..2], ["exec", "--json"]);
        assert_eq!(
            codex
                .recommended_arguments
                .iter()
                .filter(|argument| **argument == "exec")
                .count(),
            1
        );
        assert_eq!(
            codex
                .recommended_arguments
                .iter()
                .filter(|argument| **argument == "--json")
                .count(),
            1
        );
    }

    #[test]
    fn declared_environment_is_provider_scoped_and_rejects_base_overrides() {
        let valid = vec![QueryEnvironmentVariable {
            name: "CLAUDE_CONFIG_DIR".into(),
            value: "/tmp/claude-config".into(),
        }];
        assert_eq!(
            validate_environment(QueryProviderId::Claude, &valid),
            Ok(())
        );
        assert_eq!(
            validate_environment(QueryProviderId::Codex, &valid),
            Err("invalid_environment")
        );
        for name in [
            "HOME", "PATH", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME",
        ] {
            let override_value = vec![QueryEnvironmentVariable {
                name: name.into(),
                value: "/tmp/value".into(),
            }];
            assert_eq!(
                validate_environment(QueryProviderId::Custom, &override_value),
                Err("invalid_environment"),
                "base allowlist override was accepted: {name}"
            );
        }
    }

    #[test]
    fn environment_store_round_trips_owner_only_path_values() {
        let dir = temp_dir("round_trip");
        let variables = vec![QueryEnvironmentVariable {
            name: "CODEX_HOME".into(),
            value: "/tmp/codex-home".into(),
        }];
        let mut store = QueryEnvironmentStore {
            version: ENVIRONMENT_VERSION,
            providers: BTreeMap::new(),
        };
        store.providers.insert("codex".into(), variables.clone());
        write_environment_store(&dir, &store).unwrap();
        let loaded = read_environment_store(&dir).unwrap();
        assert_eq!(loaded.providers["codex"], variables);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(environment_path(&dir))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn environment_updates_merge_without_dropping_other_values() {
        let mut store = QueryEnvironmentStore {
            version: ENVIRONMENT_VERSION,
            providers: BTreeMap::new(),
        };
        apply_environment_update(
            &mut store,
            QueryProviderId::Custom,
            vec![QueryEnvironmentVariable {
                name: "CODEX_HOME".into(),
                value: "/tmp/codex-one".into(),
            }],
        );
        apply_environment_update(
            &mut store,
            QueryProviderId::Custom,
            vec![QueryEnvironmentVariable {
                name: "CLAUDE_CONFIG_DIR".into(),
                value: "/tmp/claude".into(),
            }],
        );
        assert_eq!(store.providers["custom"].len(), 2);
        apply_environment_update(
            &mut store,
            QueryProviderId::Custom,
            vec![QueryEnvironmentVariable {
                name: "CODEX_HOME".into(),
                value: "/tmp/codex-two".into(),
            }],
        );
        assert_eq!(
            store.providers["custom"]
                .iter()
                .find(|variable| variable.name == "CODEX_HOME")
                .unwrap()
                .value,
            "/tmp/codex-two"
        );
        apply_environment_update(&mut store, QueryProviderId::Custom, vec![]);
        assert!(!store.providers.contains_key("custom"));
    }

    #[test]
    fn explicit_clear_recovers_corrupt_or_future_environment_store() {
        for (tag, contents) in [
            ("corrupt", b"not-json".as_slice()),
            ("future", br#"{"version":99,"providers":{}}"#.as_slice()),
        ] {
            let dir = temp_dir(tag);
            std::fs::write(environment_path(&dir), contents).unwrap();
            assert_eq!(
                save_environment_in_dir(&dir, QueryProviderId::Claude, vec![]),
                Ok(())
            );
            let recovered = read_environment_store(&dir).unwrap();
            assert_eq!(recovered.version, ENVIRONMENT_VERSION);
            assert!(recovered.providers.is_empty());
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn nonempty_save_does_not_overwrite_a_corrupt_environment_store() {
        let dir = temp_dir("corrupt_save");
        std::fs::write(environment_path(&dir), b"not-json").unwrap();
        let variables = vec![QueryEnvironmentVariable {
            name: "CLAUDE_CONFIG_DIR".into(),
            value: "/tmp/claude-config".into(),
        }];
        assert_eq!(
            save_environment_in_dir(&dir, QueryProviderId::Claude, variables),
            Err("invalid_environment")
        );
        assert_eq!(std::fs::read(environment_path(&dir)).unwrap(), b"not-json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stderr_tail_is_bounded_and_sanitized() {
        let mut tail = TailBuffer::new(8);
        tail.push(b"private-prefix");
        tail.push(b"ok\x1b[31m");
        assert!(tail.truncated);
        assert!(tail.bytes.len() <= 8);
        assert!(!tail.text().contains('\x1b'));
    }

    #[test]
    fn known_auth_signatures_are_provider_specific() {
        assert!(is_auth_failure(
            QueryProviderId::Claude,
            "",
            "Error: Not logged in"
        ));
        assert!(is_auth_failure(
            QueryProviderId::Claude,
            "{\"loggedIn\": false}",
            ""
        ));
        assert!(!is_auth_failure(
            QueryProviderId::Custom,
            "",
            "Error: Not logged in"
        ));
    }

    #[test]
    fn generic_probe_failure_does_not_offer_auth_repair() {
        let result = run_auth_probe(QueryProviderId::Claude, Path::new("/usr/bin/false"), &[]);
        assert!(!result.ok);
        assert_eq!(result.error_code, Some("probe_failed"));
        assert_eq!(result.sign_in_fix, None);
    }

    #[test]
    fn bounded_probe_drains_fast_exit_stdout_and_stderr() {
        let python = Path::new("/usr/bin/python3");
        if !python.exists() {
            return;
        }
        let arguments = vec![
            "-c".to_string(),
            "import sys; print('ready'); print('detail', file=sys.stderr)".to_string(),
        ];
        let output =
            // This verifies fast-exit pipe draining, not timeout behavior.
            // Leave enough headroom for a cold `/usr/bin/python3` launch on a
            // saturated macOS CI runner; the dedicated hostile-child test
            // below keeps the 150 ms deadline assertion.
            run_bounded_command_with_timeout(python, &arguments, &[], Duration::from_secs(5))
                .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout, "ready");
        assert_eq!(output.stderr, "detail");
    }

    #[test]
    fn terminal_command_quoting_cannot_break_out_of_one_shell_word() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn terminal_sign_in_command_starts_from_the_exact_allowlist() {
        let base_environment = vec![
            ("HOME".to_string(), "/Users/test".to_string()),
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            (
                "MURMUR_SENTINEL_SECRET".to_string(),
                "must-not-cross-boundary".to_string(),
            ),
        ];
        let declared = vec![QueryEnvironmentVariable {
            name: "CLAUDE_CONFIG_DIR".into(),
            value: "/tmp/claude config".into(),
        }];
        let command = terminal_sign_in_command(
            Path::new("/opt/homebrew/bin/claude"),
            &["/login"],
            &declared,
            &base_environment,
        );
        assert!(command.starts_with("'/usr/bin/env' '-i' "));
        assert!(command.contains("'HOME=/Users/test'"));
        assert!(command.contains("'PATH=/usr/bin:/bin'"));
        assert!(command.contains("'CLAUDE_CONFIG_DIR=/tmp/claude config'"));
        assert!(command.ends_with("'/opt/homebrew/bin/claude' '/login'"));
        assert!(!command.contains("MURMUR_SENTINEL_SECRET"));
    }

    #[test]
    #[cfg(unix)]
    fn probe_timeout_does_not_wait_for_escaped_pipe_holder() {
        let python = Path::new("/usr/bin/python3");
        if !python.exists() {
            return;
        }
        let dir = temp_dir("escaped_pipe_holder");
        let pid_path = dir.join("escaped.pid");
        let script = r#"
import os, sys, time
pid = os.fork()
if pid == 0:
    os.setsid()
    with open(sys.argv[1], "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))
        handle.flush()
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            os.write(2, b"escaped stderr " * 256)
        except BrokenPipeError:
            break
    os._exit(0)
time.sleep(5)
"#;
        let arguments = vec![
            "-c".to_string(),
            script.to_string(),
            pid_path.to_string_lossy().into_owned(),
        ];
        let started = Instant::now();
        assert_eq!(
            run_bounded_command_with_timeout(python, &arguments, &[], Duration::from_millis(150),)
                .unwrap_err(),
            "timed_out"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "probe teardown waited for an escaped descendant's inherited pipes"
        );

        if let Ok(pid) = std::fs::read_to_string(&pid_path)
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or(())
        {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
