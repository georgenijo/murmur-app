use crate::audio::{self, AudioCommand, AudioWorkerEvent, AudioWorkerSpec};
use crate::state::WHISPER_SAMPLE_RATE;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
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
}

impl AudioOwner {
    pub(crate) fn telemetry_id(self) -> u64 {
        match self {
            Self::Dictation(id) | Self::Transform(id) => id,
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Dictation(_) => "dictation",
            Self::Transform(_) => "transform",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCancelReason {
    User,
    DeviceChanged,
    SystemSleep,
    SystemWake,
    HardDeadline,
}

impl AudioCancelReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::DeviceChanged => "device_changed",
            Self::SystemSleep => "system_sleep",
            Self::SystemWake => "system_wake",
            Self::HardDeadline => "hard_deadline",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioLifecycleEvent {
    Ready,
    StillConnecting,
    Recovering { reason: AudioCancelReason },
    InitializationFailed { error: String },
    RecoveryStalled,
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioStartError {
    AlreadyStarting,
    AudioRecovering,
    AlreadyRecording,
    SpawnFailed(String),
    InitializationFailed(String),
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
            Self::SpawnFailed(error) | Self::InitializationFailed(error) => {
                formatter.write_str(error)
            }
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
    Stopping,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicPhase {
    Idle = 0,
    Starting = 1,
    Recording = 2,
    Recovering = 3,
    Stopping = 4,
}

struct PublicState {
    phase: AtomicU8,
    still_connecting: std::sync::atomic::AtomicBool,
    owner: Mutex<Option<AudioOwner>>,
    last_device_name: Mutex<Option<String>>,
}

impl Default for PublicState {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(PublicPhase::Idle as u8),
            still_connecting: std::sync::atomic::AtomicBool::new(false),
            owner: Mutex::new(None),
            last_device_name: Mutex::new(None),
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

    fn is_still_connecting_for(&self, owner: AudioOwner) -> bool {
        self.still_connecting.load(Ordering::SeqCst)
            && *self
                .owner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                == Some(owner)
    }
}

#[derive(Clone, Copy)]
struct SupervisorConfig {
    still_connecting_after: Duration,
    hard_deadline: Duration,
    recovery_guidance_after: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            still_connecting_after: STILL_CONNECTING_AFTER,
            hard_deadline: HARD_INITIALIZATION_DEADLINE,
            recovery_guidance_after: RECOVERY_GUIDANCE_AFTER,
        }
    }
}

trait WorkerFactory: Send + Sync + 'static {
    fn spawn(
        &self,
        spec: AudioWorkerSpec,
        event_sender: Sender<AudioWorkerEvent>,
    ) -> Result<JoinHandle<()>, String>;
}

struct ProductionWorkerFactory;

impl WorkerFactory for ProductionWorkerFactory {
    fn spawn(
        &self,
        spec: AudioWorkerSpec,
        event_sender: Sender<AudioWorkerEvent>,
    ) -> Result<JoinHandle<()>, String> {
        #[cfg(debug_assertions)]
        if should_inject_hung_stream_build() {
            let owner = spec.owner;
            return std::thread::Builder::new()
                .name("murmur-audio-fault".to_string())
                .spawn(move || {
                    for phase in [
                        audio::AudioInitPhase::DeviceEnumeration,
                        audio::AudioInitPhase::ConfigLookup,
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
                        phase: audio::AudioInitPhase::StreamBuild,
                    });
                    // Model a synchronous Core Audio call that never returns.
                    loop {
                        std::thread::park();
                    }
                })
                .map_err(|error| format!("Failed to spawn audio fault worker: {error}"));
        }
        audio::spawn_capture_worker(spec, event_sender)
    }
}

#[cfg(debug_assertions)]
fn should_inject_hung_stream_build() -> bool {
    match std::env::var("MURMUR_AUDIO_TEST_SCENARIO").ok().as_deref() {
        Some("hang_stream_build") => true,
        Some("hang_stream_build_once") => std::env::var_os("MURMUR_AUDIO_TEST_SENTINEL")
            .is_some_and(|path| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .is_ok()
            }),
        _ => false,
    }
}

trait LifecycleSink: Send + Sync + 'static {
    fn notify(
        &self,
        app_handle: Option<&tauri::AppHandle>,
        owner: AudioOwner,
        event: AudioLifecycleEvent,
    );
}

struct ProductionLifecycleSink;

