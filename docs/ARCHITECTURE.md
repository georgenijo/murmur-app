# Architecture

## Overview

Murmur is a privacy-first, local-only voice dictation app for macOS. You speak, it transcribes — no cloud, no API keys, no internet. All inference runs on-device using Apple Silicon's GPU (or CPU for the Moonshine backend).

Built with **Tauri 2** (Rust backend + React frontend). ~25MB installed, no Python, no sidecar.

---

## Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Desktop framework | Tauri 2 | Rust backend, React frontend, smaller than Electron |
| UI | React 18 + TypeScript + Tailwind CSS 4 | Vite 6 build |
| Audio capture | cpal | Native multi-channel input, mono mix, 16kHz resample |
| Transcription (primary) | whisper-rs → whisper.cpp | Metal GPU-accelerated on Apple Silicon |
| Transcription (alt) | Moonshine via sherpa-rs / ONNX | CPU-only, int8-quantized, ~16ms for 3s audio |
| Text injection | arboard + osascript | Clipboard-first; osascript for auto-paste |
| Keyboard listening | rdev (git main branch) | Global key events; single background thread |
| Release | GitHub Actions + Apple notarization | Signed DMG, auto-updater via `latest.json` |

---

## Data Flow

```
Hotkey event (rdev listener thread)
    ↓
Frontend hook (useHoldDownToggle / useDoubleTapToggle / useCombinedToggle)
    ↓
invoke('start_native_recording') → cpal captures audio
    ↓
invoke('stop_native_recording') → audio thread joins, samples resampled to 16kHz mono
    ↓
TranscriptionBackend::transcribe() → whisper.cpp (Metal GPU) or Moonshine (ONNX CPU)
    ↓
injector::inject_text() → arboard writes to clipboard
    ↓
[optional] osascript simulates Cmd+V → text appears in focused app
    ↓
'transcription-complete' event → frontend history + stats update
```

---

## Rust Backend (`app/src-tauri/src/`)

### `lib.rs` — App Wiring

- Declares all modules, registers 25+ Tauri commands via `invoke_handler!`
- Defines `State` (top-level Tauri state): holds `AppState` + cached notch dimensions
- Defines `MutexExt` trait with `lock_or_recover()`: recovers poisoned mutexes after panics instead of propagating the panic — keeps the app alive if any thread panics while holding a lock
- Hides window on close (keeps app alive in tray), suppresses default "Reopen" behavior
- Caches notch info on the main thread during setup (NSScreen APIs are main-thread-only)

### `state.rs` — Shared State

```rust
enum DictationStatus { Idle, Recording, Processing }

struct DictationState {
    status: DictationStatus,
    model_name: String,   // e.g. "base.en", "moonshine-tiny"
    language: String,
    auto_paste: bool,
}

struct AppState {
    dictation: Mutex<DictationState>,
    backend: Mutex<Box<dyn TranscriptionBackend>>,  // whisper or moonshine
}
```

### `audio.rs` — Audio Capture

- cpal opens the input device and builds a stream; multi-channel interleaved samples are averaged to mono
- RMS computed per chunk → `audio-level` events emitted at ~60fps (throttled via `AtomicU64`) → waveform animation in UI
- Each recording gets a fresh `Arc<Mutex<Vec<f32>>>` buffer — prevents stale data from previous recordings
- Stop command sent via channel; thread joins before samples are consumed
- Linear interpolation resamples captured audio to 16kHz (what Whisper and Moonshine expect)

### `keyboard.rs` — Keyboard Detection

All keyboard detection runs through a **single persistent rdev background thread** shared by two detectors.

#### Hold-Down Detector

Simple 2-state machine:
```
Idle → [key press] → Held (emit 'hold-down-start')
Held → [key release] → Idle (emit 'hold-down-stop')
```
Rejects combos (e.g. Shift+A while Shift is the trigger key cancels hold and emits Stop).

#### Double-Tap Detector

4-state machine:
```
Idle → [press] → WaitingFirstUp
     → [release <200ms] → WaitingSecondDown
     → [press, gap <400ms] → WaitingSecondUp
     → [release <200ms] → FIRE → Idle
```
Rejects: taps held >200ms, modifier+letter combos, gaps >400ms, triple-tap spam.
When `recording=true`, a single tap fires immediately (to stop, not start).

#### Both Mode (Hold-Down + Double-Tap simultaneously)

The interesting one. The problem: a key press could be the start of a double-tap *or* a hold. You can't know which until time passes.

