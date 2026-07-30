use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use serde::Serialize;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tauri::Emitter;

/// Compute RMS (root mean square) of a sample slice — returns 0.0–1.0 audio level.
pub fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Compute peak (max absolute value) of a sample slice — returns 0.0–1.0.
pub fn compute_peak(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
}

/// Minimum gap between `audio-level` events (~60 fps).
const AUDIO_LEVEL_THROTTLE_MS: u64 = 16;

/// CPAL applies this timeout to backend operations that expose a deadline. On
/// CoreAudio 0.18 that includes sample-rate convergence, but not every
/// synchronous AudioUnit operation; the lifecycle supervisor therefore retains
/// strict ownership until the worker actually exits.
pub(crate) const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(10);

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
    FirstBufferTimeout,
    InitializationTimeout,
    WorkerPanicked,
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
            Self::FirstBufferTimeout => "first_buffer_timeout",
            Self::InitializationTimeout => "initialization_timeout",
            Self::WorkerPanicked => "worker_panicked",
        }
    }

    fn from_cpal(kind: cpal::ErrorKind) -> Self {
        match kind {
            cpal::ErrorKind::PermissionDenied => Self::PermissionDenied,
            cpal::ErrorKind::DeviceNotAvailable => Self::DeviceUnavailable,
            cpal::ErrorKind::DeviceBusy => Self::DeviceBusy,
            cpal::ErrorKind::DeviceChanged => Self::DeviceChanged,
            cpal::ErrorKind::HostUnavailable => Self::HostUnavailable,
            cpal::ErrorKind::InvalidInput => Self::InvalidInput,
            cpal::ErrorKind::ResourceExhausted => Self::ResourceExhausted,
            cpal::ErrorKind::StreamInvalidated => Self::StreamInvalidated,
            cpal::ErrorKind::UnsupportedConfig => Self::UnsupportedConfig,
            cpal::ErrorKind::UnsupportedOperation => Self::UnsupportedOperation,
            cpal::ErrorKind::RealtimeDenied => Self::RealtimeDenied,
            cpal::ErrorKind::Xrun => Self::Xrun,
            cpal::ErrorKind::BackendError | cpal::ErrorKind::Other => Self::BackendError,
            _ => Self::BackendError,
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

    fn from_cpal(phase: AudioInitPhase, error: cpal::Error) -> Self {
        Self::new(AudioFailureKind::from_cpal(error.kind()), phase)
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
            AudioFailureKind::HostUnavailable
            | AudioFailureKind::UnsupportedOperation
            | AudioFailureKind::RealtimeDenied
            | AudioFailureKind::Xrun
            | AudioFailureKind::BackendError
            | AudioFailureKind::WorkerPanicked => "Microphone capture failed. Try recording again.",
        }
    }
}

impl fmt::Display for AudioFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.user_message())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceDescriptor {
    /// Backend-native stable identifier. On CoreAudio this is the raw
    /// kAudioDevicePropertyDeviceUID with no CPAL host prefix.
    pub id: String,
    /// Presentation-only display name.
    pub name: String,
}

fn descriptor_for(device: &cpal::Device) -> Result<AudioDeviceDescriptor, AudioFailure> {
    let id = device
        .id()
        .map_err(|error| AudioFailure::from_cpal(AudioInitPhase::DeviceEnumeration, error))?
        .id()
        .to_string();
    let name = device
        .description()
        .map_err(|error| AudioFailure::from_cpal(AudioInitPhase::DeviceEnumeration, error))?
        .name()
        .to_string();
    Ok(AudioDeviceDescriptor { id, name })
}

fn select_explicit_device_index<T>(
    requested_id: &str,
    devices: &[(T, AudioDeviceDescriptor)],
) -> Result<usize, AudioFailure> {
    if let Some(index) = devices
        .iter()
        .position(|(_, descriptor)| descriptor.id == requested_id)
    {
        return Ok(index);
    }
    let mut legacy_matches = devices
        .iter()
        .enumerate()
        .filter(|(_, (_, descriptor))| descriptor.name == requested_id)
        .map(|(index, _)| index);
    match (legacy_matches.next(), legacy_matches.next()) {
        (Some(index), None) => Ok(index),
        _ => Err(AudioFailure::new(
            AudioFailureKind::DeviceUnavailable,
            AudioInitPhase::DeviceEnumeration,
        )),
    }
}

fn mono_from_samples<T>(data: &[T], channels: usize) -> Vec<f32>
where
    T: Sample + Copy,
    f32: FromSample<T>,
{
    data.chunks(channels)
        .map(|chunk| {
            let sum: f32 = chunk.iter().copied().map(f32::from_sample).sum();
            sum / channels as f32
        })
        .collect()
}

