use crate::state::WHISPER_SAMPLE_RATE;
use crate::{log_error, log_info, log_warn};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use std::sync::mpsc::{channel, Receiver, Sender};
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

/// Build an input stream that converts interleaved multi-channel samples to mono f32,
/// computes RMS for each buffer chunk and emits an "audio-level" event if an AppHandle
/// is provided.
/// Minimum gap between `audio-level` events (~60 fps).
const AUDIO_LEVEL_THROTTLE_MS: u64 = 16;

/// Build an input stream that converts interleaved multi-channel samples to mono f32,
/// computes RMS for each buffer chunk and emits an "audio-level" event if an AppHandle
/// is provided, throttled to ~60 fps to avoid IPC spam.
macro_rules! build_mono_input_stream {
    ($device:expr, $config:expr, $shared:expr, $channels:expr, $err_fn:expr, $sample_type:ty, $app_handle:expr) => {{
        let samples_ref = Arc::clone(&$shared);
        let app_handle_opt: Option<tauri::AppHandle> = $app_handle;
        let last_emit_ms = std::sync::atomic::AtomicU64::new(0);
        $device.build_input_stream(
            &$config.into(),
            move |data: &[$sample_type], _: &_| {
                let mono: Vec<f32> = data.chunks($channels)
                    .map(|chunk| {
                        let sum: f32 = chunk.iter().map(|&s| s.to_float_sample()).sum();
                        sum / $channels as f32
                    })
                    .collect();

                // Emit audio level throttled to ~60 fps
                if let Some(ref handle) = app_handle_opt {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let last = last_emit_ms.load(std::sync::atomic::Ordering::Relaxed);
                    if now.saturating_sub(last) >= AUDIO_LEVEL_THROTTLE_MS {
                        last_emit_ms.store(now, std::sync::atomic::Ordering::Relaxed);
                        let rms = compute_rms(&mono);
                        let _ = handle.emit("audio-level", rms);
                    }
                }

                if let Ok(mut s) = samples_ref.lock() {
                    s.extend(mono);
                }
            },
            $err_fn,
            None,
        ).map_err(|e| format!("Failed to build stream: {}", e))?
    }};
}

// Commands to send to the audio thread
enum AudioCommand {
    Stop,
}

// Global state — the OnceLock holds the RecordingState container, but the sample
// buffer inside is created fresh for each recording (Option<Arc<...>>).
static RECORDING_STATE: std::sync::OnceLock<Mutex<RecordingState>> = std::sync::OnceLock::new();

struct RecordingState {
    command_sender: Option<Sender<AudioCommand>>,
    thread_handle: Option<JoinHandle<()>>,
    /// Per-recording sample buffer. Created fresh on start, taken on stop.
    /// The cpal callback holds an Arc clone — when the stream drops and the
    /// thread joins, that clone drops too. Next recording gets a fresh Vec.
    shared: Option<Arc<Mutex<Vec<f32>>>>,
    /// Device sample rate for the current/last recording.
    sample_rate: u32,
    /// Wall-clock instant when recording started.
    started_at: Option<std::time::Instant>,
    /// Name of the audio input device used for the current/last recording.
    device_name: Option<String>,
}

fn get_state() -> &'static Mutex<RecordingState> {
    RECORDING_STATE.get_or_init(|| {
        Mutex::new(RecordingState {
            command_sender: None,
            thread_handle: None,
            shared: None,
            sample_rate: WHISPER_SAMPLE_RATE,
            started_at: None,
            device_name: None,
        })
    })
}

/// List available input device names.
pub fn list_input_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let devices = host.input_devices()
        .map_err(|e| format!("Failed to enumerate input devices: {}", e))?;
    let names: Vec<String> = devices
        .filter_map(|d| d.name().ok())
        .collect();
    Ok(names)
}

pub fn start_recording(app_handle: Option<tauri::AppHandle>, device_name: Option<String>) -> Result<(), String> {
    let state = get_state();
    let mut state_guard = state.lock().unwrap_or_else(|poisoned| {
        log_warn!("start_recording: recording state mutex was poisoned, recovering");
        poisoned.into_inner()
    });

    // Stop any existing recording
    if state_guard.command_sender.is_some() {
        drop(state_guard);
        stop_recording()?;
        state_guard = state.lock().unwrap_or_else(|poisoned| {
            log_warn!("start_recording: recording state mutex was poisoned, recovering");
            poisoned.into_inner()
        });
    }

    // Create a brand-new buffer for this recording — no stale data possible
    let new_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
    state_guard.shared = Some(Arc::clone(&new_buffer));
    log_info!("start_recording: created fresh sample buffer");

    let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
    let (ready_tx, ready_rx) = channel::<Result<(u32, String), String>>();

    // Spawn audio thread
    let handle = thread::spawn(move || {
        if let Err(e) = run_audio_capture(cmd_rx, new_buffer, ready_tx.clone(), app_handle, device_name) {
            log_error!("Audio capture error: {}", e);
            let _ = ready_tx.send(Err(e));
        }
    });

    state_guard.command_sender = Some(cmd_tx);
    state_guard.thread_handle = Some(handle);

    // Wait for thread to signal ready (with timeout)
    let init_result = match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok((device_sample_rate, actual_device_name))) => {
            state_guard.sample_rate = device_sample_rate;
            state_guard.device_name = Some(actual_device_name);
            state_guard.started_at = Some(std::time::Instant::now());
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Audio thread failed to initialize within timeout".to_string()),
    };

    if init_result.is_err() {
        if let Some(sender) = state_guard.command_sender.take() {
            let _ = sender.send(AudioCommand::Stop);
        }
        state_guard.thread_handle.take();
        state_guard.shared.take();
    }

    init_result
}

