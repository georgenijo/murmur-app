# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → resample to 16kHz mono → backend inference → text
```

Transcription processing is local. Network access occurs for model setup and may also be used to fetch a missing VAD asset in the background. New installs default to FluidAudio Core ML on the Apple Neural Engine, while Whisper/Metal and sherpa-onnx/CPU remain selectable.

## Audio Capture (`audio.rs`)

- Uses `cpal` to record from the default input device on a background thread
- Channel-based synchronization: recording thread signals readiness via `mpsc::channel` before `start_recording()` returns, preventing race conditions
- Multi-channel to mono conversion (averages channels)
- Resamples to 16kHz (expected sample rate for the backend)
- Samples stored as `Vec<f32>` in memory — no temp files

## Transcription Backend (`transcriber/`)

The backend implements the `TranscriptionBackend` trait (`transcriber/mod.rs`):

```rust
pub trait TranscriptionBackend: Send + Sync {
    fn name(&self) -> &str;
    fn load_model(&mut self, model_name: &str) -> Result<(), String>;
    fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<String, String>;
    fn model_exists(&self) -> bool;
    fn models_dir(&self) -> Result<PathBuf, String>;
    fn reset(&mut self);
}
```

The active backend is stored as `Mutex<Box<dyn TranscriptionBackend>>` in `AppState`. `configure_dictation` dispatches the explicit Core ML model before the broad `parakeet*` sherpa classifier.

### FluidAudio Core ML Backend (`transcriber/coreml.rs`)

- macOS 14+ and Apple Silicon only
- Parakeet TDT 0.6B v3 on Core ML / Apple Neural Engine
- Default for new installs; existing persisted backend choices are preserved
- FluidAudio owns download/compilation in its Application Support cache
- An installed model warms in the background after startup configuration; recording-start preparation remains the fallback after idle unloading or a model change
- Language is auto-detected; the current Rust bridge ignores language hints and initial prompts

### Whisper Backend (`transcriber/whisper.rs`)

- Uses `whisper-rs` with Metal GPU acceleration
- Enables flash attention; Murmur consumes segment text and does not use the incompatible DTW token timestamps
- **Recording-start preparation**: model initialization begins after capture starts, overlapping cold load with speech rather than post-release latency
- If the user changes models in settings, the context is dropped and re-created on next transcription
- Model files are single `.bin` files (e.g., `ggml-base.en.bin`)
- Model search paths are documented in `docs/onboarding.md`

## Model Options

| Model | Setting Value | Backend | English-only | Speed |
|-------|--------------|---------|-------------|-------|
| Parakeet v3 Core ML | `parakeet-tdt-0.6b-v3-coreml` | FluidAudio / ANE | No | Fastest |
| Parakeet v2 fp16 | `parakeet-tdt-0.6b-v2-fp16` | sherpa-onnx / CPU | Yes | Fast |
| Tiny | `tiny.en` | Whisper | Yes | Fast |
| Base | `base.en` | Whisper | Yes | Fast |
| Small | `small.en` | Whisper | Yes | Medium |
| Medium | `medium.en` | Whisper | Yes | Slow |
| Large Turbo | `large-v3-turbo` | Whisper | No (multilingual) | Slow |

## Pipeline Orchestration (`lib.rs`)

`run_transcription_pipeline()` is the shared entry point:

1. Read model/language/auto_paste from `DictationState` (single lock)
2. Confirm the recording-start model preparation completed (or load synchronously as a fallback)
3. Run transcription via the active backend
4. Inject text (clipboard + optional paste) on main thread
5. Reset status to Idle

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

## Model Downloads (`commands/models.rs`)

The `download_model` command streams Murmur-managed Whisper and sherpa downloads with `download-progress` events. FluidAudio Core ML setup runs on a blocking worker and is indeterminate because the upstream Rust bridge owns its Hugging Face download and Core ML compilation without exposing progress callbacks.

## Status Flow

```
Idle → Recording (on start) → Processing (on stop) → Idle (after transcription)
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
