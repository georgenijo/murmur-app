use crate::managed_child::{bundled_sibling, ManagedChild};
use murmur_capture_helper_protocol::{
    read_production_frame, write_production_control, CaptureBackend, CapturePhase, FailureCode,
    ProductionFrame, ProductionHelperMessage, ProductionHostMessage, SessionNonce,
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
const HELPER_STOP_DEADLINE: Duration = Duration::from_secs(2);
const HELPER_CONTROL_DEADLINE: Duration = Duration::from_secs(3);
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_millis(250);
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
        &["--production-v2", &capture_id_text, nonce_hex],
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
    transform_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_transform_recording(app_handle, device_id, transform_pass_id)
}

enum HelperRead {
    Frame(ProductionFrame<ProductionHelperMessage>),
    Invalid,
}

enum AttemptResult {
    Stopped,
    Failed {
        failure: AudioFailure,
        retained_audio: bool,
    },
}

fn backend_label(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::Cpal => "cpal",
        CaptureBackend::Auhal => "auhal",
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

fn run_backend(
    owner: crate::audio_lifecycle::AudioOwner,
    backend: CaptureBackend,
    device_id: Option<&str>,
    command_receiver: &Receiver<AudioCommand>,
    shared: &Arc<Mutex<Vec<f32>>>,
    active: &Arc<AtomicBool>,
    app_handle: &Option<tauri::AppHandle>,
    event_sender: &AudioWorkerEventSender,
) -> AttemptResult {
    // A first-ever/not-reset grant must be allowed to reach the worker's
    // device open so macOS can show the TCC prompt. Only a genuine denial is
    // terminal before spawn.
    if crate::commands::permissions::check_microphone_permission_status() == "denied" {
        return AttemptResult::Failed {
            failure: AudioFailure::new(
                AudioFailureKind::PermissionDenied,
                AudioInitPhase::StreamBuild,
            ),
            retained_audio: false,
        };
    }
    let (capture_id, nonce, nonce_hex) = capture_identity();
    let (mut child, mut input, output) = match spawn_helper(capture_id, &nonce_hex) {
        Ok(value) => value,
        Err(failure) => {
            return AttemptResult::Failed {
                failure,
                retained_audio: false,
            }
        }
    };
    let output = match hello(&mut input, output, capture_id, nonce) {
        Ok(output) => output,
        Err(failure) => {
            let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
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
            backend,
        },
    )
    .is_err()
    {
        let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
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
    let mut last_permission_check = Instant::now();
    loop {
        match command_receiver.try_recv() {
            Ok(AudioCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                let stop_started = Instant::now();
                let _ = write_production_control(
                    &mut input,
                    capture_id,
                    nonce,
                    &ProductionHostMessage::Stop,
                );
                drop(input);
                let termination = child
                    .wait_for_exit(Instant::now() + HELPER_STOP_DEADLINE)
                    .or_else(|| child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE));
                if termination.is_some() {
                    tracing::info!(
                        target: "audio",
                        capture_id,
                        stop_to_exit_ms = stop_started.elapsed().as_millis() as u64,
                        "capture helper stopped and exited"
                    );
                    let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                    return AttemptResult::Stopped;
                }
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::BackendError,
                        AudioInitPhase::Runtime,
                    ),
                    retained_audio,
                };
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        if last_permission_check.elapsed() >= PERMISSION_POLL_INTERVAL {
            last_permission_check = Instant::now();
            // During active capture, denied and not-determined both mean the
            // grant is no longer in force (not-determined is what a TCC reset
            // can expose). "unknown" is a transient probe failure and must not
            // destroy retained audio by itself.
            let permission_status =
                crate::commands::permissions::check_microphone_permission_status();
            let permission_lost = permission_status == "denied"
                || (retained_audio && permission_status == "notDetermined");
            if permission_lost {
                let _ = write_production_control(
                    &mut input,
                    capture_id,
                    nonce,
                    &ProductionHostMessage::Cancel,
                );
                drop(input);
                let _ = child
                    .wait_for_exit(Instant::now() + HELPER_STOP_DEADLINE)
                    .or_else(|| child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE));
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::PermissionDenied,
                        if retained_audio {
                            AudioInitPhase::Runtime
                        } else {
                            AudioInitPhase::StreamBuild
                        },
                    ),
                    retained_audio,
                };
            }
        }

        match reader_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(HelperRead::Frame(ProductionFrame::Pcm(pcm))) => {
                if pcm.sequence != expected_sequence
                    || pcm.samples.is_empty()
                    || sample_rate.is_some_and(|rate| rate != pcm.sample_rate)
                {
                    let _ = child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE);
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
                let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Failure {
                code,
                ..
            }))) => {
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        failure_kind(code),
                        if retained_audio {
                            AudioInitPhase::Runtime
                        } else {
                            AudioInitPhase::StreamBuild
                        },
                    ),
                    retained_audio,
                };
            }
            Ok(HelperRead::Frame(ProductionFrame::Control(ProductionHelperMessage::Stopped {
                ..
            }))) => {
                drop(input);
                let _ = child
                    .wait_for_exit(Instant::now() + HELPER_STOP_DEADLINE)
                    .or_else(|| child.hard_kill_confirmed(Instant::now() + HELPER_STOP_DEADLINE));
                let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
                return AttemptResult::Stopped;
            }
            Ok(HelperRead::Frame(_)) => {}
            Ok(HelperRead::Invalid) | Err(RecvTimeoutError::Disconnected) => {
                return AttemptResult::Failed {
                    failure: AudioFailure::new(
                        AudioFailureKind::ProtocolError,
                        if retained_audio {
                            AudioInitPhase::Runtime
                        } else {
                            AudioInitPhase::StreamBuild
                        },
                    ),
                    retained_audio,
                };
            }
            Err(RecvTimeoutError::Timeout) => {}
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
    } = spec;
    let backends = preferred_backends();
    for (attempt_index, backend) in backends.into_iter().enumerate() {
        match run_backend(
            owner,
            backend,
            device_id.as_deref(),
            &command_receiver,
            &shared,
            &active,
            &app_handle,
            event_sender,
        ) {
            AttemptResult::Stopped => return,
            AttemptResult::Failed {
                failure,
                retained_audio,
            } if !retained_audio && attempt_index == 0 && failure.permits_backend_fallback() => {
                tracing::warn!(
                    target: "audio",
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
}
