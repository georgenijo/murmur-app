//! Host-side supervisor for the signed local-LLM sidecar (#312).
//!
//! The app links **only** `murmur-local-llm-protocol`; it never links
//! `llama-cpp-2`. This module owns the child helper process lifecycle: it
//! verifies the pinned model file, spawns the helper with an empty environment
//! and the model handed over as inherited read-only fd 3, drives protocol v1
//! over piped stdin/stdout, and enforces every app-side limit, deadline,
//! cancellation, crash circuit-breaker, RSS ceiling, and idle-unload rule from
//! the ADR (docs/decisions/2026-07-20-signed-local-llm-sidecar.md).
//!
//! Privacy: no instruction / input / output text ever reaches logs, telemetry,
//! or error strings. Only durations, bucketed sizes, token counts, and stable
//! enums are recorded.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use murmur_local_llm_protocol::{ErrorCode, FinishReason};

// ---------------------------------------------------------------------------
// Per-request cooperative cancel token (#312 C2 follow-up, item 11).
// ---------------------------------------------------------------------------

/// A cooperative cancel signal scoped to exactly ONE transform request.
///
/// This replaces the earlier supervisor-wide `cancel_requested` flag, which was
/// reset at each request start (`self.cancel_requested.store(false)`) and could
/// therefore have a legitimate cancel WIPED by the next request's reset — or,
/// symmetrically, leak a stale cancel into the next request. A fresh token is
/// created per request in `transform_flow::run_transform` (before the spawn)
/// and registered for the request's lifetime; canceling it can only ever affect
/// the exact request it was created for. Cheap to clone (a shared
/// `Arc<AtomicBool>`).
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh, un-cancelled token.
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Request cooperative cancellation for this request.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested for this request.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Identity check: clones of the same token compare equal; distinct tokens
    /// do not. Lets the in-flight slot be cleared only when it still holds our
    /// token (a later request may already have replaced it).
    fn is_same(&self, other: &CancelToken) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

// ---------------------------------------------------------------------------
// Immutable catalog pins (single source of truth, shared with the installer).
// ---------------------------------------------------------------------------

/// Stable catalog identifier for the only v1 transform model.
pub const TRANSFORM_MODEL_ID: &str = "qwen2.5-1.5b-instruct-q4_k_m";
/// Model filename as published by the pinned Hugging Face revision.
pub const TRANSFORM_MODEL_FILENAME: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
/// Exact byte size the download and the pre-spawn verifier both enforce.
pub const TRANSFORM_MODEL_SIZE_BYTES: u64 = 1_117_320_736;
/// Lowercase hex SHA-256 the download and the pre-spawn verifier both enforce.
pub const TRANSFORM_MODEL_SHA256: &str =
    "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e";
/// Immutable Hugging Face repository revision the model is fetched from.
pub const TRANSFORM_MODEL_REVISION: &str = "dd26da440ef0330c47919d1ecae0966d24022222";
/// Compile-time download URL pinned to the immutable revision above.
pub const TRANSFORM_MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/dd26da440ef0330c47919d1ecae0966d24022222/qwen2.5-1.5b-instruct-q4_k_m.gguf";

/// Executable base name of the signed helper, as an `externalBin` and in dev.
pub const HELPER_BIN_NAME: &str = "murmur-llm-sidecar";

/// RSS ceiling below which the helper runs unremarked (2 GiB).
const RSS_WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// RSS ceiling above which the helper is force-killed (3 GiB).
const RSS_KILL_BYTES: u64 = 3 * 1024 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Public error / output types (portable across all targets).
// ---------------------------------------------------------------------------

/// Successful transform result: text plus bounded metadata only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformOutput {
    pub output: String,
    pub finish_reason: FinishReason,
    pub output_tokens: u32,
}

/// Content-free timing metadata returned only by the correlated transform
/// entry point. `None` means that phase was never entered, not a synthetic
/// numeric zero.
pub struct CorrelatedTransformOutcome {
    pub result: Result<TransformOutput, TransformError>,
    pub spawn_load_ms: Option<u64>,
    pub generation_ms: Option<u64>,
    pub cache_hit: Option<bool>,
    pub diagnostics: SidecarDiagnostics,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SidecarDiagnostics {
    pub host_model_verification_ms: Option<u64>,
    pub helper_spawn_ms: Option<u64>,
    pub helper_model_verification_ms: Option<u64>,
    pub backend_initialization_ms: Option<u64>,
    pub model_load_ms: Option<u64>,
    pub ready_handshake_ms: Option<u64>,
    pub request_receipt_ms: Option<u64>,
    pub first_token_ms: Option<u64>,
    pub process_exit_code: Option<i32>,
    pub process_exit_signal: Option<i32>,
    pub failure_phase: Option<SidecarDiagnosticPhase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarDiagnosticPhase {
    HostModelVerification,
    HelperSpawn,
    HelperModelVerification,
    BackendInitialization,
    ModelLoad,
    ReadyHandshake,
    RequestReceipt,
    FirstToken,
    Generation,
}

impl SidecarDiagnosticPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostModelVerification => "host_model_verification",
            Self::HelperSpawn => "helper_spawn",
            Self::HelperModelVerification => "helper_model_verification",
            Self::BackendInitialization => "backend_initialization",
            Self::ModelLoad => "model_load",
            Self::ReadyHandshake => "ready_handshake",
            Self::RequestReceipt => "request_receipt",
            Self::FirstToken => "first_token",
            Self::Generation => "generation",
        }
    }
}

impl CorrelatedTransformOutcome {
    fn before_runtime(error: TransformError) -> Self {
        Self {
            result: Err(error),
            spawn_load_ms: None,
            generation_ms: None,
            cache_hit: None,
            diagnostics: SidecarDiagnostics::default(),
        }
    }
}

/// Stable supervisor error enum. Mirrors the protocol `ErrorCode` and adds
/// supervisor-level variants. Every `Display` string is a fixed label — it
/// never embeds instruction, input, output, model path, or raw stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    /// Local-LLM runtime is not supported on this platform.
    Unsupported,
    /// The pinned transform model is not installed.
    NotDownloaded,
    /// The circuit breaker has disabled the runtime until an explicit reset.
    Disabled,
    /// A transform is already in flight (one-in-flight rule).
    Busy,
    /// Instruction / input / deadline exceeded an app-side limit.
    InvalidRequest,
    /// A heavy ASR runtime (recording / benchmark / file transcription) is active.
    HeavyRuntimeActive,
    /// The helper executable could not be launched.
    SpawnFailed,
    /// The hello/ready handshake failed (nonce, protocol, or model mismatch).
    HandshakeFailed,
    /// The installed model file failed size / SHA-256 / regular-file checks —
    /// its content is wrong, so it is removed and re-download is required.
    ModelMismatch,
    /// The model file could not be read (open / metadata / read I/O error). A
    /// transient failure that fails closed WITHOUT deleting the model.
    ModelUnreadable,
    /// The deadline elapsed; the helper was cancelled and killed if unresponsive.
    Timeout,
    /// The request was cancelled cooperatively.
    Cancelled,
    /// The helper process died unexpectedly.
    Crashed,
    /// The helper returned output that violated protocol invariants.
    OutputInvalid,
    /// The helper spoke a malformed or out-of-contract frame.
    Protocol,
    /// The helper reported a resource-limit failure.
    ResourceLimit,
    /// An internal supervisor fault.
    Internal,
}

impl TransformError {
    /// Stable machine label for telemetry (no free-form text).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::NotDownloaded => "notDownloaded",
            Self::Disabled => "disabled",
            Self::Busy => "busy",
            Self::InvalidRequest => "invalidRequest",
            Self::HeavyRuntimeActive => "heavyRuntimeActive",
            Self::SpawnFailed => "spawnFailed",
            Self::HandshakeFailed => "handshakeFailed",
            Self::ModelMismatch => "modelMismatch",
            Self::ModelUnreadable => "modelUnreadable",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Crashed => "crashed",
            Self::OutputInvalid => "outputInvalid",
            Self::Protocol => "protocol",
            Self::ResourceLimit => "resourceLimit",
            Self::Internal => "internal",
        }
    }

    /// Map a protocol `ErrorCode` reported by a healthy helper onto the stable
    /// supervisor enum.
    fn from_helper_code(code: ErrorCode) -> Self {
        match code {
            ErrorCode::DeadlineExceeded => Self::Timeout,
            ErrorCode::Cancelled => Self::Cancelled,
            ErrorCode::OutputInvalid => Self::OutputInvalid,
            ErrorCode::ResourceLimit => Self::ResourceLimit,
            ErrorCode::Busy => Self::Busy,
            ErrorCode::ModelMismatch | ErrorCode::ModelLoadFailed => Self::ModelMismatch,
            ErrorCode::InvalidFrame | ErrorCode::InvalidMessage | ErrorCode::ProtocolMismatch => {
                Self::Protocol
            }
            ErrorCode::RuntimeUnavailable | ErrorCode::Internal => Self::Internal,
        }
    }
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for TransformError {}

