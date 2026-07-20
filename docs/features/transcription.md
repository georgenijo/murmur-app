# Transcription Pipeline

## Overview

```
cpal audio capture → f32 samples in memory → 16kHz full buffer → backend inference → ordered text transformations → delivery
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

`AppState` owns a `ModelRuntimeManager`. Its catalog maps each exact model
identifier to a backend and capability set, and its single serialized backend
owner coordinates preparation, inference, model changes, and unload. Unknown
models fail closed instead of defaulting to Whisper. `configure_dictation`
selects through this catalog; recording preparation and final inference use the
same manager.

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
- Keeps single-segment decoding for short audio up to 12 seconds, while longer batch decodes retain timestamp-based continuation so an early end-of-text token cannot silently skip the remaining audio
- **Recording-start preparation**: model initialization begins after capture starts, overlapping cold load with speech rather than post-release latency
- If the user changes models in settings, the context is dropped and re-created on next transcription
- Model files are single `.bin` files (e.g., `ggml-base.en.bin`)
- Model search paths are documented in `docs/onboarding.md`
- `single_segment` decoding is duration-conditional (`should_use_single_segment`, 12s threshold): short audio stays single-segment, but longer batch/file transcriptions use multi-segment decoding so an early end-of-text token from the model can't force-skip the rest of the audio and silently truncate the tail

All supported backends follow the same final-after-stop interaction: recording only captures audio; stopping runs one authoritative full-buffer transcription; the transformed final result is then delivered exactly once. Murmur does not display or emit provisional transcript text while recording or processing.

The catalog may describe partial-result support as a backend capability for a
future product contract. There is currently no streaming worker, provisional
transcript event, live-preview setting, or model-specific preview behavior.

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

1. Capture app identity, matched profile, effective settings, vocabulary version, repository-backed commands, and deny-by-default context permissions at recording start
2. Confirm the snapshot's model preparation completed (or load synchronously as a fallback)
3. Run one full-buffer VAD and backend transcription pass with the same snapshot
4. Run the backend-neutral transcript transformation pipeline from the snapshot's stage settings and resources
5. Persist optional file output and inject text (clipboard + optional paste) from the snapshot on the main thread
6. Reset status to Idle and clear only the matching recording generation's snapshot

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

### Transcript transformations (`transcript_transform.rs`)

`transform_transcript()` is the authoritative post-recognition entry point for both live and imported-file transcription. It owns a fixed internal sequence:

```text
raw transcript → cleanup → voice commands → Smart Correction (explicit aliases, exact/derived terms, then fuzzy) → Smart Formatting → IDE context → CLI formatting → final text
```

Each stage receives immutable session/source metadata plus privacy-safe enablement flags and produces privacy-safe execution metadata (`duration_us`, changed/not-changed, outcome, and required/optional failure policy). Structured stage logs never include transcript text, model/language settings, app/profile values, custom replacement values, correction vocabulary, package/script names, or project paths.

Cleanup, voice commands, Smart Formatting, IDE context, and CLI formatting are required deterministic stages when enabled. Smart Correction is optional-fallback: a future recoverable correction failure leaves the preceding text intact. Smart Formatting is live-only and opt-in, fails closed outside its bounded prose grammar, and skips any utterance owned by the CLI grammar. Explicit IDE opt-in bypasses Smart Formatting, then applies only the matching profile's fresh memory-only project index. The final CLI stage remains authoritative, uses conservative prefix/trigger/profile activation, and returns non-command prose byte-for-byte unchanged. Imported-file transcription invokes the same entry point with every stage disabled so its existing raw-ASR output remains unchanged.

The pipeline result can compare its original and final strings in memory for tests and diagnostics, but only privacy-safe stage metadata is logged. Only the final string reaches optional file output, clipboard/paste, history, and stats; delivery remains final-only and happens once.

File persistence, clipboard/paste, history, and stats are intentionally outside the transformation pipeline. Live transformation receives an opaque recording handle plus stage configuration and resources from the same immutable per-app snapshot; app/profile resolution remains owned by the context resolver.

See [Per-App Dictation Context](per-app-profiles.md) for resolver precedence, duplicate-profile compatibility, lifetime, and privacy boundaries.
See [Spoken CLI Command Formatting](cli-command-formatting.md) for activation, grammar, local lexicon layering, and safety guarantees.
See [Explicit Spoken Vocabulary Aliases](vocabulary-aliases.md) for migration, precedence, scope, validation, and privacy guarantees.
See [Voice Commands 2.0](voice-commands.md) for typed replacements, multiline snippets, variables, app scopes, conflicts, and clipboard permission boundaries.
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

Model state is separate from recording status. `get_model_runtime_catalog` and
`get_model_runtime_status` expose catalog metadata plus install/lifecycle state.
Transitions emit generation-ordered `model-runtime-status-changed` snapshots;
their telemetry is privacy-safe bounded metadata and never contains transcript
text, model paths, or raw backend errors.

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
