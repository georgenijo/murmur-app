# Architecture

## Overview

Murmur is a privacy-first, local-only voice dictation app for macOS. You speak, it transcribes -- no cloud, no API keys, no internet. All inference runs on-device using Apple Silicon's GPU (Whisper) or CPU (Moonshine).

Built with **Tauri 2** (Rust backend + React frontend). ~25MB installed, no Python, no sidecar.

---

## Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Desktop framework | Tauri 2 | Rust backend, React frontend, smaller than Electron |
| UI | React 18 + TypeScript + Tailwind CSS 4 | Vite 6 build |
| Audio capture | cpal | Native multi-channel input, mono mix, 16kHz resample |
| VAD | Silero v5.1.2 via whisper-rs | Filters silence before transcription; prevents Whisper hallucination loops |
| Transcription (primary) | whisper-rs -> whisper.cpp | Metal GPU-accelerated on Apple Silicon |
| Transcription (alt) | Moonshine via sherpa-rs / ONNX | CPU-only, int8-quantized, ~16ms for 3s audio |
| Text injection | arboard + osascript | Clipboard-first; osascript for auto-paste |
| Keyboard listening | rdev (git main branch) | Global key events; single background thread |
| Telemetry | tracing + tracing-subscriber | Structured events: ring buffer, JSONL file, real-time frontend emission |
| System info | sysinfo | CPU and memory monitoring for resource panel |
| Release | GitHub Actions + Apple notarization | Signed DMG, auto-updater via `latest.json` |

---

## Data Flow

```
Hotkey event (rdev listener thread)
    |
Frontend hook (useHoldDownToggle / useDoubleTapToggle / useCombinedToggle)
    |
invoke('start_native_recording') --> cpal captures audio
    |
invoke('stop_native_recording') --> audio thread joins, samples resampled to 16kHz mono
    |
Silero VAD filters silence (configurable sensitivity 0-100)
    |
TranscriptionBackend::transcribe() --> whisper.cpp (Metal GPU) or Moonshine (ONNX CPU)
    |
injector::inject_text() --> arboard writes to clipboard
    |
[optional] osascript simulates Cmd+V --> text appears in focused app
    |
'transcription-complete' event --> all windows: history + stats update
```

---

## Three-Window Architecture

Murmur runs three distinct webview windows, each with separate Tauri capabilities following the principle of least privilege:

| Window | Label | Entry Point | Purpose |
|--------|-------|-------------|---------|
| Main | `main` | `index.html` | Settings, recording controls, transcription history, stats, resource monitor, permissions, model download, about/update modals |
| Overlay | `overlay` | `overlay.html` | Dynamic Island-style notch widget with waveform visualization. Always-on-top, transparent, not focusable |
| Log Viewer | `log-viewer` | `log-viewer.html` | Structured event browser with Events tab (stream/level filtering) and Metrics tab (transcription timing charts) |

All three windows intercept close requests and hide instead of being destroyed, preserving state. The overlay reads settings from localStorage directly (no shared React context across windows).

---

## Rust Backend (`app/src-tauri/src/`)

### `lib.rs` -- App Wiring

- Declares all modules, registers 30 Tauri commands via `invoke_handler!`
- Defines `State` (top-level Tauri state): holds `AppState` + cached notch dimensions
- Defines `MutexExt` trait with `lock_or_recover()`: recovers poisoned mutexes after panics instead of propagating the panic -- keeps the app alive if any thread panics while holding a lock
- Hides window on close (keeps app alive in tray), suppresses default "Reopen" behavior -- dock icon click only shows the main window when no windows are visible (prevents overlay clicks from unhiding the main window)
- Caches notch info on the main thread during setup (NSScreen APIs are main-thread-only)
- Registers 5 Tauri plugins: opener, autostart (LaunchAgent), updater, notification, process

### `state.rs` -- Shared State

```rust
enum DictationStatus { Idle, Recording, Processing }

struct DictationState {
    status: DictationStatus,
    model_name: String,        // e.g. "base.en", "moonshine-tiny"
    language: String,
    auto_paste: bool,
    auto_paste_delay_ms: u64,  // 10-500, default 50
    vad_sensitivity: u32,      // 0-100, default 50
}

struct AppState {
    dictation: Mutex<DictationState>,
    backend: Mutex<Box<dyn TranscriptionBackend>>,  // whisper or moonshine
}
```