/// Bounded size bucket for telemetry. Never records exact lengths.
pub fn size_bucket(len: usize) -> &'static str {
    match len {
        0 => "0",
        1..=256 => "le256",
        257..=1024 => "le1k",
        1025..=4096 => "le4k",
        4097..=16384 => "le16k",
        _ => "gt16k",
    }
}

// ---------------------------------------------------------------------------
// Mutual-exclusion bridge into the ASR runtime (injected by the host).
// ---------------------------------------------------------------------------

/// The supervisor is decoupled from Tauri via this bridge so it stays unit
/// testable. The production implementation lives in `lib.rs` and forwards into
/// `AppState` / `ModelRuntimeManager`.
pub trait HostGuard: Send + Sync {
    /// Return `Some(reason)` when a heavy ASR runtime is active and a transform
    /// must be refused. Reason is a stable enum label for telemetry.
    fn heavy_runtime_active(&self) -> Option<&'static str>;
    /// Release the ASR model (via the existing `MemoryPressure` unload path)
    /// before the helper spawns, so only one heavy runtime is ever resident.
    fn release_asr(&self);
}

/// Default no-op bridge (tests, and before the host installs the real one).
struct NoopHostGuard;
impl HostGuard for NoopHostGuard {
    fn heavy_runtime_active(&self) -> Option<&'static str> {
        None
    }
    fn release_asr(&self) {}
}

// ---------------------------------------------------------------------------
// Model path helpers + pre-spawn verification (portable / unix).
// ---------------------------------------------------------------------------

/// Root of the hash-versioned transform-model store, beneath the app models dir.
pub fn transform_models_root() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("local-dictation")
            .join("models")
            .join("transform-llm")
    })
}

/// Absolute path the installed, verified model publishes to.
pub fn installed_model_path() -> Option<PathBuf> {
    Some(
        transform_models_root()?
            .join(TRANSFORM_MODEL_SHA256)
            .join(TRANSFORM_MODEL_FILENAME),
    )
}

/// Stream a file and return `(size_bytes, sha256_hex)`. Used by the installer
/// and by tests to derive pins for a fixture model.
pub fn model_file_digest(path: &Path) -> std::io::Result<(u64, String)> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut size = 0_u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

/// Open the model with `O_NOFOLLOW`, verify it is a regular file of exactly
/// `expected_size` bytes hashing to `expected_sha` (lowercase hex), then rewind
/// it to offset 0 so the inherited fd 3 starts at the beginning. Returns the
/// open, verified, rewound handle ready to be handed to the child.
#[cfg(unix)]
fn open_and_verify_model(
    path: &Path,
    expected_size: u64,
    expected_sha: &str,
    cancel: &CancelToken,
) -> Result<std::fs::File, TransformError> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::fs::OpenOptionsExt;

    // I/O failures (open incl. O_NOFOLLOW ELOOP, metadata, read, seek) are
    // transient/environmental → ModelUnreadable (fail closed, keep the model).
    // Only a wrong size / hash / non-regular file is a content mismatch →
    // ModelMismatch (the caller removes it so it can be re-downloaded).
    if cancel.is_cancelled() {
        return Err(TransformError::Cancelled);
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| TransformError::ModelUnreadable)?;

    let metadata = file
        .metadata()
        .map_err(|_| TransformError::ModelUnreadable)?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(TransformError::ModelMismatch);
    }

    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(TransformError::Cancelled);
        }
        let n = file
            .read(&mut buffer)
            .map_err(|_| TransformError::ModelUnreadable)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    if cancel.is_cancelled() {
        return Err(TransformError::Cancelled);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha {
        return Err(TransformError::ModelMismatch);
    }

    file.seek(SeekFrom::Start(0))
        .map_err(|_| TransformError::ModelUnreadable)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_and_verify_model(
    _path: &Path,
    _expected_size: u64,
    _expected_sha: &str,
    _cancel: &CancelToken,
) -> Result<std::fs::File, TransformError> {
    Err(TransformError::Unsupported)
}

// ---------------------------------------------------------------------------
// Crash circuit breaker (pure, unit tested).
// ---------------------------------------------------------------------------

/// Three faults inside a rolling ten-minute window disable the runtime until an
/// explicit `reset_transform_runtime`.
#[derive(Default)]
struct Breaker {
    failures: Vec<std::time::Instant>,
    /// Latched once the threshold is crossed; only `reset` clears it.
    disabled: bool,
}

const BREAKER_WINDOW: Duration = Duration::from_secs(10 * 60);
const BREAKER_THRESHOLD: usize = 3;

impl Breaker {
    fn record_failure(&mut self, now: std::time::Instant) {
        self.failures
            .retain(|t| now.duration_since(*t) < BREAKER_WINDOW);
        self.failures.push(now);
        if self.failures.len() >= BREAKER_THRESHOLD {
            self.disabled = true;
        }
    }

    fn is_disabled(&mut self, now: std::time::Instant) -> bool {
        self.failures
            .retain(|t| now.duration_since(*t) < BREAKER_WINDOW);
        self.disabled
    }

    fn reset(&mut self) {
        self.failures.clear();
        self.disabled = false;
    }
}

// ---------------------------------------------------------------------------
// RSS policy (pure, unit tested).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RssAction {
    Ok,
    Warn,
    Kill,
}

fn rss_action(bytes: u64) -> RssAction {
    if bytes >= RSS_KILL_BYTES {
        RssAction::Kill
    } else if bytes >= RSS_WARN_BYTES {
        RssAction::Warn
    } else {
        RssAction::Ok
    }
}

// ---------------------------------------------------------------------------
// Spawn plan: production pins/paths vs. an explicit test configuration.
// ---------------------------------------------------------------------------

/// Explicit configuration used by integration tests to drive the supervisor
/// against the protocol mock helper. Not part of any stable external API.
/// `allow(dead_code)`: in a release build without `llm-test-support`, `for_test`
/// is compiled out so these fields are never read there.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TestSpawnConfig {
    pub helper_path: PathBuf,
    pub model_path: PathBuf,
    pub model_size: u64,
    pub model_sha256: String,
    /// Scenario env vars for the mock. The production path always `env_clear`s
    /// with no extra vars; this exists solely so tests can steer the mock.
    pub scenario_env: Vec<(String, String)>,
    pub request_slack: Duration,
    pub cancel_grace: Duration,
    pub handshake_timeout: Duration,
    pub idle_after: Duration,
}

enum SpawnPlan {
    Production,
    // Only constructed by `for_test`, which is compiled out of release builds.
    #[allow(dead_code)]
    Test(TestSpawnConfig),
}

impl SpawnPlan {
    fn cancel_grace(&self) -> Duration {
        match self {
            Self::Production => Duration::from_millis(250),
            Self::Test(c) => c.cancel_grace,
        }
    }
    fn request_slack(&self) -> Duration {
        match self {
            Self::Production => Duration::from_millis(1000),
            Self::Test(c) => c.request_slack,
        }
    }
    fn handshake_timeout(&self) -> Duration {
        match self {
            Self::Production => Duration::from_secs(30),
            Self::Test(c) => c.handshake_timeout,
        }
    }
    fn idle_after(&self) -> Duration {
        match self {
            Self::Production => Duration::from_secs(5 * 60),
            Self::Test(c) => c.idle_after,
        }
    }
    // Only referenced by the equally-gated env-injection loop; compiled out of
    // release builds so the scenario-env path is not present at all there.
    #[cfg(any(debug_assertions, feature = "llm-test-support"))]
    fn scenario_env(&self) -> &[(String, String)] {
        match self {
            Self::Production => &[],
            Self::Test(c) => &c.scenario_env,
        }
    }
    fn pins(&self) -> (u64, &str) {
        match self {
            Self::Production => (TRANSFORM_MODEL_SIZE_BYTES, TRANSFORM_MODEL_SHA256),
            Self::Test(c) => (c.model_size, c.model_sha256.as_str()),
        }
    }
    fn helper_path(&self) -> Result<PathBuf, TransformError> {
        match self {
            Self::Production => resolve_helper_path(),
            Self::Test(c) => Ok(c.helper_path.clone()),
        }
    }
    fn model_path(&self) -> Result<PathBuf, TransformError> {
        match self {
            Self::Production => {
                let path = installed_model_path().ok_or(TransformError::NotDownloaded)?;
                if path.is_file() {
                    Ok(path)
                } else {
                    Err(TransformError::NotDownloaded)
                }
            }
            Self::Test(c) => Ok(c.model_path.clone()),
        }
    }

