# Model Management

## Overview

The app supports three transcription backends: FluidAudio Core ML on the Apple Neural Engine, Whisper on Metal, and Parakeet through sherpa-onnx on CPU. Models are downloaded on demand, prepared when recording starts, and cached for reuse.

## Available Models

| Model | Setting Value | Backend | Size | Speed | Language |
|-------|--------------|---------|------|-------|----------|
| Parakeet TDT 0.6B v3 | `parakeet-tdt-0.6b-v3-coreml` | FluidAudio (Core ML / ANE) | ~470 MB | Fastest | Multilingual |
| Parakeet TDT 0.6B v2 | `parakeet-tdt-0.6b-v2-fp16` | Parakeet (sherpa-onnx, CPU) | ~1.2 GB | Fast | English only |
| Tiny | `tiny.en` | Whisper (Metal GPU) | ~75 MB | Fast | English only |
| Base | `base.en` | Whisper (Metal GPU) | ~150 MB | Fast | English only |
| Small | `small.en` | Whisper (Metal GPU) | ~500 MB | Medium | English only |
| Medium | `medium.en` | Whisper (Metal GPU) | ~1.5 GB | Slow | English only |
| Large Turbo | `large-v3-turbo` | Whisper (Metal GPU) | ~3 GB | Slow | Multilingual |

**Default for new macOS installs:** `parakeet-tdt-0.6b-v3-coreml`. Non-macOS builds hide Core ML and default to the CPU Parakeet model. Persisted Whisper and CPU Parakeet selections remain valid and are not migrated. The Rust `DictationState::default()` stays `base.en` until the first frontend `configure_dictation` call selects the persisted model.

## Backend

### FluidAudio Core ML (`transcriber/coreml.rs`)

FluidAudio 0.14.1 runs Parakeet TDT 0.6B v3 on the Apple Neural Engine. This backend requires Apple Silicon and macOS 14 or newer. It accepts 16 kHz mono `f32` samples directly, produces native casing and punctuation, and auto-detects among the languages supported by Parakeet v3. The Rust bridge does not currently expose language hints, initial prompts, download byte progress, or a tokenizer.

FluidAudio owns setup and compilation through `init_asr()`. Murmur runs that synchronous work on a blocking worker and treats the model as ready only when all four compiled Core ML bundles have nonempty metadata and weight files and the vocabulary is nonempty. When the user explicitly retries setup, Murmur removes only this incomplete v3 cache directory before calling FluidAudio so the upstream initializer downloads and compiles a clean copy instead of reusing broken bundle folders.

**Cache directory:** `~/Library/Application Support/FluidAudio/Models/parakeet-tdt-0.6b-v3/`. This external cache is owned by FluidAudio and is not removed with Murmur's own models directory.

**Measured on an Apple M4:** a 19.62-second prompt transcribed in 133.4 ms on average after a cached 107.8 ms fresh-process initialization. See `.perf/README.md` for the reproducible harness and full measurements.

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

### Parakeet (`transcriber/parakeet.rs`)

NVIDIA Parakeet TDT 0.6B run offline via the `sherpa-onnx` crate (CPU, no NVIDIA GPU). Self-contained and removable — teardown steps are documented at the top of `transcriber/parakeet.rs`. A model is a **directory of 4 files** (`encoder.fp16.onnx`, `decoder.fp16.onnx`, `joiner.fp16.onnx`, `tokens.txt`), not a single `.bin`.

**Inference config:** `nemo_transducer` model type, greedy decoding, CPU provider, 4 threads. English-only; ignores the language/initial-prompt args. `token_count` returns `None` (stats fall back to an estimate). Honors `smart_punctuation` via a local punctuation stripper.

**Variant registry:** `variant_for()` in `parakeet.rs` maps each dropdown value to its bundle dir + files + decoding method. Currently ships fp16 (greedy) — int8 lost accuracy and beam was a no-op in testing.

