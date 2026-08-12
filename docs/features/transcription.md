# Transcription Pipeline

## Process-isolated capture

Microphone enumeration and streaming execute only in the signed
`murmur-capture-worker`. Production protocol v3 binds every control and PCM
frame to a capture ID and nonce and rejects stale, malformed, oversized,
out-of-sequence, or sample-rate-changing input. The worker callback writes mono
samples into a preallocated SPSC ring; the parent retains PCM before declaring
readiness and calculates waveform levels locally. The Settings microphone
preview is the deliberate exception to PCM retention: it reaches readiness on
valid first PCM, aggregates only content-free RMS/peak values, and immediately
discards the samples.

Direct AUHAL is the primary backend and CPAL is the independent fallback. A
single fallback is allowed only before any audio is retained and must target the
same raw device UID. The fallback starts only after the primary process group is
confirmed empty and a final Stop check passes. Once audio exists, failure ends capture without switching
devices. Prefixes of at least 500 ms are transcribed normally, remain
clipboard-first, and are marked **Interrupted · partial** in history.

A process-local session memo, keyed per requested device, adapts the attempt
sequence to two observed hang pathologies. For a backend-bound hang (one
backend hangs, the other works), the backend that most recently delivered
first PCM is ordered first, so the timeout is paid once per app run instead of
per recording. For a first-attempt-bound hang (whichever backend goes first
hangs in `AudioOutputUnitStart` while the second attempt succeeds within
~160ms), promotion is disproven the moment a promoted backend itself times out
before first PCM: promotion is then disabled for that key for the session
(otherwise the order oscillates), a promoted backend's budget is always capped
at the default primary's 8s so a wrong promotion can never worsen the worst
case, and after two consecutive recordings of "primary failed before first
PCM, fallback delivered it within 1s" the primary attempt budget shrinks to
2s so the reliable rescue starts sooner. A primary success or a slow rescue
resets that counter and restores full budgets — a machine where both backends
are slow never arms the short budget. The memo only reorders and shrinks: both backends
always stay in the sequence, budgets never grow, and termination confirmation
and fallback-eligibility rules are unchanged. It is never persisted, and
telemetry logs only backend names, never the device key.

## Overview

```
cpal audio capture → f32 samples in memory → 16kHz full buffer → backend inference → ordered text transformations → delivery
```

Transcription processing is local. Network access occurs for model setup and may also be used to fetch a missing VAD asset in the background. New installs default to FluidAudio Core ML on the Apple Neural Engine, while Whisper/Metal and sherpa-onnx/CPU remain selectable.

## Audio Capture (`audio.rs`)

- Uses direct AUHAL first and CPAL 0.18.1 as a single pre-buffer fallback inside
  a signed, process-isolated capture worker. `system_default` resolves
  the live OS default; an explicit selection is the backend-native stable ID
  (`kAudioDevicePropertyDeviceUID` on CoreAudio), never a display name.
  Display names are presentation-only. A missing/ambiguous explicit selection
  fails as `device_unavailable` and never falls back to another microphone.
- An app-lifetime supervisor is the single microphone owner. It owns each
  live capture worker and join handle. Cancellation or deadline expiry closes
  that generation's publication gate and requests stop, but ownership remains
  in `Recovering` until the worker exits and is joined. Rapid retries therefore
  cannot create overlapping in-process CoreAudio owners.
- The Settings test uses a first-class `Preview(preview_id)` owner. It is
  mutually exclusive with dictation, transform/query/corpus capture, and
  benchmarks, clears only after joined-worker `Idle`, and emits only targeted
  `microphone-preview-*` events. It never emits the global dictation
  `audio-level` stream. See [Microphone Input Test](microphone-input-test.md).
- Dictation start returns after ownership is accepted, without waiting for
  Core Audio. The helper reports device enumeration, stream construction,
  first-buffer wait, active capture, stop, runtime failure, and exit events back
  to the supervisor.
- `Starting` emits a still-connecting signal after 5 seconds and has a
  30-second active-time contract: AUHAL 8s, AUHAL termination 2s, CPAL 16s,
  CPAL termination 2s, and 2s reserve. A genuine pending macOS TCC prompt
  suspends active deadlines and has a separate 120-second watchdog. Denial and
  Stop remain immediate. A successfully opened and started native stream does
  not imply readiness: `Recording` is reached only after the callback has
  retained a nonempty mono buffer. Reaching first-buffer wait without a callback
  fails as `first_buffer_timeout`, so a successful zero-sample recording cannot
  begin.
