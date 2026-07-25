# Architecture

## Overview

Murmur is a privacy-first, local-only voice dictation app for macOS. You speak, it transcribes — no cloud, no API keys, no internet. All inference runs on-device: transcription on the Apple Neural Engine (Core ML), the GPU (Metal/whisper.cpp), or CPU (sherpa-onnx), and selected-text rewriting through a signed local-LLM helper process.

Built with **Tauri 2** (Rust backend + React frontend). Two shipped inference stacks:

- **In-process transcription** — Core ML / whisper.cpp / sherpa-onnx behind one `TranscriptionBackend` trait.
- **Out-of-process rewriting** — a separate signed `murmur-llm-sidecar` executable runs llama.cpp. The app crate must **never** link `llama-cpp-2` directly: its ggml ABI clashes with whisper's.

---

## Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Desktop framework | Tauri 2 | Rust backend, React frontend |
| UI | React 18 + TypeScript + Tailwind CSS 4 | Vite 6 build |
| Audio capture | cpal | Native multi-channel input, mono mix, 16kHz resample |
| VAD | Silero v5.1.2 via whisper-rs | Filters silence before transcription; thread-local cached context |
| Transcription | FluidAudio (Core ML/ANE), whisper-rs → whisper.cpp (Metal), sherpa-onnx (CPU) | Selected per model from one catalog |
| Local rewriting | `murmur-llm-sidecar` (llama.cpp, Qwen2.5-1.5B-Instruct Q4_K_M) | Separate signed process, model passed as read-only fd |
| Text injection | arboard + CGEvent (osascript fallback) | Clipboard-first |
| Selection capture | Accessibility (AX) APIs + sentinel-guarded clipboard fallback | Secure fields fail closed |
| Keyboard listening | rdev (git main branch) | Global key events; one background thread, three detectors |
| Local storage | rusqlite (SQLite) | Personal knowledge store + performance diagnostics |
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
    |-- cpal capture begins
    |-- immutable DictationContextSnapshot resolved (global settings + per-app profile)
    +-- spawn_model_preparation() warms the model *concurrently with speech*
    |
invoke('stop_native_recording') --> audio thread joins, samples resampled to 16kHz mono
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
2. `voice_commands` — typed replacements/snippets from the knowledge store
3. `smart_correction` — vocabulary matcher (exact tier + optional fuzzy tier)
4. `smart_formatting` — deterministic prose grammar, lists, same-utterance backtracking (live only)
5. `ide_context` — project symbol correction and `@file` canonicalization (live only, opt-in)
6. `cli_command` — spoken CLI command formatting

### Selected-text transform

```text
Transform hold key down (rdev, assigns a monotonic transform_pass_id)
    |
Mic arms immediately (before capture — Chromium capture can take >1s)
    |
selection.rs freezes an AX selection snapshot
    |-- secure/password field --> fail closed, no popover content
    +-- no AX selection --> AX retry ladder, then sentinel-guarded synthetic Cmd+C
    |
Popover shows 'listening' (non-focusable, never steals focus)
    |
Key release --> instruction ASR (cleanup-only transcript path)
    |
expand_instruction(): built-in preset > saved transform > raw text
    |
llm_sidecar: spawn + handshake (if cold) --> generate --> 'thinking' to 'ready'
    |
User reviews a word diff. Approve --> transform_apply (AX set-value, or paste
fallback with full-pasteboard save/restore). Undo restores the frozen original.
```

Dictation and transform are mutually exclusive in both directions (status guards, sidecar busy guard, helper shutdown before recording).

---

## Four-Window Architecture

Each window is a separate webview with its own Tauri capability set, following least privilege.

| Window | Label | Entry Point | Size | Purpose |
|--------|-------|-------------|------|---------|
| Main | `main` | `index.html` | 720×560 | Settings, recording controls, history, stats, onboarding, modals |
| Overlay | `overlay` | `overlay.html` | 260×100 | Dynamic Island notch widget. Always-on-top, transparent, non-activating |
| Log Viewer | `log-viewer` | `log-viewer.html` | 800×600 | Events, Performance/Runs, Transform diagnostics, Reports |
| Transform Review | `transform-review` | popover entry | 320×76 (compact) | Transform proposal review. Non-focusable until `ready`/`failed` |

