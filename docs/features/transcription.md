# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → resample to 16kHz mono → whisper-rs inference → text
```

All processing is local. No network calls.

## Audio Capture (`audio.rs`)

- Uses `cpal` to record from the default input device on a background thread
- Channel-based synchronization: recording thread signals readiness via `mpsc::channel` before `start_recording()` returns, preventing race conditions
- Multi-channel to mono conversion (averages channels)
- Resamples to 16kHz (whisper's expected sample rate)
- Samples stored as `Vec<f32>` in memory — no temp files

## Whisper Inference (`transcriber.rs`)

- Uses `whisper-rs` with Metal GPU acceleration
- **Lazy loading**: Whisper context is initialized on first transcription, not at app startup. Stored in `AppState.whisper_context: Mutex<Option<WhisperContext>>`
- If the user changes models in settings, the context is dropped and re-created on next transcription
- Model search paths are documented in `docs/onboarding.md`

### Model Options

| Model | Setting Value | English-only |
|-------|--------------|-------------|
| Tiny | `tiny.en` | Yes |
| Base | `base.en` | Yes |
| Small | `small.en` | Yes |
| Medium | `medium.en` | Yes |
| Large Turbo | `large-v3-turbo` | No (multilingual) |

## Pipeline Orchestration (`lib.rs`)

`run_transcription_pipeline()` is the shared entry point:

1. Read model/language/auto_paste from `DictationState` (single lock)
2. Initialize whisper context if needed
3. Run transcription
4. Inject text (clipboard + optional paste) on main thread
5. Reset status to Idle

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

## Status Flow

```
Idle → Recording (on start) → Processing (on stop) → Idle (after transcription)
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
