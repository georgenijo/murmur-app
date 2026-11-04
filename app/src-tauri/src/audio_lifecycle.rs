#[path = "audio_gesture.rs"]
pub(crate) mod gesture;
use crate::audio::{
    self, AudioCommand, AudioFailure, AudioFailureKind, AudioInitPhase, AudioWorkerEvent,
    AudioWorkerEventSender, AudioWorkerSpec,
};
use crate::state::WHISPER_SAMPLE_RATE;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const STILL_CONNECTING_AFTER: Duration = Duration::from_secs(5);
const HARD_INITIALIZATION_DEADLINE: Duration = Duration::from_secs(30);
const RECOVERY_GUIDANCE_AFTER: Duration = Duration::from_secs(10);
const SUPERVISOR_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AudioOwner {
    Dictation(u64),
    Transform(u64),
    Query(u64),
    Preview(u64),
    MicrophoneBenchmark(u64),
    #[cfg(feature = "internal-benchmark")]
    Corpus(u64),
}

impl AudioOwner {
    pub(crate) fn telemetry_id(self) -> u64 {
        match self {
            Self::Dictation(id)
            | Self::Transform(id)
            | Self::Query(id)
            | Self::Preview(id)
            | Self::MicrophoneBenchmark(id) => id,
            #[cfg(feature = "internal-benchmark")]
            Self::Corpus(id) => id,
        }
    }

    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Dictation(_) => "dictation",
            Self::Transform(_) => "transform",
            Self::Query(_) => "query",
            Self::Preview(_) => "preview",
            Self::MicrophoneBenchmark(_) => "microphone_benchmark",
            #[cfg(feature = "internal-benchmark")]
            Self::Corpus(_) => "corpus",
        }
    }

    /// Preview capture proves device readiness and drives a level meter, but
    /// never keeps microphone samples for transcription or later replay.
    pub(crate) fn retains_samples(self) -> bool {
        !matches!(self, Self::Preview(_) | Self::MicrophoneBenchmark(_))
    }

    pub(crate) fn preview_id(self) -> Option<u64> {
        match self {
            Self::Preview(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn dictation_id(self) -> Option<u64> {
        match self {
            Self::Dictation(id) => Some(id),
            _ => None,
        }
    }

    pub(crate) fn is_microphone_benchmark(self) -> bool {
        matches!(self, Self::MicrophoneBenchmark(_))
    }
}

/// Resolved at the accepted stop, never inferred from the recording's start mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationStopTrigger {
    HoldRelease,
    Toggle,
    Overlay,
    AutoStop,
    /// Button, chord stops, and legacy callers.
    #[default]
    Other,
}

impl DictationStopTrigger {
    pub(crate) fn resolve(self, release: Option<crate::hotkey_release::HotkeyRelease>) -> Self {
        if release.is_some() {
            Self::HoldRelease
        } else {
            self
        }
    }

    fn release_tail(self, owner: AudioOwner, standby_enabled: bool) -> Duration {
        if owner.dictation_id().is_none() || (self == Self::Toggle && !standby_enabled) {
            Duration::ZERO
        } else {
            audio::DICTATION_RELEASE_TAIL
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::HoldRelease => "hold_release",
            Self::Toggle => "toggle",
            Self::Overlay => "overlay",
            Self::AutoStop => "auto_stop",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCancelReason {
    User,
    DeviceChanged,
    RuntimeFailure,
    SystemSleep,
    SystemWake,
    HardDeadline,
}

impl AudioCancelReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::DeviceChanged => "device_changed",
            Self::RuntimeFailure => "runtime_failure",
            Self::SystemSleep => "system_sleep",
            Self::SystemWake => "system_wake",
            Self::HardDeadline => "hard_deadline",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioLifecycleEvent {
    Accepted,
    StartupDiagnostic(crate::audio::AudioStartupDiagnostic),
    Ready,
    StillConnecting,
    Recovering {
        reason: AudioCancelReason,
    },
    InitializationFailed {
        error: String,
        kind: AudioFailureKind,
        recovery_reason: Option<AudioCancelReason>,
    },
    RecoveryStalled,
    /// Capture cleanup finished independently of an already delivered result.
    CleanupFinished,
    Interrupted {
        reason: AudioCancelReason,
        delivered_samples: u64,
        duration_ms: u64,
    },
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioStartError {
    AlreadyStarting,
    AudioRecovering,
    AlreadyRecording,
    SpawnFailed(String),
    InitializationFailed(AudioFailure),
    Cancelled,
    SupervisorUnavailable,
}

impl fmt::Display for AudioStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyStarting => formatter.write_str("Audio is already starting"),
            Self::AudioRecovering => {
                formatter.write_str("Audio is still recovering from the previous attempt")
            }
            Self::AlreadyRecording => formatter.write_str("Audio is already recording"),
            Self::SpawnFailed(error) => formatter.write_str(error),
            Self::InitializationFailed(error) => fmt::Display::fmt(error, formatter),
            Self::Cancelled => formatter.write_str("Audio initialization was cancelled"),
            Self::SupervisorUnavailable => {
                formatter.write_str("Audio lifecycle supervisor is unavailable")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptPhase {
    Starting,
    Recording,
    Recovering,
    Stopping,
    Standby,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicPhase {
    Idle = 0,
    Starting = 1,
    Recording = 2,
    Recovering = 3,
    Stopping = 4,
    Standby = 5,
}

impl PublicPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Recording => "recording",
            Self::Recovering => "recovering",
            Self::Stopping => "stopping",
            Self::Standby => "standby",
        }
    }
}

struct PublicState {
    phase: AtomicU8,
    still_connecting: std::sync::atomic::AtomicBool,
    owner: Mutex<Option<AudioOwner>>,
}

impl Default for PublicState {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(PublicPhase::Idle as u8),
            still_connecting: std::sync::atomic::AtomicBool::new(false),
            owner: Mutex::new(None),
        }
    }
}

impl PublicState {
    fn set_phase(&self, phase: PublicPhase) {
        self.phase.store(phase as u8, Ordering::SeqCst);
    }

    fn is_active(&self) -> bool {
        self.phase.load(Ordering::SeqCst) != PublicPhase::Idle as u8
    }

    fn phase(&self) -> PublicPhase {
        match self.phase.load(Ordering::SeqCst) {
            value if value == PublicPhase::Idle as u8 => PublicPhase::Idle,
            value if value == PublicPhase::Starting as u8 => PublicPhase::Starting,
            value if value == PublicPhase::Recording as u8 => PublicPhase::Recording,
            value if value == PublicPhase::Recovering as u8 => PublicPhase::Recovering,
            value if value == PublicPhase::Stopping as u8 => PublicPhase::Stopping,
            value if value == PublicPhase::Standby as u8 => PublicPhase::Standby,
            _ => PublicPhase::Recovering,
        }
    }

    fn set_owner(&self, owner: AudioOwner) {
        *self
            .owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(owner);
    }

    fn clear_owner(&self, owner: AudioOwner) {
        let mut current = self
            .owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current == Some(owner) {
            *current = None;
        }
    }

    fn is_recording_for(&self, owner: AudioOwner) -> bool {
        self.phase.load(Ordering::SeqCst) == PublicPhase::Recording as u8
            && *self
                .owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                == Some(owner)
    }

    fn owns(&self, owner: AudioOwner) -> bool {
        *self
            .owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == Some(owner)
    }

    fn is_still_connecting_for(&self, owner: AudioOwner) -> bool {
        self.still_connecting.load(Ordering::SeqCst)
            && *self
                .owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                == Some(owner)
    }

    fn preview_owner(&self) -> Option<u64> {
        match *self
            .owner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            Some(AudioOwner::Preview(preview_id)) => Some(preview_id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct SupervisorConfig {
    still_connecting_after: Duration,
    hard_deadline: Duration,
    tcc_prompt_watchdog: Duration,
    recovery_guidance_after: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            still_connecting_after: STILL_CONNECTING_AFTER,
            hard_deadline: HARD_INITIALIZATION_DEADLINE,
            tcc_prompt_watchdog: audio::TCC_PROMPT_WATCHDOG,
            recovery_guidance_after: RECOVERY_GUIDANCE_AFTER,
        }
    }
}

trait WorkerFactory: Send + Sync + 'static {
    fn automatic_capture_authorized(&self) -> bool {
        crate::commands::permissions::check_microphone_permission_status() == "granted"
    }
    fn spawn(
        &self,
        spec: AudioWorkerSpec,
        event_sender: AudioWorkerEventSender,
    ) -> Result<JoinHandle<()>, String>;
}

struct ProductionWorkerFactory;

impl WorkerFactory for ProductionWorkerFactory {
    fn spawn(
        &self,
        spec: AudioWorkerSpec,
        event_sender: AudioWorkerEventSender,
    ) -> Result<JoinHandle<()>, String> {
        audio::spawn_capture_worker(spec, event_sender)
    }
}

trait LifecycleSink: Send + Sync + 'static {
    fn invalidate_input_health(&self, device_id: Option<&str>);
    fn notify(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        owner: AudioOwner,
        event: AudioLifecycleEvent,
    );
}

struct ProductionLifecycleSink;

impl LifecycleSink for ProductionLifecycleSink {
    fn invalidate_input_health(&self, device_id: Option<&str>) {
        crate::audio_inventory::invalidate_signal_evidence(device_id);
    }

    fn notify(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        owner: AudioOwner,
        event: AudioLifecycleEvent,
    ) {
        let Some(app_handle) = app_handle else {
            return;
        };
        match owner {
            AudioOwner::Dictation(recording_id) => {
                crate::commands::recording::handle_audio_lifecycle(
                    app_handle.clone(),
                    recording_id,
                    event,
                );
            }
            AudioOwner::Transform(transform_pass_id) => {
                crate::transform_flow::handle_audio_lifecycle(
                    app_handle.clone(),
                    transform_pass_id,
                    event,
                );
            }
            AudioOwner::Query(query_pass_id) => {
                crate::query_flow::handle_audio_lifecycle(app_handle.clone(), query_pass_id, event);
            }
            AudioOwner::Preview(preview_id) => {
                crate::commands::microphone_preview::handle_audio_lifecycle(
                    app_handle.clone(),
                    preview_id,
                    event,
                );
            }
            AudioOwner::MicrophoneBenchmark(cycle_id) => {
                crate::commands::microphone_startup_benchmark::handle_audio_lifecycle(
                    app_handle.clone(),
                    cycle_id,
                    event,
                );
            }
            #[cfg(feature = "internal-benchmark")]
            AudioOwner::Corpus(capture_id) => {
                crate::commands::corpus::handle_audio_lifecycle(
                    app_handle.clone(),
                    capture_id,
                    event,
                );
            }
        }
    }
}

struct StartRequest {
    gesture: gesture::StartGesture,
    caller_alive: Arc<AtomicBool>,
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    origin: String,
    wait_until_ready: bool,
    response: Sender<Result<(), AudioStartError>>,
    probe_permit: Option<Arc<crate::smart_auto_probe::ProbePermit>>,
}

// StartRequest is intentionally owned as one message so the supervisor can
// accept or reject an immutable capture request without shared mutable state.
#[allow(clippy::large_enum_variant)]
enum SupervisorMessage {
    Start(StartRequest),
    Stop {
        trigger: DictationStopTrigger,
        owner: Option<AudioOwner>,
        response: Sender<Result<Vec<f32>, String>>,
    },
    Cancel {
        owner: Option<AudioOwner>,
        reason: AudioCancelReason,
        starting_only: bool,
        response: Sender<Result<bool, String>>,
    },
    Peek {
        owner: AudioOwner,
        max_output_samples: Option<usize>,
        response: Sender<Option<Vec<f32>>>,
    },
    ConfigureStandby(StandbyPolicy),
    RetireStandby(Sender<()>),
    InputTopologyChanged,
    Worker(AudioWorkerEvent),
    #[cfg(test)]
    CheckDeadlines {
        elapsed: Duration,
        response: Sender<()>,
    },
    #[cfg(test)]
    Shutdown(Sender<()>),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct StandbyPolicy {
    seconds: u32,
    pre_roll_ms: u32,
}

struct Attempt {
    // Worker identity never changes; logical dictation owners do on resume.
    worker_owner: AudioOwner,
    input_topology_generation: u64,
    retire_responses: Vec<Sender<()>>,
    standby_policy: StandbyPolicy,
    standby_deadline: Option<Instant>,
    tail_deadline: Option<Instant>,
    // Logical recording generation whose own stop issued the worker Stop.
    // Standby retirement/recovery Stops never acquire this binding.
    recording_stop_owner: Option<AudioOwner>,
    probe_permit: Option<Arc<crate::smart_auto_probe::ProbePermit>>,
    probe_deadline: Option<Instant>,
    owner: AudioOwner,
    device_id: Option<String>,
    app_handle: Option<tauri::AppHandle>,
    origin: String,
    phase: AttemptPhase,
    accepted_at: Instant,
    tcc_pending_since: Option<Instant>,
    tcc_paused_total: Duration,
    ready_at: Option<Instant>,
    gesture: Option<gesture::AcceptedGesture>,
    stopping_started_at: Option<Instant>,
    still_connecting_emitted: bool,
    failure_reported: bool,
    stopping_guidance_emitted: bool,
    failure: Option<AudioFailure>,
    recovery_reason: Option<AudioCancelReason>,
    init_phase: AudioInitPhase,
    command_sender: Sender<AudioCommand>,
    thread_handle: Option<JoinHandle<()>>,
    shared: Arc<Mutex<audio::CaptureSamples>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    sample_rate: u32,
    start_response: Option<Sender<Result<(), AudioStartError>>>,
    stop_response: Option<Sender<Result<Vec<f32>, String>>>,
    sealed_delivery: bool,
}

impl Attempt {
    fn warm_epoch(&self) -> Option<u64> {
        (self.owner != self.worker_owner).then_some(self.input_topology_generation)
    }

    fn warm_topology_is_current(&self) -> bool {
        self.warm_epoch()
            .is_none_or(|epoch| epoch == INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire))
    }

    fn active_initialization_elapsed(&self, now: Instant) -> Duration {
        let current_pause = self
            .tcc_pending_since
            .map(|started| now.saturating_duration_since(started))
            .unwrap_or_default();
        now.saturating_duration_since(self.accepted_at)
            .saturating_sub(self.tcc_paused_total)
            .saturating_sub(current_pause)
    }
}

#[derive(Clone)]
struct AudioSupervisor {
    sender: Sender<SupervisorMessage>,
    public: Arc<PublicState>,
}

static INPUT_TOPOLOGY_GENERATION: AtomicU64 = AtomicU64::new(0);

static SUPERVISOR: OnceLock<AudioSupervisor> = OnceLock::new();
static HAL_BOUNDARY: OnceLock<Mutex<()>> = OnceLock::new();

fn hal_boundary() -> &'static Mutex<()> {
    HAL_BOUNDARY.get_or_init(|| Mutex::new(()))
}

fn supervisor() -> &'static AudioSupervisor {
    SUPERVISOR.get_or_init(|| {
        spawn_supervisor(
            Arc::new(ProductionWorkerFactory),
            Arc::new(ProductionLifecycleSink),
            SupervisorConfig::default(),
        )
    })
}

fn spawn_supervisor(
    factory: Arc<dyn WorkerFactory>,
    sink: Arc<dyn LifecycleSink>,
    config: SupervisorConfig,
) -> AudioSupervisor {
    let (sender, receiver) = mpsc::channel::<SupervisorMessage>();
    let public = Arc::new(PublicState::default());
    let worker_message_sender = sender.clone();
    let worker_event_sender = AudioWorkerEventSender::new(move |event| {
        worker_message_sender
            .send(SupervisorMessage::Worker(event))
            .map_err(|_| ())
    });

    let public_for_thread = Arc::clone(&public);
    std::thread::Builder::new()
        .name("murmur-audio-supervisor".to_string())
        .spawn(move || {
            run_supervisor(
                receiver,
                worker_event_sender,
                factory,
                sink,
                public_for_thread,
                config,
            );
        })
        .expect("audio supervisor thread must spawn");

    AudioSupervisor { sender, public }
}

