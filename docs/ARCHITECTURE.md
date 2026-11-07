# Architecture

Murmur is a native GPUI macOS application. Rust owns transcription, capture,
persistence, permissions, delivery and shutdown. There is no React runtime,
JavaScript bridge, Tauri runtime or UI webview.

## Stack

- `app/Cargo.toml` / `app/Cargo.lock`: native workspace and pinned dependencies.
- `app/gpui`: native views, accessible controls, Main, Settings, Diagnostics,
  the nonactivating overlay and Live Words preview, menus and status item.
- `app/crates/murmur-services`: shared recording, capture, model, meeting,
  text delivery, native service facade and lifecycle owners. Its normal,
  optional, target, build and dev dependencies contain no UI host runtime.
- `app/crates/murmur-core`: typed reducers, context ports, ordered event protocol,
  completion actor, source schemas, recovery and updater.
- `app/crates/capture-helper-protocol`: versioned capture transport.
- `app/crates/murmur-webkit`: permanent standalone legacy importer and store
  cleanup, using public objc2 WebKit APIs. It is not linked into the UI or
  service crates. The native host uses a disposable exact-child importer.
- `app/sidecars/capture`: isolated helper/worker workspace; its CPAL graph
  cannot share a Cargo resolution with GPUI's optional audio version.
- `app/sidecars/capture-agent`: Swift recovery agent.
- `app/macos/bundle.json`: release identity/version metadata. Permission text,
  signing entitlements, helper metadata and icon files live in `app/macos`.

The executable remains `ui`; production bundle identity remains
`com.localdictation`. The UI package and services share the release version.
The GPUI source pin and maintained patches remain under `app/gpui/vendor`.

## Data flow

Keyboard admission → immutable recording configuration and generation →
isolated capture worker → local ASR → ordered transcript pipeline →
identity-aware delivery → exactly-once completion publication.

The transform entry point preserves cleanup, voice commands, smart correction,
smart formatting, spoken structure/numbers, IDE context and CLI formatting order.
Secure fields and unprovable delivery targets are refused. Stale asynchronous
continuations cannot mutate a new recording or stop a generation they do not own.

GPUI calls the [native service facade](reference/commands.md) in process.
Native sender identity still restricts Main-only operations. Companion views
cannot claim Main identity. Binary retained-audio access is bounded separately.
The named event router targets registered consumers and coalesces waveform data;
queued waveform delivery rechecks its capture generation immediately before the
transport/callback under its delivery permit. No broadcast bus is introduced.

## Launch preparation

Startup reads durable state on background workers, preserves recovery evidence,
and keeps main dispatch responsive. Model warmup follows the accepted recording
configuration; no recording is admitted before migration and setup gates resolve.

## Process and thread ownership

The GPUI executable owns the AppKit run loop and one retained Tokio runtime.
Worker dispatch (legacy migration, diarization, Core ML installer, capture agent
and helper) happens before NSApplication or Metal initialization.

All NSWindow mutations queue onto the main thread; native service windows are
ports, not owned webviews. Blocking inference, storage and shutdown work run
outside the main thread. The FIFO main dispatcher continues during deferred
termination. rdev starts once, after `set_is_main_thread(false)`.

Capture remains in separately signed, bounded, killable helper processes. The UI
and service libraries never link CPAL/HAL capture ownership. Meeting playback is
a separate revocable native worker. Generation and revision checks apply to late
capture, playback, delivery, export and presentation effects.

## Persistence and privacy

Production paths remain `~/Library/Application Support/com.localdictation` and
the established model/export roots. Explicit dev/QA identifiers use isolated
namespaces and private pasteboards. Validate identity and paths before opening a
store. No transcript, audio, clipboard or selection content enters logs.

The completion actor journals accepted publications and serializes History,
statistics, saved reports, retention, Clear/Delete and pending replay. Recording
publication and statistics writes are exactly once. `retainHistory=false`, Clear
History and export exclusions apply at the core boundary, including recovery
copies and legacy imports. Cleared destinations are explicit disk values and
cannot be resurrected by an old WebKit copy.

Source writes use the existing FIFO, source lock, generations and per-field
revisions. Atomic publications use private temporary siblings, full file sync,
rename and parent-directory sync. Missing required destinations, future schemas,
corrupt pending publications and retained recovery evidence continue to block
unsafe defaults. Explicit recovery preserves rejected bytes.

## Legacy import and retirement

Before showing user data the host reads the durable lifecycle receipt. A retired
store returns immediately before store inspection, child spawn or WebKit loading.
If a legacy store remains, the disposable worker serves only a fixed page at the
original `tauri://localhost/` origin. The parent owns durable fill-only v1/v2/v3
imports and exact-key removal acknowledgements.

After every destination/version/removal receipt is durable, the parent commits
retirement prepare, then permits the worker to release the view, purge all
WKWebsiteDataStore types, and fetch/verify zero records. Only after successful
exact-worker exit does the parent remove the exclusive legacy directory, sync
its parent and commit `retired`. Failures leave retirement incomplete and retry
without reimporting committed data. Symlinked retirement directories are refused.
A fresh profile with no store commits retirement without starting WebKit.

See [the lifecycle contract](draft/gpui-webkit-lifecycle.md) and
[Phase 4 evidence](draft/gpui-phase4-removal.md).

## Coordinated shutdown

Menu Quit, Cmd-Q, Dock/system termination, final-window destruction, signals and
relaunch converge on one coordinator. Admission closes permanently. Original
capture generations drain by +5 seconds, helper receipts by +7 and durable writer
barriers by +10. A single terminal action follows. Missing helper/write receipts
suppress restart. AppKit defers termination while main dispatch remains live;
GPUI's 200 ms quit window performs final nonblocking cleanup only.

## Build and release

See [Development](DEVELOPMENT.md) for checked commands and [Release](release.md)
for the trusted isolated build/sign lanes. Bundling preserves executable/helper
allowlists, identity, entitlements, FoundationModels weak linkage, Swift rpath
and macOS 14 minimum. The signed upgrade-chain rehearsal and final memory-budget
measurement require their separate authorized acceptance runs.