**Bundle dir:** `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16/` under the models dir.

## Storage

Whisper, sherpa Parakeet, and Silero VAD models are stored in `~/Library/Application Support/local-dictation/models/`. FluidAudio uses its separate cache documented above.

## Recording-Start Preparation

Models are not loaded at app startup. Once audio capture is active, a blocking worker calls the backend's `load_model()` so cold initialization overlaps the user's speech. The transcription pipeline calls `load_model()` again as a safe fallback; that call is a no-op when preparation finished, or waits on the same backend lock for a very short recording. Idle release never resets a model while the app is recording or processing.

### WhisperState Caching

As of v0.7.8, the Whisper backend caches both the `WhisperContext` and `WhisperState` across transcriptions. Previously, `create_state()` was called per transcription, causing expensive GPU/Metal buffer alloc/free cycles. Now:

1. `load_model()` creates a `WhisperContext` from the model file
2. `ctx.create_state()` is called exactly once to produce a `WhisperState`
3. Both are stored in the `WhisperBackend` struct and reused for all subsequent transcriptions
4. Only a model change triggers `reset()`, which drops and recreates both

This is the same pattern described in the [transcription pipeline docs](transcription.md).

## First-Launch Downloader

On first launch (the selected model is not present), a full-screen download view presents curated models:

| Model | Description |
|-------|-------------|
| `parakeet-tdt-0.6b-v3-coreml` | "Fastest on Apple Silicon — multilingual, Apple Neural Engine (recommended)" |
| `parakeet-tdt-0.6b-v2-fp16` | "Fast CPU fallback — English only" |
| `large-v3-turbo` | "Highest accuracy, slower (1-2 seconds)" |
| `base.en` | "Good balance of speed and accuracy" |

Default selection: `parakeet-tdt-0.6b-v3-coreml`. The gate (`App.tsx`) checks the currently selected model via `check_specific_model_exists`. The downloader starts on that model when it is one of the curated choices and persists whichever model the user actually downloads. Selection is disabled during setup. Byte progress is shown for Murmur-managed downloads; FluidAudio setup is indeterminate because its Rust bridge does not expose progress. On error, a "Retry Download" button appears.

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

### Parakeet Downloads

The model bundle ships as a `.tar.bz2` from the sherpa-onnx `asr-models` GitHub release. `download_parakeet_model` streams it (same progress events), then decompresses (`bzip2`) and untars (`tar`) on a blocking thread into the models dir, replacing any stale bundle. The temp archive is removed afterward.

### FluidAudio Core ML Setup

`download_model` dispatches the explicit Core ML model value before the broad `parakeet*` sherpa classifier. It calls FluidAudio's synchronous setup through `spawn_blocking`; an existing cache that fails Murmur's completeness check is first removed at the exact v3 cache path, then FluidAudio downloads, compiles, and caches the model. Murmur validates the completed external cache again. The first setup can take tens of seconds; later initialization is normally around a tenth of a second. A newly linked app can also trigger one-time ANE specialization, so an installed model warms on a blocking worker immediately after startup configuration instead of deferring that cost to the first dictation.

### VAD Co-Download

Every transcription model download also triggers a co-download of the Silero VAD model (`ggml-silero-v5.1.2.bin`, ~1.8MB) if it is not already present. VAD download failure is non-fatal. See [vad.md](vad.md) for details on the VAD fallback download mechanism.

## Allowed Models

The `download_model` command accepts only models from a hardcoded allow-list:

```
large-v3-turbo, small.en, base.en, tiny.en, medium.en
```

The single Core ML model value is matched exactly before Parakeet model names are validated against the sherpa `download_spec` registry. Any other model name is rejected. The `check_specific_model_exists` command also includes path traversal protection, rejecting names containing `..`, `/`, or `\`.

## Model Switching

When the user changes models in settings, `configure_dictation` selects the FluidAudio, sherpa, or Whisper backend and resets it so the next transcription loads the requested model.

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
