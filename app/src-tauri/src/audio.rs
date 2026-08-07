use crate::managed_child::{bundled_sibling, ManagedChild};
use crate::MutexExt;
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, CaptureBackend, CapturePhase,
    CaptureSetupStep, FailureCode, ProductionFrame, ProductionHelperMessage,
    ProductionHostMessage, SessionNonce, SetupTransition,
};
use serde::Serialize;
use std::fmt;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::Emitter;
use uuid::Uuid;

pub fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

pub fn compute_peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max)
}

const AUDIO_LEVEL_THROTTLE_MS: u64 = 16;
// Keep key-release-to-helper-stop latency below one display frame. The worker
// otherwise waits on helper output and only observes host commands on timeout.
const CAPTURE_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(2);
const HELPER_STOP_DEADLINE: Duration = Duration::from_secs(2);
const HELPER_CONTROL_DEADLINE: Duration = Duration::from_secs(3);
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const TCC_PROMPT_WATCHDOG: Duration = Duration::from_secs(120);
const AUHAL_ATTEMPT_BUDGET: Duration = Duration::from_secs(8);
const CPAL_ATTEMPT_BUDGET: Duration = Duration::from_secs(16);
const COOPERATIVE_STOP_GRACE: Duration = Duration::from_millis(250);
const CAPTURE_TERMINATION_BUDGET: Duration = Duration::from_secs(2);
const CAPTURE_PROTOCOL_RESERVE: Duration = Duration::from_secs(2);
const CAPTURE_ACTIVE_BUDGET: Duration = Duration::from_secs(30);
const CAPTURE_WORKER_IDENTIFIER: &str = "com.localdictation.capture-worker";
pub(crate) const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioFailureKind {
    PermissionDenied,
    DeviceUnavailable,
    DeviceBusy,
    DeviceChanged,
    HostUnavailable,
    InvalidInput,
    ResourceExhausted,
    StreamInvalidated,
    UnsupportedConfig,
    UnsupportedOperation,
    RealtimeDenied,
    Xrun,
    BackendError,
    ProtocolError,
    FirstBufferTimeout,
    InitializationTimeout,
    PermissionPromptTimeout,
    TerminationUnconfirmed,
    WorkerPanicked,
    SignatureInvalid,
}

impl AudioFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission_denied",
            Self::DeviceUnavailable => "device_unavailable",
            Self::DeviceBusy => "device_busy",
            Self::DeviceChanged => "device_changed",
            Self::HostUnavailable => "host_unavailable",
            Self::InvalidInput => "invalid_input",
            Self::ResourceExhausted => "resource_exhausted",
            Self::StreamInvalidated => "stream_invalidated",
            Self::UnsupportedConfig => "unsupported_config",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::RealtimeDenied => "realtime_denied",
            Self::Xrun => "xrun",
            Self::BackendError => "backend_error",
            Self::ProtocolError => "protocol_error",
            Self::FirstBufferTimeout => "first_buffer_timeout",
            Self::InitializationTimeout => "initialization_timeout",
            Self::PermissionPromptTimeout => "permission_prompt_timeout",
            Self::TerminationUnconfirmed => "termination_unconfirmed",
            Self::WorkerPanicked => "worker_panicked",
            Self::SignatureInvalid => "signature_invalid",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AudioFailure {
    pub(crate) kind: AudioFailureKind,
    pub(crate) phase: AudioInitPhase,
}

impl AudioFailure {
    pub(crate) fn new(kind: AudioFailureKind, phase: AudioInitPhase) -> Self {
        Self { kind, phase }
    }

    pub(crate) fn user_message(&self) -> &'static str {
        match self.kind {
            AudioFailureKind::PermissionDenied => {
                "Microphone access denied. Grant permission in System Settings and try again."
            }
            AudioFailureKind::DeviceUnavailable => {
                "The selected microphone is unavailable. Choose another microphone and try again."
            }
            AudioFailureKind::DeviceBusy => {
                "The selected microphone is busy. Close other audio apps and try again."
            }
            AudioFailureKind::DeviceChanged => {
                "The microphone route changed while capture was starting."
            }
            AudioFailureKind::StreamInvalidated => {
                "The microphone stream was invalidated. Try recording again."
            }
            AudioFailureKind::InvalidInput | AudioFailureKind::UnsupportedConfig => {
                "The selected microphone configuration is not supported."
            }
            AudioFailureKind::ResourceExhausted => {
                "The system could not allocate resources for microphone capture."
            }
            AudioFailureKind::FirstBufferTimeout => {
                "The microphone started but did not deliver audio before the deadline."
            }
            AudioFailureKind::InitializationTimeout => {
                "Microphone initialization exceeded the deadline."
            }
            AudioFailureKind::PermissionPromptTimeout => {
                "Microphone permission was not decided before the prompt deadline."
            }
            AudioFailureKind::TerminationUnconfirmed => {
                "The microphone worker could not be stopped safely. Restart Murmur before trying again."
            }
            AudioFailureKind::SignatureInvalid => {
                "The bundled microphone capture worker failed integrity validation."
            }
            AudioFailureKind::ProtocolError => {
                "The bundled microphone capture worker failed to start. Restart Murmur and try again."
            }
            _ => "Microphone capture failed. Try recording again.",
        }
    }

    fn permits_backend_fallback(&self) -> bool {
        !matches!(
            self.kind,
            AudioFailureKind::PermissionDenied
                | AudioFailureKind::PermissionPromptTimeout
                | AudioFailureKind::TerminationUnconfirmed
                | AudioFailureKind::ProtocolError
                | AudioFailureKind::SignatureInvalid
        )
    }
}

impl fmt::Display for AudioFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.user_message())
    }
}