fn run_audio_capture(
    cmd_rx: Receiver<AudioCommand>,
    shared: Arc<Mutex<Vec<f32>>>,
    ready_tx: Sender<Result<(u32, String), String>>,
    app_handle: Option<tauri::AppHandle>,
    device_name: Option<String>,
) -> Result<(), String> {
    let host = cpal::default_host();

    let device = if let Some(ref name) = device_name {
        match host.input_devices() {
            Ok(mut devices) => {
                match devices.find(|d| d.name().ok().as_deref() == Some(name)) {
                    Some(d) => d,
                    None => {
                        log_warn!("Requested device '{}' not found, falling back to default", name);
                        host.default_input_device()
                            .ok_or_else(|| "No input device available. Please grant microphone permission.".to_string())?
                    }
                }
            }
            Err(e) => {
                log_warn!("Failed to enumerate devices: {}, falling back to default", e);
                host.default_input_device()
                    .ok_or_else(|| "No input device available. Please grant microphone permission.".to_string())?
            }
        }
    } else {
        host.default_input_device()
            .ok_or_else(|| "No input device available. Please grant microphone permission.".to_string())?
    };

    let actual_name = device.name().unwrap_or_else(|_| "unknown".to_string());

    let config = device.default_input_config()
        .map_err(|e| format!("Failed to get input config: {}", e))?;

    let device_sample_rate = config.sample_rate().0;
    let sample_format = config.sample_format();
    let channels = config.channels() as usize;

    log_info!("run_audio_capture: device='{}', sample_rate={}, channels={}, format={:?}",
        actual_name, device_sample_rate, channels, sample_format);

    let err_fn = |err| log_error!("Audio stream error: {}", err);

    let stream = match sample_format {
        SampleFormat::F32 => build_mono_input_stream!(device, config, shared, channels, err_fn, f32, app_handle.clone()),
        SampleFormat::I16 => build_mono_input_stream!(device, config, shared, channels, err_fn, i16, app_handle),
        _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
    };

    stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;

    // Signal ready with the device sample rate and name
    let _ = ready_tx.send(Ok((device_sample_rate, actual_name.clone())));

    // Wait for stop command
    loop {
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(AudioCommand::Stop) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

pub fn stop_recording() -> Result<Vec<f32>, String> {
    let state = get_state();
    let mut state_guard = state.lock().unwrap_or_else(|poisoned| {
        log_warn!("stop_recording: recording state mutex was poisoned, recovering");
        poisoned.into_inner()
    });

    // Send stop command
    if let Some(sender) = state_guard.command_sender.take() {
        let _ = sender.send(AudioCommand::Stop);
    }

    // Wait for thread to finish — this also drops the cpal stream and
    // the callback's Arc clone, so no more writes to the buffer.
    if let Some(handle) = state_guard.thread_handle.take() {
        let _ = handle.join();
    }

    // Take this recording's buffer — leaves None for next recording
    let buffer = state_guard.shared.take();
    let started_at = state_guard.started_at.take();
    let sample_rate = state_guard.sample_rate;

    let samples = if let Some(buf) = buffer {
        let guard = buf.lock().unwrap_or_else(|poisoned| {
            log_warn!("stop_recording: samples mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let raw_count = guard.len();
        let raw_duration = if sample_rate > 0 { raw_count as f64 / sample_rate as f64 } else { 0.0 };
        if let Some(started) = started_at {
            log_info!("stop_recording: raw_samples={}, sample_rate={}, wall_secs={:.1}, audio_secs={:.1}",
                raw_count, sample_rate, started.elapsed().as_secs_f64(), raw_duration);
        } else {
            log_info!("stop_recording: raw_samples={}, sample_rate={}, duration_secs={:.1} (no timestamp)",
                raw_count, sample_rate, raw_duration);
        }
        guard.clone()
        // guard drops, buf drops — buffer is gone, zero stale data
    } else {
        log_info!("stop_recording: no buffer (was not recording)");
        Vec::new()
    };

    // Resample to Whisper's required sample rate if needed
    if sample_rate != WHISPER_SAMPLE_RATE && !samples.is_empty() {
        Ok(resample(&samples, sample_rate, WHISPER_SAMPLE_RATE))
    } else {
        Ok(samples)
    }
}

#[allow(dead_code)]
pub fn is_recording() -> bool {
    if let Some(state) = RECORDING_STATE.get() {
        if let Ok(guard) = state.lock() {
            return guard.command_sender.is_some();
        }
    }
    false
}

/// Return the device name from the most recent recording session.
pub fn last_device_name() -> Option<String> {
    if let Some(state) = RECORDING_STATE.get() {
        if let Ok(guard) = state.lock() {
            return guard.device_name.clone();
        }
    }
    None
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
        let result = compute_rms(&[0.0f32; 100]);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn rms_full_amplitude_is_one() {
        let samples = vec![1.0f32; 100];
        let result = compute_rms(&samples);
        assert!((result - 1.0).abs() < 1e-6, "expected 1.0, got {result}");
    }

    #[test]
    fn rms_alternating_signs_is_one() {
        // [1, -1, 1, -1] → each sample squared = 1, mean = 1, sqrt = 1
        let samples = vec![1.0f32, -1.0, 1.0, -1.0];
        let result = compute_rms(&samples);
        assert!((result - 1.0).abs() < 1e-6, "expected 1.0, got {result}");
    }

    #[test]
    fn rms_half_amplitude() {
        let samples = vec![0.5f32; 100];
        let result = compute_rms(&samples);
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
        let samples = vec![0.1f32, 0.5, 0.3, 0.2];
        assert!((compute_peak(&samples) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn peak_negative() {
        let samples = vec![0.1f32, -0.8, 0.3, 0.2];
        assert!((compute_peak(&samples) - 0.8).abs() < 1e-6);
    }
}

fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
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