impl LifecycleSink for ProductionLifecycleSink {
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
        }
    }
}

struct StartRequest {
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_name: Option<String>,
    origin: String,
    wait_until_ready: bool,
    response: Sender<Result<(), AudioStartError>>,
}

enum SupervisorMessage {
    Start(StartRequest),
    Stop {
        owner: Option<AudioOwner>,
        response: Sender<Result<Vec<f32>, String>>,
    },
    Cancel {
        owner: Option<AudioOwner>,
        reason: AudioCancelReason,
        starting_only: bool,
        response: Sender<Result<bool, String>>,
    },
    Worker(AudioWorkerEvent),
    #[cfg(test)]
    Shutdown(Sender<()>),
}

struct Attempt {
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    origin: String,
    phase: AttemptPhase,
    accepted_at: Instant,
    ready_at: Option<Instant>,
    stopping_started_at: Option<Instant>,
    still_connecting_emitted: bool,
    failure_reported: bool,
    stopping_guidance_emitted: bool,
    initialization_error: Option<String>,
    command_sender: Sender<AudioCommand>,
    thread_handle: Option<JoinHandle<()>>,
    shared: Arc<Mutex<Vec<f32>>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    sample_rate: u32,
    device_name: Option<String>,
    start_response: Option<Sender<Result<(), AudioStartError>>>,
    stop_response: Option<Sender<Result<Vec<f32>, String>>>,
}

struct AbandonedWorker {
    owner: AudioOwner,
    abandoned_at: Instant,
    handle: JoinHandle<()>,
}

#[derive(Clone)]
struct AudioSupervisor {
    sender: Sender<SupervisorMessage>,
    public: Arc<PublicState>,
}

static SUPERVISOR: OnceLock<AudioSupervisor> = OnceLock::new();
static ABANDONED_REAPER: OnceLock<Sender<AbandonedWorker>> = OnceLock::new();

