use crate::managed_child::{bundled_sibling, ManagedChild};
use crate::meeting_store::{
    MeetingRepository, MeetingSegment, MeetingSegmentStatus, MeetingSessionStatus, MeetingSpeaker,
    PendingMeetingSegment,
};
use crate::model_runtime::PreparationReason;
use crate::state::WHISPER_SAMPLE_RATE;
use crate::{MutexExt, State};
use murmur_capture_helper_protocol::{
    read_production_frame, valid_input_resolution_evidence, write_production_control,
    CaptureBackend, CaptureChannel, CapturePhase, CaptureSetupStep, EchoCancellationBypassReason,
    EchoCancellationMode, EchoCancellationStatus, FailureCode, ProductionFrame,
    ProductionHelperMessage, ProductionHostMessage, ProductionPcm, SessionNonce, SetupTransition,
    SystemAudioPermissionStatus, MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::BufReader;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use uuid::Uuid;

const CAPTURE_WORKER_IDENTIFIER: &str = "com.localdictation.capture-worker";
const HELPER_STOP_DEADLINE: Duration = Duration::from_secs(2);
const MEETING_STOP_ACK_DEADLINE: Duration = Duration::from_secs(2);
const START_PERMISSION_DEADLINE: Duration = Duration::from_secs(120);
const SETUP_STEP_DEADLINE: Duration = Duration::from_secs(8);
const PROCESSING_READY_DEADLINE: Duration = Duration::from_secs(10);
const PCM_QUEUE_RETRY_DEADLINE: Duration = Duration::from_millis(100);
const FRAME_POLL: Duration = Duration::from_millis(10);
const PCM_QUEUE_CAPACITY: usize = 128;
const INFERENCE_WAKE_CAPACITY: usize = 1;
const ANALYSIS_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize / 2;
const PRE_ROLL_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize / 4;
const TRAILING_SILENCE_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize / 2;
const MAX_CHUNK_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize * 15;
const MIN_CHUNK_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize / 5;
static NEXT_MEETING_CAPTURE_ID: AtomicU64 = AtomicU64::new(1_000_000);

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioPermissionState {
    #[default]
    Unknown,
    Granted,
    Denied,
    Unsupported,
}

/// Authorization and capture health are independent facts. A granted tap on a
/// silent Mac is healthy (`permission: Granted`, `audio_flowing: false`) and is
/// never reported as a permission problem.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemAudioAccess {
    pub permission: SystemAudioPermissionState,
    /// The tap, its private aggregate device, and its IO proc all started.
    pub capture_ready: bool,
    /// A callback delivered samples inside the probe's observation window.
    pub audio_flowing: bool,
    /// macOS reports the app as authorized but Core Audio still refuses the
    /// tap, which a relaunch clears.
    pub needs_relaunch: bool,
}

