# Research: Structured Event System (#111–#114)

Research into Tauri multi-window state sharing, event system scope, custom `tracing::Layer` implementation, and existing log call site migration surface.

## Table of Contents

- [1. Tauri Multi-Window State Sharing](#1-tauri-multi-window-state-sharing)
- [2. Tauri Event System Scope](#2-tauri-event-system-scope)
- [3. Custom tracing::Layer for Tauri](#3-custom-tracinglayer-for-tauri)
- [4. Existing Log Call Site Catalog](#4-existing-log-call-site-catalog)
- [5. Risks and Design Implications](#5-risks-and-design-implications)

---

## 1. Tauri Multi-Window State Sharing

### Current State Architecture

The app manages exactly one type via `app.manage()` — the `State` struct in `lib.rs:31-35`:

```rust
pub(crate) struct State {
    pub(crate) app_state: AppState,       // Mutex<DictationState> + Mutex<Box<dyn TranscriptionBackend>>
    pub(crate) notch_info: Mutex<Option<(f64, f64)>>,
}
```

Registered at `lib.rs:48-51` on the builder. Commands access it via `tauri::State<'_, State>` dependency injection, or via `app_handle.state::<State>()` in non-command contexts.

### Can a Second Window Access the Same State?

**Yes.** `app.manage()` internally wraps the value in `Arc<T>`. All windows share the same Rust process and same managed state. Any window's frontend calling `invoke("some_command")` gets the same `State` instance. No extra wiring needed.

### Existing Multi-Window Setup

Murmur already has two windows in `tauri.conf.json`: `main` and `overlay`. Each has its own capability file (`capabilities/default.json` for main, `capabilities/overlay.json` for overlay). Both can receive the same events.

### Gotchas

1. **Capabilities are per-window.** A new `log-viewer` window needs its own capability file listing `"windows": ["log-viewer"]` with the permissions it needs (at minimum `core:default` for IPC, plus any command-specific permissions).
2. **No automatic frontend state sync.** Each window has independent JS context. Use `app_handle.emit()` to broadcast state changes.
3. **Dynamic window creation** requires `"core:webview:allow-create-webview-window"` in the creating window's capability. Alternative: define the window statically in `tauri.conf.json` with `"visible": false` and show/hide it.
4. **Vite multi-page build** — If the log viewer is a separate `.html` entrypoint, add it to `vite.config.ts` `rollupOptions.input`.

### WebviewWindow API (Frontend)

```typescript
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';

// Dynamic creation
const logViewer = new WebviewWindow('log-viewer', {
  url: '/log-viewer.html', title: 'Log Viewer', width: 800, height: 600
});

// Or get existing
const existing = await WebviewWindow.getByLabel('log-viewer');
await existing.show();
await existing.close();
```

---

## 2. Tauri Event System Scope

### `app_handle.emit()` Broadcasts to ALL Windows

**Confirmed.** `emit()` sends to every open webview window and every Rust-side listener. Murmur already relies on this — both `main` and `overlay` listen to `recording-status-changed`.

### Targeting Specific Windows

| Method | Scope |
|--------|-------|
| `app_handle.emit(event, payload)` | All targets |
| `app_handle.emit_to("label", event, payload)` | One specific window |
| `app_handle.emit_filter(event, payload, \|target\| ...)` | Custom subset |

For the log viewer: `app_handle.emit()` is ideal since the main window and log viewer window both benefit from receiving events. If you want to avoid log events hitting the main window, use `emit_to("log-viewer", ...)` instead.

### Window Lifecycle Events

Rust side has `WindowEvent::Destroyed`, `WindowEvent::Focused(bool)`, etc. via `.on_window_event()`. JS side has `tauri://window-created`, `tauri://destroyed`, etc. via `listen()`. This means the backend can detect when the log viewer closes (to stop unnecessary emission overhead if desired).

### Current Event Usage

The app has 12 custom events. All use global `emit()`, no `emit_to()` or `emit_filter()` anywhere.

| Event name | Emitted from (Rust) | Listened in (TS) |
|------------|---------------------|-------------------|
| `recording-status-changed` | `commands/recording.rs`, `commands/models.rs` | `useRecordingState.ts`, `OverlayWidget.tsx` |
| `audio-level` | `audio.rs` | `useRecordingState.ts`, `OverlayWidget.tsx` |
| `transcription-complete` | `commands/recording.rs` | `useRecordingState.ts` |
| `auto-paste-failed` | `commands/recording.rs` | `useRecordingState.ts` |
| `double-tap-toggle` | `keyboard.rs` | `useDoubleTapToggle.ts`, `useCombinedToggle.ts` |
| `hold-down-start` | `keyboard.rs` | `useHoldDownToggle.ts`, `useCombinedToggle.ts` |
| `hold-down-stop` | `keyboard.rs`, `commands/keyboard.rs` | `useHoldDownToggle.ts`, `useCombinedToggle.ts` |
| `hold-down-cancel` | `keyboard.rs` | `useCombinedToggle.ts` |
| `keyboard-listener-error` | `keyboard.rs` | `useDoubleTapToggle.ts`, `useHoldDownToggle.ts`, `useCombinedToggle.ts` |
| `download-progress` | `commands/models.rs` | `ModelDownloader.tsx`, `SettingsPanel.tsx` |
| `notch-info-changed` | `commands/overlay.rs` | `OverlayWidget.tsx` |
| `show-about` | (tray menu) | `useShowAboutListener.ts` |

---

## 3. Custom `tracing::Layer` for Tauri

### Dependencies Needed

The project has no direct `tracing` dependency currently (only transitive via reqwest/tokio/cpal). Add to `Cargo.toml`:

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["registry"] }
```

### Layer Architecture

The `Layer<S>` trait requires implementing `on_event(&self, event, ctx)`. The layer takes `&self`, so mutable state needs interior mutability (`Mutex`). Key design:

```rust
pub struct TauriEmitterLayer {
    app_handle: AppHandle,                     // Send + Sync + Clone in Tauri v2
    recent: Mutex<VecDeque<LogEvent>>,         // ring buffer for hydration
    max_recent: usize,
}
```

### Field Extraction via Visitor Pattern

Tracing events don't store field values permanently. You implement `tracing::field::Visit` and pass it to `event.record()`. The `"message"` field from `info!("hello")` arrives through `record_debug` or `record_str` as a field named `"message"`. Metadata (level, target, module_path, file, line) is available via `event.metadata()`.

### Skeleton Implementation

```rust
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

#[derive(Debug, Clone, Serialize)]
struct LogEvent {
    timestamp: String,                           // ISO 8601
    level: String,                               // "TRACE", "DEBUG", "INFO", "WARN", "ERROR"
    target: String,                              // e.g., "murmur::audio"
    message: String,                             // the formatted message field
    fields: BTreeMap<String, serde_json::Value>, // all structured key-value fields
}

pub struct TauriEmitterLayer {
    app_handle: AppHandle,
    recent: Mutex<VecDeque<LogEvent>>,
    max_recent: usize,
}

impl TauriEmitterLayer {
    pub fn new(app_handle: AppHandle, max_recent: usize) -> Self {
        Self {
            app_handle,
            recent: Mutex::new(VecDeque::with_capacity(max_recent)),
            max_recent,
        }
    }
}

// -- Visitor for field extraction --

struct FieldVisitor<'a>(&'a mut BTreeMap<String, serde_json::Value>);

impl<'a> Visit for FieldVisitor<'a> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), serde_json::json!(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_string(), serde_json::json!(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), serde_json::json!(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_string(), serde_json::json!(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), serde_json::json!(value));
    }
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.0.insert(field.name().to_string(), serde_json::json!(value.to_string()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_string(), serde_json::json!(format!("{:?}", value)));
    }
}

// -- Layer implementation --

impl<S> Layer<S> for TauriEmitterLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();

        let mut fields = BTreeMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);

        let message = fields
            .remove("message")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();

        let log_event = LogEvent {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: format!("{}", metadata.level()),
            target: metadata.target().to_string(),
            message,
            fields,
        };

        // Buffer in ring buffer (lock briefly, emit outside)
        {
            let mut recent = self.recent.lock().unwrap_or_else(|p| p.into_inner());
            if recent.len() >= self.max_recent {
                recent.pop_front();
            }
            recent.push_back(log_event.clone());
        }

        let _ = self.app_handle.emit("log://event", &log_event);
    }
}
```

### Integration Point

Register in the Tauri `setup` hook (first place you have an `AppHandle`):

```rust
use tracing_subscriber::prelude::*;

