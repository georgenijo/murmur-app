use crate::state::WHISPER_SAMPLE_RATE;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Build an input stream that converts interleaved multi-channel samples to mono f32.
macro_rules! build_mono_input_stream {
    ($device:expr, $config:expr, $shared:expr, $channels:expr, $err_fn:expr, $sample_type:ty) => {{
        let samples_ref = Arc::clone(&$shared);
        $device.build_input_stream(
            &$config.into(),
            move |data: &[$sample_type], _: &_| {
                let mono: Vec<f32> = data.chunks($channels)
                    .map(|chunk| {
                        let sum: f32 = chunk.iter().map(|&s| s.to_float_sample()).sum();
                        sum / $channels as f32
                    })
                    .collect();
                if let Ok(mut s) = samples_ref.samples.lock() {
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

// Shared state for collecting samples
struct SharedSamples {
    samples: Mutex<Vec<f32>>,
    sample_rate: Mutex<u32>,
}

// Global state
static RECORDING_STATE: std::sync::OnceLock<Mutex<RecordingState>> = std::sync::OnceLock::new();

struct RecordingState {
    command_sender: Option<Sender<AudioCommand>>,
    thread_handle: Option<JoinHandle<()>>,
    shared: Arc<SharedSamples>,
}

fn get_state() -> &'static Mutex<RecordingState> {
    RECORDING_STATE.get_or_init(|| {
        Mutex::new(RecordingState {
            command_sender: None,
            thread_handle: None,
            shared: Arc::new(SharedSamples {
                samples: Mutex::new(Vec::new()),
                sample_rate: Mutex::new(WHISPER_SAMPLE_RATE),
            }),
        })
    })
}

pub fn start_recording() -> Result<(), String> {
    let state = get_state();
    let mut state_guard = state.lock().unwrap_or_else(|poisoned| {
        eprintln!("Warning: Recording state mutex was poisoned, recovering");
        poisoned.into_inner()
    });

    // Stop any existing recording
    if state_guard.command_sender.is_some() {
        drop(state_guard);
        stop_recording()?;
        state_guard = state.lock().unwrap_or_else(|poisoned| {
            eprintln!("Warning: Recording state mutex was poisoned, recovering");
            poisoned.into_inner()
        });
    }

    // Clear previous samples
    if let Ok(mut samples) = state_guard.shared.samples.lock() {
        samples.clear();
    }

    let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
    let (ready_tx, ready_rx) = channel::<Result<(), String>>();
    let shared = Arc::clone(&state_guard.shared);

    // Spawn audio thread
    let handle = thread::spawn(move || {
        if let Err(e) = run_audio_capture(cmd_rx, shared, ready_tx.clone()) {
            eprintln!("Audio capture error: {}", e);
            let _ = ready_tx.send(Err(e));
        }
    });

    state_guard.command_sender = Some(cmd_tx);
    state_guard.thread_handle = Some(handle);

    // Wait for thread to signal ready (with timeout)
    let init_result = match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Audio thread failed to initialize within timeout".to_string()),
    };

    if init_result.is_err() {
        if let Some(sender) = state_guard.command_sender.take() {
            let _ = sender.send(AudioCommand::Stop);
        }
        state_guard.thread_handle.take();
    }

    init_result
}

fn run_audio_capture(
    cmd_rx: Receiver<AudioCommand>,
    shared: Arc<SharedSamples>,
    ready_tx: Sender<Result<(), String>>,
) -> Result<(), String> {
    let host = cpal::default_host();

    let device = host.default_input_device()
        .ok_or_else(|| "No input device available. Please grant microphone permission.".to_string())?;

    let config = device.default_input_config()
        .map_err(|e| format!("Failed to get input config: {}", e))?;

    let device_sample_rate = config.sample_rate().0;
    let sample_format = config.sample_format();
    let channels = config.channels() as usize;

    if let Ok(mut sr) = shared.sample_rate.lock() {
        *sr = device_sample_rate;
    }

    let err_fn = |err| eprintln!("Audio stream error: {}", err);

    let stream = match sample_format {
        SampleFormat::F32 => build_mono_input_stream!(device, config, shared, channels, err_fn, f32),
        SampleFormat::I16 => build_mono_input_stream!(device, config, shared, channels, err_fn, i16),
        _ => return Err(format!("Unsupported sample format: {:?}", sample_format)),
    };

    stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;

    // Signal that we're ready to record
    let _ = ready_tx.send(Ok(()));

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
        eprintln!("Warning: Recording state mutex was poisoned, recovering");
        poisoned.into_inner()
    });

    // Send stop command
    if let Some(sender) = state_guard.command_sender.take() {
        let _ = sender.send(AudioCommand::Stop);
    }

    // Wait for thread to finish
    if let Some(handle) = state_guard.thread_handle.take() {
        let _ = handle.join();
    }

    // Get samples and sample rate
    let sample_rate = *state_guard.shared.sample_rate.lock().unwrap_or_else(|poisoned| {
        eprintln!("Warning: Sample rate mutex was poisoned, recovering");
        poisoned.into_inner()
    });
    let samples = state_guard.shared.samples.lock().unwrap_or_else(|poisoned| {
        eprintln!("Warning: Samples mutex was poisoned, recovering");
        poisoned.into_inner()
    }).clone();

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