Note: `DictationState::default()` sets `model_name` to `"base.en"`, but the frontend default is `"moonshine-tiny"`. The frontend always calls `configure_dictation` before `initialized` becomes true, so the Rust default is effectively overwritten before any recording can occur. New users start with `moonshine-tiny`.

### `audio.rs` -- Audio Capture

- cpal opens the input device and builds a stream; multi-channel interleaved samples are averaged to mono
- RMS computed per chunk -> `audio-level` events emitted at ~60fps (throttled via `AtomicU64` to 16ms minimum gap) -> waveform animation in UI
- Each recording gets a fresh `Arc<Mutex<Vec<f32>>>` buffer -- prevents stale data from previous recordings
- Stop command sent via channel; thread joins before samples are consumed
- Linear interpolation resamples captured audio to 16kHz (what Whisper and Moonshine expect)
- Recording state stored in a global `OnceLock<Mutex<RecordingState>>` with command sender, thread handle, shared buffer, sample rate, start timestamp, and device name
- Initialization handshake: `start_recording` waits up to 5 seconds for the audio thread to signal ready
- Device name redacted in release build logs

### `keyboard.rs` -- Keyboard Detection

All keyboard detection runs through a **single persistent rdev background thread** shared by two detectors.

#### Hold-Down Detector

Simple 2-state machine:
```
Idle --> [key press] --> Held (emit 'hold-down-start')
Held --> [key release] --> Idle (emit 'hold-down-stop')
```
Rejects combos (e.g. Shift+A while Shift is the trigger key cancels hold and emits Stop).

In hold-down-only mode, there is no minimum hold duration: a key press immediately emits `hold-down-start` and a key release immediately emits `hold-down-stop`. The 0.3-second minimum recording threshold in `stop_native_recording` handles discarding phantom triggers.

#### Double-Tap Detector

4-state machine:
```
Idle --> [press] --> WaitingFirstUp
     --> [release <200ms] --> WaitingSecondDown
     --> [press, gap <400ms] --> WaitingSecondUp
     --> [release <200ms] --> FIRE --> Idle
```
Rejects: taps held >200ms, modifier+letter combos, gaps >400ms, triple-tap spam.
When `recording=true`, a single tap fires immediately (to stop, not start).

#### Both Mode (Hold-Down + Double-Tap simultaneously)

The interesting one. The problem: a key press could be the start of a double-tap *or* a hold. You can't know which until time passes.

Solution: **deferred hold promotion via a background timer thread + atomic invalidation counter.**

1. On key press, a timer thread is spawned for 200ms
2. If the key is released before 200ms -- it was a tap. Timer fires but is invalidated by `HOLD_PRESS_COUNTER` (atomically incremented on release)
3. If the key is still held after 200ms -- timer fires, sets `HOLD_PROMOTED` to true, and emits `hold-down-start`. Now we're in hold mode
4. The hold-down detector is suppressed when the double-tap detector is in its second phase (WaitingSecondDown or WaitingSecondUp), preventing false hold events during double-tap sequences
5. On key release: if promoted, emits `hold-down-stop`. If not promoted but double-tap fired, emits `double-tap-toggle`. Otherwise (short single tap with no double-tap), nothing is emitted -- no recording was ever started

#### Processing State Management

When entering the transcription pipeline, `set_processing(true)` is called:
- Both-mode callback ignores all key events
- Pending hold-promotion timers are invalidated via `HOLD_PRESS_COUNTER`
- Both detectors are reset
- On exit from processing, cooldowns are applied to prevent accidental re-triggers

#### macOS Thread Safety -- The rdev Segfault Fix

rdev's keyboard translation uses macOS **TIS/TSM** (Text Input Sources) APIs to map raw key codes to characters. These APIs **must run on the main thread**. rdev listens on a background thread. Without intervention, this silently segfaults.

