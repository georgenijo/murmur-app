//! One-shot voice-query flow (#538).
//!
//! Speech stays inside Rust: capture and local ASR produce one question, then
//! the exact configured executable receives it as one final argv element.
//! No shell is involved, content is never traced, and only the dedicated
//! query-review webview receives answer chunks.

use crate::dictation_context::DictationContextSnapshot;
use crate::managed_child::ManagedChild;
use crate::model_runtime::PreparationReason;
use crate::MutexExt;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const MAX_EXECUTABLE_BYTES: usize = 4096;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 4096;
const MAX_ARGUMENTS_TOTAL_BYTES: usize = 32 * 1024;
const MAX_QUERY_BYTES: usize = 32 * 1024;
pub(crate) const MAX_ANSWER_BYTES: usize = 256 * 1024;
/// Tail of the CLI's stderr kept for the failure detail (#550). A tail rather
/// than a head: the line that explains why a run failed is the last one.
const MAX_STDERR_TAIL_BYTES: usize = 16 * 1024;
const MIN_TIMEOUT_SECONDS: u64 = 5;
const MAX_TIMEOUT_SECONDS: u64 = 300;
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const TERMINATION_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueryStatus {
    #[default]
    Idle,
    Connecting,
    Listening,
    Transcribing,
    Running,
    Ready,
    Failed,
}