fn build_mono_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Vec<f32>>>,
    channels: usize,
    app_handle: Option<tauri::AppHandle>,
    publish_levels: Arc<AtomicBool>,
    first_buffer_seen: Arc<AtomicBool>,
    event_sender: AudioWorkerEventSender,
    owner: crate::audio_lifecycle::AudioOwner,
    sample_rate: u32,
) -> Result<cpal::Stream, AudioFailure>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let first_buffer_sender = event_sender.clone();
    let runtime_error_sender = event_sender;
    let last_emit_ms = std::sync::atomic::AtomicU64::new(0);
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &_| {
                if data.is_empty() {
                    return;
                }
                let mono = mono_from_samples(data, channels);
                if mono.is_empty() {
                    return;
                }

                // Retention precedes readiness. This buffer and any later
                // buffers received before supervisor acceptance remain owned
                // by this generation and are never dropped.
                shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend_from_slice(&mono);

                if !first_buffer_seen.swap(true, Ordering::AcqRel) {
                    let _ = first_buffer_sender
                        .send(AudioWorkerEvent::FirstBuffer { owner, sample_rate });
                }

                // Waveform publication is a separate generation gate. A stale
                // or recovering worker may retain only its private buffer but
                // can never publish UI state.
                if !publish_levels.load(Ordering::Acquire) {
                    return;
                }
                if let Some(ref handle) = app_handle {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let last = last_emit_ms.load(Ordering::Relaxed);
                    if now.saturating_sub(last) >= AUDIO_LEVEL_THROTTLE_MS {
                        last_emit_ms.store(now, Ordering::Relaxed);
                        let _ = handle.emit("audio-level", compute_rms(&mono));
                    }
                }
            },
            move |error| {
                let failure = AudioFailure::from_cpal(AudioInitPhase::Runtime, error);
                let _ =
                    runtime_error_sender.send(AudioWorkerEvent::RuntimeFailed { owner, failure });
            },
            Some(STREAM_BUILD_TIMEOUT),
        )
        .map_err(|error| AudioFailure::from_cpal(AudioInitPhase::StreamBuild, error))
}

/// Commands sent by the lifecycle supervisor to the single capture worker.
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

/// Cloneable worker-side handle that enqueues lifecycle events directly on the
/// supervisor's command queue. Keeping worker events and stop/cancel/deadline
/// messages on one queue gives the supervisor a single linearization order.
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

/// List available inputs as stable IDs plus presentation-only display names.
pub fn list_input_devices() -> Result<Vec<AudioDeviceDescriptor>, String> {
    let host = cpal::default_host();
    let devices = host.input_devices().map_err(|error| {
        AudioFailure::from_cpal(AudioInitPhase::DeviceEnumeration, error).to_string()
    })?;
    devices
        .map(|device| descriptor_for(&device).map_err(|failure| failure.to_string()))
        .collect()
}

/// Start transform instruction audio asynchronously under the shared
/// supervisor. The transform flow enters its public Listening state only
/// after the matching lifecycle Ready event is accepted.
pub fn start_transform_capture_audio(
    app_handle: Option<tauri::AppHandle>,
    device_id: Option<String>,
    transform_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_transform_recording(app_handle, device_id, transform_pass_id)
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
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        error_kind = error.kind.as_str(),
                        phase = error.phase.as_str(),
                        "audio capture worker failed"
                    );
                    let _ = event_sender.send(AudioWorkerEvent::InitFailed {
                        owner,
                        failure: error,
                    });
                }
                Err(_) => {
                    let failure = AudioFailure::new(
                        AudioFailureKind::WorkerPanicked,
                        AudioInitPhase::Runtime,
                    );
                    tracing::error!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        error_kind = failure.kind.as_str(),
                        phase = failure.phase.as_str(),
                        "audio capture worker panicked"
                    );
                    let _ = event_sender.send(AudioWorkerEvent::InitFailed { owner, failure });
                }
            }
            let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
        })
        .map_err(|error| format!("Failed to spawn audio thread: {error}"))
}