fn failure_kind(code: FailureCode) -> AudioFailureKind {
    match code {
        FailureCode::PermissionDenied => AudioFailureKind::PermissionDenied,
        FailureCode::NoInputDevice => AudioFailureKind::DeviceUnavailable,
        FailureCode::ConfigurationFailed => AudioFailureKind::UnsupportedConfig,
        FailureCode::CallbackStalled => AudioFailureKind::FirstBufferTimeout,
        FailureCode::InvalidMessage => AudioFailureKind::ProtocolError,
        FailureCode::EnumerationFailed => AudioFailureKind::HostUnavailable,
        FailureCode::StreamError => AudioFailureKind::StreamInvalidated,
        FailureCode::StreamOpenFailed | FailureCode::StreamStartFailed | FailureCode::Internal => {
            AudioFailureKind::BackendError
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceDescriptor {
    pub id: String,
    pub name: String,
}

pub(crate) enum AudioCommand {
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioInitPhase {
    DeviceEnumeration,
    ConfigLookup,
    StreamBuild,
    StreamPlay,
    FirstBufferWait,
    Runtime,
}

impl AudioInitPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeviceEnumeration => "device_enumeration",
            Self::ConfigLookup => "config_lookup",
            Self::StreamBuild => "stream_build",
            Self::StreamPlay => "stream_play",
            Self::FirstBufferWait => "first_buffer_wait",
            Self::Runtime => "runtime",
        }
    }
}

#[derive(Debug)]
pub(crate) enum AudioWorkerEvent {
    PhaseEntered {
        owner: crate::audio_lifecycle::AudioOwner,
        phase: AudioInitPhase,
    },
    PhaseExited {
        owner: crate::audio_lifecycle::AudioOwner,
        phase: AudioInitPhase,
        elapsed_ms: u64,
    },
    PermissionPromptPending {
        owner: crate::audio_lifecycle::AudioOwner,
    },
    PermissionPromptResolved {
        owner: crate::audio_lifecycle::AudioOwner,
    },
    TerminationUnconfirmed {
        owner: crate::audio_lifecycle::AudioOwner,
        failure: AudioFailure,
    },
    FirstBuffer {
        owner: crate::audio_lifecycle::AudioOwner,
        sample_rate: u32,
    },
    InitFailed {
        owner: crate::audio_lifecycle::AudioOwner,
        failure: AudioFailure,
    },
    RuntimeFailed {
        owner: crate::audio_lifecycle::AudioOwner,
        failure: AudioFailure,
    },
    StreamStopped {
        owner: crate::audio_lifecycle::AudioOwner,
    },
    ThreadExited {
        owner: crate::audio_lifecycle::AudioOwner,
    },
}

#[derive(Clone)]
pub(crate) struct AudioWorkerEventSender {
    send: Arc<dyn Fn(AudioWorkerEvent) -> Result<(), ()> + Send + Sync>,
}

impl AudioWorkerEventSender {
    pub(crate) fn new(
        send: impl Fn(AudioWorkerEvent) -> Result<(), ()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            send: Arc::new(send),
        }
    }

    pub(crate) fn send(&self, event: AudioWorkerEvent) -> Result<(), ()> {
        (self.send)(event)
    }
}

pub(crate) struct AudioWorkerSpec {
    pub owner: crate::audio_lifecycle::AudioOwner,
    pub command_receiver: Receiver<AudioCommand>,
    pub shared: Arc<Mutex<Vec<f32>>>,
    pub active: Arc<AtomicBool>,
    pub app_handle: Option<tauri::AppHandle>,
    pub device_id: Option<String>,
    pub fallback_to_default: bool,
}

fn capture_identity() -> (u64, SessionNonce, String) {
    let capture_id = NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
    let nonce = *Uuid::new_v4().as_bytes();
    let encoded = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    (capture_id, nonce, encoded)
}

fn helper_path() -> Result<std::path::PathBuf, String> {
    bundled_sibling("murmur-capture-worker")
        .or_else(|_| bundled_sibling("murmur-capture-worker-aarch64-apple-darwin"))
        .map_err(|_| "The signed capture worker is missing from the app bundle.".to_string())
}

fn spawn_helper(
    capture_id: u64,
    nonce_hex: &str,
) -> Result<
    (
        ManagedChild,
        std::process::ChildStdin,
        std::process::ChildStdout,
    ),
    AudioFailure,
> {
    let resolve_started = Instant::now();
    let path = helper_path().map_err(|_| {
        AudioFailure::new(
            AudioFailureKind::HostUnavailable,
            AudioInitPhase::StreamBuild,
        )
    })?;
    let resolve_ms = resolve_started.elapsed().as_millis() as u64;
    let signature_started = Instant::now();
    if !cfg!(debug_assertions) {
        crate::code_signing::validate_bundled_helper(&path, CAPTURE_WORKER_IDENTIFIER).map_err(
            |_| {
                AudioFailure::new(
                    AudioFailureKind::SignatureInvalid,
                    AudioInitPhase::StreamBuild,
                )
            },
        )?;
    }
    let signature_ms = signature_started.elapsed().as_millis() as u64;
    let capture_id_text = capture_id.to_string();
    let spawn_started = Instant::now();
    let child = ManagedChild::spawn_with_arguments(
        &path,
        &["--production-v4", &capture_id_text, nonce_hex],
        &[],
    )
    .map_err(|_| {
        AudioFailure::new(
            AudioFailureKind::HostUnavailable,
            AudioInitPhase::StreamBuild,
        )
    })?;
    tracing::info!(
        target: "audio",
        capture_id,
        resolve_ms,
        signature_ms,
        spawn_ms = spawn_started.elapsed().as_millis() as u64,
        "capture helper process spawned"
    );
    Ok(child)
}

fn read_control_frame_with_deadline(
    mut output: BufReader<std::process::ChildStdout>,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<
    (
        ProductionFrame<ProductionHelperMessage>,
        BufReader<std::process::ChildStdout>,
    ),
    AudioFailure,
> {
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name(format!("murmur-capture-control-{capture_id}"))
        .spawn(move || {
            let frame = read_production_frame(&mut output, capture_id, nonce);
            let _ = sender.send((frame, output));
        })
        .map_err(|_| {
            AudioFailure::new(
                AudioFailureKind::ResourceExhausted,
                AudioInitPhase::StreamBuild,
            )
        })?;
    let (frame, output) = receiver
        .recv_timeout(HELPER_CONTROL_DEADLINE)
        .map_err(|_| {
            AudioFailure::new(
                AudioFailureKind::InitializationTimeout,
                AudioInitPhase::StreamBuild,
            )
        })?;
    frame.map(|frame| (frame, output)).map_err(|_| {
        AudioFailure::new(AudioFailureKind::ProtocolError, AudioInitPhase::StreamBuild)
    })
}

fn hello(
    input: &mut std::process::ChildStdin,
    output: std::process::ChildStdout,
    capture_id: u64,
    nonce: SessionNonce,
) -> Result<BufReader<std::process::ChildStdout>, AudioFailure> {
    write_production_control(input, capture_id, nonce, &ProductionHostMessage::Hello).map_err(
        |_| AudioFailure::new(AudioFailureKind::ProtocolError, AudioInitPhase::StreamBuild),
    )?;
    let (frame, output) =
        read_control_frame_with_deadline(BufReader::new(output), capture_id, nonce)?;
    match frame {
        ProductionFrame::Control(ProductionHelperMessage::HelloAck) => Ok(output),
        _ => Err(AudioFailure::new(
            AudioFailureKind::ProtocolError,
            AudioInitPhase::StreamBuild,
        )),
    }
}

pub fn list_input_devices() -> Result<Vec<AudioDeviceDescriptor>, String> {
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (mut child, mut input, output) =
        spawn_helper(capture_id, &nonce_hex).map_err(|failure| failure.to_string())?;
    let output = match hello(&mut input, output, capture_id, nonce) {
        Ok(output) => output,
        Err(failure) => {
            let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
            return Err(failure.to_string());
        }
    };
    if write_production_control(
        &mut input,
        capture_id,
        nonce,
        &ProductionHostMessage::Enumerate,
    )
    .is_err()
    {
        let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
        return Err("Failed to request microphone enumeration.".to_string());
    }
    let (frame, output) = match read_control_frame_with_deadline(output, capture_id, nonce) {
        Ok(result) => result,
        Err(failure) => {
            let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
            return Err(failure.to_string());
        }
    };
    let devices = match frame {
        ProductionFrame::Control(ProductionHelperMessage::Devices { devices }) => devices
            .into_iter()
            .map(|device| AudioDeviceDescriptor {
                id: device.id,
                name: device.name,
            })
            .collect(),
        _ => {
            let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
            return Err("The capture worker returned an invalid device list.".to_string());
        }
    };
    drop((input, output));
    let _ = child
        .wait_for_exit(Instant::now() + HELPER_STOP_DEADLINE)
        .or_else(|| child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE));
    Ok(devices)
}