impl SystemAudioPermissionState {
    /// Stable, content-free token for the `meeting` telemetry allowlist.
    pub(crate) fn as_event_value(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetingRuntimePhase {
    Idle,
    Starting,
    Recording,
    Stopping,
    Processing,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MeetingEchoCancellationRuntime {
    #[default]
    Off,
    Starting,
    Active,
    Recovering {
        reason: EchoCancellationBypassReason,
        attempt: u8,
        max_attempts: u8,
    },
    Bypassed {
        reason: EchoCancellationBypassReason,
    },
}

impl MeetingEchoCancellationRuntime {
    fn event_state(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Starting => "starting",
            Self::Active => "active",
            Self::Recovering { .. } => "recovering",
            Self::Bypassed { .. } => "bypassed",
        }
    }

    fn event_reason(&self) -> Option<&'static str> {
        match self {
            Self::Recovering { reason, .. } | Self::Bypassed { reason } => match reason {
                EchoCancellationBypassReason::InitializationFailed => Some("initialization_failed"),
                EchoCancellationBypassReason::UnsupportedFormat => Some("unsupported_format"),
                EchoCancellationBypassReason::RenderDiscontinuity => Some("render_discontinuity"),
                EchoCancellationBypassReason::ProcessorFailed => Some("processor_failed"),
                EchoCancellationBypassReason::ProcessingBacklog => Some("processing_backlog"),
            },
            Self::Off | Self::Starting | Self::Active => None,
        }
    }

    fn recovery_attempt(&self) -> u8 {
        match self {
            Self::Recovering { attempt, .. } => *attempt,
            Self::Off | Self::Starting | Self::Active | Self::Bypassed { .. } => 0,
        }
    }

    fn recovery_max_attempts(&self) -> u8 {
        match self {
            Self::Recovering { max_attempts, .. } => *max_attempts,
            Self::Off | Self::Starting | Self::Active | Self::Bypassed { .. } => 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRuntimeStatus {
    pub generation: u64,
    pub session_id: Option<String>,
    pub phase: MeetingRuntimePhase,
    pub elapsed_ms: u64,
    pub microphone_active: bool,
    pub system_audio_active: bool,
    pub echo_cancellation: MeetingEchoCancellationRuntime,
    pub error_code: Option<String>,
}

impl Default for MeetingRuntimeStatus {
    fn default() -> Self {
        Self {
            generation: 0,
            session_id: None,
            phase: MeetingRuntimePhase::Idle,
            elapsed_ms: 0,
            microphone_active: false,
            system_audio_active: false,
            echo_cancellation: MeetingEchoCancellationRuntime::Off,
            error_code: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MeetingCaptureConfig {
    pub generation: u64,
    pub session_id: String,
    pub vad_sensitivity: u32,
    pub device_id: Option<String>,
    pub echo_cancellation: EchoCancellationMode,
}

#[derive(Clone, Debug)]
struct MeetingError {
    code: &'static str,
    message: &'static str,
}

impl MeetingError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

#[derive(Default)]
struct CompletionState {
    done: Mutex<bool>,
    changed: Condvar,
}

impl CompletionState {
    fn finish(&self) {
        *self.done.lock_or_recover() = true;
        self.changed.notify_all();
    }

    fn wait(&self, timeout: Duration) -> bool {
        let done = self.done.lock_or_recover();
        if *done {
            return true;
        }
        self.changed
            .wait_timeout_while(done, timeout, |done| !*done)
            .map(|(done, _)| *done)
            .unwrap_or(false)
    }
}

enum MeetingCommand {
    Stop,
}

struct ActiveMeeting {
    generation: u64,
    command_sender: mpsc::Sender<MeetingCommand>,
    completion: Arc<CompletionState>,
}

#[derive(Default)]
struct CoordinatorInner {
    active: Option<ActiveMeeting>,
    permission: SystemAudioPermissionState,
    status: MeetingRuntimeStatus,
}

#[derive(Clone, Default)]
pub struct MeetingCoordinator {
    inner: Arc<Mutex<CoordinatorInner>>,
}

impl MeetingCoordinator {
    pub fn status(&self) -> MeetingRuntimeStatus {
        self.inner.lock_or_recover().status.clone()
    }

    /// Non-blocking status read for hang diagnostics. `None` means the meeting
    /// lock was contended, which is itself reportable: a probe must never wait
    /// on the subsystem it is describing.
    pub fn status_if_uncontended(&self) -> Option<MeetingRuntimeStatus> {
        Some(self.inner.try_lock_or_recover()?.status.clone())
    }

    pub fn permission_status(&self) -> SystemAudioPermissionState {
        self.inner.lock_or_recover().permission
    }

    pub fn is_active(&self) -> bool {
        self.inner.lock_or_recover().active.is_some()
    }

    pub fn start(
        &self,
        app: tauri::AppHandle,
        repository: MeetingRepository,
        config: MeetingCaptureConfig,
    ) -> Result<(), String> {
        let (command_sender, command_receiver) = mpsc::channel();
        let completion = Arc::new(CompletionState::default());
        {
            let mut inner = self.inner.lock_or_recover();
            if inner.active.is_some() {
                return Err("A meeting is already active.".to_string());
            }
            inner.status = MeetingRuntimeStatus {
                generation: config.generation,
                session_id: Some(config.session_id.clone()),
                phase: MeetingRuntimePhase::Starting,
                echo_cancellation: match config.echo_cancellation {
                    EchoCancellationMode::Disabled => MeetingEchoCancellationRuntime::Off,
                    EchoCancellationMode::Enabled => MeetingEchoCancellationRuntime::Starting,
                },
                ..MeetingRuntimeStatus::default()
            };
            inner.active = Some(ActiveMeeting {
                generation: config.generation,
                command_sender,
                completion: Arc::clone(&completion),
            });
        }
        tracing::info!(
            target: "meeting",
            event_code = "meeting.capture_started",
            generation = config.generation,
            phase = "starting",
            "meeting capture accepted"
        );
        publish_status(&app, &self.status());

        let coordinator = self.clone();
        let app_for_thread = app.clone();
        let config_for_thread = config.clone();
        let repository_for_thread = repository.clone();
        let spawned = thread::Builder::new()
            .name(format!("murmur-meeting-{}", config.generation))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_capture_session(
                        &app_for_thread,
                        &repository_for_thread,
                        &config_for_thread,
                        command_receiver,
                        &coordinator,
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(MeetingError::new(
                        "supervisor_panicked",
                        "The meeting capture supervisor stopped unexpectedly.",
                    ))
                });
                let (session_status, error_code) = match &result {
                    Ok(()) => (MeetingSessionStatus::Complete, None),
                    Err(error) => (MeetingSessionStatus::Failed, Some(error.code)),
                };
                let _ = repository_for_thread.finish_session(
                    &config_for_thread.session_id,
                    session_status,
                    error_code,
                );
                {
                    let state = app_for_thread.state::<State>();
                    if state.app_state.meeting_generation.load(Ordering::SeqCst)
                        == config_for_thread.generation
                    {
                        state
                            .app_state
                            .meeting_active
                            .store(false, Ordering::SeqCst);
                        state
                            .app_state
                            .meeting_inference_active
                            .store(false, Ordering::SeqCst);
                    }
                }
                coordinator.finish(
                    &app_for_thread,
                    config_for_thread.generation,
                    result.as_ref().err(),
                );
                completion.finish();
            });
        if spawned.is_err() {
            let _ = repository.finish_session(
                &config.session_id,
                MeetingSessionStatus::Failed,
                Some("supervisor_unavailable"),
            );
            self.finish(
                &app,
                config.generation,
                Some(&MeetingError::new(
                    "supervisor_unavailable",
                    "The meeting capture supervisor could not start.",
                )),
            );
            return Err("The meeting capture supervisor could not start.".to_string());
        }
        Ok(())
    }

    pub fn stop(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let (sender, status) = {
            let mut inner = self.inner.lock_or_recover();
            let active = inner
                .active
                .as_ref()
                .ok_or_else(|| "No meeting is active.".to_string())?;
            let sender = active.command_sender.clone();
            inner.status.phase = MeetingRuntimePhase::Stopping;
            (sender, inner.status.clone())
        };
        tracing::info!(
            target: "meeting",
            event_code = "meeting.capture_stopped",
            generation = status.generation,
            phase = "stopping",
            "meeting stop requested"
        );
        publish_status(app, &status);
        sender
            .send(MeetingCommand::Stop)
            .map_err(|_| "The meeting capture supervisor is unavailable.".to_string())
    }

    pub fn shutdown(&self, app: &tauri::AppHandle) {
        let active = {
            let inner = self.inner.lock_or_recover();
            inner.active.as_ref().map(|active| {
                (
                    active.command_sender.clone(),
                    Arc::clone(&active.completion),
                )
            })
        };
        if let Some((sender, completion)) = active {
            let _ = sender.send(MeetingCommand::Stop);
            let _ = completion.wait(Duration::from_secs(5));
        }
        let state = app.state::<State>();
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
    }

    pub fn request_permission(&self) -> Result<SystemAudioAccess, String> {
        if self.is_active() {
            return Err("Stop the active meeting before checking System Audio access.".to_string());
        }
        let result = probe_permission();
        if let Ok(access) = result {
            self.inner.lock_or_recover().permission = access.permission;
        }
        result
    }

    pub fn recover_pending(&self, app: tauri::AppHandle, repository: MeetingRepository) {
        if !matches!(
            repository.status(),
            Ok(status) if status.pending_segment_count > 0
        ) {
            return;
        }
        let state = app.state::<State>();
        if state
            .app_state
            .meeting_inference_active
            .swap(true, Ordering::SeqCst)
        {
            return;
        }
        let app_for_thread = app.clone();
        let spawned = thread::Builder::new()
            .name("murmur-meeting-recovery".to_string())
            .spawn(move || {
                drain_pending(&app_for_thread, &repository);
                app_for_thread
                    .state::<State>()
                    .app_state
                    .meeting_inference_active
                    .store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            state
                .app_state
                .meeting_inference_active
                .store(false, Ordering::SeqCst);
        }
    }

    fn update_phase(
        &self,
        app: &tauri::AppHandle,
        generation: u64,
        phase: MeetingRuntimePhase,
        microphone_active: bool,
        system_audio_active: bool,
        elapsed_ms: u64,
    ) {
        let status = {
            let mut inner = self.inner.lock_or_recover();
            if inner
                .active
                .as_ref()
                .is_none_or(|active| active.generation != generation)
            {
                return;
            }
            inner.status.phase = phase;
            inner.status.microphone_active = microphone_active;
            inner.status.system_audio_active = system_audio_active;
            inner.status.elapsed_ms = elapsed_ms;
            if system_audio_active {
                inner.permission = SystemAudioPermissionState::Granted;
            }
            inner.status.clone()
        };
        publish_status(app, &status);
    }

    fn update_echo_cancellation(
        &self,
        app: &tauri::AppHandle,
        generation: u64,
        value: EchoCancellationStatus,
    ) {
        let (status, previous) = {
            let mut inner = self.inner.lock_or_recover();
            if inner
                .active
                .as_ref()
                .is_none_or(|active| active.generation != generation)
            {
                return;
            }
            let previous = inner.status.echo_cancellation;
            inner.status.echo_cancellation = match value {
                EchoCancellationStatus::Disabled => MeetingEchoCancellationRuntime::Off,
                EchoCancellationStatus::Active => MeetingEchoCancellationRuntime::Active,
                EchoCancellationStatus::Recovering {
                    reason,
                    attempt,
                    max_attempts,
                } => MeetingEchoCancellationRuntime::Recovering {
                    reason,
                    attempt,
                    max_attempts,
                },
                EchoCancellationStatus::Bypassed { reason } => {
                    MeetingEchoCancellationRuntime::Bypassed { reason }
                }
            };
            (inner.status.clone(), previous)
        };
        let reason = status
            .echo_cancellation
            .event_reason()
            .or_else(|| previous.event_reason())
            .unwrap_or("none");
        let recovery_attempt = status
            .echo_cancellation
            .recovery_attempt()
            .max(previous.recovery_attempt());
        let recovery_max_attempts = status
            .echo_cancellation
            .recovery_max_attempts()
            .max(previous.recovery_max_attempts());
        tracing::info!(
            target: "meeting",
            event_code = "meeting.echo_cancellation_state_changed",
            generation,
            from = previous.event_state(),
            to = status.echo_cancellation.event_state(),
            reason,
            recovery_attempt = u64::from(recovery_attempt),
            recovery_max_attempts = u64::from(recovery_max_attempts),
            "meeting echo cancellation state changed"
        );
        publish_status(app, &status);
    }

    fn finish(&self, app: &tauri::AppHandle, generation: u64, error: Option<&MeetingError>) {
        let status = {
            let mut inner = self.inner.lock_or_recover();
            if inner
                .active
                .as_ref()
                .is_none_or(|active| active.generation != generation)
            {
                return;
            }
            if error.is_some_and(|error| error.code == "system_audio_permission_denied") {
                inner.permission = SystemAudioPermissionState::Denied;
            }
            if error.is_some_and(|error| error.code == "unsupported_os") {
                inner.permission = SystemAudioPermissionState::Unsupported;
            }
            inner.active = None;
            inner.status.phase = if error.is_some() {
                MeetingRuntimePhase::Failed
            } else {
                MeetingRuntimePhase::Idle
            };
            inner.status.microphone_active = false;
            inner.status.system_audio_active = false;
            if error.is_none() {
                inner.status.session_id = None;
                inner.status.elapsed_ms = 0;
                inner.status.echo_cancellation = MeetingEchoCancellationRuntime::Off;
            }
            inner.status.error_code = error.map(|error| error.code.to_string());
            inner.status.clone()
        };
        if let Some(error) = error {
            tracing::warn!(
                target: "meeting",
                event_code = "meeting.capture_failed",
                generation,
                phase = "failed",
                error_code = error.code,
                "meeting capture failed"
            );
        } else {
            tracing::info!(
                target: "meeting",
                event_code = "meeting.capture_stopped",
                generation,
                phase = "idle",
                "meeting capture completed"
            );
        }
        publish_status(app, &status);
    }
}

fn publish_status(app: &tauri::AppHandle, status: &MeetingRuntimeStatus) {
    let _ = app.emit("meeting-status-changed", status);
}

fn helper_path() -> Result<std::path::PathBuf, MeetingError> {
    bundled_sibling("murmur-capture-worker")
        .or_else(|_| bundled_sibling("murmur-capture-worker-aarch64-apple-darwin"))
        .map_err(|_| {
            MeetingError::new(
                "worker_missing",
                "The signed meeting capture worker is missing from the app bundle.",
            )
        })
}

fn capture_identity() -> (u64, SessionNonce, String) {
    let capture_id = NEXT_MEETING_CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
    let nonce = *Uuid::new_v4().as_bytes();
    let encoded = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    (capture_id, nonce, encoded)
}

fn spawn_worker(
    capture_id: u64,
    nonce_hex: &str,
) -> Result<
    (
        ManagedChild,
        std::process::ChildStdin,
        std::process::ChildStdout,
    ),
    MeetingError,
> {
    let path = helper_path()?;
    if !cfg!(debug_assertions)
        && crate::code_signing::validate_bundled_helper(&path, CAPTURE_WORKER_IDENTIFIER).is_err()
    {
        return Err(MeetingError::new(
            "signature_invalid",
            "The bundled meeting capture worker failed integrity validation.",
        ));
    }
    let capture_id_text = capture_id.to_string();
    ManagedChild::spawn_with_arguments(
        &path,
        &["--production-v9", capture_id_text.as_str(), nonce_hex],
        &[],
    )
    .map_err(|_| {
        MeetingError::new(
            "worker_unavailable",
            "The meeting capture worker could not start.",
        )
    })
}

enum WorkerRead {
    Frame(ProductionFrame<ProductionHelperMessage>),
    Invalid,
}

fn spawn_reader(
    output: std::process::ChildStdout,
    capture_id: u64,
    nonce: SessionNonce,
) -> Receiver<WorkerRead> {
    // Backpressure must reach the worker pipe and its fixed callback rings.
    // An unbounded reader queue would turn a long meeting into unbounded host
    // RSS even though the segmenter queue below is bounded.
    let (sender, receiver) = mpsc::sync_channel(PCM_QUEUE_CAPACITY);
    thread::Builder::new()
        .name(format!("murmur-meeting-reader-{capture_id}"))
        .spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                match read_production_frame(&mut output, capture_id, nonce) {
                    Ok(frame) => {
                        if sender.send(WorkerRead::Frame(frame)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(WorkerRead::Invalid);
                        return;
                    }
                }
            }
        })
        .expect("meeting worker reader thread must spawn");
    receiver
}

fn terminate_worker(
    child: &mut ManagedChild,
    input: Option<std::process::ChildStdin>,
) -> Result<(), MeetingError> {
    drop(input);
    let deadline = Instant::now() + HELPER_STOP_DEADLINE;
    if child.wait_for_exit(deadline).is_some()
        || child
            .hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE)
            .is_some()
    {
        Ok(())
    } else {
        Err(MeetingError::new(
            "termination_unconfirmed",
            "The meeting worker could not be stopped safely. Restart Murmur before trying again.",
        ))
    }
}

/// Query the macOS Screen & System Audio Recording authorization without
/// touching Core Audio. This never prompts and returns immediately.
#[cfg(target_os = "macos")]
fn tcc_preflight() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

#[cfg(not(target_os = "macos"))]
fn tcc_preflight() -> bool {
    false
}

/// Surface the macOS authorization prompt when the app is not yet determined.
/// Returns immediately; the user's answer arrives via a later preflight.
#[cfg(target_os = "macos")]
fn tcc_request() -> bool {
    core_graphics::access::ScreenCaptureAccess.request()
}

#[cfg(not(target_os = "macos"))]
fn tcc_request() -> bool {
    false
}

fn probe_permission() -> Result<SystemAudioAccess, String> {
    let authorized_before = tcc_preflight();
    tracing::info!(
        target: "meeting",
        event_code = "meeting.permission_probe_started",
        tcc_authorized = authorized_before,
        "system audio permission probe started"
    );
    // Not-yet-determined apps need the system prompt before the tap can
    // succeed. Already-authorized apps must never be prompted again.
    if !authorized_before {
        tcc_request();
    }
    let outcome = probe_permission_tap();
    match &outcome {
        Ok(access) => tracing::info!(
            target: "meeting",
            event_code = "meeting.permission_probe_finished",
            tcc_authorized = authorized_before,
            permission = access.permission.as_event_value(),
            capture_ready = access.capture_ready,
            audio_flowing = access.audio_flowing,
            needs_relaunch = access.needs_relaunch,
            "system audio permission probe finished"
        ),
        Err(_) => tracing::warn!(
            target: "meeting",
            event_code = "meeting.permission_probe_failed",
            tcc_authorized = authorized_before,
            "system audio permission probe did not complete"
        ),
    }
    outcome.map(|access| resolve_access(authorized_before, access))
}

/// Combine the TCC answer with the tap result. The tap is authoritative for
/// permission; TCC only explains a contradiction. macOS reporting the app as
/// authorized while Core Audio refuses the tap means the grant has not reached
/// this process image, which a relaunch clears.
fn resolve_access(authorized_before: bool, probed: SystemAudioAccess) -> SystemAudioAccess {
    SystemAudioAccess {
        needs_relaunch: authorized_before
            && probed.permission == SystemAudioPermissionState::Denied,
        ..probed
    }
}

fn probe_permission_tap() -> Result<SystemAudioAccess, String> {
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (mut child, mut input, output) =
        spawn_worker(capture_id, &nonce_hex).map_err(|error| error.message.to_string())?;
    let receiver = spawn_reader(output, capture_id, nonce);
    write_production_control(&mut input, capture_id, nonce, &ProductionHostMessage::Hello)
        .map_err(|_| "The meeting capture protocol failed.".to_string())?;
    match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(WorkerRead::Frame(ProductionFrame::Control(ProductionHelperMessage::HelloAck))) => {}
        _ => {
            let _ = terminate_worker(&mut child, Some(input));
            return Err("The meeting capture worker did not complete its handshake.".to_string());
        }
    }
    write_production_control(
        &mut input,
        capture_id,
        nonce,
        &ProductionHostMessage::ProbeSystemAudio,
    )
    .map_err(|_| "The System Audio permission request could not start.".to_string())?;
    let deadline = Instant::now() + START_PERMISSION_DEADLINE;
    let mut setup_watchdog = SetupWatchdog::default();
    let result = loop {
        if setup_watchdog.expired() {
            break Err(
                "Core Audio stalled while starting System Audio. Quit other audio capture apps and try again."
                    .to_string(),
            );
        }
        if Instant::now() >= deadline {
            break Err("The System Audio permission request did not complete.".to_string());
        }
        match receiver.recv_timeout(FRAME_POLL) {
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingSetupStep {
                    channel,
                    step,
                    transition,
                },
            ))) => setup_watchdog.observe(channel, step, transition),
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::SystemAudioPermission {
                    status,
                    audio_flowing,
                },
            ))) => {
                let permission = match status {
                    SystemAudioPermissionStatus::Granted => SystemAudioPermissionState::Granted,
                    SystemAudioPermissionStatus::Denied => SystemAudioPermissionState::Denied,
                    SystemAudioPermissionStatus::Unsupported => {
                        SystemAudioPermissionState::Unsupported
                    }
                };
                break Ok(SystemAudioAccess {
                    permission,
                    capture_ready: permission == SystemAudioPermissionState::Granted,
                    audio_flowing,
                    needs_relaunch: false,
                });
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingFailure { code, .. },
            ))) => {
                break Err(user_message_for_failure(code, Some(CaptureChannel::System)).to_string())
            }
            Ok(WorkerRead::Invalid) | Err(RecvTimeoutError::Disconnected) => {
                break Err("The System Audio permission request did not complete.".to_string())
            }
            Ok(WorkerRead::Frame(_)) | Err(RecvTimeoutError::Timeout) => {}
        }
    };
    let _ = terminate_worker(&mut child, Some(input));
    result
}