impl QueryStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Listening => "listening",
            Self::Transcribing => "transcribing",
            Self::Running => "running",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    /// True while the query owns the microphone or the shared ASR backend.
    ///
    /// `finish_query_capture` stops and joins the query's audio owner before it
    /// transitions to `Transcribing`, and ASR completes before `Running`, so a
    /// query that has reached `Running` holds only its CLI child. Capture-only
    /// work (dictation, file transcription, microphone preview) is therefore
    /// free to start once the answer is being generated — which is the long
    /// phase of a query and used to lock the user out of dictation entirely.
    pub(crate) fn blocks_capture(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Listening | Self::Transcribing
        )
    }

    /// True for every non-terminal state, `Running` included.
    ///
    /// Stricter than [`Self::blocks_capture`]: the CLI child competes for CPU
    /// and may itself be a heavy inference runtime. Latency-sensitive work
    /// (benchmarks, corpus capture) and the local-LLM transform runtime stay
    /// mutually exclusive with it, so they keep using this predicate.
    pub(crate) fn blocks_pipeline(self) -> bool {
        self.blocks_capture() || matches!(self, Self::Running)
    }

    fn accepts_new_pass(self) -> bool {
        matches!(self, Self::Idle | Self::Ready | Self::Failed)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryCommandConfig {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub timeout_seconds: u64,
    /// Provider preset this command was configured from, when it was not
    /// assembled by hand. It selects the auth-failure signatures and the login
    /// the popover offers; it never changes what is spawned.
    #[serde(default)]
    pub preset_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ValidatedQueryCommand {
    executable: PathBuf,
    arguments: Vec<String>,
    timeout: Duration,
    /// Declared name/value pairs, already validated by `query_env`.
    environment: Vec<(String, String)>,
    preset: Option<&'static crate::query_presets::QueryPreset>,
}

impl ValidatedQueryCommand {
    pub(crate) fn executable_path(&self) -> &Path {
        &self.executable
    }
}

#[derive(Clone)]
struct QuerySession {
    pass_id: u64,
    context: Arc<DictationContextSnapshot>,
    command: ValidatedQueryCommand,
    answer: String,
    /// Bounded stderr tail from the failed run. Local to the popover, exactly
    /// like the answer: it can quote paths, account names, and prompts.
    error_detail: Option<String>,
}

struct ActiveQueryChild {
    pass_id: u64,
    child: Arc<Mutex<ManagedChild>>,
}

pub(crate) struct QueryCoordinator {
    pass_sequence: AtomicU64,
    active_pass_id: AtomicU64,
    cancelled_pass_id: AtomicU64,
    status: Mutex<QueryStatus>,
    session: Mutex<Option<QuerySession>>,
    child: Mutex<Option<ActiveQueryChild>>,
}

impl Default for QueryCoordinator {
    fn default() -> Self {
        Self {
            pass_sequence: AtomicU64::new(0),
            active_pass_id: AtomicU64::new(0),
            cancelled_pass_id: AtomicU64::new(0),
            status: Mutex::new(QueryStatus::Idle),
            session: Mutex::new(None),
            child: Mutex::new(None),
        }
    }
}

impl QueryCoordinator {
    /// Called only from the shared rdev callback. A terminal review can be
    /// superseded; an in-flight pass is never replaced.
    pub(crate) fn allocate_keyboard_pass(&self) -> Option<u64> {
        let status = *self.status.lock_or_recover();
        if !status.accepts_new_pass() || self.child.lock_or_recover().is_some() {
            return None;
        }
        let pass_id = self.pass_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.active_pass_id.store(pass_id, Ordering::SeqCst);
        Some(pass_id)
    }

    pub(crate) fn active_pass_id(&self) -> Option<u64> {
        match self.active_pass_id.load(Ordering::SeqCst) {
            0 => None,
            value => Some(value),
        }
    }

    pub(crate) fn status(&self) -> QueryStatus {
        *self.status.lock_or_recover()
    }

    pub(crate) fn is_active(&self, pass_id: u64) -> bool {
        self.active_pass_id() == Some(pass_id)
            && self.cancelled_pass_id.load(Ordering::SeqCst) < pass_id
    }

    fn set_status(&self, pass_id: u64, status: QueryStatus) -> bool {
        if !self.is_active(pass_id) {
            return false;
        }
        *self.status.lock_or_recover() = status;
        true
    }

    fn mark_cancelled(&self, pass_id: u64) {
        self.cancelled_pass_id.fetch_max(pass_id, Ordering::SeqCst);
    }

    fn clear_pass(&self, pass_id: u64) -> bool {
        self.active_pass_id
            .compare_exchange(pass_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn install_session(&self, pass_id: u64, session: QuerySession) -> bool {
        if !self.is_active(pass_id) {
            return false;
        }
        *self.session.lock_or_recover() = Some(session);
        true
    }

    fn session(&self, pass_id: u64) -> Option<QuerySession> {
        self.is_active(pass_id)
            .then(|| self.session.lock_or_recover().clone())
            .flatten()
            .filter(|session| session.pass_id == pass_id)
    }

    fn append_answer(&self, pass_id: u64, text: &str) -> Result<(), &'static str> {
        if !self.is_active(pass_id) {
            return Err("stale_pass");
        }
        let mut slot = self.session.lock_or_recover();
        let session = slot.as_mut().ok_or("stale_pass")?;
        if session.answer.len().saturating_add(text.len()) > MAX_ANSWER_BYTES {
            return Err("output_too_large");
        }
        session.answer.push_str(text);
        Ok(())
    }

    fn answer(&self, pass_id: u64) -> Option<String> {
        self.session(pass_id).map(|session| session.answer)
    }

    fn set_error_detail(&self, pass_id: u64, detail: String) {
        if !self.is_active(pass_id) {
            return;
        }
        let mut slot = self.session.lock_or_recover();
        if let Some(session) = slot.as_mut().filter(|session| session.pass_id == pass_id) {
            session.error_detail = Some(detail);
        }
    }

    /// Executable and argv for the provider's own login, for the popover's
    /// "Sign in…" button. `None` when the pass is gone or was configured by
    /// hand, in which case no vendor login is known.
    pub(crate) fn login_target(
        &self,
        pass_id: u64,
    ) -> Option<(PathBuf, &'static [&'static str])> {
        let session = self.session(pass_id)?;
        let preset = session.command.preset?;
        Some((session.command.executable, preset.login_arguments))
    }

    fn install_child(&self, pass_id: u64, child: Arc<Mutex<ManagedChild>>) -> bool {
        if !self.is_active(pass_id) {
            return false;
        }
        *self.child.lock_or_recover() = Some(ActiveQueryChild { pass_id, child });
        true
    }

    fn clear_child(&self, pass_id: u64) {
        let mut slot = self.child.lock_or_recover();
        if slot
            .as_ref()
            .is_some_and(|active| active.pass_id == pass_id)
        {
            *slot = None;
        }
    }

    fn terminate_child(&self, pass_id: u64) -> bool {
        let child = self
            .child
            .lock_or_recover()
            .as_ref()
            .filter(|active| active.pass_id == pass_id)
            .map(|active| Arc::clone(&active.child));
        let Some(child) = child else {
            return true;
        };
        let confirmed = child
            .lock_or_recover()
            .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
            .is_some();
        if confirmed {
            self.clear_child(pass_id);
        }
        confirmed
    }

    pub(crate) fn shutdown(&self) {
        let active = self
            .child
            .lock_or_recover()
            .as_ref()
            .map(|active| (active.pass_id, Arc::clone(&active.child)));
        if let Some((pass_id, child)) = active {
            let confirmed = child
                .lock_or_recover()
                .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                .is_some();
            if confirmed {
                self.clear_child(pass_id);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuerySignIn {
    provider: &'static str,
    /// The exact command to run, shown verbatim so the instruction is copyable.
    hint: &'static str,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryReviewContent {
    query_pass_id: Option<u64>,
    answer: String,
    error_detail: Option<String>,
    sign_in: Option<QuerySignIn>,
}

pub(crate) fn validate_command(
    config: QueryCommandConfig,
    environment: Vec<(String, String)>,
) -> Result<ValidatedQueryCommand, &'static str> {
    let executable = config.executable.trim();
    if executable.is_empty() {
        return Err("not_configured");
    }
    if executable.len() > MAX_EXECUTABLE_BYTES || executable.contains('\0') {
        return Err("invalid_executable");
    }
    let path = Path::new(executable);
    if !path.is_absolute() {
        return Err("invalid_executable");
    }
    let executable = std::fs::canonicalize(path).map_err(|_| "invalid_executable")?;
    let metadata = std::fs::metadata(&executable).map_err(|_| "invalid_executable")?;
    if !metadata.is_file() {
        return Err("invalid_executable");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("invalid_executable");
        }
    }
    if config.arguments.len() > MAX_ARGUMENTS
        || config
            .arguments
            .iter()
            .any(|argument| argument.len() > MAX_ARGUMENT_BYTES || argument.contains('\0'))
        || config.arguments.iter().map(String::len).sum::<usize>() > MAX_ARGUMENTS_TOTAL_BYTES
    {
        return Err("invalid_arguments");
    }
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&config.timeout_seconds) {
        return Err("invalid_timeout");
    }
    // An unrecognised preset id is a configuration the app does not understand,
    // not a licence to run the command without provider context.
    let preset = match config.preset_id.as_deref() {
        None | Some("") | Some("custom") => None,
        Some(id) => Some(crate::query_presets::preset(id).ok_or("invalid_preset")?),
    };
    Ok(ValidatedQueryCommand {
        executable,
        arguments: config.arguments,
        timeout: Duration::from_secs(config.timeout_seconds),
        environment,
        preset,
    })
}

/// Rolling tail of a child's stderr.
///
/// Bounded from the start: a CLI that logs a megabyte of progress must not be
/// able to grow Murmur's memory, and the useful part of a failure is the end.
#[derive(Default)]
struct StderrTail {
    buffer: Vec<u8>,
    truncated: bool,
}

impl StderrTail {
    fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_STDERR_TAIL_BYTES {
            let excess = self.buffer.len() - MAX_STDERR_TAIL_BYTES;
            self.buffer.drain(..excess);
            self.truncated = true;
        }
    }

    fn text(&self) -> String {
        let text = String::from_utf8_lossy(&self.buffer);
        // Dropping the tail mid-scalar leaves a leading replacement character.
        let text = text.trim_start_matches('\u{fffd}').trim();
        if text.is_empty() {
            String::new()
        } else if self.truncated {
            format!("…{text}")
        } else {
            text.to_string()
        }
    }
}

/// Turn a terminal failure into the most actionable error code available.
///
/// A provider that has lost its credentials is by far the most common failure
/// and the one a bare `exit_nonzero` explains worst, so it gets its own code
/// and its own fix. Everything else keeps the code the run produced.
fn actionable_error_code(
    error_code: &'static str,
    preset: Option<&crate::query_presets::QueryPreset>,
    stderr_tail: &str,
    answer: &str,
) -> &'static str {
    const AUTH_MAPPABLE: [&str; 3] = ["exit_nonzero", "process_failed", "empty_answer"];
    if !AUTH_MAPPABLE.contains(&error_code) {
        return error_code;
    }
    if crate::query_presets::indicates_auth_failure(preset, stderr_tail)
        || crate::query_presets::indicates_auth_failure(preset, answer)
    {
        return "provider_not_authenticated";
    }
    error_code
}

fn emit_state(
    app: &tauri::AppHandle,
    pass_id: u64,
    status: QueryStatus,
    error_code: Option<&'static str>,
) {
    let _ = app.emit(
        "query-state-changed",
        serde_json::json!({
            "queryPassId": pass_id,
            "state": status.as_str(),
            "errorCode": error_code,
        }),
    );
    tracing::info!(
        target: "query",
        event_code = "query.pass_state",
        query_pass_id = pass_id,
        state = status.as_str(),
        error_code,
        "query state changed"
    );
}

/// True when the answer may claim the clipboard: nothing else has written it
/// since `snapshot` was taken at the start of the CLI run.
fn may_claim_clipboard(snapshot: u64, current: u64) -> bool {
    snapshot == current
}

/// Fail the pass and leave the CLI's own words behind for the popover.
///
/// The detail is stored on the session rather than broadcast: it is requester
/// gated to the `query-review` window exactly like the answer, because a stderr
/// tail can quote the question, a path, or an account name.
fn fail_query_with_detail(
    app: &tauri::AppHandle,
    state: &crate::State,
    pass_id: u64,
    error_code: &'static str,
    detail: String,
) {
    if !detail.is_empty() {
        state.query.set_error_detail(pass_id, detail);
    }
    fail_query(app, state, pass_id, error_code);
}

fn fail_query(
    app: &tauri::AppHandle,
    state: &crate::State,
    pass_id: u64,
    error_code: &'static str,
) {
    crate::keyboard::set_query_recording_state(false);
    if state.query.set_status(pass_id, QueryStatus::Failed) {
        // Validation and ownership refusals happen before the normal compact
        // popover show, so terminal failures must make themselves visible too.
        let _ = crate::commands::query_popover::show_internal(app, true);
        let _ = crate::commands::query_popover::set_expanded_internal(app, true);
        emit_state(app, pass_id, QueryStatus::Failed, Some(error_code));
    }
}

fn prepare_model(app: tauri::AppHandle, pass_id: u64, model_name: String) {
    drop(tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::State>();
        if !state.query.is_active(pass_id)
            || !matches!(
                state.query.status(),
                QueryStatus::Connecting | QueryStatus::Listening
            )
        {
            return;
        }
        let _ = state.app_state.model_runtime.prepare(
            Some(&app),
            &model_name,
            PreparationReason::Recording,
        );
    }));
}