/// Own join handles after a cancelled generation releases logical microphone
/// ownership. Rust cannot interrupt the synchronous Core Audio call, so this
/// single polling thread joins each worker only after macOS lets it return.
fn abandoned_reaper() -> &'static Sender<AbandonedWorker> {
    ABANDONED_REAPER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<AbandonedWorker>();
        std::thread::Builder::new()
            .name("murmur-audio-reaper".to_string())
            .spawn(move || {
                let mut workers = Vec::<AbandonedWorker>::new();
                loop {
                    let received = if workers.is_empty() {
                        receiver
                            .recv()
                            .map_err(|_| RecvTimeoutError::Disconnected)
                    } else {
                        receiver.recv_timeout(Duration::from_millis(100))
                    };
                    match received {
                        Ok(worker) => {
                            let owner = worker.owner;
                            workers.push(worker);
                            tracing::warn!(
                                target: "audio",
                                owner = owner.telemetry_id(),
                                owner_kind = owner.kind(),
                                abandoned_workers = workers.len(),
                                "audio worker remains blocked in macOS and is being reaped asynchronously"
                            );
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    let mut index = 0;
                    while index < workers.len() {
                        if workers[index].handle.is_finished() {
                            let worker = workers.swap_remove(index);
                            let panicked = worker.handle.join().is_err();
                            tracing::info!(
                                target: "audio",
                                owner = worker.owner.telemetry_id(),
                                owner_kind = worker.owner.kind(),
                                abandoned_ms = worker.abandoned_at.elapsed().as_millis() as u64,
                                panicked,
                                remaining_abandoned_workers = workers.len(),
                                "abandoned audio worker exited and was reaped"
                            );
                        } else {
                            index += 1;
                        }
                    }
                }
            })
            .expect("audio reaper thread must spawn");
        sender
    })
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
    let (worker_event_sender, worker_event_receiver) = mpsc::channel::<AudioWorkerEvent>();
    let public = Arc::new(PublicState::default());

    let forward_sender = sender.clone();
    std::thread::Builder::new()
        .name("murmur-audio-events".to_string())
        .spawn(move || {
            while let Ok(event) = worker_event_receiver.recv() {
                if forward_sender
                    .send(SupervisorMessage::Worker(event))
                    .is_err()
                {
                    break;
                }
            }
        })
        .expect("audio event forwarder thread must spawn");

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
    worker_event_sender: Sender<AudioWorkerEvent>,
    factory: Arc<dyn WorkerFactory>,
    sink: Arc<dyn LifecycleSink>,
    public: Arc<PublicState>,
    config: SupervisorConfig,
) {
    let mut attempt: Option<Attempt> = None;
    loop {
        let timeout = deadline_wait(attempt.as_ref(), config);
        match receiver.recv_timeout(timeout) {
            Ok(message) => {
                #[cfg(test)]
                if let SupervisorMessage::Shutdown(response) = message {
                    let _ = response.send(());
                    return;
                }
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
                handle_deadlines(&mut attempt, sink.as_ref(), &public, config);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn deadline_wait(attempt: Option<&Attempt>, config: SupervisorConfig) -> Duration {
    let Some(attempt) = attempt else {
        return Duration::from_secs(60);
    };
    let now = Instant::now();
    let deadline = match attempt.phase {
        AttemptPhase::Starting if !attempt.still_connecting_emitted => {
            attempt.accepted_at + config.still_connecting_after
        }
        AttemptPhase::Starting => attempt.accepted_at + config.hard_deadline,
        AttemptPhase::Stopping if !attempt.stopping_guidance_emitted => {
            attempt.stopping_started_at.unwrap_or(now) + config.recovery_guidance_after
        }
        _ => now + Duration::from_secs(60),
    };
    deadline.saturating_duration_since(now)
}

fn handle_message(
    message: SupervisorMessage,
    attempt: &mut Option<Attempt>,
    worker_event_sender: &Sender<AudioWorkerEvent>,
    factory: &dyn WorkerFactory,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    match message {
        SupervisorMessage::Start(request) => {
            handle_start(request, attempt, worker_event_sender, factory, public);
        }
        SupervisorMessage::Stop { owner, response } => {
            let Some(current) = attempt.as_ref() else {
                let _ = response.send(Ok(Vec::new()));
                return;
            };
            if owner.is_some_and(|owner| owner != current.owner) {
                let _ = response.send(Err("Audio owner changed before stop".to_string()));
                return;
            }
            if current.phase == AttemptPhase::Starting {
                abandon_attempt(attempt, AudioCancelReason::User, sink, public, false);
                let _ = response.send(Ok(Vec::new()));
                return;
            }
            let current = attempt.as_mut().expect("attempt was checked above");
            match current.phase {
                AttemptPhase::Recording => {
                    current.active.store(false, Ordering::SeqCst);
                    let _ = current.command_sender.send(AudioCommand::Stop);
                    current.phase = AttemptPhase::Stopping;
                    current.stopping_started_at = Some(Instant::now());
                    current.stop_response = Some(response);
                    public.set_phase(PublicPhase::Stopping);
                }
                AttemptPhase::Starting => unreachable!("starting was handled above"),
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
                        AttemptPhase::Starting | AttemptPhase::Recording
                    )
                    && (!starting_only || current.phase == AttemptPhase::Starting)
            } else {
                false
            };
            if should_abandon {
                abandon_attempt(attempt, reason, sink, public, false);
            }
            let cancelled = should_abandon;
            let _ = response.send(Ok(cancelled));
        }
        SupervisorMessage::Worker(event) => {
            handle_worker_event(event, attempt, sink, public);
        }
        #[cfg(test)]
        SupervisorMessage::Shutdown(_) => unreachable!(),
    }
}

fn handle_start(
    request: StartRequest,
    attempt: &mut Option<Attempt>,
    worker_event_sender: &Sender<AudioWorkerEvent>,
    factory: &dyn WorkerFactory,
    public: &PublicState,
) {
    if let Some(current) = attempt.as_ref() {
        let error = match current.phase {
            AttemptPhase::Starting => AudioStartError::AlreadyStarting,
            AttemptPhase::Recording => AudioStartError::AlreadyRecording,
            AttemptPhase::Stopping => AudioStartError::AudioRecovering,
        };
        let _ = request.response.send(Err(error));
        return;
    }

    let shared = Arc::new(Mutex::new(Vec::<f32>::new()));
    // Fail closed until readiness is accepted for this exact owner.
    let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (command_sender, command_receiver) = mpsc::channel();
    let spec = AudioWorkerSpec {
        owner: request.owner,
        command_receiver,
        shared: Arc::clone(&shared),
        active: Arc::clone(&active),
        app_handle: request.app_handle.clone(),
        device_name: request.device_name,
    };
    let thread_handle = match factory.spawn(spec, worker_event_sender.clone()) {
        Ok(handle) => handle,
        Err(error) => {
            let _ = request
                .response
                .send(Err(AudioStartError::SpawnFailed(error)));
            return;
        }
    };

    tracing::info!(
        target: "audio",
        owner = request.owner.telemetry_id(),
        owner_kind = request.owner.kind(),
        origin = request.origin.as_str(),
        "audio initialization accepted"
    );
    let start_response = if request.wait_until_ready {
        Some(request.response)
    } else {
        let _ = request.response.send(Ok(()));
        None
    };
    *attempt = Some(Attempt {
        owner: request.owner,
        app_handle: request.app_handle,
        origin: request.origin,
        phase: AttemptPhase::Starting,
        accepted_at: Instant::now(),
        ready_at: None,
        stopping_started_at: None,
        still_connecting_emitted: false,
        failure_reported: false,
        stopping_guidance_emitted: false,
        initialization_error: None,
        command_sender,
        thread_handle: Some(thread_handle),
        shared,
        active,
        sample_rate: WHISPER_SAMPLE_RATE,
        device_name: None,
        start_response,
        stop_response: None,
    });
    public.still_connecting.store(false, Ordering::SeqCst);
    public.set_owner(request.owner);
    public.set_phase(PublicPhase::Starting);
}

fn handle_worker_event(
    event: AudioWorkerEvent,
    attempt: &mut Option<Attempt>,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    let owner = match &event {
        AudioWorkerEvent::PhaseEntered { owner, .. }
        | AudioWorkerEvent::PhaseExited { owner, .. }
        | AudioWorkerEvent::Ready { owner, .. }
        | AudioWorkerEvent::InitFailed { owner, .. }
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
    if current.owner != owner {
        tracing::warn!(
            target: "audio",
            owner = owner.telemetry_id(),
            current_owner = current.owner.telemetry_id(),
            "stale audio worker event ignored"
        );
        return;
    }

    match event {
        AudioWorkerEvent::PhaseEntered { phase, .. } => {
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
        AudioWorkerEvent::Ready {
            sample_rate,
            device_name,
            ..
        } => match current.phase {
            AttemptPhase::Starting => {
                current.sample_rate = sample_rate;
                current.device_name = Some(device_name.clone());
                current.ready_at = Some(Instant::now());
                current.active.store(true, Ordering::SeqCst);
                public.still_connecting.store(false, Ordering::SeqCst);
                current.phase = AttemptPhase::Recording;
                *public
                    .last_device_name
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(device_name);
                public.set_phase(PublicPhase::Recording);
                tracing::info!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    startup_ms = current.accepted_at.elapsed().as_millis() as u64,
                    origin = current.origin.as_str(),
                    "audio readiness accepted"
                );
                sink.notify(
                    current.app_handle.as_ref(),
                    current.owner,
                    AudioLifecycleEvent::Ready,
                );
                if let Some(response) = current.start_response.take() {
                    let _ = response.send(Ok(()));
                }
            }
            AttemptPhase::Recording | AttemptPhase::Stopping => {
                tracing::warn!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    "duplicate audio readiness ignored"
                );
            }
        },
        AudioWorkerEvent::InitFailed { error, .. } => {
            current.initialization_error = Some(error.clone());
            if let Some(response) = current.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(error)));
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

    match finished.phase {
        AttemptPhase::Stopping => {
            let samples = take_samples(&mut finished);
            if let Some(response) = finished.stop_response.take() {
                let _ = response.send(Ok(samples));
            }
        }
        AttemptPhase::Starting => {
            let error = finished
                .initialization_error
                .take()
                .unwrap_or_else(|| "Audio initialization ended before readiness".to_string());
            report_failure_once(&mut finished, sink, error.clone());
            if let Some(response) = finished.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(error)));
            }
            sink.notify(
                finished.app_handle.as_ref(),
                finished.owner,
                AudioLifecycleEvent::Idle,
            );
        }
        AttemptPhase::Recording => {
            report_failure_once(
                &mut finished,
                sink,
                "Audio capture stopped unexpectedly".to_string(),
            );
            sink.notify(
                finished.app_handle.as_ref(),
                finished.owner,
                AudioLifecycleEvent::Idle,
            );
        }
    }
    finished.active.store(false, Ordering::SeqCst);
    public.still_connecting.store(false, Ordering::SeqCst);
    public.clear_owner(finished.owner);
    public.set_phase(PublicPhase::Idle);
}

