# GPUI 1.1: core dependency map

Part of #985. Stack 02, based on `gpui/01-ownership-matrix`; do not merge.
The [Phase 0 decision](../decisions/2026-11-05-gpui-phase-0-exit.md)
overrides the plan's intermediate-shipping route. This is a foundation, not
completion of the [Phase 1 exit gates](gpui-feature-matrix.md#phase-1-exit-gates).

## Before extraction

Source audit of `app/src-tauri/src` at stack 01 (`crate::` references,
`tauri::` uses, command attributes, and module declarations). “No direct
Tauri” does not prove a module can be compiled separately. Tests and feature-
gated branches count too. All these modules currently compile inside `ui`.

| Module / cluster | Dependency path and boundary |
|---|---|
| `model_artifact`, `model_digest` | Already Tauri-free leaves: std I/O and SHA-256. Contrary to the plan's warning, `model_digest` itself has no Tauri import at this revision; `smart_turn → model_digest` is an extraction dependency, not a hidden Tauri use. |
| `audio::compute_rms`, `audio::compute_peak` | Pure math embedded in a Tauri module. Move without capture policy. |
| `resource_monitor::get_process_rss_*` | Leaf `memory-stats` queries embedded in a Tauri command/heartbeat module. |
| `alloc` readers | macOS malloc-zone statistics; zone creation and GlobalAlloc installation belong to the executable. Readers must receive the executable's exact zone, never create a second zone. |
| `cleanup`, `cli_command`, `meeting_artifact` | Tauri-free algorithms today; defer until their owning service moves. |
| `transcriber/{whisper,coreml,parakeet}` | No direct Tauri, but `→ resource_monitor → model_runtime / AppHandle::state / Tauri command`. Whisper also uses `model_artifact`. Native inference libraries remain platform dependencies after breaking the Tauri edge. |
| `vad` | `→ audio` for RMS/peak (including diagnostic/test paths), `→ model_artifact`, and preview-feature/test `→ transcriber → resource_monitor`. Local model paths currently use `dirs`. |
| `smart_turn` | `→ model_digest`; ONNX/FFT native inference, global model cache and `dirs` paths remain. Its command wrapper owns download locks and recording-generation checks. |
| `audio` | Direct `Emitter`, `Manager`, `AppHandle`; managed State, window lookups, capture graph/hang diagnostics, inventory invalidation, keyboard admission and preview callbacks. `audio_prepare` also belongs to this host-dependent cluster. |
| `audio_lifecycle` | Direct AppHandle; `→ audio`, permission command, keyboard, inventory; lifecycle callbacks route into recording, preview, corpus and startup-diagnostic commands. Replacing emits alone is insufficient. |
| `meeting_capture` | Direct `Emitter`/`Manager`, managed State and worker ownership; `→ audio`, inventory, model_runtime, meeting_diarization, suggestions, stores, transcriber and VAD. Lifecycle callbacks and generation checks must stay together. |
| `state`, `dictation_context` | State has no direct Tauri import, but `→ model_runtime / commands / microphone_preview`; therefore not a core-state type yet. Extract narrow typed service state, not the entire State graph. |
| `model_runtime`, `microphone_preview` | Direct Tauri and command callbacks, recording admission, model/cache and capture ownership. |
| `performance_metrics`, `resource_monitor` | Direct host commands/state, idle release, persistence and heartbeat events; only raw memory readers move now. CPU sampler/schema remain with their service. |
| `knowledge_store`, `meeting_store` | No direct Tauri in repository code, but follow helpers: `meeting_store → meeting_audio / meeting_review` and `knowledge_store → voice_commands`; split persistence DTOs and repository dependencies before declaring these clusters independent. |
| `commands/*`, `lib`, telemetry, windows, keyboard, injector | Host boundary: IPC authorization, managed services, targeted-event migration, native window/main-thread work, rdev, secure-field proof and delivery. Keep wrappers until the corresponding service is extracted. |

## Extraction order and remaining work

1. **1.1:** move math, raw RSS/heap readers and model checks to
   `crates/murmur-core`. App re-exports preserve current callers. Point VAD
   math and transcriber RSS directly at the core to cut the accidental edges.
2. **1.1:** pass an explicit `AppContext` to core services. Prove paths and
   blocking work with the existing VAD installer; keep its download, cancellation,
   lock and atomic publication policy unchanged. Provide read-only permission,
   async-work and main-thread adapters for subsequent services.
3. **1.2:** extract typed recording-controller state and hotkey/auto-stop/Paste
   Last/launch gates. Split capture callbacks and events from worker transport;
   inject capture, permission and delivery capabilities. Preserve immutable start
   configuration, generation rechecks and stop-only-own-generation semantics.
4. **1.3:** completion de-duplication, History, statistics, reports and v1
   migration together. Add serialized durable journal/holding-file services;
   preserve retention, clear/reset, crash recovery and exactly-once publication.
   Do not move individual transcript transforms outside their ordered entry point.
5. **1.4:** introduce typed named event consumers, then remove broadcast emits
   and the remaining monitor/capture event edges. AppContext is not an event bus.
6. **1.5:** Rust schemas, settings and appearance ownership, migration v2 and
   the standalone WebKit importer; explicit independent data and model roots.
7. **1.6:** host-neutral updater, v3 migration, existing feed/key and canaries;
   inject relaunch only after shutdown has drained. No updater/quit work moves now.

Inference wrappers, CPU sampling, database services, native capture and text
injection remain in the app crate. Linux builds of the small core prove this
boundary only; they do not prove that macOS inference/capture is portable.

## AppContext and runtime decision

The **host owns Tokio**. Today this is Tauri's existing async runtime; the
future GPUI executable will create and retain one Tokio runtime until shutdown
has drained services. The core creates no runtime and never calls `block_on`.
This avoids nested runtimes, early runtime drop, and blocking AppKit on inference
or teardown. Native capture threads and rdev retain their current owners.

`AppContext` contains explicit shared adapters for paths, read-only microphone
permission status, background async/blocking work, and main-thread dispatch.
Dispatch always queues, never runs inline (including main-thread callers), and
executes tasks in FIFO enqueue order across callers. A single host FIFO drains
on macOS's main `NSOperationQueue`, serviced by tao/Tauri's AppKit run loop.
Callers may dispatch while holding state locks; tasks run after returning to
the main run loop. Other hosts currently report scheduling failure.
There is no type-erased managed-state lookup: future services own typed state
and receive it alongside the context. Only the Tauri boundary retrieves the
context from managed state. The VAD installer resolves its unchanged legacy
model root through the adapter, and awaits core artifact validation on a host
blocking worker. Scheduling errors and worker disappearance propagate to callers.
Scheduling is not task completion; services retain responsibility for generation
checks, cancellation, durable publication and shutdown drains. No new global
runtime or singleton service is introduced.

The executable retains `#[global_allocator]`, the zone singleton, and the
FoundationModels Swift `build.rs`. Core heap readers receive the live Rust zone
for statistics; the binary forwards existing query functions. All NSWindow
mutations still require main-thread dispatch; rdev still calls
`set_is_main_thread(false)` before listening. No content logging is added.

## Verification contract

- Linux: `cd app/src-tauri && cargo test -p murmur-core`.
- CI: inspect `cargo tree -p murmur-core --target all --all-features --edges all`
  and reject every Tauri package, including dev/build and target-only edges.
  Fail on cargo errors rather than treating failed resolution as absence.
- macOS: helper build, formatting, clippy and workspace tests; best-effort
  isolated development-app startup with no synthetic input. Exact results belong
  in the stack PR, including any missing native verification.