#[derive(Default)]
struct ChannelSequence {
    next_sequence: u64,
    next_sample_offset: u64,
    sample_rate: Option<u32>,
}

struct AecProtocolTracker {
    requested: EchoCancellationMode,
    observed: Option<EchoCancellationStatus>,
    recovery_attempts: u8,
}

impl AecProtocolTracker {
    fn new(requested: EchoCancellationMode) -> Self {
        Self {
            requested,
            observed: None,
            recovery_attempts: 0,
        }
    }

    fn observe(&mut self, status: EchoCancellationStatus) -> bool {
        let valid = matches!(
            (self.requested, self.observed, status),
            (
                EchoCancellationMode::Disabled,
                None,
                EchoCancellationStatus::Disabled
            ) | (
                EchoCancellationMode::Enabled,
                None,
                EchoCancellationStatus::Active | EchoCancellationStatus::Bypassed { .. },
            ) | (
                EchoCancellationMode::Enabled,
                Some(EchoCancellationStatus::Active),
                EchoCancellationStatus::Bypassed { .. },
            ) | (
                EchoCancellationMode::Enabled,
                Some(EchoCancellationStatus::Active),
                EchoCancellationStatus::Recovering { .. },
            ) | (
                EchoCancellationMode::Enabled,
                Some(EchoCancellationStatus::Recovering { .. }),
                EchoCancellationStatus::Active | EchoCancellationStatus::Bypassed { .. },
            ) | (
                EchoCancellationMode::Enabled,
                Some(EchoCancellationStatus::Recovering { .. }),
                EchoCancellationStatus::Recovering { .. },
            )
        );
        if !valid {
            return false;
        }
        if let EchoCancellationStatus::Recovering {
            reason,
            attempt,
            max_attempts,
        } = status
        {
            let previous_reason = match self.observed {
                Some(EchoCancellationStatus::Recovering { reason, .. }) => Some(reason),
                _ => None,
            };
            if attempt != self.recovery_attempts.saturating_add(1)
                || attempt > max_attempts
                || max_attempts != MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS
                || previous_reason.is_some_and(|previous| previous != reason)
                || !matches!(
                    reason,
                    EchoCancellationBypassReason::RenderDiscontinuity
                        | EchoCancellationBypassReason::ProcessingBacklog
                )
            {
                return false;
            }
            self.recovery_attempts = attempt;
        }
        self.observed = Some(status);
        true
    }