fn run_supervisor(
    receiver: Receiver<SupervisorMessage>,
    worker_event_sender: AudioWorkerEventSender,
    factory: Arc<dyn WorkerFactory>,
    sink: Arc<dyn LifecycleSink>,
    public: Arc<PublicState>,
    config: SupervisorConfig,
) {
    let mut attempt: Option<Attempt> = None;
    let mut standby_policy = StandbyPolicy::default();
    let mut pending_start: Option<StartRequest> = None;
    loop {
        let timeout = deadline_wait(attempt.as_ref(), config);
        match receiver.recv_timeout(timeout) {
            Ok(message) => {
                #[cfg(test)]
                if let SupervisorMessage::Shutdown(response) = message {
                    let _ = response.send(());
                    return;
                }
                #[cfg(test)]
                if let SupervisorMessage::CheckDeadlines { elapsed, response } = message {
                    let now = attempt
                        .as_ref()
                        .map(|current| current.accepted_at + elapsed)
                        .unwrap_or_else(Instant::now);
                    handle_deadlines_at(&mut attempt, sink.as_ref(), &public, config, now);
                    let _ = response.send(());
                    continue;
                }
                if attempt.as_ref().is_some_and(|current| {
                    current.probe_permit.is_some()
                        || current.tail_deadline.is_some()
                        || current.phase == AttemptPhase::Standby
                }) {
                    handle_deadlines_at(
                        &mut attempt,
                        sink.as_ref(),
                        &public,
                        config,
                        Instant::now(),
                    );
                }
                let message = match message {
                    SupervisorMessage::InputTopologyChanged => {
                        if attempt.as_ref().is_some_and(|a| {
                            a.worker_owner != a.owner
                                && a.input_topology_generation
                                    != INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire)
                        }) {
                            // The notification may race a warm press. Revoke the
                            // resumed logical owner too, never an unrelated cold one.
                            begin_recovery(
                                &mut attempt,
                                AudioCancelReason::DeviceChanged,
                                sink.as_ref(),
                                &public,
                                None,
                            );
                        } else {
                            handle_deadlines_at(
                                &mut attempt,
                                sink.as_ref(),
                                &public,
                                config,
                                Instant::now(),
                            );
                        }
                        continue;
                    }
                    SupervisorMessage::Start(request)
                        if !request.caller_alive.load(Ordering::Acquire) =>
                    {
                        let _ = request.response.send(Err(AudioStartError::Cancelled));
                        continue;
                    }
                    SupervisorMessage::Cancel {
                        owner, response, ..
                    } if pending_start
                        .as_ref()
                        .is_some_and(|r| owner.is_none_or(|o| o == r.owner)) =>
                    {
                        let request = pending_start.take().unwrap();
                        let _ = request.response.send(Err(AudioStartError::Cancelled));
                        sink.notify(
                            request.app_handle.as_ref(),
                            request.owner,
                            AudioLifecycleEvent::Idle,
                        );
                        let _ = response.send(Ok(true));
                        continue;
                    }
                    SupervisorMessage::Stop {
                        owner, response, ..
                    } if pending_start
                        .as_ref()
                        .is_some_and(|r| owner.is_none_or(|o| o == r.owner)) =>
                    {
                        let request = pending_start.take().unwrap();
                        let _ = request.response.send(Err(AudioStartError::Cancelled));
                        sink.notify(
                            request.app_handle.as_ref(),
                            request.owner,
                            AudioLifecycleEvent::Idle,
                        );
                        let _ = response.send(Ok(Vec::new()));
                        continue;
                    }
                    SupervisorMessage::ConfigureStandby(policy) => {
                        tracing::info!(target: "audio",
                            event_code = "audio.standby_policy_applied",
                            enabled = policy.seconds > 0,
                            standby_seconds = policy.seconds,
                            pre_roll_ms = policy.pre_roll_ms,
                            changed = standby_policy != policy,
                            "microphone standby policy applied by audio supervisor"
                        );
                        if standby_policy == policy {
                            continue;
                        }
                        standby_policy = policy;
                        if let Some(current) = attempt.as_mut() {
                            current.standby_policy = policy;
                            if current.phase == AttemptPhase::Standby {
                                begin_recovery(
                                    &mut attempt,
                                    AudioCancelReason::User,
                                    sink.as_ref(),
                                    &public,
                                    None,
                                );
                            }
                        }
                        continue;
                    }
                    SupervisorMessage::RetireStandby(response) => {
                        if let Some(current) = attempt.as_mut().filter(|a| {
                            a.phase == AttemptPhase::Standby || a.standby_deadline.is_some()
                        }) {
                            current.retire_responses.push(response);
                            begin_recovery(
                                &mut attempt,
                                AudioCancelReason::User,
                                sink.as_ref(),
                                &public,
                                None,
                            );
                        } else {
                            let _ = response.send(());
                        }
                        continue;
                    }
                    SupervisorMessage::Start(request)
                        if attempt.as_ref().is_some_and(|a| {
                            a.phase == AttemptPhase::Standby
                                || (a.phase == AttemptPhase::Recovering
                                    && a.standby_deadline.is_some())
                        }) && pending_start.is_none() =>
                    {
                        let current = attempt.as_ref().unwrap();
                        if current.phase == AttemptPhase::Standby
                            && request.owner.dictation_id().is_some()
                            && current.device_id == request.device_id
                            && current.standby_deadline.is_some_and(|d| Instant::now() < d)
                            && current.input_topology_generation
                                == INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire)
                            && standby_policy.seconds > 0
                            && !current
                                .shared
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .is_empty()
                        {
                            let current = attempt.as_mut().unwrap();
                            current
                                .shared
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .resume();
                            public.clear_owner(current.owner);
                            current.owner = request.owner;
                            current.recording_stop_owner = None;
                            current.app_handle = request.app_handle;
                            current.origin = request.origin;
                            current.accepted_at = Instant::now();
                            current.gesture = request.gesture.timing;
                            sink.notify(
                                current.app_handle.as_ref(),
                                request.owner,
                                AudioLifecycleEvent::StartupDiagnostic(
                                    audio::AudioStartupDiagnostic::GestureAccepted {
                                        gesture_expected: request.gesture.expected,
                                        double_tap_expected: request.gesture.expected
                                            && current.origin == "toggle",
                                        press_to_accepted_ms: current.gesture.map(|g| {
                                            gesture::millis(
                                                g.accepted
                                                    .saturating_duration_since(g.candidate.pressed),
                                            )
                                        }),
                                        double_tap_gap_ms: current.gesture.and_then(|g| g.gap_ms),
                                    },
                                ),
                            );
                            current.ready_at = Some(Instant::now());
                            current.standby_deadline = None;
                            current.phase = AttemptPhase::Recording;
                            current.active.store(true, Ordering::SeqCst);
                            public.set_owner(current.owner);
                            public.set_phase(PublicPhase::Recording);
                            publish_gesture_ready(current, sink.as_ref(), Instant::now());
                            sink.notify(
                                current.app_handle.as_ref(),
                                current.owner,
                                AudioLifecycleEvent::Accepted,
                            );
                            sink.notify(
                                current.app_handle.as_ref(),
                                current.owner,
                                AudioLifecycleEvent::Ready,
                            );
                            tracing::info!(target: "audio", event_code = "audio.capture_ready", recording_id = current.owner.telemetry_id(), warm = true, startup_ms = current.accepted_at.elapsed().as_millis() as u64, "standby PCM available at press");
                            let _ = request.response.send(Ok(()));
                        } else {
                            begin_recovery(
                                &mut attempt,
                                AudioCancelReason::User,
                                sink.as_ref(),
                                &public,
                                None,
                            );
                            pending_start = Some(request);
                        }
                        continue;
                    }
                    other => other,
                };
                handle_message(
                    message,
                    &mut attempt,
                    &worker_event_sender,
                    factory.as_ref(),
                    sink.as_ref(),
                    &public,
                );
            }
            Err(RecvTimeoutError::Timeout) => {
                handle_deadlines_at(&mut attempt, sink.as_ref(), &public, config, Instant::now());
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if attempt.is_none() {
            if let Some(request) = pending_start.take() {
                if crate::keyboard::is_app_disabled() {
                    let _ = request.response.send(Err(AudioStartError::Cancelled));
                    sink.notify(
                        request.app_handle.as_ref(),
                        request.owner,
                        AudioLifecycleEvent::Idle,
                    );
                    continue;
                }
                handle_start(
                    request,
                    &mut attempt,
                    &worker_event_sender,
                    factory.as_ref(),
                    sink.as_ref(),
                    &public,
                );
            }
        }
        if let Some(current) = attempt.as_mut() {
            current.standby_policy = standby_policy;
        }
    }
}

fn deadline_wait(attempt: Option<&Attempt>, config: SupervisorConfig) -> Duration {
    let Some(attempt) = attempt else {
        return Duration::from_secs(60);
    };
    let now = Instant::now();
    let deadline = match attempt.phase {
        AttemptPhase::Standby => attempt.standby_deadline.unwrap_or(now),
        AttemptPhase::Stopping if attempt.tail_deadline.is_some() => {
            attempt.tail_deadline.unwrap_or(now)
        }
        AttemptPhase::Starting if attempt.tcc_pending_since.is_some() => {
            attempt.tcc_pending_since.unwrap_or(now) + config.tcc_prompt_watchdog
        }
        AttemptPhase::Starting if !attempt.still_connecting_emitted => {
            attempt.accepted_at + attempt.tcc_paused_total + config.still_connecting_after
        }
        AttemptPhase::Starting => {
            attempt.accepted_at + attempt.tcc_paused_total + config.hard_deadline
        }
        AttemptPhase::Stopping if !attempt.stopping_guidance_emitted => {
            attempt.stopping_started_at.unwrap_or(now) + config.recovery_guidance_after
        }
        AttemptPhase::Recovering if !attempt.stopping_guidance_emitted => {
            attempt.stopping_started_at.unwrap_or(now) + config.recovery_guidance_after
        }
        _ => now + Duration::from_secs(60),
    };
    let wait = deadline.saturating_duration_since(now);
    if attempt.probe_permit.is_some()
        && matches!(
            attempt.phase,
            AttemptPhase::Starting | AttemptPhase::Recording
        )
    {
        wait.min(Duration::from_millis(10))
    } else {
        wait
    }
}

fn handle_message(
    message: SupervisorMessage,
    attempt: &mut Option<Attempt>,
    worker_event_sender: &AudioWorkerEventSender,
    factory: &dyn WorkerFactory,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    match message {
        SupervisorMessage::Start(request) => {
            handle_start(request, attempt, worker_event_sender, factory, sink, public);
        }
        SupervisorMessage::Stop {
            trigger,
            owner,
            response,
        } => {
            let Some(current) = attempt.as_ref() else {
                let _ = response.send(Ok(Vec::new()));
                return;
            };
            if owner.is_some_and(|owner| owner != current.owner) {
                let _ = response.send(Err("Audio owner changed before stop".to_string()));
                return;
            }
            if current.phase == AttemptPhase::Starting {
                begin_recovery(attempt, AudioCancelReason::User, sink, public, None);
                let _ = response.send(Ok(Vec::new()));
                return;
            }
            let current = attempt.as_mut().expect("attempt was checked above");
            match current.phase {
                AttemptPhase::Recording => {
                    if let Some(permit) = &current.probe_permit {
                        permit.revoke();
                    }
                    current.active.store(false, Ordering::SeqCst);
                    let tail =
                        trigger.release_tail(current.owner, current.standby_policy.seconds > 0);
                    tracing::info!(target: "audio",
                        event_code = "audio.stop_tail_applied",
                        recording_id = current.owner.telemetry_id(),
                        owner_kind = current.owner.kind(),
                        stop_trigger = trigger.as_str(),
                        standby_enabled = current.standby_policy.seconds > 0,
                        release_tail_ms = tail.as_millis() as u64,
                        "stop tail resolved from accepted trigger"
                    );
                    // The supervisor owns the tail for both cold and standby
                    // stops. The worker only drains after Stop; never add it twice.
                    if current.owner.dictation_id().is_some() {
                        current.tail_deadline = Some(Instant::now() + tail);
                    } else {
                        let _ = current.command_sender.send(AudioCommand::Stop);
                    }
                    current.phase = AttemptPhase::Stopping;
                    current.stopping_started_at = Some(Instant::now());
                    current.stop_response = Some(response);
                    public.set_phase(PublicPhase::Stopping);
                }
                AttemptPhase::Standby => {
                    let _ = response.send(Ok(Vec::new()));
                }
                AttemptPhase::Starting => unreachable!("starting was handled above"),
                AttemptPhase::Recovering => {
                    let _ = response.send(Err(
                        "Audio is still recovering from the previous attempt".to_string(),
                    ));
                }
                AttemptPhase::Stopping => {
                    let _ = response.send(Err("Audio is already stopping".to_string()));
                }
            }
        }
        SupervisorMessage::Cancel {
            owner,
            reason,
            starting_only,
            response,
        } => {
            let should_abandon = if let Some(current) = attempt.as_ref() {
                let owner_matches = owner.is_none_or(|owner| owner == current.owner);
                owner_matches
                    && matches!(
                        current.phase,
                        AttemptPhase::Starting | AttemptPhase::Recording | AttemptPhase::Standby
                    )
                    && (!starting_only
                        || matches!(
                            current.phase,
                            AttemptPhase::Starting | AttemptPhase::Standby
                        ))
            } else {
                false
            };
            if starting_only && attempt.as_ref().is_some_and(|a| a.tail_deadline.is_some()) {
                let current = attempt.as_mut().unwrap();
                current.tail_deadline = None;
                let _ = current.command_sender.send(AudioCommand::Stop);
            }
            if should_abandon {
                begin_recovery(attempt, reason, sink, public, None);
            }
            let cancelled = should_abandon;
            let _ = response.send(Ok(cancelled));
        }
        SupervisorMessage::Peek {
            owner,
            max_output_samples,
            response,
        } => {
            let samples = attempt.as_ref().and_then(|current| {
                if current.owner != owner || current.phase != AttemptPhase::Recording {
                    return None;
                }
                let shared = current
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let snapshot = snapshot_for_asr(&shared, current.sample_rate, max_output_samples);
                current.warm_topology_is_current().then_some(snapshot)
            });
            let _ = response.send(samples);
        }
        SupervisorMessage::ConfigureStandby(_)
        | SupervisorMessage::RetireStandby(_)
        | SupervisorMessage::InputTopologyChanged => {
            unreachable!()
        }
        SupervisorMessage::Worker(event) => {
            handle_worker_event(event, attempt, sink, public);
        }
        #[cfg(test)]
        SupervisorMessage::CheckDeadlines { .. } => unreachable!(),
        #[cfg(test)]
        SupervisorMessage::Shutdown(_) => unreachable!(),
    }
}

fn handle_start(
    request: StartRequest,
    attempt: &mut Option<Attempt>,
    worker_event_sender: &AudioWorkerEventSender,
    factory: &dyn WorkerFactory,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    if !request.caller_alive.load(Ordering::Acquire) {
        let _ = request.response.send(Err(AudioStartError::Cancelled));
        return;
    }
    if request
        .probe_permit
        .as_ref()
        .is_some_and(|permit| !permit.is_valid())
    {
        let _ = request.response.send(Err(AudioStartError::Cancelled));
        return;
    }
    if request.probe_permit.is_some() && !factory.automatic_capture_authorized() {
        let _ = request
            .response
            .send(Err(AudioStartError::InitializationFailed(
                AudioFailure::new(
                    AudioFailureKind::PermissionDenied,
                    AudioInitPhase::DeviceEnumeration,
                ),
            )));
        return;
    }
    if let Some(current) = attempt.as_ref() {
        let error = match current.phase {
            AttemptPhase::Starting => AudioStartError::AlreadyStarting,
            AttemptPhase::Recording | AttemptPhase::Standby => AudioStartError::AlreadyRecording,
            AttemptPhase::Recovering | AttemptPhase::Stopping => AudioStartError::AudioRecovering,
        };
        let _ = request.response.send(Err(error));
        return;
    }

    let input_topology_generation = INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire);
    let shared = Arc::new(Mutex::new(audio::CaptureSamples::default()));
    // Fail closed until readiness is accepted for this exact owner.
    let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (command_sender, command_receiver) = mpsc::channel();
    let spec = AudioWorkerSpec {
        gesture: request.gesture.timing,
        owner: request.owner,
        command_receiver,
        shared: Arc::clone(&shared),
        active: Arc::clone(&active),
        app_handle: request.app_handle.clone(),
        device_id: request.device_id.clone(),
        probe_permit: request.probe_permit.clone(),
    };
    // Serialize the transition into capture ownership with any idle-only
    // diagnostic enumeration. Once Starting is published below, the
    // diagnostic side will refuse the boundary until this attempt is joined.
    let _hal_boundary = hal_boundary()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if request
        .probe_permit
        .as_ref()
        .is_some_and(|permit| !permit.is_valid())
    {
        let _ = request.response.send(Err(AudioStartError::Cancelled));
        return;
    }
    if request.probe_permit.is_some() && !factory.automatic_capture_authorized() {
        let _ = request
            .response
            .send(Err(AudioStartError::InitializationFailed(
                AudioFailure::new(
                    AudioFailureKind::PermissionDenied,
                    AudioInitPhase::DeviceEnumeration,
                ),
            )));
        return;
    }
    if !request.caller_alive.load(Ordering::Acquire) {
        let _ = request.response.send(Err(AudioStartError::Cancelled));
        return;
    }
    // Publish ownership before the worker can spawn its capture helper. Any
    // diagnostic/state-only enumeration racing this start must see Starting
    // and defer until the supervisor returns to Idle.
    public.still_connecting.store(false, Ordering::SeqCst);
    public.set_owner(request.owner);
    public.set_phase(PublicPhase::Starting);
    let thread_handle = match factory.spawn(spec, worker_event_sender.clone()) {
        Ok(handle) => handle,
        Err(error) => {
            sink.invalidate_input_health(request.device_id.as_deref());
            public.clear_owner(request.owner);
            public.set_phase(PublicPhase::Idle);
            crate::audio_inventory::lifecycle_became_idle();
            let _ = request
                .response
                .send(Err(AudioStartError::SpawnFailed(error)));
            return;
        }
    };

    let owner = request.owner;
    let app_handle = request.app_handle;
    let wait_until_ready = request.wait_until_ready;
    let origin = request.origin;
    *attempt = Some(Attempt {
        worker_owner: owner,
        input_topology_generation,
        retire_responses: Vec::new(),
        standby_policy: StandbyPolicy::default(),
        standby_deadline: None,
        tail_deadline: None,
        recording_stop_owner: None,
        probe_deadline: request
            .probe_permit
            .as_ref()
            .map(|permit| Instant::now() + permit.remaining()),
        probe_permit: request.probe_permit,
        owner,
        device_id: request.device_id,
        app_handle,
        origin,
        phase: AttemptPhase::Starting,
        accepted_at: Instant::now(),
        tcc_pending_since: None,
        tcc_paused_total: Duration::ZERO,
        ready_at: None,
        gesture: request.gesture.timing,
        stopping_started_at: None,
        still_connecting_emitted: false,
        failure_reported: false,
        stopping_guidance_emitted: false,
        failure: None,
        recovery_reason: None,
        init_phase: AudioInitPhase::DeviceEnumeration,
        command_sender,
        thread_handle: Some(thread_handle),
        shared,
        active,
        sample_rate: WHISPER_SAMPLE_RATE,
        start_response: Some(request.response),
        stop_response: None,
        sealed_delivery: false,
    });
    let current = attempt.as_ref().expect("accepted attempt was installed");
    if owner.dictation_id().is_some() {
        sink.notify(
            current.app_handle.as_ref(),
            owner,
            AudioLifecycleEvent::StartupDiagnostic(
                audio::AudioStartupDiagnostic::GestureAccepted {
                    gesture_expected: request.gesture.expected,
                    double_tap_expected: request.gesture.expected && current.origin == "toggle",
                    press_to_accepted_ms: current.gesture.map(|g| {
                        gesture::millis(g.accepted.saturating_duration_since(g.candidate.pressed))
                    }),
                    double_tap_gap_ms: current.gesture.and_then(|g| g.gap_ms),
                },
            ),
        );
    }

    if let Some(recording_id) = owner.dictation_id() {
        tracing::info!(
            target: "audio",
            event_code = "audio.capture_started",
            recording_id,
            owner = owner.telemetry_id(),
            owner_kind = owner.kind(),
            origin = current.origin.as_str(),
            "audio initialization accepted"
        );
    } else {
        tracing::info!(
            target: "audio",
            event_code = "audio.capture_started",
            owner = owner.telemetry_id(),
            owner_kind = owner.kind(),
            origin = current.origin.as_str(),
            "audio initialization accepted"
        );
    }
    sink.notify(
        current.app_handle.as_ref(),
        owner,
        AudioLifecycleEvent::Accepted,
    );
    if !wait_until_ready {
        if let Some(response) = attempt
            .as_mut()
            .and_then(|current| current.start_response.take())
        {
            let _ = response.send(Ok(()));
        }
    }
}

