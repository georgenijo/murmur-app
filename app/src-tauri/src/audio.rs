use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
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

/// Build an input stream that converts interleaved multi-channel samples to mono f32,
/// computes RMS for each buffer chunk and emits an "audio-level" event if an AppHandle
/// is provided, throttled to ~60 fps to avoid IPC spam.
macro_rules! build_mono_input_stream {
    ($device:expr, $config:expr, $shared:expr, $channels:expr, $err_fn:expr, $sample_type:ty, $app_handle:expr, $active:expr) => {{
        let samples_ref = Arc::clone(&$shared);
        let active_ref = Arc::clone(&$active);
        let app_handle_opt: Option<tauri::AppHandle> = $app_handle;
        let last_emit_ms = std::sync::atomic::AtomicU64::new(0);
        $device
            .build_input_stream(
                &$config.into(),
                move |data: &[$sample_type], _: &_| {
                    // Starting/recovering generations keep this false. That
                    // suppresses samples and level events until the supervisor
                    // accepts readiness for the current owner.
                    if !active_ref.load(Ordering::Relaxed) {
                        return;
                    }

                    let mono: Vec<f32> = data
                        .chunks($channels)
                        .map(|chunk| {
                            let sum: f32 = chunk.iter().map(|&s| s.to_float_sample()).sum();
                            sum / $channels as f32
                        })
                        .collect();

                    if let Some(ref handle) = app_handle_opt {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        let last = last_emit_ms.load(Ordering::Relaxed);
                        if now.saturating_sub(last) >= AUDIO_LEVEL_THROTTLE_MS {
                            last_emit_ms.store(now, Ordering::Relaxed);
                            let rms = compute_rms(&mono);
                            let _ = handle.emit("audio-level", rms);
                        }
                    }

                    if let Ok(mut samples) = samples_ref.lock() {
                        samples.extend(mono);
                    }
                },
                $err_fn,
                None,
            )
            .map_err(|error| format!("Failed to build stream: {error}"))?
    }};
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
    ReadySignal,
}

impl AudioInitPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DeviceEnumeration => "device_enumeration",
            Self::ConfigLookup => "config_lookup",
            Self::StreamBuild => "stream_build",
            Self::StreamPlay => "stream_play",
            Self::ReadySignal => "ready_signal",
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
    Ready {
        owner: crate::audio_lifecycle::AudioOwner,
        sample_rate: u32,
        device_name: String,
    },
    InitFailed {
        owner: crate::audio_lifecycle::AudioOwner,
        error: String,
    },
    StreamStopped {
        owner: crate::audio_lifecycle::AudioOwner,
    },
    ThreadExited {
        owner: crate::audio_lifecycle::AudioOwner,
    },
}

pub(crate) struct AudioWorkerSpec {
    pub owner: crate::audio_lifecycle::AudioOwner,
    pub command_receiver: Receiver<AudioCommand>,
    pub shared: Arc<Mutex<Vec<f32>>>,
    pub active: Arc<AtomicBool>,
    pub app_handle: Option<tauri::AppHandle>,
    pub device_name: Option<String>,
}

/// List available input device names.
pub fn list_input_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|error| format!("Failed to enumerate input devices: {error}"))?;
    Ok(devices.filter_map(|device| device.name().ok()).collect())
}

/// Start transform instruction audio asynchronously under the shared
/// supervisor. The transform flow enters its public Listening state only
/// after the matching lifecycle Ready event is accepted.
pub fn start_transform_capture_audio(
    app_handle: Option<tauri::AppHandle>,
    device_name: Option<String>,
    transform_pass_id: u64,
) -> Result<(), String> {
    crate::audio_lifecycle::start_transform_recording(app_handle, device_name, transform_pass_id)
}