Solution: **deferred hold promotion via a background timer thread + atomic invalidation counter.**

1. On key press, a timer thread is spawned for 200ms
2. If the key is released before 200ms — it was a tap. Timer fires but is invalidated by `HOLD_PRESS_COUNTER` (atomically incremented on release)
3. If the key is still held after 200ms — timer fires `hold-down-start`. Now we're in hold mode
4. Double-tap has priority during the second-press window; hold only wins after 200ms of uninterrupted hold

#### macOS Thread Safety — The rdev Segfault Fix

rdev's keyboard translation uses macOS **TIS/TSM** (Text Input Sources) APIs to map raw key codes to characters. These APIs **must run on the main thread**. rdev listens on a background thread. Without intervention, this silently segfaults.

Fix — one line, called before `rdev::listen()`:
```rust
rdev::set_is_main_thread(false);
```
This tells rdev it is *not* on the main thread, which causes it to wrap TIS/TSM calls in `dispatch_sync(dispatch_get_main_queue(), ...)` — marshaling only those calls to main, while the listener loop stays on the background thread.

### `transcriber/` — Inference Backends

Both backends implement a shared trait:

```rust
trait TranscriptionBackend: Send + Sync {
    fn load_model(&mut self, model_name: &str) -> Result<(), String>;
    fn transcribe(&mut self, samples: &[f32], language: &str) -> Result<String, String>;
    fn model_exists(&self) -> bool;
    fn reset(&mut self);
}
```

**Whisper (`whisper.rs`)**
- wraps whisper.cpp via `whisper-rs` with the Metal GPU backend enabled by default
- single `.bin` file per model (GGML format), sourced from Hugging Face
- model context created lazily on first transcription (fast startup)
- scans 6 standard paths to find existing model files
- suppresses whisper.cpp's verbose stdout via log trampoline

**Moonshine (`moonshine.rs`)**
- wraps sherpa-rs (ONNX runtime), CPU-only, int8-quantized
- requires a directory of 5 ONNX files: `preprocess`, `encode`, `uncached_decode`, `cached_decode`, `tokens`
- downloaded as `.tar.bz2`, extracted in-process using pure Rust (`bzip2` + `tar` crates)
- English-only; ignores the `language` parameter
- ~16ms latency for 3 seconds of audio on Apple Silicon

### `injector.rs` — Text Injection

1. **Clipboard** (always): `arboard` writes text to the system clipboard
2. **Auto-paste** (optional): waits 150ms (clipboard sync + window focus), then:
   ```
   osascript -e 'tell application "System Events" to keystroke "v" using command down'
   ```
   The 150ms delay is intentional — without it, the target window hasn't regained focus yet.
   Previous approaches (`engio`, rdev simulate) broke on Sonoma/Sequoia. osascript is the reliable path.
   Requires Accessibility permission.

### `commands/recording.rs` — Transcription Pipeline & RAII Guard

**`IdleGuard`** — RAII guard wrapping the transcription pipeline:
- On drop (if not disarmed), resets status to `Idle` and clears the "processing" keyboard flag
- Guarantees the UI never gets stuck in "Processing" on any error path
- Disarmed when handing off successfully to prevent double-reset

**`run_transcription_pipeline()`**:
1. Load model (lazy; no-op if already loaded)
2. Run inference, timed
3. Inject text via `app_handle.run_on_main_thread()` (osascript requires main thread)

### `commands/overlay.rs` — Notch Overlay

- `detect_notch_info()`: reads `NSScreen.mainScreen().safeAreaInsets()` via `objc2`; uses `auxiliaryTopLeftArea` + `auxiliaryTopRightArea` to compute notch width. Main-thread only.
- `raise_window_above_menubar()`: sets NSWindow level to **25** (NSMainMenuWindowLevel = 24). Calls private API `_setPreventsActivation(true)` to prevent focus-stealing on click; guarded with `respondsToSelector()` for forward compatibility.
- `register_screen_change_observer()`: subscribes to `NSApplicationDidChangeScreenParametersNotification` — repositions overlay automatically when displays are plugged/unplugged or lid opens. Observer intentionally leaked (app lifetime).

### `logging.rs` — File Logging

- Writes to `~/Library/Application Support/local-dictation/logs/app.log`
- Rotates to `app.log.1` at 5MB
- ISO 8601 timestamps implemented without external libraries (Howard Hinnant algorithm)
- Separate `frontend.log` for JS-side log calls
- `LOG_MUX` static mutex ensures thread-safe appends

