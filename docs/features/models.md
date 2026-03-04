# Model Management

## Overview

The app supports 5 transcription models using the Whisper backend. Models are downloaded on demand, loaded lazily on first transcription, and cached for reuse.

## Available Models

| Model | Setting Value | Backend | Size | Speed | Language |
|-------|--------------|---------|------|-------|----------|
| Tiny | `tiny.en` | Whisper (Metal GPU) | ~75 MB | Fast | English only |
| Base | `base.en` | Whisper (Metal GPU) | ~150 MB | Fast | English only |
| Small | `small.en` | Whisper (Metal GPU) | ~500 MB | Medium | English only |
| Medium | `medium.en` | Whisper (Metal GPU) | ~1.5 GB | Slow | English only |
| Large Turbo | `large-v3-turbo` | Whisper (Metal GPU) | ~3 GB | Slow | Multilingual |

**Default model:** `base.en` (set in `settings.ts` and `DictationState::default()`).

## Backend

### Whisper (`transcriber/whisper.rs`)

Uses `whisper-rs` with Apple Metal GPU acceleration. Model files are single `.bin` files (e.g., `ggml-base.en.bin`).

**Inference config:** Greedy sampling (best_of=1), single segment mode, timestamps/progress/special tokens suppressed, blank suppression enabled.

**Model search paths** (checked in order):
1. `$WHISPER_MODEL_DIR` environment variable
2. `~/Library/Application Support/local-dictation/models`
3. `~/Library/Application Support/pywhispercpp/models`
4. `~/.cache/whisper.cpp`
5. `~/.cache/whisper`
6. `~/.whisper/models`

**Download URL:** `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin`

## Storage

All models are stored in `~/Library/Application Support/local-dictation/models/`.

## Lazy Loading

Models are not loaded at app startup. The backend's `load_model()` is called on the first transcription attempt. If the model is already loaded (`loaded_model_name` matches the requested model), the call is a no-op.

### WhisperState Caching

As of v0.7.8, the Whisper backend caches both the `WhisperContext` and `WhisperState` across transcriptions. Previously, `create_state()` was called per transcription, causing expensive GPU/Metal buffer alloc/free cycles. Now:

1. `load_model()` creates a `WhisperContext` from the model file
2. `ctx.create_state()` is called exactly once to produce a `WhisperState`
3. Both are stored in the `WhisperBackend` struct and reused for all subsequent transcriptions
4. Only a model change triggers `reset()`, which drops and recreates both

This is the same pattern described in the [transcription pipeline docs](transcription.md).

## First-Launch Downloader

On first launch (no model downloaded), a full-screen download view presents 2 curated models:

| Model | Description |
|-------|-------------|
| `large-v3-turbo` | "Highest accuracy, slower (1-2 seconds)" |
| `base.en` | "Good balance of speed and accuracy" |

Default selection: `large-v3-turbo`. The user selects a model and clicks "Download." A progress bar shows percentage and byte counts. Selection is disabled during download. On error, a "Retry Download" button appears.

The main app controls are gated on `initialized` (which requires a model to exist), so the download screen blocks all other interaction.

## Download Pipeline

### Streaming Download

`stream_download()` handles all model downloads:

- Uses `reqwest` with 30s connect timeout and 15-minute overall timeout
- Writes chunks to a temp file (`.tmp` suffix)
- Emits `download-progress` events with `{ received, total }` payload
- On success: atomic rename from `.tmp` to final path
- On failure: temp file cleaned up

### Whisper Downloads

Single `.bin` file downloaded directly from HuggingFace. Atomic rename on completion.

### VAD Co-Download

Every transcription model download also triggers a co-download of the Silero VAD model (`ggml-silero-v5.1.2.bin`, ~1.8MB) if it is not already present. VAD download failure is non-fatal. See [vad.md](vad.md) for details on the VAD fallback download mechanism.

## Allowed Models

The `download_model` command accepts only models from a hardcoded allow-list:

```
large-v3-turbo, small.en, base.en, tiny.en, medium.en
```

Any other model name is rejected. The `check_specific_model_exists` command also includes path traversal protection, rejecting names containing `..`, `/`, or `\`.

## Model Switching

When the user changes models in settings, `configure_dictation` calls `reset()` on the backend to force a reload on the next transcription.

The active backend is stored as `Mutex<Box<dyn TranscriptionBackend>>` in `AppState`.

## Inline Download in Settings

The settings panel supports downloading models without leaving the settings view:

1. On model selection change, `check_specific_model_exists` verifies the model is on disk
2. If not downloaded, an amber warning appears with a "Download" link
3. During download: progress bar with percentage and "Downloading..." text
4. On error: red error banner with message and "Retry" link
5. Stale-request protection prevents progress updates from a previously selected model

Model selection is disabled while recording is active.

## Settings

- `model: ModelOption` — Selected model name. Persisted to localStorage. Sent to Rust via `configure_dictation`.

Model options are defined in `settings.ts` with the `MODEL_OPTIONS` array. Each option includes the setting value, display label, size string, and backend type.