    fn permits_microphone_pcm(&self) -> bool {
        self.observed.is_some()
    }
}

#[derive(Default)]
struct SetupWatchdog {
    pending: Option<(CaptureChannel, CaptureSetupStep, Instant)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MeetingInputResolutionState {
    #[default]
    NotStarted,
    Started,
    Observed,
    Completed,
}

#[derive(Clone, Copy)]
struct MeetingInputResolutionEvidence {
    input_enumeration_ok: bool,
    requested_present: Option<bool>,
}

#[derive(Default)]
struct MeetingInputResolutionTracker {
    state: MeetingInputResolutionState,
    requested_device: bool,
    evidence: Option<MeetingInputResolutionEvidence>,
}

impl MeetingInputResolutionTracker {
    fn observe_setup(
        &mut self,
        channel: CaptureChannel,
        step: CaptureSetupStep,
        transition: SetupTransition,
    ) -> bool {
        if channel != CaptureChannel::Microphone {
            return true;
        }
        match (step, transition, self.state) {
            (
                CaptureSetupStep::DeviceResolution,
                SetupTransition::Entered,
                MeetingInputResolutionState::NotStarted,
            ) => self.state = MeetingInputResolutionState::Started,
            (
                CaptureSetupStep::DeviceResolution,
                SetupTransition::Completed,
                MeetingInputResolutionState::Observed,
            ) => self.state = MeetingInputResolutionState::Completed,
            (CaptureSetupStep::DeviceResolution, _, _) => return false,
            (_, _, MeetingInputResolutionState::Completed) => {}
            _ => return false,
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_evidence(
        &mut self,
        expected_backend: CaptureBackend,
        reported_backend: CaptureBackend,
        requested_device: bool,
        input_enumeration_ok: bool,
        requested_present: Option<bool>,
        input_device_count: u16,
        input_device_count_capped: bool,
    ) -> bool {
        if reported_backend != expected_backend
            || self.state != MeetingInputResolutionState::Started
            || !valid_input_resolution_evidence(
                requested_device,
                input_enumeration_ok,
                requested_present,
                input_device_count,
                input_device_count_capped,
            )
        {
            return false;
        }
        self.requested_device = requested_device;
        self.evidence = Some(MeetingInputResolutionEvidence {
            input_enumeration_ok,
            requested_present,
        });
        self.state = MeetingInputResolutionState::Observed;
        true
    }

    fn permits_phase(&self, channel: CaptureChannel, phase: CapturePhase) -> bool {
        channel != CaptureChannel::Microphone
            || !matches!(
                phase,
                CapturePhase::AwaitingFirstCallback | CapturePhase::Active
            )
            || self.state == MeetingInputResolutionState::Completed
    }

    fn permits_pcm(&self, channel: CaptureChannel) -> bool {
        channel != CaptureChannel::Microphone
            || self.state == MeetingInputResolutionState::Completed
    }

    fn permits_failure(&self, code: FailureCode, channel: Option<CaptureChannel>) -> bool {
        if channel != Some(CaptureChannel::Microphone)
            && matches!(
                code,
                FailureCode::NoInputDevice | FailureCode::EnumerationFailed
            )
        {
            return false;
        }
        if channel != Some(CaptureChannel::Microphone) {
            return true;
        }
        match self.state {
            MeetingInputResolutionState::Observed => {
                let Some(evidence) = self.evidence else {
                    return false;
                };
                match code {
                    FailureCode::NoInputDevice if self.requested_device => {
                        evidence.input_enumeration_ok && evidence.requested_present == Some(false)
                    }
                    FailureCode::NoInputDevice => evidence.requested_present.is_none(),
                    FailureCode::EnumerationFailed => !evidence.input_enumeration_ok,
                    _ => false,
                }
            }
            MeetingInputResolutionState::Completed => !matches!(
                code,
                FailureCode::NoInputDevice | FailureCode::EnumerationFailed
            ),
            MeetingInputResolutionState::NotStarted => code == FailureCode::PermissionDenied,
            MeetingInputResolutionState::Started => false,
        }
    }
}

impl SetupWatchdog {
    fn observe(
        &mut self,
        channel: CaptureChannel,
        step: CaptureSetupStep,
        transition: SetupTransition,
    ) {
        match transition {
            SetupTransition::Entered => self.pending = Some((channel, step, Instant::now())),
            SetupTransition::Completed
                if self
                    .pending
                    .is_some_and(|(pending_channel, pending_step, _)| {
                        pending_channel == channel && pending_step == step
                    }) =>
            {
                self.pending = None;
            }
            SetupTransition::Completed => {}
        }
    }

    fn expired(&self) -> bool {
        self.pending.is_some_and(|(_, step, entered_at)| {
            step != CaptureSetupStep::SystemTapCreate && entered_at.elapsed() >= SETUP_STEP_DEADLINE
        })
    }
}

impl ChannelSequence {
    fn validate(&mut self, pcm: &ProductionPcm) -> bool {
        let valid = pcm.sequence == self.next_sequence
            && pcm.sample_offset == self.next_sample_offset
            && self
                .sample_rate
                .is_none_or(|sample_rate| sample_rate == pcm.sample_rate)
            && pcm.sample_rate >= 8_000
            && !pcm.samples.is_empty();
        if valid {
            self.next_sequence += 1;
            self.next_sample_offset += pcm.samples.len() as u64;
            self.sample_rate = Some(pcm.sample_rate);
        }
        valid
    }
}

enum PcmQueueError {
    Full,
    Disconnected,
}

fn send_pcm_with_retry(
    sender: &mpsc::SyncSender<ProductionPcm>,
    mut pcm: ProductionPcm,
) -> Result<(), PcmQueueError> {
    let deadline = Instant::now() + PCM_QUEUE_RETRY_DEADLINE;
    loop {
        match sender.try_send(pcm) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => return Err(PcmQueueError::Disconnected),
            Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                pcm = returned;
                thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Full(_)) => return Err(PcmQueueError::Full),
        }
    }
}

fn run_capture_session(
    app: &tauri::AppHandle,
    repository: &MeetingRepository,
    config: &MeetingCaptureConfig,
    command_receiver: Receiver<MeetingCommand>,
    coordinator: &MeetingCoordinator,
) -> Result<(), MeetingError> {
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (mut child, mut input, output) = spawn_worker(capture_id, &nonce_hex)?;
    let receiver = spawn_reader(output, capture_id, nonce);
    write_production_control(&mut input, capture_id, nonce, &ProductionHostMessage::Hello)
        .map_err(|_| MeetingError::new("protocol_error", "Meeting capture failed to start."))?;
    match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(WorkerRead::Frame(ProductionFrame::Control(ProductionHelperMessage::HelloAck))) => {}
        _ => {
            let _ = terminate_worker(&mut child, Some(input));
            return Err(MeetingError::new(
                "protocol_error",
                "The meeting capture worker did not complete its handshake.",
            ));
        }
    }