pub(crate) fn spawn_capture_worker(
    spec: AudioWorkerSpec,
    event_sender: Sender<AudioWorkerEvent>,
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
                        "Audio capture error: {}",
                        error
                    );
                    let _ = event_sender.send(AudioWorkerEvent::InitFailed { owner, error });
                }
                Err(_) => {
                    let error = "Audio capture thread panicked".to_string();
                    tracing::error!(
                        target: "audio",
                        owner = owner.telemetry_id(),
                        "{}",
                        error
                    );
                    let _ = event_sender.send(AudioWorkerEvent::InitFailed { owner, error });
                }
            }
            let _ = event_sender.send(AudioWorkerEvent::ThreadExited { owner });
        })
        .map_err(|error| format!("Failed to spawn audio thread: {error}"))
}

fn timed_phase<T>(
    owner: crate::audio_lifecycle::AudioOwner,
    phase: AudioInitPhase,
    event_sender: &Sender<AudioWorkerEvent>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
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
    event_sender: &Sender<AudioWorkerEvent>,
) -> Result<(), String> {
    let AudioWorkerSpec {
        owner,
        command_receiver,
        shared,
        active,
        app_handle,
        device_name,
    } = spec;
    let host = cpal::default_host();

    let device = timed_phase(
        owner,
        AudioInitPhase::DeviceEnumeration,
        event_sender,
        || {
            if let Some(ref requested_name) = device_name {
                match host.input_devices() {
                    Ok(mut devices) => match devices
                        .find(|device| device.name().ok().as_deref() == Some(requested_name))
                    {
                        Some(device) => Ok(device),
                        None => {
                            tracing::warn!(
                                target: "audio",
                                "Requested device '{}' not found, falling back to default",
                                requested_name
                            );
                            host.default_input_device().ok_or_else(|| {
                                "No input device available. Please grant microphone permission."
                                    .to_string()
                            })
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: "audio",
                            "Failed to enumerate devices: {}, falling back to default",
                            error
                        );
                        host.default_input_device().ok_or_else(|| {
                            "No input device available. Please grant microphone permission."
                                .to_string()
                        })
                    }
                }
            } else {
                host.default_input_device().ok_or_else(|| {
                    "No input device available. Please grant microphone permission.".to_string()
                })
            }
        },
    )?;

    let actual_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    let config = timed_phase(owner, AudioInitPhase::ConfigLookup, event_sender, || {
        device
            .default_input_config()
            .map_err(|error| format!("Failed to get input config: {error}"))
    })?;
    let device_sample_rate = config.sample_rate().0;
    let sample_format = config.sample_format();
    let channels = config.channels() as usize;

    let telemetry_device = if cfg!(debug_assertions) {
        actual_name.clone()
    } else {
        "<redacted>".to_string()
    };
    tracing::info!(
        target: "audio",
        owner = owner.telemetry_id(),
        device = telemetry_device,
        sample_rate = device_sample_rate,
        channels,
        format = ?sample_format,
        "run_audio_capture"
    );

    let err_fn = |error| tracing::error!(target: "audio", "Audio stream error: {}", error);
    let stream =
        timed_phase(
            owner,
            AudioInitPhase::StreamBuild,
            event_sender,
            || match sample_format {
                SampleFormat::F32 => Ok(build_mono_input_stream!(
                    device,
                    config,
                    shared,
                    channels,
                    err_fn,
                    f32,
                    app_handle.clone(),
                    active
                )),
                SampleFormat::I16 => Ok(build_mono_input_stream!(
                    device, config, shared, channels, err_fn, i16, app_handle, active
                )),
                _ => Err(format!("Unsupported sample format: {sample_format:?}")),
            },
        )?;

    timed_phase(owner, AudioInitPhase::StreamPlay, event_sender, || {
        stream
            .play()
            .map_err(|error| format!("Failed to start stream: {error}"))
    })?;

    timed_phase(owner, AudioInitPhase::ReadySignal, event_sender, || {
        event_sender
            .send(AudioWorkerEvent::Ready {
                owner,
                sample_rate: device_sample_rate,
                device_name: actual_name,
            })
            .map_err(|_| "Audio lifecycle supervisor stopped".to_string())
    })?;

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

/// Return the device name from the most recent recording session.
pub fn last_device_name() -> Option<String> {
    crate::audio_lifecycle::last_device_name()
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