pub fn start_transform_capture_audio(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    fallback_to_default: bool,
    transform_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_transform_recording(
        app_handle,
        device_id,
        fallback_to_default,
        transform_pass_id,
    )
}

enum HelperRead {
    Frame(ProductionFrame<ProductionHelperMessage>),
    Invalid,
}

enum AttemptResult {
    Stopped,
    TerminalHandled,
    Failed {
        failure: AudioFailure,
        retained_audio: bool,
    },
}

#[derive(Debug)]
struct ActiveAttemptClock {
    started_at: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

impl ActiveAttemptClock {
    fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            paused_at: None,
            paused_total: Duration::ZERO,
        }
    }

    fn pause(&mut self, now: Instant) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    fn resume(&mut self, now: Instant) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(paused_at);
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        let current_pause = self
            .paused_at
            .map(|paused_at| now.saturating_duration_since(paused_at))
            .unwrap_or_default();
        now.saturating_duration_since(self.started_at)
            .saturating_sub(self.paused_total)
            .saturating_sub(current_pause)
    }
}

fn begin_permission_prompt_pause(
    prompt_started: &mut Option<Instant>,
    clock: &mut ActiveAttemptClock,
    owner: crate::audio_lifecycle::AudioOwner,
    event_sender: &AudioWorkerEventSender,
    now: Instant,
) {
    if prompt_started.is_none() {
        *prompt_started = Some(now);
        clock.pause(now);
        let _ = event_sender.send(AudioWorkerEvent::PermissionPromptPending { owner });
    }
}

fn end_permission_prompt_pause(
    prompt_started: &mut Option<Instant>,
    clock: &mut ActiveAttemptClock,
    owner: crate::audio_lifecycle::AudioOwner,
    event_sender: &AudioWorkerEventSender,
    now: Instant,
) {
    if prompt_started.take().is_some() {
        clock.resume(now);
        let _ = event_sender.send(AudioWorkerEvent::PermissionPromptResolved { owner });
    }
}

fn backend_label(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::Cpal => "cpal",
        CaptureBackend::Auhal => "auhal",
    }
}

fn backend_attempt_budget(backend: CaptureBackend) -> Duration {
    match backend {
        CaptureBackend::Auhal => AUHAL_ATTEMPT_BUDGET,
        CaptureBackend::Cpal => CPAL_ATTEMPT_BUDGET,
    }
}

fn terminate_helper(
    mut child: ManagedChild,
    mut input: Option<std::process::ChildStdin>,
    capture_id: u64,
    nonce: SessionNonce,
    control: Option<ProductionHostMessage>,
) -> Result<(), ManagedChild> {
    if let (Some(input), Some(control)) = (input.as_mut(), control) {
        let _ = write_production_control(input, capture_id, nonce, &control);
    }
    drop(input);
    let deadline = Instant::now() + CAPTURE_TERMINATION_BUDGET;
    let cooperative_deadline = std::cmp::min(deadline, Instant::now() + COOPERATIVE_STOP_GRACE);
    if child.wait_for_exit(cooperative_deadline).is_some()
        || child.hard_kill_confirmed(deadline).is_some()
    {
        Ok(())
    } else {
        Err(child)
    }
}

fn quarantine_unconfirmed_child(
    mut child: ManagedChild,
    owner: crate::audio_lifecycle::AudioOwner,
    event_sender: &AudioWorkerEventSender,
    phase: AudioInitPhase,
) {
    let failure = AudioFailure::new(AudioFailureKind::TerminationUnconfirmed, phase);
    let _ = event_sender.send(AudioWorkerEvent::TerminationUnconfirmed { owner, failure });
    tracing::error!(
        target: "audio",
        owner = owner.telemetry_id(),
        helper_pid = child.pid(),
        error_kind = AudioFailureKind::TerminationUnconfirmed.as_str(),
        "capture helper termination could not be confirmed; retaining ownership"
    );
    while child
        .hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE)
        .is_none()
    {
        tracing::warn!(
            target: "audio",
            owner = owner.telemetry_id(),
            helper_pid = child.pid(),
            "capture helper remains under recovery ownership"
        );
    }
}

fn terminate_or_quarantine(
    child: ManagedChild,
    input: Option<std::process::ChildStdin>,
    capture_id: u64,
    nonce: SessionNonce,
    control: Option<ProductionHostMessage>,
    owner: crate::audio_lifecycle::AudioOwner,
    event_sender: &AudioWorkerEventSender,
    phase: AudioInitPhase,
) -> bool {
    match terminate_helper(child, input, capture_id, nonce, control) {
        Ok(()) => true,
        Err(child) => {
            quarantine_unconfirmed_child(child, owner, event_sender, phase);
            false
        }
    }
}

fn preferred_backends() -> [CaptureBackend; 2] {
    // Direct AUHAL avoids CPAL's synchronous Core Audio stream builder, which
    // can block indefinitely on otherwise healthy USB default inputs. The
    // helper remains the hard-kill boundary and CPAL remains the exact-device,
    // pre-buffer fallback if direct AUHAL cannot configure that device.
    #[cfg(target_os = "macos")]
    {
        [CaptureBackend::Auhal, CaptureBackend::Cpal]
    }
    #[cfg(not(target_os = "macos"))]
    {
        [CaptureBackend::Cpal, CaptureBackend::Auhal]
    }
}

// Session memo, keyed by the requested device (None = system default input).
// It adapts the attempt sequence to two observed hang pathologies without
// touching the safety contract (both backends always stay in the sequence,
// per-attempt budgets only ever shrink, termination confirmation and
// fallback-eligibility rules are unchanged, nothing is persisted, and device
// keys never reach telemetry):
//
// - Backend-bound hang (one backend hangs, the other works): the backend
//   that most recently delivered first PCM is ordered first.
// - First-attempt-bound hang (whichever backend goes first hangs in
//   AudioOutputUnitStart and the second attempt succeeds in ~160ms —
//   observed in the field on macOS 26.6/M5): promotion is proven wrong when
//   the promoted backend itself times out before first PCM, so promotion is
//   disabled for the key (sticky for the session, otherwise the order
//   oscillates and doubles the latency on CPAL-first recordings). After
//   FAST_FAIL_ARM_COUNT consecutive recordings of "primary failed before
//   first PCM, fallback delivered it within FAST_RESCUE_THRESHOLD", the
//   primary attempt budget shrinks to FAST_FAIL_PRIMARY_BUDGET so the
//   reliable rescue starts sooner. A primary success resets the counter and
//   restores full budgets.
const PROMOTED_PRIMARY_BUDGET_CAP: Duration = AUHAL_ATTEMPT_BUDGET;
const FAST_FAIL_PRIMARY_BUDGET: Duration = Duration::from_secs(2);
const FAST_RESCUE_THRESHOLD: Duration = Duration::from_secs(1);
const FAST_FAIL_ARM_COUNT: u32 = 2;