fn publish_gesture_ready(current: &Attempt, sink: &dyn LifecycleSink, ready: Instant) {
    if let Some(g) = current.gesture {
        sink.notify(
            current.app_handle.as_ref(),
            current.owner,
            AudioLifecycleEvent::StartupDiagnostic(audio::AudioStartupDiagnostic::GestureReady {
                press_to_ready_ms: gesture::millis(
                    ready.saturating_duration_since(g.candidate.pressed),
                ),
                accepted_to_ready_ms: gesture::millis(ready.saturating_duration_since(g.accepted)),
            }),
        );
    }
}

fn handle_worker_event(
    event: AudioWorkerEvent,
    attempt: &mut Option<Attempt>,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    let dequeued_at = Instant::now();
    let owner = match &event {
        AudioWorkerEvent::PhaseEntered { owner, .. }
        | AudioWorkerEvent::PhaseExited { owner, .. }
        | AudioWorkerEvent::PermissionPromptPending { owner }
        | AudioWorkerEvent::PermissionPromptResolved { owner }
        | AudioWorkerEvent::TerminationUnconfirmed { owner, .. }
        | AudioWorkerEvent::StartupDiagnostic { owner, .. }
        | AudioWorkerEvent::FirstBuffer { owner, .. }
        | AudioWorkerEvent::InitFailed { owner, .. }
        | AudioWorkerEvent::RuntimeFailed { owner, .. }
        | AudioWorkerEvent::RecordingSealed { owner }
        | AudioWorkerEvent::StreamStopped { owner }
        | AudioWorkerEvent::ThreadExited { owner } => *owner,
    };
    let Some(current) = attempt.as_mut() else {
        tracing::warn!(
            target: "audio",
            owner = owner.telemetry_id(),
            "stale audio worker event after owner was cleared"
        );
        return;
    };
    if current.worker_owner != owner {
        if owner.dictation_id().is_some() {
            sink.notify(
                current.app_handle.as_ref(),
                owner,
                AudioLifecycleEvent::StartupDiagnostic(
                    crate::audio::AudioStartupDiagnostic::WorkerInvariantViolation,
                ),
            );
        }
        tracing::warn!(
            target: "audio",
            owner = owner.telemetry_id(),
            current_owner = current.owner.telemetry_id(),
            "stale audio worker event ignored"
        );
        return;
    }

    let owner = current.owner;
    match event {
        AudioWorkerEvent::PermissionPromptPending { .. } => {
            if current.probe_permit.is_some() {
                begin_recovery(
                    attempt,
                    AudioCancelReason::HardDeadline,
                    sink,
                    public,
                    Some(AudioFailure::new(
                        AudioFailureKind::PermissionDenied,
                        AudioInitPhase::StreamBuild,
                    )),
                );
                return;
            }
            if current.phase == AttemptPhase::Starting && current.tcc_pending_since.is_none() {
                current.tcc_pending_since = Some(Instant::now());
                if let Some(recording_id) = owner.dictation_id() {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.permission_prompt_changed",
                        recording_id,
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        state = "pending",
                        "microphone permission prompt pending; initialization deadlines suspended"
                    );
                } else {
                    tracing::info!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        "microphone permission prompt pending; initialization deadlines suspended"
                    );
                }
            }
        }
        AudioWorkerEvent::PermissionPromptResolved { .. } => {
            if let Some(started) = current.tcc_pending_since.take() {
                let paused = started.elapsed();
                current.tcc_paused_total += paused;
                if let Some(recording_id) = owner.dictation_id() {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.permission_prompt_changed",
                        recording_id,
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        state = "resolved",
                        prompt_pending_ms = paused.as_millis() as u64,
                        "microphone permission prompt resolved; initialization deadlines resumed"
                    );
                } else {
                    tracing::info!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        prompt_pending_ms = paused.as_millis() as u64,
                        "microphone permission prompt resolved; initialization deadlines resumed"
                    );
                }
            }
        }
        AudioWorkerEvent::TerminationUnconfirmed { failure, .. } => {
            // Delivery may already be complete. Preserve teardown evidence for
            // this exact owner independently of presentation/recovery gates.
            if current.standby_deadline.is_none() {
                sink.notify(
                    current.app_handle.as_ref(),
                    owner,
                    AudioLifecycleEvent::StartupDiagnostic(
                        audio::AudioStartupDiagnostic::WorkerInvariantViolation,
                    ),
                );
            }
            if current.phase == AttemptPhase::Stopping {
                // A sealed result is final, but failed teardown is a separate
                // capture failure. Keep the stop response/pipeline intact.
                current.failure = Some(failure);
                report_cleanup_stalled(current, sink, public);
                return;
            }
            if let Some(response) = current.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(failure.clone())));
            }
            begin_recovery(
                attempt,
                AudioCancelReason::RuntimeFailure,
                sink,
                public,
                Some(failure),
            );
        }
        AudioWorkerEvent::StartupDiagnostic { diagnostic, .. } => {
            if matches!(
                diagnostic,
                audio::AudioStartupDiagnostic::WorkerStopped { .. }
            ) && current.recording_stop_owner != Some(current.owner)
            {
                // Worker identity alone cannot correlate a stop: standby can
                // retire long after its last dictation has been delivered.
                return;
            }
            sink.notify(
                current.app_handle.as_ref(),
                current.owner,
                AudioLifecycleEvent::StartupDiagnostic(diagnostic),
            );
        }
        AudioWorkerEvent::PhaseEntered { phase, .. } => {
            current.init_phase = phase;
            tracing::info!(
                target: "audio",
                owner = owner.telemetry_id(),
                owner_kind = owner.kind(),
                phase = phase.as_str(),
                "audio initialization phase entered"
            );
        }
        AudioWorkerEvent::PhaseExited {
            phase, elapsed_ms, ..
        } => {
            tracing::info!(
                target: "audio",
                owner = owner.telemetry_id(),
                owner_kind = owner.kind(),
                phase = phase.as_str(),
                elapsed_ms,
                "audio initialization phase exited"
            );
        }
        AudioWorkerEvent::FirstBuffer {
            sample_rate,
            sent_at,
            ..
        } => match current.phase {
            AttemptPhase::Starting => {
                if current.owner.retains_samples()
                    && current
                        .shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .is_empty()
                {
                    tracing::error!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        error_kind = AudioFailureKind::InvalidInput.as_str(),
                        phase = AudioInitPhase::FirstBufferWait.as_str(),
                        "empty first-buffer event ignored"
                    );
                    return;
                }
                current.sample_rate = sample_rate;
                let ready_at = Instant::now();
                current.ready_at = Some(ready_at);
                current.active.store(true, Ordering::SeqCst);
                public.still_connecting.store(false, Ordering::SeqCst);
                current.phase = AttemptPhase::Recording;
                public.set_phase(PublicPhase::Recording);
                sink.notify(
                    current.app_handle.as_ref(),
                    current.owner,
                    AudioLifecycleEvent::Ready,
                );
                publish_gesture_ready(current, sink, ready_at);
                if owner.is_microphone_benchmark() || owner.dictation_id().is_some() {
                    sink.notify(
                        current.app_handle.as_ref(),
                        current.owner,
                        AudioLifecycleEvent::StartupDiagnostic(
                            crate::audio::AudioStartupDiagnostic::CycleReady {
                                cycle_start_to_first_pcm_ms: ready_at
                                    .saturating_duration_since(current.accepted_at)
                                    .as_millis()
                                    as u64,
                            },
                        ),
                    );
                }
                if let Some(recording_id) = owner.dictation_id() {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.capture_ready",
                        recording_id,
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        first_buffer_queue_ms = dequeued_at.saturating_duration_since(sent_at).as_millis() as u64,
                        startup_ms = ready_at.saturating_duration_since(current.accepted_at).as_millis() as u64,
                        origin = current.origin.as_str(),
                        "audio readiness accepted"
                    );
                } else {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.capture_ready",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        first_buffer_queue_ms = dequeued_at.saturating_duration_since(sent_at).as_millis() as u64,
                        startup_ms = ready_at.saturating_duration_since(current.accepted_at).as_millis() as u64,
                        origin = current.origin.as_str(),
                        "audio readiness accepted"
                    );
                }
                if let Some(response) = current.start_response.take() {
                    let _ = response.send(Ok(()));
                }
            }
            AttemptPhase::Recording
            | AttemptPhase::Recovering
            | AttemptPhase::Stopping
            | AttemptPhase::Standby => {
                tracing::warn!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    "duplicate audio readiness ignored"
                );
            }
        },
        AudioWorkerEvent::InitFailed { failure, .. } => {
            sink.invalidate_input_health(current.device_id.as_deref());
            current.failure = Some(failure.clone());
            if let Some(response) = current.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(failure)));
            }
        }
        AudioWorkerEvent::RuntimeFailed { failure, .. } => {
            if matches!(
                current.phase,
                AttemptPhase::Starting | AttemptPhase::Recording | AttemptPhase::Standby
            ) {
                if current.phase == AttemptPhase::Standby {
                    begin_recovery(attempt, AudioCancelReason::User, sink, public, None);
                    return;
                }
                current.failure = Some(failure.clone());
                // Dictation reports a retained prefix through Interrupted at
                // worker exit. Transform audio has no partial-transcript path,
                // so preserve its typed, content-free failure before recovery.
                let report_failure = match current.owner {
                    AudioOwner::Transform(_)
                    | AudioOwner::Query(_)
                    | AudioOwner::Preview(_)
                    | AudioOwner::MicrophoneBenchmark(_) => Some(failure),
                    #[cfg(feature = "internal-benchmark")]
                    AudioOwner::Corpus(_) => Some(failure),
                    AudioOwner::Dictation(_) => None,
                };
                begin_recovery(
                    attempt,
                    AudioCancelReason::RuntimeFailure,
                    sink,
                    public,
                    report_failure,
                );
            }
        }
        AudioWorkerEvent::RecordingSealed { .. } => {
            // Exact worker identity was checked above. Keep Stopping and its
            // public owner even when the final samples leave this supervisor.
            if current.phase == AttemptPhase::Stopping
                && current.owner.dictation_id().is_some()
                && current.tail_deadline.is_none()
                && current.stop_response.is_some()
            {
                let result = take_samples(current);
                let samples_ready = result.is_ok();
                current.sealed_delivery = current
                    .stop_response
                    .take()
                    .is_some_and(|response| response.send(result).is_ok());
                tracing::info!(target: "audio",
                    event_code = "audio.sealed_recording_delivered",
                    recording_id = current.owner.telemetry_id(),
                    delivered = current.sealed_delivery && samples_ready,
                    response_delivered = current.sealed_delivery,
                    exit_confirmed = false,
                    "sealed recording handed off; microphone ownership retained"
                );
            }
        }
        AudioWorkerEvent::StreamStopped { .. } => {
            tracing::info!(
                target: "audio",
                owner = owner.telemetry_id(),
                owner_kind = owner.kind(),
                "audio stream stop acknowledged"
            );
        }
        AudioWorkerEvent::ThreadExited { .. } => {
            finish_attempt(attempt, sink, public);
        }
    }
}

fn finish_attempt(attempt: &mut Option<Attempt>, sink: &dyn LifecycleSink, public: &PublicState) {
    let Some(mut finished) = attempt.take() else {
        return;
    };
    if let Some(handle) = finished.thread_handle.take() {
        if handle.join().is_err() {
            tracing::error!(
                target: "audio",
                owner = finished.owner.telemetry_id(),
                "audio thread join observed a panic"
            );
        }
    }
    tracing::info!(
        target: "audio",
        owner = finished.owner.telemetry_id(),
        owner_kind = finished.owner.kind(),
        "audio thread exited and joined"
    );

    let idle_after_clear = matches!(
        finished.owner,
        AudioOwner::Preview(_) | AudioOwner::MicrophoneBenchmark(_)
    );
    match finished.phase {
        AttemptPhase::Standby => {}
        AttemptPhase::Stopping => {
            let response_delivered = if finished.sealed_delivery {
                true
            } else if let Some(response) = finished.stop_response.take() {
                response.send(take_samples(&mut finished)).is_ok()
            } else {
                false
            };
            if !response_delivered && !idle_after_clear {
                sink.notify(
                    finished.app_handle.as_ref(),
                    finished.owner,
                    AudioLifecycleEvent::Idle,
                );
            }
        }
        AttemptPhase::Starting => {
            let failure = finished.failure.take().unwrap_or_else(|| {
                AudioFailure::new(AudioFailureKind::BackendError, finished.init_phase)
            });
            report_failure_once(&mut finished, sink, failure.clone());
            if let Some(response) = finished.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(failure)));
            }
            if !idle_after_clear {
                sink.notify(
                    finished.app_handle.as_ref(),
                    finished.owner,
                    AudioLifecycleEvent::Idle,
                );
            }
        }
        AttemptPhase::Recording => {
            report_failure_once(
                &mut finished,
                sink,
                AudioFailure::new(AudioFailureKind::BackendError, AudioInitPhase::Runtime),
            );
            if !idle_after_clear {
                sink.notify(
                    finished.app_handle.as_ref(),
                    finished.owner,
                    AudioLifecycleEvent::Idle,
                );
            }
        }
        AttemptPhase::Recovering => {
            if finished.recovery_reason == Some(AudioCancelReason::RuntimeFailure)
                && finished.standby_deadline.is_none()
                && finished.warm_topology_is_current()
                && matches!(finished.owner, AudioOwner::Dictation(_))
            {
                let reason = finished
                    .failure
                    .as_ref()
                    .map(|failure| failure.kind.as_str())
                    .unwrap_or("runtime_failure")
                    .to_string();
                if let Ok(samples) = take_samples(&mut finished) {
                    let delivered_samples = samples.len() as u64;
                    let duration_ms = delivered_samples / 16;
                    interrupted_captures()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(
                            finished.owner,
                            InterruptedCapture {
                                samples,
                                reason,
                                duration_ms,
                            },
                        );
                    sink.notify(
                        finished.app_handle.as_ref(),
                        finished.owner,
                        AudioLifecycleEvent::Interrupted {
                            reason: AudioCancelReason::RuntimeFailure,
                            delivered_samples,
                            duration_ms,
                        },
                    );
                } else {
                    sink.notify(
                        finished.app_handle.as_ref(),
                        finished.owner,
                        AudioLifecycleEvent::Idle,
                    );
                }
            } else if !idle_after_clear && finished.standby_deadline.is_none() {
                sink.notify(
                    finished.app_handle.as_ref(),
                    finished.owner,
                    AudioLifecycleEvent::Idle,
                );
            }
        }
    }
    finished.active.store(false, Ordering::SeqCst);
    public.still_connecting.store(false, Ordering::SeqCst);
    public.clear_owner(finished.owner);
    public.set_phase(PublicPhase::Idle);
    crate::audio_inventory::lifecycle_became_idle();
    if finished.sealed_delivery {
        sink.notify(
            finished.app_handle.as_ref(),
            finished.owner,
            AudioLifecycleEvent::CleanupFinished,
        );
    }
    for response in finished.retire_responses.drain(..) {
        let _ = response.send(());
    }
    if idle_after_clear {
        // Preview callers wait for this event before opening another device.
        // Publish it only after the worker is joined and supervisor ownership
        // is visibly idle, so device switching cannot race teardown.
        sink.notify(
            finished.app_handle.as_ref(),
            finished.owner,
            AudioLifecycleEvent::Idle,
        );
    }
}

fn take_samples(attempt: &mut Attempt) -> Result<Vec<f32>, String> {
    let mut samples = attempt
        .shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let raw = samples.take();
    drop(samples);
    let raw_duration = if attempt.sample_rate > 0 {
        raw.len() as f64 / attempt.sample_rate as f64
    } else {
        0.0
    };
    tracing::info!(
        target: "audio",
        owner = attempt.owner.telemetry_id(),
        raw_samples = raw.len(),
        sample_rate = attempt.sample_rate,
        wall_secs = recording_duration_since_ready(attempt.ready_at, Instant::now()).as_secs_f64(),
        audio_secs = raw_duration,
        "audio capture finalized"
    );
    convert_for_delivery(
        raw,
        attempt.sample_rate,
        attempt.warm_epoch(),
        samples_for_asr,
    )
}

/// All retained-PCM delivery paths check the warm epoch AFTER conversion.
/// The conversion seam lets tests invalidate topology during finalization.
fn convert_for_delivery(
    raw: Vec<f32>,
    sample_rate: u32,
    warm_epoch: Option<u64>,
    convert: impl FnOnce(Vec<f32>, u32) -> Vec<f32>,
) -> Result<Vec<f32>, String> {
    let samples = convert(raw, sample_rate);
    if warm_epoch.is_none_or(|epoch| epoch == INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire)) {
        Ok(samples)
    } else {
        Err("Microphone changed during the warm recording. Try again.".to_string())
    }
}

fn samples_for_asr(raw: Vec<f32>, sample_rate: u32) -> Vec<f32> {
    if sample_rate != WHISPER_SAMPLE_RATE && !raw.is_empty() {
        audio::resample(&raw, sample_rate, WHISPER_SAMPLE_RATE)
    } else {
        raw
    }
}

fn snapshot_for_asr(
    shared: &[f32],
    sample_rate: u32,
    max_output_samples: Option<usize>,
) -> Vec<f32> {
    let raw = match max_output_samples {
        Some(max_output_samples) => {
            let source_samples = max_output_samples
                .saturating_mul(sample_rate as usize)
                .div_ceil(WHISPER_SAMPLE_RATE as usize)
                .saturating_add(2)
                .min(shared.len());
            shared[shared.len().saturating_sub(source_samples)..].to_vec()
        }
        None => shared.to_vec(),
    };
    let mut samples = samples_for_asr(raw, sample_rate);
    if let Some(max_output_samples) = max_output_samples {
        let start = samples.len().saturating_sub(max_output_samples);
        samples.drain(..start);
    }
    samples
}

fn recording_duration_since_ready(ready_at: Option<Instant>, now: Instant) -> Duration {
    ready_at
        .map(|ready| now.saturating_duration_since(ready))
        .unwrap_or_default()
}

