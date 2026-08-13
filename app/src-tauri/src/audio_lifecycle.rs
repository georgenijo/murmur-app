use crate::audio::{
    self, AudioCommand, AudioFailure, AudioFailureKind, AudioInitPhase, AudioWorkerEvent,
    AudioWorkerEventSender, AudioWorkerSpec,
};
use crate::state::WHISPER_SAMPLE_RATE;
use std::collections::HashMap;
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
    Query(u64),
    Preview(u64),
    #[cfg(feature = "internal-benchmark")]
    Corpus(u64),
}

impl AudioOwner {
    pub(crate) fn telemetry_id(self) -> u64 {
        match self {
            Self::Dictation(id) | Self::Transform(id) | Self::Query(id) | Self::Preview(id) => id,
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
            #[cfg(feature = "internal-benchmark")]
            Self::Corpus(_) => "corpus",
        }
    }

    /// Preview capture proves device readiness and drives a level meter, but
    /// never keeps microphone samples for transcription or later replay.
    pub(crate) fn retains_samples(self) -> bool {
        !matches!(self, Self::Preview(_))
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

    #[cfg(test)]
    fn phase(&self) -> PublicPhase {
        match self.phase.load(Ordering::SeqCst) {
            value if value == PublicPhase::Idle as u8 => PublicPhase::Idle,
            value if value == PublicPhase::Starting as u8 => PublicPhase::Starting,
            value if value == PublicPhase::Recording as u8 => PublicPhase::Recording,
            value if value == PublicPhase::Recovering as u8 => PublicPhase::Recovering,
            value if value == PublicPhase::Stopping as u8 => PublicPhase::Stopping,
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
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    origin: String,
    wait_until_ready: bool,
    response: Sender<Result<(), AudioStartError>>,
}

// StartRequest is intentionally owned as one message so the supervisor can
// accept or reject an immutable capture request without shared mutable state.
#[allow(clippy::large_enum_variant)]
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
    CheckDeadlines {
        elapsed: Duration,
        response: Sender<()>,
    },
    #[cfg(test)]
    Shutdown(Sender<()>),
}

struct Attempt {
    owner: AudioOwner,
    app_handle: Option<tauri::AppHandle>,
    origin: String,
    phase: AttemptPhase,
    accepted_at: Instant,
    tcc_pending_since: Option<Instant>,
    tcc_paused_total: Duration,
    ready_at: Option<Instant>,
    stopping_started_at: Option<Instant>,
    still_connecting_emitted: bool,
    failure_reported: bool,
    stopping_guidance_emitted: bool,
    failure: Option<AudioFailure>,
    recovery_reason: Option<AudioCancelReason>,
    init_phase: AudioInitPhase,
    command_sender: Sender<AudioCommand>,
    thread_handle: Option<JoinHandle<()>>,
    shared: Arc<Mutex<Vec<f32>>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    sample_rate: u32,
    start_response: Option<Sender<Result<(), AudioStartError>>>,
    stop_response: Option<Sender<Result<Vec<f32>, String>>>,
}

impl Attempt {
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
    }
}

fn deadline_wait(attempt: Option<&Attempt>, config: SupervisorConfig) -> Duration {
    let Some(attempt) = attempt else {
        return Duration::from_secs(60);
    };
    let now = Instant::now();
    let deadline = match attempt.phase {
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
    deadline.saturating_duration_since(now)
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
                begin_recovery(attempt, AudioCancelReason::User, sink, public, None);
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
                        AttemptPhase::Starting | AttemptPhase::Recording
                    )
                    && (!starting_only || current.phase == AttemptPhase::Starting)
            } else {
                false
            };
            if should_abandon {
                begin_recovery(attempt, reason, sink, public, None);
            }
            let cancelled = should_abandon;
            let _ = response.send(Ok(cancelled));
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
    if let Some(current) = attempt.as_ref() {
        let error = match current.phase {
            AttemptPhase::Starting => AudioStartError::AlreadyStarting,
            AttemptPhase::Recording => AudioStartError::AlreadyRecording,
            AttemptPhase::Recovering | AttemptPhase::Stopping => AudioStartError::AudioRecovering,
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
        device_id: request.device_id,
    };
    // Serialize the transition into capture ownership with any idle-only
    // diagnostic enumeration. Once Starting is published below, the
    // diagnostic side will refuse the boundary until this attempt is joined.
    let _hal_boundary = hal_boundary()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Publish ownership before the worker can spawn its capture helper. Any
    // diagnostic/state-only enumeration racing this start must see Starting
    // and defer until the supervisor returns to Idle.
    public.still_connecting.store(false, Ordering::SeqCst);
    public.set_owner(request.owner);
    public.set_phase(PublicPhase::Starting);
    let thread_handle = match factory.spawn(spec, worker_event_sender.clone()) {
        Ok(handle) => handle,
        Err(error) => {
            public.clear_owner(request.owner);
            public.set_phase(PublicPhase::Idle);
            crate::log_shipper::audio_lifecycle_became_idle();
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
        owner,
        app_handle,
        origin,
        phase: AttemptPhase::Starting,
        accepted_at: Instant::now(),
        tcc_pending_since: None,
        tcc_paused_total: Duration::ZERO,
        ready_at: None,
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
    });
    let current = attempt.as_ref().expect("accepted attempt was installed");
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

fn handle_worker_event(
    event: AudioWorkerEvent,
    attempt: &mut Option<Attempt>,
    sink: &dyn LifecycleSink,
    public: &PublicState,
) {
    let owner = match &event {
        AudioWorkerEvent::PhaseEntered { owner, .. }
        | AudioWorkerEvent::PhaseExited { owner, .. }
        | AudioWorkerEvent::PermissionPromptPending { owner }
        | AudioWorkerEvent::PermissionPromptResolved { owner }
        | AudioWorkerEvent::TerminationUnconfirmed { owner, .. }
        | AudioWorkerEvent::FirstBuffer { owner, .. }
        | AudioWorkerEvent::InitFailed { owner, .. }
        | AudioWorkerEvent::RuntimeFailed { owner, .. }
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
        AudioWorkerEvent::PermissionPromptPending { .. } => {
            if current.phase == AttemptPhase::Starting && current.tcc_pending_since.is_none() {
                current.tcc_pending_since = Some(Instant::now());
                tracing::info!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    "microphone permission prompt pending; initialization deadlines suspended"
                );
            }
        }
        AudioWorkerEvent::PermissionPromptResolved { .. } => {
            if let Some(started) = current.tcc_pending_since.take() {
                let paused = started.elapsed();
                current.tcc_paused_total += paused;
                tracing::info!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    owner_kind = owner.kind(),
                    prompt_pending_ms = paused.as_millis() as u64,
                    "microphone permission prompt resolved; initialization deadlines resumed"
                );
            }
        }
        AudioWorkerEvent::TerminationUnconfirmed { failure, .. } => {
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
        AudioWorkerEvent::FirstBuffer { sample_rate, .. } => match current.phase {
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
                current.ready_at = Some(Instant::now());
                current.active.store(true, Ordering::SeqCst);
                public.still_connecting.store(false, Ordering::SeqCst);
                current.phase = AttemptPhase::Recording;
                public.set_phase(PublicPhase::Recording);
                if let Some(recording_id) = owner.dictation_id() {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.capture_ready",
                        recording_id,
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        startup_ms = current.accepted_at.elapsed().as_millis() as u64,
                        origin = current.origin.as_str(),
                        "audio readiness accepted"
                    );
                } else {
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.capture_ready",
                        owner = owner.telemetry_id(),
                        owner_kind = owner.kind(),
                        startup_ms = current.accepted_at.elapsed().as_millis() as u64,
                        origin = current.origin.as_str(),
                        "audio readiness accepted"
                    );
                }
                sink.notify(
                    current.app_handle.as_ref(),
                    current.owner,
                    AudioLifecycleEvent::Ready,
                );
                if let Some(response) = current.start_response.take() {
                    let _ = response.send(Ok(()));
                }
            }
            AttemptPhase::Recording | AttemptPhase::Recovering | AttemptPhase::Stopping => {
                tracing::warn!(
                    target: "audio",
                    owner = owner.telemetry_id(),
                    "duplicate audio readiness ignored"
                );
            }
        },
        AudioWorkerEvent::InitFailed { failure, .. } => {
            current.failure = Some(failure.clone());
            if let Some(response) = current.start_response.take() {
                let _ = response.send(Err(AudioStartError::InitializationFailed(failure)));
            }
        }
        AudioWorkerEvent::RuntimeFailed { failure, .. } => {
            if matches!(
                current.phase,
                AttemptPhase::Starting | AttemptPhase::Recording
            ) {
                current.failure = Some(failure.clone());
                // Dictation reports a retained prefix through Interrupted at
                // worker exit. Transform audio has no partial-transcript path,
                // so preserve its typed, content-free failure before recovery.
                let report_failure = match current.owner {
                    AudioOwner::Transform(_) | AudioOwner::Query(_) | AudioOwner::Preview(_) => {
                        Some(failure)
                    }
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

    let preview_idle_after_clear = matches!(finished.owner, AudioOwner::Preview(_));
    match finished.phase {
        AttemptPhase::Stopping => {
            let samples = take_samples(&mut finished);
            let response_delivered = finished
                .stop_response
                .take()
                .is_some_and(|response| response.send(Ok(samples)).is_ok());
            if !response_delivered && !preview_idle_after_clear {
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
            if !preview_idle_after_clear {
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
            if !preview_idle_after_clear {
                sink.notify(
                    finished.app_handle.as_ref(),
                    finished.owner,
                    AudioLifecycleEvent::Idle,
                );
            }
        }
        AttemptPhase::Recovering => {
            if finished.recovery_reason == Some(AudioCancelReason::RuntimeFailure)
                && matches!(finished.owner, AudioOwner::Dictation(_))
            {
                let reason = finished
                    .failure
                    .as_ref()
                    .map(|failure| failure.kind.as_str())
                    .unwrap_or("runtime_failure")
                    .to_string();
                let samples = take_samples(&mut finished);
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
            } else if !preview_idle_after_clear {
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
    crate::log_shipper::audio_lifecycle_became_idle();
    if preview_idle_after_clear {
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
    current.active.store(false, Ordering::SeqCst);
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
    sink.notify(
        current.app_handle.as_ref(),
        current.owner,
        AudioLifecycleEvent::Recovering { reason },
    );
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
    device_id: Option<String>,
    origin: &str,
    wait_until_ready: bool,
) -> Result<(), AudioStartError> {
    let (response_sender, response_receiver) = mpsc::channel();
    supervisor()
        .sender
        .send(SupervisorMessage::Start(StartRequest {
            owner,
            app_handle,
            device_id,
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
    device_id: Option<String>,
    recording_id: u64,
    origin: &str,
) -> Result<(), AudioStartError> {
    send_start(
        AudioOwner::Dictation(recording_id),
        Some(app_handle),
        device_id,
        origin,
        false,
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

pub(crate) fn stop_dictation_recording(recording_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Dictation(recording_id)))
}

pub(crate) fn stop_query_recording(query_pass_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Query(query_pass_id)))
}

pub(crate) fn stop_preview_recording(preview_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Preview(preview_id)))
}

#[cfg(feature = "internal-benchmark")]
pub(crate) fn stop_corpus_recording(capture_id: u64) -> Result<Vec<f32>, String> {
    stop(Some(AudioOwner::Corpus(capture_id)))
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

pub(crate) fn is_audio_active() -> bool {
    supervisor().public.is_active()
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
                owner,
                app_handle: None,
                device_id,
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
            }))
            .unwrap();
        assert_eq!(
            cancel_any_phase(&supervisor, owner).recv().unwrap(),
            Ok(true)
        );
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
        assert_eq!(start(&supervisor, owner).recv().unwrap(), Ok(()));
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
