use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

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
                sample_rate: Mutex::new(16000),
            }),
        })
    })
}

pub fn start_recording() -> Result<(), String> {
    let state = get_state();
    let mut state_guard = state.lock().map_err(|e| e.to_string())?;

    // Stop any existing recording
    if state_guard.command_sender.is_some() {
        drop(state_guard);
        stop_recording()?;
        state_guard = state.lock().map_err(|e| e.to_string())?;
    }

    // Clear previous samples
    if let Ok(mut samples) = state_guard.shared.samples.lock() {
        samples.clear();
    }

    let (cmd_tx, cmd_rx) = channel::<AudioCommand>();
    let shared = Arc::clone(&state_guard.shared);

    // Spawn audio thread
    let handle = thread::spawn(move || {
        if let Err(e) = run_audio_capture(cmd_rx, shared) {
            eprintln!("Audio capture error: {}", e);
        }
    });

    state_guard.command_sender = Some(cmd_tx);
    state_guard.thread_handle = Some(handle);

    // Give the thread a moment to initialize
    thread::sleep(std::time::Duration::from_millis(100));

    Ok(())
}

fn run_audio_capture(cmd_rx: Receiver<AudioCommand>, shared: Arc<SharedSamples>) -> Result<(), String> {
    let host = cpal::default_host();

    let device = host.default_input_device()
        .ok_or_else(|| "No input device available. Please grant microphone permission.".to_string())?;

    let config = device.default_input_config()
        .map_err(|e| format!("Failed to get input config: {}", e))?;

    let device_sample_rate = config.sample_rate().0;
    if let Ok(mut sr) = shared.sample_rate.lock() {
        *sr = device_sample_rate;
    }

    let channels = config.channels() as usize;
    let samples_clone = Arc::clone(&shared);

    let err_fn = |err| eprintln!("Audio stream error: {}", err);

    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| {
                    let mono: Vec<f32> = data.chunks(channels)
                        .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                        .collect();
                    if let Ok(mut s) = samples_clone.samples.lock() {
                        s.extend(mono);
                    }
                },
                err_fn,
                None,
            ).map_err(|e| format!("Failed to build stream: {}", e))?
        },
        SampleFormat::I16 => {
            device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &_| {
                    let mono: Vec<f32> = data.chunks(channels)
                        .map(|chunk| {
                            let sum: f32 = chunk.iter().map(|&s| s.to_float_sample()).sum();
                            sum / channels as f32
                        })
                        .collect();
                    if let Ok(mut s) = samples_clone.samples.lock() {
                        s.extend(mono);
                    }
                },
                err_fn,
                None,
            ).map_err(|e| format!("Failed to build stream: {}", e))?
        },
        _ => return Err("Unsupported sample format".to_string()),
    };

    stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;

    // Wait for stop command
    loop {
        match cmd_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(AudioCommand::Stop) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Stream is dropped here, stopping recording
    Ok(())
}

pub fn stop_recording() -> Result<Vec<f32>, String> {
    let state = get_state();
    let mut state_guard = state.lock().map_err(|e| e.to_string())?;

    // Send stop command
    if let Some(sender) = state_guard.command_sender.take() {
        let _ = sender.send(AudioCommand::Stop);
    }

    // Wait for thread to finish
    if let Some(handle) = state_guard.thread_handle.take() {
        let _ = handle.join();
    }

    // Get samples and sample rate
    let sample_rate = *state_guard.shared.sample_rate.lock().map_err(|e| e.to_string())?;
    let samples = state_guard.shared.samples.lock()
        .map_err(|e| e.to_string())?
        .clone();

    // Resample to 16kHz if needed
    if sample_rate != 16000 && !samples.is_empty() {
        Ok(resample(&samples, sample_rate, 16000))
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