fn begin_recovery(
    attempt: &mut Option<Attempt>,
    reason: AudioCancelReason,
    sink: &dyn LifecycleSink,
    public: &PublicState,
    failure: Option<AudioFailure>,
) {
    let Some(current) = attempt.as_mut() else {
        return;
    };
    if matches!(
        current.phase,
        AttemptPhase::Recovering | AttemptPhase::Stopping
    ) {
        return;
    }
    let was_standby = current.phase == AttemptPhase::Standby;
    if was_standby {
        current
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
    current.active.store(false, Ordering::SeqCst);
    if let Some(permit) = &current.probe_permit {
        permit.revoke();
    }
    if reason != AudioCancelReason::User {
        sink.invalidate_input_health(current.device_id.as_deref());
    }
    let _ = current.command_sender.send(AudioCommand::Stop);
    current.phase = AttemptPhase::Recovering;
    current.recovery_reason = Some(reason);
    current.stopping_started_at = Some(Instant::now());
    public.set_phase(PublicPhase::Recovering);
    tracing::info!(
        target: "audio",
        owner = current.owner.telemetry_id(),
        owner_kind = current.owner.kind(),
        cancellation_reason = reason.as_str(),
        "audio attempt entered recovery; ownership retained until worker exit"
    );
    if !was_standby {
        sink.notify(
            current.app_handle.as_ref(),
            current.owner,
            AudioLifecycleEvent::Recovering { reason },
        );
    }
    if let Some(response) = current.start_response.take() {
        let _ = response.send(Err(AudioStartError::Cancelled));
    }
    if let Some(failure) = failure {
        current.failure = Some(failure.clone());
        report_failure_once(current, sink, failure);
    }
    public.still_connecting.store(false, Ordering::SeqCst);
}

pub(crate) struct InterruptedCapture {
    pub(crate) samples: Vec<f32>,
    pub(crate) reason: String,
    pub(crate) duration_ms: u64,
}

static INTERRUPTED_CAPTURES: OnceLock<Mutex<HashMap<AudioOwner, InterruptedCapture>>> =
    OnceLock::new();

fn interrupted_captures() -> &'static Mutex<HashMap<AudioOwner, InterruptedCapture>> {
    INTERRUPTED_CAPTURES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn take_interrupted_dictation(recording_id: u64) -> Option<InterruptedCapture> {
    interrupted_captures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&AudioOwner::Dictation(recording_id))
}

fn report_failure_once(attempt: &mut Attempt, sink: &dyn LifecycleSink, failure: AudioFailure) {
    if attempt.failure_reported {
        return;
    }
    attempt.failure_reported = true;
    sink.invalidate_input_health(attempt.device_id.as_deref());
    tracing::error!(
        target: "audio",
        event_code = "audio.lifecycle_failed",
        owner = attempt.owner.telemetry_id(),
        owner_kind = attempt.owner.kind(),
        error_kind = failure.kind.as_str(),
        phase = failure.phase.as_str(),
        "audio lifecycle failed"
    );
    sink.notify(
        attempt.app_handle.as_ref(),
        attempt.owner,
        AudioLifecycleEvent::InitializationFailed {
            error: failure.to_string(),
            kind: failure.kind,
            recovery_reason: attempt.recovery_reason,
        },
    );
}

fn handle_deadlines_at(
    attempt: &mut Option<Attempt>,
    sink: &dyn LifecycleSink,
    public: &PublicState,
    config: SupervisorConfig,
    now: Instant,
) {
    let Some(current) = attempt.as_ref() else {
        return;
    };
    if (current
        .probe_permit
        .as_ref()
        .is_some_and(|permit| !permit.is_valid())
        || current
            .probe_deadline
            .is_some_and(|deadline| now >= deadline))
        && matches!(
            current.phase,
            AttemptPhase::Starting | AttemptPhase::Recording
        )
    {
        // A probe lease also ends after successful signal verification. Its
        // cleanup must not advance the failure epoch of the result being saved.
        begin_recovery(attempt, AudioCancelReason::User, sink, public, None);
        return;
    }
    if current.phase == AttemptPhase::Standby
        && (current.standby_deadline.is_some_and(|d| now >= d)
            || current.input_topology_generation
                != INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire))
    {
        begin_recovery(attempt, AudioCancelReason::User, sink, public, None);
        return;
    }
    if current.phase == AttemptPhase::Stopping && current.tail_deadline.is_some_and(|d| now >= d) {
        let current = attempt.as_mut().unwrap();
        current.tail_deadline = None;
        if current.standby_policy.seconds == 0
            || crate::keyboard::is_app_disabled()
            || current.input_topology_generation
                != INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire)
        {
            if current.command_sender.send(AudioCommand::Stop).is_ok() {
                current.recording_stop_owner = Some(current.owner);
                sink.notify(
                    current.app_handle.as_ref(),
                    current.owner,
                    AudioLifecycleEvent::StartupDiagnostic(
                        audio::AudioStartupDiagnostic::RecordingStopIssued,
                    ),
                );
            }
            return;
        }
        // Preserve the pre-optimization standby handoff: a 50 ms margin
        // followed by a live-buffer snapshot. This is not a helper fence;
        // arbitrarily delayed PCM can still arrive after the snapshot.
        let raw = current
            .shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .enter_standby(current.standby_policy.pre_roll_ms);
        current.phase = AttemptPhase::Standby;
        current.standby_deadline =
            Some(now + Duration::from_secs(u64::from(current.standby_policy.seconds)));
        public.set_phase(PublicPhase::Standby);
        tracing::info!(target: "audio",
            event_code = "audio.standby_entered",
            recording_id = current.owner.telemetry_id(),
            standby_seconds = current.standby_policy.seconds,
            pre_roll_ms = current.standby_policy.pre_roll_ms,
            "microphone kept open for bounded standby"
        );
        if let Some(response) = current.stop_response.take() {
            let result = convert_for_delivery(
                raw,
                current.sample_rate,
                current.warm_epoch(),
                samples_for_asr,
            );
            let _ = response.send(result);
        }
        return;
    }
    let current = attempt.as_ref().unwrap();
    let elapsed = current.active_initialization_elapsed(now);
    let permission_prompt_timed_out = current.phase == AttemptPhase::Starting
        && current.tcc_pending_since.is_some_and(|started| {
            now.saturating_duration_since(started) >= config.tcc_prompt_watchdog
        });
    if permission_prompt_timed_out {
        let phase = current.init_phase;
        begin_recovery(
            attempt,
            AudioCancelReason::HardDeadline,
            sink,
            public,
            Some(AudioFailure::new(
                AudioFailureKind::PermissionPromptTimeout,
                phase,
            )),
        );
        return;
    }
    let should_emit_still_connecting = current.phase == AttemptPhase::Starting
        && current.tcc_pending_since.is_none()
        && !current.still_connecting_emitted
        && elapsed >= config.still_connecting_after;
    if should_emit_still_connecting {
        let current = attempt.as_mut().expect("attempt was checked above");
        current.still_connecting_emitted = true;
        public.still_connecting.store(true, Ordering::SeqCst);
        tracing::warn!(
            target: "audio",
            owner = current.owner.telemetry_id(),
            owner_kind = current.owner.kind(),
            elapsed_ms = elapsed.as_millis() as u64,
            "audio initialization still connecting"
        );
        sink.notify(
            current.app_handle.as_ref(),
            current.owner,
            AudioLifecycleEvent::StillConnecting,
        );
    }
    if attempt.as_ref().is_some_and(|current| {
        current.phase == AttemptPhase::Starting && elapsed >= config.hard_deadline
    }) {
        let phase = attempt
            .as_ref()
            .map(|current| current.init_phase)
            .unwrap_or(AudioInitPhase::DeviceEnumeration);
        let kind = if phase == AudioInitPhase::FirstBufferWait {
            AudioFailureKind::FirstBufferTimeout
        } else {
            AudioFailureKind::InitializationTimeout
        };
        begin_recovery(
            attempt,
            AudioCancelReason::HardDeadline,
            sink,
            public,
            Some(AudioFailure::new(kind, phase)),
        );
        return;
    }
    let current = attempt.as_mut().expect("attempt was checked above");
    match current.phase {
        AttemptPhase::Stopping | AttemptPhase::Recovering
            if !current.stopping_guidance_emitted
                && current.stopping_started_at.is_some_and(|started| {
                    now.saturating_duration_since(started) >= config.recovery_guidance_after
                }) =>
        {
            report_cleanup_stalled(current, sink, public);
        }
        _ => {}
    }
}

fn report_cleanup_stalled(current: &mut Attempt, sink: &dyn LifecycleSink, public: &PublicState) {
    if current.stopping_guidance_emitted {
        return;
    }
    current.stopping_guidance_emitted = true;
    public.set_phase(PublicPhase::Recovering);
    sink.invalidate_input_health(current.device_id.as_deref());
    tracing::warn!(target: "audio",
        event_code = "audio.teardown_stalled",
        owner = current.owner.telemetry_id(),
        owner_kind = current.owner.kind(),
        sealed_delivery = current.sealed_delivery,
        "capture cleanup blocked; ownership retained and recovery guidance published"
    );
    sink.notify(
        current.app_handle.as_ref(),
        current.owner,
        AudioLifecycleEvent::RecoveryStalled,
    );
}

fn send_start(
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    origin: &str,
    wait_until_ready: bool,
) -> Result<(), AudioStartError> {
    send_start_with_permit(
        owner,
        app_handle,
        device_id,
        origin,
        wait_until_ready,
        None,
        gesture::StartGesture::default(),
    )
}

fn send_start_with_permit(
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    origin: &str,
    wait_until_ready: bool,
    probe_permit: Option<Arc<crate::smart_auto_probe::ProbePermit>>,
    gesture: gesture::StartGesture,
) -> Result<(), AudioStartError> {
    let (response_sender, response_receiver) = mpsc::channel();
    let caller_alive = Arc::new(AtomicBool::new(true));
    supervisor()
        .sender
        .send(SupervisorMessage::Start(StartRequest {
            gesture,
            caller_alive: caller_alive.clone(),
            owner,
            app_handle,
            device_id,
            origin: origin.to_string(),
            wait_until_ready,
            response: response_sender,
            probe_permit,
        }))
        .map_err(|_| AudioStartError::SupervisorUnavailable)?;
    let timeout = if wait_until_ready {
        HARD_INITIALIZATION_DEADLINE + Duration::from_secs(2)
    } else {
        SUPERVISOR_RESPONSE_TIMEOUT
    };
    await_start_response(
        supervisor(),
        owner,
        caller_alive,
        response_receiver,
        timeout,
    )
}

fn await_start_response(
    supervisor: &AudioSupervisor,
    owner: AudioOwner,
    caller_alive: Arc<AtomicBool>,
    response: Receiver<Result<(), AudioStartError>>,
    timeout: Duration,
) -> Result<(), AudioStartError> {
    match response.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            // A queued handoff may outlive the ordinary response budget. Revoke
            // it before returning failure; also cancel if spawn just won the race.
            caller_alive.store(false, Ordering::Release);
            let (response, _) = mpsc::channel();
            let _ = supervisor.sender.send(SupervisorMessage::Cancel {
                owner: Some(owner),
                reason: AudioCancelReason::User,
                starting_only: false,
                response,
            });
            Err(AudioStartError::SupervisorUnavailable)
        }
    }
}

pub(crate) fn start_dictation_recording(
    app_handle: tauri::AppHandle,
    device_id: Option<String>,
    recording_id: u64,
    origin: &str,
    gesture: gesture::StartGesture,
) -> Result<(), AudioStartError> {
    send_start_with_permit(
        AudioOwner::Dictation(recording_id),
        Some(app_handle),
        device_id,
        origin,
        false,
        None,
        gesture,
    )
}

pub(crate) fn start_transform_recording(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    transform_pass_id: u64,
) -> Result<(), String> {
    send_start(
        AudioOwner::Transform(transform_pass_id),
        app_handle,
        device_id,
        "transform",
        false,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn start_query_recording(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    query_pass_id: u64,
) -> Result<(), String> {
    send_start(
        AudioOwner::Query(query_pass_id),
        app_handle,
        device_id,
        "query",
        false,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn start_preview_recording(
    app_handle: tauri::AppHandle,
    device_id: Option<String>,
    preview_id: u64,
) -> Result<(), AudioStartError> {
    send_start(
        AudioOwner::Preview(preview_id),
        Some(app_handle),
        device_id,
        "preview",
        false,
    )
}

pub(crate) fn start_automatic_preview_recording(
    app_handle: tauri::AppHandle,
    device_id: String,
    preview_id: u64,
    permit: Arc<crate::smart_auto_probe::ProbePermit>,
) -> Result<(), AudioStartError> {
    send_start_with_permit(
        AudioOwner::Preview(preview_id),
        Some(app_handle),
        Some(device_id),
        "automatic_preview",
        false,
        Some(permit),
        gesture::StartGesture::default(),
    )
}

pub(crate) fn start_microphone_benchmark_recording(
    app_handle: tauri::AppHandle,
    device_id: Option<String>,
    cycle_id: u64,
) -> Result<(), AudioStartError> {
    send_start(
        AudioOwner::MicrophoneBenchmark(cycle_id),
        Some(app_handle),
        device_id,
        "microphone_startup_benchmark",
        false,
    )
}

#[cfg(feature = "internal-benchmark")]
pub(crate) fn start_corpus_recording(
    app_handle: tauri::AppHandle,
    device_id: Option<String>,
    capture_id: u64,
) -> Result<(), String> {
    send_start(
        AudioOwner::Corpus(capture_id),
        Some(app_handle),
        device_id,
        "corpus",
        true,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn stop_dictation_recording(
    recording_id: u64,
    trigger: DictationStopTrigger,
) -> Result<Vec<f32>, String> {
    stop_with_trigger(Some(AudioOwner::Dictation(recording_id)), trigger)
}

pub(crate) fn stop_query_recording(query_pass_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Query(query_pass_id)))
}

/// Clone the live query buffer without stopping capture.
///
/// Returns `None` when this pass is not the recording owner. The clone is a
/// snapshot; later PCM stays in the retained buffer for the authoritative pass.
pub(crate) fn peek_query_samples(query_pass_id: u64) -> Option<Vec<f32>> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Peek {
            owner: AudioOwner::Query(query_pass_id),
            max_output_samples: None,
            response: response_sender,
        })
        .ok()?;
    response_receiver
        .recv_timeout(SUPERVISOR_RESPONSE_TIMEOUT)
        .ok()?
}

/// Clone the live dictation buffer without stopping capture. The authoritative
/// final stop continues to own and consume the retained samples.
pub(crate) fn peek_dictation_samples(
    recording_id: u64,
    max_output_samples: usize,
) -> Option<Vec<f32>> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Peek {
            owner: AudioOwner::Dictation(recording_id),
            max_output_samples: Some(max_output_samples),
            response: response_sender,
        })
        .ok()?;
    response_receiver
        .recv_timeout(SUPERVISOR_RESPONSE_TIMEOUT)
        .ok()?
}

pub(crate) fn stop_preview_recording(preview_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Preview(preview_id)))
}

pub(crate) fn stop_microphone_benchmark_recording(cycle_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::MicrophoneBenchmark(cycle_id)))
}

#[cfg(feature = "internal-benchmark")]
pub(crate) fn stop_corpus_recording(capture_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Corpus(capture_id)))
}

pub(crate) fn stop_current_recording() -> Result<Vec<f32>, String> {
    stop(None)
}

fn stop(owner: Option<AudioOwner>) -> Result<Vec<f32>, String> {
    stop_with_trigger(owner, DictationStopTrigger::Other)
}

fn stop_with_trigger(
    owner: Option<AudioOwner>,
    trigger: DictationStopTrigger,
) -> Result<Vec<f32>, String> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Stop {
            trigger,
            owner,
            response: response_sender,
        })
        .map_err(|_| "Audio lifecycle supervisor is unavailable".to_string())?;
    response_receiver
        .recv_timeout(STOP_RESPONSE_TIMEOUT)
        .map_err(|error| match error {
            RecvTimeoutError::Timeout => {
                "Microphone teardown is still blocked; the owned worker remains in recovery"
                    .to_string()
            }
            RecvTimeoutError::Disconnected => {
                "Audio lifecycle supervisor stopped during teardown".to_string()
            }
        })?
}

pub(crate) fn cancel_dictation_capture(
    recording_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(Some(AudioOwner::Dictation(recording_id)), reason, false)
}

pub(crate) fn cancel_query_capture(
    query_pass_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(Some(AudioOwner::Query(query_pass_id)), reason, false)
}

pub(crate) fn cancel_preview_capture(
    preview_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(Some(AudioOwner::Preview(preview_id)), reason, false)
}

pub(crate) fn cancel_microphone_benchmark_capture(
    cycle_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(
        Some(AudioOwner::MicrophoneBenchmark(cycle_id)),
        reason,
        false,
    )
}

#[cfg(feature = "internal-benchmark")]
pub(crate) fn cancel_corpus_capture(
    capture_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(Some(AudioOwner::Corpus(capture_id)), reason, false)
}

pub(crate) fn cancel_current(reason: AudioCancelReason) -> Result<(), String> {
    cancel(None, reason, false).map(|_| ())
}

pub(crate) fn cancel_starting_for_environment_change(reason: AudioCancelReason) {
    let (response_sender, _response_receiver) = mpsc::channel();
    let _ = supervisor().sender.send(SupervisorMessage::Cancel {
        owner: None,
        reason,
        starting_only: true,
        response: response_sender,
    });
}

pub(crate) fn cancel_preview_for_environment_change(reason: AudioCancelReason) {
    let Some(preview_id) = supervisor().public.preview_owner() else {
        return;
    };
    let (response_sender, _response_receiver) = mpsc::channel();
    let _ = supervisor().sender.send(SupervisorMessage::Cancel {
        owner: Some(AudioOwner::Preview(preview_id)),
        reason,
        starting_only: false,
        response: response_sender,
    });
}

fn cancel(
    owner: Option<AudioOwner>,
    reason: AudioCancelReason,
    starting_only: bool,
) -> Result<bool, String> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Cancel {
            owner,
            reason,
            starting_only,
            response: response_sender,
        })
        .map_err(|_| "Audio lifecycle supervisor is unavailable".to_string())?;
    response_receiver
        .recv_timeout(SUPERVISOR_RESPONSE_TIMEOUT)
        .map_err(|_| "Audio lifecycle supervisor did not acknowledge cancellation".to_string())?
}

/// Called directly by the native default-input/device topology notification.
/// The epoch closes the notification-to-supervisor race before any warm reuse.
pub(crate) fn input_topology_generation() -> u64 {
    INPUT_TOPOLOGY_GENERATION.load(Ordering::Acquire)
}

pub(crate) fn input_topology_changed() {
    INPUT_TOPOLOGY_GENERATION.fetch_add(1, Ordering::AcqRel);
    audio::prepare::invalidate(audio::prepare::InvalidateReason::TopologyChanged);
    if let Some(supervisor) = SUPERVISOR.get() {
        let _ = supervisor
            .sender
            .send(SupervisorMessage::InputTopologyChanged);
    }
}

