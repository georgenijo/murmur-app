# Architecture

> Production microphone ownership is process-isolated. The Tauri process owns
> policy and retained PCM; `murmur-capture-worker` exclusively owns CPAL/AUHAL
> objects and is killable as an exact managed process group. See
> [the capture-helper ADR](decisions/2026-08-01-production-capture-helper.md).

## Overview

Murmur is a privacy-first, local voice dictation app for macOS. Its speech transcription runs on-device on the Apple Neural Engine (Core ML), the GPU (Metal/whisper.cpp), or CPU (sherpa-onnx).

Built with **Tauri 2** (Rust backend + React frontend). Transcription backends:

- **In-process transcription** — Core ML / whisper.cpp / sherpa-onnx behind one `TranscriptionBackend` trait.

---

## Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Desktop framework | Tauri 2 | Rust backend, React frontend |
| UI | React 18 + TypeScript + Tailwind CSS 4 | Vite 6 build |
| Audio capture | cpal + Core Audio CATap | Signed worker owns microphone AUHAL and explicit-session System Audio tap; host resamples channels independently to 16kHz mono |
| VAD | Silero v5.1.2 via whisper-rs | Filters silence before transcription; thread-local cached context |
| Transcription | FluidAudio (Core ML/ANE), whisper-rs → whisper.cpp (Metal), sherpa-onnx (CPU) | Selected per model from one catalog |
| Text injection | arboard + CGEvent (osascript fallback) | Clipboard-first |
| Keyboard listening | rdev (git main branch) | Global key events; one background thread for dictation and Paste Last |
| Local storage | rusqlite (SQLite) | Performance diagnostics + meetings |
| Telemetry | tracing + tracing-subscriber | Structured events: ring buffer, JSONL file, real-time frontend emission |
| System info | sysinfo + custom malloc zone | CPU, RSS, and separated Rust/FFI heap accounting |
| Release | GitHub Actions + Apple notarization | Signed DMG, hardened runtime, sidecar entitlements |

---

## Data Flow

### Dictation

```text
Hotkey event (rdev listener thread)
    |
Frontend hook (useHoldDownToggle / useDoubleTapToggle / useCombinedToggle)
    |
invoke('start_native_recording')
    |-- CPAL 0.18 capture starts; first retained nonempty callback establishes readiness
    |-- immutable DictationContextSnapshot resolved (global settings + per-app profile)
    +-- spawn_model_preparation() warms the model *concurrently with speech*
    |
invoke('stop_native_recording') --> trigger-selected tail, drain, samples resampled to 16kHz mono
    |-- cold teardown: checked native stop + drained seal may deliver before helper exit/join
    |-- standby: retain 50 ms before live-buffer snapshot; capture ownership stays with the worker
    |
Silero VAD trims silence (NoSpeech short-circuits the whole pipeline)
    |
ModelRuntimeManager::with_ready_backend() --> transcribe()
    |
transform_transcript() -- ordered, backend-neutral text pipeline (see below)
    |
injector::inject_text() --> arboard clipboard write
    |
[optional] CGEvent Cmd+V (osascript fallback) --> text appears in focused app
    |
[optional] file_output --> numbered .txt / .wav
    |
'transcription-complete' event + PerformanceRunV1 persisted
```

The **transcript transform pipeline** (`transcript_transform.rs`) runs stages in exactly this order, each with a declared failure policy and per-stage timing/outcome telemetry:

1. `cleanup` — filler removal, capitalization
2. `smart_correction` — vocabulary matcher (explicit aliases, register/cue-gated contextual variants, exact tier + optional fuzzy tier)
3. `homophone_context` — opt-in live-only bounded homophone scan; eligible passes promptly use the existing unavailable fallback (#1081), with no FoundationModels preparation/generation; deterministic text retained
4. `smart_formatting` — deterministic prose grammar, lists, same-utterance backtracking (live only)
5. `spoken_structure` — punctuation, breaks, paired delimiters, symbols, and sentence-boundary editing (live only)
6. `spoken_numbers` — English cardinals, times, dates, years, labels, and versions rendered under the frozen text register's prose or technical policy (live dictation)
7. `ide_context` — project symbol correction and `@file` canonicalization (live only, opt-in)
8. `cli_command` — spoken CLI command formatting

### Meeting capture

```text
explicit Start Meeting
    |
signed murmur-capture-worker --production-v11
    |-- microphone AUHAL callback --> preallocated SPSC ring --> optional AEC3 --> Me frames
    +-- private unmuted CATap + aggregate IOProc --> SPSC ring --> Them frames
    |
bounded protocol reader (channel/sequence/rate/offset validation)
    |
per-channel streaming resampler + VAD chunker
    |                                      |
fsynced spool WAV --> pending SQLite row   +-- opted-in Them PCM --> bounded private session WAV
    |                                                                          |
shared ModelRuntimeManager (one chunk at a time)                               |
    |                                                                          |
final segment + FTS transaction --> optional WAV deletion                      |
                                                                               |
capture completes --> idle-only diarization coordinator <---------------------+
    |
network-denied worker + SHA-256-pinned Core ML speaker model
    |
bounded speaker turns --> conservative assignment to complete Them segments
    |
session-scoped Speaker N labels + meeting-speakers-updated --> temporary WAV deleted
```

Meeting capture is mutually exclusive with dictation, imported-file
transcription, corpus capture, and benchmarks. The worker owns tap teardown;
normal stop waits for a teardown receipt and the host confirms termination.
AEC3 is constructed only for an opted-in meeting. It uses the System stream as
a private render reference, never changes Them, and bypasses to raw microphone
PCM on initialization, processing, or backlog failure.
See [Meeting Capture](features/meeting-capture.md).

Calendar naming is an explicit review action, separate from capture. The main
window requests Calendar permission, then `commands/meeting_calendar.rs` reads
the stored session interval. `calendar/native.rs` owns a short-lived EventKit
store and enumerates overlapping events off the main thread. Apply repeats the
query and matches a digest of the displayed occurrence and metadata before the
shared metadata transaction writes a title, attendees, and `calendar` source.
No event objects or lookup cache survive the operation. Calendar identifiers,
notes, and URLs never enter the meeting database. See [Calendar naming](features/meeting-calendar.md).

The separate `meeting_suggestions.rs` coordinator stays dormant until explicitly
enabled. It keeps a bounded, in-memory snapshot of video-call events and wakes
at event boundaries or native activity changes. A native EventKit store lives
only while enabled to receive Calendar changes; queried event objects remain
scoped to their query. The overlay presents a suggestion without claiming audio
or preparing a model. Accept sends an opaque token to the main window's normal
`start_meeting` call, which rechecks the native frontmost bundle ID and all busy
owners under `recording_transition` before consuming it and saving the title.
See [Meeting suggestions](features/meeting-suggestions.md).

Retained-audio playback uses `meeting_audio.rs` and two bounded data commands:
a paginated timeline manifest and a binary WAV range reader. Both resolve the
session and segment in SQLite; the frontend supplies no filesystem path. A
separate busy command hydrates playback admission from the native recording
owners. The main webview schedules small decoded chunks on their saved Me/Them
timeline, preserving overlapping speech and silent gaps. Deletion invalidates
pending reads and notifies only the main window. See [Meeting audio playback](features/meeting-audio-playback.md).

`meeting_bundle.rs` exports a with-audio `.zip` (review document, retained WAVs,
manifest). It lists segments from the playback manifest and reads each WAV through
the same validated `meeting_audio` open path, so no second path resolver exists.
Entries are stored, and the archive streams to a unique exclusive temp file that is
renamed only after a final revocation and capture check. One checked database
connection serves the whole operation, and a separate single-slot permit keeps
playback's two-slot pool free. See
[Meeting review workspace](features/meeting-review-workspace.md#export-with-audio).

## Five-window architecture

Each window is a separate webview with its own Tauri capability set.

| Window | Label | Entry point | Size | Purpose |
|--------|-------|-------------|------|---------|
| Main | `main` | `index.html` | 1120×820, minimum 720×560 | Settings, embedded diagnostics, recording controls, history, stats, onboarding, and modals |
| Diagnostics | `diagnostics` | `diagnostics.html` | 1040×760 | Persistent pop-out for events, runs, performance, UI latency, comparisons |
| Overlay | `overlay` | `overlay.html` | 260×100 | Dynamic Island notch widget. Always on top, transparent, and non-activating |
| Dictation Preview | `dictation-preview` | `dictation-preview.html` | 460×104 | Live Core ML and Whisper Tiny/Base transcript preview under the notch. Always on top, transparent, non-activating, and click-through |

Main and Diagnostics hide on close instead of being destroyed. Overlay calls the shared non-activating window treatment in `commands/native_window.rs`. Dictation Preview uses its `PopoverSpec` transport, which applies the same window level and activation policy. Every raw `NSWindow` mutation runs on the main thread through `run_on_main_thread`. macOS 26 hard-traps on an off-main mutation (#325).

Rust owns the native frame for each overlay or popover. `geometry_for()` calculates the Overlay frame. The `frame()` function in `commands/dictation_preview.rs` calculates the preview frame. Checked-in Rust and frontend fixtures cover the shared geometry contracts.

Main owns appearance writes, the durable saved-theme library, Open VSX
discovery, and application-level native `setTheme`. Main and Diagnostics resolve System mode to a concrete `data-appearance` with a
parser-blocking, same-origin bootstrap plus a strictly validated resolved-token
cache to avoid a wrong-theme first paint. Library variants are compiled into
that small active document, so first paint never depends on the larger library
or a network. The Diagnostics pop-out observes the
same local storage as Main so its latency table updates while the user
navigates. Overlay remains unsynchronized transparent,
always-dark glass surfaces.

---

## Rust Backend (`app/src-tauri/src/`)

### Capture worker boundary

The signed `murmur-capture-worker` owns production microphone capture and the
phase-one System Audio CATap. Production protocol v11 has capture-scoped
identity, nonce, strict bounded frames, and separate mic/system channel
sequences. Before each live backend opens the microphone, an `InputResolution`
message reports only bounded resolution evidence: backend, enumeration success,
whether the pinned input was present (when knowable), capped input count, and
default-input availability. It never carries a device ID, display name, or raw
error. Native callbacks write only to preallocated SPSC rings; the worker's
drain loop owns protocol I/O. Release builds validate fixed identifier, Team ID,
hardened runtime, and entitlements before spawn, then retain exact managed
process-group ownership through confirmed exit. The inventory path bounds its
ordinary termination wait, retracts authoritative state and blocks replacements
before transferring an unconfirmed child to the single prestarted app-owned
reaper service, then performs a bounded exit drain; the worker parent watchdog
remains the final boundary if the host exits first. If that reaper service
cannot start, inventory stays unavailable and no inventory helper is launched.
The earlier health probe
remains available for packaging and callback-boundary validation. See the
[capture-helper ADR](decisions/2026-08-01-production-capture-helper.md).

An explicit pinned input that resolves as unavailable in both AUHAL and CPAL
may run at most two additional complete primary-to-fallback resolution passes
using the recording's immutable memo-resolved backend order. The
500 ms gaps are cancellable, every pass uses the same immutable raw device UID,
and no pass substitutes the OS default. Mixed failures, system-default capture,
retained audio, or an unavailable device after first PCM remain terminal.

The per-device backend memo that resolves that order (last ready backend,
promotion-disabled flag, consecutive fast-rescue count) is durable: a bounded,
versioned `capture-memo.json` beside `settings.json` in the app data
directory, loaded once during setup and republished atomically on a detached
writer thread after each change, so a relaunch keeps the shrunken retry budgets
an affected machine already earned. It is fail-open and never blocks capture
start; device keys
stay local and never reach logs or telemetry. See
[transcription.md](features/transcription.md).

### Module map

| Module | Purpose |
|--------|---------|
| `lib.rs` | App wiring: module declarations, `State`, `MutexExt`, 158 registered commands, setup, tray, run loop |
| `alloc.rs` | Custom macOS malloc zone ("RustHeapZone") so Rust heap is accounted separately from whisper.cpp's FFI heap |
| `audio.rs` | AUHAL/CPAL capture-worker supervision, stable device-ID selection, bounded pinned-input re-resolution, durable per-device backend/retry-budget memo, typed resolution/error/phase telemetry, first-buffer readiness, mono mix, 16kHz resample, `audio-level` emission |
| `audio_inventory.rs` | App-lifetime versioned microphone inventory; supervised passive-worker invalidation, coalesced startup/five-minute fallback refresh, idle-HAL deferral, stale-cache policy, local-only change events, and privacy-safe shipper aggregate |
| `audio_lifecycle.rs` | App-lifetime single-owner supervisor; async start, generation cancellation, deadlines, generation-gated publication, and strict worker ownership through exit. Also owns the Fn / Globe speculative capture phase (`AudioOwner::Speculative`): adopted by the matching confirmed start, or discarded through confirmed teardown with a 1.5 s backstop (`audio_speculative.rs`). Hands a flowing visible Settings microphone test (`AudioOwner::Preview`) on the same input and topology to that key-down capture or to a confirmed dictation start in place (`audio.preview_handed_off`); the worker keeps running and only post-handoff PCM is retained. Any other test is stopped first and the start queues behind its exit (`audio.preview_yielded`) |
| `audio_decode.rs` | Decoding imported audio files for `transcribe_file` |
| `audio_graph_snapshot.rs` | Deadline-bounded Core Audio HAL snapshots for armed capture-hang reports, plus content-free graph counts and Murmur audio-owner state |
| `capture_helper_probe.rs` | Probe-only capture-helper handshake, callback-health observation, cancel, and confirmed termination evidence |
| `code_signing.rs` / `managed_child.rs` | Runtime helper identity validation and direct-child/process-group ownership primitives |
| `accuracy_corpus.rs` | Internal-benchmark-only `accuracy-v1` corpus loader (corrected-history or synthetic clips) for `scripts/murmur_accuracy.py` |
| `benchmark.rs` | Performance Lab: fixture corpus, scoring (raw/normalized/delivered WER), reports |
| `capture_agent_probe.rs` | Opt-in LaunchAgent service and recovery probe CLI. Production dictation does not use this path |
| `capture_health.rs` | Bounded capture startup and fallback history derived from telemetry, persisted in `capture-health-v1.json` |
| `commands/microphone_startup_benchmark.rs` | Main-window-gated production capture startup cycles, exact-owner progress/cancel, and typed privacy-safe export |
| `cleanup.rs` | Filler removal and capitalization |
| `cli_command.rs` | Spoken CLI command grammar and lexicon |
| `correction.rs` | Smart Correction matcher (exact + fuzzy tiers) |
| `delivery_recovery.rs` | Generation-bound last-delivery memory and one-at-a-time clipboard or paste retry |
| `diarization_assignment.rs` | Validation of bounded speaker turns and conservative assignment to complete remote meeting segments |
| `diarization_audio.rs` | Private temporary owner for up to two hours of 16 kHz remote meeting audio. Drop deletes the WAV |
| `diarization_model.rs` | Locked download, SHA-256 validation, status, and removal for the pinned Core ML speaker model |
| `dictation_context.rs` | Immutable per-recording context snapshot resolution |
| `dictation_diagnostics.rs` | Explicit one-shot private dictation capture store with bounded text, retention, deletion, and upload |
| `dictation_diagnostics_contract.rs` | Cross-path tests for diagnostic arming, exact recording ownership, content bounds, revocation, and corrupt-data removal |
| `dictation_telemetry.rs` | Stable content-free lifecycle telemetry and error classification for accepted live dictations |
| `commands/export.rs` | `save_text_export`: validated, atomic sink for user-chosen text exports |
| `commands/corpus.rs` | Guided capture-only personal corpus recording, WAV integrity checks, quality reports, and benchmark fixture loading |
| `commands/dictation_diagnostics.rs` | Window-gated arm, status, list, read, delete, and upload commands for private dictation captures |
| `commands/integrations.rs` | Local Launch Services availability probe for the optional NotchPill companion app |
| `commands/microphone_preview.rs` | Main-window microphone test lifecycle, input validation, VAD sensitivity, and exact-owner stop |
| `commands/updater.rs` | App-translocation status and opt-in updater canary file transport |
| `evaluation.rs` | Versioned fixture evaluation harness (`murmur-eval`) |
| `file_output.rs` | Numbered `.txt` / `.wav` output |
| `frontmost.rs` | Native frontmost-app query + running-application list |
| `dock_watch.rs` | Live Dock accessibility geometry → floating-pill limit, glide, static fallback |
| `hang_diagnostics.rs` | Server-armed, consent-bound capture-hang bundle collection with bounded native probes and upload |
| `ide_context.rs` | Memory-only bounded IDE symbol / root-relative file index |
| `injector.rs` | Clipboard write, CGEvent paste (osascript fallback), focused-field AX role checks |
| `keyboard.rs` | Dictation and Paste Last detectors on one shared rdev thread; `keyboard/fn_capture.rs` opens, binds, or discards Fn / Globe key-down capture from detector state |
| `meeting_capture.rs` | Meeting supervisor: worker protocol, per-channel resample/VAD chunking, durable spool publication, serialized inference, teardown |
| `meeting_store/` | SQLite meeting sessions/segments/FTS: migrations, backup/recovery, search, retention, audio ownership |
| `meeting_artifact.rs` | Legacy stored artifact validation for database compatibility |
| `meeting_diarization.rs` | Idle-only speaker worker ownership, foreground preemption, bounded retries, turn validation, and assignment publication |
| `meeting_diarization_probe.rs` | Debug-only hardware proof for diarization preemption and seeded meeting-review fixtures |
| `meeting_review.rs` | Meeting review validation, edits, export rendering, and session-scoped speaker-label resolution |
| `log_shipper.rs` | Tails privacy-stripped telemetry JSONL to bounded NDJSON batches and advances its durable offset only after acknowledgment |
| `model_runtime.rs` | Model catalog + lifecycle manager (load/warm/readiness/unload, generation-ordered status events) |
| `microphone_auto.rs` | Cached-only Smart Auto input policy that freezes an eligible stable microphone ID for one recording |
| `microphone_preview.rs` | Capture-only microphone-test state, VAD analysis, level accumulation, and quiet or clipping classification |
| `output_quieting.rs` | Opt-in mute of the default output during live dictation: status-seam signals, cue-gated background Core Audio mute/unmute, expiring crash record, bounded restore retries |
| `model_artifact.rs` | Size and prefix validation that rejects HTML, JSON, and Git LFS responses masquerading as downloaded model binaries |
| `coreml_installer.rs` | Same-signed killable Core ML install worker/supervisor: bounded phases, hard deadline, confirmed process-group cleanup |
| `performance_metrics/` | SQLite run history, stage timings, resource samples, retention |
| `platform/` | macOS CPU/resource metrics seam |
| `resource_monitor.rs` | CPU/RSS sampling, 1s heartbeat, idle-timeout enforcement |
| `smart_formatting.rs` | Deterministic prose formatting and same-utterance backtracking |
| `smart_turn.rs` | Smart Auto-Stop: Smart Turn log-mel features, scoring on the sherpa-linked ONNX Runtime, model pins |
| `spoken_numbers.rs` | Deterministic English spoken-cardinal rendering |
| `spoken_structure.rs` | Single owner for spoken punctuation, layout, symbols, arbitration, and `scratch that` |
| `sqlite_support.rs` | Shared SQLite pragma, integrity, schema-version, sidecar quarantine, timestamp, and newest-first file helpers |
| `state.rs` | `DictationStatus`, `TransformStatus`, `DictationState`, `AppState` |
| `telemetry.rs` | Structured event system: `TauriEmitterLayer`, ring buffer, JSONL, privacy stripping |
| `transcriber/` | `TranscriptionBackend` trait + whisper / parakeet / coreml implementations |
| `transcript_transform.rs` | The ordered post-recognition pipeline and its stage contracts |
| `vad.rs` | Silero VAD filtering, thread-local context cache |
| `vocab.rs`, `vocabulary_alias.rs` | Code-vocabulary scanning and explicit spoken aliases |

Commands and command-side window transports live under `commands/`: `benchmark`, `corpus`, `dictation_diagnostics`, `dictation_preview`, `export`, `integrations`, `keyboard`, `logging`, `meeting`, `microphone_preview`, `microphone_startup_benchmark`, `models`, `native_window`, `overlay`, `performance`, `permissions`, `recording`, `settings_store`, `theme`, `tray`, and `updater`. Theme resolution remains frontend-only. `commands/theme.rs` is a main-window-gated UTF-8 file transport with 256 KiB regular-file reads and 64 KiB atomic sibling-temp exports. `commands/settings_store.rs` provides the independent main-only 1 MiB `theme-library.json` blob boundary.

### `state.rs` — Shared State

```rust
enum DictationStatus { Idle, Starting, Recording, Recovering, Processing }
enum TransformStatus { /* Idle, Capturing, Connecting, Listening, Thinking, ReviewPending, ... */ }

struct AppState {
    dictation: Mutex<DictationState>,
    recording_transition: tokio::sync::Mutex<()>,   // serializes start/stop/cancel
    model_runtime: ModelRuntimeManager,
    recording_id: AtomicU64,                        // generation counter; supersedes stale work
    meeting_generation: AtomicU64,
    meeting_active: AtomicBool,                     // native capture/setup owner
    meeting_inference_active: AtomicBool,           // live or crash-recovery inference
    cancelled_id: AtomicU64,
    settings_revision: AtomicU64,
    correction_matcher: Mutex<...>,                 // immutable, swapped by generation
    ide_context: Mutex<IdeContextStore>,
    // ...
}
```

Two patterns do most of the concurrency work:

- **Generation counters.** recording generation IDs are monotonic. Every async continuation re-checks whether it still owns the current generation before mutating shared state, so a delayed handler can never cancel or overwrite the pass that replaced it.
- **Immutable snapshots.** A recording resolves its full context (model, language, delivery, profile overrides, vocabulary, transform stage config) once at start. Settings or focus changes mid-recording apply to the *next* session, never the running one.

### Model runtime (`model_runtime.rs`)

One catalog is the single source of truth for all seven shipped models:

| Model | Backend | Accelerator | Size |
|-------|---------|-------------|------|
| `parakeet-tdt-0.6b-v3-coreml` | FluidAudio / Core ML | Apple Neural Engine | ~470 MB |
| `parakeet-tdt-0.6b-v2-fp16` | sherpa-onnx | CPU | ~1.2 GB |
| `tiny.en` | whisper.cpp | Metal GPU | ~75 MB |
| `base.en` | whisper.cpp | Metal GPU | ~150 MB |
| `small.en` | whisper.cpp | Metal GPU | ~500 MB |
| `medium.en` | whisper.cpp | Metal GPU | ~1.5 GB |
| `large-v3-turbo` | whisper.cpp | Metal GPU | ~3 GB |

Each entry declares its backend, accelerator, capabilities (multilingual, translation, timestamps, confidence, punctuation control), install kind, platform requirement, `warm_on_startup`, and `retry_unfiltered_on_empty`. Unknown model identifiers **fail closed** — Murmur never silently falls back to a different model.

The manager serializes load/warm/readiness/unload, emits generation-ordered `model-runtime-status-changed` events, and reports a `LoadReport` (`lock_wait_ms`, `load_ms`, `cache_hit`) that feeds the performance record. Idle release is user-configurable (`idleTimeoutMinutes`, including a power-adapter-only keep-loaded policy).

**Warm-on-record:** `start_native_recording` kicks off `spawn_model_preparation()` immediately after audio capture starts, so model load overlaps with the user still speaking. By the time the key is released the model is usually resident.

### `transcriber/` — Inference Backends

```rust
trait TranscriptionBackend: Send + Sync {
    fn name(&self) -> &str;
    fn load_model(&mut self, model_name: &str) -> Result<(), String>;
    fn is_model_loaded(&self, model_name: &str) -> bool;
    fn transcribe(&mut self, samples: &[f32], language: &str,
                  initial_prompt: Option<&str>, smart_punctuation: bool) -> Result<String, String>;
    fn token_count(&self, text: &str) -> Option<usize>;
    fn model_exists(&self) -> bool;
    fn models_dir(&self) -> Result<PathBuf, String>;
    fn reset(&mut self);
}
```

- **Whisper** — `WhisperContext` and `WhisperState` are both cached; GPU/Metal buffers are allocated once and reused across transcriptions. Greedy sampling, blank suppression, timestamp-based continuation for long audio.
- **Core ML (FluidAudio)** — Parakeet v3 on the ANE. Decoder state is reset between one-shot dictations. An empty result after VAD trimming retries once with the original unfiltered audio.
- **Parakeet CPU (sherpa-onnx)** — English fallback, also the non-macOS path.

All backends take one authoritative final-after-stop pass; delivery happens exactly once.
After confirmed delivery, Core ML shadow diagnostics compare VAD-filtered prefixes
after Idle and the captured delivery timestamp. Full-clip shadow bounds are also
recomputed there, with no bounds collection in final VAD. Live determinism always
records `notMeasuredInApp`; an ignored developer-only model test measures repeat
decode equality outside the app.
Shadow work has no shared-backend capability and never supplies delivered text;
only content-free evidence amends the completed
run UUID. See [performance diagnostics](features/performance-diagnostics.md#early-final-shadow-measurement-1081-phase-3). Core ML and Whisper Tiny/Base also emit display-only partials. Whisper partials own a separate cancellable CPU context for the recording, leaving the final Metal context and runtime lock untouched.

### `keyboard.rs` — Keyboard Detection

One persistent rdev background thread feeds dictation hold-down and double-tap detectors plus Paste Last.

- **Hold-down** — 2 states; press emits `hold-down-start`, release emits `hold-down-stop`. Combos (trigger+letter) cancel.
- **Double-tap** — 4 states; each tap under 200ms, gap under 400ms. Rejects holds, combos, slow taps, triple-tap spam. Emits `hotkey-tap-rejected` when an idle first tap expires (opt-in overlay feedback).
- **Both** — deferred hold promotion: a 200ms timer thread with an atomic invalidation counter decides tap-vs-hold. Releasing before 200ms invalidates the timer; still held at 200ms promotes to a hold.

**macOS thread safety.** rdev's key translation uses TIS/TSM APIs that must run on the main thread. `rdev::set_is_main_thread(false)` is called before `listen()`, which makes rdev marshal only those calls to main via `dispatch_sync` while the listener loop stays on its background thread. Without it, the app silently segfaults.

Global modifier hotkeys recover when macOS disables the underlying event tap, and the hot path performs no main-thread key-name translation and ignores mouse movement.

### `injector.rs` — Text Injection

1. **Clipboard** (always): `arboard`. Empty/whitespace-only text is skipped.
2. **Auto-paste** (optional): waits the configured delay, then posts a native `CGEvent` Cmd+V key-down/key-up pair to `CGEventTapLocation::HID`. Event-construction failure retries once; there is no paste-event AppleScript fallback; the whole operation is timeout-bounded. A typed, generation-gated delivery outcome distinguishes completed automatic paste, confirmed clipboard-only delivery, failed clipboard write, and an unconfirmed timeout. The overlay shows `⌘V` only for the confirmed clipboard-only case; detailed failures remain on `auto-paste-failed` for the main window.

Focused-field role checks use native AX with a per-element messaging timeout and a bounded osascript compatibility query. The content-free `deliveryTiming` performance companion separates queue, owner wait, work and native/Smart child timings. Normal delivery retains fresh process checks without diagnostic-only AX window IPC; Smart's exact-window checks remain mandatory.

### `telemetry.rs` — Structured Events

```text
tracing event
    |
    +--> Pretty text file (app.log / app.dev.log)
    |
    +--> TauriEmitterLayer
             |
             +--> Ring buffer (500 events, FIFO)
             +--> JSONL file (events.jsonl / events.dev.jsonl, rotated at 5 MB)
             +--> 'app-event' emitted to all windows
```

Six active streams plus the retired `query` stream for reading old diagnostics: `pipeline`, `audio`, `keyboard`, `transform`, `meeting`, `system`.

**Privacy stripping.** In release builds, all string fields on `pipeline` events are removed from the data object; only numerics survive. `transform` and `meeting` events are stricter still and stripped in *all* builds: every key, type, and stable string value must satisfy that stream's exact allowlist. The `performance.store_operation_failed` system event is likewise reduced in every build to its stable code, operation, safe SQLite class, attempt count, and recording ID. Anything else is dropped at the layer, independent of the call site. Transcript content, SQL, database paths, and stderr therefore cannot enter those events. The fleet shipper also rejects the entire `meeting` stream and malformed JSONL, so meeting content and lifecycle never leave the Mac.

### Local storage

Two SQLite databases, all stored locally:

| Store | File | Retention |
|-------|------|-----------|
| Performance diagnostics | `diagnostics/performance.sqlite3` | 200 completed runs, 600 resource samples, 8 follow-ups per run |
| Meetings | `meetings/meetings.sqlite3` | User-managed searchable sessions; age/count caps applied before start; audio off by default |

---

On upgrade, startup removes the retired Voice Query store
`query-history/query-history.sqlite3` and Assistant mirror
`assistant/conversations-v1.json`, including only their `-wal`, `-shm`, and
`-journal` companions. These private questions and answers no longer have a
view or deletion UI. Cleanup pins directories, refuses symlink traversal,
never reads or logs content, and warns without blocking startup on failure.
Other app data and remote provider sessions are untouched. Old settings with
these retired fields still load; active dictation and meeting preferences survive.

## Launch preparation

`main.rs` anchors a monotonic clock at process entry. After telemetry becomes
available, Rust setup reads the same bounded, quarantine-aware `settings.json`
as `load_settings_blob`. Native preparation requires settings schema version 7,
an explicitly completed durable onboarding gate, valid model/key/mode IDs,
Accessibility permission, and `disabled: false`. Missing, unreadable, legacy,
or ambiguous settings retain the frontend-driven startup path.

`launch.rs` starts the single shared rdev thread before unrelated store setup
and prepares models with the catalog's existing `warm_on_startup` policy on a
blocking worker. `set_is_main_thread(false)` still precedes `rdev::listen`.
Detectors remain disarmed until the frontend has configured capture and
subscribed; setup never opens a microphone or applies partial capture settings.
Identical frontend listener registration is idempotent in Rust. A stopped or
changed registration still resets the detectors normally. Frontend confirmation
preserves the already warmed model. A native worker rechecks launch ownership
under the backend lock so it cannot replace a newer frontend model selection.

The main window mirrors onboarding completion/reset in the settings blob as
`launchOnboardingComplete`; this metadata is excluded from capture settings.
Durable gate hydration precedes App mounting. Legacy local flags migrate on the
first frontend boot, so upgrades use the frontend fallback once. Native startup
does not reproduce the permission/model checks used to grandfather old installs.

This change retains the main window's pre-render history/stats/theme hydration.
History mutations use React snapshots; stats use synchronous read/modify/write;
theme startup can repair and persist a library and reconcile active selections.
Deferring only the reads could overwrite durable data with empty or stale
snapshots. A later change needs mutation replay and a theme readiness boundary.
Native stores also remain synchronous: diagnostics and meeting recovery currently
assume initialization before their commands run. No webview declarations change.

Content-free `system.launch_*` events carry `launch_elapsed_ms` in the structured
log/event ring; see [Performance diagnostics](features/performance-diagnostics.md#launch-milestones).

## Frontend (`app/src/`)

`App.tsx` is a thin orchestrator that wires hooks together. All recording-mode hooks are always called (Rules of Hooks) and gated by an `enabled` prop:

```tsx
useHoldDownToggle({ enabled: settings.recordingMode === 'hold_down', ... });
useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', ... });
useCombinedToggle({ enabled: settings.recordingMode === 'both', ... });
```

See [reference/hooks.md](reference/hooks.md) for the full hook inventory.

Two rules keep the multi-window state coherent:

- **`transcription-complete` is the single source of truth** for history and stats. Entries are added only from the Rust event, never in `handleStop()` — otherwise an overlay-initiated recording double-counts.
- **The overlay has no shared React settings context.** `useOverlaySettingsMirror` seeds from its cache and refreshes guarded, read-only durable snapshots after host commit notifications; quick controls retain the latest snapshot.
- **Appearance has a separate ownership domain.** Main is the only writer and runtime for the small `murmur-appearance` active cache and the revisioned `murmur-theme-library`. Open VSX access is explicit and renderer-side; installed variants compile locally into the active cache. Transparent utility windows remain outside theme synchronization.
- **Meeting history is Rust-owned.** `useMeetings` reads SQLite through bounded commands and events; transcript localStorage remains dictation-only.

---

## Tauri Commands

158 commands are registered in `lib.rs`. See [reference/commands.md](reference/commands.md) for the full signature-level list, grouped by module.

## Events

See [reference/events.md](reference/events.md) for every Rust → frontend event, its payload, and its listeners.

---

## macOS Permissions

| Permission | Required For |
|-----------|-------------|
| Microphone | Audio capture — dictation and meetings |
| Accessibility | Global hotkeys (rdev), auto-paste, AX selection capture, AX write-back |
| System Audio (optional; macOS 14.2+) | Meeting playback capture as Them; tap exists only during an explicit check or active meeting |

Accessibility is checked via `AXIsProcessTrusted()`; the prompt is triggered with `AXIsProcessTrustedWithOptions()`. Microphone access can be requested in-app via `AVCaptureDevice.requestAccess`. Both core permissions have reset paths for stale TCC entries. System Audio uses an explicit short-lived CATap probe and links denial to the matching privacy pane; passive status reads never create a tap.

---

## Data Directories

Murmur uses two roots — a legacy one from before the rename, and the Tauri app-data root:

```text
~/Library/Application Support/local-dictation/
├── models/                    # whisper .bin files, Silero VAD, sherpa-onnx bundle
└── logs/                      # app.log, events.jsonl (+ .dev variants)

~/Library/Application Support/com.localdictation/        (com.localdictation.dev in dev)
├── diagnostics/               # performance.sqlite3
└── meetings/                  # meetings.sqlite3, backups/, quarantine/, audio/
```

Earlier versions also kept a `knowledge/` folder here (`knowledge.sqlite3` with
`backups/` and `quarantine/`). Murmur no longer opens, reads, migrates, or
deletes it; the folder can be removed by hand.

Core ML models are managed by FluidAudio under `~/Library/Application Support/FluidAudio/Models/`.

---

## Build & Release

### Dev

```bash
python3 scripts/build_capture_helpers.py   # once, before anything else on macOS
cd app && npm install
cd app && npm run tauri:dev                  # hot reload, isolated dev bundle/data
cd app && npm run tauri build                # .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1
cd app && npx tsc --noEmit && npm test
```

`tauri.macos.conf.json` declares the three capture-helper executables. Build them before `tauri dev`, `cargo check`, or `cargo test`. See [DEVELOPMENT.md](DEVELOPMENT.md).

`tauri.dev.conf.json` overrides only `identifier` → `com.localdictation.dev` and `productName` → `Local Dictation Dev`, so dev installs alongside the release build.

### Release pipeline

1. Push a `chore: bump version ...` commit to trusted `main`.
2. `Release Build` runs frontend verification and macOS signing/notarization (including capture helper entitlements) concurrently, with launch smoke tests.
3. Artifacts plus SHA-256 provenance are stored under names keyed by the exact commit SHA.
4. The completed-build event verifies the trusted push, version-bump message, matching versions, exact run ID, source SHA, and immutable artifacts.
5. Promotion creates `vX.Y.Z`, verifies remote `.sig` files, generates `latest-v2.json` plus the legacy-safe `latest.json`, then publishes.

Release finalization **fails closed** on any unexpected executable in the bundle: only the Murmur binary and its three capture helpers may ship (#324). Manual dispatches are rehearsal-only. See [release.md](release.md).

### Release profile

```toml
[profile.release]
panic = "abort"
codegen-units = 16
lto = false
opt-level = "s"
strip = false   # retain Tauri's updater bundle-type marker
```

---

## Thread Model

| Thread / task | Lifetime | Purpose |
|--------|----------|---------|
| rdev keyboard listener | App lifetime | Global key event loop for all three detectors |
| Keyboard heartbeat | App lifetime | Trace-level liveness check every 60s |
| Resource heartbeat (tokio) | App lifetime | 1s CPU/RSS sample → diagnostics; idle-timeout enforcement |
| Sidecar maintenance (tokio) | App lifetime | 30s tick: RSS ceiling + idle unload |
| Sidecar reader thread | Per spawn | Reads the helper's protocol stream |
| Audio capture | Per recording | cpal stream + mono mix |
| Meeting capture worker | Per explicit meeting/probe | Microphone AUHAL plus CATap/aggregate/IOProc; dual bounded callback rings |
| Meeting chunker/inference | Per meeting + startup recovery | Per-channel VAD/spool publication and serialized shared-model transcription |
| Hold-promotion timer | Per key press (both mode) | 200ms sleep, atomic validity check |
| Model preparation | Per recording start / model change | Warms the backend concurrently with speech |

Plus tokio `spawn_blocking` for VAD (its context is `!Send`), downloads, and injection dispatch.

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Model handed to the helper as fd 3 | The helper takes no paths and needs no filesystem reach; pinning is enforced host-side by size + SHA-256 |
| One model catalog, fail-closed | Unknown identifiers error instead of silently substituting a model with different accuracy/latency |
| Warm-on-record | Model load overlaps with speech instead of being charged to the user after key release |
| Generation counters everywhere | A delayed continuation can observe that it lost ownership rather than corrupting the pass that replaced it |
| Immutable per-recording context | Settings or focus changes mid-utterance can't reinterpret a recording already in flight |
| Secure-field fail-closed | An *errored* AX secure-field check is treated as "possibly a password field" — no read, no clipboard fallback |
| Transform telemetry allow-list | String keys and values must be in a stable vocabulary; leakage is prevented at the layer, not at the call site |
| Ordered transcript pipeline | One entry point, declared stage order and failure policy, per-stage timing — instead of ad-hoc string munging per backend |
| Rust owns all window geometry | Pure `geometry_for()` / `popover_geometry_for()` with fixtures on both sides; no drifting CSS pixel constants |
| Main-thread `NSWindow` mutation | macOS 26 hard-traps on off-main mutation (#325) |
| Autorelease pool around off-main AppKit queries | Tokio/std threads have no run-loop pool, so AppKit's internal autoreleases (`NSWorkspace`, `NSRunningApplication`, Launch Services) never drain: a 500 ms polling watcher leaked ~2.5 MB/hour idle. Wrap each synchronous query in `objc2::rc::autoreleasepool` inside the helper, return owned Rust data, and never hold the pool across an `.await` |
| CGEvent paste with osascript fallback | Native path avoids process spawn; osascript remains the compatibility net |
| Custom malloc zone | Separates Rust heap from whisper.cpp's FFI heap; `GlobalAlloc` wrappers drift when FFI frees Rust allocations |
| `set_is_main_thread(false)` | Prevents the rdev TIS/TSM segfault on the background listener thread |
| `MutexExt::lock_or_recover()` | Survives a panic while a lock is held; no stuck UI state |
| `IdleGuard` RAII | Guarantees status reset on every error path in the pipeline |
| Clipboard-first delivery | Reliable across all apps; auto-paste is layered on top and never the only path |
| Explicit-session CATap | Avoids idle tap churn; setup/teardown is phased, timeout-bounded, and process-isolated |
| Durable chunk before inference | A crash loses at most each channel's open VAD chunk; published chunks remain recoverable from SQLite + fsynced spool audio |
| Per-window least privilege | Overlay and dictation preview get minimal capabilities; only the main window gets the full set |
| Main-only native appearance | Main owns application `setTheme`; transparent glass windows never join theme synchronization |
| Bounded theme transport | Dialogs select paths; main-window-gated Rust commands read up to 256 KiB of UTF-8, keep exports at 64 KiB, and publish exports atomically without touching the clipboard |
| Data-only community themes | Open VSX is queried only from an explicit dialog; supported-license packages are checksum/ZIP/identity bounded and only declared workbench colors are converted—extension code and assets never execute |