    /// Remove a corrupt / wrong-content published model so status reports
    /// `not_downloaded` and the user can re-download. Production only — a test
    /// fixture model is never touched.
    fn cleanup_bad_model(&self) {
        if let Self::Production = self {
            if let Some(dir) = transform_models_root().map(|r| r.join(TRANSFORM_MODEL_SHA256)) {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }
}

/// Resolve the exact helper executable. Production resolves ONLY the bundled
/// `externalBin` sitting next to the app binary (`Contents/MacOS/…`), honoring
/// the ADR requirement to start the exact pinned nested executable — no `..`
/// probing that could reach an unpinned path. A debug-only fallback covers the
/// dev layout where the app ran from an unusual cwd.
///
/// NOTE: path pinning is necessary but not sufficient. Before shipping, the
/// packaging integration (#312 PR-A3/C2) must additionally validate the helper
/// against its fixed designated requirement via `SecStaticCodeCheckValidity`
/// (ADR threat table "Helper replacement" row). See `spawn_and_handshake`.
fn resolve_helper_path() -> Result<PathBuf, TransformError> {
    let exe = std::env::current_exe().map_err(|_| TransformError::SpawnFailed)?;
    let dir = exe.parent().ok_or(TransformError::SpawnFailed)?;
    // Packaged (Contents/MacOS/murmur-llm-sidecar) and the common dev layout
    // (target/<profile>/murmur-llm-sidecar) both put the helper next to the app.
    let primary = dir.join(HELPER_BIN_NAME);
    if primary.is_file() {
        return Ok(primary);
    }
    // Dev-only fallback: probe sibling target profiles. Never compiled into a
    // release binary, so a packaged app resolves only the pinned sibling path.
    #[cfg(debug_assertions)]
    for profile in ["debug", "release"] {
        let candidate = dir.join("..").join(profile).join(HELPER_BIN_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(TransformError::SpawnFailed)
}

// ---------------------------------------------------------------------------
// Unique per-process identifiers (no external crate).
// ---------------------------------------------------------------------------

fn unique_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{nanos}-{n}", std::process::id())
}

// ===========================================================================
// Supported platform: full supervisor.
// ===========================================================================

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod supported {
    use super::*;
    use murmur_local_llm_protocol::{
        read_frame, write_frame, FrameError, HelperMessage, HostMessage,
        HostMessage::Cancel as HostCancel, ModelIdentity, ProtocolLimits, MAX_DEADLINE_MS,
        MAX_INPUT_BYTES, MAX_INSTRUCTION_BYTES, MAX_OUTPUT_BYTES, MAX_OUTPUT_TOKENS, PROTOCOL_NAME,
        PROTOCOL_VERSION,
    };
    use std::sync::mpsc::{Receiver, RecvTimeoutError};
    use std::sync::Mutex;
    use std::time::Instant;

    /// Events surfaced by the stdout reader thread.
    enum HelperEvent {
        Frame(HelperMessage),
        /// Clean EOF / process exit (incomplete header on a closed pipe).
        Exited,
        /// A malformed, oversized, or truncated frame — fail closed and kill.
        ProtocolViolation,
    }

    struct Child {
        proc: std::process::Child,
        stdin: std::process::ChildStdin,
        events: Receiver<HelperEvent>,
        session_nonce: String,
        pid: u32,
        /// Keeps the verified model fd alive for the process lifetime.
        _model_file: std::fs::File,
    }

    struct SpawnPidGuard<'a> {
        resident_pid: &'a AtomicU32,
        keep: bool,
    }

    impl<'a> SpawnPidGuard<'a> {
        fn new(resident_pid: &'a AtomicU32, pid: u32) -> Self {
            resident_pid.store(pid, Ordering::Release);
            Self {
                resident_pid,
                keep: false,
            }
        }

        fn keep(mut self) {
            self.keep = true;
        }
    }

    impl Drop for SpawnPidGuard<'_> {
        fn drop(&mut self) {
            if !self.keep {
                self.resident_pid.store(0, Ordering::Release);
            }
        }
    }

    struct Inner {
        child: Option<Child>,
        breaker: Breaker,
        last_activity: Instant,
        /// Latched once we log an RSS warning, so the reaper warns once per
        /// crossing rather than every 30s tick. Cleared when RSS drops back.
        rss_warned: bool,
    }

    /// RAII guard that clears the in-flight `busy` flag AND the per-request
    /// cancel slot on drop. It is moved into the `spawn_blocking` closure so
    /// both are released when the blocking work finishes on EVERY path —
    /// including when the async future is dropped (e.g. a `tokio::time::timeout`
    /// wrapper) mid-request. A blocking task is never cancelled, so its guard
    /// always drops.
    struct BusyGuard {
        sidecar: Arc<LlmSidecar>,
        cancel: CancelToken,
    }
    impl Drop for BusyGuard {
        fn drop(&mut self) {
            // Clear our per-request cancel slot (only if it still holds our
            // token) BEFORE releasing busy, so a cancel arriving after busy
            // frees can never land on the wrong (already-finished) request.
            self.sidecar.clear_inflight_cancel(&self.cancel);
            self.sidecar.busy.store(false, Ordering::Release);
        }
    }

    pub struct LlmSidecar {
        plan: SpawnPlan,
        host_guard: OnceLock<Arc<dyn HostGuard>>,
        busy: AtomicBool,
        /// PID of the resident helper, including while its startup handshake
        /// is loading the model. Kept atomic because `transform_blocking`
        /// holds `inner` across the request while the resource sampler must
        /// remain non-blocking.
        resident_pid: AtomicU32,
        /// The in-flight request's cancel token, registered for the duration of
        /// one `transform` call and cleared by its `BusyGuard`. Per-request (not
        /// supervisor-wide): [`Self::cancel_inflight_request`] cancels ONLY the
        /// token currently in flight, so a cancel can never wipe or leak into a
        /// neighbouring request. `None` between requests → cancel is a no-op.
        inflight_cancel: Mutex<Option<CancelToken>>,
        inner: Mutex<Inner>,
    }

    impl LlmSidecar {
        fn record_process_exit(
            proc: &mut std::process::Child,
            diagnostics: &mut SidecarDiagnostics,
        ) {
            // EOF can beat waitpid visibility by a few scheduler ticks. Poll
            // briefly without ever turning diagnostics into an unbounded wait.
            for _ in 0..20 {
                if let Ok(Some(status)) = proc.try_wait() {
                    diagnostics.process_exit_code = status.code();
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        diagnostics.process_exit_signal = status.signal();
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        pub fn new() -> Self {
            Self::with_plan(SpawnPlan::Production)
        }

        /// Test-only constructor allowing scenario env injection into the child.
        /// Excluded from release builds (`debug_assertions` off, feature off) so
        /// the env-injection path is not linkable in a shipped binary.
        #[cfg(any(debug_assertions, feature = "llm-test-support"))]
        pub fn for_test(config: TestSpawnConfig) -> Self {
            Self::with_plan(SpawnPlan::Test(config))
        }

        fn with_plan(plan: SpawnPlan) -> Self {
            Self {
                plan,
                host_guard: OnceLock::new(),
                busy: AtomicBool::new(false),
                resident_pid: AtomicU32::new(0),
                inflight_cancel: Mutex::new(None),
                inner: Mutex::new(Inner {
                    child: None,
                    breaker: Breaker::default(),
                    last_activity: Instant::now(),
                    rss_warned: false,
                }),
            }
        }

        pub fn set_host_guard(&self, guard: Arc<dyn HostGuard>) {
            let _ = self.host_guard.set(guard);
        }

        fn guard(&self) -> &dyn HostGuard {
            match self.host_guard.get() {
                Some(g) => g.as_ref(),
                None => &NoopHostGuard,
            }
        }

        fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
            self.inner.lock().unwrap_or_else(|p| p.into_inner())
        }

        /// True while a transform is in flight. Recording paths guard on this so
        /// ASR never starts over a resident transform runtime.
        pub fn is_transform_busy(&self) -> bool {
            self.busy.load(Ordering::Acquire)
        }

        /// True once the circuit breaker has latched disabled after repeated
        /// faults (cleared only by `reset`). Surfaced in the model status so the
        /// settings UI can show the Reset button + notice only when it matters
        /// (#312 D1 round-2 finding 7).
        pub fn runtime_disabled(&self) -> bool {
            self.lock().breaker.disabled
        }

        /// Request cooperative cancel of the in-flight transform (if any).
        ///
        /// Cancels ONLY the request currently in flight — it flips that
        /// request's own [`CancelToken`], never a shared flag — so a cancel can
        /// never affect the next request. The blocking `run_request` loop
        /// observes the token, sends a protocol Cancel frame, and settles via
        /// the same cancel-then-kill dance used for deadline expiry. Dropping
        /// the outer async future alone does **not** clear `busy` — only the
        /// blocking work finishing does — so callers that abort a
        /// `tokio::spawn` wrapper must also call this (see `cancel_transform`).
        /// A no-op when no request is in flight (`inflight_cancel` is `None`).
        pub fn cancel_inflight_request(&self) {
            if let Some(token) = self
                .inflight_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                token.cancel();
            }
        }

        /// Register the in-flight request's cancel token (called once per
        /// request, before the blocking work starts).
        fn set_inflight_cancel(&self, token: CancelToken) {
            *self
                .inflight_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(token);
        }

        /// Clear the in-flight slot, but only if it still holds `token` — a
        /// later request may already have replaced it.
        fn clear_inflight_cancel(&self, token: &CancelToken) {
            let mut slot = self
                .inflight_cancel
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if slot.as_ref().map(|t| t.is_same(token)).unwrap_or(false) {
                *slot = None;
            }
        }

        /// Test-support: whether a helper process is currently resident. Not
        /// part of any stable external API.
        pub fn has_live_child(&self) -> bool {
            self.resident_pid().is_some()
        }

        /// Non-blocking resource-attribution seam. PID 0 is never a child PID.
        pub fn resident_pid(&self) -> Option<u32> {
            match self.resident_pid.load(Ordering::Acquire) {
                0 => None,
                pid => Some(pid),
            }
        }

        fn take_child(&self, inner: &mut Inner) -> Option<Child> {
            let child = inner.child.take();
            if child.is_some() {
                self.resident_pid.store(0, Ordering::Release);
            }
            child
        }

        /// Async transform facade. Serializes to one in-flight request via a
        /// busy flag (queue-reject: a second concurrent call gets `Busy`).
        pub async fn transform(
            self: &Arc<Self>,
            instruction: &str,
            input: &str,
            deadline: Duration,
            cancel: CancelToken,
        ) -> Result<TransformOutput, TransformError> {
            self.transform_inner(instruction, input, deadline, cancel, None)
                .await
                .result
        }

        /// Correlated transform entry point for the selected-text flow. The
        /// existing uncorrelated method remains for isolated protocol tests.
        pub async fn transform_for_pass(
            self: &Arc<Self>,
            transform_pass_id: u64,
            instruction: &str,
            input: &str,
            deadline: Duration,
            cancel: CancelToken,
        ) -> CorrelatedTransformOutcome {
            self.transform_inner(
                instruction,
                input,
                deadline,
                cancel,
                Some(transform_pass_id),
            )
            .await
        }

        async fn transform_inner(
            self: &Arc<Self>,
            instruction: &str,
            input: &str,
            deadline: Duration,
            cancel: CancelToken,
            transform_pass_id: Option<u64>,
        ) -> CorrelatedTransformOutcome {
            // App-side limit enforcement (defence in depth over the helper).
            if instruction.len() > MAX_INSTRUCTION_BYTES
                || input.len() > MAX_INPUT_BYTES
                || instruction.contains('\0')
                || input.contains('\0')
                || deadline.is_zero()
                || deadline > Duration::from_millis(MAX_DEADLINE_MS)
            {
                return CorrelatedTransformOutcome::before_runtime(TransformError::InvalidRequest);
            }

            if self
                .busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return CorrelatedTransformOutcome::before_runtime(TransformError::Busy);
            }
            // Register this request's cancel token so cancel_inflight_request()
            // reaches exactly THIS request. No stale-flag reset is needed (the
            // token is fresh per request), so a concurrent cancel can never be
            // wiped. Cleared by the BusyGuard drop below.
            self.set_inflight_cancel(cancel.clone());
            // Own the busy flag + cancel slot from here. Moving this guard into
            // the blocking closure guarantees both clear when the blocking work
            // ends even if this async future is dropped at the `.await` below.
            // No `.await` sits between the busy claim and this move, so there is
            // no window where busy is held without its guard.
            let busy_guard = BusyGuard {
                sidecar: Arc::clone(self),
                cancel: cancel.clone(),
            };

            // Bucket sizes up front so no raw length leaves this frame.
            let instruction_bucket = size_bucket(instruction.len());
            let input_bucket = size_bucket(input.len());

            let this = Arc::clone(self);
            let instruction = instruction.to_string();
            let input = input.to_string();
            let started = Instant::now();
            let join = tokio::task::spawn_blocking(move || {
                let _busy = busy_guard;
                this.transform_blocking(&instruction, &input, deadline, &cancel)
            })
            .await;

            let outcome = match join {
                Ok(outcome) => outcome,
                Err(_) => CorrelatedTransformOutcome::before_runtime(TransformError::Internal),
            };

            // Telemetry: enums, durations, buckets, token counts only.
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let (result_code, output_tokens) = match &outcome.result {
                Ok(out) => ("ok", out.output_tokens),
                Err(err) => (err.as_str(), 0),
            };
            if let Some(transform_pass_id) = transform_pass_id {
                tracing::info!(
                    target: "pipeline",
                    transform_pass_id,
                    outcome = result_code,
                    duration_ms = elapsed_ms,
                    instruction_bucket,
                    input_bucket,
                    output_tokens,
                    "llm_transform"
                );
            } else {
                tracing::info!(
                    target: "pipeline",
                    outcome = result_code,
                    duration_ms = elapsed_ms,
                    instruction_bucket,
                    input_bucket,
                    output_tokens,
                    "llm_transform"
                );
            }
            outcome
        }

        fn transform_blocking(
            &self,
            instruction: &str,
            input: &str,
            deadline: Duration,
            cancel: &CancelToken,
        ) -> CorrelatedTransformOutcome {
            let mut inner = self.lock();
            let cache_hit = inner.child.is_some();
            let load_started = Instant::now();
            let mut diagnostics = SidecarDiagnostics::default();

            if inner.breaker.is_disabled(Instant::now()) {
                return CorrelatedTransformOutcome {
                    result: Err(TransformError::Disabled),
                    spawn_load_ms: Some(load_started.elapsed().as_millis() as u64),
                    generation_ms: None,
                    cache_hit: Some(cache_hit),
                    diagnostics,
                };
            }
            if let Some(_reason) = self.guard().heavy_runtime_active() {
                return CorrelatedTransformOutcome {
                    result: Err(TransformError::HeavyRuntimeActive),
                    spawn_load_ms: Some(load_started.elapsed().as_millis() as u64),
                    generation_ms: None,
                    cache_hit: Some(cache_hit),
                    diagnostics,
                };
            }

            // Verify the model exists before touching the helper.
            let model_path = match self.plan.model_path() {
                Ok(path) => path,
                Err(error) => {
                    return CorrelatedTransformOutcome {
                        result: Err(error),
                        spawn_load_ms: Some(load_started.elapsed().as_millis() as u64),
                        generation_ms: None,
                        cache_hit: Some(cache_hit),
                        diagnostics,
                    };
                }
            };

            // Lazy spawn. Release the ASR model first so only one heavy runtime
            // is ever resident (mutual exclusion, ADR "Lifecycle and resources").
            if inner.child.is_none() {
                self.guard().release_asr();
                match self.spawn_and_handshake(&model_path, cancel, &mut diagnostics) {
                    Ok(child) => inner.child = Some(child),
                    Err(TransformError::ModelMismatch) => {
                        // A corrupt / wrong-content model at the ready path. Remove
                        // it (production only) so status reports not_downloaded and
                        // the user can re-download. Not a helper fault → no breaker
                        // trip; the file is gone so this cannot loop.
                        self.plan.cleanup_bad_model();
                        return CorrelatedTransformOutcome {
                            result: Err(TransformError::ModelMismatch),
                            spawn_load_ms: Some(load_started.elapsed().as_millis() as u64),
                            generation_ms: None,
                            cache_hit: Some(cache_hit),
                            diagnostics,
                        };
                    }
                    Err(err) => {
                        if err != TransformError::Cancelled {
                            inner.breaker.record_failure(Instant::now());
                        }
                        return CorrelatedTransformOutcome {
                            result: Err(err),
                            spawn_load_ms: Some(load_started.elapsed().as_millis() as u64),
                            generation_ms: None,
                            cache_hit: Some(cache_hit),
                            diagnostics,
                        };
                    }
                }
            }
            let spawn_load_ms = load_started.elapsed().as_millis() as u64;

            let request_id = unique_id("req");
            let generation_started = Instant::now();
            diagnostics.failure_phase = Some(SidecarDiagnosticPhase::RequestReceipt);
            let outcome = {
                let child = inner.child.as_mut().expect("child present");
                run_request(
                    child,
                    &request_id,
                    instruction,
                    input,
                    deadline,
                    &self.plan,
                    cancel,
                    &mut diagnostics,
                )
            };

            let result = match outcome {
                RequestOutcome::Ok(out) => {
                    inner.last_activity = Instant::now();
                    diagnostics.failure_phase = None;
                    Ok(out)
                }
                RequestOutcome::HelperError(err) => {
                    // The frame is well-formed so the helper is kept for reuse.
                    // A helper that faults every request must not respawn forever,
                    // so most errors count toward the breaker — but a self-reported
                    // DeadlineExceeded (Timeout) or Cancelled is a designed outcome,
                    // not a runtime fault, and must never disable the runtime.
                    inner.last_activity = Instant::now();
                    if !matches!(err, TransformError::Timeout | TransformError::Cancelled) {
                        inner.breaker.record_failure(Instant::now());
                    }
                    Err(err)
                }
                RequestOutcome::TimedOutKeepChild => {
                    // Cancel was cooperatively confirmed; helper cleared context.
                    inner.last_activity = Instant::now();
                    Err(TransformError::Timeout)
                }
                RequestOutcome::TimedOutKilled => {
                    kill_child(self.take_child(&mut inner));
                    Err(TransformError::Timeout)
                }
                RequestOutcome::CancelledKeepChild => {
                    // User cancel confirmed; helper stays resident for reuse.
                    inner.last_activity = Instant::now();
                    Err(TransformError::Cancelled)
                }
                RequestOutcome::CancelledKilled => {
                    kill_child(self.take_child(&mut inner));
                    Err(TransformError::Cancelled)
                }
                RequestOutcome::Crashed => {
                    kill_child(self.take_child(&mut inner));
                    inner.breaker.record_failure(Instant::now());
                    Err(TransformError::Crashed)
                }
                RequestOutcome::Protocol => {
                    kill_child(self.take_child(&mut inner));
                    inner.breaker.record_failure(Instant::now());
                    Err(TransformError::Protocol)
                }
                RequestOutcome::OutputInvalid => {
                    kill_child(self.take_child(&mut inner));
                    inner.breaker.record_failure(Instant::now());
                    Err(TransformError::OutputInvalid)
                }
            };
            CorrelatedTransformOutcome {
                result,
                spawn_load_ms: Some(spawn_load_ms),
                generation_ms: Some(generation_started.elapsed().as_millis() as u64),
                cache_hit: Some(cache_hit),
                diagnostics,
            }
        }

        fn spawn_and_handshake(
            &self,
            model_path: &Path,
            cancel: &CancelToken,
            diagnostics: &mut SidecarDiagnostics,
        ) -> Result<Child, TransformError> {
            use murmur_local_llm_protocol::MODEL_FD;
            use std::os::unix::io::AsRawFd;
            use std::os::unix::process::CommandExt;

            let (size, sha) = self.plan.pins();
            diagnostics.failure_phase = Some(SidecarDiagnosticPhase::HostModelVerification);
            let verify_started = Instant::now();
            let model_file = open_and_verify_model(model_path, size, sha, cancel)?;
            diagnostics.host_model_verification_ms =
                Some(verify_started.elapsed().as_millis() as u64);
            if cancel.is_cancelled() {
                return Err(TransformError::Cancelled);
            }
            diagnostics.failure_phase = Some(SidecarDiagnosticPhase::HelperSpawn);
            let helper_path = self.plan.helper_path()?;
            let raw_fd = model_file.as_raw_fd();
            let spawn_started = Instant::now();

            // TODO(#312 PR-A3/C2 packaging integration): before spawn, validate
            // `helper_path` with `SecStaticCodeCheckValidity` against the fixed
            // designated requirement (identifier
            // `com.localdictation.local-llm-sidecar`, matching Team ID, hardened
            // runtime). Path pinning alone does not defend the ADR threat-model
            // "Helper replacement" row — this is a hard gate for the signed,
            // notarized build and must land before shipping the runtime.
            let mut command = std::process::Command::new(&helper_path);
            command
                .current_dir("/")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                // Never ingest or inherit helper stderr: failure details can
                // contain runtime paths or device strings. The enum-only phase
                // protocol is the sole diagnostics channel.
                .stderr(std::process::Stdio::null());
            command.env_clear();
            // Scenario env is injected only in test-support builds; production
            // spawns with a strictly empty environment.
            #[cfg(any(debug_assertions, feature = "llm-test-support"))]
            for (k, v) in self.plan.scenario_env() {
                command.env(k, v);
            }

            // Hand the verified model over as inherited read-only fd 3, then
            // close every other inherited descriptor above fd 3 so only
            // stdin/stdout/stderr and the model fd survive exec. `dup2` clears
            // FD_CLOEXEC on the new fd, but is a no-op (leaving CLOEXEC set) when
            // oldfd already equals 3 — so clear it explicitly. Only
            // async-signal-safe libc calls run here. No request-controlled args.
            unsafe {
                command.pre_exec(move || {
                    if raw_fd != MODEL_FD && libc::dup2(raw_fd, MODEL_FD) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::fcntl(MODEL_FD, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let max_fd = libc::getdtablesize();
                    let mut fd = MODEL_FD + 1;
                    while fd < max_fd {
                        // Ignore EBADF and other errors: closing unused fds is
                        // best-effort hardening, not a correctness dependency.
                        libc::close(fd);
                        fd += 1;
                    }
                    Ok(())
                });
            }

            let mut proc = command.spawn().map_err(|_| TransformError::SpawnFailed)?;
            diagnostics.helper_spawn_ms = Some(spawn_started.elapsed().as_millis() as u64);
            diagnostics.failure_phase = Some(SidecarDiagnosticPhase::ReadyHandshake);
            let handshake_started = Instant::now();
            let pid = proc.id();
            let pid_guard = SpawnPidGuard::new(&self.resident_pid, pid);
            let mut stdin = match proc.stdin.take() {
                Some(stdin) => stdin,
                None => {
                    kill_and_reap(&mut proc);
                    return Err(TransformError::SpawnFailed);
                }
            };
            let stdout = match proc.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    kill_and_reap(&mut proc);
                    return Err(TransformError::SpawnFailed);
                }
            };

            // Reader thread: classifies frames, protocol violations, and EOF.
            let (tx, rx) = std::sync::mpsc::channel::<HelperEvent>();
            std::thread::spawn(move || {
                let mut stdout = stdout;
                loop {
                    match read_frame::<HelperMessage>(&mut stdout) {
                        Ok(frame) => {
                            if tx.send(HelperEvent::Frame(frame)).is_err() {
                                break;
                            }
                        }
                        Err(FrameError::IncompleteHeader) => {
                            let _ = tx.send(HelperEvent::Exited);
                            break;
                        }
                        Err(_) => {
                            let _ = tx.send(HelperEvent::ProtocolViolation);
                            break;
                        }
                    }
                }
            });

            let session_nonce = unique_id("nonce");
            let hello = HostMessage::Hello {
                protocol: PROTOCOL_NAME.to_string(),
                version: PROTOCOL_VERSION,
                session_nonce: session_nonce.clone(),
                model: ModelIdentity {
                    id: TRANSFORM_MODEL_ID.to_string(),
                    sha256: sha.to_string(),
                    size_bytes: size,
                },
                limits: ProtocolLimits::default(),
            };
            if write_frame(&mut stdin, &hello).is_err() {
                kill_and_reap(&mut proc);
                return Err(TransformError::HandshakeFailed);
            }

            // Await Ready within the handshake / model-load deadline.
            let deadline_at = Instant::now() + self.plan.handshake_timeout();
            let expected_phases = [
                (
                    murmur_local_llm_protocol::DiagnosticPhase::HelperModelVerification,
                    murmur_local_llm_protocol::PhaseState::Started,
                ),
                (
                    murmur_local_llm_protocol::DiagnosticPhase::HelperModelVerification,
                    murmur_local_llm_protocol::PhaseState::Completed,
                ),
                (
                    murmur_local_llm_protocol::DiagnosticPhase::BackendInitialization,
                    murmur_local_llm_protocol::PhaseState::Started,
                ),
                (
                    murmur_local_llm_protocol::DiagnosticPhase::BackendInitialization,
                    murmur_local_llm_protocol::PhaseState::Completed,
                ),
                (
                    murmur_local_llm_protocol::DiagnosticPhase::ModelLoad,
                    murmur_local_llm_protocol::PhaseState::Started,
                ),
                (
                    murmur_local_llm_protocol::DiagnosticPhase::ModelLoad,
                    murmur_local_llm_protocol::PhaseState::Completed,
                ),
            ];
            let mut expected_phase_index = 0_usize;
            loop {
                if cancel.is_cancelled() {
                    kill_and_reap(&mut proc);
                    return Err(TransformError::Cancelled);
                }
                let now = Instant::now();
                if now >= deadline_at {
                    kill_and_reap(&mut proc);
                    return Err(TransformError::Timeout);
                }
                let slice = (deadline_at - now).min(Duration::from_millis(25));
                match rx.recv_timeout(slice) {
                    Ok(HelperEvent::Frame(HelperMessage::DiagnosticPhase {
                        protocol,
                        version,
                        session_nonce: got_nonce,
                        request_id,
                        phase,
                        state,
                        duration_ms,
                    })) => {
                        use murmur_local_llm_protocol::{DiagnosticPhase, PhaseState};
                        if protocol != PROTOCOL_NAME
                            || version != PROTOCOL_VERSION
                            || got_nonce != session_nonce
                            || !murmur_local_llm_protocol::validate_diagnostic_phase(
                                request_id.as_deref(),
                                phase,
                                state,
                                duration_ms,
                            )
                            || expected_phases.get(expected_phase_index) != Some(&(phase, state))
                        {
                            kill_and_reap(&mut proc);
                            return Err(TransformError::HandshakeFailed);
                        }
                        expected_phase_index += 1;
                        let mapped = match phase {
                            DiagnosticPhase::HelperModelVerification => {
                                SidecarDiagnosticPhase::HelperModelVerification
                            }
                            DiagnosticPhase::BackendInitialization => {
                                SidecarDiagnosticPhase::BackendInitialization
                            }
                            DiagnosticPhase::ModelLoad => SidecarDiagnosticPhase::ModelLoad,
                            DiagnosticPhase::RequestReceipt => {
                                SidecarDiagnosticPhase::RequestReceipt
                            }
                            DiagnosticPhase::FirstToken => SidecarDiagnosticPhase::FirstToken,
                        };
                        diagnostics.failure_phase = Some(mapped);
                        if state == PhaseState::Completed {
                            match phase {
                                DiagnosticPhase::HelperModelVerification => {
                                    diagnostics.helper_model_verification_ms = duration_ms
                                }
                                DiagnosticPhase::BackendInitialization => {
                                    diagnostics.backend_initialization_ms = duration_ms
                                }
                                DiagnosticPhase::ModelLoad => {
                                    diagnostics.model_load_ms = duration_ms
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(HelperEvent::Frame(HelperMessage::Ready {
                        protocol,
                        version,
                        session_nonce: got_nonce,
                        model,
                        ..
                    })) => {
                        if protocol != PROTOCOL_NAME
                            || version != PROTOCOL_VERSION
                            || got_nonce != session_nonce
                            || model.sha256 != sha
                            || model.size_bytes != size
                            || expected_phase_index != expected_phases.len()
                        {
                            kill_and_reap(&mut proc);
                            return Err(TransformError::HandshakeFailed);
                        }
                        let child = Child {
                            proc,
                            stdin,
                            events: rx,
                            session_nonce,
                            pid,
                            _model_file: model_file,
                        };
                        pid_guard.keep();
                        diagnostics.ready_handshake_ms =
                            Some(handshake_started.elapsed().as_millis() as u64);
                        diagnostics.failure_phase = None;
                        return Ok(child);
                    }
                    Ok(HelperEvent::Exited) | Err(RecvTimeoutError::Disconnected) => {
                        Self::record_process_exit(&mut proc, diagnostics);
                        kill_and_reap(&mut proc);
                        return Err(TransformError::HandshakeFailed);
                    }
                    Ok(_) => {
                        kill_and_reap(&mut proc);
                        return Err(TransformError::HandshakeFailed);
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                }
            }
        }

        /// Periodic maintenance: RSS ceiling + idle unload. Driven by the host
        /// heartbeat. Skips while a transform is in flight.
        pub fn maintenance_tick(&self) {
            if self.busy.load(Ordering::Acquire) {
                return;
            }
            // Decide under a short lock; do any blocking shutdown after release.
            let (idle_child, kill_now) = {
                let mut inner = match self.inner.try_lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let Some(child) = inner.child.as_ref() else {
                    return;
                };
                let pid = child.pid;

                let mut kill = false;
                if let Some(bytes) = child_rss_bytes(pid) {
                    match rss_action(bytes) {
                        RssAction::Ok => inner.rss_warned = false,
                        RssAction::Warn => {
                            // Log once per crossing, not every tick.
                            if !inner.rss_warned {
                                tracing::warn!(
                                    target: "pipeline",
                                    rss_mb = bytes / (1024 * 1024),
                                    "llm_sidecar_rss_warn"
                                );
                                inner.rss_warned = true;
                            }
                        }
                        RssAction::Kill => {
                            tracing::warn!(
                                target: "pipeline",
                                rss_mb = bytes / (1024 * 1024),
                                "llm_sidecar_rss_kill"
                            );
                            kill = true;
                        }
                    }
                }

                if kill {
                    (self.take_child(&mut inner), true)
                } else if inner.last_activity.elapsed() >= self.plan.idle_after() {
                    (self.take_child(&mut inner), false)
                } else {
                    return;
                }
            };

            if kill_now {
                kill_child(idle_child);
            } else {
                shutdown_child(idle_child, self.plan.cancel_grace());
                tracing::info!(target: "pipeline", "llm_sidecar_idle_unload");
            }
        }

        /// Reset the crash circuit breaker and drop any resident helper.
        /// Fail-fast when a transform is running: it owns the child and will
        /// idle-unload on its own; the breaker is not disabled while it runs.
        pub fn reset(&self) {
            let child = match self.inner.try_lock() {
                Ok(mut inner) => {
                    inner.breaker.reset();
                    self.take_child(&mut inner)
                }
                Err(_) => return,
            };
            shutdown_child(child, self.plan.cancel_grace());
            tracing::info!(target: "pipeline", "llm_sidecar_reset");
        }

        /// Stop the helper (used before recording / file transcription /
        /// benchmark, and on model removal). Takes the child out under a short
        /// lock, then shuts it down outside the lock. Fail-fast (a no-op) when a
        /// transform is in flight — the guard never blocks behind a ≤31s request.
        pub fn shutdown(&self) {
            let child = match self.inner.try_lock() {
                Ok(mut inner) => self.take_child(&mut inner),
                Err(_) => return,
            };
            shutdown_child(child, self.plan.cancel_grace());
        }
    }

    enum RequestOutcome {
        Ok(TransformOutput),
        HelperError(TransformError),
        TimedOutKeepChild,
        TimedOutKilled,
        /// User-triggered cooperative cancel confirmed; helper stays resident.
        CancelledKeepChild,
        /// User-triggered cancel; helper unresponsive and was killed.
        CancelledKilled,
        Crashed,
        Protocol,
        OutputInvalid,
    }

    /// Why `cancel_and_settle` was invoked — maps keep/kill onto the matching
    /// timeout vs user-cancel `RequestOutcome` variants.
    #[derive(Clone, Copy)]
    enum CancelReason {
        Deadline,
        User,
    }

    /// The ADR requires the protocol name, version, and per-process session
    /// nonce on EVERY frame. Validate before acting on any helper message; a
    /// mismatch is a protocol violation that fails closed.
    fn helper_frame_valid(message: &HelperMessage, nonce: &str) -> bool {
        let (protocol, version, session_nonce) = match message {
            HelperMessage::DiagnosticPhase {
                protocol,
                version,
                session_nonce,
                ..
            }
            | HelperMessage::Ready {
                protocol,
                version,
                session_nonce,
                ..
            }
            | HelperMessage::Result {
                protocol,
                version,
                session_nonce,
                ..
            }
            | HelperMessage::Cancelled {
                protocol,
                version,
                session_nonce,
                ..
            }
            | HelperMessage::Error {
                protocol,
                version,
                session_nonce,
                ..
            }
            | HelperMessage::Stopped {
                protocol,
                version,
                session_nonce,
            } => (protocol, version, session_nonce),
        };
        protocol == PROTOCOL_NAME && *version == PROTOCOL_VERSION && session_nonce == nonce
    }

    #[allow(clippy::too_many_arguments)]
    fn run_request(
        child: &mut Child,
        request_id: &str,
        instruction: &str,
        input: &str,
        deadline: Duration,
        plan: &SpawnPlan,
        cancel: &CancelToken,
        diagnostics: &mut SidecarDiagnostics,
    ) -> RequestOutcome {
        let transform = HostMessage::Transform {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: child.session_nonce.clone(),
            request_id: request_id.to_string(),
            instruction: instruction.to_string(),
            input: input.to_string(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
            deadline_ms: deadline.as_millis() as u64,
        };
        if write_frame(&mut child.stdin, &transform).is_err() {
            return RequestOutcome::Crashed;
        }

        // Wait the deadline plus a small slack so a healthy helper can self-report
        // DeadlineExceeded before we escalate to a cooperative cancel.
        let wait = deadline + plan.request_slack();
        let deadline_at = Instant::now() + wait;
        let mut receipt_seen = false;
        let mut first_token_seen = false;
        loop {
            // User cancel (e.g. Esc / short-tap) wins over the deadline wait.
            if cancel.is_cancelled() {
                return cancel_and_settle(
                    child,
                    request_id,
                    plan.cancel_grace(),
                    CancelReason::User,
                );
            }
            let now = Instant::now();
            if now >= deadline_at {
                return cancel_and_settle(
                    child,
                    request_id,
                    plan.cancel_grace(),
                    CancelReason::Deadline,
                );
            }
            // Poll in short slices so a cancel token flip is observed promptly
            // even when the helper is silent.
            let slice = (deadline_at - now).min(Duration::from_millis(50));
            match child.events.recv_timeout(slice) {
                Ok(HelperEvent::Frame(frame)) => {
                    if !helper_frame_valid(&frame, &child.session_nonce) {
                        return RequestOutcome::Protocol;
                    }
                    match frame {
                        HelperMessage::DiagnosticPhase {
                            request_id: got,
                            phase,
                            state,
                            duration_ms,
                            ..
                        } => {
                            use murmur_local_llm_protocol::DiagnosticPhase;
                            if got.as_deref() != Some(request_id)
                                || !murmur_local_llm_protocol::validate_diagnostic_phase(
                                    got.as_deref(),
                                    phase,
                                    state,
                                    duration_ms,
                                )
                            {
                                return RequestOutcome::Protocol;
                            }
                            match phase {
                                DiagnosticPhase::RequestReceipt
                                    if !receipt_seen && !first_token_seen =>
                                {
                                    receipt_seen = true;
                                    diagnostics.request_receipt_ms = duration_ms;
                                    diagnostics.failure_phase =
                                        Some(SidecarDiagnosticPhase::FirstToken);
                                }
                                DiagnosticPhase::FirstToken
                                    if receipt_seen && !first_token_seen =>
                                {
                                    first_token_seen = true;
                                    diagnostics.first_token_ms = duration_ms;
                                    diagnostics.failure_phase =
                                        Some(SidecarDiagnosticPhase::Generation);
                                }
                                _ => return RequestOutcome::Protocol,
                            }
                            continue;
                        }
                        HelperMessage::Result {
                            request_id: got,
                            output,
                            finish_reason,
                            output_tokens,
                            ..
                        } => {
                            if got != request_id {
                                return RequestOutcome::Protocol;
                            }
                            if !receipt_seen {
                                return RequestOutcome::Protocol;
                            }
                            if output.len() > MAX_OUTPUT_BYTES
                                || output_tokens > MAX_OUTPUT_TOKENS
                                || output.contains('\0')
                            {
                                return RequestOutcome::OutputInvalid;
                            }
                            return RequestOutcome::Ok(TransformOutput {
                                output,
                                finish_reason,
                                output_tokens,
                            });
                        }
                        HelperMessage::Error {
                            code,
                            request_id: err_request_id,
                            ..
                        } => {
                            // An Error may omit the request id, but if present it
                            // must name the in-flight request.
                            if let Some(id) = err_request_id {
                                if id != request_id {
                                    return RequestOutcome::Protocol;
                                }
                            }
                            return RequestOutcome::HelperError(TransformError::from_helper_code(
                                code,
                            ));
                        }
                        // A stray cancelled/late frame: ignore and keep waiting.
                        HelperMessage::Cancelled { .. } => continue,
                        // Ready/Stopped mid-request: fail closed.
                        HelperMessage::Ready { .. } | HelperMessage::Stopped { .. } => {
                            return RequestOutcome::Protocol
                        }
                    }
                }
                Ok(HelperEvent::ProtocolViolation) => return RequestOutcome::Protocol,
                Ok(HelperEvent::Exited) | Err(RecvTimeoutError::Disconnected) => {
                    LlmSidecar::record_process_exit(&mut child.proc, diagnostics);
                    return RequestOutcome::Crashed;
                }
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    /// Cooperative cancel: send a Cancel frame, wait `grace` for a Cancelled
    /// confirmation, then kill if the helper never confirms.
    fn cancel_and_settle(
        child: &mut Child,
        request_id: &str,
        grace: Duration,
        reason: CancelReason,
    ) -> RequestOutcome {
        let cancel = HostCancel {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: child.session_nonce.clone(),
            request_id: request_id.to_string(),
        };
        let _ = write_frame(&mut child.stdin, &cancel);

        let (keep, kill) = match reason {
            CancelReason::Deadline => (
                RequestOutcome::TimedOutKeepChild,
                RequestOutcome::TimedOutKilled,
            ),
            CancelReason::User => (
                RequestOutcome::CancelledKeepChild,
                RequestOutcome::CancelledKilled,
            ),
        };

        let grace_at = Instant::now() + grace;
        loop {
            let now = Instant::now();
            if now >= grace_at {
                return kill;
            }
            match child.events.recv_timeout(grace_at - now) {
                Ok(HelperEvent::Frame(frame)) => {
                    // Even during cancellation, a frame with the wrong nonce /
                    // protocol is a violation — kill rather than trust it.
                    if !helper_frame_valid(&frame, &child.session_nonce) {
                        return kill;
                    }
                    match frame {
                        // Only an explicit acknowledgement for this exact
                        // request confirms cooperative cancellation. Receipt /
                        // first-token frames may already be queued when Cancel
                        // is sent and must never be mistaken for an ack.
                        HelperMessage::Cancelled {
                            request_id: got, ..
                        } if got == request_id => return keep,
                        HelperMessage::DiagnosticPhase {
                            request_id,
                            phase,
                            state,
                            duration_ms,
                            ..
                        } => {
                            if !murmur_local_llm_protocol::validate_diagnostic_phase(
                                request_id.as_deref(),
                                phase,
                                state,
                                duration_ms,
                            ) {
                                return kill;
                            }
                            continue;
                        }
                        // A stale/mismatched Cancelled or any other valid frame
                        // is not confirmation. Keep waiting until the grace
                        // deadline, then kill and reap the helper.
                        _ => continue,
                    }
                }
                Ok(HelperEvent::Exited)
                | Ok(HelperEvent::ProtocolViolation)
                | Err(RecvTimeoutError::Disconnected) => return kill,
                Err(RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    /// Kill a raw process handle and reap it so it never lingers as a zombie.
    fn kill_and_reap(proc: &mut std::process::Child) {
        let _ = proc.kill();
        let _ = proc.wait();
    }

    fn kill_child(child: Option<Child>) {
        if let Some(mut child) = child {
            kill_and_reap(&mut child.proc);
        }
    }

    fn shutdown_child(child: Option<Child>, grace: Duration) {
        let Some(mut child) = child else {
            return;
        };
        let shutdown = HostMessage::Shutdown {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: child.session_nonce.clone(),
        };
        let _ = write_frame(&mut child.stdin, &shutdown);
        let deadline = Instant::now() + grace;
        loop {
            match child.proc.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.proc.kill();
                        let _ = child.proc.wait();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    let _ = child.proc.kill();
                    let _ = child.proc.wait();
                    return;
                }
            }
        }
    }

    /// Read a child process RSS in bytes by pid via sysinfo.
    fn child_rss_bytes(pid: u32) -> Option<u64> {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut system = System::new();
        let target = Pid::from_u32(pid);
        system.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
        system.process(target).map(|p| p.memory())
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub use supported::LlmSidecar;

// ===========================================================================
// Unsupported platforms: uniform stub reporting Unsupported.
// ===========================================================================

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub struct LlmSidecar {
    host_guard: OnceLock<Arc<dyn HostGuard>>,
    busy: AtomicBool,
    resident_pid: AtomicU32,
    _plan: SpawnPlan,
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
impl LlmSidecar {
    pub fn new() -> Self {
        Self::with_plan(SpawnPlan::Production)
    }

    #[cfg(any(debug_assertions, feature = "llm-test-support"))]
    pub fn for_test(config: TestSpawnConfig) -> Self {
        Self::with_plan(SpawnPlan::Test(config))
    }

    fn with_plan(plan: SpawnPlan) -> Self {
        Self {
            host_guard: OnceLock::new(),
            busy: AtomicBool::new(false),
            resident_pid: AtomicU32::new(0),
            _plan: plan,
        }
    }

    pub fn set_host_guard(&self, guard: Arc<dyn HostGuard>) {
        let _ = self.host_guard.set(guard);
    }

    pub fn is_transform_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub fn runtime_disabled(&self) -> bool {
        false
    }

    /// No runtime here, so nothing is ever in flight — a no-op.
    pub fn cancel_inflight_request(&self) {}

    pub fn has_live_child(&self) -> bool {
        false
    }

    pub fn resident_pid(&self) -> Option<u32> {
        None
    }

    pub async fn transform(
        self: &Arc<Self>,
        _instruction: &str,
        _input: &str,
        _deadline: Duration,
        _cancel: CancelToken,
    ) -> Result<TransformOutput, TransformError> {
        Err(TransformError::Unsupported)
    }

    pub async fn transform_for_pass(
        self: &Arc<Self>,
        _transform_pass_id: u64,
        _instruction: &str,
        _input: &str,
        _deadline: Duration,
        _cancel: CancelToken,
    ) -> CorrelatedTransformOutcome {
        CorrelatedTransformOutcome::before_runtime(TransformError::Unsupported)
    }

    pub fn maintenance_tick(&self) {}
    pub fn reset(&self) {}
    pub fn shutdown(&self) {}
}

impl Default for LlmSidecar {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Unit tests (portable + supported-platform).
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_constants_are_internally_consistent() {
        assert_eq!(TRANSFORM_MODEL_SHA256.len(), 64);
        assert!(TRANSFORM_MODEL_SHA256
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(TRANSFORM_MODEL_REVISION.len(), 40);
        assert!(TRANSFORM_MODEL_URL.contains(TRANSFORM_MODEL_REVISION));
        assert!(TRANSFORM_MODEL_URL.ends_with(TRANSFORM_MODEL_FILENAME));
        assert_eq!(TRANSFORM_MODEL_SIZE_BYTES, 1_117_320_736);
        // The published model path is hash-versioned beneath the app models dir.
        if let Some(path) = installed_model_path() {
            assert!(path.to_string_lossy().contains(TRANSFORM_MODEL_SHA256));
            assert!(path.ends_with(TRANSFORM_MODEL_FILENAME));
        }
    }

    #[test]
    fn cancel_token_is_scoped_per_instance() {
        // The per-request cancel token is the core of item 11: two distinct
        // tokens are independent, and clones of one share its state. This is
        // what makes a cancel for request N unable to affect request N+1.
        let a = CancelToken::new();
        let b = CancelToken::new();
        assert!(!a.is_cancelled() && !b.is_cancelled());

        a.cancel();
        assert!(a.is_cancelled(), "cancelling A must flip A");
        assert!(
            !b.is_cancelled(),
            "cancelling A must NOT flip B (per-request)"
        );

        // A clone observes the same cancellation; identity holds across clones.
        let a2 = a.clone();
        assert!(a2.is_cancelled());
        assert!(a.is_same(&a2));
        assert!(!a.is_same(&b));
    }

    #[test]
    fn size_buckets_are_monotonic_and_bounded() {
        assert_eq!(size_bucket(0), "0");
        assert_eq!(size_bucket(256), "le256");
        assert_eq!(size_bucket(257), "le1k");
        assert_eq!(size_bucket(4096), "le4k");
        assert_eq!(size_bucket(16384), "le16k");
        assert_eq!(size_bucket(16385), "gt16k");
    }

    #[test]
    fn rss_policy_thresholds() {
        assert_eq!(rss_action(0), RssAction::Ok);
        assert_eq!(rss_action(RSS_WARN_BYTES - 1), RssAction::Ok);
        assert_eq!(rss_action(RSS_WARN_BYTES), RssAction::Warn);
        assert_eq!(rss_action(RSS_KILL_BYTES - 1), RssAction::Warn);
        assert_eq!(rss_action(RSS_KILL_BYTES), RssAction::Kill);
    }

    #[test]
    fn breaker_opens_after_three_failures_and_resets() {
        let mut breaker = Breaker::default();
        let now = std::time::Instant::now();
        assert!(!breaker.is_disabled(now));
        breaker.record_failure(now);
        breaker.record_failure(now);
        assert!(!breaker.is_disabled(now));
        breaker.record_failure(now);
        assert!(breaker.is_disabled(now));
        breaker.reset();
        assert!(!breaker.is_disabled(now));
    }

    #[test]
    fn breaker_prunes_failures_outside_the_window() {
        let mut breaker = Breaker::default();
        let base = std::time::Instant::now();
        breaker.record_failure(base);
        breaker.record_failure(base);
        // A third failure more than ten minutes later prunes the first two.
        let later = base + BREAKER_WINDOW + Duration::from_secs(1);
        breaker.record_failure(later);
        assert!(!breaker.is_disabled(later));
    }

    #[cfg(unix)]
    #[test]
    fn model_verification_accepts_exact_pins_and_rejects_mismatches() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("murmur-llm-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"deterministic fixture model bytes").unwrap();
        f.sync_all().unwrap();
        drop(f);

        let (size, sha) = model_file_digest(&path).unwrap();
        // Exact pins verify and leave the handle rewound to offset 0.
        let mut handle = open_and_verify_model(&path, size, &sha, &CancelToken::new()).unwrap();
        use std::io::Read;
        let mut first = [0_u8; 4];
        handle.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"dete");

        // Wrong hash and wrong size both fail closed.
        let wrong_sha = "0".repeat(64);
        assert_eq!(
            open_and_verify_model(&path, size, &wrong_sha, &CancelToken::new()).unwrap_err(),
            TransformError::ModelMismatch
        );
        assert_eq!(
            open_and_verify_model(&path, size + 1, &sha, &CancelToken::new()).unwrap_err(),
            TransformError::ModelMismatch
        );
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert_eq!(
            open_and_verify_model(&path, size, &sha, &cancelled).unwrap_err(),
            TransformError::Cancelled
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