/// Disabled by default; settings transport clamps the memory/time bounds.
pub(crate) fn configure_standby(enabled: bool, seconds: u32, pre_roll_ms: u32) {
    if enabled {
        audio::prepare::capture_claimed(audio::prepare::CaptureClaim::Standby);
    }
    let policy = StandbyPolicy {
        seconds: if enabled { seconds.clamp(30, 900) } else { 0 },
        pre_roll_ms: pre_roll_ms.clamp(100, 450),
    };
    let _ = supervisor()
        .sender
        .send(SupervisorMessage::ConfigureStandby(policy));
}

/// Release optional standby before another feature claims the microphone.
/// Keep the public busy state until the exact worker has actually exited.
pub(crate) fn retire_standby() {
    if SUPERVISOR.get().is_none() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let _ = supervisor()
        .sender
        .send(SupervisorMessage::RetireStandby(tx));
    let _ = rx.recv_timeout(SUPERVISOR_RESPONSE_TIMEOUT);
}

pub(crate) fn is_audio_active() -> bool {
    supervisor().public.is_active()
}

/// A new logical dictation generation must not supersede cleanup guidance
/// while the previous worker is still exclusively owned.
pub(crate) fn is_dictation_cleanup_pending(recording_id: u64) -> bool {
    let public = &supervisor().public;
    public.owns(AudioOwner::Dictation(recording_id))
        && matches!(
            public.phase(),
            PublicPhase::Stopping | PublicPhase::Recovering
        )
}

/// Read-only lifecycle view for hang diagnostics.
pub(crate) struct AudioLifecycleSnapshot {
    pub(crate) phase: &'static str,
    /// Rendered owner: `none`, `<busy>` when the owner lock is contended, or
    /// `kind#id`.
    pub(crate) owner: String,
    pub(crate) still_connecting: bool,
}

/// Current capture phase and owner, or `None` when the supervisor has never
/// started (so no capture context exists at all).
///
/// Deliberately reads `SUPERVISOR.get()` rather than `supervisor()`: the
/// latter is `get_or_init` and would spawn the supervisor thread from a
/// diagnostic path. The owner is read with `try_lock` so a diagnostic never
/// waits on the lock it is trying to describe.
pub(crate) fn diagnostic_snapshot() -> Option<AudioLifecycleSnapshot> {
    let supervisor = SUPERVISOR.get()?;
    let owner = crate::MutexExt::try_lock_or_recover(&supervisor.public.owner)
        .map(|guard| render_owner(*guard))
        .unwrap_or_else(|| "<busy>".to_string());
    Some(AudioLifecycleSnapshot {
        phase: supervisor.public.phase().as_str(),
        owner,
        still_connecting: supervisor.public.still_connecting.load(Ordering::SeqCst),
    })
}

fn render_owner(owner: Option<AudioOwner>) -> String {
    match owner {
        Some(owner) => format!("{}#{}", owner.kind(), owner.telemetry_id()),
        None => "none".to_string(),
    }
}

pub(crate) fn is_microphone_benchmark_owner(cycle_id: u64) -> bool {
    supervisor()
        .public
        .owns(AudioOwner::MicrophoneBenchmark(cycle_id))
}

/// Run a diagnostic-only HAL operation only when capture does not own the
/// boundary. Capture start takes the same mutex before publishing Starting,
/// closing the check-to-enumeration race in both directions.
pub(crate) fn with_idle_hal_boundary<T>(operation: impl FnOnce() -> T) -> Option<T> {
    let _hal_boundary = hal_boundary()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if is_audio_active() {
        return None;
    }
    Some(operation())
}

pub(crate) fn is_dictation_recording(recording_id: u64) -> bool {
    supervisor()
        .public
        .is_recording_for(AudioOwner::Dictation(recording_id))
}

pub(crate) fn is_transform_recording(transform_pass_id: u64) -> bool {
    supervisor()
        .public
        .is_recording_for(AudioOwner::Transform(transform_pass_id))
}

pub(crate) fn is_transform_still_connecting(transform_pass_id: u64) -> bool {
    supervisor()
        .public
        .is_still_connecting_for(AudioOwner::Transform(transform_pass_id))
}

