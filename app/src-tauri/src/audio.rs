use crate::managed_child::{bundled_sibling, ManagedChild};
use crate::microphone_preview::{
    can_schedule_vad_analysis, classify_level, schedule_vad_analysis, MicrophonePreviewLevel,
    PreviewLevelAccumulator, PreviewLevelTracker, PreviewVadWindow,
};
use crate::MutexExt;
use murmur_capture_helper_protocol::{
    read_production_frame, valid_input_resolution_evidence, write_production_control,
    CaptureBackend, CaptureChannel, CapturePhase, CaptureSetupStep, FailureCode, ProductionFrame,
    ProductionHelperMessage, ProductionHostMessage, SessionNonce, SetupTransition,
};
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
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
const INVENTORY_QUARANTINE_RETRY_LIMIT: usize = 2;
#[cfg(target_os = "macos")]
const INVENTORY_REAPER_SHUTDOWN_DRAIN: Duration = Duration::from_secs(2);
const HELPER_CONTROL_DEADLINE: Duration = Duration::from_secs(3);
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const TCC_PROMPT_WATCHDOG: Duration = Duration::from_secs(120);
const AUHAL_ATTEMPT_BUDGET: Duration = Duration::from_secs(8);
const CPAL_ATTEMPT_BUDGET: Duration = Duration::from_secs(16);
const DEVICE_RERESOLUTION_DELAY: Duration = Duration::from_millis(500);
const DEVICE_RERESOLUTION_MAX_PASSES: usize = 3;
const COOPERATIVE_STOP_GRACE: Duration = Duration::from_millis(250);
const CAPTURE_TERMINATION_BUDGET: Duration = Duration::from_secs(2);
const CAPTURE_PROTOCOL_RESERVE: Duration = Duration::from_secs(2);
const CAPTURE_ACTIVE_BUDGET: Duration = Duration::from_secs(30);
const CAPTURE_WORKER_IDENTIFIER: &str = "com.localdictation.capture-worker";
static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);
static INVENTORY_HELPERS_IN_DETACHED_REAP: AtomicUsize = AtomicUsize::new(0);

struct InventoryHelperSpawnGate {
    lock: Mutex<()>,
    reaper_changed: Condvar,
}

fn inventory_helper_spawn_gate() -> &'static InventoryHelperSpawnGate {
    static GATE: OnceLock<InventoryHelperSpawnGate> = OnceLock::new();
    GATE.get_or_init(|| InventoryHelperSpawnGate {
        lock: Mutex::new(()),
        reaper_changed: Condvar::new(),
    })
}

struct InventoryReaperShared {
    queue: Mutex<VecDeque<ManagedChild>>,
    changed: Condvar,
    accepting: AtomicBool,
    #[cfg(target_os = "macos")]
    shutdown: AtomicBool,
}

impl Default for InventoryReaperShared {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            changed: Condvar::new(),
            accepting: AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            shutdown: AtomicBool::new(false),
        }
    }
}

struct InventoryReaperService {
    join: JoinHandle<()>,
}

fn inventory_reaper_shared() -> &'static Arc<InventoryReaperShared> {
    static SHARED: OnceLock<Arc<InventoryReaperShared>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::new(InventoryReaperShared::default()))
}