#[tauri::command]
pub(crate) async fn start_query_capture(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    device_name: Option<String>,
    query_pass_id: u64,
    command: QueryCommandConfig,
) -> Result<(), String> {
    // Resolved once, at the start of the pass, like every other piece of
    // per-recording context: editing the declared variables mid-question
    // applies to the next query, never this one.
    let declared_environment = crate::query_env::spawn_pairs(&app_handle);
    let command = match validate_command(command, declared_environment) {
        Ok(command) => command,
        Err(error_code) => {
            fail_query(&app_handle, &state, query_pass_id, error_code);
            return Ok(());
        }
    };
    let _transition = crate::commands::microphone_preview::transition_after_stopping_preview(
        &app_handle,
        state.inner(),
    )
    .await?;
    if !state.query.is_active(query_pass_id) {
        return Ok(());
    }

    #[cfg(feature = "internal-benchmark")]
    let corpus_idle = !state.corpus.is_active();
    #[cfg(not(feature = "internal-benchmark"))]
    let corpus_idle = true;
    let allowed = {
        let dictation = state.app_state.dictation.lock_or_recover();
        dictation.status == crate::state::DictationStatus::Idle
            && !state.app_state.file_transcribing.load(Ordering::SeqCst)
            && !state.benchmark.is_running()
            && !state.app_state.transform_status().blocks_recording()
            && !state.transform_runtime.is_transform_busy()
            && corpus_idle
            && !crate::keyboard::is_app_disabled()
    };
    if !allowed {
        fail_query(&app_handle, &state, query_pass_id, "busy");
        let _ = app_handle.emit("query-busy", ());
        return Ok(());
    }

    let identity = crate::frontmost::frontmost_app_identity();
    let context = crate::commands::recording::resolve_live_context(
        &state.app_state,
        &state.knowledge,
        &identity,
    );
    let session = QuerySession {
        pass_id: query_pass_id,
        context: Arc::clone(&context),
        command,
        answer: String::new(),
        error_detail: None,
    };
    if !state.query.install_session(query_pass_id, session)
        || !state
            .query
            .set_status(query_pass_id, QueryStatus::Connecting)
    {
        return Ok(());
    }

    let _ = crate::commands::query_popover::show_internal(&app_handle, false);
    emit_state(&app_handle, query_pass_id, QueryStatus::Connecting, None);
    if let Err(_error) = crate::audio::start_query_capture_audio(
        Some(app_handle.clone()),
        device_name,
        query_pass_id,
    ) {
        fail_query(&app_handle, &state, query_pass_id, "audio_start_failed");
        return Ok(());
    }
    prepare_model(
        app_handle,
        query_pass_id,
        context.transcription.model_name.clone(),
    );
    Ok(())
}

pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    query_pass_id: u64,
    event: crate::audio_lifecycle::AudioLifecycleEvent,
) {
    let state = app_handle.state::<crate::State>();
    if !state.query.is_active(query_pass_id) {
        return;
    }
    match event {
        crate::audio_lifecycle::AudioLifecycleEvent::Accepted => {}
        crate::audio_lifecycle::AudioLifecycleEvent::Ready => {
            if state
                .query
                .set_status(query_pass_id, QueryStatus::Listening)
            {
                crate::keyboard::set_query_recording_state(true);
                emit_state(&app_handle, query_pass_id, QueryStatus::Listening, None);
            }
        }
        crate::audio_lifecycle::AudioLifecycleEvent::StillConnecting => {
            if state.query.status() == QueryStatus::Connecting {
                emit_state(
                    &app_handle,
                    query_pass_id,
                    QueryStatus::Connecting,
                    Some("audio_stalled"),
                );
            }
        }
        crate::audio_lifecycle::AudioLifecycleEvent::InitializationFailed { .. }
        | crate::audio_lifecycle::AudioLifecycleEvent::Interrupted { .. } => {
            fail_query(&app_handle, &state, query_pass_id, "audio_start_failed");
        }
        crate::audio_lifecycle::AudioLifecycleEvent::Recovering { .. } => {
            fail_query(&app_handle, &state, query_pass_id, "audio_recovering");
        }
        crate::audio_lifecycle::AudioLifecycleEvent::RecoveryStalled => {
            fail_query(&app_handle, &state, query_pass_id, "audio_recovery_stalled");
        }
        crate::audio_lifecycle::AudioLifecycleEvent::Idle => {}
    }
}

async fn transcribe_query(
    app: &tauri::AppHandle,
    state: &crate::State,
    pass_id: u64,
    samples: Vec<f32>,
    context: &DictationContextSnapshot,
) -> Result<String, &'static str> {
    if samples.is_empty() {
        return Err("no_speech");
    }
    let transcription = &context.transcription;
    let (samples_for_transcription, vad_trimmed) =
        if !crate::vad::is_enabled(transcription.vad_sensitivity) {
            (samples.clone(), false)
        } else {
            let threshold = crate::vad::threshold_for_sensitivity(transcription.vad_sensitivity);
            match crate::vad::vad_model_path().filter(|path| path.exists()) {
                Some(path) => {
                    let path = path.to_string_lossy().into_owned();
                    let input = samples.clone();
                    match tokio::task::spawn_blocking(move || {
                        crate::vad::filter_speech(&path, &input, threshold)
                    })
                    .await
                    .map_err(|_| "transcription_failed")?
                    {
                        Ok(crate::vad::VadResult::NoSpeech) => return Err("no_speech"),
                        Ok(crate::vad::VadResult::Speech(filtered)) => {
                            let trimmed = filtered.len() != samples.len();
                            (filtered, trimmed)
                        }
                        Err(_) => (samples.clone(), false),
                    }
                }
                None => (samples.clone(), false),
            }
        };
    if !state.query.is_active(pass_id) {
        return Err("cancelled");
    }
    let (raw, _) = state
        .app_state
        .model_runtime
        .with_ready_backend(
            Some(app),
            &transcription.model_name,
            PreparationReason::Pipeline,
            |backend| {
                crate::commands::recording::transcribe_with_coreml_vad_retry(
                    backend,
                    &transcription.model_name,
                    &samples_for_transcription,
                    &samples,
                    vad_trimmed,
                    &transcription.language,
                    transcription.prompt.as_deref(),
                    transcription.smart_punctuation,
                )
            },
        )
        .map_err(|_| "transcription_failed")?;
    let transform_context = crate::transcript_transform::TranscriptContext {
        session_id: state.app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::Live,
        context_handle: None,
        cli_formatting_mode: crate::cli_command::CliFormattingMode::Auto,
        stages: crate::transcript_transform::TranscriptStageConfig::instruction_cleanup(),
    };
    let cleaned = crate::transcript_transform::transform_transcript(
        raw,
        &transform_context,
        crate::transcript_transform::TranscriptTransformResources::empty(),
    )
    .map_err(|_| "transcription_failed")?
    .text;
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        Err("empty_query")
    } else if cleaned.len() > MAX_QUERY_BYTES {
        Err("query_too_large")
    } else {
        Ok(cleaned)
    }
}