#[cfg(target_os = "macos")]
pub(crate) fn register_sleep_wake_observer() {
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSNotification, NSOperationQueue};

    fn register(
        center: &objc2_foundation::NSNotificationCenter,
        name: &objc2_foundation::NSNotificationName,
        reason: AudioCancelReason,
    ) {
        let block =
            block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
                crate::audio_inventory::invalidate_signal_evidence(None);
                if reason == AudioCancelReason::SystemSleep {
                    cancel_preview_for_environment_change(reason);
                }
                cancel_starting_for_environment_change(reason);
            });
        unsafe {
            let observer = center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            );
            std::mem::forget(observer);
        }
    }

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    register(
        &center,
        unsafe { NSWorkspaceWillSleepNotification },
        AudioCancelReason::SystemSleep,
    );
    register(
        &center,
        unsafe { NSWorkspaceDidWakeNotification },
        AudioCancelReason::SystemWake,
    );
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn register_sleep_wake_observer() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_trigger_tail_mapping_preserves_speech_except_explicit_toggle() {
        for trigger in [
            DictationStopTrigger::HoldRelease,
            DictationStopTrigger::Overlay,
            DictationStopTrigger::AutoStop,
            DictationStopTrigger::Other,
        ] {
            assert_eq!(
                trigger.release_tail(AudioOwner::Dictation(1), false),
                Duration::from_millis(50)
            );
        }
        assert_eq!(
            DictationStopTrigger::Toggle.release_tail(AudioOwner::Dictation(1), false),
            Duration::ZERO
        );
        for owner in [
            AudioOwner::Transform(1),
            AudioOwner::Query(1),
            AudioOwner::Preview(1),
        ] {
            for trigger in [
                DictationStopTrigger::HoldRelease,
                DictationStopTrigger::Toggle,
                DictationStopTrigger::Other,
            ] {
                assert_eq!(trigger.release_tail(owner, false), Duration::ZERO);
                assert_eq!(trigger.release_tail(owner, true), Duration::ZERO);
            }
        }
        assert_eq!(
            DictationStopTrigger::Toggle.release_tail(AudioOwner::Dictation(1), true),
            Duration::from_millis(50)
        );
        assert_eq!(DictationStopTrigger::default(), DictationStopTrigger::Other);
        let now = Instant::now();
        let release = crate::hotkey_release::HotkeyRelease::for_test(now, now);
        assert_eq!(
            DictationStopTrigger::Toggle.resolve(Some(release)),
            DictationStopTrigger::HoldRelease
        );
        assert_eq!(
            DictationStopTrigger::Toggle.resolve(None),
            DictationStopTrigger::Toggle
        );

        assert_eq!(
            serde_json::from_str::<DictationStopTrigger>("\"toggle\"").unwrap(),
            DictationStopTrigger::Toggle
        );
        assert!(serde_json::from_str::<DictationStopTrigger>("\"unknown\"").is_err());
    }

    #[test]
    fn microphone_benchmark_has_distinct_non_retaining_ownership() {
        let owner = AudioOwner::MicrophoneBenchmark(41);
        assert!(owner.is_microphone_benchmark());
        assert!(!owner.retains_samples());
        assert_eq!(owner.kind(), "microphone_benchmark");
        assert_ne!(owner, AudioOwner::Preview(41));
    }
    use crate::audio::{AudioInitPhase, AudioWorkerEvent};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};

    struct AutomaticFactory {
        authorized: bool,
        ready: bool,
        spawned: Arc<AtomicUsize>,
        teardown: Gate,
    }

    impl WorkerFactory for AutomaticFactory {
        fn automatic_capture_authorized(&self) -> bool {
            self.authorized
        }

        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            events: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            let ready = self.ready;
            let teardown = self.teardown.clone();
            Ok(std::thread::spawn(move || {
                assert!(spec.probe_permit.is_some());
                assert!(!spec.owner.retains_samples());
                if ready {
                    let _ = events.send(AudioWorkerEvent::FirstBuffer {
                        owner: spec.owner,
                        sample_rate: WHISPER_SAMPLE_RATE,
                        sent_at: std::time::Instant::now(),
                    });
                }
                let _ = spec.command_receiver.recv();
                teardown.wait();
                assert!(spec.shared.lock().unwrap().is_empty());
                let _ = events.send(AudioWorkerEvent::ThreadExited { owner: spec.owner });
            }))
        }
    }

    fn automatic_harness(
        authorized: bool,
        ready: bool,
    ) -> (AudioSupervisor, Gate, Arc<AtomicUsize>) {
        let teardown = Gate::closed();
        let spawned = Arc::new(AtomicUsize::new(0));
        let supervisor = spawn_supervisor(
            Arc::new(AutomaticFactory {
                authorized,
                ready,
                teardown: teardown.clone(),
                spawned: spawned.clone(),
            }),
            Arc::new(RecordingSink::default()),
            SupervisorConfig::default(),
        );
        (supervisor, teardown, spawned)
    }

    fn automatic_start(
        supervisor: &AudioSupervisor,
        permit: Arc<crate::smart_auto_probe::ProbePermit>,
    ) -> Receiver<Result<(), AudioStartError>> {
        let (response, result) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Start(StartRequest {
                gesture: gesture::StartGesture::default(),
                caller_alive: Arc::new(AtomicBool::new(true)),
                owner: AudioOwner::Preview(525),
                app_handle: None,
                device_id: Some("test-input".into()),
                origin: "automatic_preview".into(),
                wait_until_ready: false,
                response,
                probe_permit: Some(permit),
            }))
            .unwrap();
        result
    }

    #[test]
    fn automatic_probe_expired_revoked_or_ungranted_never_spawns() {
        for (expired, revoked, authorized) in [
            (true, false, true),
            (false, true, true),
            (false, false, false),
        ] {
            let (supervisor, _, spawned) = automatic_harness(authorized, false);
            let now = if expired {
                Instant::now() - crate::smart_auto_probe::PROBE_LIMIT
            } else {
                Instant::now()
            };
            let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(now));
            if revoked {
                permit.revoke();
            }
            assert!(automatic_start(&supervisor, permit)
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err());
            assert_eq!(spawned.load(Ordering::SeqCst), 0);
            assert_eq!(supervisor.public.phase(), PublicPhase::Idle);
            shutdown(&supervisor);
        }
    }

    #[test]
    fn automatic_probe_recording_revocation_and_expiry_hold_owner_until_join() {
        for expire in [false, true] {
            let (supervisor, teardown, spawned) = automatic_harness(true, true);
            let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
            assert_eq!(
                automatic_start(&supervisor, permit.clone())
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap(),
                Ok(())
            );
            wait_until("automatic capture did not become ready", || {
                supervisor.public.phase() == PublicPhase::Recording
            });
            if expire {
                check_deadlines(&supervisor, Duration::from_secs(9));
            } else {
                permit.revoke();
            }
            wait_until("automatic capture was not cancelled", || {
                supervisor.public.phase() == PublicPhase::Recovering
            });
            assert!(!permit.is_valid());
            assert_eq!(spawned.load(Ordering::SeqCst), 1);
            assert_eq!(
                start(&supervisor, AudioOwner::Dictation(526))
                    .recv()
                    .unwrap(),
                Err(AudioStartError::AudioRecovering)
            );
            teardown.open();
            wait_until("automatic owner did not join", || {
                supervisor.public.phase() == PublicPhase::Idle
            });
            shutdown(&supervisor);
        }
    }

    #[test]
    fn automatic_probe_permission_pause_is_rejected_and_keeps_teardown_ownership() {
        let (supervisor, teardown, _) = automatic_harness(true, false);
        let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
        assert_eq!(
            automatic_start(&supervisor, permit.clone()).recv().unwrap(),
            Ok(())
        );
        supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::PermissionPromptPending {
                    owner: AudioOwner::Preview(525),
                },
            ))
            .unwrap();
        wait_until("automatic TCC pause was accepted", || {
            supervisor.public.phase() == PublicPhase::Recovering
        });
        assert!(!permit.is_valid());
        teardown.open();
        wait_until("automatic TCC teardown did not join", || {
            supervisor.public.phase() == PublicPhase::Idle
        });
        shutdown(&supervisor);
    }

    #[test]
    fn automatic_probe_stop_requires_exact_owner_and_waits_for_teardown() {
        let (supervisor, teardown, _) = automatic_harness(true, true);
        let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
        assert_eq!(
            automatic_start(&supervisor, permit.clone()).recv().unwrap(),
            Ok(())
        );
        wait_until("automatic capture did not become ready", || {
            supervisor.public.phase() == PublicPhase::Recording
        });
        let (response, result) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(AudioOwner::Preview(526)),
                response,
            })
            .unwrap();
        assert!(result.recv().unwrap().is_err());
        assert!(permit.is_valid());
        assert_eq!(supervisor.public.phase(), PublicPhase::Recording);
        let (response, result) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(AudioOwner::Preview(525)),
                response,
            })
            .unwrap();
        wait_until("automatic capture did not start stopping", || {
            supervisor.public.phase() == PublicPhase::Stopping
        });
        assert!(!permit.is_valid());
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        teardown.open();
        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok(Vec::new())
        );
        wait_until("automatic stop did not join", || {
            supervisor.public.phase() == PublicPhase::Idle
        });
        shutdown(&supervisor);
    }

    #[test]
    fn automatic_probe_verified_revoke_then_stop_preserves_signal_health() {
        let teardown = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(AutomaticFactory {
                authorized: true,
                ready: true,
                spawned: Arc::new(AtomicUsize::new(0)),
                teardown: teardown.clone(),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
        assert_eq!(
            automatic_start(&supervisor, permit.clone()).recv().unwrap(),
            Ok(())
        );
        wait_until("automatic capture did not become ready", || {
            supervisor.public.phase() == PublicPhase::Recording
        });
        // Match the scheduler: collect its result, revoke the permit, then ask
        // the exact owner to stop. Deadlines run before that Stop is processed.
        permit.revoke();
        let (response, result) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(AudioOwner::Preview(525)),
                response,
            })
            .unwrap();
        assert!(result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_err());
        assert_eq!(supervisor.public.phase(), PublicPhase::Recovering);
        assert!(sink.invalidated_inputs.lock().unwrap().is_empty());
        teardown.open();
        wait_until("automatic verified stop did not join", || {
            supervisor.public.phase() == PublicPhase::Idle
        });
        assert!(
            sink.invalidated_inputs.lock().unwrap().is_empty(),
            "normal cleanup must not invalidate the saved signal key's failure epoch"
        );
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| matches!(
                event,
                AudioLifecycleEvent::Recovering {
                    reason: AudioCancelReason::User
                }
            )));
        shutdown(&supervisor);
    }

    #[test]
    fn automatic_probe_runtime_failure_still_invalidates_signal_health() {
        let teardown = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(AutomaticFactory {
                authorized: true,
                ready: true,
                spawned: Arc::new(AtomicUsize::new(0)),
                teardown: teardown.clone(),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
        assert_eq!(automatic_start(&supervisor, permit).recv().unwrap(), Ok(()));
        wait_until("automatic capture did not become ready", || {
            supervisor.public.phase() == PublicPhase::Recording
        });
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::RuntimeFailed {
                owner: AudioOwner::Preview(525),
                failure: AudioFailure::new(AudioFailureKind::BackendError, AudioInitPhase::Runtime),
            }))
            .unwrap();
        wait_until("automatic runtime failure was not invalidated", || {
            !sink.invalidated_inputs.lock().unwrap().is_empty()
        });
        teardown.open();
        wait_until("automatic failed capture did not join", || {
            supervisor.public.phase() == PublicPhase::Idle
        });
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| matches!(event, AudioLifecycleEvent::InitializationFailed { .. })));
        shutdown(&supervisor);
    }

    #[test]
    fn automatic_probe_revoked_while_waiting_for_hal_never_spawns() {
        let (supervisor, _, spawned) = automatic_harness(true, false);
        let permit = Arc::new(crate::smart_auto_probe::ProbePermit::new(Instant::now()));
        let boundary = hal_boundary().lock().unwrap();
        let result = automatic_start(&supervisor, permit.clone());
        permit.revoke();
        drop(boundary);
        assert_eq!(
            result.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(AudioStartError::Cancelled)
        );
        assert_eq!(spawned.load(Ordering::SeqCst), 0);
        shutdown(&supervisor);
    }

    #[derive(Clone)]
    struct Gate {
        state: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Gate {
        fn closed() -> Self {
            Self {
                state: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }

        fn wait(&self) {
            let (lock, condition) = &*self.state;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = condition.wait(open).unwrap();
            }
        }

        fn open(&self) {
            let (lock, condition) = &*self.state;
            *lock.lock().unwrap() = true;
            condition.notify_all();
        }
    }

    struct BlockingFactory {
        gate: Gate,
        retry_gate: Option<Gate>,
        spawn_count: Arc<AtomicUsize>,
        active_flags: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
        phase: AudioInitPhase,
        phase_entered: Option<Sender<()>>,
    }

    struct BlockingTeardownFactory {
        gate: Gate,
    }

    struct FailingFactory;

    struct NoCallbackFactory {
        first_buffer_wait_entered: Sender<()>,
    }

    impl WorkerFactory for FailingFactory {
        fn spawn(
            &self,
            _spec: AudioWorkerSpec,
            _event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            Err("test spawn failure".to_string())
        }
    }

    impl WorkerFactory for NoCallbackFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            let first_buffer_wait_entered = self.first_buffer_wait_entered.clone();
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                for phase in [
                    AudioInitPhase::DeviceEnumeration,
                    AudioInitPhase::StreamBuild,
                ] {
                    let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
                    let _ = event_sender.send(AudioWorkerEvent::PhaseExited {
                        owner,
                        phase,
                        elapsed_ms: 0,
                    });
                }
                let _ = event_sender.send(AudioWorkerEvent::PhaseEntered {
                    owner,
                    phase: AudioInitPhase::FirstBufferWait,
                });
                let _ = first_buffer_wait_entered.send(());
                let _ = spec.command_receiver.recv();
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            }))
        }
    }

    struct TailPolicyFactory(Arc<Mutex<Option<Receiver<AudioCommand>>>>);

    impl WorkerFactory for TailPolicyFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            _events: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            spec.shared
                .lock()
                .unwrap()
                .append_pcm(&[0.25; 160], WHISPER_SAMPLE_RATE);
            *self.0.lock().unwrap() = Some(spec.command_receiver);
            Ok(std::thread::spawn(|| {}))
        }
    }

    #[test]
    fn stop_tail_deadline_applies_once_for_cold_and_standby_capture() {
        for standby in [false, true] {
            for trigger in [
                DictationStopTrigger::HoldRelease,
                DictationStopTrigger::Toggle,
                DictationStopTrigger::Overlay,
                DictationStopTrigger::AutoStop,
                DictationStopTrigger::Other,
            ] {
                let commands = Arc::new(Mutex::new(None));
                let factory = TailPolicyFactory(commands.clone());
                let events = AudioWorkerEventSender::new(|_| Ok(()));
                let public = PublicState::default();
                let sink = RecordingSink::default();
                let owner = AudioOwner::Dictation(702);
                let mut attempt = None;
                let (response, _started) = mpsc::channel();
                handle_start(
                    StartRequest {
                        gesture: gesture::StartGesture::default(),
                        caller_alive: Arc::new(AtomicBool::new(true)),
                        owner,
                        app_handle: None,
                        device_id: None,
                        origin: "toggle".to_string(),
                        wait_until_ready: false,
                        response,
                        probe_permit: None,
                    },
                    &mut attempt,
                    &events,
                    &factory,
                    &sink,
                    &public,
                );
                handle_worker_event(
                    AudioWorkerEvent::FirstBuffer {
                        owner,
                        sample_rate: WHISPER_SAMPLE_RATE,
                        sent_at: std::time::Instant::now(),
                    },
                    &mut attempt,
                    &sink,
                    &public,
                );
                attempt.as_mut().unwrap().standby_policy.seconds = if standby { 300 } else { 0 };
                let (response, result) = mpsc::channel();
                let before = Instant::now();
                handle_message(
                    SupervisorMessage::Stop {
                        owner: Some(owner),
                        trigger,
                        response,
                    },
                    &mut attempt,
                    &events,
                    &factory,
                    &sink,
                    &public,
                );
                let deadline = attempt.as_ref().unwrap().tail_deadline.unwrap();
                assert!(deadline >= before + trigger.release_tail(owner, standby));
                assert!(deadline <= Instant::now() + trigger.release_tail(owner, standby));
                let commands = commands.lock().unwrap().take().unwrap();
                handle_deadlines_at(
                    &mut attempt,
                    &sink,
                    &public,
                    SupervisorConfig::default(),
                    deadline - Duration::from_nanos(1),
                );
                assert!(matches!(
                    commands.try_recv(),
                    Err(mpsc::TryRecvError::Empty)
                ));
                assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
                handle_deadlines_at(
                    &mut attempt,
                    &sink,
                    &public,
                    SupervisorConfig::default(),
                    deadline,
                );
                assert!(attempt.as_ref().unwrap().tail_deadline.is_none());
                if standby {
                    assert_eq!(public.phase(), PublicPhase::Standby);
                    assert_eq!(result.recv().unwrap().unwrap(), vec![0.25; 160]);
                    assert!(matches!(
                        commands.try_recv(),
                        Err(mpsc::TryRecvError::Empty)
                    ));
                } else {
                    assert!(matches!(commands.try_recv(), Ok(AudioCommand::Stop)));
                    assert_eq!(public.phase(), PublicPhase::Stopping);
                    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
                }
                handle_deadlines_at(
                    &mut attempt,
                    &sink,
                    &public,
                    SupervisorConfig::default(),
                    deadline,
                );
                assert!(matches!(
                    commands.try_recv(),
                    Err(mpsc::TryRecvError::Empty)
                ));
                finish_attempt(&mut attempt, &sink, &public);
            }
        }
    }

    #[test]
    fn standby_margin_keeps_pcm_delivered_before_the_deadline() {
        assert_standby_margin(false);
    }

    #[test]
    fn standby_margin_preserves_baseline_with_pcm_still_delayed_at_the_deadline() {
        assert_standby_margin(true);
    }

    fn assert_standby_margin(delayed_past_deadline: bool) {
        let commands = Arc::new(Mutex::new(None));
        let factory = TailPolicyFactory(commands);
        let events = AudioWorkerEventSender::new(|_| Ok(()));
        let public = PublicState::default();
        let sink = RecordingSink::default();
        let owner = AudioOwner::Dictation(703);
        let mut attempt = None;
        let (response, _started) = mpsc::channel();
        handle_start(
            StartRequest {
                gesture: gesture::StartGesture::default(),
                caller_alive: Arc::new(AtomicBool::new(true)),
                owner,
                app_handle: None,
                device_id: None,
                origin: "toggle".to_string(),
                wait_until_ready: false,
                response,
                probe_permit: None,
            },
            &mut attempt,
            &events,
            &factory,
            &sink,
            &public,
        );
        handle_worker_event(
            AudioWorkerEvent::FirstBuffer {
                owner,
                sample_rate: WHISPER_SAMPLE_RATE,
                sent_at: std::time::Instant::now(),
            },
            &mut attempt,
            &sink,
            &public,
        );
        attempt.as_mut().unwrap().standby_policy = StandbyPolicy {
            seconds: 300,
            pre_roll_ms: 450,
        };
        let shared = attempt.as_ref().unwrap().shared.clone();
        let gate = Gate::closed();
        let final_pcm = gate.clone();
        let mut producer = Some(std::thread::spawn(move || {
            // This callback has already captured the ending, but delivery from
            // the helper/reader is stalled until after the accepted stop.
            let captured = [0.75; 80];
            final_pcm.wait();
            shared
                .lock()
                .unwrap()
                .append_pcm(&captured, WHISPER_SAMPLE_RATE);
        }));
        let (response, result) = mpsc::channel();
        handle_message(
            SupervisorMessage::Stop {
                owner: Some(owner),
                trigger: DictationStopTrigger::Toggle,
                response,
            },
            &mut attempt,
            &events,
            &factory,
            &sink,
            &public,
        );
        let now = attempt.as_ref().unwrap().stopping_started_at.unwrap();
        handle_deadlines_at(
            &mut attempt,
            &sink,
            &public,
            SupervisorConfig::default(),
            now,
        );
        assert_eq!(public.phase(), PublicPhase::Stopping);
        assert!(
            matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "standby must not finalize before the retained release tail"
        );
        if !delayed_past_deadline {
            gate.open();
            producer.take().unwrap().join().unwrap();
        }
        let deadline = attempt.as_ref().unwrap().tail_deadline.unwrap();
        handle_deadlines_at(
            &mut attempt,
            &sink,
            &public,
            SupervisorConfig::default(),
            deadline,
        );
        let samples = result.recv().unwrap().unwrap();
        assert_eq!(&samples[..160], &[0.25; 160]);
        if delayed_past_deadline {
            // The producer is still blocked at the deadline. This is the
            // existing standby limitation, not a proven delivery fence:
            // late PCM becomes pre-roll instead of the finalized recording.
            assert_eq!(samples.len(), 160);
            assert_eq!(public.phase(), PublicPhase::Standby);
            gate.open();
            producer.take().unwrap().join().unwrap();
            assert_eq!(
                attempt.as_ref().unwrap().shared.lock().unwrap().as_slice(),
                &[0.75; 80]
            );
        } else {
            assert_eq!(&samples[160..], &[0.75; 80]);
        }
        finish_attempt(&mut attempt, &sink, &public);
    }

    struct WireDrainFactory {
        pcm: Gate,
        receipt: Gate,
        exit: Gate,
        stop_seen: Sender<()>,
        terminal: murmur_capture_helper_protocol::ProductionHelperMessage,
    }

    impl WorkerFactory for WireDrainFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            events: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            use murmur_capture_helper_protocol::{
                read_production_frame, write_production_control, write_production_pcm,
                CaptureChannel, ProductionPcmMetadata,
            };
            let pcm = self.pcm.clone();
            let receipt = self.receipt.clone();
            let exit = self.exit.clone();
            let stop_seen = self.stop_seen.clone();
            let terminal = self.terminal.clone();
            Ok(std::thread::spawn(move || {
                spec.shared
                    .lock()
                    .unwrap()
                    .append_pcm(&[0.25; 160], WHISPER_SAMPLE_RATE);
                events
                    .send(AudioWorkerEvent::FirstBuffer {
                        owner: spec.owner,
                        sample_rate: WHISPER_SAMPLE_RATE,
                        sent_at: std::time::Instant::now(),
                    })
                    .unwrap();
                spec.command_receiver.recv().unwrap();
                let (frames, reader) = mpsc::channel();
                let owner = spec.owner;
                let producer = std::thread::spawn(move || {
                    // Encode and authenticate real protocol frames, with final
                    // PCM held in flight independently of Stopped and exit.
                    let nonce = [3; 16];
                    let id = owner.telemetry_id();
                    let mut wire = Vec::new();
                    write_production_pcm(
                        &mut wire,
                        id,
                        nonce,
                        ProductionPcmMetadata {
                            channel: CaptureChannel::Microphone,
                            sequence: 1,
                            sample_rate: WHISPER_SAMPLE_RATE,
                            captured_at_ns: 0,
                            sample_offset: 160,
                        },
                        &[0.75; 80],
                    )
                    .unwrap();
                    pcm.wait();
                    frames
                        .send(audio::HelperRead::Frame(
                            read_production_frame(&mut wire.as_slice(), id, nonce).unwrap(),
                        ))
                        .unwrap();
                    receipt.wait();
                    wire.clear();
                    write_production_control(&mut wire, id, nonce, &terminal).unwrap();
                    frames
                        .send(audio::HelperRead::Frame(
                            read_production_frame(&mut wire.as_slice(), id, nonce).unwrap(),
                        ))
                        .unwrap();
                });
                stop_seen.send(()).unwrap();
                audio::drain_stopping_recording(
                    &reader,
                    Instant::now() + Duration::from_secs(5),
                    &mut 1,
                    Some(WHISPER_SAMPLE_RATE),
                    owner,
                    &events,
                    &mut 160,
                    &mut |samples| {
                        spec.shared
                            .lock()
                            .unwrap()
                            .append_pcm(samples, WHISPER_SAMPLE_RATE)
                    },
                );
                producer.join().unwrap();
                exit.wait();
                events
                    .send(AudioWorkerEvent::ThreadExited { owner })
                    .unwrap();
            }))
        }
    }

    #[test]
    fn delayed_protocol_pcm_reaches_supervisor_only_after_valid_stopped() {
        use murmur_capture_helper_protocol::{
            CaptureBackend, FailureCode, ProductionHelperMessage,
        };
        for terminal in [
            ProductionHelperMessage::Stopped {
                retained_samples: 240,
            },
            ProductionHelperMessage::Stopped {
                retained_samples: 241,
            },
            ProductionHelperMessage::Failure {
                code: FailureCode::StreamError,
                backend: CaptureBackend::Auhal,
                retained_samples: 240,
            },
        ] {
            let complete = matches!(
                terminal,
                ProductionHelperMessage::Stopped {
                    retained_samples: 240
                }
            );
            let pcm = Gate::closed();
            let receipt = Gate::closed();
            let exit = Gate::closed();
            let (stop_seen, seen) = mpsc::channel();
            let supervisor = spawn_supervisor(
                Arc::new(WireDrainFactory {
                    pcm: pcm.clone(),
                    receipt: receipt.clone(),
                    exit: exit.clone(),
                    stop_seen,
                    terminal,
                }),
                Arc::new(RecordingSink::default()),
                SupervisorConfig::default(),
            );
            let owner = AudioOwner::Dictation(704);
            start(&supervisor, owner).recv().unwrap().unwrap();
            wait_until("wire ready", || {
                supervisor.public.phase() == PublicPhase::Recording
            });
            let (response, result) = mpsc::channel();
            supervisor
                .sender
                .send(SupervisorMessage::Stop {
                    owner: Some(owner),
                    trigger: DictationStopTrigger::Toggle,
                    response,
                })
                .unwrap();
            seen.recv_timeout(Duration::from_secs(1)).unwrap();
            assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            pcm.open();
            // FIFO barrier through supervisor: PCM without Stopped must not seal.
            assert_eq!(
                start(&supervisor, AudioOwner::Query(705)).recv().unwrap(),
                Err(AudioStartError::AudioRecovering)
            );
            assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            receipt.open();
            if complete {
                let samples = result
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap()
                    .unwrap();
                assert_eq!(&samples[..160], &[0.25; 160]);
                assert_eq!(&samples[160..], &[0.75; 80]);
            } else {
                // A count mismatch or native-stop Failure never claims a seal.
                assert!(result.recv_timeout(Duration::from_millis(30)).is_err());
            }
            assert!(supervisor.public.owns(owner));
            exit.open();
            if !complete {
                assert_eq!(
                    result
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap()
                        .unwrap()
                        .len(),
                    240
                );
            }
            wait_until("wire joined", || {
                supervisor.public.phase() == PublicPhase::Idle
            });
            shutdown(&supervisor);
        }
    }

    struct SealedFactory {
        teardown: Gate,
        seal: bool,
    }

    impl WorkerFactory for SealedFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            events: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            let teardown = self.teardown.clone();
            let seal = self.seal;
            Ok(std::thread::spawn(move || {
                spec.shared
                    .lock()
                    .unwrap()
                    .append_pcm(&[0.25; 160], WHISPER_SAMPLE_RATE);
                events
                    .send(AudioWorkerEvent::FirstBuffer {
                        owner: spec.owner,
                        sample_rate: WHISPER_SAMPLE_RATE,
                        sent_at: std::time::Instant::now(),
                    })
                    .unwrap();
                spec.command_receiver.recv().unwrap();
                if seal {
                    events
                        .send(AudioWorkerEvent::RecordingSealed { owner: spec.owner })
                        .unwrap();
                }
                teardown.wait();
                events
                    .send(AudioWorkerEvent::ThreadExited { owner: spec.owner })
                    .unwrap();
            }))
        }
    }

    #[test]
    fn sealed_delivery_overlaps_exit_but_retains_exclusive_owner_and_processing() {
        for seal in [false, true] {
            let teardown = Gate::closed();
            let sink = Arc::new(RecordingSink::default());
            let supervisor = spawn_supervisor(
                Arc::new(SealedFactory {
                    teardown: teardown.clone(),
                    seal,
                }),
                sink.clone(),
                SupervisorConfig::default(),
            );
            let owner = AudioOwner::Dictation(700);
            start(&supervisor, owner).recv().unwrap().unwrap();
            wait_until("sealed worker ready", || {
                supervisor.public.phase() == PublicPhase::Recording
            });
            let (response, result) = mpsc::channel();
            supervisor
                .sender
                .send(SupervisorMessage::Stop {
                    owner: Some(owner),
                    trigger: DictationStopTrigger::Toggle,
                    response,
                })
                .unwrap();
            wait_until("sealed worker stopping", || {
                supervisor.public.phase() == PublicPhase::Stopping
            });
            // A stale generation cannot release samples or ownership.
            supervisor
                .sender
                .send(SupervisorMessage::Worker(
                    AudioWorkerEvent::RecordingSealed {
                        owner: AudioOwner::Dictation(699),
                    },
                ))
                .unwrap();
            if seal {
                assert_eq!(
                    result
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap()
                        .unwrap(),
                    vec![0.25; 160]
                );
            } else {
                assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            }
            assert!(supervisor.public.is_active());
            assert!(supervisor.public.owns(owner));
            for next in [
                AudioOwner::Dictation(701),
                AudioOwner::Transform(701),
                AudioOwner::Query(701),
                AudioOwner::Preview(701),
            ] {
                assert_eq!(
                    start(&supervisor, next).recv().unwrap(),
                    Err(AudioStartError::AudioRecovering)
                );
            }
            if !seal {
                // The start replies above are also a supervisor FIFO barrier:
                // the stale seal has now been processed and still delivered nothing.
                assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
            }
            if seal {
                // Even an unconfirmed exit must keep ownership without resetting
                // the already running delivery pipeline or publishing Idle.
                supervisor
                    .sender
                    .send(SupervisorMessage::Worker(
                        AudioWorkerEvent::TerminationUnconfirmed {
                            owner,
                            failure: AudioFailure::new(
                                AudioFailureKind::TerminationUnconfirmed,
                                AudioInitPhase::Runtime,
                            ),
                        },
                    ))
                    .unwrap();
                // A round trip proves unconfirmed exit publishes guidance
                // immediately, independently of the long-stop deadline.
                assert_eq!(
                    start(&supervisor, AudioOwner::Query(706)).recv().unwrap(),
                    Err(AudioStartError::AudioRecovering)
                );
                assert_eq!(supervisor.public.phase(), PublicPhase::Recovering);
                assert!(sink
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(event_owner, event)| {
                        *event_owner == owner
                            && *event
                                == AudioLifecycleEvent::StartupDiagnostic(
                                    audio::AudioStartupDiagnostic::WorkerInvariantViolation,
                                )
                    }));
                assert!(sink
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, event)| *event == AudioLifecycleEvent::RecoveryStalled));
                check_deadlines(
                    &supervisor,
                    RECOVERY_GUIDANCE_AFTER + Duration::from_secs(1),
                );
                assert!(supervisor.public.owns(owner));
                assert_eq!(supervisor.public.phase(), PublicPhase::Recovering);
                assert_eq!(
                    sink.events
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(_, event)| *event == AudioLifecycleEvent::RecoveryStalled)
                        .count(),
                    1
                );
                assert!(!sink
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, event)| matches!(
                        event,
                        AudioLifecycleEvent::Idle | AudioLifecycleEvent::Recovering { .. }
                    )));
            }
            teardown.open();
            if !seal {
                assert_eq!(
                    result
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap()
                        .unwrap(),
                    vec![0.25; 160]
                );
            }
            wait_until("sealed worker joined", || {
                supervisor.public.phase() == PublicPhase::Idle
            });
            assert!(!supervisor.public.owns(owner));
            assert_eq!(
                sink.events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(_, event)| *event == AudioLifecycleEvent::CleanupFinished)
                    .count(),
                usize::from(seal)
            );
            assert!(!sink
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::Idle));
            shutdown(&supervisor);
        }
    }

    struct CommandCleanupSink {
        app_state: Arc<crate::state::AppState>,
        completed: AtomicUsize,
    }

    impl LifecycleSink for CommandCleanupSink {
        fn invalidate_input_health(&self, _: Option<&str>) {}
        fn notify(
            &self,
            _: Option<&tauri::AppHandle>,
            owner: AudioOwner,
            event: AudioLifecycleEvent,
        ) {
            match event {
                AudioLifecycleEvent::CleanupFinished => {
                    // Exercise the same generation/status guard as the real
                    // recording command, without starting a native app.
                    crate::commands::recording::complete_dictation_capture_cleanup(
                        &self.app_state,
                        owner.telemetry_id(),
                        |restore_idle| {
                            assert!(restore_idle);
                            self.completed.fetch_add(1, Ordering::SeqCst);
                        },
                    );
                }
                AudioLifecycleEvent::RecoveryStalled => {
                    panic!("normal exit must not need stall guidance")
                }
                _ => {}
            }
        }
    }

    #[test]
    fn retry_during_sealed_cleanup_then_normal_exit_allows_the_next_start() {
        use crate::state::{AppState, DictationStatus};
        let app_state = Arc::new(AppState::default());
        let owner = AudioOwner::Dictation(707);
        app_state
            .recording_id
            .store(owner.telemetry_id(), Ordering::SeqCst);
        let sink = Arc::new(CommandCleanupSink {
            app_state: app_state.clone(),
            completed: AtomicUsize::new(0),
        });
        let teardown = Gate::closed();
        let supervisor = spawn_supervisor(
            Arc::new(SealedFactory {
                teardown: teardown.clone(),
                seal: true,
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        start(&supervisor, owner).recv().unwrap().unwrap();
        wait_until("normal seal worker ready", || {
            supervisor.public.phase() == PublicPhase::Recording
        });
        let (response, result) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                owner: Some(owner),
                trigger: DictationStopTrigger::Toggle,
                response,
            })
            .unwrap();
        assert_eq!(
            result
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap(),
            vec![0.25; 160]
        );
        // ASR has finished, but the capture thread still owns cleanup.
        assert_eq!(
            app_state.dictation.lock().unwrap().status,
            DictationStatus::Idle
        );
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(708))
                .recv()
                .unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        // The native retry path keeps this generation and marks it Recovering.
        app_state.dictation.lock().unwrap().status = DictationStatus::Recovering;
        assert_eq!(sink.completed.load(Ordering::SeqCst), 0);
        teardown.open();
        wait_until("normal cleanup did not publish completion", || {
            sink.completed.load(Ordering::SeqCst) == 1
        });
        assert_eq!(
            app_state.dictation.lock().unwrap().status,
            DictationStatus::Idle
        );
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(708))
                .recv()
                .unwrap(),
            Ok(())
        );
        wait_until("next recording did not become ready", || {
            supervisor.public.phase() == PublicPhase::Recording
        });
        cancel_any_phase(&supervisor, AudioOwner::Dictation(708))
            .recv()
            .unwrap()
            .unwrap();
        wait_until("next worker did not exit", || {
            supervisor.public.phase() == PublicPhase::Idle
        });
        shutdown(&supervisor);
    }

    struct RuntimeFailureFactory;

    struct OrderedReadyFactory {
        first_buffer_enqueued: Sender<()>,
        release_spawn: Gate,
        samples: Vec<f32>,
    }

    impl WorkerFactory for OrderedReadyFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            let first_buffer_enqueued = self.first_buffer_enqueued.clone();
            let samples = self.samples.clone();
            let handle = std::thread::spawn(move || {
                let owner = spec.owner;
                spec.shared.lock().unwrap().extend_from_slice(&samples);
                event_sender
                    .send(AudioWorkerEvent::FirstBuffer {
                        owner,
                        sample_rate: WHISPER_SAMPLE_RATE,
                        sent_at: std::time::Instant::now(),
                    })
                    .unwrap();
                first_buffer_enqueued.send(()).unwrap();
                let _ = spec.command_receiver.recv();
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            });
            // Keep the supervisor inside Start while the worker appends PCM and
            // enqueues readiness. Tests can then enqueue stop/deadline messages
            // behind readiness before allowing Start to return.
            self.release_spawn.wait();
            Ok(handle)
        }
    }

    impl WorkerFactory for RuntimeFailureFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                spec.shared.lock().unwrap().push(0.5);
                let _ = event_sender.send(AudioWorkerEvent::FirstBuffer {
                    owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    sent_at: std::time::Instant::now(),
                });
                let _ = event_sender.send(AudioWorkerEvent::RuntimeFailed {
                    owner,
                    failure: AudioFailure::new(
                        AudioFailureKind::StreamInvalidated,
                        AudioInitPhase::Runtime,
                    ),
                });
                let _ = spec.command_receiver.recv();
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            }))
        }
    }

    type CapturedSpecs = Arc<Mutex<Vec<(AudioOwner, Option<String>)>>>;

    struct SpecCaptureFactory {
        specs: CapturedSpecs,
    }

    impl WorkerFactory for SpecCaptureFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            self.specs
                .lock()
                .unwrap()
                .push((spec.owner, spec.device_id.clone()));
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                let _ = spec.command_receiver.recv();
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            }))
        }
    }

    impl WorkerFactory for BlockingTeardownFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            let gate = self.gate.clone();
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                spec.shared.lock().unwrap().push(0.25);
                let _ = event_sender.send(AudioWorkerEvent::FirstBuffer {
                    owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    sent_at: std::time::Instant::now(),
                });
                let _ = spec.command_receiver.recv();
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                gate.wait();
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            }))
        }
    }

    impl WorkerFactory for BlockingFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: AudioWorkerEventSender,
        ) -> Result<JoinHandle<()>, String> {
            let spawn_index = self.spawn_count.fetch_add(1, Ordering::SeqCst);
            self.active_flags
                .lock()
                .unwrap()
                .push(Arc::clone(&spec.active));
            let gate = if spawn_index == 0 {
                self.gate.clone()
            } else {
                self.retry_gate.clone().unwrap_or_else(|| self.gate.clone())
            };
            let phase = self.phase;
            let phase_entered = self.phase_entered.clone();
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
                if let Some(phase_entered) = phase_entered {
                    let _ = phase_entered.send(());
                }
                gate.wait();
                let _ = event_sender.send(AudioWorkerEvent::PhaseExited {
                    owner,
                    phase,
                    elapsed_ms: 1,
                });
                spec.shared.lock().unwrap().push(0.25);
                let _ = event_sender.send(AudioWorkerEvent::FirstBuffer {
                    owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    sent_at: std::time::Instant::now(),
                });
                while let Ok(command) = spec.command_receiver.recv() {
                    if matches!(command, AudioCommand::Stop) {
                        break;
                    }
                }
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
            }))
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<(AudioOwner, AudioLifecycleEvent)>>,
        invalidated_inputs: Mutex<Vec<Option<String>>>,
    }

    impl LifecycleSink for RecordingSink {
        fn invalidate_input_health(&self, device_id: Option<&str>) {
            self.invalidated_inputs
                .lock()
                .unwrap()
                .push(device_id.map(str::to_string));
        }

        fn notify(
            &self,
            _app_handle: Option<&tauri::AppHandle>,
            owner: AudioOwner,
            event: AudioLifecycleEvent,
        ) {
            self.events.lock().unwrap().push((owner, event));
        }
    }

    type ActiveFlags = Arc<Mutex<Vec<Arc<AtomicBool>>>>;
    type Harness = (
        AudioSupervisor,
        Gate,
        Arc<AtomicUsize>,
        Arc<RecordingSink>,
        ActiveFlags,
    );

    fn harness(phase: AudioInitPhase, config: SupervisorConfig) -> Harness {
        let gate = Gate::closed();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(RecordingSink::default());
        let active_flags = Arc::new(Mutex::new(Vec::new()));
        let supervisor = spawn_supervisor(
            Arc::new(BlockingFactory {
                gate: gate.clone(),
                retry_gate: None,
                spawn_count: Arc::clone(&spawn_count),
                active_flags: Arc::clone(&active_flags),
                phase,
                phase_entered: None,
            }),
            sink.clone(),
            config,
        );
        (supervisor, gate, spawn_count, sink, active_flags)
    }

    fn start(
        supervisor: &AudioSupervisor,
        owner: AudioOwner,
    ) -> Receiver<Result<(), AudioStartError>> {
        start_with_device(supervisor, owner, None)
    }

    fn start_with_device(
        supervisor: &AudioSupervisor,
        owner: AudioOwner,
        device_id: Option<String>,
    ) -> Receiver<Result<(), AudioStartError>> {
        let (sender, receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Start(StartRequest {
                gesture: gesture::StartGesture::default(),
                caller_alive: Arc::new(AtomicBool::new(true)),
                owner,
                app_handle: None,
                device_id,
                origin: "hold".to_string(),
                wait_until_ready: false,
                response: sender,
                probe_permit: None,
            }))
            .unwrap();
        receiver
    }

    fn cancel(supervisor: &AudioSupervisor, owner: AudioOwner) -> Receiver<Result<bool, String>> {
        let (sender, receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Cancel {
                owner: Some(owner),
                reason: AudioCancelReason::User,
                starting_only: true,
                response: sender,
            })
            .unwrap();
        receiver
    }

    fn cancel_any_phase(
        supervisor: &AudioSupervisor,
        owner: AudioOwner,
    ) -> Receiver<Result<bool, String>> {
        let (sender, receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Cancel {
                owner: Some(owner),
                reason: AudioCancelReason::User,
                starting_only: false,
                response: sender,
            })
            .unwrap();
        receiver
    }

    fn shutdown(supervisor: &AudioSupervisor) {
        let (sender, receiver) = mpsc::channel();
        let _ = supervisor.sender.send(SupervisorMessage::Shutdown(sender));
        let _ = receiver.recv_timeout(Duration::from_secs(1));
    }

    fn check_deadlines(supervisor: &AudioSupervisor, elapsed: Duration) {
        let (sender, receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::CheckDeadlines {
                elapsed,
                response: sender,
            })
            .unwrap();
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    fn wait_until(message: &str, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !predicate() {
            assert!(Instant::now() < deadline, "{message}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    mod capture_standby_tests {
        include!("capture_standby_tests.rs");
    }

    mod stop_diagnostics_tests {
        include!("capture_stop_diagnostics_tests.rs");
    }

    #[test]
    fn start_returns_promptly_and_second_start_never_spawns() {
        let (supervisor, gate, spawn_count, _, _) = harness(
            AudioInitPhase::DeviceEnumeration,
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(1);
        assert_eq!(
            start(&supervisor, owner)
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(())
        );
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(2))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Err(AudioStartError::AlreadyStarting)
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            cancel(&supervisor, owner)
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(true)
        );
        gate.open();
        wait_until("cancelled worker did not exit", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn failed_worker_spawn_releases_prepublished_ownership() {
        let supervisor = spawn_supervisor(
            Arc::new(FailingFactory),
            Arc::new(RecordingSink::default()),
            SupervisorConfig::default(),
        );

        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(2)).recv().unwrap(),
            Err(AudioStartError::SpawnFailed(
                "test spawn failure".to_string()
            ))
        );
        assert_eq!(supervisor.public.phase(), PublicPhase::Idle);
        assert!(!supervisor.public.is_active());
        shutdown(&supervisor);
    }

    #[test]
    fn cancel_retains_owner_until_exit_and_late_buffer_never_becomes_recording() {
        for phase in [
            AudioInitPhase::DeviceEnumeration,
            AudioInitPhase::StreamBuild,
            AudioInitPhase::FirstBufferWait,
        ] {
            let (supervisor, gate, _, sink, active_flags) =
                harness(phase, SupervisorConfig::default());
            let owner = AudioOwner::Dictation(3);
            assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
            let active = Arc::clone(&active_flags.lock().unwrap()[0]);
            assert!(!active.load(Ordering::SeqCst));
            assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
            assert!(
                supervisor.public.is_active(),
                "recovery must retain exclusive ownership until the worker exits"
            );
            gate.open();
            wait_until("recovering worker did not exit", || {
                !supervisor.public.is_active()
            });
            assert!(
                !active.load(Ordering::SeqCst),
                "recovering attempts must never publish levels"
            );
            let events = sink.events.lock().unwrap().clone();
            assert!(events.iter().any(|(_, event)| matches!(
                event,
                AudioLifecycleEvent::Recovering {
                    reason: AudioCancelReason::User
                }
            )));
            assert!(!events
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::Ready));
            assert!(events
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::Idle));
            assert!(!supervisor.public.is_active());
            shutdown(&supervisor);
        }
    }

    #[test]
    fn cancelled_hang_rejects_retry_until_owned_worker_exits() {
        let (supervisor, gate, spawn_count, _, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let first = AudioOwner::Dictation(30);
        assert_eq!(start(&supervisor, first).recv().unwrap(), Ok(()));

        assert_eq!(cancel(&supervisor, first).recv().unwrap(), Ok(true));
        assert!(supervisor.public.is_active());

        let second = AudioOwner::Dictation(31);
        assert_eq!(
            start(&supervisor, second).recv().unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);

        gate.open();
        wait_until("owned worker did not exit", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn hard_deadline_reports_failure_once_and_blocks_retry_until_exit() {
        let gate = Gate::closed();
        let retry_gate = Gate::closed();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(RecordingSink::default());
        let active_flags = Arc::new(Mutex::new(Vec::new()));
        let (phase_sender, phase_receiver) = mpsc::channel();
        let supervisor = spawn_supervisor(
            Arc::new(BlockingFactory {
                gate: gate.clone(),
                retry_gate: Some(retry_gate.clone()),
                spawn_count: Arc::clone(&spawn_count),
                active_flags,
                phase: AudioInitPhase::StreamBuild,
                phase_entered: Some(phase_sender),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(4);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        phase_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker never entered config lookup");
        check_deadlines(
            &supervisor,
            HARD_INITIALIZATION_DEADLINE + Duration::from_secs(1),
        );
        assert!(supervisor.public.is_active());
        {
            let events = sink.events.lock().unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|(_, event)| matches!(
                        event,
                        AudioLifecycleEvent::InitializationFailed { .. }
                    ))
                    .count(),
                1
            );
            assert!(events
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::StillConnecting));
            assert!(events.iter().any(|(_, event)| matches!(
                event,
                AudioLifecycleEvent::InitializationFailed {
                    kind: AudioFailureKind::InitializationTimeout,
                    ..
                }
            )));
        }
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);
        gate.open();
        wait_until("deadline-expired worker did not exit", || {
            !supervisor.public.is_active()
        });
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(())
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
        phase_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("retry worker never entered config lookup");
        assert_eq!(
            cancel(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(true)
        );
        retry_gate.open();
        wait_until("retry worker did not exit", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn complete_teardown_allows_a_successful_retry() {
        let (supervisor, gate, spawn_count, _, _) =
            harness(AudioInitPhase::FirstBufferWait, SupervisorConfig::default());
        let first = AudioOwner::Dictation(6);
        assert_eq!(start(&supervisor, first).recv().unwrap(), Ok(()));
        assert_eq!(cancel(&supervisor, first).recv().unwrap(), Ok(true));
        gate.open();
        wait_until("first attempt did not finish teardown", || {
            !supervisor.public.is_active()
        });

        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(7))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(())
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            cancel_any_phase(&supervisor, AudioOwner::Dictation(7))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(true)
        );
        wait_until("retry attempt did not finish teardown", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn stop_is_prompt_while_initialization_is_blocked() {
        let (supervisor, gate, _, _, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(8);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        let (response_sender, response_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: response_sender,
            })
            .unwrap();
        assert_eq!(
            response_receiver
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(Vec::new())
        );
        assert!(supervisor.public.is_active());
        gate.open();
        wait_until("stopped initializing worker did not exit", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn query_peek_clones_samples_without_stopping_capture() {
        let (supervisor, gate, _, sink, _) =
            harness(AudioInitPhase::FirstBufferWait, SupervisorConfig::default());
        let owner = AudioOwner::Query(56);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));

        let (peek_sender, peek_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Peek {
                owner,
                max_output_samples: None,
                response: peek_sender,
            })
            .unwrap();
        assert_eq!(
            peek_receiver
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            None,
            "starting capture must not expose a buffer"
        );

        gate.open();
        wait_until("query capture did not become ready", || {
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::Ready)
        });

        let (peek_sender, peek_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Peek {
                owner,
                max_output_samples: None,
                response: peek_sender,
            })
            .unwrap();
        assert_eq!(
            peek_receiver
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Some(vec![0.25])
        );
        assert!(supervisor.public.is_active());

        let (stale_sender, stale_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Peek {
                owner: AudioOwner::Query(99),
                max_output_samples: None,
                response: stale_sender,
            })
            .unwrap();
        assert_eq!(
            stale_receiver
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            None
        );
        assert!(supervisor.public.is_active());

        let (stop_sender, stop_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: stop_sender,
            })
            .unwrap();
        assert_eq!(
            stop_receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
            Ok(vec![0.25])
        );
        wait_until("query peek stop did not finish teardown", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn stale_worker_generation_cannot_activate_current_attempt() {
        let (supervisor, gate, _, sink, active_flags) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(9);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::FirstBuffer {
                owner: AudioOwner::Dictation(8),
                sample_rate: WHISPER_SAMPLE_RATE,
                sent_at: std::time::Instant::now(),
            }))
            .unwrap();
        // The cancel response is a FIFO barrier proving the stale Ready event
        // was handled before these assertions.
        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        assert!(!active_flags.lock().unwrap()[0].load(Ordering::SeqCst));
        let invariant = AudioLifecycleEvent::StartupDiagnostic(
            crate::audio::AudioStartupDiagnostic::WorkerInvariantViolation,
        );
        let observed = sink.events.lock().unwrap();
        assert!(observed.contains(&(AudioOwner::Dictation(8), invariant.clone())));
        assert!(!observed.contains(&(owner, invariant)));
        drop(observed);

        assert!(!sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| *event == AudioLifecycleEvent::Ready));
        gate.open();
        wait_until("stale-generation attempt did not finish", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn empty_first_buffer_event_cannot_enter_recording() {
        let (supervisor, gate, _, sink, active_flags) =
            harness(AudioInitPhase::FirstBufferWait, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(10);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::FirstBuffer {
                owner,
                sample_rate: WHISPER_SAMPLE_RATE,
                sent_at: std::time::Instant::now(),
            }))
            .unwrap();
        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        assert!(!active_flags.lock().unwrap()[0].load(Ordering::SeqCst));
        assert!(!sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| *event == AudioLifecycleEvent::Ready));
        gate.open();
        wait_until("empty-buffer attempt did not exit", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn preview_readiness_does_not_require_retained_pcm() {
        let (supervisor, gate, _, sink, _) =
            harness(AudioInitPhase::FirstBufferWait, SupervisorConfig::default());
        let owner = AudioOwner::Preview(11);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::FirstBuffer {
                owner,
                sample_rate: WHISPER_SAMPLE_RATE,
                sent_at: std::time::Instant::now(),
            }))
            .unwrap();
        let diagnostic =
            AudioLifecycleEvent::StartupDiagnostic(audio::AudioStartupDiagnostic::PhaseDuration {
                phase: AudioInitPhase::FirstBufferWait,
                elapsed_ms: 7,
            });
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::PhaseExited {
                owner,
                phase: AudioInitPhase::FirstBufferWait,
                elapsed_ms: 7,
            }))
            .unwrap();
        supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::StartupDiagnostic {
                    owner,
                    diagnostic: audio::AudioStartupDiagnostic::PhaseDuration {
                        phase: AudioInitPhase::FirstBufferWait,
                        elapsed_ms: 7,
                    },
                },
            ))
            .unwrap();
        assert_eq!(
            cancel_any_phase(&supervisor, owner).recv().unwrap(),
            Ok(true)
        );
        let events = sink.events.lock().unwrap();
        let ready_index = events
            .iter()
            .position(|(o, event)| *o == owner && *event == AudioLifecycleEvent::Ready)
            .unwrap();
        let diagnostic_index = events
            .iter()
            .position(|(o, event)| *o == owner && *event == diagnostic)
            .unwrap();
        assert!(ready_index < diagnostic_index);
        drop(events);
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(event_owner, event)| *event_owner == owner
                && *event == AudioLifecycleEvent::Ready));
        gate.open();
        wait_until("preview worker did not finish teardown", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn preview_stop_while_connecting_emits_idle_only_after_worker_exit() {
        let (supervisor, gate, _, sink, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let owner = AudioOwner::Preview(12);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        let (response_sender, response_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: response_sender,
            })
            .unwrap();
        assert_eq!(response_receiver.recv().unwrap(), Ok(Vec::new()));
        assert!(supervisor.public.is_active());
        assert!(!sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(
                |(event_owner, event)| *event_owner == owner && *event == AudioLifecycleEvent::Idle
            ));
        gate.open();
        wait_until("stopped preview worker did not exit", || {
            !supervisor.public.is_active()
        });
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(
                |(event_owner, event)| *event_owner == owner && *event == AudioLifecycleEvent::Idle
            ));
        shutdown(&supervisor);
    }

    #[test]
    fn first_buffer_enqueued_before_stop_returns_retained_pcm_exactly_once() {
        let release_spawn = Gate::closed();
        let (first_buffer_sender, first_buffer_receiver) = mpsc::channel();
        let expected = vec![0.25, -0.5, 0.75];
        let supervisor = spawn_supervisor(
            Arc::new(OrderedReadyFactory {
                first_buffer_enqueued: first_buffer_sender,
                release_spawn: release_spawn.clone(),
                samples: expected.clone(),
            }),
            Arc::new(RecordingSink::default()),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(70);
        let start_response = start(&supervisor, owner);
        first_buffer_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker did not append and enqueue its first buffer");
        assert_eq!(
            supervisor.public.phase(),
            PublicPhase::Starting,
            "capture ownership must publish before worker spawn returns"
        );

        let (stop_sender, stop_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: stop_sender,
            })
            .unwrap();
        release_spawn.open();

        assert_eq!(start_response.recv().unwrap(), Ok(()));
        assert_eq!(stop_receiver.recv().unwrap(), Ok(expected));
        wait_until("stopped worker did not finish", || {
            !supervisor.public.is_active()
        });

        let (second_stop_sender, second_stop_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: second_stop_sender,
            })
            .unwrap();
        assert_eq!(second_stop_receiver.recv().unwrap(), Ok(Vec::new()));
        shutdown(&supervisor);
    }

    #[test]
    fn first_buffer_enqueued_before_deadline_wins_the_deadline_race() {
        let release_spawn = Gate::closed();
        let (first_buffer_sender, first_buffer_receiver) = mpsc::channel();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(OrderedReadyFactory {
                first_buffer_enqueued: first_buffer_sender,
                release_spawn: release_spawn.clone(),
                samples: vec![0.5],
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(71);
        let start_response = start(&supervisor, owner);
        first_buffer_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker did not append and enqueue its first buffer");

        let (deadline_sender, deadline_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::CheckDeadlines {
                elapsed: HARD_INITIALIZATION_DEADLINE + Duration::from_secs(1),
                response: deadline_sender,
            })
            .unwrap();
        release_spawn.open();

        assert_eq!(start_response.recv().unwrap(), Ok(()));
        deadline_receiver.recv().unwrap();
        assert!(supervisor.public.is_recording_for(owner));
        assert!(!sink.events.lock().unwrap().iter().any(|(_, event)| {
            matches!(event, AudioLifecycleEvent::InitializationFailed { .. })
        }));

        let (stop_sender, stop_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: stop_sender,
            })
            .unwrap();
        assert_eq!(stop_receiver.recv().unwrap(), Ok(vec![0.5]));
        wait_until("deadline-race worker did not finish", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn environment_change_cancels_starting_with_its_reason() {
        for (index, reason) in [
            AudioCancelReason::DeviceChanged,
            AudioCancelReason::SystemSleep,
            AudioCancelReason::SystemWake,
        ]
        .into_iter()
        .enumerate()
        {
            let (supervisor, gate, _, sink, _) = harness(
                AudioInitPhase::DeviceEnumeration,
                SupervisorConfig::default(),
            );
            let owner = AudioOwner::Dictation(10 + index as u64);
            assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
            let (response_sender, response_receiver) = mpsc::channel();
            supervisor
                .sender
                .send(SupervisorMessage::Cancel {
                    owner: Some(owner),
                    reason,
                    starting_only: true,
                    response: response_sender,
                })
                .unwrap();
            assert_eq!(response_receiver.recv().unwrap(), Ok(true));
            assert!(sink.events.lock().unwrap().iter().any(|(_, event)| {
                matches!(
                    event,
                    AudioLifecycleEvent::Recovering {
                        reason: event_reason
                    } if *event_reason == reason
                )
            }));
            gate.open();
            wait_until("environment-cancelled worker did not exit", || {
                !supervisor.public.is_active()
            });
            shutdown(&supervisor);
        }
    }

    #[test]
    fn first_buffer_wait_without_callback_fails_as_timeout() {
        let sink = Arc::new(RecordingSink::default());
        let (phase_sender, phase_receiver) = mpsc::channel();
        let supervisor = spawn_supervisor(
            Arc::new(NoCallbackFactory {
                first_buffer_wait_entered: phase_sender,
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(80);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        phase_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker never entered the explicit first-buffer wait phase");
        check_deadlines(
            &supervisor,
            HARD_INITIALIZATION_DEADLINE + Duration::from_secs(1),
        );
        assert!(sink.events.lock().unwrap().iter().any(|(_, event)| {
            matches!(
                event,
                AudioLifecycleEvent::InitializationFailed {
                    kind: AudioFailureKind::FirstBufferTimeout,
                    ..
                }
            )
        }));
        assert!(
            !sink
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::Ready),
            "first-buffer wait cannot become Recording without retained PCM"
        );
        wait_until("no-callback worker did not exit after timeout", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn runtime_cpal_failure_reaches_supervisor_as_content_free_kind() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(RuntimeFailureFactory),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Transform(81);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        wait_until("runtime failure did not reach lifecycle sink", || {
            sink.events.lock().unwrap().iter().any(|(_, event)| {
                matches!(
                    event,
                    AudioLifecycleEvent::InitializationFailed {
                        kind: AudioFailureKind::StreamInvalidated,
                        ..
                    }
                )
            })
        });
        wait_until("runtime-failed worker did not exit", || {
            !supervisor.public.is_active()
        });
        assert_eq!(
            sink.events
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, event)| {
                    matches!(
                        event,
                        AudioLifecycleEvent::InitializationFailed {
                            kind: AudioFailureKind::StreamInvalidated,
                            ..
                        }
                    )
                })
                .count(),
            1,
            "transform runtime failure must be reported exactly once"
        );
        shutdown(&supervisor);
    }

    #[test]
    fn preview_runtime_failure_reaches_supervisor_as_typed_error() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(RuntimeFailureFactory),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Preview(82);
        assert_eq!(
            start_with_device(
                &supervisor,
                owner,
                Some("verified-preview-input".to_string())
            )
            .recv()
            .unwrap(),
            Ok(())
        );
        wait_until(
            "preview runtime failure did not reach lifecycle sink",
            || {
                sink.events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(event_owner, event)| {
                        *event_owner == owner
                            && matches!(
                                event,
                                AudioLifecycleEvent::InitializationFailed {
                                    kind: AudioFailureKind::StreamInvalidated,
                                    ..
                                }
                            )
                    })
            },
        );
        wait_until("runtime-failed preview worker did not exit", || {
            !supervisor.public.is_active()
        });
        let invalidations = sink.invalidated_inputs.lock().unwrap();
        assert!(!invalidations.is_empty());
        assert!(invalidations
            .iter()
            .all(|id| id.as_deref() == Some("verified-preview-input")));
        drop(invalidations);
        shutdown(&supervisor);
    }

    #[test]
    fn stale_failure_and_user_cancel_preserve_input_health() {
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(SpecCaptureFactory {
                specs: Arc::new(Mutex::new(Vec::new())),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Preview(800);
        assert_eq!(
            start_with_device(&supervisor, owner, Some("current-input".to_string()))
                .recv()
                .unwrap(),
            Ok(())
        );
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::InitFailed {
                owner: AudioOwner::Preview(799),
                failure: AudioFailure::new(
                    AudioFailureKind::FirstBufferTimeout,
                    AudioInitPhase::FirstBufferWait,
                ),
            }))
            .unwrap();
        check_deadlines(&supervisor, Duration::ZERO);
        assert!(sink.invalidated_inputs.lock().unwrap().is_empty());
        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        wait_until("cancelled preview did not close", || {
            !supervisor.public.is_active()
        });
        assert!(sink.invalidated_inputs.lock().unwrap().is_empty());
        shutdown(&supervisor);
    }

    #[test]
    fn first_pcm_deadline_revokes_only_the_frozen_input_and_keeps_ownership() {
        let gate = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let (phase_sender, phase_receiver) = mpsc::channel();
        let supervisor = spawn_supervisor(
            Arc::new(BlockingFactory {
                gate: gate.clone(),
                retry_gate: None,
                spawn_count: Arc::new(AtomicUsize::new(0)),
                active_flags: Arc::new(Mutex::new(Vec::new())),
                phase: AudioInitPhase::FirstBufferWait,
                phase_entered: Some(phase_sender),
            }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(801);
        assert_eq!(
            start_with_device(&supervisor, owner, Some("frozen-input".to_string()))
                .recv()
                .unwrap(),
            Ok(())
        );
        phase_receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        check_deadlines(
            &supervisor,
            HARD_INITIALIZATION_DEADLINE + Duration::from_secs(1),
        );
        assert!(supervisor.public.is_active());
        let invalidations = sink.invalidated_inputs.lock().unwrap();
        assert!(!invalidations.is_empty());
        assert!(invalidations
            .iter()
            .all(|id| id.as_deref() == Some("frozen-input")));
        drop(invalidations);
        assert_eq!(
            start_with_device(
                &supervisor,
                AudioOwner::Dictation(802),
                Some("replacement".to_string())
            )
            .recv()
            .unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        gate.open();
        wait_until("timed-out worker did not close", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn dictation_and_transform_share_system_default_and_explicit_id_contracts() {
        let specs = Arc::new(Mutex::new(Vec::new()));
        let supervisor = spawn_supervisor(
            Arc::new(SpecCaptureFactory {
                specs: Arc::clone(&specs),
            }),
            Arc::new(RecordingSink::default()),
            SupervisorConfig::default(),
        );
        for (owner, device_id) in [
            (AudioOwner::Dictation(90), None),
            (AudioOwner::Transform(90), None),
            (
                AudioOwner::Dictation(91),
                Some("raw-coreaudio-uid".to_string()),
            ),
            (
                AudioOwner::Transform(91),
                Some("raw-coreaudio-uid".to_string()),
            ),
        ] {
            assert_eq!(
                start_with_device(&supervisor, owner, device_id)
                    .recv()
                    .unwrap(),
                Ok(())
            );
            assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
            wait_until("spec-capture worker did not exit", || {
                !supervisor.public.is_active()
            });
        }
        assert_eq!(
            *specs.lock().unwrap(),
            vec![
                (AudioOwner::Dictation(90), None),
                (AudioOwner::Transform(90), None),
                (
                    AudioOwner::Dictation(91),
                    Some("raw-coreaudio-uid".to_string())
                ),
                (
                    AudioOwner::Transform(91),
                    Some("raw-coreaudio-uid".to_string())
                ),
            ]
        );
        shutdown(&supervisor);
    }

    #[test]
    fn recording_duration_uses_readiness_not_attempt_acceptance() {
        let now = Instant::now();
        let accepted_at = now - Duration::from_secs(12);
        let ready_at = now - Duration::from_secs(2);
        assert_eq!(
            recording_duration_since_ready(Some(ready_at), now),
            Duration::from_secs(2)
        );
        assert_ne!(
            recording_duration_since_ready(Some(ready_at), now),
            now.saturating_duration_since(accepted_at)
        );
    }

    #[test]
    fn bounded_snapshot_keeps_only_the_requested_asr_tail() {
        let shared: Vec<f32> = (0..96_000).map(|value| value as f32).collect();
        let snapshot = snapshot_for_asr(&shared, 48_000, Some(16_000));

        assert_eq!(snapshot.len(), 16_000);
        assert!(snapshot[0] > 47_000.0, "snapshot must come from the tail");
        assert_eq!(
            snapshot_for_asr(&shared, 48_000, Some(0)),
            Vec::<f32>::new()
        );
    }

    #[test]
    fn pending_tcc_prompt_suspends_active_deadline_and_cancel_stays_responsive() {
        let (supervisor, gate, _, sink, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(96);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::PermissionPromptPending { owner },
            ))
            .unwrap();

        check_deadlines(
            &supervisor,
            HARD_INITIALIZATION_DEADLINE + Duration::from_secs(10),
        );
        assert!(supervisor.public.is_active());
        assert!(!sink.events.lock().unwrap().iter().any(|(_, event)| {
            matches!(event, AudioLifecycleEvent::InitializationFailed { .. })
        }));

        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        gate.open();
        wait_until("TCC-pending cancellation did not release ownership", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn pending_tcc_prompt_has_a_separate_bounded_watchdog() {
        let config = SupervisorConfig {
            tcc_prompt_watchdog: Duration::from_secs(3),
            ..SupervisorConfig::default()
        };
        let (supervisor, gate, _, sink, _) = harness(AudioInitPhase::StreamBuild, config);
        let owner = AudioOwner::Dictation(97);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::PermissionPromptPending { owner },
            ))
            .unwrap();

        check_deadlines(&supervisor, Duration::from_secs(4));
        assert!(sink.events.lock().unwrap().iter().any(|(_, event)| {
            matches!(
                event,
                AudioLifecycleEvent::InitializationFailed {
                    kind: AudioFailureKind::PermissionPromptTimeout,
                    ..
                }
            )
        }));
        assert!(supervisor.public.is_active());

        gate.open();
        wait_until("TCC watchdog recovery did not release ownership", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn unconfirmed_termination_fails_closed_and_retains_exclusive_ownership() {
        let (supervisor, gate, spawn_count, sink, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(98);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(
                AudioWorkerEvent::TerminationUnconfirmed {
                    owner,
                    failure: AudioFailure::new(
                        AudioFailureKind::TerminationUnconfirmed,
                        AudioInitPhase::StreamBuild,
                    ),
                },
            ))
            .unwrap();

        wait_until("unconfirmed termination did not enter recovery", || {
            sink.events.lock().unwrap().iter().any(|(_, event)| {
                matches!(
                    event,
                    AudioLifecycleEvent::InitializationFailed {
                        kind: AudioFailureKind::TerminationUnconfirmed,
                        ..
                    }
                )
            })
        });
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(99))
                .recv()
                .unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 1);

        gate.open();
        wait_until("unconfirmed termination recovery did not finish", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn stopping_deadline_surfaces_guidance_without_detaching_the_worker() {
        let gate = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(BlockingTeardownFactory { gate: gate.clone() }),
            sink.clone(),
            SupervisorConfig {
                still_connecting_after: Duration::from_secs(1),
                hard_deadline: Duration::from_secs(2),
                tcc_prompt_watchdog: Duration::from_secs(120),
                recovery_guidance_after: Duration::from_millis(5),
            },
        );
        let owner = AudioOwner::Dictation(99);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        wait_until("worker never reached recording", || {
            supervisor.public.is_recording_for(owner)
        });

        let (response_sender, response_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: response_sender,
            })
            .unwrap();
        wait_until("stalled stopping guidance was not emitted", || {
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, event)| *event == AudioLifecycleEvent::RecoveryStalled)
        });
        assert!(supervisor.public.is_active());
        assert!(response_receiver.try_recv().is_err());

        gate.open();
        assert_eq!(
            response_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Ok(vec![0.25])
        );
        wait_until("stopping worker was not joined", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }

    #[test]
    fn completed_teardown_notifies_idle_when_the_stop_caller_timed_out() {
        let gate = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(BlockingTeardownFactory { gate: gate.clone() }),
            sink.clone(),
            SupervisorConfig::default(),
        );
        let owner = AudioOwner::Dictation(99);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        wait_until("worker never reached recording", || {
            supervisor.public.is_recording_for(owner)
        });

        let (response_sender, response_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(owner),
                response: response_sender,
            })
            .unwrap();
        drop(response_receiver);

        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(100))
                .recv()
                .unwrap(),
            Err(AudioStartError::AudioRecovering),
            "the blocked worker must retain exclusive ownership"
        );

        gate.open();
        wait_until("joined teardown did not publish idle", || {
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|(event_owner, event)| {
                    *event_owner == owner && *event == AudioLifecycleEvent::Idle
                })
        });
        wait_until("stopping worker was not joined", || {
            !supervisor.public.is_active()
        });

        let retry_owner = AudioOwner::Dictation(101);
        assert_eq!(start(&supervisor, retry_owner).recv().unwrap(), Ok(()));
        wait_until("fresh owner never reached recording", || {
            supervisor.public.is_recording_for(retry_owner)
        });
        let (retry_stop_sender, retry_stop_receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Stop {
                trigger: DictationStopTrigger::Other,
                owner: Some(retry_owner),
                response: retry_stop_sender,
            })
            .unwrap();
        assert_eq!(retry_stop_receiver.recv().unwrap(), Ok(vec![0.25]));
        wait_until("retry worker was not joined", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }
}