    let (pcm_sender, pcm_receiver) = mpsc::sync_channel(PCM_QUEUE_CAPACITY);
    let processing_app = app.clone();
    let processing_repository = repository.clone();
    let processing_config = config.clone();
    let (processing_result_sender, processing_result_receiver) = mpsc::sync_channel(1);
    let (processing_ready_sender, processing_ready_receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name(format!("murmur-meeting-segmenter-{}", config.generation))
        .spawn(move || {
            let result = process_pcm_stream(
                &processing_app,
                &processing_repository,
                &processing_config,
                pcm_receiver,
                processing_ready_sender,
            );
            let _ = processing_result_sender.send(result);
        })
        .map_err(|_| {
            MeetingError::new(
                "segmenter_unavailable",
                "The meeting segmenter could not start.",
            )
        })?;

    if processing_ready_receiver
        .recv_timeout(PROCESSING_READY_DEADLINE)
        .is_err()
    {
        drop(pcm_sender);
        let error = processing_result_receiver
            .recv_timeout(Duration::from_secs(1))
            .ok()
            .and_then(Result::err)
            .unwrap_or_else(|| {
                MeetingError::new(
                    "segmenter_unavailable",
                    "The meeting segmenter did not become ready.",
                )
            });
        let _ = terminate_worker(&mut child, Some(input));
        return Err(error);
    }

    let microphone_backend = CaptureBackend::Auhal;
    write_production_control(
        &mut input,
        capture_id,
        nonce,
        &ProductionHostMessage::StartMeeting {
            device_id: config.device_id.clone(),
            backend: microphone_backend,
            echo_cancellation: config.echo_cancellation,
        },
    )
    .map_err(|_| MeetingError::new("protocol_error", "Meeting capture failed to start."))?;

    let started_at = Instant::now();
    let mut microphone_active = false;
    let mut system_audio_active = false;
    let mut microphone_sequence = ChannelSequence::default();
    let mut system_sequence = ChannelSequence::default();
    let mut setup_watchdog = SetupWatchdog::default();
    let mut input_resolution = MeetingInputResolutionTracker::default();
    let mut aec_protocol = AecProtocolTracker::new(config.echo_cancellation);
    let mut stopping = false;
    let mut stop_requested_at = None;
    let mut terminal_error = None;

    loop {
        if !stopping && command_receiver.try_recv().is_ok() {
            stopping = true;
            coordinator.update_phase(
                app,
                config.generation,
                MeetingRuntimePhase::Stopping,
                microphone_active,
                system_audio_active,
                started_at.elapsed().as_millis() as u64,
            );
            if !microphone_active || !system_audio_active {
                let _ = write_production_control(
                    &mut input,
                    capture_id,
                    nonce,
                    &ProductionHostMessage::Cancel,
                );
                break;
            }
            stop_requested_at = Some(Instant::now());
            let _ = write_production_control(
                &mut input,
                capture_id,
                nonce,
                &ProductionHostMessage::Stop,
            );
        }
        if stop_requested_at.is_some_and(|started| started.elapsed() >= MEETING_STOP_ACK_DEADLINE) {
            terminal_error = Some(MeetingError::new(
                "capture_stop_timeout",
                "The meeting capture worker did not stop cleanly.",
            ));
            break;
        }
        if !stopping && setup_watchdog.expired() {
            terminal_error = Some(MeetingError::new(
                "capture_setup_timeout",
                "Core Audio stalled while starting meeting capture.",
            ));
            break;
        }
        if !stopping
            && (!microphone_active || !system_audio_active)
            && started_at.elapsed() >= START_PERMISSION_DEADLINE
        {
            terminal_error = Some(MeetingError::new(
                "permission_prompt_timeout",
                "Meeting capture permission was not decided before the prompt deadline.",
            ));
            let _ = write_production_control(
                &mut input,
                capture_id,
                nonce,
                &ProductionHostMessage::Cancel,
            );
            break;
        }
        match processing_result_receiver.try_recv() {
            Ok(Err(error)) => {
                terminal_error = Some(error);
                let _ = write_production_control(
                    &mut input,
                    capture_id,
                    nonce,
                    &ProductionHostMessage::Cancel,
                );
                break;
            }
            Ok(Ok(())) | Err(TryRecvError::Disconnected) => {
                terminal_error = Some(MeetingError::new(
                    "segmenter_unavailable",
                    "The meeting segmenter stopped unexpectedly.",
                ));
                let _ = write_production_control(
                    &mut input,
                    capture_id,
                    nonce,
                    &ProductionHostMessage::Cancel,
                );
                break;
            }
            Err(TryRecvError::Empty) => {}
        }

        match receiver.recv_timeout(FRAME_POLL) {
            Ok(WorkerRead::Frame(ProductionFrame::Pcm(pcm))) => {
                if !input_resolution.permits_pcm(pcm.channel) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned microphone audio before completing input resolution.",
                    ));
                    break;
                }
                if pcm.channel == CaptureChannel::Microphone
                    && !aec_protocol.permits_microphone_pcm()
                {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned microphone audio before reporting echo-cancellation state.",
                    ));
                    break;
                }
                let sequence = match pcm.channel {
                    CaptureChannel::Microphone => &mut microphone_sequence,
                    CaptureChannel::System => &mut system_sequence,
                };
                if !sequence.validate(&pcm) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned an invalid audio sequence.",
                    ));
                    break;
                }
                match send_pcm_with_retry(&pcm_sender, pcm) {
                    Ok(()) => {}
                    Err(PcmQueueError::Full) => {
                        terminal_error = Some(MeetingError::new(
                            "capture_backlog",
                            "Meeting audio processing could not keep up safely.",
                        ));
                        break;
                    }
                    Err(PcmQueueError::Disconnected) => {
                        terminal_error = Some(MeetingError::new(
                            "segmenter_unavailable",
                            "The meeting segmenter stopped unexpectedly.",
                        ));
                        break;
                    }
                }
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingEchoCancellation { status },
            ))) => {
                if !aec_protocol.observe(status) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned an invalid echo-cancellation transition.",
                    ));
                    break;
                }
                coordinator.update_echo_cancellation(app, config.generation, status);
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingPhase { phase, channel },
            ))) => {
                if !input_resolution.permits_phase(channel, phase) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker advanced before completing input resolution.",
                    ));
                    break;
                }
                if phase == CapturePhase::Active {
                    match channel {
                        CaptureChannel::Microphone => microphone_active = true,
                        CaptureChannel::System => system_audio_active = true,
                    }
                    tracing::info!(
                        target: "meeting",
                        event_code = "meeting.channel_active",
                        generation = config.generation,
                        channel = match channel {
                            CaptureChannel::Microphone => "microphone",
                            CaptureChannel::System => "system",
                        },
                        "meeting channel delivered first PCM"
                    );
                    if channel == CaptureChannel::System && system_audio_active {
                        tracing::info!(
                            target: "meeting",
                            event_code = "meeting.tap_active",
                            generation = config.generation,
                            channel = "system",
                            "system audio tap delivered first PCM"
                        );
                    }
                    if !stopping {
                        coordinator.update_phase(
                            app,
                            config.generation,
                            if microphone_active && system_audio_active {
                                MeetingRuntimePhase::Recording
                            } else {
                                MeetingRuntimePhase::Starting
                            },
                            microphone_active,
                            system_audio_active,
                            started_at.elapsed().as_millis() as u64,
                        );
                    }
                }
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::InputResolution {
                    backend,
                    input_enumeration_ok,
                    requested_present,
                    input_device_count,
                    input_device_count_capped,
                    ..
                },
            ))) => {
                if !input_resolution.observe_evidence(
                    microphone_backend,
                    backend,
                    config.device_id.is_some(),
                    input_enumeration_ok,
                    requested_present,
                    input_device_count,
                    input_device_count_capped,
                ) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned invalid input resolution evidence.",
                    ));
                    break;
                }
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingSetupStep {
                    channel,
                    step,
                    transition,
                },
            ))) => {
                if !input_resolution.observe_setup(channel, step, transition) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned input resolution steps out of order.",
                    ));
                    break;
                }
                setup_watchdog.observe(channel, step, transition);
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingFailure { code, channel, .. },
            ))) => {
                if !input_resolution.permits_failure(code, channel) {
                    terminal_error = Some(MeetingError::new(
                        "protocol_error",
                        "The meeting capture worker returned a failure inconsistent with input resolution.",
                    ));
                    break;
                }
                terminal_error = Some(meeting_error_for_failure(code, channel));
                break;
            }
            Ok(WorkerRead::Frame(ProductionFrame::Control(
                ProductionHelperMessage::MeetingStopped { .. },
            ))) => break,
            Ok(WorkerRead::Frame(_)) => {}
            Ok(WorkerRead::Invalid) | Err(RecvTimeoutError::Disconnected) => {
                terminal_error = Some(MeetingError::new(
                    "protocol_error",
                    "The meeting capture worker stopped unexpectedly.",
                ));
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }

    if terminal_error.is_some() {
        let _ = write_production_control(
            &mut input,
            capture_id,
            nonce,
            &ProductionHostMessage::Cancel,
        );
    }
    drop(pcm_sender);
    coordinator.update_phase(
        app,
        config.generation,
        MeetingRuntimePhase::Processing,
        microphone_active,
        system_audio_active,
        started_at.elapsed().as_millis() as u64,
    );
    let termination_result = terminate_worker(&mut child, Some(input));
    if termination_result.is_ok() {
        tracing::info!(
            target: "meeting",
            event_code = "meeting.tap_destroyed",
            generation = config.generation,
            channel = "system",
            "meeting capture worker termination confirmed"
        );
    } else {
        tracing::warn!(
            target: "meeting",
            event_code = "meeting.capture_failed",
            generation = config.generation,
            phase = "failed",
            error_code = "termination_unconfirmed",
            "meeting capture worker termination could not be confirmed"
        );
    }
    let processing_result = processing_result_receiver.recv().unwrap_or_else(|_| {
        Err(MeetingError::new(
            "segmenter_unavailable",
            "The meeting segmenter stopped unexpectedly.",
        ))
    });
    termination_result?;
    if let Some(error) = terminal_error {
        return Err(error);
    }
    processing_result
}

fn user_message_for_failure(code: FailureCode, channel: Option<CaptureChannel>) -> &'static str {
    match (code, channel) {
        (FailureCode::UnsupportedOs, _) => "Meeting capture requires macOS 14.2 or newer.",
        (FailureCode::PermissionDenied, Some(CaptureChannel::System)) => {
            "System Audio access was denied. Grant it in System Settings and try again."
        }
        (FailureCode::PermissionDenied, _) => {
            "Microphone access was denied. Grant it in System Settings and try again."
        }
        (FailureCode::NoInputDevice, _) => "The selected microphone is unavailable.",
        (FailureCode::SystemAudioUnavailable, _) => {
            "System Audio capture is unavailable. Try again after checking your output device."
        }
        (FailureCode::CallbackStalled, Some(CaptureChannel::System)) => {
            "System Audio started but delivered no audio. Quit other audio capture apps and try again."
        }
        (FailureCode::CallbackStalled, _) => {
            "The microphone started but delivered no audio. Quit other audio capture apps and try again."
        }
        _ => "Meeting audio capture failed. Try again.",
    }
}

