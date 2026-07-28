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
    Recovering,
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
    last_device_name: Mutex<Option<String>>,
}

impl Default for PublicState {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(PublicPhase::Idle as u8),
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
        audio::spawn_capture_worker(spec, event_sender)
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
    recovery_started_at: Option<Instant>,
    still_connecting_emitted: bool,
    failure_reported: bool,
    recovery_guidance_emitted: bool,
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

#[derive(Clone)]
struct AudioSupervisor {
    sender: Sender<SupervisorMessage>,
    public: Arc<PublicState>,
}

static SUPERVISOR: OnceLock<AudioSupervisor> = OnceLock::new();

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
        AttemptPhase::Recovering if !attempt.recovery_guidance_emitted => {
            attempt.recovery_started_at.unwrap_or(now) + config.recovery_guidance_after
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
            let Some(current) = attempt.as_mut() else {
                let _ = response.send(Ok(Vec::new()));
                return;
            };
            if owner.is_some_and(|owner| owner != current.owner) {
                let _ = response.send(Err("Audio owner changed before stop".to_string()));
                return;
            }
            match current.phase {
                AttemptPhase::Recording => {
                    current.active.store(false, Ordering::SeqCst);
                    let _ = current.command_sender.send(AudioCommand::Stop);
                    current.phase = AttemptPhase::Stopping;
                    current.stop_response = Some(response);
                    public.set_phase(PublicPhase::Stopping);
                }
                AttemptPhase::Starting => {
                    enter_recovering(current, AudioCancelReason::User, sink, public, false);
                    let _ = response.send(Ok(Vec::new()));
                }
                AttemptPhase::Recovering => {
                    let _ = response.send(Err("Audio is recovering".to_string()));
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
            let cancelled = if let Some(current) = attempt.as_mut() {
                let owner_matches = owner.is_none_or(|owner| owner == current.owner);
                if owner_matches
                    && matches!(
                        current.phase,
                        AttemptPhase::Starting | AttemptPhase::Recording
                    )
                    && (!starting_only || current.phase == AttemptPhase::Starting)
                {
                    enter_recovering(current, reason, sink, public, false);
                    true
                } else {
                    false
                }
            } else {
                false
            };
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
            AttemptPhase::Recovering => AudioStartError::AudioRecovering,
            AttemptPhase::Recording | AttemptPhase::Stopping => AudioStartError::AlreadyRecording,
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
        recovery_started_at: None,
        still_connecting_emitted: false,
        failure_reported: false,
        recovery_guidance_emitted: false,
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
            AttemptPhase::Recovering => {
                current.active.store(false, Ordering::SeqCst);
                let _ = current.command_sender.send(AudioCommand::Stop);
                tracing::info!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    "late audio readiness stopped"
                );
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
        AttemptPhase::Recovering => {
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

fn enter_recovering(
    attempt: &mut Attempt,
    reason: AudioCancelReason,
    sink: &dyn LifecycleSink,
    public: &PublicState,
    report_failure: bool,
) {
    if attempt.phase == AttemptPhase::Recovering {
        return;
    }
    attempt.active.store(false, Ordering::SeqCst);
    let _ = attempt.command_sender.send(AudioCommand::Stop);
    attempt.phase = AttemptPhase::Recovering;
    attempt.recovery_started_at = Some(Instant::now());
    public.set_phase(PublicPhase::Recovering);
    tracing::info!(
        target: "audio",
        owner = attempt.owner.telemetry_id(),
        owner_kind = attempt.owner.kind(),
        cancellation_reason = reason.as_str(),
        "audio attempt entering recovery"
    );
    sink.notify(
        attempt.app_handle.as_ref(),
        attempt.owner,
        AudioLifecycleEvent::Recovering { reason },
    );
    if let Some(response) = attempt.start_response.take() {
        let _ = response.send(Err(AudioStartError::Cancelled));
    }
    if report_failure {
        report_failure_once(
            attempt,
            sink,
            "Microphone initialization exceeded the 30 second deadline".to_string(),
        );
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
    let Some(current) = attempt.as_mut() else {
        return;
    };
    let elapsed = current.accepted_at.elapsed();
    match current.phase {
        AttemptPhase::Starting
            if !current.still_connecting_emitted && elapsed >= config.still_connecting_after =>
        {
            current.still_connecting_emitted = true;
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
        AttemptPhase::Starting if elapsed >= config.hard_deadline => {
            enter_recovering(current, AudioCancelReason::HardDeadline, sink, public, true);
        }
        AttemptPhase::Recovering
            if !current.recovery_guidance_emitted
                && current
                    .recovery_started_at
                    .is_some_and(|started| started.elapsed() >= config.recovery_guidance_after) =>
        {
            current.recovery_guidance_emitted = true;
            tracing::warn!(
                target: "audio",
                owner = current.owner.telemetry_id(),
                owner_kind = current.owner.kind(),
                "audio recovery remains blocked"
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
        .recv()
        .map_err(|_| "Audio lifecycle supervisor stopped during teardown".to_string())?
}

pub(crate) fn cancel_dictation_initialization(
    recording_id: u64,
    reason: AudioCancelReason,
) -> Result<bool, String> {
    cancel(Some(AudioOwner::Dictation(recording_id)), reason, true)
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

    fn shutdown(supervisor: &AudioSupervisor) {
        let (sender, receiver) = mpsc::channel();
        let _ = supervisor.sender.send(SupervisorMessage::Shutdown(sender));
        let _ = receiver.recv_timeout(Duration::from_secs(1));
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
        std::thread::sleep(Duration::from_millis(20));
        shutdown(&supervisor);
    }

    #[test]
    fn cancel_enters_recovering_and_late_ready_never_becomes_recording() {
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
            assert!(supervisor.public.is_active());
            gate.open();
            std::thread::sleep(Duration::from_millis(20));
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
    fn hard_deadline_reports_failure_once_and_retains_owner_until_exit() {
        let config = SupervisorConfig {
            still_connecting_after: Duration::from_millis(5),
            hard_deadline: Duration::from_millis(12),
            recovery_guidance_after: Duration::from_millis(10),
        };
        let (supervisor, gate, _, sink, _) = harness(AudioInitPhase::ConfigLookup, config);
        let owner = AudioOwner::Dictation(4);
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
        std::thread::sleep(Duration::from_millis(25));
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
        }
        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(5))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Err(AudioStartError::AudioRecovering)
        );
        gate.open();
        std::thread::sleep(Duration::from_millis(20));
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
        std::thread::sleep(Duration::from_millis(20));

        assert_eq!(
            start(&supervisor, AudioOwner::Dictation(7))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(())
        );
        assert_eq!(spawn_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            cancel(&supervisor, AudioOwner::Dictation(7))
                .recv_timeout(Duration::from_millis(100))
                .unwrap(),
            Ok(true)
        );
        std::thread::sleep(Duration::from_millis(20));
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
        assert!(supervisor.public.is_active());
        gate.open();
        std::thread::sleep(Duration::from_millis(20));
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
        std::thread::sleep(Duration::from_millis(10));
        assert!(!active_flags.lock().unwrap()[0].load(Ordering::SeqCst));
        assert!(!sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, event)| *event == AudioLifecycleEvent::Ready));
        assert_eq!(cancel(&supervisor, owner).recv().unwrap(), Ok(true));
        gate.open();
        std::thread::sleep(Duration::from_millis(20));
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
            std::thread::sleep(Duration::from_millis(20));
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
}