fn take_samples(attempt: &mut Attempt) -> Vec<f32> {
    let mut samples = attempt
        .shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let raw = std::mem::take(&mut *samples);
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
    if attempt.sample_rate != WHISPER_SAMPLE_RATE && !raw.is_empty() {
        audio::resample(&raw, attempt.sample_rate, WHISPER_SAMPLE_RATE)
    } else {
        raw
    }
}

fn recording_duration_since_ready(ready_at: Option<Instant>, now: Instant) -> Duration {
    ready_at
        .map(|ready| now.saturating_duration_since(ready))
        .unwrap_or_default()
}

fn abandon_attempt(
    attempt: &mut Option<Attempt>,
    reason: AudioCancelReason,
    sink: &dyn LifecycleSink,
    public: &PublicState,
    report_failure: bool,
) {
    let Some(mut abandoned) = attempt.take() else {
        return;
    };
    abandoned.active.store(false, Ordering::SeqCst);
    let _ = abandoned.command_sender.send(AudioCommand::Stop);
    public.set_phase(PublicPhase::Recovering);
    tracing::info!(
        target: "audio",
        owner = abandoned.owner.telemetry_id(),
        owner_kind = abandoned.owner.kind(),
        cancellation_reason = reason.as_str(),
        "audio attempt detached for asynchronous recovery"
    );
    sink.notify(
        abandoned.app_handle.as_ref(),
        abandoned.owner,
        AudioLifecycleEvent::Recovering { reason },
    );
    if let Some(response) = abandoned.start_response.take() {
        let _ = response.send(Err(AudioStartError::Cancelled));
    }
    if report_failure {
        report_failure_once(
            &mut abandoned,
            sink,
            "Microphone initialization exceeded the 30 second deadline".to_string(),
        );
    }
    public.still_connecting.store(false, Ordering::SeqCst);
    public.clear_owner(abandoned.owner);
    public.set_phase(PublicPhase::Idle);
    sink.notify(
        abandoned.app_handle.as_ref(),
        abandoned.owner,
        AudioLifecycleEvent::Idle,
    );
    if let Some(handle) = abandoned.thread_handle.take() {
        let _ = abandoned_reaper().send(AbandonedWorker {
            owner: abandoned.owner,
            abandoned_at: Instant::now(),
            handle,
        });
    }
}

