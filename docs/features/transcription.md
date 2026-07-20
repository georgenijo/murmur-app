# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → 16kHz windows/full buffer → backend inference → ordered text transformations → delivery
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

### Incremental Whisper path (`streaming.rs`, `transcriber/chunking.rs`)

Long live recordings selected to the Whisper backend use true incremental output:

- A single sequential worker snapshots one bounded 10-second window every 8 seconds (2-second overlap) while capture continues.
- Every window passes through the same Silero VAD threshold and the existing cached Whisper backend. There is no second model/context and never more than one inference in flight.
- Chunk text is reconciled by a deterministic word-boundary algorithm. It removes the largest near-equal suffix/prefix overlap within 12 words and retains the earlier surface form.
- After reconciliation, the worker validates the recording ID, cancellation generation, status, and selected model again before publishing a versioned `partial-transcript` event. Its camelCase payload is `{ contractVersion, recordingId, text, chunkIndex, processedAudioMs }`; the text is cumulative and remains in memory only.
- On stop, the worker is signalled before audio teardown and only the unprocessed tail plus the 2-second overlap is inferred. The reconciled result becomes the authoritative text used by the unchanged cleanup, voice-command, correction, file-output, clipboard, paste, history, and stats paths.
- Short recordings (under the first 10-second window), non-Whisper backends, missing/failed VAD, stale sessions, worker lag, panics, or final-tail failures use the original full-buffer pipeline. A model setting change during recording applies to the next session; the active worker keeps its recording-start model snapshot.

The worker holds no queued audio. It reads the current buffer length, copies only one fixed window, awaits that inference, and abandons incremental mode if capture advances by more than one step. Recording IDs and cancellation generations prevent completed work from a stopped/cancelled session from being adopted by a newer recording.

The backend also emits versioned `recording-session-started` and `partial-transcript-cleared` lifecycle events. Clears are recording-ID scoped, so a late cancellation or fallback from an older session cannot erase or update a newer preview. Fallback clears provisional text immediately while the existing full-buffer path remains authoritative. Partial contents are never written to logs, history, stats, settings, files, the clipboard, or auto-paste; telemetry records only first-partial latency, update count, last-partial-to-final latency, and completed/fallback flags.

The overlay registers status and transcript lifecycle listeners as one set before reconciling a privacy-safe active-session snapshot from Rust. The snapshot restores both the active recording ID and recording/processing status, closing the WebView-startup ordering gap without replaying provisional content. Frontend diagnostics record listener readiness, accepted/rejected counts, recording-ID match, chunk index, and clear reason only. The native overlay renders accepted Whisper updates below the physical notch; Parakeet/Core ML remain final-only and say so explicitly.

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

## Pipeline Orchestration (`commands/recording.rs`)

`run_transcription_pipeline()` remains the single authoritative completion entry point. `start_native_recording` resolves one immutable `DictationContextSnapshot` from the frontmost bundle identifier and current configuration; every live stage receives that snapshot instead of re-reading mutable settings:

1. Capture app identity, matched profile, effective settings, vocabulary version, commands, and deny-by-default context permissions at recording start
2. Confirm the snapshot's model preparation completed (or load synchronously as a fallback)
3. Adopt a successfully finalized incremental Whisper transcript, or run the full-buffer backend fallback with the same snapshot
4. Run the backend-neutral transcript transformation pipeline from the snapshot's stage settings and resources
5. Persist optional file output and inject text (clipboard + optional paste) from the snapshot on the main thread
6. Reset status to Idle and clear only the matching recording generation's snapshot

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

### Transcript transformations (`transcript_transform.rs`)

`transform_transcript()` is the authoritative post-recognition entry point for both live and imported-file transcription. It owns a fixed internal sequence:

```text
raw transcript → cleanup → voice commands → Smart Correction → Smart Formatting → IDE context → CLI formatting → final text
```

Each stage receives immutable session/source metadata plus privacy-safe enablement flags and produces privacy-safe execution metadata (`duration_us`, changed/not-changed, outcome, and required/optional failure policy). Structured stage logs never include transcript text, model/language settings, app/profile values, custom replacement values, correction vocabulary, package/script names, or project paths.

Cleanup, voice commands, Smart Formatting, IDE context, and CLI formatting are required deterministic stages when enabled. Smart Correction is optional-fallback: a future recoverable correction failure leaves the preceding text intact. Smart Formatting is live-only and opt-in, fails closed outside its bounded prose grammar, and skips any utterance owned by the CLI grammar. Explicit IDE opt-in bypasses Smart Formatting, then applies only the matching profile's fresh memory-only project index. The final CLI stage remains authoritative, uses conservative prefix/trigger/profile activation, and returns non-command prose byte-for-byte unchanged. Imported-file transcription invokes the same entry point with every stage disabled so its existing raw-ASR output remains unchanged.

The pipeline result can compare its original and final strings in memory for tests and diagnostics, but only privacy-safe stage metadata is logged. Only the final string reaches optional file output, clipboard/paste, history, and stats; delivery remains final-only and happens once.

File persistence, clipboard/paste, history, and stats are intentionally outside the transformation pipeline. Live transformation receives an opaque recording handle plus stage configuration and resources from the same immutable per-app snapshot; app/profile resolution remains owned by the context resolver.

See [Per-App Dictation Context](per-app-profiles.md) for resolver precedence, duplicate-profile compatibility, lifetime, and privacy boundaries.
See [Spoken CLI Command Formatting](cli-command-formatting.md) for activation, grammar, local lexicon layering, and safety guarantees.
See [Smart Formatting and Same-Utterance Backtracking](smart-formatting.md) for its explicit prose grammar, bounds, bypass rules, and privacy contract.
See [Local IDE Symbols and `@file` Context](ide-context.md) for opt-in, scan boundaries, ambiguity, expiry, and privacy guarantees.

## Model Downloads (`commands/models.rs`)

The `download_model` command streams Murmur-managed Whisper and sherpa downloads with `download-progress` events. FluidAudio Core ML setup runs on a blocking worker and is indeterminate because the upstream Rust bridge owns its Hugging Face download and Core ML compilation without exposing progress callbacks.

## Status Flow

```
Idle → Recording (on start) → Processing (on stop) → Idle (after transcription)
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).
Recorder start, stop, and cancel also share an async transition mutex. The lock
is held until cpal confirms startup or audio teardown completes, preventing a
fast hotkey release from stopping a recorder that is still starting.

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