fn meeting_error_for_failure(code: FailureCode, channel: Option<CaptureChannel>) -> MeetingError {
    let stable = match (code, channel) {
        (FailureCode::UnsupportedOs, _) => "unsupported_os",
        (FailureCode::PermissionDenied, Some(CaptureChannel::System)) => {
            "system_audio_permission_denied"
        }
        (FailureCode::PermissionDenied, _) => "microphone_permission_denied",
        (FailureCode::NoInputDevice, _) => "microphone_unavailable",
        (FailureCode::SystemAudioUnavailable, _) => "system_audio_unavailable",
        (FailureCode::CallbackStalled, Some(CaptureChannel::System)) => {
            "system_audio_callback_stalled"
        }
        (FailureCode::CallbackStalled, _) => "microphone_callback_stalled",
        _ => "capture_failed",
    };
    MeetingError::new(stable, user_message_for_failure(code, channel))
}

struct StreamingResampler {
    input_rate: Option<u32>,
    input_samples_seen: u64,
    next_output_numerator: u64,
    previous_sample: Option<f32>,
}

impl StreamingResampler {
    fn new() -> Self {
        Self {
            input_rate: None,
            input_samples_seen: 0,
            next_output_numerator: 0,
            previous_sample: None,
        }
    }

    fn push(&mut self, sample_rate: u32, samples: &[f32]) -> Result<Vec<f32>, MeetingError> {
        if sample_rate < WHISPER_SAMPLE_RATE {
            return Err(MeetingError::new(
                "unsupported_sample_rate",
                "Meeting audio used an unsupported sample rate.",
            ));
        }
        if self.input_rate.is_some_and(|rate| rate != sample_rate) {
            return Err(MeetingError::new(
                "sample_rate_changed",
                "A meeting audio device changed sample rate during capture.",
            ));
        }
        self.input_rate = Some(sample_rate);
        if sample_rate == WHISPER_SAMPLE_RATE {
            self.input_samples_seen += samples.len() as u64;
            self.previous_sample = samples.last().copied();
            return Ok(samples.to_vec());
        }
        let mut output = Vec::with_capacity(
            samples.len() * WHISPER_SAMPLE_RATE as usize / sample_rate as usize + 2,
        );
        let output_rate = WHISPER_SAMPLE_RATE as u64;
        for &sample in samples {
            let input_index = self.input_samples_seen;
            if input_index == 0 {
                output.push(sample);
                self.next_output_numerator = sample_rate as u64;
            } else if let Some(previous) = self.previous_sample {
                let interval_start = (input_index - 1).saturating_mul(output_rate);
                let interval_end = input_index.saturating_mul(output_rate);
                while self.next_output_numerator <= interval_end {
                    let fraction =
                        (self.next_output_numerator - interval_start) as f32 / output_rate as f32;
                    output.push(previous * (1.0 - fraction) + sample * fraction);
                    self.next_output_numerator += sample_rate as u64;
                }
            }
            self.previous_sample = Some(sample);
            self.input_samples_seen += 1;
        }
        Ok(output)
    }
}

#[derive(Debug)]
struct SpeechChunk {
    start_sample: u64,
    end_sample: u64,
    samples: Vec<f32>,
}

struct VadChunker {
    vad_sensitivity: u32,
    vad_model_path: Option<String>,
    pending_analysis: Vec<f32>,
    pre_roll: VecDeque<f32>,
    active: Vec<f32>,
    active_start_sample: u64,
    trailing_silence: usize,
    processed_samples: u64,
}

impl VadChunker {
    fn new(vad_sensitivity: u32) -> Result<Self, MeetingError> {
        let vad_model_path = if crate::vad::is_enabled(vad_sensitivity) {
            let path = crate::vad::vad_model_path()
                .filter(|path| path.exists())
                .ok_or_else(|| {
                    MeetingError::new(
                        "vad_model_missing",
                        "Install the Voice Activity Detection model before starting a meeting.",
                    )
                })?;
            Some(path.to_string_lossy().into_owned())
        } else {
            None
        };
        if let Some(path) = vad_model_path.as_deref() {
            let threshold = 1.0 - (vad_sensitivity as f32 / 100.0);
            crate::vad::filter_speech(path, &vec![0.0; ANALYSIS_SAMPLES], threshold).map_err(
                |_| {
                    MeetingError::new(
                        "vad_failed",
                        "Meeting speech detection could not become ready.",
                    )
                },
            )?;
        }
        Ok(Self {
            vad_sensitivity,
            vad_model_path,
            pending_analysis: Vec::with_capacity(ANALYSIS_SAMPLES * 2),
            pre_roll: VecDeque::with_capacity(PRE_ROLL_SAMPLES),
            active: Vec::with_capacity(MAX_CHUNK_SAMPLES + ANALYSIS_SAMPLES),
            active_start_sample: 0,
            trailing_silence: 0,
            processed_samples: 0,
        })
    }

    fn push(&mut self, samples: &[f32]) -> Result<Vec<SpeechChunk>, MeetingError> {
        self.pending_analysis.extend_from_slice(samples);
        let mut chunks = Vec::new();
        while self.pending_analysis.len() >= ANALYSIS_SAMPLES {
            let window = self
                .pending_analysis
                .drain(..ANALYSIS_SAMPLES)
                .collect::<Vec<_>>();
            if let Some(chunk) = self.process_window(&window)? {
                chunks.push(chunk);
            }
        }
        Ok(chunks)
    }

    fn flush(&mut self) -> Result<Vec<SpeechChunk>, MeetingError> {
        let mut chunks = Vec::new();
        if !self.pending_analysis.is_empty() {
            let window = std::mem::take(&mut self.pending_analysis);
            if let Some(chunk) = self.process_window(&window)? {
                chunks.push(chunk);
            }
        }
        if let Some(chunk) = self.finish_active() {
            chunks.push(chunk);
        }
        Ok(chunks)
    }

    fn process_window(&mut self, window: &[f32]) -> Result<Option<SpeechChunk>, MeetingError> {
        let window_start = self.processed_samples;
        self.processed_samples += window.len() as u64;
        let speech = if let Some(path) = self.vad_model_path.as_deref() {
            let threshold = 1.0 - (self.vad_sensitivity as f32 / 100.0);
            match crate::vad::filter_speech(path, window, threshold) {
                Ok(crate::vad::VadResult::Speech(_)) => true,
                Ok(crate::vad::VadResult::NoSpeech) => false,
                Err(_) => {
                    return Err(MeetingError::new(
                        "vad_failed",
                        "Meeting speech detection stopped unexpectedly.",
                    ));
                }
            }
        } else {
            true
        };

        if speech {
            if self.active.is_empty() {
                self.active_start_sample = window_start.saturating_sub(self.pre_roll.len() as u64);
                self.active.extend(self.pre_roll.drain(..));
            }
            self.active.extend_from_slice(window);
            self.trailing_silence = 0;
        } else if self.active.is_empty() {
            self.pre_roll.extend(window.iter().copied());
            while self.pre_roll.len() > PRE_ROLL_SAMPLES {
                self.pre_roll.pop_front();
            }
        } else {
            self.active.extend_from_slice(window);
            self.trailing_silence += window.len();
        }

        if self.active.len() >= MAX_CHUNK_SAMPLES
            || self.trailing_silence >= TRAILING_SILENCE_SAMPLES
        {
            Ok(self.finish_active())
        } else {
            Ok(None)
        }
    }

    fn finish_active(&mut self) -> Option<SpeechChunk> {
        if self.active.len() < MIN_CHUNK_SAMPLES {
            self.active.clear();
            self.trailing_silence = 0;
            return None;
        }
        let samples = std::mem::take(&mut self.active);
        let start_sample = self.active_start_sample;
        let end_sample = start_sample + samples.len() as u64;
        self.trailing_silence = 0;
        self.pre_roll.clear();
        Some(SpeechChunk {
            start_sample,
            end_sample,
            samples,
        })
    }
}

struct ChannelProcessor {
    speaker: MeetingSpeaker,
    sequence: u64,
    base_ns: Option<u64>,
    resampler: StreamingResampler,
    chunker: VadChunker,
}

impl ChannelProcessor {
    fn new(speaker: MeetingSpeaker, vad_sensitivity: u32) -> Result<Self, MeetingError> {
        Ok(Self {
            speaker,
            sequence: 0,
            base_ns: None,
            resampler: StreamingResampler::new(),
            chunker: VadChunker::new(vad_sensitivity)?,
        })
    }

    fn push(&mut self, pcm: ProductionPcm) -> Result<Vec<SpeechChunk>, MeetingError> {
        self.base_ns.get_or_insert_with(|| {
            pcm.captured_at_ns.saturating_sub(
                pcm.sample_offset
                    .saturating_mul(1_000_000_000)
                    .checked_div(pcm.sample_rate as u64)
                    .unwrap_or_default(),
            )
        });
        let samples = self.resampler.push(pcm.sample_rate, &pcm.samples)?;
        self.chunker.push(&samples)
    }