fn report_failure_once(attempt: &mut Attempt, sink: &dyn LifecycleSink, error: String) {
    if attempt.failure_reported {
        return;
    }
    attempt.failure_reported = true;
    tracing::error!(
        target: "audio",
        owner = attempt.owner.telemetry_id(),
        owner_kind = attempt.owner.kind(),
        error = error.as_str(),
        "audio initialization failed"
    );
    sink.notify(
        attempt.app_handle.as_ref(),
        attempt.owner,
        AudioLifecycleEvent::InitializationFailed { error },
    );
}

fn handle_deadlines(
    attempt: &mut Option<Attempt>,
    sink: &dyn LifecycleSink,
    public: &PublicState,
    config: SupervisorConfig,
) {
    let Some(current) = attempt.as_ref() else {
        return;
    };
    let elapsed = current.accepted_at.elapsed();
    let should_emit_still_connecting = current.phase == AttemptPhase::Starting
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
        abandon_attempt(attempt, AudioCancelReason::HardDeadline, sink, public, true);
        return;
    }
    let current = attempt.as_mut().expect("attempt was checked above");
    match current.phase {
        AttemptPhase::Stopping
            if !current.stopping_guidance_emitted
                && current
                    .stopping_started_at
                    .is_some_and(|started| started.elapsed() >= config.recovery_guidance_after) =>
        {
            current.stopping_guidance_emitted = true;
            tracing::warn!(
                target: "audio",
                owner = current.owner.telemetry_id(),
                owner_kind = current.owner.kind(),
                "audio stopping remains blocked"
            );
            sink.notify(
                current.app_handle.as_ref(),
                current.owner,
                AudioLifecycleEvent::RecoveryStalled,
            );
        }
        _ => {}
    }
}

fn send_start(
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_name: Option<String>,
    origin: &str,
    wait_until_ready: bool,
) -> Result<(), AudioStartError> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Start(StartRequest {
            owner,
            app_handle,
            device_name,
            origin: origin.to_string(),
            wait_until_ready,
            response: response_sender,
        }))
        .map_err(|_| AudioStartError::SupervisorUnavailable)?;
    let timeout = if wait_until_ready {
        HARD_INITIALIZATION_DEADLINE + Duration::from_secs(2)
    } else {
        SUPERVISOR_RESPONSE_TIMEOUT
    };
    response_receiver
        .recv_timeout(timeout)
        .map_err(|_| AudioStartError::SupervisorUnavailable)?
}