#[derive(Default)]
struct BackendMemo {
    last_ready: Option<CaptureBackend>,
    promotion_disabled: bool,
    consecutive_fast_rescues: u32,
}

static CAPTURE_MEMO: Mutex<Vec<(Option<String>, BackendMemo)>> = Mutex::new(Vec::new());

// Memo writes are deliberately not gated on capture ownership: they record
// observations about device behavior (a backend delivered PCM, a promoted
// backend hung), and an observation made by an attempt that was cancelled a
// moment later is exactly as true about the hardware. Ownership generations
// guard recording state and publication, not this ordering heuristic.
fn with_memo<R>(device_id: Option<&str>, apply: impl FnOnce(&mut BackendMemo) -> R) -> R {
    let mut memo = CAPTURE_MEMO.lock_or_recover();
    if let Some(position) = memo.iter().position(|(key, _)| key.as_deref() == device_id) {
        apply(&mut memo[position].1)
    } else {
        memo.push((device_id.map(str::to_string), BackendMemo::default()));
        apply(&mut memo.last_mut().expect("entry just pushed").1)
    }
}

fn note_first_pcm(
    device_id: Option<&str>,
    backend: CaptureBackend,
    was_primary: bool,
    start_to_first_pcm: Duration,
) {
    with_memo(device_id, |memo| {
        if was_primary {
            memo.consecutive_fast_rescues = 0;
            memo.last_ready = Some(backend);
        } else {
            memo.consecutive_fast_rescues = if start_to_first_pcm <= FAST_RESCUE_THRESHOLD {
                memo.consecutive_fast_rescues.saturating_add(1)
            } else {
                0
            };
            if !memo.promotion_disabled {
                memo.last_ready = Some(backend);
            }
        }
    });
}

fn note_promoted_primary_timeout(device_id: Option<&str>) {
    with_memo(device_id, |memo| {
        memo.promotion_disabled = true;
        memo.last_ready = None;
    });
}

fn preferred_backends_for(device_id: Option<&str>) -> [CaptureBackend; 2] {
    let default_order = preferred_backends();
    let last_ready = with_memo(device_id, |memo| {
        if memo.promotion_disabled {
            None
        } else {
            memo.last_ready
        }
    });
    match last_ready {
        Some(backend) if backend == default_order[1] => [default_order[1], default_order[0]],
        _ => default_order,
    }
}

fn primary_attempt_budget(
    backend: CaptureBackend,
    device_id: Option<&str>,
    memo_promoted: bool,
) -> Duration {
    let mut budget = backend_attempt_budget(backend);
    if memo_promoted {
        // A promoted backend must never cost more than the default primary
        // would have: cap it so a wrong promotion cannot worsen the worst
        // case beyond the default order's.
        budget = budget.min(PROMOTED_PRIMARY_BUDGET_CAP);
    }
    if with_memo(device_id, |memo| {
        memo.consecutive_fast_rescues >= FAST_FAIL_ARM_COUNT
    }) {
        budget = budget.min(FAST_FAIL_PRIMARY_BUDGET);
    }
    budget
}

#[derive(Clone, Copy)]
struct AttemptContext {
    is_primary: bool,
    memo_promoted: bool,
}

fn stop_requested_between_attempts(command_receiver: &Receiver<AudioCommand>) -> bool {
    matches!(
        command_receiver.try_recv(),
        Ok(AudioCommand::Stop) | Err(mpsc::TryRecvError::Disconnected)
    )
}