struct Utf8Chunks {
    pending: Vec<u8>,
}

impl Utf8Chunks {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_string();
                self.pending.clear();
                text
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if error.error_len().is_none() {
                    let text = String::from_utf8_lossy(&self.pending[..valid]).into_owned();
                    self.pending.drain(..valid);
                    text
                } else {
                    let text = String::from_utf8_lossy(&self.pending).into_owned();
                    self.pending.clear();
                    text
                }
            }
        }
    }

    fn finish(self) -> String {
        String::from_utf8_lossy(&self.pending).into_owned()
    }
}

fn run_cli(
    app: tauri::AppHandle,
    pass_id: u64,
    command: ValidatedQueryCommand,
    query: String,
    stderr_tail: Arc<Mutex<StderrTail>>,
) -> Result<(), &'static str> {
    let mut arguments = command.arguments;
    // The transcript is one final argv element. It is never parsed, quoted,
    // substituted, or evaluated by a shell.
    arguments.push(query);
    let (child, stdin, mut stdout, mut stderr) =
        ManagedChild::spawn_user_cli(&command.executable, &arguments, &command.environment)
            .map_err(|_| "spawn_failed")?;
    drop(stdin);
    let child = Arc::new(Mutex::new(child));

    // stderr must be drained continuously or a chatty CLI fills the pipe buffer
    // and blocks forever. The thread ends at EOF, which the confirmed
    // process-group teardown below guarantees on every exit path, so the tail
    // stays readable through the shared bound without joining first.
    let tail_writer = Arc::clone(&stderr_tail);
    drop(std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => tail_writer.lock_or_recover().push(&buffer[..count]),
            }
        }
    }));
    {
        let state = app.state::<crate::State>();
        if !state.query.install_child(pass_id, Arc::clone(&child)) {
            let _ = child
                .lock_or_recover()
                .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
            return Err("cancelled");
        }
    }

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let reader = std::thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match stdout.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if tx.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + command.timeout;
    let mut decoder = Utf8Chunks::new();
    let mut sequence = 0_u64;
    let exit_status = loop {
        {
            let state = app.state::<crate::State>();
            if !state.query.is_active(pass_id) {
                let _ = child
                    .lock_or_recover()
                    .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE);
                let _ = reader.join();
                state.query.clear_child(pass_id);
                return Err("cancelled");
            }
        }
        while let Ok(bytes) = rx.try_recv() {
            let text = decoder.push(&bytes);
            if !text.is_empty() {
                let state = app.state::<crate::State>();
                if let Err(error_code) = state.query.append_answer(pass_id, &text) {
                    let confirmed = child
                        .lock_or_recover()
                        .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                        .is_some();
                    let _ = reader.join();
                    state.query.clear_child(pass_id);
                    return Err(if confirmed {
                        error_code
                    } else {
                        "termination_unconfirmed"
                    });
                }
                let _ = crate::commands::query_popover::set_expanded_internal(&app, true);
                let _ = app.emit_to(
                    "query-review",
                    "query-answer-chunk",
                    serde_json::json!({
                        "queryPassId": pass_id,
                        "sequence": sequence,
                        "text": text,
                    }),
                );
                sequence += 1;
            }
        }
        if Instant::now() >= deadline {
            let confirmed = child
                .lock_or_recover()
                .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                .is_some();
            let _ = reader.join();
            app.state::<crate::State>().query.clear_child(pass_id);
            return Err(if confirmed {
                "timed_out"
            } else {
                "termination_unconfirmed"
            });
        }
        let wait_result = { child.lock_or_recover().try_wait() };
        match wait_result {
            Ok(Some(status)) => {
                // A wrapper CLI can exit while a descendant still inherits
                // stdout. Confirm (and, if necessary, kill) the entire owned
                // process group before joining the reader, otherwise that
                // inherited pipe could keep this pass blocked indefinitely.
                let confirmed = child
                    .lock_or_recover()
                    .wait_for_exit(Instant::now() + TERMINATION_DEADLINE)
                    .is_some();
                if !confirmed {
                    app.state::<crate::State>().query.clear_child(pass_id);
                    return Err("termination_unconfirmed");
                }
                break status;
            }
            Ok(None) => {}
            Err(_) => {
                let confirmed = child
                    .lock_or_recover()
                    .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                    .is_some();
                let _ = reader.join();
                app.state::<crate::State>().query.clear_child(pass_id);
                return Err(if confirmed {
                    "process_failed"
                } else {
                    "termination_unconfirmed"
                });
            }
        }
        std::thread::sleep(CHILD_POLL_INTERVAL);
    };

    let _ = reader.join();
    while let Ok(bytes) = rx.try_recv() {
        let text = decoder.push(&bytes);
        if !text.is_empty() {
            let state = app.state::<crate::State>();
            if let Err(error_code) = state.query.append_answer(pass_id, &text) {
                let confirmed = child
                    .lock_or_recover()
                    .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                    .is_some();
                state.query.clear_child(pass_id);
                return Err(if confirmed {
                    error_code
                } else {
                    "termination_unconfirmed"
                });
            }
            let _ = app.emit_to(
                "query-review",
                "query-answer-chunk",
                serde_json::json!({
                    "queryPassId": pass_id,
                    "sequence": sequence,
                    "text": text,
                }),
            );
            sequence += 1;
        }
    }
    let tail = decoder.finish();
    if !tail.is_empty() {
        let state = app.state::<crate::State>();
        if let Err(error_code) = state.query.append_answer(pass_id, &tail) {
            let confirmed = child
                .lock_or_recover()
                .hard_kill_confirmed(Instant::now() + TERMINATION_DEADLINE)
                .is_some();
            state.query.clear_child(pass_id);
            return Err(if confirmed {
                error_code
            } else {
                "termination_unconfirmed"
            });
        }
        let _ = app.emit_to(
            "query-review",
            "query-answer-chunk",
            serde_json::json!({
                "queryPassId": pass_id,
                "sequence": sequence,
                "text": tail,
            }),
        );
    }
    let confirmed = child
        .lock_or_recover()
        .wait_for_exit(Instant::now() + TERMINATION_DEADLINE)
        .is_some();
    app.state::<crate::State>().query.clear_child(pass_id);
    if !confirmed {
        return Err("termination_unconfirmed");
    }
    if !exit_status.success() {
        return Err("exit_nonzero");
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn finish_query_capture(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    query_pass_id: u64,
) -> Result<(), String> {
    let samples = {
        let _transition = state.app_state.recording_transition.lock().await;
        if !state.query.is_active(query_pass_id) {
            return Ok(());
        }
        match state.query.status() {
            QueryStatus::Connecting => {
                let _ = crate::audio_lifecycle::cancel_query_capture(
                    query_pass_id,
                    crate::audio_lifecycle::AudioCancelReason::User,
                );
                fail_query(&app_handle, &state, query_pass_id, "audio_not_ready");
                return Ok(());
            }
            QueryStatus::Listening => {}
            _ => return Ok(()),
        }
        crate::keyboard::set_query_recording_state(false);
        let samples = crate::audio_lifecycle::stop_query_recording(query_pass_id)
            .map_err(|_| "Could not stop query recording".to_string())?;
        state
            .query
            .set_status(query_pass_id, QueryStatus::Transcribing);
        emit_state(&app_handle, query_pass_id, QueryStatus::Transcribing, None);
        samples
    };

    let Some(session) = state.query.session(query_pass_id) else {
        return Ok(());
    };
    let query = match transcribe_query(
        &app_handle,
        &state,
        query_pass_id,
        samples,
        &session.context,
    )
    .await
    {
        Ok(query) => query,
        Err("cancelled") => return Ok(()),
        Err(error_code) => {
            fail_query(&app_handle, &state, query_pass_id, error_code);
            return Ok(());
        }
    };
    if !state.query.set_status(query_pass_id, QueryStatus::Running) {
        return Ok(());
    }
    emit_state(&app_handle, query_pass_id, QueryStatus::Running, None);

    // Snapshot the clipboard before the CLI runs. Dictation is allowed to start
    // during `Running`, so the user may deliberately produce and paste text
    // while the answer generates; if that happened we must not overwrite it.
    let clipboard_generation = crate::injector::clipboard_write_generation();

    let command = session.command;
    let preset = command.preset;
    let stderr_tail = Arc::new(Mutex::new(StderrTail::default()));
    let worker_tail = Arc::clone(&stderr_tail);
    let worker_app = app_handle.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_cli(worker_app, query_pass_id, command, query, worker_tail)
    })
    .await
    .unwrap_or(Err("process_failed"));
    if !state.query.is_active(query_pass_id) {
        return Ok(());
    }
    let stderr_detail = stderr_tail.lock_or_recover().text();
    match result {
        Ok(()) => {
            let answer = state.query.answer(query_pass_id).unwrap_or_default();
            if answer.trim().is_empty() {
                fail_query_with_detail(
                    &app_handle,
                    &state,
                    query_pass_id,
                    actionable_error_code("empty_answer", preset, &stderr_detail, &answer),
                    stderr_detail,
                );
            } else if state.query.set_status(query_pass_id, QueryStatus::Ready) {
                let clipboard_error = if !may_claim_clipboard(
                    clipboard_generation,
                    crate::injector::clipboard_write_generation(),
                ) {
                    // Something else claimed the clipboard while the answer was
                    // generating. Defer to it: the answer stays in the popover
                    // behind Copy, rather than silently replacing text the user
                    // may already have pasted somewhere.
                    Some("clipboard_superseded")
                } else {
                    crate::injector::write_clipboard_text(&answer)
                        .err()
                        .map(|_| "clipboard_unavailable")
                };
                let _ = crate::commands::query_popover::set_expanded_internal(&app_handle, true);
                emit_state(
                    &app_handle,
                    query_pass_id,
                    QueryStatus::Ready,
                    clipboard_error,
                );
            }
        }
        Err("cancelled") => {}
        Err(error_code) => {
            let answer = state.query.answer(query_pass_id).unwrap_or_default();
            fail_query_with_detail(
                &app_handle,
                &state,
                query_pass_id,
                actionable_error_code(error_code, preset, &stderr_detail, &answer),
                stderr_detail,
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_query(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, crate::State>,
    query_pass_id: u64,
) -> Result<(), String> {
    if state.query.active_pass_id() != Some(query_pass_id) {
        return Ok(());
    }
    state.query.mark_cancelled(query_pass_id);
    crate::keyboard::set_query_recording_state(false);
    if matches!(
        state.query.status(),
        QueryStatus::Connecting | QueryStatus::Listening
    ) {
        let _ = crate::audio_lifecycle::cancel_query_capture(
            query_pass_id,
            crate::audio_lifecycle::AudioCancelReason::User,
        );
    }
    if !state.query.terminate_child(query_pass_id) {
        *state.query.status.lock_or_recover() = QueryStatus::Failed;
        emit_state(
            &app_handle,
            query_pass_id,
            QueryStatus::Failed,
            Some("termination_unconfirmed"),
        );
        return Err("The configured CLI could not be confirmed terminated.".to_string());
    }
    // A terminal pass can be superseded by a new hotkey while this command is
    // awaiting teardown. Never clear or hide the newer owner's session.
    if state.query.active_pass_id() != Some(query_pass_id) {
        return Ok(());
    }
    *state.query.session.lock_or_recover() = None;
    *state.query.status.lock_or_recover() = QueryStatus::Idle;
    state.query.clear_pass(query_pass_id);
    let _ = crate::commands::query_popover::hide_internal(&app_handle);
    let _ = app_handle.emit("query-review-hidden", ());
    Ok(())
}

#[tauri::command]
pub(crate) fn copy_query_answer(
    state: tauri::State<'_, crate::State>,
    query_pass_id: u64,
) -> Result<(), String> {
    if state.query.active_pass_id() != Some(query_pass_id)
        || state.query.status() != QueryStatus::Ready
    {
        return Err("That query answer is no longer available.".to_string());
    }
    let answer = state
        .query
        .answer(query_pass_id)
        .filter(|answer| !answer.is_empty())
        .ok_or_else(|| "That query answer is no longer available.".to_string())?;
    crate::injector::write_clipboard_text(&answer)
}

#[tauri::command]
pub(crate) fn get_query_review_content(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::State>,
) -> QueryReviewContent {
    if window.label() != "query-review" {
        return QueryReviewContent::default();
    }
    let Some(query_pass_id) = state.query.active_pass_id() else {
        return QueryReviewContent::default();
    };
    let Some(session) = state.query.session(query_pass_id) else {
        return QueryReviewContent {
            query_pass_id: Some(query_pass_id),
            ..QueryReviewContent::default()
        };
    };
    QueryReviewContent {
        query_pass_id: Some(query_pass_id),
        answer: session.answer,
        error_detail: session.error_detail,
        sign_in: session.command.preset.map(|preset| QuerySignIn {
            provider: preset.label,
            hint: preset.login_hint,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(executable: &str, preset_id: Option<&str>) -> QueryCommandConfig {
        QueryCommandConfig {
            executable: executable.into(),
            arguments: vec!["%s".into()],
            timeout_seconds: 60,
            preset_id: preset_id.map(str::to_string),
        }
    }

    #[test]
    fn validates_only_absolute_executable_and_bounded_fixed_arguments() {
        assert_eq!(
            validate_command(config("claude", None), Vec::new()).unwrap_err(),
            "invalid_executable"
        );

        let valid = validate_command(config("/usr/bin/printf", None), Vec::new())
            .expect("printf must be executable");
        assert_eq!(valid.arguments, vec!["%s"]);
        assert!(valid.preset.is_none());
        assert!(valid.environment.is_empty());
    }

    #[test]
    fn resolves_a_known_preset_and_refuses_an_unknown_one() {
        let claude = validate_command(config("/usr/bin/printf", Some("claude")), Vec::new())
            .expect("printf must be executable");
        assert_eq!(claude.preset.map(|preset| preset.id), Some("claude"));

        // "custom" and an absent id both mean "assembled by hand".
        for id in [Some("custom"), Some(""), None] {
            let command = validate_command(config("/usr/bin/printf", id), Vec::new()).unwrap();
            assert!(command.preset.is_none(), "{id:?}");
        }
        assert_eq!(
            validate_command(config("/usr/bin/printf", Some("gemini")), Vec::new()).unwrap_err(),
            "invalid_preset"
        );
    }

    #[test]
    fn declared_environment_rides_along_with_the_validated_command() {
        let declared = vec![("CLAUDE_CONFIG_DIR".to_string(), "/tmp/cfg".to_string())];
        let command =
            validate_command(config("/usr/bin/printf", Some("claude")), declared.clone()).unwrap();
        assert_eq!(command.environment, declared);
    }

    #[test]
    fn a_failed_run_that_says_signed_out_becomes_an_actionable_code() {
        let claude = crate::query_presets::preset("claude");
        assert_eq!(
            actionable_error_code("exit_nonzero", claude, "Error: Not logged in", ""),
            "provider_not_authenticated"
        );
        // The incident shape: the CLI printed its refusal on stdout.
        assert_eq!(
            actionable_error_code("exit_nonzero", claude, "", "Not logged in"),
            "provider_not_authenticated"
        );
        // An empty answer with an auth complaint is the same failure.
        assert_eq!(
            actionable_error_code("empty_answer", claude, "invalid API key", ""),
            "provider_not_authenticated"
        );
        // Unrelated failures keep their own code…
        assert_eq!(
            actionable_error_code("exit_nonzero", claude, "rate limit exceeded", ""),
            "exit_nonzero"
        );
        // …and codes that describe Murmur's own bounds are never reinterpreted.
        assert_eq!(
            actionable_error_code("timed_out", claude, "not logged in", ""),
            "timed_out"
        );
        assert_eq!(
            actionable_error_code("termination_unconfirmed", claude, "not logged in", ""),
            "termination_unconfirmed"
        );
    }

    #[test]
    fn the_stderr_tail_keeps_the_end_and_marks_what_it_dropped() {
        let mut tail = StderrTail::default();
        tail.push(b"early noise\n");
        assert_eq!(tail.text(), "early noise");

        tail.push(&vec![b'x'; MAX_STDERR_TAIL_BYTES]);
        tail.push(b"\nfatal: not logged in\n");
        let text = tail.text();
        assert!(text.starts_with('…'), "truncation must be visible");
        assert!(text.ends_with("fatal: not logged in"), "{text}");
        assert!(!text.contains("early noise"), "the head is what is dropped");
        assert!(text.len() <= MAX_STDERR_TAIL_BYTES + 8);

        // A tail cut mid-scalar must not surface a stray replacement char.
        let mut split = StderrTail::default();
        split.push(&vec![b'y'; MAX_STDERR_TAIL_BYTES]);
        split.push("🦀 done".as_bytes());
        assert!(split.text().ends_with("done"));
        assert!(!split.text().starts_with('\u{fffd}'));
    }

    #[test]
    fn answer_defers_to_a_clipboard_write_made_while_it_was_generating() {
        // Nothing else wrote: the answer copies itself as usual.
        assert!(may_claim_clipboard(7, 7));
        // A dictation (or transform) landed mid-run — leave its text in place.
        assert!(!may_claim_clipboard(7, 8));
    }

    #[test]
    fn running_frees_capture_but_still_blocks_heavy_runtimes() {
        // Capture-only work may start once the answer is generating: the audio
        // owner is stopped and joined before `Transcribing`, and ASR finishes
        // before `Running`.
        assert!(!QueryStatus::Running.blocks_capture());
        assert!(QueryStatus::Running.blocks_pipeline());

        for status in [
            QueryStatus::Connecting,
            QueryStatus::Listening,
            QueryStatus::Transcribing,
        ] {
            assert!(status.blocks_capture(), "{status:?} owns mic or ASR");
            assert!(status.blocks_pipeline(), "{status:?} must stay strict");
        }

        for status in [QueryStatus::Idle, QueryStatus::Ready, QueryStatus::Failed] {
            assert!(!status.blocks_capture(), "{status:?} is terminal");
            assert!(!status.blocks_pipeline(), "{status:?} is terminal");
        }
    }

    #[test]
    fn query_pass_ids_are_monotonic_and_inflight_passes_cannot_be_replaced() {
        let query = QueryCoordinator::default();
        let first = query.allocate_keyboard_pass().unwrap();
        assert_eq!(first, 1);
        query.set_status(first, QueryStatus::Running);
        assert_eq!(query.allocate_keyboard_pass(), None);
        query.set_status(first, QueryStatus::Ready);
        let second = query.allocate_keyboard_pass().unwrap();
        assert_eq!(second, 2);
        assert!(!query.set_status(first, QueryStatus::Failed));
        assert_eq!(query.active_pass_id(), Some(second));
    }

    #[test]
    fn answer_cap_rejects_unbounded_growth() {
        let query = QueryCoordinator::default();
        let pass_id = query.allocate_keyboard_pass().unwrap();
        let context = Arc::new(crate::dictation_context::resolve(
            crate::dictation_context::ResolverInputs {
                bundle_id: None,
                global: &crate::state::DictationState::default(),
                prompt: None,
                correction_matcher: None,
                ide_context_index: None,
                vocabulary_version: 0,
                voice_commands: None,
                session_overrides: crate::dictation_context::SessionOverrides::default(),
            },
        ));
        query.install_session(
            pass_id,
            QuerySession {
                pass_id,
                context,
                command: ValidatedQueryCommand {
                    executable: PathBuf::from("/usr/bin/printf"),
                    arguments: vec![],
                    timeout: Duration::from_secs(5),
                    environment: Vec::new(),
                    preset: None,
                },
                answer: "a".repeat(MAX_ANSWER_BYTES),
                error_detail: None,
            },
        );
        assert_eq!(query.append_answer(pass_id, "b"), Err("output_too_large"));
        query.set_status(pass_id, QueryStatus::Ready);
        let next_pass_id = query.allocate_keyboard_pass().unwrap();
        assert_eq!(next_pass_id, pass_id + 1);
        assert_eq!(query.answer(next_pass_id), None);
    }

    #[test]
    fn utf8_decoder_preserves_split_scalar_values() {
        let mut decoder = Utf8Chunks::new();
        let bytes = "hello 🦀".as_bytes();
        assert_eq!(decoder.push(&bytes[..8]), "hello ");
        assert_eq!(decoder.push(&bytes[8..]), "🦀");
        assert_eq!(decoder.finish(), "");
    }
}