- PCM retention and waveform publication are separate gates. The callback
  retains the first buffer before signalling readiness and continues retaining
  this generation's PCM while the supervisor accepts it. Levels remain disabled
  until that exact owner is accepted; stale/recovering workers cannot publish.
- Stream construction passes `Some(10s)` to CPAL. CPAL 0.18 CoreAudio applies
  that deadline to sample-rate convergence, but some synchronous AudioUnit
  creation/configuration/initialization calls remain uninterruptible in-process.
  If one blocks, Murmur retains exclusive ownership rather than detaching the
  worker or accepting a competing retry. Process isolation is the final fault
  boundary.
- Protocol v3 emits stable entered/completed setup-step constants that bracket
  each native Core Audio call on the AUHAL path — device resolution, unit
  creation (`audio_unit_new`: component find/instantiate/initialize), input
  enable, output disable, current-device binding, stream-format configuration,
  callback installation, and `stream_start` (exactly `AudioOutputUnitStart`) —
  plus comparable CPAL steps and the callback wait. A step that entered without
  completing therefore names the exact hung operation; the supervisor logs each
  step with elapsed time since start and repeats the last step and transition
  in the attempt-budget-exceeded event. These events contain no device label,
  UID, raw backend error, or content.
- Debug fault scenarios `hang_stream_build` and `hang_stream_build_once` run
  inside the capture worker process. They exercise the same bounded
  terminate/kill/reap boundary as a real blocked HAL call instead of parking an
  unkillable thread in the app process.
- A timed-out backend can advance only after its owned helper process group is
  positively confirmed empty. Unconfirmed termination leaves the lifecycle in
  exclusive recovery and suppresses fallback and new starts.
- If a stop caller times out first, the helper remains exclusively owned until
  it exits and is joined. That late completion publishes Idle to clear the
  current dictation's Processing state before a new recording can start.
- Capture-worker errors are reduced immediately to stable content-free kinds
  (`permission_denied`, `device_unavailable`, `stream_invalidated`,
  `invalid_input`, `resource_exhausted`, and bounded fallback kinds).
  Phase/error telemetry never includes a device label, UID, raw backend
  message, or audio/transcript content.
- Recording duration begins at accepted readiness, not at the user's initial
  activation.
- Multi-channel to mono conversion (averages channels) supports every PCM
  sample type CPAL may select, including signed/unsigned 24- and 32-bit formats.
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

1. Capture app identity, matched profile, effective settings, vocabulary version, repository-backed commands, and deny-by-default context permissions at recording start. Built-in/scanned developer terms are included only when the matching profile explicitly selects Code / technical or Local IDE project context.
2. Confirm the snapshot's model preparation completed (or load synchronously as a fallback)
3. Run one full-buffer VAD and backend transcription pass with the same snapshot
4. Run the backend-neutral transcript transformation pipeline from the snapshot's stage settings and resources
5. Persist optional file output and inject text (clipboard + optional paste) from the snapshot on the main thread
6. Reset status to Idle and clear only the matching recording generation's snapshot

Uses `IdleGuard` (RAII) to reset status on any early return or error — prevents the app from getting stuck in "processing" state.

### Transcript transformations (`transcript_transform.rs`)

`transform_transcript()` is the authoritative post-recognition entry point for both live and imported-file transcription. It owns a fixed internal sequence:

```text
raw transcript → cleanup → voice commands → Smart Correction (explicit aliases, scoped replacement knowledge, exact/derived terms, then fuzzy) → Smart Formatting → Spoken Structure → spoken numbers → IDE context → CLI formatting → final text
```

Each stage receives immutable session/source metadata plus privacy-safe enablement flags and produces privacy-safe execution metadata (`duration_us`, changed/not-changed, outcome, and required/optional failure policy). Structured stage logs never include transcript text, model/language settings, app/profile values, custom replacement values, correction vocabulary, package/script names, or project paths.

