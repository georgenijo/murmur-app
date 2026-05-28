# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → resample to 16kHz mono → backend inference → text
```

All processing is local during transcription (network only required to download models beforehand). Transcription uses the **Whisper** backend (`whisper-rs`) with Metal GPU acceleration.

## Audio Capture (`audio.rs`)

- Uses `cpal` to record from the default input device on a background thread
- Channel-based synchronization: recording thread signals readiness via `mpsc::channel` before `start_recording()` returns, preventing race conditions
- Multi-channel to mono conversion (averages channels)
- Resamples to 16kHz (expected sample rate for the backend)
- Samples stored as `Vec<f32>` in memory — no temp files
- Logs the selected input route, current output route, sample rate, channel count, sample format, and capture stream start/stop events to the `audio` stream

### macOS Audio Ducking / Bluetooth Routes

Murmur does not intentionally lower, mix, or control other apps' output volume. On macOS, the capture path opens a CoreAudio input stream through `cpal`; the app does not open an output stream or call any volume APIs.

If the selected microphone is a Bluetooth headset-style input (for example AirPods, Beats, Bose, Jabra, Sony, or a "Hands-Free" device), macOS may switch the device into a call-style route while the input stream is open. That route can make other app/system audio sound quieter, lower quality, or otherwise different until recording stops. This can look like a focus-change bug because background keyboard triggers and overlay interactions start recording while another app is playing audio, but the relevant trigger is the input capture lifecycle.

Workarounds:
- Use the Mac built-in microphone or a USB microphone for Murmur.
- Keep Bluetooth headphones as the output device, but avoid selecting their microphone as Murmur's input.
- Use the log viewer's `audio` stream to compare `audio capture stream started` / `audio capture stream stopped` events against the moment other audio changes.

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

The active backend is stored as `Mutex<Box<dyn TranscriptionBackend>>` in `AppState`. The trait is kept for future extensibility.

### Whisper Backend (`transcriber/whisper.rs`)

- Uses `whisper-rs` with Metal GPU acceleration
- **Lazy loading**: Whisper context is initialized on first transcription, not at app startup
- If the user changes models in settings, the context is dropped and re-created on next transcription
- Model files are single `.bin` files (e.g., `ggml-base.en.bin`)
- Model search paths are documented in `docs/onboarding.md`

## Model Options

| Model | Setting Value | Backend | English-only | Speed |
|-------|--------------|---------|-------------|-------|
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

## Model Downloads (`commands/models.rs`)

The `download_model` command downloads Whisper models as single `.bin` files from Hugging Face, streaming the download with progress events (`download-progress`).

## Status Flow

```
Idle → Recording (on start) → Processing (on stop) → Idle (after transcription)
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