// In run(), inside .setup(|app| { ... }):
let emitter_layer = TauriEmitterLayer::new(app.handle().clone(), 500);

tracing_subscriber::registry()
    .with(emitter_layer)
    .init();  // sets global default — can only be called once per process
```

**Risk:** If any dependency already calls `set_global_default`, this will fail. Tauri itself doesn't do this by default. Verify no other dependency sets it.

### Performance Considerations

- **Filter early**: Use `enabled()` (per-event, metadata-only) to reject low-priority events before constructing the `Event`.
- **Lock contention**: Keep mutex critical section to push/pop only; do `emit()` outside the lock.
- **Tauri emit overhead**: `emit()` evaluates JavaScript directly — not designed for high-throughput. Consider batching or using Tauri **Channels** for high-volume streaming.
- **No existing precedent**: `tauri-plugin-log` uses the `log` crate, not `tracing::Layer`. There's an [open PR](https://github.com/tauri-apps/plugins-workspace/issues/2516) for tracing support but it hasn't merged.

---

## 4. Existing Log Call Site Catalog

**72 total call sites** across 9 files. No `tracing`/`log` crate usage, no `println!`/`eprintln!`. Logging is entirely through the custom `log_info!`/`log_warn!`/`log_error!` macros plus one `logging::log_transcription()` call.

### Summary by Stream

| Stream | `log_info!` | `log_warn!` | `log_error!` | Other | Total |
|--------|-------------|-------------|--------------|-------|-------|
| `pipeline` | 15 | 5 | 2 | 1 (`log_transcription`) | 23 |
| `audio` | 6 | 6 | 3 | 0 | 15 |
| `system` | 8 | 8 | 0 | 0 | 16 |
| `keyboard` | 9 | 0 | 2 | 0 | 11 |
| `model` | 4 | 3 | 0 | 0 | 7 |
| **Total** | **42** | **22** | **7** | **1** | **72** |

### `lib.rs` (6 sites → `system`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 25 | WARN | `log_warn!("Mutex was poisoned, recovering data")` | system |
| 85 | INFO | `log_info!("window hidden on close request")` | system |
| 89 | INFO | `log_info!("app setup — Murmur v{}", env!("CARGO_PKG_VERSION"))` | system |
| 102 | INFO | `log_info!("setup: overlay window found, enabling cursor events")` | system |
| 106 | WARN | `log_warn!("Failed to set overlay cursor events: {}", e)` | system |
| 109 | WARN | `log_warn!("setup: overlay window NOT found")` | system |

### `keyboard.rs` (7 sites → `keyboard`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 477 | INFO | `log_info!("keyboard: rdev listener thread started")` | keyboard |
| 575 | INFO | `log_info!("keyboard: BOTH -> timer promoted to hold-down-start")` | keyboard |
| 589 | INFO | `log_info!("keyboard: BOTH -> emit hold-down-stop (promoted hold)")` | keyboard |
| 593 | INFO | `log_info!("keyboard: BOTH -> emit double-tap-toggle")` | keyboard |
| 600 | INFO | `log_info!("keyboard: BOTH -> emit double-tap-toggle (hold=None)")` | keyboard |
| 610 | ERROR | `log_error!("keyboard: rdev listener error: {:?}", e)` | keyboard |
| 622 | INFO | `log_info!("keyboard: listener heartbeat — active")` | keyboard |

### `audio.rs` (13 sites → `audio`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 123 | WARN | `log_warn!("start_recording: recording state mutex was poisoned, recovering")` | audio |
| 132 | WARN | `log_warn!("start_recording: recording state mutex was poisoned, recovering")` | audio |
| 140 | INFO | `log_info!("start_recording: created fresh sample buffer")` | audio |
| 148 | ERROR | `log_error!("Audio capture error: {}", e)` | audio |
| 194 | WARN | `log_warn!("Requested device '{}' not found, falling back to default", name)` | audio |
| 201 | WARN | `log_warn!("Failed to enumerate devices: {}, falling back to default", e)` | audio |
| 220 | INFO | `log_info!("run_audio_capture: device='{}', sample_rate={}, channels={}, format={:?}", ...)` | audio |
| 223 | ERROR | `log_error!("Audio stream error: {}", err)` | audio |
| 251 | WARN | `log_warn!("stop_recording: recording state mutex was poisoned, recovering")` | audio |
| 273 | WARN | `log_warn!("stop_recording: samples mutex was poisoned, recovering")` | audio |
| 279 | INFO | `log_info!("stop_recording: raw_samples={}, sample_rate={}, wall_secs={:.1}, audio_secs={:.1}", ...)` | audio |
| 282 | INFO | `log_info!("stop_recording: raw_samples={}, sample_rate={}, duration_secs={:.1} (no timestamp)", ...)` | audio |
| 288 | INFO | `log_info!("stop_recording: no buffer (was not recording)")` | audio |

### `injector.rs` (7 sites → `pipeline` / `system`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 10 | INFO | `log_info!("inject_text called with auto_paste={}, delay_ms={}, text_len={}", ...)` | pipeline |
| 14 | INFO | `log_info!("inject_text: text is empty, skipping")` | pipeline |
| 24 | INFO | `log_info!("inject_text: text copied to clipboard")` | pipeline |
| 34 | WARN | `log_warn!("inject_text: accessibility permission not granted — text in clipboard only")` | system |
| 45 | WARN | `log_warn!("inject_text: first paste attempt failed: {}, retrying in 100ms", first_err)` | pipeline |
| 58 | INFO | `log_info!("simulate_paste: using osascript to simulate Cmd+V")` | pipeline |
| 67 | INFO | `log_info!("simulate_paste: completed successfully")` | pipeline |

### `commands/recording.rs` (26 sites → `pipeline` / `audio` / `system` / `model`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 54 | INFO | `log_info!("pipeline: audio rms={:.4} peak={:.4} (device={})", ...)` | audio |
| 73 | INFO | `log_info!("pipeline: VAD detected no speech ... skipping transcription", ...)` | audio |
| 78 | INFO | `log_info!("pipeline: VAD trimmed {} → {} samples ...", ...)` | audio |
| 85 | WARN | `log_warn!("pipeline: VAD failed ({}), proceeding without filtering", e)` | audio |
| 95 | WARN | `log_warn!("pipeline: VAD model download failed ({}), skipping VAD", e)` | model |
| 113 | INFO | `log_info!("pipeline: transcription ({} samples): {:?}", ...)` | pipeline |
| 114 | — | `crate::logging::log_transcription(&model_name, &backend_name, audio_secs, transcribe_secs, &text)` | pipeline |
| 129 | ERROR | `log_error!("Text injection failed: {}", e)` | pipeline |
| 133 | WARN | `log_warn!("Text injection sender dropped")` | pipeline |
| 137 | WARN | `log_warn!("Text injection timed out")` | pipeline |
| 143 | INFO | `log_info!("pipeline: inject (clipboard + paste): {:?}", ...)` | pipeline |
| 151 | INFO | `log_info!("init_dictation")` | system |
| 185 | INFO | `log_info!("pipeline: audio parse (base64 + WAV): {:?}", ...)` | pipeline |
| 217 | INFO | `log_info!("configure_dictation: {}", options)` | system |
| 263 | INFO | `log_info!("Switched transcription backend to {}", backend.name())` | pipeline |
| 285 | WARN | `log_warn!("start_native_recording: already recording")` | pipeline |
| 292 | WARN | `log_warn!("start_native_recording: currently processing")` | pipeline |
| 304 | INFO | `log_info!("start_native_recording: device={}", ...)` | pipeline |
| 306 | ERROR | `log_error!("start_native_recording: audio failed: {}", e)` | audio |
| 312 | INFO | `log_info!("start_native_recording: started")` | pipeline |
| 334 | WARN | `log_warn!("stop_native_recording: not recording")` | pipeline |
| 346 | INFO | `log_info!("stop_native_recording: stopping")` | pipeline |
| 356 | ERROR | `log_error!("stop_native_recording: stop_recording failed: {}", e)` | audio |
| 360 | INFO | `log_info!("pipeline: audio teardown + resample: {:?}", ...)` | audio |
| 363 | INFO | `log_info!("stop_native_recording: no audio captured")` | pipeline |
| 378 | INFO | `log_info!("stop_native_recording: recording too short ({}ms), discarding", ...)` | pipeline |
| 395 | ERROR | `log_error!("stop_native_recording: pipeline failed: {}", e)` | pipeline |
| 402 | INFO | `log_info!("pipeline: total end-to-end: {:?} ...", ...)` | pipeline |
| 438 | INFO | `log_info!("cancel_native_recording: speculative recording discarded")` | pipeline |

### `commands/keyboard.rs` (5 sites → `keyboard`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 9 | ERROR | `log_error!("Invalid keyboard listener mode: {}", mode)` | keyboard |
| 16 | INFO | `log_info!("Keyboard listener started: mode={}, key={}, accessibility={}", ...)` | keyboard |
| 23 | INFO | `log_info!("Keyboard listener stopped: accessibility={}", ...)` | keyboard |
| 31 | INFO | `log_info!("Keyboard key changed while held — emitted stop; updated to: {}", hotkey)` | keyboard |
| 33 | INFO | `log_info!("Keyboard key updated to: {}", hotkey)` | keyboard |

### `commands/models.rs` (7 sites → `model`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 74 | WARN | `log_warn!("Failed to finalize VAD model download: {}", e)` | model |
| 76 | INFO | `log_info!("VAD model co-downloaded: {} ({} bytes)", ...)` | model |
| 80 | WARN | `log_warn!("VAD model co-download failed (non-fatal): {}", e)` | model |
| 111 | INFO | `log_info!("Model downloaded: {} ({} bytes)", ...)` | model |
| 155 | INFO | `log_info!("Moonshine model downloaded and extracted: {} ({} bytes)", ...)` | model |
| 176 | INFO | `log_info!("VAD model not found, downloading...")` | model |
| 190 | INFO | `log_info!("VAD model downloaded: {} ({} bytes)", ...)` | model |

### `commands/overlay.rs` (10 sites → `system`)

| Line | Level | Call | Stream |
|------|-------|------|--------|
| 35 | INFO | `log_info!("detect_notch_info: notch_w={}, menu_bar_h={}, screen_w={}", ...)` | system |
| 51 | INFO | `log_info!("screen parameters changed — re-detecting notch info")` | system |
| 105 | WARN | `log_warn!("_setPreventsActivation: not available on this macOS version")` | system |
| 119 | INFO | `log_info!("position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", ...)` | system |
| 123 | WARN | `log_warn!("position_overlay_default: set_size({}, {}) failed: {}", ...)` | system |
| 133 | INFO | `log_info!("position_overlay_default: x={}, y=0, sf={}", ...)` | system |
| 135 | WARN | `log_warn!("position_overlay_default: set_position({}, 0) failed: {}", ...)` | system |
| 138 | WARN | `log_warn!("position_overlay_default: no current monitor, falling back to (100, 100)")` | system |
| 162 | WARN | `log_warn!("show_overlay: overlay window not found — skipping")` | system |
| 174 | WARN | `log_warn!("hide_overlay: overlay window not found — skipping")` | system |

### Files With No Logging

- `transcriber/whisper.rs`, `transcriber/moonshine.rs`, `transcriber/mod.rs` — model loading and inference are silent
- `vad.rs` — VAD internal operations are silent
- `commands/permissions.rs` — permission checks not logged
- `state.rs`, `resource_monitor.rs`, `commands/tray.rs`, `commands/logging.rs`, `main.rs`

---

## 5. Risks and Design Implications

1. **`set_global_default` exclusivity**: Only one subscriber can be the global default. Verify no dependency sets it. If problematic, use a `tracing::Dispatch` scoped to a guard instead.

2. **Migration surface is moderate**: 72 call sites + 1 structured log call. Could be migrated incrementally by having the custom macros delegate to `tracing` macros internally (wrapper approach), or replaced in bulk.

3. **`log_transcription` is special**: It writes structured JSONL to `transcriptions.jsonl`, not the standard `app.log`. The tracing layer would need a separate handler or a separate layer to replicate this.

4. **Ring buffer sizing**: 500 events is reasonable. At ~72 logging sites and normal usage patterns, this represents many minutes of history. Memory cost is modest (each `LogEvent` with serialized fields is ~200-500 bytes, so ~250KB max).

5. **Event name collision**: Use a namespaced event name like `log://event` to avoid colliding with the 12 existing app events.