fn run_backend(
    owner: crate::audio_lifecycle::AudioOwner,
    backend: CaptureBackend,
    device_id: Option<&str>,
    fallback_to_default: bool,
    ctx: AttemptContext,
    command_receiver: &Receiver<AudioCommand>,
    shared: &Arc<Mutex<Vec<f32>>>,
    active: &Arc<AtomicBool>,
    app_handle: &Option<tauri::AppHandle>,
    event_sender: &AudioWorkerEventSender,
) -> AttemptResult {
    let started_at = Instant::now();
    let mut clock = ActiveAttemptClock::new(started_at);
    let mut permission_prompt_started = None;
    let permission_status = crate::commands::permissions::check_microphone_permission_status();
    if permission_status == "denied" {
        return AttemptResult::Failed {
            failure: AudioFailure::new(
                AudioFailureKind::PermissionDenied,
                AudioInitPhase::StreamBuild,
            ),
            retained_audio: false,
        };
    }
    if permission_status == "notDetermined" {
        begin_permission_prompt_pause(
            &mut permission_prompt_started,
            &mut clock,
            owner,
            event_sender,
            started_at,
        );
    }

    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (child, mut input, output) = match spawn_helper(capture_id, &nonce_hex) {
        Ok(value) => value,
        Err(failure) => {
            end_permission_prompt_pause(
                &mut permission_prompt_started,
                &mut clock,
                owner,
                event_sender,
                Instant::now(),
            );
            return AttemptResult::Failed {
                failure,
                retained_audio: false,
            };
        }
    };
    let output = match hello(&mut input, output, capture_id, nonce) {
        Ok(output) => output,
        Err(failure) => {
            end_permission_prompt_pause(
                &mut permission_prompt_started,
                &mut clock,
                owner,
                event_sender,
                Instant::now(),
            );
            if !terminate_or_quarantine(
                child,
                Some(input),
                capture_id,
                nonce,
                None,
                owner,
                event_sender,
                AudioInitPhase::StreamBuild,
            ) {
                return AttemptResult::TerminalHandled;
            }
            return AttemptResult::Failed {
                failure,
                retained_audio: false,
            };
        }
    };
    let start_sent_at = Instant::now();
    if write_production_control(
        &mut input,
        capture_id,
        nonce,
        &ProductionHostMessage::Start {
            device_id: device_id.map(str::to_string),
            fallback_to_default,
            backend,
        },
    )
    .is_err()
    {
        end_permission_prompt_pause(
            &mut permission_prompt_started,
            &mut clock,
            owner,
            event_sender,
            Instant::now(),
        );
        if !terminate_or_quarantine(
            child,
            Some(input),
            capture_id,
            nonce,
            None,
            owner,
            event_sender,
            AudioInitPhase::StreamBuild,
        ) {
            return AttemptResult::TerminalHandled;
        }
        return AttemptResult::Failed {
            failure: AudioFailure::new(
                AudioFailureKind::ProtocolError,
                AudioInitPhase::StreamBuild,
            ),
            retained_audio: false,
        };
    }
    tracing::info!(
        target: "audio",
        capture_id,
        backend = backend_label(backend),
        start_write_ms = start_sent_at.elapsed().as_millis() as u64,
        "capture helper start sent"
    );

    let (reader_tx, reader_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = output;
        loop {
            match read_production_frame(&mut output, capture_id, nonce) {
                Ok(frame) => {
                    if reader_tx.send(HelperRead::Frame(frame)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = reader_tx.send(HelperRead::Invalid);
                    return;
                }
            }
        }
    });

    let mut expected_sequence = 0_u64;
    let mut sample_rate = None;
    let mut retained_audio = false;
    let mut first_callback_wait_started = None;
    let mut last_level_emit = Instant::now() - Duration::from_secs(1);
    let mut last_permission_check = Instant::now() - PERMISSION_POLL_INTERVAL;
    let mut current_phase = AudioInitPhase::StreamBuild;
    let mut last_setup_step: Option<(CaptureSetupStep, SetupTransition)> = None;
    let worker_pid = child.pid();
    let mut hang_probe: Option<crate::hang_diagnostics::HangProbe> = None;
    let attempt_budget = if ctx.is_primary {
        primary_attempt_budget(backend, device_id, ctx.memo_promoted)
    } else {
        backend_attempt_budget(backend)
    };
    if attempt_budget != backend_attempt_budget(backend) {
        tracing::info!(
            target: "audio",
            capture_id,
            backend = backend_label(backend),
            attempt_budget_ms = attempt_budget.as_millis() as u64,
            "capture primary attempt budget shortened by session adaptation"
        );
    }
    loop {
        match command_receiver.try_recv() {
            Ok(AudioCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                let stop_started = Instant::now();
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    stop_started,
                );
                if terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    Some(ProductionHostMessage::Stop),
                    owner,
                    event_sender,
                    current_phase,
                ) {
                    tracing::info!(
                        target: "audio",
                        capture_id,
                        stop_to_exit_ms = stop_started.elapsed().as_millis() as u64,
                        "capture helper stopped and exited"
                    );
                    let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                    return AttemptResult::Stopped;
                }
                return AttemptResult::TerminalHandled;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        if last_permission_check.elapsed() >= PERMISSION_POLL_INTERVAL {
            let now = Instant::now();
            last_permission_check = now;
            // During active capture, denied and not-determined both mean the
            // grant is no longer in force (not-determined is what a TCC reset
            // can expose). "unknown" is a transient probe failure and must not
            // destroy retained audio by itself.
            let permission_status =
                crate::commands::permissions::check_microphone_permission_status();
            let permission_lost = permission_status == "denied"
                || (retained_audio && permission_status == "notDetermined");
            if !retained_audio && permission_status == "notDetermined" {
                begin_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    now,
                );
            } else if permission_status == "granted" {
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    now,
                );
            }
            let permission_prompt_timed_out = permission_prompt_started.is_some_and(|started| {
                now.saturating_duration_since(started) >= TCC_PROMPT_WATCHDOG
            });
            if permission_lost {
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    now,
                );
                let phase = if retained_audio {
                    AudioInitPhase::Runtime
                } else {
                    current_phase
                };
                if !terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    Some(ProductionHostMessage::Cancel),
                    owner,
                    event_sender,
                    phase,
                ) {
                    return AttemptResult::TerminalHandled;
                }
                return AttemptResult::Failed {
                    failure: AudioFailure::new(AudioFailureKind::PermissionDenied, phase),
                    retained_audio,
                };
            }
            if permission_prompt_timed_out {
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    now,
                );
                if !terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    Some(ProductionHostMessage::Cancel),
                    owner,
                    event_sender,
                    current_phase,
                ) {
                    return AttemptResult::TerminalHandled;
                }
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::PermissionPromptTimeout,
                        current_phase,
                    ),
                    retained_audio: false,
                };
            }
        }

        let now = Instant::now();
        if hang_probe.is_none()
            && !retained_audio
            && crate::hang_diagnostics::armed()
            && clock.elapsed(now) >= attempt_budget / 2
        {
            // Halfway to the budget with no PCM: sample the worker now, while
            // the blocked native call is still on its stack.
            hang_probe =
                crate::hang_diagnostics::HangProbe::start(capture_id, backend_label(backend), worker_pid);
        }
        if !retained_audio && clock.elapsed(now) >= attempt_budget {
            end_permission_prompt_pause(
                &mut permission_prompt_started,
                &mut clock,
                owner,
                event_sender,
                now,
            );
            let failure = AudioFailure::new(
                if current_phase == AudioInitPhase::FirstBufferWait {
                    AudioFailureKind::FirstBufferTimeout
                } else {
                    AudioFailureKind::InitializationTimeout
                },
                current_phase,
            );
            if ctx.is_primary && ctx.memo_promoted {
                // The promoted backend hung too, so the hang on this device
                // is first-attempt-bound rather than backend-bound; keeping
                // the promotion would oscillate the order every recording.
                note_promoted_primary_timeout(device_id);
                tracing::info!(
                    target: "audio",
                    capture_id,
                    backend = backend_label(backend),
                    "capture memo promotion disabled after the promoted backend timed out before first PCM"
                );
            }
            // last_setup_step "entered" without "completed" names the exact
            // native call the worker is stuck in (see CaptureSetupStep docs
            // for the step -> Core Audio call mapping).
            tracing::warn!(
                target: "audio",
                event_code = "audio.capture_backend_timeout",
                capture_id,
                owner = owner.telemetry_id(),
                backend = backend_label(backend),
                active_elapsed_ms = clock.elapsed(now).as_millis() as u64,
                attempt_budget_ms = attempt_budget.as_millis() as u64,
                error_kind = failure.kind.as_str(),
                last_setup_step = last_setup_step
                    .map(|(step, _)| step.as_str())
                    .unwrap_or("none"),
                last_setup_transition = last_setup_step
                    .map(|(_, transition)| transition.as_str())
                    .unwrap_or("none"),
                "capture backend exceeded its active initialization budget"
            );
            if !terminate_or_quarantine(
                child,
                Some(input),
                capture_id,
                nonce,
                Some(ProductionHostMessage::Stop),
                owner,
                event_sender,
                current_phase,
            ) {
                return AttemptResult::TerminalHandled;
            }
            if let Some(probe) = hang_probe.take() {
                probe.finish_and_ship(failure.kind.as_str());
            }
            return AttemptResult::Failed {
                failure,
                retained_audio: false,
            };
        }

        match reader_rx.recv_timeout(CAPTURE_COMMAND_POLL_INTERVAL) {
            Ok(HelperRead::Frame(ProductionFrame::Pcm(pcm))) => {
                if pcm.sequence != expected_sequence
                    || pcm.samples.is_empty()
                    || sample_rate.is_some_and(|rate| rate != pcm.sample_rate)
                {
                    end_permission_prompt_pause(
                        &mut permission_prompt_started,
                        &mut clock,
                        owner,
                        event_sender,
                        Instant::now(),
                    );
                    if !terminate_or_quarantine(
                        child,
                        Some(input),
                        capture_id,
                        nonce,
                        Some(ProductionHostMessage::Cancel),
                        owner,
                        event_sender,
                        AudioInitPhase::Runtime,
                    ) {
                        return AttemptResult::TerminalHandled;
                    }
                    return AttemptResult::Failed {
                        failure: AudioFailure::new(
                            AudioFailureKind::ProtocolError,
                            AudioInitPhase::Runtime,
                        ),
                        retained_audio,
                    };
                }
                expected_sequence += 1;
                sample_rate = Some(pcm.sample_rate);
                shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend_from_slice(&pcm.samples);
                if !retained_audio {
                    retained_audio = true;
                    current_phase = AudioInitPhase::Runtime;
                    note_first_pcm(device_id, backend, ctx.is_primary, start_sent_at.elapsed());
                    end_permission_prompt_pause(
                        &mut permission_prompt_started,
                        &mut clock,
                        owner,
                        event_sender,
                        Instant::now(),
                    );
                    tracing::info!(
                        target: "audio",
                        capture_id,
                        start_to_first_pcm_ms = start_sent_at.elapsed().as_millis() as u64,
                        "capture helper first PCM retained"
                    );
                    let _ = event_sender.send(AudioWorkerEvent::PhaseExited {
                        owner,
                        phase: AudioInitPhase::FirstBufferWait,
                        elapsed_ms: first_callback_wait_started
                            .take()
                            .map(|started: Instant| started.elapsed().as_millis() as u64)
                            .unwrap_or_default(),
                    });
                    let _ = event_sender.send(AudioWorkerEvent::FirstBuffer {
                        owner,
                        sample_rate: pcm.sample_rate,
                    });
                }
                if active.load(Ordering::Acquire)
                    && last_level_emit.elapsed() >= Duration::from_millis(AUDIO_LEVEL_THROTTLE_MS)
                {
                    if let Some(handle) = app_handle {
                        let _ = handle.emit("audio-level", compute_rms(&pcm.samples));
                    }
                    last_level_emit = Instant::now();
                }
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Phase {
                phase,
                ..
            }))) => {
                tracing::info!(
                    target: "audio",
                    capture_id,
                    backend = backend_label(backend),
                    worker_phase = ?phase,
                    start_elapsed_ms = start_sent_at.elapsed().as_millis() as u64,
                    "capture helper phase received"
                );
                if phase == CapturePhase::AwaitingFirstCallback {
                    first_callback_wait_started = Some(Instant::now());
                }
                let phase = match phase {
                    CapturePhase::Enumeration => AudioInitPhase::DeviceEnumeration,
                    CapturePhase::StreamOpen => AudioInitPhase::StreamBuild,
                    CapturePhase::AwaitingFirstCallback => AudioInitPhase::FirstBufferWait,
                    CapturePhase::Active => AudioInitPhase::Runtime,
                    CapturePhase::Stopping => AudioInitPhase::Runtime,
                };
                current_phase = phase;
                let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::SetupStep {
                    backend: reported_backend,
                    step,
                    transition,
                },
            ))) => {
                if reported_backend != backend {
                    end_permission_prompt_pause(
                        &mut permission_prompt_started,
                        &mut clock,
                        owner,
                        event_sender,
                        Instant::now(),
                    );
                    if !terminate_or_quarantine(
                        child,
                        Some(input),
                        capture_id,
                        nonce,
                        Some(ProductionHostMessage::Cancel),
                        owner,
                        event_sender,
                        current_phase,
                    ) {
                        return AttemptResult::TerminalHandled;
                    }
                    return AttemptResult::Failed {
                        failure: AudioFailure::new(AudioFailureKind::ProtocolError, current_phase),
                        retained_audio,
                    };
                }
                last_setup_step = Some((step, transition));
                tracing::info!(
                    target: "audio",
                    capture_id,
                    backend = backend_label(backend),
                    setup_step = step.as_str(),
                    setup_transition = transition.as_str(),
                    start_elapsed_ms = start_sent_at.elapsed().as_millis() as u64,
                    "capture helper setup step"
                );
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Failure {
                code,
                ..
            }))) => {
                let phase = if retained_audio {
                    AudioInitPhase::Runtime
                } else {
                    current_phase
                };
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    Instant::now(),
                );
                if !terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    None,
                    owner,
                    event_sender,
                    phase,
                ) {
                    return AttemptResult::TerminalHandled;
                }
                return AttemptResult::Failed {
                    failure: AudioFailure::new(failure_kind(code), phase),
                    retained_audio,
                };
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Stopped {
                ..
            }))) => {
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    Instant::now(),
                );
                if !terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    None,
                    owner,
                    event_sender,
                    current_phase,
                ) {
                    return AttemptResult::TerminalHandled;
                }
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                return AttemptResult::Stopped;
            }
            Ok(HelperRead::Frame(_)) => {}
            Ok(HelperRead::Invalid) | Err(RecvTimeoutError::Disconnected) => {
                end_permission_prompt_pause(
                    &mut permission_prompt_started,
                    &mut clock,
                    owner,
                    event_sender,
                    Instant::now(),
                );
                if !terminate_or_quarantine(
                    child,
                    Some(input),
                    capture_id,
                    nonce,
                    None,
                    owner,
                    event_sender,
                    current_phase,
                ) {
                    return AttemptResult::TerminalHandled;
                }
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::ProtocolError,
                        if retained_audio {
                            AudioInitPhase::Runtime
                        } else {
                            current_phase
                        },
                    ),
                    retained_audio,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn run_capture_backend_sequence(
    owner: crate::audio_lifecycle::AudioOwner,
    command_receiver: &Receiver<AudioCommand>,
    event_sender: &AudioWorkerEventSender,
    backends: [CaptureBackend; 2],
    mut run_attempt: impl FnMut(CaptureBackend) -> AttemptResult,
) {
    for (attempt_index, backend) in backends.into_iter().enumerate() {
        match run_attempt(backend) {
            AttemptResult::Stopped | AttemptResult::TerminalHandled => return,
            AttemptResult::Failed {
                failure,
                retained_audio,
            } if !retained_audio && attempt_index == 0 && failure.permits_backend_fallback() => {
                if stop_requested_between_attempts(command_receiver) {
                    tracing::info!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        "capture fallback suppressed by stop between backend attempts"
                    );
                    return;
                }
                tracing::warn!(
                    target: "audio",
                    event_code = "audio.fallback_started",
                    owner = owner.telemetry_id(),
                    from_backend = backend_label(backend),
                    to_backend = backend_label(backends[1]),
                    error_kind = failure.kind.as_str(),
                    "capture backend failed before retained audio; trying bounded fallback"
                );
            }
            AttemptResult::Failed {
                failure,
                retained_audio,
            } => {
                if !retained_audio && attempt_index == 1 {
                    tracing::error!(
                        target: "audio",
                        event_code = "audio.capture_failed",
                        owner = owner.telemetry_id(),
                        primary_backend = backend_label(backends[0]),
                        fallback_backend = backend_label(backend),
                        fallback_exhausted = true,
                        error_kind = failure.kind.as_str(),
                        "both capture backend attempts failed before first PCM"
                    );
                }
                let event = if retained_audio {
                    AudioWorkerEvent::RuntimeFailed { owner, failure }
                } else {
                    AudioWorkerEvent::InitFailed { owner, failure }
                };
                let _ = event_sender.send(event);
                return;
            }
        }
    }
}