Main and log-viewer hide on close instead of being destroyed. The overlay and transform popover both use the shared non-activating window treatment in `commands/native_window.rs`, and **all** of their raw `NSWindow` mutation is dispatched to the main thread via `run_on_main_thread` — macOS 26 hard-traps on off-main `NSWindow` mutation (#325).

Rust is the sole author of every overlay and popover pixel: `geometry_for()` and `popover_geometry_for()` are pure functions asserted by checked-in fixtures on both sides (cargo test + vitest). The frontend never hardcodes dimensions.

---

## Rust Backend (`app/src-tauri/src/`)

### Module map

| Module | Purpose |
|--------|---------|
| `lib.rs` | App wiring: module declarations, `State`, `MutexExt`, 105 registered commands, setup, tray, run loop |
| `alloc.rs` | Custom macOS malloc zone ("RustHeapZone") so Rust heap is accounted separately from whisper.cpp's FFI heap |
| `audio.rs` | cpal capture, mono mix, 16kHz resample, `audio-level` emission |
| `audio_decode.rs` | Decoding imported audio files for `transcribe_file` |
| `benchmark.rs` | Performance Lab: fixture corpus, scoring (raw/normalized/delivered WER), reports |
| `cleanup.rs` | Filler removal and capitalization |
| `cli_command.rs` | Spoken CLI command grammar and lexicon |
| `correct_and_teach.rs` | Bounded local diff proposals from a user's edit; never writes without confirmation |
| `correction.rs` | Smart Correction matcher (exact + fuzzy tiers) |
| `dictation_context.rs` | Immutable per-recording context snapshot resolution |
| `evaluation.rs` | Versioned fixture evaluation harness (`murmur-eval`) |
| `file_output.rs` | Numbered `.txt` / `.wav` output |
| `frontmost.rs` | Native frontmost-app query + running-application list |
| `ide_context.rs` | Memory-only bounded IDE symbol / root-relative file index |
| `injector.rs` | Clipboard write, CGEvent paste (osascript fallback), focused-field AX role checks |
| `keyboard.rs` | Hold-down, double-tap, and transform-hold detectors on one shared rdev thread |
| `knowledge_store/` | SQLite personal knowledge store: migrations, repository, backup/recovery |
| `llm_sidecar.rs` | Host supervisor for the signed local-LLM helper: spawn, handshake, RSS ceilings, idle unload, circuit breaker |
| `model_runtime.rs` | Model catalog + lifecycle manager (load/warm/readiness/unload, generation-ordered status events) |
| `performance_metrics/` | SQLite run history, stage timings, resource samples, retention |
| `platform/` | Platform abstraction seams (macOS / Linux) |
| `resource_monitor.rs` | CPU/RSS sampling, 1s heartbeat, idle-timeout enforcement |
| `selection.rs` | AX selection capture with secure-field fail-closed and clipboard fallback |
| `smart_formatting.rs` | Deterministic prose formatting and same-utterance backtracking |
| `state.rs` | `DictationStatus`, `TransformStatus`, `DictationState`, `AppState` |
| `telemetry.rs` | Structured event system: `TauriEmitterLayer`, ring buffer, JSONL, privacy stripping |
| `transcriber/` | `TranscriptionBackend` trait + whisper / parakeet / coreml implementations |
| `transcript_transform.rs` | The ordered post-recognition pipeline and its stage contracts |
| `transform_apply.rs` | Approve/undo write-back — the only path that writes into the target app |
| `transform_diagnostics.rs` | Per-attempt transform records and consented content captures |
| `transform_flow.rs` | End-to-end transform orchestrator and its Tauri commands |
| `transform_presets.rs` | Built-in spoken presets (Shorten, Bullets, Professional, Fix grammar, Casual) |
| `transform_trace.rs` | Pass-scoped correlation tracing |
| `vad.rs` | Silero VAD filtering, thread-local context cache |
| `vocab.rs`, `vocabulary_alias.rs` | Code-vocabulary scanning and explicit spoken aliases |
| `voice_commands.rs` | Typed voice command execution and variable expansion |

Commands live under `commands/` (`recording`, `permissions`, `keyboard`, `logging`, `models`, `knowledge`, `correct_and_teach`, `benchmark`, `performance`, `transform_model`, `transform_popover`, `transform_diagnostics`, `overlay`, `native_window`, `tray`).

### `state.rs` — Shared State

```rust
enum DictationStatus { Idle, Recording, Processing }
enum TransformStatus { /* Idle, Capturing, Listening, Thinking, ReviewPending, ... */ }

struct AppState {
    dictation: Mutex<DictationState>,
    recording_transition: tokio::sync::Mutex<()>,   // serializes start/stop/cancel/transform
    model_runtime: ModelRuntimeManager,
    recording_id: AtomicU64,                        // generation counter; supersedes stale work
    cancelled_id: AtomicU64,
    settings_revision: AtomicU64,
    correction_matcher: Mutex<...>,                 // immutable, swapped by generation
    ide_context: Mutex<IdeContextStore>,
    transform_status: Mutex<TransformStatus>,
    transform_pass_sequence: AtomicU64,             // monotonic per physical key hold
    transform_session: Mutex<Option<TransformSession>>,
    transform_apply_epoch: AtomicU64,               // guards clipboard restore races
    // ...
}
```

Two patterns do most of the concurrency work:

- **Generation counters.** `recording_id` and `transform_pass_id` are monotonic. Every async continuation re-checks whether it still owns the current generation before mutating shared state, so a delayed handler can never cancel or overwrite the pass that replaced it.
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

The manager serializes load/warm/readiness/unload, emits generation-ordered `model-runtime-status-changed` events, and reports a `LoadReport` (`lock_wait_ms`, `load_ms`, `cache_hit`) that feeds the performance record. Idle release is user-configurable (`idleTimeoutMinutes`).

**Warm-on-record:** `start_native_recording` kicks off `spawn_model_preparation()` immediately after audio capture starts, so model load overlaps with the user still speaking. By the time the key is released the model is usually resident. (The transform sidecar does **not** do this yet — see issue #340.)

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

All backends take one final-after-stop pass. The Whisper-only incremental/preview worker was removed in #279; delivery happens exactly once.

### `llm_sidecar.rs` — Local LLM Supervisor

The helper is a separate signed executable (`murmur-llm-sidecar`), packaged as a Tauri `externalBin`, running with hardened runtime and App Sandbox.

- **Model pin:** Qwen2.5-1.5B-Instruct Q4_K_M, ~1.1 GB, exact byte size **and** SHA-256 verified at download and again before every spawn.
- **Spawn hardening:** empty environment, fixed cwd, no path arguments, no network. The model is handed over as an inherited read-only file descriptor (fd 3), so the helper never resolves a path itself.
- **Handshake:** nonce + protocol + model-id verification within a deadline; a mismatch is a hard failure, not a downgrade.
- **Resource ceilings:** RSS warn at 2 GB, kill at 3 GB. Idle unload after inactivity. A circuit breaker (3 faults in a 10-minute window) disables the runtime until `reset_transform_runtime`.
- **Maintenance:** a 30-second tokio interval task runs `maintenance_tick()` for idle unload and RSS enforcement; `RunEvent::Exit` shuts the helper down so it never outlives the app.

### `keyboard.rs` — Keyboard Detection

One persistent rdev background thread feeds three detectors: hold-down, double-tap, and transform-hold.

- **Hold-down** — 2 states; press emits `hold-down-start`, release emits `hold-down-stop`. Combos (trigger+letter) cancel.
- **Double-tap** — 4 states; each tap under 200ms, gap under 400ms. Rejects holds, combos, slow taps, triple-tap spam. Emits `hotkey-tap-rejected` when an idle first tap expires (opt-in overlay feedback).
- **Both** — deferred hold promotion: a 200ms timer thread with an atomic invalidation counter decides tap-vs-hold. Releasing before 200ms invalidates the timer; still held at 200ms promotes to a hold.
- **Transform hold** — an independent key (`alt_r` / `ctrl_l` / `shift_r`, rejects the dictation key). Each physical hold gets a monotonic `transform_pass_id` assigned *in the rdev callback*, which is then carried explicitly through every event, command, and worker — correlation never relies on an ambient tracing span.

**macOS thread safety.** rdev's key translation uses TIS/TSM APIs that must run on the main thread. `rdev::set_is_main_thread(false)` is called before `listen()`, which makes rdev marshal only those calls to main via `dispatch_sync` while the listener loop stays on its background thread. Without it, the app silently segfaults.

Global modifier hotkeys recover when macOS disables the underlying event tap, and the hot path performs no main-thread key-name translation and ignores mouse movement.

### `injector.rs` — Text Injection

1. **Clipboard** (always): `arboard`. Empty/whitespace-only text is skipped.
2. **Auto-paste** (optional): waits the configured delay, then posts a native `CGEvent` Cmd+V key-down/key-up pair to `CGEventTapLocation::HID`. If the native path fails, it falls back to `osascript`. Retries once on failure; the whole operation is timeout-bounded. On failure, `auto-paste-failed` is emitted and the text stays on the clipboard.

Focused-field role checks use native AX with a per-element messaging timeout and an osascript fallback, so a hung app can't stall the pipeline.

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

Five streams (tracing targets): `pipeline`, `audio`, `keyboard`, `transform`, `system`.

**Privacy stripping.** In release builds, all string fields on `pipeline` events are removed from the data object; only numerics survive. `transform` events are stricter still and stripped in *all* builds: every string key **and** value must appear in an explicit stable vocabulary of enum values, stage names, error codes, and bucket labels. Anything else is dropped at the layer, independent of the call site — so a careless log statement cannot leak selected text, instructions, proposals, paths, or bundle IDs.

### Local storage

Two SQLite databases, both local-only:

| Store | File | Retention |
|-------|------|-----------|
| Personal knowledge | `knowledge/knowledge.sqlite3` | User-managed; versioned migrations, backups, quarantine on corruption |
| Performance diagnostics | `diagnostics/performance.sqlite3` | 200 completed runs, 600 resource samples, 8 follow-ups per run |

Transform diagnostic captures (explicitly consented, content-bearing) live under `diagnostics/transforms/transform-captures/` in a `0700` directory with `0600` files: max 3 retained, 7-day expiry, symlink targets refused, no export path.

---

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
- **The overlay reads settings from localStorage directly** (`useOverlaySettingsMirror`), not through React context or IPC. There is no shared context across windows.

---

## Tauri Commands

105 commands are registered in `lib.rs`. See [reference/commands.md](reference/commands.md) for the full signature-level list, grouped by module.

## Events

See [reference/events.md](reference/events.md) for every Rust → frontend event, its payload, and its listeners.

---

## macOS Permissions

| Permission | Required For |
|-----------|-------------|
| Microphone | Audio capture — dictation and transform instructions |
| Accessibility | Global hotkeys (rdev), auto-paste, AX selection capture, AX write-back |

Accessibility is checked via `AXIsProcessTrusted()`; the prompt is triggered with `AXIsProcessTrustedWithOptions()`. Microphone access can be requested in-app via `AVCaptureDevice.requestAccess`. Both have reset paths for stale TCC entries.

---

## Data Directories

Murmur uses two roots — a legacy one from before the rename, and the Tauri app-data root:

```text
~/Library/Application Support/local-dictation/
├── models/                    # whisper .bin files, Silero VAD, sherpa-onnx bundle
│   └── transform-llm/<sha256>/qwen2.5-1.5b-instruct-q4_k_m.gguf
└── logs/                      # app.log, events.jsonl (+ .dev variants)

~/Library/Application Support/com.localdictation/        (com.localdictation.dev in dev)
├── knowledge/                 # knowledge.sqlite3, backups/, quarantine/
└── diagnostics/               # performance.sqlite3, transforms/
```

Core ML models are managed by FluidAudio under `~/Library/Application Support/FluidAudio/Models/`.

---

## Build & Release

### Dev

```bash
python3 scripts/build_local_llm_sidecar.py   # once, before anything else on macOS
cd app && npm install
cd app && npm run tauri dev                  # hot reload
cd app && npm run tauri build                # .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1
cd app && npx tsc --noEmit && npm test
```

`tauri.macos.conf.json` declares the `murmur-llm-sidecar` externalBin, so on macOS both `tauri dev` and `cargo check`/`cargo test` fail on a fresh clone until that binary exists. The build script produces a real helper on arm64 macOS and is a no-op elsewhere. See [DEVELOPMENT.md](DEVELOPMENT.md).

`tauri.dev.conf.json` overrides only `identifier` → `com.localdictation.dev` and `productName` → `Local Dictation Dev`, so dev installs alongside the release build.

### Release pipeline

1. Push a `chore: bump version ...` commit to trusted `main`.
2. `Release Build` runs frontend verification, macOS signing/notarization (including the sidecar's split entitlements), and Linux packaging concurrently, with launch smoke tests.
3. Artifacts plus SHA-256 provenance are stored under names keyed by the exact commit SHA.
4. The completed-build event verifies the trusted push, version-bump message, matching versions, exact run ID, source SHA, and immutable artifacts.
5. Promotion creates `vX.Y.Z`, verifies remote `.sig` files, generates `latest-v2.json` plus the legacy-safe `latest.json`, then publishes.

Release finalization **fails closed** on any unexpected executable in the bundle: only the Murmur binary and the signed sidecar may ship (#324). Manual dispatches are rehearsal-only. See [release.md](release.md).

### Release profile

```toml
[profile.release]
panic = "abort"
codegen-units = 1
lto = true
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
| `murmur-llm-sidecar` process | On demand | Out-of-process llama.cpp; killed on idle, fault, or app exit |
| Sidecar reader thread | Per spawn | Reads the helper's protocol stream |
| Audio capture | Per recording | cpal stream + mono mix |
| Hold-promotion timer | Per key press (both mode) | 200ms sleep, atomic validity check |
| Model preparation | Per recording start / model change | Warms the backend concurrently with speech |

Plus tokio `spawn_blocking` for VAD (its context is `!Send`), downloads, and injection dispatch.

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Out-of-process LLM sidecar | llama.cpp's ggml ABI clashes with whisper's; isolation also gives per-process RSS ceilings, a kill switch, and a smaller signed attack surface |
| Model handed to the helper as fd 3 | The helper takes no paths and needs no filesystem reach; pinning is enforced host-side by size + SHA-256 |
| One model catalog, fail-closed | Unknown identifiers error instead of silently substituting a model with different accuracy/latency |
| Warm-on-record | Model load overlaps with speech instead of being charged to the user after key release |
| Generation counters everywhere | A delayed continuation can observe that it lost ownership rather than corrupting the pass that replaced it |
| Immutable per-recording context | Settings or focus changes mid-utterance can't reinterpret a recording already in flight |
| Review-first transform | The LLM never writes into another app without explicit approval; Undo restores the frozen original |
| Secure-field fail-closed | An *errored* AX secure-field check is treated as "possibly a password field" — no read, no clipboard fallback |
| Transform telemetry allow-list | String keys and values must be in a stable vocabulary; leakage is prevented at the layer, not at the call site |
| Ordered transcript pipeline | One entry point, declared stage order and failure policy, per-stage timing — instead of ad-hoc string munging per backend |
| Rust owns all window geometry | Pure `geometry_for()` / `popover_geometry_for()` with fixtures on both sides; no drifting CSS pixel constants |
| Main-thread `NSWindow` mutation | macOS 26 hard-traps on off-main mutation (#325) |
| CGEvent paste with osascript fallback | Native path avoids process spawn; osascript remains the compatibility net |
| Custom malloc zone | Separates Rust heap from whisper.cpp's FFI heap; `GlobalAlloc` wrappers drift when FFI frees Rust allocations |
| `set_is_main_thread(false)` | Prevents the rdev TIS/TSM segfault on the background listener thread |
| `MutexExt::lock_or_recover()` | Survives a panic while a lock is held; no stuck UI state |
| `IdleGuard` RAII | Guarantees status reset on every error path in the pipeline |
| Clipboard-first delivery | Reliable across all apps; auto-paste is layered on top and never the only path |
| Per-window least privilege | Overlay and transform popover get minimal capabilities; only the main window gets the full set |