    fn timing(&self, chunk: &SpeechChunk) -> (u64, u64) {
        let base_ms = self.base_ns.unwrap_or_default() / 1_000_000;
        (
            base_ms + chunk.start_sample * 1_000 / WHISPER_SAMPLE_RATE as u64,
            base_ms + chunk.end_sample * 1_000 / WHISPER_SAMPLE_RATE as u64,
        )
    }
}

fn process_pcm_stream(
    app: &tauri::AppHandle,
    repository: &MeetingRepository,
    config: &MeetingCaptureConfig,
    receiver: Receiver<ProductionPcm>,
    ready_sender: mpsc::SyncSender<()>,
) -> Result<(), MeetingError> {
    let mut microphone = ChannelProcessor::new(MeetingSpeaker::Me, config.vad_sensitivity)?;
    let mut system = ChannelProcessor::new(MeetingSpeaker::Them, config.vad_sensitivity)?;
    let (wake_sender, wake_receiver) = mpsc::sync_channel(INFERENCE_WAKE_CAPACITY);
    let inference_app = app.clone();
    let inference_repository = repository.clone();
    let inference = thread::Builder::new()
        .name(format!("murmur-meeting-inference-{}", config.generation))
        .spawn(move || run_inference_worker(&inference_app, &inference_repository, wake_receiver))
        .map_err(|_| {
            MeetingError::new(
                "inference_unavailable",
                "The meeting transcription worker could not start.",
            )
        })?;

    if ready_sender.send(()).is_err() {
        drop(wake_sender);
        let _ = inference.join();
        return Err(MeetingError::new(
            "segmenter_unavailable",
            "The meeting segmenter lost its host before capture began.",
        ));
    }

    while let Ok(pcm) = receiver.recv() {
        let processor = match pcm.channel {
            CaptureChannel::Microphone => &mut microphone,
            CaptureChannel::System => &mut system,
        };
        for chunk in processor.push(pcm)? {
            persist_chunk(repository, config, processor, chunk)?;
            let _ = wake_sender.try_send(());
        }
    }
    for processor in [&mut microphone, &mut system] {
        for chunk in processor.chunker.flush()? {
            persist_chunk(repository, config, processor, chunk)?;
            let _ = wake_sender.try_send(());
        }
    }
    drop(wake_sender);
    inference.join().map_err(|_| {
        MeetingError::new(
            "inference_panicked",
            "The meeting transcription worker stopped unexpectedly.",
        )
    })?;
    Ok(())
}

fn persist_chunk(
    repository: &MeetingRepository,
    config: &MeetingCaptureConfig,
    processor: &mut ChannelProcessor,
    chunk: SpeechChunk,
) -> Result<(), MeetingError> {
    let (start_ms, end_ms) = processor.timing(&chunk);
    let speaker = processor.speaker;
    let sequence = processor.sequence;
    processor.sequence += 1;
    let relative = format!(
        "audio/{}/{}-{sequence:08}.wav",
        config.session_id,
        speaker.as_db()
    );
    write_spool_wav(repository.root(), &relative, &chunk.samples)?;
    if repository
        .insert_pending_segment(
            &config.session_id,
            speaker,
            sequence,
            start_ms,
            end_ms,
            &relative,
        )
        .is_err()
    {
        let _ = fs::remove_file(repository.root().join(relative));
        return Err(MeetingError::new(
            "store_unavailable",
            "The meeting transcript store became unavailable.",
        ));
    }
    Ok(())
}

fn write_spool_wav(
    root: &std::path::Path,
    relative: &str,
    samples: &[f32],
) -> Result<(), MeetingError> {
    let destination = root.join(relative);
    let parent = destination.parent().ok_or_else(|| {
        MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
    })?;
    fs::create_dir_all(parent).map_err(|_| {
        MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
    })?;
    let temporary = destination.with_extension("wav.tmp");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&temporary, spec).map_err(|_| {
        MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
    })?;
    for sample in samples {
        let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer.write_sample(scaled).map_err(|_| {
            MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
        })?;
    }
    writer.finalize().map_err(|_| {
        MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
    })?;
    fs::File::open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|_| {
            MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
        })?;
    fs::rename(&temporary, &destination).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        MeetingError::new("spool_failed", "Meeting audio could not be stored safely.")
    })?;
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn run_inference_worker(
    app: &tauri::AppHandle,
    repository: &MeetingRepository,
    wake_receiver: Receiver<()>,
) {
    loop {
        match wake_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                drain_pending(app, repository);
                return;
            }
        }
        drain_pending(app, repository);
    }
}

fn drain_pending(app: &tauri::AppHandle, repository: &MeetingRepository) {
    while let Ok(Some(segment)) = repository.next_pending_segment() {
        process_pending_segment(app, repository, segment);
    }
}

fn process_pending_segment(
    app: &tauri::AppHandle,
    repository: &MeetingRepository,
    pending: PendingMeetingSegment,
) {
    let path = match repository.audio_path(&pending.audio_relative_path) {
        Ok(path) => path,
        Err(_) => {
            let _ = repository.fail_segment(pending.id, "spool_failed");
            return;
        }
    };
    let samples = fs::read(&path)
        .map_err(|_| ())
        .and_then(|bytes| crate::transcriber::parse_wav_to_samples(&bytes).map_err(|_| ()));
    let result = samples.and_then(|samples| {
        let state = app.state::<State>();
        state
            .app_state
            .model_runtime
            .with_ready_backend(
                Some(app),
                &pending.model_name,
                PreparationReason::Meeting,
                |backend| {
                    backend.transcribe(&samples, &pending.language, None, pending.smart_punctuation)
                },
            )
            .map(|(text, _)| normalize_transcript(&text))
            .map_err(|_| ())
    });
    match result {
        Ok(text) => {
            if repository
                .finalize_segment(pending.id, &text, pending.retain_audio)
                .is_ok()
            {
                if !pending.retain_audio {
                    let _ = fs::remove_file(path);
                }
                *app.state::<State>()
                    .app_state
                    .last_transcription_at
                    .lock_or_recover() = Some(Instant::now());
                let segment = MeetingSegment {
                    id: pending.id,
                    session_id: pending.session_id,
                    speaker: pending.speaker,
                    sequence: pending.sequence,
                    start_ms: pending.start_ms,
                    end_ms: pending.end_ms,
                    status: MeetingSegmentStatus::Final,
                    text,
                    audio_available: pending.retain_audio,
                    error_code: None,
                };
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.emit("meeting-segment-finalized", segment);
                }
            }
        }
        Err(()) => {
            let _ = repository.fail_segment(pending.id, "transcription_failed");
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.emit(
                    "meeting-segment-failed",
                    serde_json::json!({
                        "sessionId": pending.session_id,
                        "segmentId": pending.id,
                        "errorCode": "transcription_failed"
                    }),
                );
            }
        }
    }
}