Fix -- one line, called before `rdev::listen()`:
```rust
rdev::set_is_main_thread(false);
```
This tells rdev it is *not* on the main thread, which causes it to wrap TIS/TSM calls in `dispatch_sync(dispatch_get_main_queue(), ...)` -- marshaling only those calls to main, while the listener loop stays on the background thread.

A heartbeat monitor thread logs trace-level messages every 60 seconds while the listener is active.

### `vad.rs` -- Voice Activity Detection

- Uses **Silero VAD v5.1.2** via whisper-rs `WhisperVadContext`
- Filters out silence before transcription to prevent Whisper hallucination loops on quiet recordings
- VAD threshold computed from user-configurable sensitivity: `threshold = 1.0 - (vad_sensitivity / 100.0)` (0-100 scale, default 50)
- Configured: 1 thread, GPU disabled
- Returns speech segments as centisecond timestamps, converted to sample indices
- Three outcomes: `NoSpeech` (skip transcription entirely), `Speech` (trimmed samples), `Error` (fallback to unfiltered audio)
- `filter_speech` is `!Send` (WhisperVadContext constraint), must run via `spawn_blocking`
- VAD context is created and destroyed per transcription (not cached like WhisperState)

### `transcriber/` -- Inference Backends

Both backends implement a shared trait:

```rust
trait TranscriptionBackend: Send + Sync {
    fn name(&self) -> &str;
    fn load_model(&mut self, model_name: &str) -> Result<(), String>;
    fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<String, String>;
    fn model_exists(&self) -> bool;
    fn models_dir(&self) -> PathBuf;
    fn reset(&mut self);
}
```

Backend selection is determined by model name prefix: names starting with `"moonshine-"` use MoonshineBackend, everything else uses WhisperBackend. When the model changes across the whisper/moonshine boundary, the entire `Box<dyn TranscriptionBackend>` is replaced. When the model changes within the same backend type, `reset()` is called to force a reload.

**Whisper (`whisper.rs`)**
- Wraps whisper.cpp via `whisper-rs` with the Metal GPU backend enabled by default
- Single `.bin` file per model (GGML format), sourced from Hugging Face
- **WhisperState caching (v0.7.8)**: The `WhisperBackend` struct holds `context: Option<WhisperContext>`, `state: Option<WhisperState>`, and `loaded_model_name: Option<String>`. On first `load_model()`, a `WhisperContext` is created from the model file and `ctx.create_state()` allocates GPU/Metal buffers exactly once. The `WhisperState` is stored and reused across all subsequent transcriptions (`state.full(params, samples)`). Only a model name change triggers `reset()` and reallocation. Previously, `create_state()` was called per-transcription, causing expensive alloc/free cycles
- Uses greedy sampling (best_of=1), single segment mode, timestamps/progress/special tokens suppressed, blank suppression enabled
- Scans 6 standard paths to find existing model files
- Suppresses whisper.cpp's verbose stdout via log trampoline (`install_logging_hooks()` called once via `std::sync::Once`)

**Moonshine (`moonshine.rs`)**
- Wraps sherpa-rs (ONNX runtime), CPU-only, int8-quantized
- Requires a directory of 5 ONNX files: `preprocess.onnx`, `encode.int8.onnx`, `uncached_decode.int8.onnx`, `cached_decode.int8.onnx`, `tokens.txt`
- Downloaded as `.tar.bz2` from sherpa-onnx GitHub releases, extracted in-process using pure Rust (`bzip2` + `tar` crates). Partial extractions are cleaned up on failure
- English-only; ignores the `language` parameter
- ~16ms latency for 3 seconds of audio on Apple Silicon

**Available Models (7 total)**

| Model | Backend | Size |
|-------|---------|------|
| `moonshine-tiny` | Moonshine (CPU) | ~124 MB |
| `moonshine-base` | Moonshine (CPU) | ~286 MB |
| `tiny.en` | Whisper (Metal GPU) | ~75 MB |
| `base.en` | Whisper (Metal GPU) | ~150 MB |
| `small.en` | Whisper (Metal GPU) | ~500 MB |
| `medium.en` | Whisper (Metal GPU) | ~1.5 GB |
| `large-v3-turbo` | Whisper (Metal GPU) | ~3 GB |

