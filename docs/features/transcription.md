# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → resample to 16kHz mono → backend inference → text
```

All processing is local. No network calls. Two transcription backends are available:

- **Whisper** (`whisper-rs`) — Metal GPU acceleration, higher accuracy, slower on short clips
- **Moonshine** (`sherpa-rs`) — CPU inference with int8 quantization, extremely fast on short clips

## Audio Capture (`audio.rs`)

- Uses `cpal` to record from the default input device on a background thread
- Channel-based synchronization: recording thread signals readiness via `mpsc::channel` before `start_recording()` returns, preventing race conditions
- Multi-channel to mono conversion (averages channels)
- Resamples to 16kHz (expected sample rate for both backends)
- Samples stored as `Vec<f32>` in memory — no temp files

## Transcription Backends (`transcriber/`)

Both backends implement the `TranscriptionBackend` trait (`transcriber/mod.rs`):

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

The active backend is stored as `Mutex<Box<dyn TranscriptionBackend>>` in `AppState`. When the user switches between whisper and moonshine models, `configure_dictation` swaps the backend instance.

### Whisper Backend (`transcriber/whisper.rs`)

- Uses `whisper-rs` with Metal GPU acceleration
- **Lazy loading**: Whisper context is initialized on first transcription, not at app startup
- If the user changes models in settings, the context is dropped and re-created on next transcription
- Model files are single `.bin` files (e.g., `ggml-base.en.bin`)
- Model search paths are documented in `docs/onboarding.md`

### Moonshine Backend (`transcriber/moonshine.rs`)

- Uses `sherpa-rs` with CPU inference (ONNX runtime, int8 quantized)
- Same lazy loading pattern as Whisper
- Model files are directories containing multiple ONNX files (e.g., `sherpa-onnx-moonshine-tiny-en-int8/`)
- English-only — the `language` parameter is ignored
- Download URLs: `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-{variant}-en-int8.tar.bz2`

## Model Options

| Model | Setting Value | Backend | English-only | Speed |
|-------|--------------|---------|-------------|-------|
| Moonshine Tiny | `moonshine-tiny` | Moonshine | Yes | Fastest (~16ms for 3s audio) |
| Moonshine Base | `moonshine-base` | Moonshine | Yes | Very fast (~37ms for 3s audio) |
| Tiny | `tiny.en` | Whisper | Yes | Fast |
| Base | `base.en` | Whisper | Yes | Fast |
| Small | `small.en` | Whisper | Yes | Medium |
| Medium | `medium.en` | Whisper | Yes | Slow |
| Large Turbo | `large-v3-turbo` | Whisper | No (multilingual) | Slow |

## Pipeline Orchestration (`lib.rs`)

`run_transcription_pipeline()` is the shared entry point:

1. Read model/language/auto_paste from `DictationState` (single lock)
2. Load model via backend if needed (lazy init)
3. Run transcription via the active backend
4. Inject text (clipboard + optional paste) on main thread
5. Reset status to Idle

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

## Model Downloads (`lib.rs`)

The `download_model` command handles both model types:

- **Whisper**: Downloads a single `.bin` file from Hugging Face
- **Moonshine**: Downloads a `.tar.bz2` archive from GitHub, extracts it using the `tar` and `bzip2` crates

Both paths stream the download with progress events (`download-progress`).

## Status Flow

```
Idle → Recording (on start) → Processing (on stop) → Idle (after transcription)
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