fn inventory_reaper_service_slot() -> &'static Mutex<Option<InventoryReaperService>> {
    static SERVICE: OnceLock<Mutex<Option<InventoryReaperService>>> = OnceLock::new();
    SERVICE.get_or_init(|| Mutex::new(None))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioFailureKind {
    PermissionDenied,
    DeviceUnavailable,
    HostUnavailable,
    InvalidInput,
    ResourceExhausted,
    StreamInvalidated,
    UnsupportedConfig,
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
            Self::HostUnavailable => "host_unavailable",
            Self::InvalidInput => "invalid_input",
            Self::ResourceExhausted => "resource_exhausted",
            Self::StreamInvalidated => "stream_invalidated",
            Self::UnsupportedConfig => "unsupported_config",
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
        FailureCode::UnsupportedOs => AudioFailureKind::UnsupportedConfig,
        FailureCode::SystemAudioUnavailable => AudioFailureKind::HostUnavailable,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnumeratedAudioInputInventory {
    pub(crate) devices: Vec<AudioDeviceDescriptor>,
    pub(crate) default_input_id: Option<String>,
}

pub(crate) enum AudioCommand {
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioInitPhase {
    DeviceEnumeration,
    StreamBuild,
    FirstBufferWait,
    Runtime,
}

impl AudioInitPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeviceEnumeration => "device_enumeration",
            Self::StreamBuild => "stream_build",
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
    StartupDiagnostic {
        owner: crate::audio_lifecycle::AudioOwner,
        diagnostic: AudioStartupDiagnostic,
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

/// Content-free observations from the production capture worker. These are
/// emitted only for the bounded microphone startup benchmark; regular capture
/// owners keep their existing lifecycle/event surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioStartupDiagnostic {
    BackendPlan {
        primary: CaptureBackend,
        fallback: CaptureBackend,
        source: AudioBackendOrderSource,
    },
    AttemptStarted {
        backend: CaptureBackend,
        resolution_pass: u8,
        attempt_index: u8,
        attempt_budget_ms: u64,
    },
    SetupStep {
        backend: CaptureBackend,
        resolution_pass: u8,
        attempt_index: u8,
        step: CaptureSetupStep,
        transition: SetupTransition,
    },
    FirstPcm {
        backend: CaptureBackend,
        resolution_pass: u8,
        attempt_index: u8,
        attempt_start_to_first_pcm_ms: u64,
        active_elapsed_ms: u64,
    },
    CycleReady {
        cycle_start_to_first_pcm_ms: u64,
    },
    AttemptFailed {
        backend: CaptureBackend,
        resolution_pass: u8,
        attempt_index: u8,
        active_elapsed_ms: u64,
        failure_kind: AudioFailureKind,
        failure_phase: AudioInitPhase,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioBackendOrderSource {
    Default,
    SessionFirstPcmMemo,
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

#[cfg(debug_assertions)]
fn capture_fault_for_scenario(
    scenario: Option<&str>,
    create_sentinel: impl FnOnce() -> bool,
) -> Option<&'static str> {
    match scenario {
        Some("hang_stream_build") => Some("hang-stream-build"),
        Some("hang_stream_build_once") if create_sentinel() => Some("hang-stream-build"),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn requested_capture_fault() -> Option<&'static str> {
    let scenario = std::env::var("MURMUR_AUDIO_TEST_SCENARIO").ok();
    capture_fault_for_scenario(scenario.as_deref(), || {
        std::env::var_os("MURMUR_AUDIO_TEST_SENTINEL").is_some_and(|path| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .is_ok()
        })
    })
}

#[cfg(not(debug_assertions))]
fn requested_capture_fault() -> Option<&'static str> {
    None
}

fn spawn_helper(
    capture_id: u64,
    nonce_hex: &str,
    fault: Option<&str>,
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
    let mut arguments = vec!["--production-v6", capture_id_text.as_str(), nonce_hex];
    if let Some(fault) = fault {
        arguments.extend(["--fault", fault]);
    }
    let spawn_started = Instant::now();
    let child = ManagedChild::spawn_with_arguments(&path, &arguments, &[]).map_err(|_| {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryHelperTermination {
    ConfirmedWithinBudget,
    ConfirmedDuringQuarantine,
    DetachedReaperRequired,
}

fn bounded_inventory_quarantine(
    initially_confirmed: bool,
    retry_limit: usize,
    mut retry: impl FnMut() -> bool,
) -> InventoryHelperTermination {
    if initially_confirmed {
        return InventoryHelperTermination::ConfirmedWithinBudget;
    }
    for _ in 0..retry_limit {
        if retry() {
            return InventoryHelperTermination::ConfirmedDuringQuarantine;
        }
    }
    InventoryHelperTermination::DetachedReaperRequired
}

fn inventory_helper_reaper_active() -> bool {
    INVENTORY_HELPERS_IN_DETACHED_REAP.load(Ordering::Acquire) != 0
}

fn inventory_helper_publish_allowed() -> bool {
    let _spawn_guard = inventory_helper_spawn_gate().lock.lock_or_recover();
    !inventory_helper_reaper_active()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryHelperSpawnAction {
    Spawn,
    WaitForReaper,
    Refuse,
}

fn inventory_helper_spawn_action(
    reaper_service_ready: bool,
    reaper_active: bool,
    shutdown: Option<bool>,
) -> InventoryHelperSpawnAction {
    if shutdown == Some(true) || !reaper_service_ready {
        InventoryHelperSpawnAction::Refuse
    } else if !reaper_active {
        InventoryHelperSpawnAction::Spawn
    } else if shutdown == Some(false) {
        InventoryHelperSpawnAction::WaitForReaper
    } else {
        InventoryHelperSpawnAction::Refuse
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryReaperServiceAction {
    Wait,
    Reap,
    Exit,
}

#[cfg(any(target_os = "macos", test))]
fn inventory_reaper_service_action(
    queue_is_empty: bool,
    shutdown: bool,
) -> InventoryReaperServiceAction {
    if !queue_is_empty {
        InventoryReaperServiceAction::Reap
    } else if shutdown {
        InventoryReaperServiceAction::Exit
    } else {
        InventoryReaperServiceAction::Wait
    }
}

fn inventory_reaper_service_accepts(
    registered: bool,
    accepting: bool,
    thread_finished: bool,
) -> bool {
    registered && accepting && !thread_finished
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryReaperShutdownAction {
    Join,
    Wait,
    Detach,
}

#[cfg(any(target_os = "macos", test))]
fn inventory_reaper_shutdown_action(
    thread_finished: bool,
    now: Instant,
    deadline: Instant,
) -> InventoryReaperShutdownAction {
    if thread_finished {
        InventoryReaperShutdownAction::Join
    } else if now >= deadline {
        InventoryReaperShutdownAction::Detach
    } else {
        InventoryReaperShutdownAction::Wait
    }
}

fn inventory_reaper_service_ready() -> bool {
    let slot = inventory_reaper_service_slot().lock_or_recover();
    let Some(service) = slot.as_ref() else {
        return false;
    };
    inventory_reaper_service_accepts(
        true,
        inventory_reaper_shared().accepting.load(Ordering::Acquire),
        service.join.is_finished(),
    )
}

#[cfg(target_os = "macos")]
fn run_inventory_reaper_service(shared: Arc<InventoryReaperShared>) {
    loop {
        let child = {
            let mut queue = shared.queue.lock_or_recover();
            loop {
                match inventory_reaper_service_action(
                    queue.is_empty(),
                    shared.shutdown.load(Ordering::Acquire),
                ) {
                    InventoryReaperServiceAction::Reap => break queue.pop_front(),
                    InventoryReaperServiceAction::Exit => {
                        shared.accepting.store(false, Ordering::Release);
                        return;
                    }
                    InventoryReaperServiceAction::Wait => {
                        queue = shared
                            .changed
                            .wait(queue)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                }
            }
        };
        let Some(mut child) = child else {
            continue;
        };
        while child
            .hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE)
            .is_none()
        {}
        let gate = inventory_helper_spawn_gate();
        let _spawn_guard = gate.lock.lock_or_recover();
        INVENTORY_HELPERS_IN_DETACHED_REAP.fetch_sub(1, Ordering::AcqRel);
        gate.reaper_changed.notify_all();
    }
}

/// Start the single app-owned inventory reaper before any watcher or
/// enumeration helper can be created. A failed service spawn leaves accepting
/// false, so every helper request fails closed.
#[cfg(target_os = "macos")]
pub(crate) fn start_inventory_helper_reaper_service() -> bool {
    let mut slot = inventory_reaper_service_slot().lock_or_recover();
    if slot.is_some() {
        return inventory_reaper_service_accepts(
            true,
            inventory_reaper_shared().accepting.load(Ordering::Acquire),
            slot.as_ref()
                .is_some_and(|service| service.join.is_finished()),
        );
    }
    let shared = Arc::clone(inventory_reaper_shared());
    shared.shutdown.store(false, Ordering::Release);
    let worker_shared = Arc::clone(&shared);
    let spawn = thread::Builder::new()
        .name("murmur-inventory-reaper".to_string())
        .spawn(move || run_inventory_reaper_service(worker_shared));
    match spawn {
        Ok(join) => {
            shared.accepting.store(true, Ordering::Release);
            *slot = Some(InventoryReaperService { join });
            true
        }
        Err(_) => false,
    }
}

/// Signal the app-owned reaper after every producer has joined, then wait only
/// for the fixed shutdown budget. An unfinished service handle is detached at
/// process exit; queued/active ManagedChild ownership remains in the service,
/// and the worker parent watchdog is the final safety boundary.
#[cfg(target_os = "macos")]
pub(crate) fn shutdown_inventory_helper_reaper_service() {
    let Some(service) = inventory_reaper_service_slot().lock_or_recover().take() else {
        return;
    };
    let shared = inventory_reaper_shared();
    shared.accepting.store(false, Ordering::Release);
    shared.shutdown.store(true, Ordering::Release);
    shared.changed.notify_all();

    let deadline = Instant::now() + INVENTORY_REAPER_SHUTDOWN_DRAIN;
    loop {
        match inventory_reaper_shutdown_action(service.join.is_finished(), Instant::now(), deadline)
        {
            InventoryReaperShutdownAction::Join => {
                let _ = service.join.join();
                return;
            }
            InventoryReaperShutdownAction::Wait => thread::sleep(Duration::from_millis(10)),
            InventoryReaperShutdownAction::Detach => {
                tracing::warn!(
                    target: "audio",
                    active_reaper_count = INVENTORY_HELPERS_IN_DETACHED_REAP
                        .load(Ordering::Acquire),
                    "microphone inventory reaper exceeded the bounded shutdown drain"
                );
                drop(service.join);
                return;
            }
        }
    }
}

fn transfer_inventory_helper_to_detached_reaper(child: ManagedChild) {
    let pid = child.pid();
    INVENTORY_HELPERS_IN_DETACHED_REAP.fetch_add(1, Ordering::AcqRel);
    crate::audio_inventory::helper_entered_detached_reap();
    tracing::error!(
        target: "audio",
        event_code = "audio.input_inventory_helper_detached_reap",
        helper_pid = pid,
        "microphone inventory helper transferred to detached termination reaper"
    );

    let shared = inventory_reaper_shared();
    shared.queue.lock_or_recover().push_back(child);
    shared.changed.notify_one();
}

fn spawn_inventory_helper(
    capture_id: u64,
    nonce_hex: &str,
    shutdown: Option<&AtomicBool>,
) -> Result<
    (
        ManagedChild,
        std::process::ChildStdin,
        std::process::ChildStdout,
    ),
    String,
> {
    let gate = inventory_helper_spawn_gate();
    let mut guard = gate.lock.lock_or_recover();
    loop {
        let shutdown_requested = shutdown.map(|shutdown| shutdown.load(Ordering::Acquire));
        let reaper_service_ready = inventory_reaper_service_ready();
        match inventory_helper_spawn_action(
            reaper_service_ready,
            inventory_helper_reaper_active(),
            shutdown_requested,
        ) {
            InventoryHelperSpawnAction::Spawn => break,
            InventoryHelperSpawnAction::Refuse => {
                return Err(if shutdown_requested == Some(true) {
                    "Microphone inventory recovery stopped during shutdown.".to_string()
                } else if !reaper_service_ready {
                    "Microphone inventory reaper is unavailable.".to_string()
                } else {
                    "Microphone inventory recovery is still pending.".to_string()
                });
            }
            InventoryHelperSpawnAction::WaitForReaper => {}
        }
        let (next_guard, _) = gate
            .reaper_changed
            .wait_timeout(guard, Duration::from_millis(50))
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard = next_guard;
    }
    spawn_helper(capture_id, nonce_hex, None).map_err(|failure| failure.to_string())
}

/// Return whether the helper exited within its ordinary bounded budget. A
/// delayed confirmation still fails the current operation. If the bounded
/// quarantine also expires, ownership moves to a registered process-lifetime
/// reaper and the inventory/watch spawn gate remains closed until confirmation.
fn confirm_or_quarantine_inventory_helper(mut child: ManagedChild) -> bool {
    // Close the replacement-spawn race before termination begins. Existing
    // passive watcher/enumerator children may finish concurrently, but no new
    // inventory helper can pass spawn_inventory_helper until this child is
    // confirmed or its owned detached-reaper state is visible.
    let _spawn_guard = inventory_helper_spawn_gate().lock.lock_or_recover();
    let initially_confirmed = child
        .wait_for_exit(Instant::now() + HELPER_STOP_DEADLINE)
        .or_else(|| child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE))
        .is_some();
    if !initially_confirmed {
        tracing::error!(
            target: "audio",
            helper_pid = child.pid(),
            "microphone inventory helper entered termination quarantine"
        );
    }
    let termination = bounded_inventory_quarantine(
        initially_confirmed,
        INVENTORY_QUARANTINE_RETRY_LIMIT,
        || {
            let confirmed = child
                .hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE)
                .is_some();
            if !confirmed {
                tracing::warn!(
                    target: "audio",
                    helper_pid = child.pid(),
                    "microphone inventory helper remains in termination quarantine"
                );
            }
            confirmed
        },
    );
    match termination {
        InventoryHelperTermination::ConfirmedWithinBudget => true,
        InventoryHelperTermination::ConfirmedDuringQuarantine => false,
        InventoryHelperTermination::DetachedReaperRequired => {
            transfer_inventory_helper_to_detached_reaper(child);
            false
        }
    }
}

pub(crate) fn enumerate_input_devices() -> Result<EnumeratedAudioInputInventory, String> {
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (child, mut input, output) = spawn_inventory_helper(capture_id, &nonce_hex, None)?;
    let output = match hello(&mut input, output, capture_id, nonce) {
        Ok(output) => output,
        Err(failure) => {
            drop(input);
            let _ = confirm_or_quarantine_inventory_helper(child);
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
        drop(input);
        let _ = confirm_or_quarantine_inventory_helper(child);
        return Err("Failed to request microphone enumeration.".to_string());
    }
    let (frame, output) = match read_control_frame_with_deadline(output, capture_id, nonce) {
        Ok(result) => result,
        Err(failure) => {
            drop(input);
            let _ = confirm_or_quarantine_inventory_helper(child);
            return Err(failure.to_string());
        }
    };
    let inventory = match frame {
        ProductionFrame::Control(ProductionHelperMessage::Devices {
            devices,
            default_input_id,
        }) => EnumeratedAudioInputInventory {
            devices: devices
                .into_iter()
                .map(|device| AudioDeviceDescriptor {
                    id: device.id,
                    name: device.name,
                })
                .collect(),
            default_input_id,
        },
        _ => {
            drop(input);
            let _ = confirm_or_quarantine_inventory_helper(child);
            return Err("The capture worker returned an invalid device list.".to_string());
        }
    };
    drop((input, output));
    let termination_confirmed = confirm_or_quarantine_inventory_helper(child);
    inventory_after_confirmed_helper_exit(
        inventory,
        termination_confirmed,
        !inventory_helper_publish_allowed(),
    )
}

fn inventory_after_confirmed_helper_exit(
    inventory: EnumeratedAudioInputInventory,
    termination_confirmed: bool,
    another_helper_in_detached_reap: bool,
) -> Result<EnumeratedAudioInputInventory, String> {
    (termination_confirmed && !another_helper_in_detached_reap)
        .then_some(inventory)
        .ok_or_else(|| "The microphone inventory worker could not be reaped safely.".to_string())
}

#[cfg(any(target_os = "macos", test))]
fn should_forward_input_topology_change(ready: bool, stop_sent: bool) -> bool {
    ready && !stop_sent
}

#[cfg(any(target_os = "macos", test))]
fn input_topology_watch_timeout(
    ready: bool,
    stop_sent: bool,
    now: Instant,
    ready_deadline: Instant,
    stop_deadline: Option<Instant>,
) -> Option<&'static str> {
    if stop_deadline.is_some_and(|deadline| now >= deadline) {
        Some("The microphone topology watcher did not stop in time.")
    } else if !ready && !stop_sent && now >= ready_deadline {
        Some("The microphone topology watcher did not become ready.")
    } else {
        None
    }
}

/// Own one passive topology watcher until shutdown or protocol failure. The
/// callback is invoked only for a content-free invalidation; callers decide
/// when it is safe to enumerate.
#[cfg(target_os = "macos")]
pub(crate) fn run_input_topology_watch(
    shutdown: &AtomicBool,
    mut on_changed: impl FnMut(),
) -> Result<(), String> {
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (child, mut input, output) =
        spawn_inventory_helper(capture_id, &nonce_hex, Some(shutdown))?;
    let output = match hello(&mut input, output, capture_id, nonce) {
        Ok(output) => output,
        Err(failure) => {
            drop(input);
            let _ = confirm_or_quarantine_inventory_helper(child);
            return Err(failure.to_string());
        }
    };
    if write_production_control(
        &mut input,
        capture_id,
        nonce,
        &ProductionHostMessage::WatchInputTopology,
    )
    .is_err()
    {
        drop(input);
        let _ = confirm_or_quarantine_inventory_helper(child);
        return Err("Failed to start the microphone topology watcher.".to_string());
    }

    let (reader_sender, reader_receiver) = mpsc::channel();
    let reader_spawn = thread::Builder::new()
        .name("murmur-input-topology-reader".to_string())
        .spawn(move || {
            let mut output = output;
            loop {
                match read_production_frame(&mut output, capture_id, nonce) {
                    Ok(frame) => {
                        if reader_sender.send(Some(frame)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = reader_sender.send(None);
                        return;
                    }
                }
            }
        });
    if reader_spawn.is_err() {
        drop(input);
        let _ = confirm_or_quarantine_inventory_helper(child);
        return Err("Failed to supervise the microphone topology watcher.".to_string());
    }

    let ready_deadline = Instant::now() + HELPER_CONTROL_DEADLINE;
    let mut ready = false;
    let mut stop_sent = false;
    let mut stop_deadline = None;
    let result = loop {
        if shutdown.load(Ordering::Acquire) && !stop_sent {
            stop_sent = true;
            if write_production_control(&mut input, capture_id, nonce, &ProductionHostMessage::Stop)
                .is_err()
            {
                break Err("Failed to stop the microphone topology watcher.".to_string());
            }
            stop_deadline = Some(Instant::now() + HELPER_STOP_DEADLINE);
        }
        if let Some(message) = input_topology_watch_timeout(
            ready,
            stop_sent,
            Instant::now(),
            ready_deadline,
            stop_deadline,
        ) {
            break Err(message.to_string());
        }
        match reader_receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Some(ProductionFrame::Control(
                ProductionHelperMessage::InputTopologyWatchReady,
            ))) if !ready => {
                ready = true;
                // Establishing the listener is itself an invalidation fence:
                // refresh after it is live so no startup topology change can
                // fall into a gap between enumeration and subscription.
                on_changed();
            }
            Ok(Some(ProductionFrame::Control(ProductionHelperMessage::InputTopologyChanged)))
                if ready =>
            {
                if should_forward_input_topology_change(ready, stop_sent) {
                    on_changed();
                }
            }
            Ok(Some(ProductionFrame::Control(ProductionHelperMessage::Stopped { .. })))
                if stop_sent =>
            {
                break Ok(())
            }
            Ok(Some(_)) | Ok(None) => {
                break Err("The microphone topology watcher stopped unexpectedly.".to_string())
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err("The microphone topology watcher disconnected.".to_string())
            }
        }
    };

    drop(input);
    let terminated = confirm_or_quarantine_inventory_helper(child);
    if !terminated {
        return Err("The microphone topology watcher could not be reaped.".to_string());
    }
    result
}

pub fn list_input_devices() -> Result<Vec<AudioDeviceDescriptor>, String> {
    crate::audio_inventory::available_devices()
}

pub fn start_transform_capture_audio(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    transform_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_transform_recording(app_handle, device_id, transform_pass_id)
}

pub fn start_query_capture_audio(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    query_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_query_recording(app_handle, device_id, query_pass_id)
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
        active_elapsed_ms: u64,
    },
}

struct BackendPassFailure {
    failure: AudioFailure,
    retained_audio: bool,
    fallback_exhausted: bool,
    both_backends_device_unavailable: bool,
}

enum BackendPassResult {
    Stopped,
    TerminalHandled,
    Failed(BackendPassFailure),
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

// These arguments mirror the authenticated helper shutdown frame plus the
// ownership evidence needed when termination cannot be confirmed.
#[allow(clippy::too_many_arguments)]
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
//
// The fast-fail shrink has a second, deeper tier. Once the same pattern has
// repeated FAST_FAIL_DEEP_ARM_COUNT times in a row the primary budget shrinks
// again to FAST_FAIL_DEEP_PRIMARY_BUDGET. The deeper arm count is what makes
// this safe: a slow-but-working primary would have to die before first PCM in
// four consecutive recordings — each time rescued inside FAST_RESCUE_THRESHOLD
// — before it is cut off, so one flukey recording can never trigger it. A
// healthy start reaches first PCM in roughly 300ms on known-good machines, so
// 750ms still leaves headroom for a primary that is merely slow. As in the
// first tier the budget is only ever taken as a minimum, so it can shrink and
// never grow, and a single primary success clears consecutive_fast_rescues in
// note_first_pcm, which drops out of both tiers at once and restores the full
// per-backend budgets. On an affected machine this takes the steady-state dead
// time from about 2.45s per recording (2s sacrificial primary + ~250ms
// confirmed termination + ~200ms rescue) to about 1.2s.
const PROMOTED_PRIMARY_BUDGET_CAP: Duration = AUHAL_ATTEMPT_BUDGET;
const FAST_FAIL_PRIMARY_BUDGET: Duration = Duration::from_secs(2);
const FAST_RESCUE_THRESHOLD: Duration = Duration::from_secs(1);
const FAST_FAIL_ARM_COUNT: u32 = 2;
const FAST_FAIL_DEEP_ARM_COUNT: u32 = 4;
const FAST_FAIL_DEEP_PRIMARY_BUDGET: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BackendMemo {
    last_ready: Option<CaptureBackend>,
    promotion_disabled: bool,
    consecutive_fast_rescues: u32,
}

static CAPTURE_MEMO: Mutex<Vec<(Option<String>, BackendMemo)>> = Mutex::new(Vec::new());

// Normal production owners update this memo from observed device behavior.
// Diagnostic benchmark capture is explicitly excluded at each write site so
// measuring startup cannot retrain later dictation; all policy reads remain
// non-inserting as well.
fn with_memo<R>(device_id: Option<&str>, apply: impl FnOnce(&mut BackendMemo) -> R) -> R {
    let mut memo = CAPTURE_MEMO.lock_or_recover();
    if let Some(position) = memo.iter().position(|(key, _)| key.as_deref() == device_id) {
        apply(&mut memo[position].1)
    } else {
        memo.push((device_id.map(str::to_string), BackendMemo::default()));
        apply(&mut memo.last_mut().expect("entry just pushed").1)
    }
}

fn read_memo<R>(device_id: Option<&str>, apply: impl FnOnce(Option<&BackendMemo>) -> R) -> R {
    let memo = CAPTURE_MEMO.lock_or_recover();
    apply(
        memo.iter()
            .find(|(key, _)| key.as_deref() == device_id)
            .map(|(_, value)| value),
    )
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

fn should_update_capture_memo(owner: crate::audio_lifecycle::AudioOwner) -> bool {
    !owner.is_microphone_benchmark()
}

fn preferred_backends_for(device_id: Option<&str>) -> [CaptureBackend; 2] {
    let default_order = preferred_backends();
    let last_ready = read_memo(device_id, |memo| {
        memo.and_then(|memo| {
            if memo.promotion_disabled {
                None
            } else {
                memo.last_ready
            }
        })
    });
    match last_ready {
        Some(backend) if backend == default_order[1] => [default_order[1], default_order[0]],
        _ => default_order,
    }
}

pub(crate) fn microphone_startup_backend_plan(device_id: Option<&str>) -> AudioStartupDiagnostic {
    let backends = preferred_backends_for(device_id);
    AudioStartupDiagnostic::BackendPlan {
        primary: backends[0],
        fallback: backends[1],
        source: if backends != preferred_backends() {
            AudioBackendOrderSource::SessionFirstPcmMemo
        } else {
            AudioBackendOrderSource::Default
        },
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
    let consecutive_fast_rescues = read_memo(device_id, |memo| {
        memo.map_or(0, |memo| memo.consecutive_fast_rescues)
    });
    if consecutive_fast_rescues >= FAST_FAIL_ARM_COUNT {
        budget = budget.min(FAST_FAIL_PRIMARY_BUDGET);
    }
    if consecutive_fast_rescues >= FAST_FAIL_DEEP_ARM_COUNT {
        budget = budget.min(FAST_FAIL_DEEP_PRIMARY_BUDGET);
    }
    budget
}

#[derive(Clone, Copy)]
struct AttemptContext {
    is_primary: bool,
    memo_promoted: bool,
    resolution_pass: usize,
    backend_attempt: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InputResolutionState {
    #[default]
    NotStarted,
    Started,
    Observed,
    Completed,
}

#[derive(Clone, Copy)]
struct ReportedInputResolution {
    input_enumeration_ok: bool,
    requested_present: Option<bool>,
    input_device_count: u16,
    input_device_count_capped: bool,
}

#[derive(Default)]
struct InputResolutionTracker {
    state: InputResolutionState,
    requested_device: bool,
    evidence: Option<ReportedInputResolution>,
}

impl InputResolutionTracker {
    fn observe_setup(&mut self, step: CaptureSetupStep, transition: SetupTransition) -> bool {
        match (step, transition, self.state) {
            (
                CaptureSetupStep::DeviceResolution,
                SetupTransition::Entered,
                InputResolutionState::NotStarted,
            ) => self.state = InputResolutionState::Started,
            (
                CaptureSetupStep::DeviceResolution,
                SetupTransition::Completed,
                InputResolutionState::Observed,
            ) => self.state = InputResolutionState::Completed,
            (CaptureSetupStep::DeviceResolution, _, _) => return false,
            (_, _, InputResolutionState::Completed) => {}
            _ => return false,
        }
        true
    }

    fn observe_evidence(
        &mut self,
        expected_backend: CaptureBackend,
        reported_backend: CaptureBackend,
        requested_device: bool,
        evidence: ReportedInputResolution,
    ) -> bool {
        if reported_backend != expected_backend
            || self.state != InputResolutionState::Started
            || !valid_input_resolution_evidence(
                requested_device,
                evidence.input_enumeration_ok,
                evidence.requested_present,
                evidence.input_device_count,
                evidence.input_device_count_capped,
            )
        {
            return false;
        }
        self.requested_device = requested_device;
        self.evidence = Some(evidence);
        self.state = InputResolutionState::Observed;
        true
    }

    fn permits_failure(&self, code: FailureCode) -> bool {
        match self.state {
            InputResolutionState::Observed => {
                let Some(evidence) = self.evidence else {
                    return false;
                };
                match code {
                    FailureCode::NoInputDevice if self.requested_device => {
                        evidence.input_enumeration_ok && evidence.requested_present == Some(false)
                    }
                    FailureCode::NoInputDevice => {
                        // The selected system default and the supplementary
                        // topology facts come from separate native queries.
                        // A topology/API race can therefore report any
                        // enumeration/default facts while selection itself
                        // truthfully returns no device. System-default mode
                        // never qualifies for pinned-device retry.
                        evidence.requested_present.is_none()
                    }
                    FailureCode::EnumerationFailed => !evidence.input_enumeration_ok,
                    _ => false,
                }
            }
            InputResolutionState::Completed => !matches!(
                code,
                FailureCode::NoInputDevice | FailureCode::EnumerationFailed
            ),
            InputResolutionState::NotStarted => code == FailureCode::PermissionDenied,
            InputResolutionState::Started => false,
        }
    }

    fn permits_phase(
        &self,
        expected_backend: CaptureBackend,
        reported_backend: CaptureBackend,
        phase: CapturePhase,
    ) -> bool {
        expected_backend == reported_backend
            && (!matches!(
                phase,
                CapturePhase::AwaitingFirstCallback | CapturePhase::Active
            ) || self.state == InputResolutionState::Completed)
    }

    fn permits_pcm(&self) -> bool {
        self.state == InputResolutionState::Completed
    }
}

fn stop_requested_between_attempts(command_receiver: &Receiver<AudioCommand>) -> bool {
    matches!(
        command_receiver.try_recv(),
        Ok(AudioCommand::Stop) | Err(mpsc::TryRecvError::Disconnected)
    )
}

// Keep the attempt's ownership, cancellation, buffer, application, and event
// channels explicit; bundling them would obscure the capture lifecycle.
#[allow(clippy::too_many_arguments)]
fn run_backend(
    owner: crate::audio_lifecycle::AudioOwner,
    backend: CaptureBackend,
    device_id: Option<&str>,
    ctx: AttemptContext,
    command_receiver: &Receiver<AudioCommand>,
    shared: &Arc<Mutex<Vec<f32>>>,
    active: &Arc<AtomicBool>,
    app_handle: &Option<tauri::AppHandle>,
    event_sender: &AudioWorkerEventSender,
) -> AttemptResult {
    // Close the retry-delay timeout boundary before permission probing or a
    // new helper spawn. A Stop queued as the wait expires must own the next
    // action rather than allowing another same-device attempt to start.
    if stop_requested_between_attempts(command_receiver) {
        return AttemptResult::Stopped;
    }
    let started_at = Instant::now();
    let mut clock = ActiveAttemptClock::new(started_at);
    let attempt_budget = if ctx.is_primary {
        primary_attempt_budget(backend, device_id, ctx.memo_promoted)
    } else {
        backend_attempt_budget(backend)
    };
    if owner.is_microphone_benchmark() {
        let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
            owner,
            diagnostic: AudioStartupDiagnostic::AttemptStarted {
                backend,
                resolution_pass: ctx.resolution_pass as u8,
                attempt_index: ctx.backend_attempt as u8,
                attempt_budget_ms: attempt_budget.as_millis() as u64,
            },
        });
    }
    let mut permission_prompt_started = None;
    let permission_status = crate::commands::permissions::check_microphone_permission_status();
    if permission_status == "denied" {
        return AttemptResult::Failed {
            failure: AudioFailure::new(
                AudioFailureKind::PermissionDenied,
                AudioInitPhase::StreamBuild,
            ),
            retained_audio: false,
            active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
    let (child, mut input, output) =
        match spawn_helper(capture_id, &nonce_hex, requested_capture_fault()) {
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
                    active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
                active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
            active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
    let mut preview_levels = PreviewLevelAccumulator::default();
    let mut preview_classification = PreviewLevelTracker::default();
    let mut preview_vad_window = PreviewVadWindow::default();
    let mut last_permission_check = Instant::now() - PERMISSION_POLL_INTERVAL;
    let mut current_phase = AudioInitPhase::StreamBuild;
    let mut last_setup_step: Option<(CaptureSetupStep, SetupTransition)> = None;
    let mut input_resolution = InputResolutionTracker::default();
    let worker_pid = child.pid();
    let mut hang_probe: Option<crate::hang_diagnostics::HangProbe> = None;
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
                    active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
                    active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
            hang_probe = crate::hang_diagnostics::HangProbe::start(
                capture_id,
                backend_label(backend),
                worker_pid,
            );
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
            if ctx.is_primary && ctx.memo_promoted && should_update_capture_memo(owner) {
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
                owner_kind = owner.kind(),
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
                active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
            };
        }

        match reader_rx.recv_timeout(CAPTURE_COMMAND_POLL_INTERVAL) {
            Ok(HelperRead::Frame(ProductionFrame::Pcm(pcm))) => {
                if pcm.channel != CaptureChannel::Microphone
                    || pcm.sequence != expected_sequence
                    || pcm.samples.is_empty()
                    || sample_rate.is_some_and(|rate| rate != pcm.sample_rate)
                    || !input_resolution.permits_pcm()
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
                        active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                    };
                }
                expected_sequence += 1;
                sample_rate = Some(pcm.sample_rate);
                if owner.retains_samples() {
                    shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .extend_from_slice(&pcm.samples);
                }
                if let Some(preview_id) = owner.preview_id() {
                    // Accumulate every callback between paint-rate emissions;
                    // otherwise a short peak could disappear in the throttle.
                    preview_levels.observe(&pcm.samples);
                    // Keep only a bounded rolling window in memory. The
                    // analyzer runs off-thread and never writes preview PCM to
                    // the retained recording buffer, disk, or telemetry.
                    preview_vad_window.observe(&pcm.samples, pcm.sample_rate);
                    let vad_now = Instant::now();
                    if preview_vad_window.is_due(vad_now) {
                        let analysis = app_handle
                            .as_ref()
                            .filter(|handle| can_schedule_vad_analysis(handle, preview_id))
                            .and_then(|handle| {
                                preview_vad_window
                                    .snapshot_if_due(vad_now)
                                    .map(|snapshot| (handle.clone(), snapshot))
                            });
                        if let Some((handle, (samples, sample_rate))) = analysis {
                            schedule_vad_analysis(handle, preview_id, samples, sample_rate);
                        } else {
                            // Sensitivity can be Off, an inference can already
                            // be running, or teardown can have started. Advance
                            // the cadence without copying the rolling window.
                            preview_vad_window.defer_due_snapshot(vad_now);
                        }
                    }
                }
                if !retained_audio {
                    retained_audio = true;
                    current_phase = AudioInitPhase::Runtime;
                    // A successful preview is real device-readiness evidence,
                    // so it intentionally trains the per-device backend memo.
                    if owner.is_microphone_benchmark() {
                        let active_elapsed_ms = clock.elapsed(Instant::now()).as_millis() as u64;
                        let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
                            owner,
                            diagnostic: AudioStartupDiagnostic::FirstPcm {
                                backend,
                                resolution_pass: ctx.resolution_pass as u8,
                                attempt_index: ctx.backend_attempt as u8,
                                // Match the existing production
                                // `audio.capture_ready.startup_ms` contract:
                                // helper Start write -> first accepted PCM.
                                attempt_start_to_first_pcm_ms: start_sent_at.elapsed().as_millis()
                                    as u64,
                                active_elapsed_ms,
                            },
                        });
                    } else if should_update_capture_memo(owner) {
                        note_first_pcm(device_id, backend, ctx.is_primary, start_sent_at.elapsed());
                    }
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
                        "capture helper first PCM accepted"
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
                        if let Some(preview_id) = owner.preview_id() {
                            let (rms, peak) = preview_levels.take();
                            let raw = classify_level(rms, peak);
                            let (classification, first_observation) =
                                preview_classification.stabilize(raw, Instant::now());
                            if first_observation {
                                tracing::info!(
                                    target: "audio",
                                    event_code = "audio.preview_level_classified",
                                    preview_id,
                                    classification = classification.as_str(),
                                    "microphone preview observed a signal classification"
                                );
                            }
                            let _ = handle.emit_to(
                                "main",
                                "microphone-preview-level",
                                MicrophonePreviewLevel {
                                    preview_id,
                                    rms,
                                    peak,
                                    classification,
                                },
                            );
                        } else {
                            let _ = handle.emit("audio-level", compute_rms(&pcm.samples));
                        }
                    }
                    last_level_emit = Instant::now();
                }
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Phase {
                phase,
                backend: reported_backend,
            }))) => {
                if !input_resolution.permits_phase(backend, reported_backend, phase) {
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
                        active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                    };
                }
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
                ProductionHelperMessage::InputResolution {
                    backend: reported_backend,
                    input_enumeration_ok,
                    requested_present,
                    input_device_count,
                    input_device_count_capped,
                    default_input_available,
                },
            ))) => {
                let evidence = ReportedInputResolution {
                    input_enumeration_ok,
                    requested_present,
                    input_device_count,
                    input_device_count_capped,
                };
                if !input_resolution.observe_evidence(
                    backend,
                    reported_backend,
                    device_id.is_some(),
                    evidence,
                ) {
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
                        active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                    };
                }
                tracing::info!(
                    target: "audio",
                    event_code = "audio.input_resolution_observed",
                    capture_id,
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    backend = backend_label(backend),
                    resolution_pass = ctx.resolution_pass,
                    backend_attempt = ctx.backend_attempt,
                    microphone_mode = if device_id.is_some() { "pinned" } else { "system_default" },
                    input_enumeration_ok,
                    requested_present = requested_present.unwrap_or(false),
                    requested_present_known = requested_present.is_some(),
                    input_device_count,
                    input_device_count_capped,
                    default_input_available,
                    "capture helper reported bounded input resolution evidence"
                );
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::SetupStep {
                    backend: reported_backend,
                    step,
                    transition,
                },
            ))) => {
                if reported_backend != backend || !input_resolution.observe_setup(step, transition)
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
                        current_phase,
                    ) {
                        return AttemptResult::TerminalHandled;
                    }
                    return AttemptResult::Failed {
                        failure: AudioFailure::new(AudioFailureKind::ProtocolError, current_phase),
                        retained_audio,
                        active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                    };
                }
                last_setup_step = Some((step, transition));
                if owner.is_microphone_benchmark() {
                    let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
                        owner,
                        diagnostic: AudioStartupDiagnostic::SetupStep {
                            backend,
                            resolution_pass: ctx.resolution_pass as u8,
                            attempt_index: ctx.backend_attempt as u8,
                            step,
                            transition,
                        },
                    });
                }
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
                backend: reported_backend,
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
                if reported_backend != backend || !input_resolution.permits_failure(code) {
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
                        failure: AudioFailure::new(AudioFailureKind::ProtocolError, phase),
                        retained_audio,
                        active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                    };
                }
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
                    active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
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
                    active_elapsed_ms: clock.elapsed(Instant::now()).as_millis() as u64,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