The initial model downloader screen shows a curated subset of 4 models: `moonshine-tiny`, `moonshine-base`, `large-v3-turbo`, `base.en`. Default selection: `moonshine-tiny`.

### `injector.rs` -- Text Injection

1. **Clipboard** (always): `arboard` writes text to the system clipboard. Empty/whitespace-only text is silently skipped.
2. **Auto-paste** (optional): waits a configurable delay (10-500ms, default 50ms), then:
   ```
   osascript -e 'tell application "System Events" to keystroke "v" using command down'
   ```
   The delay allows the target window to regain focus.
   Retries once on failure with 100ms backoff. 2-second timeout on the entire operation.
   On failure, emits `auto-paste-failed` with a hint message ("press Cmd+V to paste manually").
   Requires Accessibility permission -- if not granted, text stays in clipboard with a warning log.
   Previous approaches (`engio`, rdev simulate) broke on Sonoma/Sequoia. osascript is the reliable path.

### `commands/recording.rs` -- Transcription Pipeline & RAII Guard

**`IdleGuard`** -- RAII guard wrapping the transcription pipeline:
- On drop (if not disarmed), resets status to `Idle` and clears the "processing" keyboard flag
- Guarantees the UI never gets stuck in "Processing" on any error path
- Both the outer command and inner pipeline have their own guards with a disarm handoff pattern

**`run_transcription_pipeline()`**:
1. Read state -- model name, language, auto-paste settings, paste delay, VAD sensitivity in a single lock acquisition
2. Pre-VAD diagnostics -- compute RMS and peak amplitude of raw audio, log device name
3. VAD phase -- if VAD model exists, run Silero VAD on a blocking thread. Three outcomes: NoSpeech (skip), Speech (trimmed), Error (fallback). If VAD model missing, spawn background download for next time
4. Transcription phase -- lock backend mutex, call `load_model()` (lazy), then `transcribe()`
5. Text injection -- dispatched to main thread via `run_on_main_thread()` (osascript requires main thread). Clipboard write + optional paste with 2-second timeout
6. Structured logging -- emits `vad_ms`, `inference_ms`, `paste_ms`, `total_ms`, `audio_secs`, `word_count`, `char_count`, `model`, `backend` as tracing fields

**`cancel_native_recording`**: Transitions Recording -> Idle without transcription. Used by "both" mode to discard speculative recordings from short taps.

**Minimum recording threshold**: Recordings shorter than 0.3 seconds (4,800 samples at 16kHz) are silently discarded as phantom triggers.

### `commands/overlay.rs` -- Notch Overlay

- `detect_notch_info()`: reads `NSScreen.mainScreen().safeAreaInsets()` via `objc2`; uses `auxiliaryTopLeftArea` + `auxiliaryTopRightArea` to compute notch width. Main-thread only. Fallback: 200px wide, 37px tall.
- `raise_window_above_menubar()`: sets NSWindow level to **25** (NSMainMenuWindowLevel = 24). Calls private API `_setPreventsActivation(true)` to prevent focus-stealing on click; guarded with `respondsToSelector()` for forward compatibility.
- `register_screen_change_observer()`: subscribes to `NSApplicationDidChangeScreenParametersNotification` -- repositions overlay automatically when displays are plugged/unplugged or lid opens. Emits `notch-info-changed` to frontend. Observer intentionally leaked (app lifetime).
- Overlay width = notch_width + 120px (60px expansion per side). Mouse events explicitly re-enabled (`setIgnoreCursorEvents(false)`) because `focusable:false` disables them on macOS.

### `commands/tray.rs` -- Tray Icon

- Static 66x66 RGBA icon (3x resolution for 22pt Retina menu bar) showing 5 vertical capsule bars in an equalizer style, rendered as white with anti-aliased edges
- `update_tray_icon` is a registered no-op command -- the tray icon is always static white. Command kept to avoid breaking the registered handler
- Tray menu: "Show Murmur" (shows and focuses main window) and "Quit Murmur" (exits app). Left-click on tray icon also shows the main window

### `commands/models.rs` -- Model Downloads