fn run_audio_capture(spec: AudioWorkerSpec, event_sender: &AudioWorkerEventSender) {
    let AudioWorkerSpec {
        owner,
        command_receiver,
        shared,
        active,
        app_handle,
        device_id,
        fallback_to_default,
    } = spec;
    tracing::info!(
        target: "audio",
        owner = owner.telemetry_id(),
        active_budget_ms = CAPTURE_ACTIVE_BUDGET.as_millis() as u64,
        protocol_reserve_ms = CAPTURE_PROTOCOL_RESERVE.as_millis() as u64,
        "capture backend budget contract started"
    );
    let backends = preferred_backends_for(device_id.as_deref());
    let memo_promoted = backends != preferred_backends();
    if memo_promoted {
        tracing::info!(
            target: "audio",
            owner = owner.telemetry_id(),
            primary_backend = backend_label(backends[0]),
            backend_order_source = "session_first_pcm_memo",
            "capture backend order adjusted by prior first PCM in this session"
        );
    }
    run_capture_backend_sequence(
        owner,
        &command_receiver,
        event_sender,
        backends,
        |backend| {
            run_backend(
                owner,
                backend,
                device_id.as_deref(),
                fallback_to_default,
                AttemptContext {
                    is_primary: backend == backends[0],
                    memo_promoted,
                },
                &command_receiver,
                &shared,
                &active,
                &app_handle,
                event_sender,
            )
        },
    );
}