pub(crate) fn start_dictation_recording(
    app_handle: tauri::AppHandle,
    device_name: Option<String>,
    recording_id: u64,
    origin: &str,
) -> Result<(), AudioStartError> {
    send_start(
        AudioOwner::Dictation(recording_id),
        Some(app_handle),
        device_name,
        origin,
        false,
    )
}

pub(crate) fn start_transform_recording(
    app_handle: Option<tauri::AppHandle>,
    device_name: Option<String>,
    transform_pass_id: u64,
) -> Result<(), String> {
    send_start(
        AudioOwner::Transform(transform_pass_id),
        app_handle,
        device_name,
        "transform",
        false,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn stop_dictation_recording(recording_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Dictation(recording_id)))
}

pub(crate) fn stop_current_recording() -> Result<Vec<f32>, String> {
    stop(None)
}

fn stop(owner: Option<AudioOwner>) -> Result<Vec<f32>, String> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Stop {
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

pub(crate) fn is_audio_active() -> bool {
    supervisor().public.is_active()
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

pub(crate) fn last_device_name() -> Option<String> {
    supervisor()
        .public
        .last_device_name
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
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
    use crate::audio::{AudioInitPhase, AudioWorkerEvent};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};

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
        spawn_count: Arc<AtomicUsize>,
        active_flags: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
        phase: AudioInitPhase,
    }

    struct BlockingTeardownFactory {
        gate: Gate,
    }

    impl WorkerFactory for BlockingTeardownFactory {
        fn spawn(
            &self,
            spec: AudioWorkerSpec,
            event_sender: Sender<AudioWorkerEvent>,
        ) -> Result<JoinHandle<()>, String> {
            let gate = self.gate.clone();
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                let _ = event_sender.send(AudioWorkerEvent::Ready {
                    owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    device_name: "test".to_string(),
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
            event_sender: Sender<AudioWorkerEvent>,
        ) -> Result<JoinHandle<()>, String> {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            self.active_flags
                .lock()
                .unwrap()
                .push(Arc::clone(&spec.active));
            let gate = self.gate.clone();
            let phase = self.phase;
            Ok(std::thread::spawn(move || {
                let owner = spec.owner;
                let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
                gate.wait();
                let _ = event_sender.send(AudioWorkerEvent::PhaseExited {
                    owner,
                    phase,
                    elapsed_ms: 1,
                });
                let _ = event_sender.send(AudioWorkerEvent::Ready {
                    owner,
                    sample_rate: WHISPER_SAMPLE_RATE,
                    device_name: "test".to_string(),
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
    }

    impl LifecycleSink for RecordingSink {
        fn notify(
            &self,
            _app_handle: Option<&tauri::AppHandle>,
            owner: AudioOwner,
            event: AudioLifecycleEvent,
        ) {
            self.events.lock().unwrap().push((owner, event));
        }
    }

    fn harness(
        phase: AudioInitPhase,
        config: SupervisorConfig,
    ) -> (
        AudioSupervisor,
        Gate,
        Arc<AtomicUsize>,
        Arc<RecordingSink>,
        Arc<Mutex<Vec<Arc<AtomicBool>>>>,
    ) {
        let gate = Gate::closed();
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(RecordingSink::default());
        let active_flags = Arc::new(Mutex::new(Vec::new()));
        let supervisor = spawn_supervisor(
            Arc::new(BlockingFactory {
                gate: gate.clone(),
                spawn_count: Arc::clone(&spawn_count),
                active_flags: Arc::clone(&active_flags),
                phase,
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
        let (sender, receiver) = mpsc::channel();
        supervisor
            .sender
            .send(SupervisorMessage::Start(StartRequest {
                owner,
                app_handle: None,
                device_name: None,
                origin: "hold".to_string(),
                wait_until_ready: false,
                response: sender,
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

    fn wait_until(message: &str, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !predicate() {
            assert!(Instant::now() < deadline, "{message}");
            std::thread::sleep(Duration::from_millis(1));
        }
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
    fn cancel_releases_owner_and_late_ready_never_becomes_recording() {
        for phase in [
            AudioInitPhase::DeviceEnumeration,
            AudioInitPhase::StreamBuild,
            AudioInitPhase::StreamPlay,
        ] {
            let (supervisor, gate, _, sink, active_flags) =
                harness(phase, SupervisorConfig::default());
            let owner = AudioOwner::Dictation(3);
            assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
            let active = Arc::clone(&active_flags.lock().unwrap()[0]);
            assert!(!active.load(Ordering::SeqCst));
            assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
            assert!(!supervisor.public.is_active());
            gate.open();
            assert!(
                !active.load(Ordering::SeqCst),
                "abandoned attempts must never accept samples or emit levels"
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
    fn benchmark_cancelled_hang_and_immediate_retry() {
        let (supervisor, gate, spawn_count, _, _) =
            harness(AudioInitPhase::StreamBuild, SupervisorConfig::default());
        let first = AudioOwner::Dictation(30);
        assert_eq!(start(&supervisor, first).recv().unwrap(), Ok(()));

        let recovery_started = Instant::now();
        assert_eq!(cancel(&supervisor, first).recv().unwrap(), Ok(true));
        let recovery_us = recovery_started.elapsed().as_micros() as u64;
        assert!(!supervisor.public.is_active());

        let retry_started = Instant::now();
        let second = AudioOwner::Dictation(31);
        assert_eq!(start(&supervisor, second).recv().unwrap(), Ok(()));
        let retry_accepted_us = retry_started.elapsed().as_micros() as u64;
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
        assert_eq!(cancel(&supervisor, second).recv().unwrap(), Ok(true));

        println!(
            "audio_recovery_benchmark recovery_us={recovery_us} retry_accepted_us={retry_accepted_us}"
        );
        gate.open();
        shutdown(&supervisor);
    }

    #[test]
    fn hard_deadline_reports_failure_once_and_allows_immediate_retry() {
        let config = SupervisorConfig {
            still_connecting_after: Duration::from_millis(5),
            hard_deadline: Duration::from_millis(12),
            recovery_guidance_after: Duration::from_millis(10),
        };
        let (supervisor, gate, spawn_count, sink, _) =
            harness(AudioInitPhase::ConfigLookup, config);
        let owner = AudioOwner::Dictation(4);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        wait_until("hard deadline did not report failure", || {
            sink.events
                .lock()
                .unwrap()
                .iter()
                .any(|(_, event)| matches!(event, AudioLifecycleEvent::InitializationFailed { .. }))
        });
        assert!(!supervisor.public.is_active());
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
        }
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(())
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            cancel(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(true)
        );
        gate.open();
        assert!(!supervisor.public.is_active());
        shutdown(&supervisor);
    }

    #[test]
    fn complete_teardown_allows_a_successful_retry() {
        let (supervisor, gate, spawn_count, _, _) =
            harness(AudioInitPhase::ReadySignal, SupervisorConfig::default());
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
        assert!(!supervisor.public.is_active());
        gate.open();
        shutdown(&supervisor);
    }

    #[test]
    fn stale_worker_generation_cannot_activate_current_attempt() {
        let (supervisor, gate, _, sink, active_flags) =
            harness(AudioInitPhase::ConfigLookup, SupervisorConfig::default());
        let owner = AudioOwner::Dictation(9);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        supervisor
            .sender
            .send(SupervisorMessage::Worker(AudioWorkerEvent::Ready {
                owner: AudioOwner::Dictation(8),
                sample_rate: WHISPER_SAMPLE_RATE,
                device_name: "stale".to_string(),
            }))
            .unwrap();
        // The cancel response is a FIFO barrier proving the stale Ready event
        // was handled before these assertions.
        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        assert!(!active_flags.lock().unwrap()[0].load(Ordering::SeqCst));
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
    fn stopping_deadline_surfaces_guidance_without_detaching_the_worker() {
        let gate = Gate::closed();
        let sink = Arc::new(RecordingSink::default());
        let supervisor = spawn_supervisor(
            Arc::new(BlockingTeardownFactory { gate: gate.clone() }),
            sink.clone(),
            SupervisorConfig {
                still_connecting_after: Duration::from_secs(1),
                hard_deadline: Duration::from_secs(2),
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
            Ok(Vec::new())
        );
        wait_until("stopping worker was not joined", || {
            !supervisor.public.is_active()
        });
        shutdown(&supervisor);
    }
}