fn run_capture_backend_pass(
    owner: crate::audio_lifecycle::AudioOwner,
    command_receiver: &Receiver<AudioCommand>,
    event_sender: &AudioWorkerEventSender,
    backends: [CaptureBackend; 2],
    resolution_pass: usize,
    run_attempt: &mut impl FnMut(CaptureBackend, usize, usize) -> AttemptResult,
) -> BackendPassResult {
    let mut primary_device_unavailable = false;
    for (attempt_index, backend) in backends.into_iter().enumerate() {
        match run_attempt(backend, resolution_pass, attempt_index + 1) {
            AttemptResult::Stopped => return BackendPassResult::Stopped,
            AttemptResult::TerminalHandled => return BackendPassResult::TerminalHandled,
            AttemptResult::Failed {
                failure,
                retained_audio,
                active_elapsed_ms,
            } if !retained_audio && attempt_index == 0 && failure.permits_backend_fallback() => {
                if owner.is_microphone_benchmark() {
                    let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
                        owner,
                        diagnostic: AudioStartupDiagnostic::AttemptFailed {
                            backend,
                            resolution_pass: resolution_pass as u8,
                            attempt_index: (attempt_index + 1) as u8,
                            active_elapsed_ms,
                            failure_kind: failure.kind,
                            failure_phase: failure.phase,
                        },
                    });
                }
                primary_device_unavailable = failure.kind == AudioFailureKind::DeviceUnavailable;
                if stop_requested_between_attempts(command_receiver) {
                    tracing::info!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        "capture fallback suppressed by stop between backend attempts"
                    );
                    return BackendPassResult::Stopped;
                }
                tracing::warn!(
                    target: "audio",
                    event_code = "audio.fallback_started",
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    from_backend = backend_label(backend),
                    to_backend = backend_label(backends[1]),
                    error_kind = failure.kind.as_str(),
                    "capture backend failed before retained audio; trying bounded fallback"
                );
            }
            AttemptResult::Failed {
                failure,
                retained_audio,
                active_elapsed_ms,
            } => {
                if owner.is_microphone_benchmark() {
                    let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
                        owner,
                        diagnostic: AudioStartupDiagnostic::AttemptFailed {
                            backend,
                            resolution_pass: resolution_pass as u8,
                            attempt_index: (attempt_index + 1) as u8,
                            active_elapsed_ms,
                            failure_kind: failure.kind,
                            failure_phase: failure.phase,
                        },
                    });
                }
                let fallback_exhausted = !retained_audio && attempt_index == 1;
                let both_backends_device_unavailable = fallback_exhausted
                    && primary_device_unavailable
                    && failure.kind == AudioFailureKind::DeviceUnavailable;
                return BackendPassResult::Failed(BackendPassFailure {
                    failure,
                    retained_audio,
                    fallback_exhausted,
                    both_backends_device_unavailable,
                });
            }
        }
    }
    unreachable!("a backend pass always stops, is terminally handled, or fails")
}

