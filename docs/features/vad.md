# Voice Activity Detection

## Overview

Silero VAD v5.1.2 pre-filters silence and non-speech audio before it reaches the transcription backend. Without VAD, Whisper tends to hallucinate text on silent or near-silent recordings (repeating phrases, generating phantom words). VAD solves this by detecting whether the audio actually contains speech and trimming the samples to only speech segments.

## How It Works

VAD runs between audio capture and transcription in the pipeline. After recording stops and the samples are collected, `filter_speech` is called on a `spawn_blocking` thread (the `WhisperVadContext` is `!Send`, so it cannot run on a normal async task).

```text
audio capture → VAD filter → transcription backend → text injection
```

The VAD analysis produces one of three outcomes:

- **NoSpeech** — No speech segments detected. Transcription is skipped entirely and an empty string is returned. This prevents Whisper hallucination loops on silent recordings.
- **Speech** — Speech segments found. The samples are trimmed to only the detected speech regions and passed to the transcription backend.
- **Error** — VAD failed for some reason. The pipeline falls back to using the unfiltered audio, so transcription still happens.

### Segment Processing

Silero VAD returns speech segments as centisecond timestamps. These are converted to sample indices (at 16kHz) and the corresponding sample ranges are extracted. If all extracted segments are empty after conversion, the result is treated as NoSpeech.

### Configuration: 1 thread, GPU disabled

The VAD context is configured to use a single thread with GPU acceleration disabled. Unlike the transcription `WhisperState`, the `WhisperVadContext` is created fresh per transcription and not cached across runs.

## Sensitivity

VAD sensitivity is user-configurable on a 0-100 scale (default: 50). The internal threshold is computed as:

```text
threshold = 1.0 - (sensitivity / 100.0)
```

- **Higher sensitivity** (e.g., 80) = lower threshold = keeps more audio, less aggressive trimming
- **Lower sensitivity** (e.g., 20) = higher threshold = trims silence more aggressively

The slider appears in the Recording section of the settings panel, labeled "Voice Detection" with a percentage display. Values are sent to the Rust backend via `configure_dictation` and clamped to 0-100.

## VAD Model

The VAD model file is `ggml-silero-v5.1.2.bin` (~1.8MB), stored alongside transcription models in `~/Library/Application Support/local-dictation/models/`.

### Co-download

When downloading any transcription model (whisper or moonshine), the VAD model is automatically co-downloaded if not already present. VAD download failure during co-download is non-fatal — the transcription model download still succeeds.

### Fallback Download

For users who upgraded from a pre-VAD version, the `ensure_vad_model` function checks for the VAD model at transcription time. If missing, it spawns a silent background download with no UI side effects -- the current transcription proceeds with unfiltered audio, and the downloaded model is used on the next recording.

If the background download fails, the error is logged but no notification is sent to the frontend.

## Pipeline Integration

In `run_transcription_pipeline()`, the VAD phase runs after pre-VAD diagnostics (RMS and peak amplitude logging) and before the transcription phase:

1. State read (model, language, VAD sensitivity, etc.)
2. Pre-VAD audio diagnostics
3. **VAD filter** — runs on `spawn_blocking`, returns `VadResult`
4. Transcription — uses trimmed samples (Speech) or original samples (Error/missing model)
5. Text injection

The VAD execution time is logged as `vad_ms` in the structured telemetry output alongside `inference_ms`, `paste_ms`, and `total_ms`. This timing data is visible in the log viewer's Metrics tab. See [log-viewer.md](log-viewer.md) for details.

## Settings

- `vadSensitivity: number` — Sensitivity value (0-100, default 50). Persisted to localStorage. Sent to Rust via `configure_dictation`.