- `check_model_exists`: checks both the currently configured backend AND the other backend type, so the download screen does not appear if any model is installed
- `check_specific_model_exists`: verifies a named model exists on disk. Includes path traversal protection (rejects `..`, `/`, `\` in model names)
- `download_model`: streaming download with progress events. Whisper models download as single `.bin` files from Hugging Face. Moonshine models download as `.tar.bz2` archives from sherpa-onnx GitHub releases, extracted via `bzip2` + `tar` on a blocking thread
- **VAD model co-download**: when downloading any transcription model, the Silero VAD model (`ggml-silero-v5.1.2.bin`, ~1.8MB) is automatically co-downloaded if not already present. VAD download failure is non-fatal
- **Lazy VAD download**: `ensure_vad_model` is a fallback for users who upgrade from a pre-VAD version. If the VAD model is missing at transcription time, a background download is kicked off for next time. Emits `recording-status-changed` with value `"downloading-vad"` during explicit download
- All downloads use a temp-file-then-rename pattern for atomicity. Partial downloads never appear as valid models

### `telemetry.rs` -- Structured Event System

Replaces the former `logging.rs`. All application logging goes through `tracing` with two output layers:

```
tracing event
    |
    +--> Pretty-printed text file (app.log / app.dev.log) via tracing_appender
    |
    +--> TauriEmitterLayer
             |
             +--> In-memory ring buffer (500 events, FIFO eviction)
             |
             +--> JSONL file (events.jsonl / events.dev.jsonl)
             |
             +--> 'app-event' emission to all frontend windows
```

**TauriEmitterLayer**: A custom `tracing_subscriber::Layer` that intercepts all tracing events and:
1. Collects fields via `JsonVisitor` into serde_json values
2. Builds an `AppEvent` struct (timestamp, stream/target, level, summary/message, data/fields)
3. Pushes to ring buffer (capped at 500, evicts oldest)
4. Writes as JSONL line to persistent file
5. Emits `app-event` to all Tauri windows

**Tracing targets (streams)**: `pipeline`, `audio`, `system`, `keyboard`.

**Privacy stripping**: In release builds, all string fields from `pipeline` target events are stripped from the data object. Only numeric fields survive. The summary (message) is not stripped.

**JSONL rotation**: File is rotated (renamed to `.jsonl.1`) when it exceeds 5MB.

**Buffer seeding**: On startup, the ring buffer is pre-populated with up to 500 events from the existing JSONL file.

**Log files**: Separate filenames for dev (`app.dev.log`, `events.dev.jsonl`) vs release builds.

**Frontend logging**: The `log_frontend` command routes frontend log messages through the Rust tracing system at INFO/WARN/ERROR levels with `source="frontend"`.

### `resource_monitor.rs` -- System Resource Monitoring

- Reports CPU usage percentage and used memory (MB) via `sysinfo` crate
- Uses a persistent `System` instance stored in a `static Mutex<Option<System>>`, initialized on first call
- First call returns ~0% CPU (baseline measurement behavior of sysinfo)
- Polled every 1 second by the frontend when the resource panel is expanded

---

## Frontend (`app/src/`)

### `App.tsx` -- Main Orchestrator

Wires all hooks together. Key state:
- `modelReady` -- null (checking) | false (needs download) | true (ready)
- `initialized` -- backend init complete
- `accessibilityGranted` -- macOS Accessibility permission
- `status` -- `'idle' | 'recording' | 'processing'`

**Dual-mode hook pattern** -- all three hooks always called (Rules of Hooks); gated by `enabled`:
```tsx
useHoldDownToggle({ enabled: settings.recordingMode === 'hold_down', ... });
useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', ... });
useCombinedToggle({ enabled: settings.recordingMode === 'both', ... });
```

### Key Hooks

| Hook | Responsibility |
|------|---------------|
| `useRecordingState` | Recording/transcription state machine, event listeners, audio level tracking, locked mode, stats integration |
| `useHoldDownToggle` | Hold-down mode (rdev press/release events), error recovery + auto-restart |
| `useDoubleTapToggle` | Double-tap mode (rdev events), syncs `recording` state to backend |
| `useCombinedToggle` | Both modes; `holdActiveRef` prevents double-tap firing on hold release. Calls `cancel_native_recording` for speculative recording discard |
| `useSettings` | localStorage persistence, OS autostart sync, backend configuration pushes |
| `useAutoUpdater` | OTA updates, min-version enforcement, semver comparison, forced updates, macOS notifications |
| `useInitialization` | One-time init sequence (initDictation + configure) on mount |
| `useHistoryManagement` | Transcription history array with localStorage persistence (max 50 entries) |
| `useEventStore` | Structured event log buffer: backend hydration, live `app-event` streaming, batched rendering via rAF, filter/clear |
| `useResourceMonitor` | CPU/memory polling every 1 second, rolling 60-reading buffer |
| `useShowAboutListener` | Listens for `show-about` tray event, manages about modal state |

**`transcription-complete` as single source of truth** -- history entries are added *only* via the Rust event, never in `handleStop()`. Prevents duplicates when the overlay initiates recording independently.

**Ref-based state in callbacks** -- `statusRef` stays in sync with `status` state so hotkey callbacks always read current status without stale closure captures. Callbacks stored in refs prevent listener setup from re-running on identity changes.

### `lib/settings.ts` -- Settings & Model Configuration

```typescript
interface Settings {
    model: ModelOption;         // default: 'moonshine-tiny'
    doubleTapKey: DoubleTapKey; // default: 'shift_l'
    language: string;           // default: 'en'
    autoPaste: boolean;         // default: false
    autoPasteDelayMs: number;   // default: 50 (range: 10-500)
    recordingMode: RecordingMode; // default: 'hold_down'
    microphone: string;         // default: 'system_default'
    launchAtLogin: boolean;     // default: false
    vadSensitivity: number;     // default: 50 (range: 0-100)
}
```

Settings persisted to localStorage under key `"dictation-settings"`. Migration handles legacy `hotkey` recording mode -> `hold_down`.

### `lib/stats.ts` -- Usage Metrics

Persisted to localStorage:
- `totalWords`, `totalRecordings`, `totalDurationSeconds`
- `wpmSamples: number[]` -- rolling 100-sample history (outlier-resistant)
- Approx tokens = `totalWords * 1.3`

### `lib/events.ts` -- Event System Types

```typescript
interface AppEvent {
    timestamp: string;
    stream: StreamName;   // 'pipeline' | 'audio' | 'keyboard' | 'system'
    level: LevelName;     // 'trace' | 'debug' | 'info' | 'warn' | 'error'
    summary: string;
    data: Record<string, unknown>;
}
```

Color constants map each stream and level to Tailwind classes (bg, text, dot) for the log viewer UI.

### `OverlayWidget.tsx` -- Notch Widget

- Rendered in the overlay window, always-on-top, transparent, no decorations
- **Three visual states**: Idle (small mic icon, dimmed), Recording (expanded, red pulsing dot + animated waveform), Processing (expanded, spinning circle + dimmed waveform)
- **7-bar waveform** driven by `requestAnimationFrame` + direct DOM refs -- bypasses React reconciliation for 60fps. Center bars are taller (envelope shaping), random jitter for organic feel
- Spring-like expand/collapse transition (`cubic-bezier(0.34, 1.56, 0.64, 1)` over 500ms)
- Single click: stop recording (250ms debounce). Double-click: toggle locked mode (keeps recording across single clicks)
- Reads microphone setting from localStorage (no Tauri IPC needed)

### Log Viewer (`components/log-viewer/`)

**Events tab**:
- Stream filter chips -- toggle which event streams to show (default active: `pipeline`, `audio`, `system`)
- Level filter -- toggle info/warn/error visibility
- Event list with auto-scroll (disengages on manual scroll up, re-engages within 40px of bottom)
- Expandable rows: timestamp, colored stream chip, level label, summary text. Click to reveal JSON data
- Copy All and Clear buttons

**Metrics tab**:
- Extracts transcription timing from pipeline events where `summary === 'transcription complete'`
- Last 20 transcriptions displayed
- Four toggleable series: Total, Inference, VAD, Paste
- Stat cards with latest value, average, trend indicator (up/down/flat, 10% threshold)
- Two SVG line charts: Total + Inference (upper, 150px), VAD + Paste (lower, 120px)
- Auto-scaled Y-axis with round tick marks

### Resource Monitor

- Collapsible panel in the main window; collapse state persisted to localStorage
- Header always shows current CPU% and memory MB
- Expanded view: SVG polyline chart with CPU and memory lines, grid at 25/50/75%, legend
- Only polls when expanded (performance optimization)
- Rolling window of 60 readings (1 minute of data)

---

## Tauri Commands (30)

| Module | Command | Description |
|--------|---------|-------------|
| recording | `init_dictation` | Returns initialized/idle response (no-op marker) |
| recording | `process_audio` | Accepts base64-encoded WAV, runs full pipeline |
| recording | `get_status` | Returns status, model name, language |
| recording | `configure_dictation` | Updates model, language, autoPaste, autoPasteDelayMs, vadSensitivity |
| recording | `start_native_recording` | Begins cpal audio capture; Idle -> Recording |
| recording | `stop_native_recording` | Stops capture, runs VAD + transcription + injection pipeline |
| recording | `cancel_native_recording` | Discards recording without transcribing (speculative hold-down) |
| permissions | `open_system_preferences` | Opens macOS System Settings to Microphone pane |
| permissions | `check_accessibility_permission` | Returns boolean for Accessibility status |
| permissions | `request_accessibility_permission` | Triggers Accessibility prompt + opens Settings |
| permissions | `request_microphone_permission` | Opens Microphone privacy pane |
| permissions | `list_audio_devices` | Returns Vec of input device names |
| keyboard | `start_keyboard_listener` | Starts rdev listener with hotkey and mode |
| keyboard | `stop_keyboard_listener` | Stops processing keyboard events (thread stays alive) |
| keyboard | `update_keyboard_key` | Changes hotkey at runtime; emits stop if held |
| keyboard | `set_keyboard_recording` | Syncs recording state to double-tap detector |
| logging | `get_log_contents` | Returns last N lines of pretty-printed log file |
| logging | `clear_logs` | Deletes all log files + clears event ring buffer |
| logging | `log_frontend` | Routes frontend message through Rust tracing |
| logging | `open_log_viewer` | Shows and focuses the log-viewer window |
| models | `check_model_exists` | Checks if any model exists (either backend) |
| models | `check_specific_model_exists` | Checks named model on disk (path traversal protected) |
| models | `download_model` | Streaming download + VAD co-download |
| tray | `update_tray_icon` | No-op (static white icon). Kept for API compat |
| overlay | `show_overlay` | Positions and shows the overlay window |
| overlay | `hide_overlay` | Hides the overlay window |
| overlay | `get_notch_info` | Returns cached notch dimensions or None |
| telemetry | `get_event_history` | Returns ring buffer (up to 500 events) |
| telemetry | `clear_event_history` | Clears the event ring buffer |
| resource_monitor | `get_resource_usage` | Returns CPU% and memory MB |

---

## Events (Rust -> Frontend)

| Event | Payload | Description |
|-------|---------|-------------|
| `audio-level` | f32 (RMS 0.0-1.0) | Real-time audio level during recording, ~60fps |
| `recording-status-changed` | String | Status transitions: `"idle"`, `"recording"`, `"processing"`, `"downloading-vad"` |
| `transcription-complete` | `{text, duration}` | Broadcast to all windows after non-empty transcription |
| `auto-paste-failed` | String (hint) | Paste failed; text is in clipboard |
| `download-progress` | `{received, total}` | Streaming download progress (bytes) |
| `double-tap-toggle` | `()` | Double-tap detected |
| `hold-down-start` | `()` | Hold key pressed (or promoted in both mode) |
| `hold-down-stop` | `()` | Hold key released |
| `keyboard-listener-error` | String | rdev thread error; frontend retries after 2s |
| `notch-info-changed` | `Option<{notch_width, notch_height}>` | Display config changed |
| `app-event` | `AppEvent` | Every tracing event, powers log viewer |
| `show-about` | `()` | Tray menu "About" click |

---

## macOS Permissions

| Permission | Required For |
|-----------|-------------|
| Microphone | Audio capture (always required) |
| Accessibility | Global hotkeys (rdev), auto-paste (osascript) |

Accessibility is checked via `AXIsProcessTrusted()` FFI. If not granted, a system prompt is triggered via `AXIsProcessTrustedWithOptions()` with `kAXTrustedCheckOptionPrompt`.

---

## Data Directory

All models, logs, and event data are stored under:
```
~/Library/Application Support/local-dictation/
```

This is a legacy name from before the app was renamed to Murmur. Contents:
- `models/` -- Whisper GGML `.bin` files, Moonshine ONNX directories, Silero VAD model
- `logs/` -- `app.log` / `app.dev.log` (pretty-printed), `events.jsonl` / `events.dev.jsonl` (structured)

---

## Build & Release

### Dev Configuration

```bash
cd app && npm run tauri dev        # Dev with hot reload
cd app && npm run tauri build      # Production .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1  # Rust tests (single-threaded)
cd app && npx tsc --noEmit         # TypeScript check
```

`tauri.dev.conf.json` overrides only two fields from production config: `identifier` -> `"com.localdictation.dev"` and `productName` -> `"Local Dictation Dev"`. This ensures the dev build installs as a separate app.

### Release Pipeline

1. `git tag vX.Y.Z && git push --tags`
2. GitHub Actions: TypeScript check + `cargo test`
3. macOS: `tauri-action` -> Developer ID signing -> Apple notarization
4. Smoke test: launch built `.app`, verify alive for 5 seconds
5. Publish: `.dmg`, `.app.tar.gz`, `.sig`, `latest.json` -> GitHub Release

**Auto-updater**: Tauri updater plugin checks `latest.json` on GitHub Releases. Updates are signed (ed25519). Min-version enforcement removes "Skip"/"Later" for required updates.

### Release Profile

```toml
[profile.release]
panic = "abort"
codegen-units = 1
lto = true
opt-level = "s"
strip = true
```

---

## Thread Model

The app uses at least 4 persistent background threads plus short-lived workers:

| Thread | Lifetime | Purpose |
|--------|----------|---------|
| rdev keyboard listener | App lifetime (spawned once) | Global key event loop |
| Keyboard heartbeat | App lifetime | Trace-level heartbeat every 60s |
| Audio capture | Per-recording session | cpal stream + mono mix |
| Hold-promotion timer | Per key press (both mode) | 200ms sleep, atomic check |

Plus Tokio async tasks for VAD (`spawn_blocking`), downloads, and text injection dispatch.

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Pure Rust backend | No Python subprocess; faster startup, smaller bundle, no dependency hell |
| Pluggable `TranscriptionBackend` trait | Whisper and Moonshine swap cleanly; same pipeline code |
| Lazy model loading + WhisperState caching | Fast app startup; GPU buffers allocated once, reused across transcriptions |
| VAD pre-filtering | Prevents Whisper hallucination loops on silence; skips unnecessary inference |
| Clipboard-first injection | Reliable across all apps; auto-paste layered on top |
| osascript for auto-paste | `engio` and rdev simulate broke on Sonoma/Sequoia |
| Single rdev thread, two detectors | Avoids multiple listeners; both detectors share one event stream |
| `set_is_main_thread(false)` | Prevents TIS/TSM segfault on background rdev thread |
| `MutexExt::lock_or_recover()` | Survives panics; no stuck UI state |
| `IdleGuard` RAII | Guarantees status reset on any error path in the transcription pipeline |
| Atomic timer invalidation (Both mode) | Stale hold-timers can't fire after key is released and re-pressed |
| `_setPreventsActivation` + `respondsToSelector` | Overlay never steals focus; forward-compatible with future macOS |
| tracing-based telemetry | Unified observability: every backend event captured, persisted, and streamed to frontend |
| Privacy stripping in release builds | Pipeline string fields stripped from structured events; only numerics survive |
| Three-window least-privilege | Overlay gets minimal permissions; log viewer gets event-only access; main window gets full permissions |
| VAD co-download | Silero model bundled with transcription model downloads; lazy fallback for pre-VAD upgrades |