fn normalize_transcript(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
fn render_meeting_text(segments: &[MeetingSegment]) -> String {
    let mut output = String::new();
    for segment in segments
        .iter()
        .filter(|segment| segment.status == MeetingSegmentStatus::Final)
    {
        let total_seconds = segment.start_ms / 1_000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        let speaker = match segment.speaker {
            MeetingSpeaker::Me => "Me",
            MeetingSpeaker::Them => "Them",
        };
        output.push_str(&format!(
            "[{minutes:02}:{seconds:02}] {speaker}: {}\n",
            segment.text
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted(audio_flowing: bool) -> SystemAudioAccess {
        SystemAudioAccess {
            permission: SystemAudioPermissionState::Granted,
            capture_ready: true,
            audio_flowing,
            needs_relaunch: false,
        }
    }

    #[test]
    fn aec_protocol_accepts_bounded_recovery_and_return_to_active() {
        let mut tracker = AecProtocolTracker::new(EchoCancellationMode::Enabled);
        assert!(tracker.observe(EchoCancellationStatus::Active));
        assert!(tracker.observe(EchoCancellationStatus::Recovering {
            reason: EchoCancellationBypassReason::RenderDiscontinuity,
            attempt: 1,
            max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
        }));
        assert!(tracker.observe(EchoCancellationStatus::Recovering {
            reason: EchoCancellationBypassReason::RenderDiscontinuity,
            attempt: 2,
            max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
        }));
        assert!(tracker.observe(EchoCancellationStatus::Active));
        assert!(tracker.observe(EchoCancellationStatus::Recovering {
            reason: EchoCancellationBypassReason::ProcessingBacklog,
            attempt: 3,
            max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
        }));
        assert!(tracker.observe(EchoCancellationStatus::Bypassed {
            reason: EchoCancellationBypassReason::ProcessingBacklog,
        }));
    }

    #[test]
    fn aec_protocol_rejects_unbounded_or_out_of_order_recovery() {
        let mut skipped = AecProtocolTracker::new(EchoCancellationMode::Enabled);
        assert!(skipped.observe(EchoCancellationStatus::Active));
        assert!(!skipped.observe(EchoCancellationStatus::Recovering {
            reason: EchoCancellationBypassReason::RenderDiscontinuity,
            attempt: 2,
            max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
        }));

        let mut permanent = AecProtocolTracker::new(EchoCancellationMode::Enabled);
        assert!(permanent.observe(EchoCancellationStatus::Active));
        assert!(!permanent.observe(EchoCancellationStatus::Recovering {
            reason: EchoCancellationBypassReason::ProcessorFailed,
            attempt: 1,
            max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
        }));
    }

    #[test]
    fn granted_tap_without_audio_is_healthy_not_a_permission_failure() {
        // The #638 case: authorized, tap healthy, nothing playing.
        let access = resolve_access(true, granted(false));
        assert_eq!(access.permission, SystemAudioPermissionState::Granted);
        assert!(access.capture_ready);
        assert!(!access.audio_flowing);
        assert!(!access.needs_relaunch);
    }

    #[test]
    fn granted_tap_with_audio_reports_flow() {
        let access = resolve_access(true, granted(true));
        assert_eq!(access.permission, SystemAudioPermissionState::Granted);
        assert!(access.capture_ready);
        assert!(access.audio_flowing);
    }

    #[test]
    fn first_grant_is_granted_without_requiring_a_relaunch() {
        // Not authorized at preflight, prompt accepted, tap then succeeds.
        let access = resolve_access(false, granted(false));
        assert_eq!(access.permission, SystemAudioPermissionState::Granted);
        assert!(!access.needs_relaunch);
    }

    #[test]
    fn denial_is_not_capture_ready() {
        let probed = SystemAudioAccess {
            permission: SystemAudioPermissionState::Denied,
            capture_ready: false,
            audio_flowing: false,
            needs_relaunch: false,
        };
        let access = resolve_access(false, probed);
        assert_eq!(access.permission, SystemAudioPermissionState::Denied);
        assert!(!access.capture_ready);
        assert!(!access.needs_relaunch);
    }

    #[test]
    fn authorized_but_refused_tap_asks_for_a_relaunch() {
        let probed = SystemAudioAccess {
            permission: SystemAudioPermissionState::Denied,
            capture_ready: false,
            audio_flowing: false,
            needs_relaunch: false,
        };
        let access = resolve_access(true, probed);
        assert!(access.needs_relaunch);
    }

    #[test]
    fn unsupported_os_is_preserved_and_never_asks_for_a_relaunch() {
        let probed = SystemAudioAccess {
            permission: SystemAudioPermissionState::Unsupported,
            capture_ready: false,
            audio_flowing: false,
            needs_relaunch: false,
        };
        let access = resolve_access(true, probed);
        assert_eq!(access.permission, SystemAudioPermissionState::Unsupported);
        assert!(!access.needs_relaunch);
    }

    #[test]
    fn stalled_capture_has_its_own_message_and_code_per_channel() {
        let system =
            user_message_for_failure(FailureCode::CallbackStalled, Some(CaptureChannel::System));
        let microphone = user_message_for_failure(
            FailureCode::CallbackStalled,
            Some(CaptureChannel::Microphone),
        );
        let generic = user_message_for_failure(FailureCode::Internal, None);
        assert_ne!(system, generic);
        assert_ne!(microphone, generic);
        assert_ne!(system, microphone);
        assert_eq!(
            meeting_error_for_failure(FailureCode::CallbackStalled, Some(CaptureChannel::System))
                .code,
            "system_audio_callback_stalled"
        );
    }

    #[test]
    fn streaming_resampler_preserves_ratio_across_frame_boundaries() {
        let mut resampler = StreamingResampler::new();
        let first = resampler.push(48_000, &[1.0; 1_001]).unwrap();
        let second = resampler.push(48_000, &[1.0; 1_999]).unwrap();
        assert_eq!(first.len() + second.len(), 1_000);
        assert!(first.iter().chain(&second).all(|sample| *sample == 1.0));
    }

    #[test]
    fn streaming_resampler_interpolates_non_integer_ratios_across_frames() {
        let input = (0..10).map(|value| value as f32).collect::<Vec<_>>();
        let mut resampler = StreamingResampler::new();
        let mut output = resampler.push(44_100, &input[..3]).unwrap();
        output.extend(resampler.push(44_100, &input[3..]).unwrap());

        assert_eq!(output.len(), 4);
        for (actual, expected) in output.iter().zip([0.0, 2.75625, 5.5125, 8.26875]) {
            assert!((actual - expected).abs() < 0.0001);
        }
    }

    #[test]
    fn setup_watchdog_allows_tcc_but_bounds_core_audio_calls() {
        let stale = Instant::now() - SETUP_STEP_DEADLINE - Duration::from_millis(1);
        let mut watchdog = SetupWatchdog {
            pending: Some((
                CaptureChannel::System,
                CaptureSetupStep::SystemTapCreate,
                stale,
            )),
        };
        assert!(!watchdog.expired());

        watchdog.pending = Some((CaptureChannel::System, CaptureSetupStep::IoProcStart, stale));
        assert!(watchdog.expired());
        watchdog.observe(
            CaptureChannel::System,
            CaptureSetupStep::IoProcStart,
            SetupTransition::Completed,
        );
        assert!(!watchdog.expired());
    }

    #[test]
    fn meeting_input_resolution_accepts_one_complete_microphone_sequence() {
        let mut tracker = MeetingInputResolutionTracker::default();
        assert!(tracker.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Entered,
        ));
        assert!(tracker.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(true),
            2,
            false,
        ));
        assert!(tracker.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(tracker.permits_phase(
            CaptureChannel::Microphone,
            CapturePhase::AwaitingFirstCallback,
        ));
        assert!(tracker.permits_phase(CaptureChannel::Microphone, CapturePhase::Active));
        assert!(tracker.permits_pcm(CaptureChannel::Microphone));
    }

    #[test]
    fn meeting_input_resolution_rejects_missing_duplicate_and_wrong_evidence() {
        let mut tracker = MeetingInputResolutionTracker::default();
        assert!(tracker.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Entered,
        ));
        assert!(!tracker.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Completed,
        ));
        assert!(!tracker.permits_phase(CaptureChannel::Microphone, CapturePhase::Active));
        assert!(!tracker.permits_pcm(CaptureChannel::Microphone));
        assert!(!tracker.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Cpal,
            true,
            true,
            Some(true),
            1,
            false,
        ));
        assert!(tracker.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(true),
            1,
            false,
        ));
        assert!(!tracker.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(true),
            1,
            false,
        ));

        let mut invalid = MeetingInputResolutionTracker::default();
        assert!(invalid.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Entered,
        ));
        assert!(!invalid.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(true),
            (murmur_capture_helper_protocol::MAX_INPUT_DEVICE_COUNT + 1) as u16,
            false,
        ));
    }

    #[test]
    fn meeting_input_resolution_failure_codes_match_microphone_evidence() {
        let mut absent = MeetingInputResolutionTracker::default();
        assert!(absent.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Entered,
        ));
        assert!(absent.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(false),
            1,
            false,
        ));
        assert!(
            absent.permits_failure(FailureCode::NoInputDevice, Some(CaptureChannel::Microphone),)
        );
        assert!(!absent.permits_failure(
            FailureCode::EnumerationFailed,
            Some(CaptureChannel::Microphone),
        ));

        let mut present = MeetingInputResolutionTracker::default();
        assert!(present.observe_setup(
            CaptureChannel::Microphone,
            CaptureSetupStep::DeviceResolution,
            SetupTransition::Entered,
        ));
        assert!(present.observe_evidence(
            CaptureBackend::Auhal,
            CaptureBackend::Auhal,
            true,
            true,
            Some(true),
            1,
            false,
        ));
        assert!(
            !present.permits_failure(FailureCode::NoInputDevice, Some(CaptureChannel::Microphone),)
        );

        assert!(!present.permits_failure(FailureCode::NoInputDevice, None));
        assert!(
            !present.permits_failure(FailureCode::EnumerationFailed, Some(CaptureChannel::System),)
        );
        assert!(
            present.permits_failure(FailureCode::PermissionDenied, Some(CaptureChannel::System),)
        );
    }

    #[test]
    fn fixed_chunk_mode_is_bounded_for_accelerated_hour() {
        let mut chunker = VadChunker::new(0).unwrap();
        let frame = vec![0.25; 4_000];
        let mut chunk_count = 0;
        for _ in 0..(60 * 60 * 4) {
            let chunks = chunker.push(&frame).unwrap();
            chunk_count += chunks.len();
            assert!(chunks
                .iter()
                .all(|chunk| chunk.samples.len() <= MAX_CHUNK_SAMPLES + ANALYSIS_SAMPLES));
            assert!(chunker.active.len() <= MAX_CHUNK_SAMPLES + ANALYSIS_SAMPLES);
            assert!(chunker.pending_analysis.len() < ANALYSIS_SAMPLES);
        }
        chunk_count += chunker.flush().unwrap().len();
        assert!(chunk_count > 0);
    }

    #[test]
    fn transcript_renderer_keeps_attribution_and_relative_time() {
        let segments = vec![MeetingSegment {
            id: 1,
            session_id: "s".into(),
            speaker: MeetingSpeaker::Them,
            sequence: 0,
            start_ms: 62_000,
            end_ms: 63_000,
            status: MeetingSegmentStatus::Final,
            text: "hello".into(),
            audio_available: false,
            error_code: None,
        }];
        assert_eq!(render_meeting_text(&segments), "[01:02] Them: hello\n");
    }
}