6. **Tauri Channel alternative**: For high-frequency events (e.g., `audio-level` fires per audio buffer), consider Tauri Channels over the event system. Channels are designed for ordered streaming and are more performant than `emit()`.

7. **Coverage gaps**: The `transcriber/`, `vad.rs`, and `commands/permissions.rs` modules have no logging at all. Adding tracing instrumentation there would improve observability of model loading, VAD internals, and permission checks.

---

## Sources

- [Tauri 2 State Management](https://v2.tauri.app/develop/state-management/)
- [Tauri 2 Capabilities](https://v2.tauri.app/security/capabilities/)
- [Tauri 2 Capabilities for Windows and Platforms](https://v2.tauri.app/learn/security/capabilities-for-windows-and-platforms/)
- [Tauri 2 Calling the Frontend from Rust](https://v2.tauri.app/develop/calling-frontend/)
- [Tauri `Emitter` trait docs](https://docs.rs/tauri/2.0.0/tauri/trait.Emitter.html)
- [Tauri `Listener` trait docs](https://docs.rs/tauri/2.0.0/tauri/trait.Listener.html)
- [Tauri `WebviewWindow` API](https://v2.tauri.app/reference/javascript/api/namespacewebviewwindow/)
- [tracing_subscriber Layer trait docs](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/trait.Layer.html)
- [tracing Visit trait docs](https://docs.rs/tracing/latest/tracing/field/trait.Visit.html)
- [Custom Logging in Rust Using tracing (Bryan Burgers)](https://burgers.io/custom-logging-in-rust-using-tracing)
- [tauri-plugin-log tracing support request (#2516)](https://github.com/tauri-apps/plugins-workspace/issues/2516)