fn timed_phase<T>(
    owner: crate::audio_lifecycle::AudioOwner,
    phase: AudioInitPhase,
    event_sender: &AudioWorkerEventSender,
    operation: impl FnOnce() -> Result<T, AudioFailure>,
) -> Result<T, AudioFailure> {
    let started = std::time::Instant::now();
    let _ = event_sender.send(AudioWorkerEvent::PhaseEntered { owner, phase });
    let result = operation();
    let _ = event_sender.send(AudioWorkerEvent::PhaseExited {
        owner,
        phase,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    result
}

fn run_audio_capture(
    spec: AudioWorkerSpec,
    event_sender: &AudioWorkerEventSender,
) -> Result<(), AudioFailure> {
    let AudioWorkerSpec {
        owner,
        command_receiver,
        shared,
        active,
        app_handle,
        device_id,
    } = spec;
    let host = cpal::default_host();

    let device = timed_phase(
        owner,
        AudioInitPhase::DeviceEnumeration,
        event_sender,
        || {
            if let Some(ref requested_id) = device_id {
                let devices = host.input_devices().map_err(|error| {
                    AudioFailure::from_cpal(AudioInitPhase::DeviceEnumeration, error)
                })?;
                let candidates = devices
                    .map(|device| descriptor_for(&device).map(|descriptor| (device, descriptor)))
                    .collect::<Result<Vec<_>, _>>()?;
                let index = select_explicit_device_index(requested_id, &candidates)?;
                Ok(candidates
                    .into_iter()
                    .nth(index)
                    .expect("selected audio device index must exist")
                    .0)
            } else {
                host.default_input_device().ok_or_else(|| {
                    AudioFailure::new(
                        AudioFailureKind::DeviceUnavailable,
                        AudioInitPhase::DeviceEnumeration,
                    )
                })
            }
        },
    )?;

    let config = timed_phase(owner, AudioInitPhase::ConfigLookup, event_sender, || {
        device
            .default_input_config()
            .map_err(|error| AudioFailure::from_cpal(AudioInitPhase::ConfigLookup, error))
    })?;
    let device_sample_rate = config.sample_rate();
    let sample_format = config.sample_format();
    let channels = config.channels() as usize;

    tracing::info!(
        target: "audio",
        owner = owner.telemetry_id(),
        device_selection = if device_id.is_some() { "explicit" } else { "system_default" },
        sample_rate = device_sample_rate,
        channels,
        format = ?sample_format,
        "run_audio_capture"
    );

    let stream_config = config.config();
    let first_buffer_seen = Arc::new(AtomicBool::new(false));
    macro_rules! build_for {
        ($sample:ty) => {
            build_mono_input_stream::<$sample>(
                &device,
                stream_config,
                Arc::clone(&shared),
                channels,
                app_handle.clone(),
                Arc::clone(&active),
                Arc::clone(&first_buffer_seen),
                event_sender.clone(),
                owner,
                device_sample_rate,
            )
        };
    }
    let stream =
        timed_phase(
            owner,
            AudioInitPhase::StreamBuild,
            event_sender,
            || match sample_format {
                SampleFormat::I8 => build_for!(i8),
                SampleFormat::I16 => build_for!(i16),
                SampleFormat::I24 => build_for!(cpal::I24),
                SampleFormat::I32 => build_for!(i32),
                SampleFormat::I64 => build_for!(i64),
                SampleFormat::U8 => build_for!(u8),
                SampleFormat::U16 => build_for!(u16),
                SampleFormat::U24 => build_for!(cpal::U24),
                SampleFormat::U32 => build_for!(u32),
                SampleFormat::U64 => build_for!(u64),
                SampleFormat::F32 => build_for!(f32),
                SampleFormat::F64 => build_for!(f64),
                SampleFormat::DsdU8 | SampleFormat::DsdU16 | SampleFormat::DsdU32 => {
                    Err(AudioFailure::new(
                        AudioFailureKind::UnsupportedConfig,
                        AudioInitPhase::StreamBuild,
                    ))
                }
                _ => Err(AudioFailure::new(
                    AudioFailureKind::UnsupportedConfig,
                    AudioInitPhase::StreamBuild,
                )),
            },
        )?;

    if command_receiver.try_recv().is_ok() {
        return Ok(());
    }

    timed_phase(owner, AudioInitPhase::StreamPlay, event_sender, || {
        stream
            .play()
            .map_err(|error| AudioFailure::from_cpal(AudioInitPhase::StreamPlay, error))
    })?;

    let _ = event_sender.send(AudioWorkerEvent::PhaseEntered {
        owner,
        phase: AudioInitPhase::FirstBufferWait,
    });

    loop {
        match command_receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(AudioCommand::Stop) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = stream.pause();
    let _ = event_sender.send(AudioWorkerEvent::StreamStopped { owner });
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_empty_slice_returns_zero() {
        assert_eq!(compute_rms(&[]), 0.0);
    }

    #[test]
    fn rms_silence_is_zero() {
        assert_eq!(compute_rms(&[0.0f32; 100]), 0.0);
    }

    #[test]
    fn rms_full_amplitude_is_one() {
        let result = compute_rms(&[1.0f32; 100]);
        assert!((result - 1.0).abs() < 1e-6, "expected 1.0, got {result}");
    }

    #[test]
    fn rms_alternating_signs_is_one() {
        let result = compute_rms(&[1.0f32, -1.0, 1.0, -1.0]);
        assert!((result - 1.0).abs() < 1e-6, "expected 1.0, got {result}");
    }

    #[test]
    fn rms_half_amplitude() {
        let result = compute_rms(&[0.5f32; 100]);
        assert!((result - 0.5).abs() < 1e-6, "expected 0.5, got {result}");
    }

    #[test]
    fn rms_single_sample() {
        assert!((compute_rms(&[0.6f32]) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn peak_empty_slice_returns_zero() {
        assert_eq!(compute_peak(&[]), 0.0);
    }

    #[test]
    fn peak_silence_is_zero() {
        assert_eq!(compute_peak(&[0.0f32; 100]), 0.0);
    }

    #[test]
    fn peak_positive() {
        assert!((compute_peak(&[0.1f32, 0.5, 0.3, 0.2]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn peak_negative() {
        assert!((compute_peak(&[0.1f32, -0.8, 0.3, 0.2]) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn mono_conversion_supports_i24_and_i32() {
        let i24 = [
            cpal::I24::from_sample(0.5_f32),
            cpal::I24::from_sample(-0.5_f32),
            cpal::I24::from_sample(0.25_f32),
            cpal::I24::from_sample(0.25_f32),
        ];
        let i24_mono = mono_from_samples(&i24, 2);
        assert!(i24_mono[0].abs() < 1e-5);
        assert!((i24_mono[1] - 0.25).abs() < 1e-5);

        let i32_samples = [
            i32::from_sample(0.5_f32),
            i32::from_sample(-0.5_f32),
            i32::from_sample(0.25_f32),
            i32::from_sample(0.25_f32),
        ];
        let i32_mono = mono_from_samples(&i32_samples, 2);
        assert!(i32_mono[0].abs() < 1e-5);
        assert!((i32_mono[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn explicit_device_selection_prefers_raw_id_and_legacy_names_must_be_unique() {
        let candidates = vec![
            (
                (),
                AudioDeviceDescriptor {
                    id: "raw-uid-a".to_string(),
                    name: "Studio Mic".to_string(),
                },
            ),
            (
                (),
                AudioDeviceDescriptor {
                    id: "raw-uid-b".to_string(),
                    name: "Studio Mic".to_string(),
                },
            ),
        ];
        assert_eq!(
            select_explicit_device_index("raw-uid-b", &candidates).unwrap(),
            1
        );
        assert_eq!(
            select_explicit_device_index("Studio Mic", &candidates)
                .unwrap_err()
                .kind,
            AudioFailureKind::DeviceUnavailable
        );
        assert_eq!(
            select_explicit_device_index("missing", &candidates)
                .unwrap_err()
                .kind,
            AudioFailureKind::DeviceUnavailable
        );
    }

    #[test]
    fn cpal_error_mapping_is_typed_and_messages_never_include_backend_content() {
        for (cpal_kind, expected) in [
            (
                cpal::ErrorKind::PermissionDenied,
                AudioFailureKind::PermissionDenied,
            ),
            (
                cpal::ErrorKind::DeviceNotAvailable,
                AudioFailureKind::DeviceUnavailable,
            ),
            (cpal::ErrorKind::DeviceBusy, AudioFailureKind::DeviceBusy),
            (
                cpal::ErrorKind::DeviceChanged,
                AudioFailureKind::DeviceChanged,
            ),
            (
                cpal::ErrorKind::StreamInvalidated,
                AudioFailureKind::StreamInvalidated,
            ),
            (
                cpal::ErrorKind::InvalidInput,
                AudioFailureKind::InvalidInput,
            ),
            (
                cpal::ErrorKind::ResourceExhausted,
                AudioFailureKind::ResourceExhausted,
            ),
        ] {
            let failure = AudioFailure::from_cpal(
                AudioInitPhase::StreamBuild,
                cpal::Error::with_message(cpal_kind, "secret device label and raw backend detail"),
            );
            assert_eq!(failure.kind, expected);
            assert!(!failure.to_string().contains("secret"));
            assert!(!failure.to_string().contains("backend detail"));
        }
    }
}

/// Linear-interpolation resample from `from_rate` to `to_rate`.
/// Used by both live capture (`stop_recording`) and file decoding (`audio_decode`).
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let new_len = (samples.len() as f64 / ratio) as usize;
    let mut resampled = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;
        let sample = if idx + 1 < samples.len() {
            samples[idx] * (1.0 - frac as f32) + samples[idx + 1] * frac as f32
        } else if idx < samples.len() {
            samples[idx]
        } else {
            0.0
        };
        resampled.push(sample);
    }

    resampled
}