---

## Frontend (`app/src/`)

### `App.tsx` — Main Orchestrator

Wires all hooks together. Key state:
- `modelReady` — null (checking) | false (needs download) | true (ready)
- `initialized` — backend init complete
- `accessibilityGranted` — macOS Accessibility permission
- `status` — `'idle' | 'recording' | 'processing'`

**Dual-mode hook pattern** — all three hooks always called (Rules of Hooks); gated by `enabled`:
```tsx
useHoldDownToggle({ enabled: settings.recordingMode === 'hold_down', ... });
useDoubleTapToggle({ enabled: settings.recordingMode === 'double_tap', ... });
useCombinedToggle({ enabled: settings.recordingMode === 'both', ... });
```

### Key Hooks

| Hook | Responsibility |
|------|---------------|
| `useRecordingState` | Recording/transcription state machine, event listeners |
| `useHoldDownToggle` | Hold-down mode, error recovery + auto-restart |
| `useDoubleTapToggle` | Double-tap mode, syncs `recording` state to backend |
| `useCombinedToggle` | Both modes; `holdActiveRef` prevents double-tap firing on hold release |
| `useSettings` | localStorage persistence, OS autostart sync |
| `useAutoUpdater` | OTA updates, min-version enforcement, semver comparison |

**`transcription-complete` as single source of truth** — history entries are added *only* via the Rust event, never in `handleStop()`. Prevents duplicates when the overlay initiates recording independently.

**Ref-based state in callbacks** — `statusRef` stays in sync with `status` state so hotkey callbacks always read current status without stale closure captures.

### `lib/stats.ts` — Usage Metrics

Persisted to localStorage:
- `totalWords`, `totalRecordings`, `totalDurationSeconds`
- `wpmSamples: number[]` — rolling 100-sample history (outlier-resistant)
- Approx tokens = `totalWords × 1.3`

### `OverlayWidget.tsx` — Notch Widget

- Rendered in a separate Tauri window (`overlay.html`), always-on-top, transparent, no decorations
- 7-bar waveform driven by `requestAnimationFrame` + direct DOM refs — bypasses React reconciliation for 60fps
- Single click: stop recording (250ms debounce). Double-click: toggle locked mode (keeps recording across single clicks)
- Reads microphone setting from localStorage (no Tauri IPC needed)

---

## macOS Permissions

| Permission | Required For |
|-----------|-------------|
| Microphone | Audio capture (always required) |
| Accessibility | Global hotkeys (rdev), auto-paste (osascript) |

Accessibility is checked via `AXIsProcessTrusted()` FFI. If not granted, a system prompt is triggered via `AXIsProcessTrustedWithOptions()` with `kAXTrustedCheckOptionPrompt`.

---

## Release Pipeline

1. `git tag vX.Y.Z && git push --tags`
2. GitHub Actions: TypeScript check + `cargo test`
3. macOS: `tauri-action` → Developer ID signing → Apple notarization
4. Smoke test: launch built `.app`, verify alive for 5 seconds
5. Publish: `.dmg`, `.app.tar.gz`, `.sig`, `latest.json` → GitHub Release

**Auto-updater**: Tauri updater plugin checks `latest.json` on GitHub Releases. Updates are signed (ed25519). Min-version enforcement removes "Skip"/"Later" for required updates.

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Pure Rust backend | No Python subprocess; faster startup, smaller bundle, no dependency hell |
| Pluggable `TranscriptionBackend` trait | Whisper and Moonshine swap cleanly; same pipeline code |
| Lazy model loading | Fast app startup; model context created on first transcription |
| Clipboard-first injection | Reliable across all apps; auto-paste layered on top |
| osascript for auto-paste | `engio` and rdev simulate broke on Sonoma/Sequoia |
| Single rdev thread, two detectors | Avoids multiple listeners; both detectors share one event stream |
| `set_is_main_thread(false)` | Prevents TIS/TSM segfault on background rdev thread |
| `MutexExt::lock_or_recover()` | Survives panics; no stuck UI state |
| `IdleGuard` RAII | Guarantees status reset on any error path in the transcription pipeline |
| Atomic timer invalidation (Both mode) | Stale hold-timers can't fire after key is released and re-pressed |
| `_setPreventsActivation` + `respondsToSelector` | Overlay never steals focus; forward-compatible with future macOS |