fn wait_for_device_reresolution(
    command_receiver: &Receiver<AudioCommand>,
    delay: Duration,
    after_timeout: impl FnOnce(),
) -> bool {
    match command_receiver.recv_timeout(delay) {
        Err(RecvTimeoutError::Timeout) => {
            after_timeout();
            !stop_requested_between_attempts(command_receiver)
        }
        Ok(AudioCommand::Stop) | Err(RecvTimeoutError::Disconnected) => false,
    }
}

fn run_capture_backend_sequence_with_delay(
    owner: crate::audio_lifecycle::AudioOwner,
    command_receiver: &Receiver<AudioCommand>,
    event_sender: &AudioWorkerEventSender,
    backends: [CaptureBackend; 2],
    pinned_device: bool,
    retry_delay: Duration,
    mut run_attempt: impl FnMut(CaptureBackend, usize, usize) -> AttemptResult,
) {
    for pass_index in 0..DEVICE_RERESOLUTION_MAX_PASSES {
        match run_capture_backend_pass(
            owner,
            command_receiver,
            event_sender,
            backends,
            pass_index + 1,
            &mut run_attempt,
        ) {
            BackendPassResult::Stopped | BackendPassResult::TerminalHandled => return,
            BackendPassResult::Failed(pass_failure) => {
                let can_reresolve = pinned_device
                    && pass_failure.both_backends_device_unavailable
                    && pass_index + 1 < DEVICE_RERESOLUTION_MAX_PASSES;
                if can_reresolve {
                    tracing::warn!(
                        target: "audio",
                        event_code = "audio.device_reresolution_started",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        completed_pass = pass_index + 1,
                        next_pass = pass_index + 2,
                        retry_delay_ms = retry_delay.as_millis() as u64,
                        error_kind = pass_failure.failure.kind.as_str(),
                        "pinned microphone was absent on both backends; waiting for bounded same-device re-resolution"
                    );
                    if wait_for_device_reresolution(command_receiver, retry_delay, || {}) {
                        continue;
                    }
                    tracing::info!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        completed_pass = pass_index + 1,
                        "capture device re-resolution suppressed by stop"
                    );
                    return;
                }

                if pass_failure.fallback_exhausted {
                    tracing::error!(
                        target: "audio",
                        event_code = "audio.capture_failed",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        primary_backend = backend_label(backends[0]),
                        fallback_backend = backend_label(backends[1]),
                        fallback_exhausted = true,
                        resolution_passes = pass_index + 1,
                        error_kind = pass_failure.failure.kind.as_str(),
                        "both capture backend attempts failed before first PCM"
                    );
                }
                let event = if pass_failure.retained_audio {
                    AudioWorkerEvent::RuntimeFailed {
                        owner,
                        failure: pass_failure.failure,
                    }
                } else {
                    AudioWorkerEvent::InitFailed {
                        owner,
                        failure: pass_failure.failure,
                    }
                };
                let _ = event_sender.send(event);
                return;
            }
        }
    }
}