pub(crate) fn spawn_capture_worker(
    spec: AudioWorkerSpec,
    event_sender: AudioWorkerEventSender,
) -> Result<JoinHandle<()>, String> {
    thread::Builder::new()
        .name(format!("murmur-audio-{}", spec.owner.telemetry_id()))
        .spawn(move || {
            let owner = spec.owner;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_audio_capture(spec, &event_sender)
            }));
            if result.is_err() {
                let _ = event_sender.send(AudioWorkerEvent::InitFailed {
                    owner,
                    failure: AudioFailure::new(
                        AudioFailureKind::WorkerPanicked,
                        AudioInitPhase::Runtime,
                    ),
                });
            }
            let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
        })
        .map_err(|error| format!("Failed to spawn capture supervisor thread: {error}"))
}

pub fn stop_recording() -> Result<Vec<f32>, String> {
    crate::audio_lifecycle::stop_current_recording()
}

pub fn cancel_recording(reason: crate::audio_lifecycle::AudioCancelReason) -> Result<(), String> {
    crate::audio_lifecycle::cancel_current(reason)
}

pub fn is_recording() -> bool {
    crate::audio_lifecycle::is_audio_active()
}

pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }
    // The native macOS capture path normally produces 48 kHz audio. For exact
    // integer downsampling (48k -> 16k, 32k -> 16k), linear interpolation lands
    // on an original sample every time, so step_by is bit-for-bit equivalent
    // without per-sample floating-point position and interpolation work.
    if from_rate > to_rate && from_rate % to_rate == 0 {
        let step = (from_rate / to_rate) as usize;
        let new_len = samples.len() / step;
        return samples.iter().step_by(step).take(new_len).copied().collect();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let new_len = (samples.len() as f64 / ratio) as usize;
    let mut resampled = Vec::with_capacity(new_len);
    for index in 0..new_len {
        let source = index as f64 * ratio;
        let source_index = source as usize;
        let fraction = source - source_index as f64;
        let sample = if source_index + 1 < samples.len() {
            samples[source_index] * (1.0 - fraction as f32)
                + samples[source_index + 1] * fraction as f32
        } else {
            samples.get(source_index).copied().unwrap_or_default()
        };
        resampled.push(sample);
    }
    resampled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_downsampling_uses_the_same_source_positions_as_linear_interpolation() {
        let samples = (0..10).map(|value| value as f32).collect::<Vec<_>>();
        assert_eq!(resample(&samples, 48_000, 16_000), vec![0.0, 3.0, 6.0]);
        assert_eq!(resample(&samples, 32_000, 16_000), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn macos_prefers_direct_auhal_with_cpal_fallback() {
        #[cfg(target_os = "macos")]
        assert_eq!(
            preferred_backends(),
            [CaptureBackend::Auhal, CaptureBackend::Cpal]
        );

        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            preferred_backends(),
            [CaptureBackend::Cpal, CaptureBackend::Auhal]
        );
    }

    // Memo tests use distinct device keys so they stay independent of each
    // other and of the process-wide static regardless of test ordering.
    const FAST: Duration = Duration::from_millis(200);
    const SLOW: Duration = Duration::from_millis(1500);

    #[test]
    fn session_memo_promotes_the_fallback_backend_after_first_pcm() {
        let default_order = preferred_backends();
        assert_eq!(preferred_backends_for(Some("memo-device-a")), default_order);

        note_first_pcm(Some("memo-device-a"), default_order[1], false, FAST);
        assert_eq!(
            preferred_backends_for(Some("memo-device-a")),
            [default_order[1], default_order[0]]
        );
        // Other device keys are unaffected.
        assert_eq!(
            preferred_backends_for(Some("memo-device-a-other")),
            default_order
        );
    }

    #[test]
    fn session_memo_on_the_primary_backend_keeps_the_default_order() {
        let default_order = preferred_backends();
        note_first_pcm(Some("memo-device-b"), default_order[0], true, FAST);
        assert_eq!(preferred_backends_for(Some("memo-device-b")), default_order);
    }

    #[test]
    fn session_memo_tracks_the_most_recent_ready_backend() {
        let default_order = preferred_backends();
        note_first_pcm(Some("memo-device-c"), default_order[1], false, FAST);
        assert_eq!(
            preferred_backends_for(Some("memo-device-c")),
            [default_order[1], default_order[0]]
        );
        note_first_pcm(Some("memo-device-c"), default_order[0], true, FAST);
        assert_eq!(preferred_backends_for(Some("memo-device-c")), default_order);
    }

    #[test]
    fn promoted_backend_timeout_disables_promotion_for_the_session() {
        let key = Some("memo-device-guard");
        let default_order = preferred_backends();
        note_first_pcm(key, default_order[1], false, FAST);
        assert_ne!(preferred_backends_for(key), default_order);

        // The promoted backend hung too: the hang is first-attempt-bound.
        note_promoted_primary_timeout(key);
        assert_eq!(preferred_backends_for(key), default_order);

        // Sticky: a later fallback success must not re-promote and restart
        // the oscillation.
        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(preferred_backends_for(key), default_order);
    }

    #[test]
    fn repeated_fast_rescues_arm_the_short_primary_budget_and_success_resets_it() {
        let key = Some("memo-device-fastfail");
        let default_order = preferred_backends();
        let full = backend_attempt_budget(default_order[0]);

        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(primary_attempt_budget(default_order[0], key, false), full);
        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(
            primary_attempt_budget(default_order[0], key, false),
            FAST_FAIL_PRIMARY_BUDGET
        );

        // A primary success means the machine healed: full budgets return.
        note_first_pcm(key, default_order[0], true, FAST);
        assert_eq!(primary_attempt_budget(default_order[0], key, false), full);
    }

    #[test]
    fn slow_rescues_reset_the_fast_fail_counter() {
        let key = Some("memo-device-slowrescue");
        let default_order = preferred_backends();
        let full = backend_attempt_budget(default_order[0]);

        note_first_pcm(key, default_order[1], false, FAST);
        note_first_pcm(key, default_order[1], false, SLOW);
        note_first_pcm(key, default_order[1], false, FAST);
        // fast, slow, fast -> counter is 1, not 2: still the full budget.
        assert_eq!(primary_attempt_budget(default_order[0], key, false), full);
    }

    #[test]
    fn a_promoted_backend_never_gets_more_budget_than_the_default_primary() {
        assert_eq!(
            primary_attempt_budget(CaptureBackend::Cpal, Some("memo-device-cap"), true),
            PROMOTED_PRIMARY_BUDGET_CAP
        );
        // Unpromoted, CPAL keeps its own budget.
        assert_eq!(
            primary_attempt_budget(CaptureBackend::Cpal, Some("memo-device-cap"), false),
            CPAL_ATTEMPT_BUDGET
        );
    }

    #[test]
    fn worker_protocol_failures_are_actionable_and_terminal() {
        let failure =
            AudioFailure::new(AudioFailureKind::ProtocolError, AudioInitPhase::StreamBuild);

        assert_eq!(failure.kind.as_str(), "protocol_error");
        assert_eq!(
            failure.user_message(),
            "The bundled microphone capture worker failed to start. Restart Murmur and try again."
        );
        assert!(!failure.permits_backend_fallback());
    }

    #[test]
    fn device_configuration_failures_still_permit_bounded_backend_fallback() {
        for kind in [
            AudioFailureKind::DeviceUnavailable,
            AudioFailureKind::InvalidInput,
            AudioFailureKind::UnsupportedConfig,
            AudioFailureKind::BackendError,
        ] {
            assert!(AudioFailure::new(kind, AudioInitPhase::StreamBuild).permits_backend_fallback());
        }
    }

    #[test]
    fn capture_budget_split_leaves_the_decided_protocol_reserve() {
        assert_eq!(
            AUHAL_ATTEMPT_BUDGET
                + CAPTURE_TERMINATION_BUDGET
                + CPAL_ATTEMPT_BUDGET
                + CAPTURE_TERMINATION_BUDGET
                + CAPTURE_PROTOCOL_RESERVE,
            CAPTURE_ACTIVE_BUDGET
        );
    }

    #[test]
    fn active_attempt_clock_excludes_only_the_pending_prompt_interval() {
        let started = Instant::now();
        let mut clock = ActiveAttemptClock::new(started);
        clock.pause(started + Duration::from_secs(2));
        assert_eq!(
            clock.elapsed(started + Duration::from_secs(40)),
            Duration::from_secs(2)
        );
        clock.resume(started + Duration::from_secs(42));
        assert_eq!(
            clock.elapsed(started + Duration::from_secs(45)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn permission_and_unconfirmed_termination_never_permit_fallback() {
        for kind in [
            AudioFailureKind::PermissionDenied,
            AudioFailureKind::PermissionPromptTimeout,
            AudioFailureKind::TerminationUnconfirmed,
        ] {
            assert!(
                !AudioFailure::new(kind, AudioInitPhase::StreamBuild).permits_backend_fallback()
            );
        }
    }

    #[test]
    fn stop_is_consumed_before_a_fallback_attempt_can_spawn() {
        let (sender, receiver) = mpsc::channel();
        assert!(!stop_requested_between_attempts(&receiver));
        sender.send(AudioCommand::Stop).unwrap();
        assert!(stop_requested_between_attempts(&receiver));
    }

    #[test]
    fn confirmed_primary_timeout_advances_once_to_successful_fallback() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();

        run_capture_backend_sequence(
            crate::audio_lifecycle::AudioOwner::Dictation(1),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            |backend| {
                calls.push(backend);
                if backend == CaptureBackend::Auhal {
                    // Failed is emitted by the production runner only after
                    // terminate_or_quarantine positively confirms exit.
                    AttemptResult::Failed {
                        failure: AudioFailure::new(
                            AudioFailureKind::InitializationTimeout,
                            AudioInitPhase::StreamBuild,
                        ),
                        retained_audio: false,
                    }
                } else {
                    AttemptResult::Stopped
                }
            },
        );

        assert_eq!(calls, vec![CaptureBackend::Auhal, CaptureBackend::Cpal]);
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn unconfirmed_primary_termination_never_reaches_fallback() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let event_sender = AudioWorkerEventSender::new(|_| Ok(()));
        let mut calls = Vec::new();
        run_capture_backend_sequence(
            crate::audio_lifecycle::AudioOwner::Dictation(2),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            |backend| {
                calls.push(backend);
                AttemptResult::TerminalHandled
            },
        );
        assert_eq!(calls, vec![CaptureBackend::Auhal]);
    }

    #[test]
    fn two_backend_timeouts_emit_one_terminal_initialization_failure() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();
        run_capture_backend_sequence(
            crate::audio_lifecycle::AudioOwner::Dictation(3),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            |backend| {
                calls.push(backend);
                AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::InitializationTimeout,
                        AudioInitPhase::StreamBuild,
                    ),
                    retained_audio: false,
                }
            },
        );

        assert_eq!(calls, vec![CaptureBackend::Auhal, CaptureBackend::Cpal]);
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AudioWorkerEvent::InitFailed {
                failure: AudioFailure {
                    kind: AudioFailureKind::InitializationTimeout,
                    ..
                },
                ..
            }]
        ));
    }

    #[test]
    fn retained_pcm_disables_fallback() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();
        run_capture_backend_sequence(
            crate::audio_lifecycle::AudioOwner::Dictation(4),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            |backend| {
                calls.push(backend);
                AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::StreamInvalidated,
                        AudioInitPhase::Runtime,
                    ),
                    retained_audio: true,
                }
            },
        );
        assert_eq!(calls, vec![CaptureBackend::Auhal]);
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AudioWorkerEvent::RuntimeFailed { .. }]
        ));
    }
}