Cleanup, voice commands, Smart Formatting, Spoken Structure, spoken numbers, IDE context, and CLI formatting are required deterministic stages when enabled. Smart Correction is optional-fallback: a future recoverable correction failure leaves the preceding text intact. Explicit vocabulary aliases outrank enabled replacement knowledge; knowledge then uses project/app/global scope and repository provenance precedence before derived and fuzzy vocabulary. A bounded built-in homophone rule resolves `Maine`/`me` to the Git branch name `main` only when nearby branch or version-control language establishes that meaning, with geographic cues failing closed. The compiled matcher is captured at recording start and never queries SQLite in the stage. Smart Formatting is live-only and opt-in, fails closed outside its bounded prose grammar, and skips any utterance owned by the CLI grammar. Spoken Structure is live-only, applies the immutable Off/Basic/Extended/Union policy, and owns punctuation, layout, symbols, and `scratch that`. Spoken-number rendering is live-only and enabled by default outside the explicit Verbatim profile; it runs after Spoken Structure so fractions receive numeric separators while list ordinals remain authoritative. Explicit IDE opt-in bypasses Smart Formatting, then applies only the matching profile's fresh memory-only project index. The final CLI stage remains authoritative, uses conservative prefix/trigger/profile activation, and returns non-command prose byte-for-byte unchanged. Imported-file transcription invokes the same entry point with every stage disabled so its existing raw-ASR output remains unchanged.

The pipeline result can compare its original and final strings in memory for tests and diagnostics, but only privacy-safe stage metadata is logged. Only the final string reaches optional file output, clipboard/paste, history, and stats; delivery remains final-only and happens once.

File persistence, clipboard/paste, history, and stats are intentionally outside the transformation pipeline. Live transformation receives an opaque recording handle plus stage configuration and resources from the same immutable per-app snapshot; app/profile resolution remains owned by the context resolver.

See [Per-App Dictation Context](per-app-profiles.md) for resolver precedence, duplicate-profile compatibility, lifetime, and privacy boundaries.
See [Spoken CLI Command Formatting](cli-command-formatting.md) for activation, grammar, local lexicon layering, and safety guarantees.
See [Explicit Spoken Vocabulary Aliases](vocabulary-aliases.md) for migration, precedence, scope, validation, and privacy guarantees.
See [Voice Commands 2.0](voice-commands.md) for typed replacements, multiline snippets, variables, app scopes, conflicts, and clipboard permission boundaries.
See [Smart Formatting and Same-Utterance Backtracking](smart-formatting.md) for its explicit prose grammar, bounds, bypass rules, and privacy contract.
See [Spoken Number Formatting](spoken-number-formatting.md) for supported English scales, decimals, bounds, and fail-closed rules.
See [Local IDE Symbols and `@file` Context](ide-context.md) for opt-in, scan boundaries, ambiguity, expiry, and privacy guarantees.

## Model Downloads (`commands/models.rs`)

The `download_model` command streams Murmur-managed Whisper and sherpa downloads with `download-progress` events. FluidAudio Core ML setup runs on a blocking worker and is indeterminate because the upstream Rust bridge owns its Hugging Face download and Core ML compilation without exposing progress callbacks.

## Status Flow

```
Idle → Starting → Recording → Processing → Idle
           └────→ Recovering → Idle
```

Status is managed in `DictationState` behind a `Mutex` with poison recovery (`MutexExt` trait).
Recorder start, stop, and cancel also share a short-lived async transition
mutex for synchronous ownership commits. No Core Audio operation or supervisor
channel wait occurs while the recording-state mutex is held; a fast hotkey
release can therefore cancel a recorder that is still starting.

Model state is separate from recording status. `get_model_runtime_catalog` and
`get_model_runtime_status` expose catalog metadata plus install/lifecycle state.
Transitions emit generation-ordered `model-runtime-status-changed` snapshots;
their telemetry is privacy-safe bounded metadata and never contains transcript
text, model paths, or raw backend errors.

## Frontend Integration

- `lib/dictation.ts` has `startRecording()` and `stopRecording()` wrappers around Tauri `invoke()`
- `useRecordingState` hook manages status, transcription text, recording duration timer, and error state
- `toggleRecording()` checks current status via ref and calls start or stop accordingly