fn run_capture_backend_sequence(
    owner: crate::audio_lifecycle::AudioOwner,
    command_receiver: &Receiver<AudioCommand>,
    event_sender: &AudioWorkerEventSender,
    backends: [CaptureBackend; 2],
    pinned_device: bool,
    run_attempt: impl FnMut(CaptureBackend, usize, usize) -> AttemptResult,
) {
    run_capture_backend_sequence_with_delay(
        owner,
        command_receiver,
        event_sender,
        backends,
        pinned_device,
        DEVICE_RERESOLUTION_DELAY,
        run_attempt,
    );
}

fn run_audio_capture(spec: AudioWorkerSpec, event_sender: &AudioWorkerEventSender) {
    let AudioWorkerSpec {
        owner,
        command_receiver,
        shared,
        active,
        app_handle,
        device_id,
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
    if owner.is_microphone_benchmark() {
        let _ = event_sender.send(AudioWorkerEvent::StartupDiagnostic {
            owner,
            diagnostic: AudioStartupDiagnostic::BackendPlan {
                primary: backends[0],
                fallback: backends[1],
                source: if memo_promoted {
                    AudioBackendOrderSource::SessionFirstPcmMemo
                } else {
                    AudioBackendOrderSource::Default
                },
            },
        });
    }
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
        device_id.is_some(),
        |backend, resolution_pass, backend_attempt| {
            run_backend(
                owner,
                backend,
                device_id.as_deref(),
                AttemptContext {
                    is_primary: backend == backends[0],
                    memo_promoted,
                    resolution_pass,
                    backend_attempt,
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
    if from_rate > to_rate && from_rate.is_multiple_of(to_rate) {
        let step = (from_rate / to_rate) as usize;
        let new_len = samples.len() / step;
        return samples
            .iter()
            .step_by(step)
            .take(new_len)
            .copied()
            .collect();
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
    fn inventory_is_never_published_without_confirmed_helper_exit() {
        let inventory = EnumeratedAudioInputInventory {
            devices: vec![AudioDeviceDescriptor {
                id: "uid".to_string(),
                name: "Mic".to_string(),
            }],
            default_input_id: Some("uid".to_string()),
        };
        assert!(inventory_after_confirmed_helper_exit(inventory.clone(), false, false).is_err());
        assert!(inventory_after_confirmed_helper_exit(inventory.clone(), true, true).is_err());
        assert_eq!(
            inventory_after_confirmed_helper_exit(inventory.clone(), true, false).unwrap(),
            inventory
        );
    }

    #[test]
    fn ordinary_inventory_quarantine_is_bounded_before_detached_transfer() {
        let mut retries = 0;
        assert_eq!(
            bounded_inventory_quarantine(false, 2, || {
                retries += 1;
                false
            }),
            InventoryHelperTermination::DetachedReaperRequired
        );
        assert_eq!(retries, 2);

        let mut delayed_retries = 0;
        assert_eq!(
            bounded_inventory_quarantine(false, 3, || {
                delayed_retries += 1;
                delayed_retries == 2
            }),
            InventoryHelperTermination::ConfirmedDuringQuarantine
        );
        assert_eq!(delayed_retries, 2);

        let mut unexpected_retry = false;
        assert_eq!(
            bounded_inventory_quarantine(true, 2, || {
                unexpected_retry = true;
                true
            }),
            InventoryHelperTermination::ConfirmedWithinBudget
        );
        assert!(!unexpected_retry);
    }

    #[test]
    fn detached_reaper_gate_waits_for_watcher_but_refuses_enumeration() {
        assert_eq!(
            inventory_helper_spawn_action(true, false, Some(false)),
            InventoryHelperSpawnAction::Spawn
        );
        assert_eq!(
            inventory_helper_spawn_action(true, true, Some(false)),
            InventoryHelperSpawnAction::WaitForReaper
        );
        assert_eq!(
            inventory_helper_spawn_action(true, true, None),
            InventoryHelperSpawnAction::Refuse
        );
        assert_eq!(
            inventory_helper_spawn_action(true, true, Some(true)),
            InventoryHelperSpawnAction::Refuse
        );
        assert_eq!(
            inventory_helper_spawn_action(true, false, Some(true)),
            InventoryHelperSpawnAction::Refuse
        );
        assert_eq!(
            inventory_helper_spawn_action(false, false, Some(false)),
            InventoryHelperSpawnAction::Refuse
        );
    }

    #[test]
    fn inventory_reaper_service_state_queue_and_shutdown_decisions_are_bounded() {
        assert_eq!(
            inventory_reaper_service_action(true, false),
            InventoryReaperServiceAction::Wait
        );
        assert_eq!(
            inventory_reaper_service_action(false, false),
            InventoryReaperServiceAction::Reap
        );
        assert_eq!(
            inventory_reaper_service_action(false, true),
            InventoryReaperServiceAction::Reap
        );
        assert_eq!(
            inventory_reaper_service_action(true, true),
            InventoryReaperServiceAction::Exit
        );

        assert!(inventory_reaper_service_accepts(true, true, false));
        assert!(!inventory_reaper_service_accepts(false, true, false));
        assert!(!inventory_reaper_service_accepts(true, false, false));
        assert!(!inventory_reaper_service_accepts(true, true, true));

        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        assert_eq!(
            inventory_reaper_shutdown_action(true, now, deadline),
            InventoryReaperShutdownAction::Join
        );
        assert_eq!(
            inventory_reaper_shutdown_action(false, now, deadline),
            InventoryReaperShutdownAction::Wait
        );
        assert_eq!(
            inventory_reaper_shutdown_action(false, deadline, deadline),
            InventoryReaperShutdownAction::Detach
        );
    }

    #[test]
    fn topology_changes_are_ignored_after_stop_is_sent() {
        assert!(!should_forward_input_topology_change(false, false));
        assert!(should_forward_input_topology_change(true, false));
        assert!(!should_forward_input_topology_change(true, true));
    }

    #[test]
    fn topology_watch_has_a_post_stop_deadline() {
        let now = Instant::now();
        assert_eq!(
            input_topology_watch_timeout(true, true, now, now + Duration::from_secs(30), Some(now),),
            Some("The microphone topology watcher did not stop in time.")
        );
        assert_eq!(
            input_topology_watch_timeout(true, true, now, now, Some(now + Duration::from_secs(1)),),
            None
        );
    }

    #[test]
    fn integer_downsampling_uses_the_same_source_positions_as_linear_interpolation() {
        let samples = (0..10).map(|value| value as f32).collect::<Vec<_>>();
        assert_eq!(resample(&samples, 48_000, 16_000), vec![0.0, 3.0, 6.0]);
        assert_eq!(
            resample(&samples, 32_000, 16_000),
            vec![0.0, 2.0, 4.0, 6.0, 8.0]
        );
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
    fn four_fast_rescues_arm_the_deep_primary_budget_and_success_resets_it() {
        let key = Some("memo-device-deepfastfail");
        let default_order = preferred_backends();
        let full = backend_attempt_budget(default_order[0]);

        // Two and three consecutive fast rescues arm only the first tier.
        note_first_pcm(key, default_order[1], false, FAST);
        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(
            primary_attempt_budget(default_order[0], key, false),
            FAST_FAIL_PRIMARY_BUDGET
        );
        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(
            primary_attempt_budget(default_order[0], key, false),
            FAST_FAIL_PRIMARY_BUDGET
        );

        // The fourth arms the deeper tier.
        note_first_pcm(key, default_order[1], false, FAST);
        assert_eq!(
            primary_attempt_budget(default_order[0], key, false),
            FAST_FAIL_DEEP_PRIMARY_BUDGET
        );

        // A primary success drops out of both tiers at once.
        note_first_pcm(key, default_order[0], true, FAST);
        assert_eq!(primary_attempt_budget(default_order[0], key, false), full);
    }

    #[test]
    fn a_slow_rescue_resets_out_of_the_deep_fast_fail_tier() {
        let key = Some("memo-device-deepslowrescue");
        let default_order = preferred_backends();
        let full = backend_attempt_budget(default_order[0]);

        for _ in 0..FAST_FAIL_DEEP_ARM_COUNT {
            note_first_pcm(key, default_order[1], false, FAST);
        }
        assert_eq!(
            primary_attempt_budget(default_order[0], key, false),
            FAST_FAIL_DEEP_PRIMARY_BUDGET
        );

        // One rescue that took longer than FAST_RESCUE_THRESHOLD is not the
        // first-attempt-bound pathology: the counter resets to zero.
        note_first_pcm(key, default_order[1], false, SLOW);
        assert_eq!(primary_attempt_budget(default_order[0], key, false), full);
    }

    #[test]
    fn a_promoted_primary_with_deep_arming_takes_the_smaller_of_cap_and_deep_budget() {
        let key = Some("memo-device-deepcap");
        for _ in 0..FAST_FAIL_DEEP_ARM_COUNT {
            note_first_pcm(key, CaptureBackend::Auhal, false, FAST);
        }
        // min(PROMOTED_PRIMARY_BUDGET_CAP = 8s, deep = 750ms) = 750ms.
        assert_eq!(
            primary_attempt_budget(CaptureBackend::Cpal, key, true),
            Duration::from_millis(750)
        );
        assert_eq!(
            primary_attempt_budget(CaptureBackend::Cpal, key, true),
            FAST_FAIL_DEEP_PRIMARY_BUDGET
        );
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
    fn stream_build_faults_run_inside_the_killable_capture_process() {
        let mut sentinel_creations = 0;
        assert_eq!(
            capture_fault_for_scenario(Some("hang_stream_build"), || {
                sentinel_creations += 1;
                true
            }),
            Some("hang-stream-build")
        );
        assert_eq!(sentinel_creations, 0);

        assert_eq!(
            capture_fault_for_scenario(Some("hang_stream_build_once"), || {
                sentinel_creations += 1;
                true
            }),
            Some("hang-stream-build")
        );
        assert_eq!(
            capture_fault_for_scenario(Some("hang_stream_build_once"), || {
                sentinel_creations += 1;
                false
            }),
            None
        );
        assert_eq!(sentinel_creations, 2);
        assert_eq!(
            capture_fault_for_scenario(Some("unrelated"), || panic!("must not create sentinel")),
            None
        );
    }

    #[test]
    fn stop_is_consumed_before_a_fallback_attempt_can_spawn() {
        let (sender, receiver) = mpsc::channel();
        assert!(!stop_requested_between_attempts(&receiver));
        sender.send(AudioCommand::Stop).unwrap();
        assert!(stop_requested_between_attempts(&receiver));
    }

    #[test]
    fn diagnostic_capture_never_retrains_the_production_backend_memo() {
        assert!(should_update_capture_memo(
            crate::audio_lifecycle::AudioOwner::Dictation(1)
        ));
        assert!(should_update_capture_memo(
            crate::audio_lifecycle::AudioOwner::Preview(2)
        ));
        assert!(!should_update_capture_memo(
            crate::audio_lifecycle::AudioOwner::MicrophoneBenchmark(3)
        ));
    }

    #[test]
    fn diagnostic_policy_reads_leave_capture_memo_byte_equivalent() {
        let unseen = Some("memo-diagnostic-unseen");
        let before_unseen = CAPTURE_MEMO.lock_or_recover().clone();
        let unseen_plan = microphone_startup_backend_plan(unseen);
        let unseen_order = preferred_backends_for(unseen);
        let _ = primary_attempt_budget(unseen_order[0], unseen, false);
        assert!(matches!(
            unseen_plan,
            AudioStartupDiagnostic::BackendPlan { .. }
        ));
        assert_eq!(before_unseen, CAPTURE_MEMO.lock_or_recover().clone());

        let existing = Some("memo-diagnostic-existing");
        let default_order = preferred_backends();
        note_first_pcm(existing, default_order[1], false, FAST);
        let before_existing = CAPTURE_MEMO.lock_or_recover().clone();
        let existing_plan = microphone_startup_backend_plan(existing);
        let existing_order = preferred_backends_for(existing);
        let _ = primary_attempt_budget(existing_order[0], existing, true);
        assert!(matches!(
            existing_plan,
            AudioStartupDiagnostic::BackendPlan { .. }
        ));
        assert_eq!(before_existing, CAPTURE_MEMO.lock_or_recover().clone());
    }

    fn failed_attempt(kind: AudioFailureKind, retained_audio: bool) -> AttemptResult {
        AttemptResult::Failed {
            failure: AudioFailure::new(
                kind,
                if retained_audio {
                    AudioInitPhase::Runtime
                } else {
                    AudioInitPhase::StreamBuild
                },
            ),
            retained_audio,
            active_elapsed_ms: 1,
        }
    }

    fn resolution_evidence(requested_present: Option<bool>) -> ReportedInputResolution {
        ReportedInputResolution {
            input_enumeration_ok: true,
            requested_present,
            input_device_count: 2,
            input_device_count_capped: false,
        }
    }

    fn started_resolution_tracker() -> InputResolutionTracker {
        let mut tracker = InputResolutionTracker::default();
        assert!(
            tracker.observe_setup(CaptureSetupStep::DeviceResolution, SetupTransition::Entered,)
        );
        tracker
    }

    #[test]
    fn input_resolution_evidence_is_accepted_once_between_entered_and_completed() {
        let mut pinned = started_resolution_tracker();
        assert!(pinned.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(false)),
        ));
        assert!(pinned.permits_failure(FailureCode::NoInputDevice));
        assert!(pinned.observe_setup(
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(pinned.observe_setup(CaptureSetupStep::AudioUnitNew, SetupTransition::Entered,));

        let mut system_default = started_resolution_tracker();
        assert!(system_default.observe_evidence(
            CaptureBackend::Cpal,
            CaptureBackend::Cpal,
            false,
            resolution_evidence(None),
        ));
    }

    #[test]
    fn input_resolution_rejects_missing_duplicate_and_wrong_backend_evidence() {
        let mut missing = started_resolution_tracker();
        assert!(!missing.observe_setup(
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(!missing.permits_failure(FailureCode::NoInputDevice));
        assert!(InputResolutionTracker::default().permits_failure(FailureCode::PermissionDenied));
        assert!(!InputResolutionTracker::default().permits_failure(FailureCode::NoInputDevice));

        let mut duplicate = started_resolution_tracker();
        assert!(duplicate.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(true)),
        ));
        assert!(!duplicate.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(true)),
        ));

        let mut wrong_backend = started_resolution_tracker();
        assert!(!wrong_backend.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Cpal,
            true,
            resolution_evidence(Some(true)),
        ));
    }

    #[test]
    fn input_resolution_must_complete_before_active_phases_or_pcm() {
        let mut tracker = InputResolutionTracker::default();
        assert!(tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::StreamOpen,
        ));
        assert!(!tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Cpal,
            CapturePhase::StreamOpen,
        ));
        assert!(!tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::AwaitingFirstCallback,
        ));
        assert!(!tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::Active,
        ));
        assert!(!tracker.permits_pcm());

        assert!(
            tracker.observe_setup(CaptureSetupStep::DeviceResolution, SetupTransition::Entered,)
        );
        assert!(tracker.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(true)),
        ));
        assert!(!tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::AwaitingFirstCallback,
        ));
        assert!(!tracker.permits_pcm());
        assert!(tracker.observe_setup(
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::AwaitingFirstCallback,
        ));
        assert!(tracker.permits_phase(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            CapturePhase::Active,
        ));
        assert!(tracker.permits_pcm());
    }

    #[test]
    fn input_resolution_failure_must_match_the_observed_resolver_result() {
        let mut pinned_present = started_resolution_tracker();
        assert!(pinned_present.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(true)),
        ));
        assert!(!pinned_present.permits_failure(FailureCode::NoInputDevice));

        let mut enumeration_unknown = started_resolution_tracker();
        assert!(enumeration_unknown.observe_evidence(
            CaptureBackend::Cpal,
            CaptureBackend::Cpal,
            true,
            ReportedInputResolution {
                input_enumeration_ok: false,
                requested_present: None,
                input_device_count: 0,
                input_device_count_capped: false,
            },
        ));
        assert!(enumeration_unknown.permits_failure(FailureCode::EnumerationFailed));
        assert!(!enumeration_unknown.permits_failure(FailureCode::NoInputDevice));

        let mut missing_default = started_resolution_tracker();
        assert!(missing_default.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            false,
            resolution_evidence(None),
        ));
        assert!(missing_default.permits_failure(FailureCode::NoInputDevice));

        let mut completed = started_resolution_tracker();
        assert!(completed.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            resolution_evidence(Some(true)),
        ));
        assert!(completed.observe_setup(
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(!completed.permits_failure(FailureCode::NoInputDevice));
        assert!(!completed.permits_failure(FailureCode::EnumerationFailed));
        assert!(completed.permits_failure(FailureCode::ConfigurationFailed));
    }

    #[test]
    fn input_resolution_enforces_protocol_cross_field_invariants() {
        let invalid = [
            (
                true,
                ReportedInputResolution {
                    requested_present: None,
                    ..resolution_evidence(Some(true))
                },
            ),
            (
                false,
                ReportedInputResolution {
                    requested_present: Some(true),
                    ..resolution_evidence(None)
                },
            ),
            (
                true,
                ReportedInputResolution {
                    input_enumeration_ok: false,
                    requested_present: None,
                    input_device_count: 1,
                    ..resolution_evidence(Some(true))
                },
            ),
            (
                true,
                ReportedInputResolution {
                    input_device_count: 1,
                    input_device_count_capped: true,
                    ..resolution_evidence(Some(true))
                },
            ),
        ];
        for (requested_device, evidence) in invalid {
            let mut tracker = started_resolution_tracker();
            assert!(!tracker.observe_evidence(
                CaptureBackend::Auhal,
                CaptureBackend::Auhal,
                requested_device,
                evidence,
            ));
        }
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
            false,
            |backend, _, _| {
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
                        active_elapsed_ms: 1,
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
            false,
            |backend, _, _| {
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
            false,
            |backend, _, _| {
                calls.push(backend);
                AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::InitializationTimeout,
                        AudioInitPhase::StreamBuild,
                    ),
                    retained_audio: false,
                    active_elapsed_ms: 1,
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
    fn pinned_device_unavailable_retries_at_most_two_full_backend_passes() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();

        run_capture_backend_sequence_with_delay(
            crate::audio_lifecycle::AudioOwner::Dictation(31),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            true,
            Duration::ZERO,
            |backend, resolution_pass, backend_attempt| {
                calls.push((backend, resolution_pass, backend_attempt));
                failed_attempt(AudioFailureKind::DeviceUnavailable, false)
            },
        );

        assert_eq!(
            calls,
            vec![
                (CaptureBackend::Auhal, 1, 1),
                (CaptureBackend::Cpal, 1, 2),
                (CaptureBackend::Auhal, 2, 1),
                (CaptureBackend::Cpal, 2, 2),
                (CaptureBackend::Auhal, 3, 1),
                (CaptureBackend::Cpal, 3, 2),
            ]
        );
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AudioWorkerEvent::InitFailed {
                failure: AudioFailure {
                    kind: AudioFailureKind::DeviceUnavailable,
                    ..
                },
                ..
            }]
        ));
    }

    #[test]
    fn stop_queued_at_reresolution_timeout_boundary_prevents_the_next_pass() {
        let (command_sender, command_receiver) = mpsc::channel();
        assert!(!wait_for_device_reresolution(
            &command_receiver,
            Duration::ZERO,
            || command_sender.send(AudioCommand::Stop).unwrap(),
        ));
    }

    #[test]
    fn pinned_device_can_succeed_on_a_later_reresolution_pass_without_a_terminal() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();

        run_capture_backend_sequence_with_delay(
            crate::audio_lifecycle::AudioOwner::Dictation(32),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            true,
            Duration::ZERO,
            |backend, _, _| {
                calls.push(backend);
                if calls.len() == 4 {
                    AttemptResult::Stopped
                } else {
                    failed_attempt(AudioFailureKind::DeviceUnavailable, false)
                }
            },
        );

        assert_eq!(
            calls,
            vec![
                CaptureBackend::Auhal,
                CaptureBackend::Cpal,
                CaptureBackend::Auhal,
                CaptureBackend::Cpal,
            ]
        );
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn system_default_device_unavailable_does_not_reresolve() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();

        run_capture_backend_sequence_with_delay(
            crate::audio_lifecycle::AudioOwner::Dictation(33),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            false,
            Duration::ZERO,
            |backend, _, _| {
                calls.push(backend);
                failed_attempt(AudioFailureKind::DeviceUnavailable, false)
            },
        );

        assert_eq!(calls, vec![CaptureBackend::Auhal, CaptureBackend::Cpal]);
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AudioWorkerEvent::InitFailed { .. }]
        ));
    }

    #[test]
    fn mixed_backend_failures_do_not_reresolve_a_pinned_device() {
        let (_command_sender, command_receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sender = AudioWorkerEventSender::new(move |event| {
            captured_events.lock().unwrap().push(event);
            Ok(())
        });
        let mut calls = Vec::new();

        run_capture_backend_sequence_with_delay(
            crate::audio_lifecycle::AudioOwner::Dictation(34),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            true,
            Duration::ZERO,
            |backend, _, _| {
                calls.push(backend);
                failed_attempt(
                    if backend == CaptureBackend::Auhal {
                        AudioFailureKind::BackendError
                    } else {
                        AudioFailureKind::DeviceUnavailable
                    },
                    false,
                )
            },
        );

        assert_eq!(calls, vec![CaptureBackend::Auhal, CaptureBackend::Cpal]);
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AudioWorkerEvent::InitFailed {
                failure: AudioFailure {
                    kind: AudioFailureKind::DeviceUnavailable,
                    ..
                },
                ..
            }]
        ));
    }

    #[test]
    fn stop_during_reresolution_delay_prevents_another_helper_spawn() {
        let (command_sender, command_receiver) = mpsc::channel();
        let event_sender = AudioWorkerEventSender::new(|_| Ok(()));
        let mut calls = Vec::new();

        run_capture_backend_sequence_with_delay(
            crate::audio_lifecycle::AudioOwner::Dictation(35),
            &command_receiver,
            &event_sender,
            [CaptureBackend::Auhal, CaptureBackend::Cpal],
            true,
            DEVICE_RERESOLUTION_DELAY,
            |backend, _, _| {
                calls.push(backend);
                if calls.len() == 2 {
                    command_sender.send(AudioCommand::Stop).unwrap();
                }
                failed_attempt(AudioFailureKind::DeviceUnavailable, false)
            },
        );

        assert_eq!(calls, vec![CaptureBackend::Auhal, CaptureBackend::Cpal]);
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
            false,
            |backend, _, _| {
                calls.push(backend);
                AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::StreamInvalidated,
                        AudioInitPhase::Runtime,
                    ),
                    retained_audio: true,
                    active_elapsed_ms: 1,
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
