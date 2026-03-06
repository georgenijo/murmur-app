# Orchestrator Review

Cross-reference of all 5 agent findings against ARCHITECTURE.md, FEATURES.md (v0.4.0), CHANGELOG, and open issues.

---

## Confirmed Accurate

The following findings are consistent across agents and match known architecture:

### Core Architecture
- **Three recording modes**: hold-down, double-tap, both — all agents confirm (FEATURES.md only lists 2; "both" mode added later)
- **Two transcription backends**: Whisper (Metal GPU via whisper-rs) and Moonshine (CPU via sherpa-rs/ONNX) — confirmed by Agents 2-5
- **Clipboard-first text injection**: arboard always, osascript auto-paste optional — Agents 1-4
- **Three-window architecture**: main, overlay, log-viewer — Agents 1, 4, 5
- **IdleGuard RAII pattern**: Guarantees status reset on any error path — Agents 3, 4
- **MutexExt poison recovery**: `lock_or_recover()` throughout — Agents 3, 4
- **Single rdev listener thread** with `set_is_main_thread(false)` TIS/TSM fix — Agent 4
- **Both-mode arbitration**: 200ms deferred hold promotion via timer thread + atomic counter — Agent 4

### Transcription Pipeline
- **VAD pre-filtering**: Silero VAD v5.1.2 filters silence before Whisper — Agents 3, 4 (not in FEATURES.md v0.4.0)
- **Lazy model loading**: Model context created on first transcription, cached across subsequent runs — Agents 3, 4
- **WhisperState caching**: Agent 4 confirms cached whisper context persisted across transcriptions (v0.7.8 optimization)
- **Minimum 0.3s recording threshold**: Phantom triggers silently discarded — Agents 3, 4
- **Auto-paste delay**: Configurable 10-500ms (default 50ms) — Agents 2, 3
- **VAD sensitivity**: Configurable 0-100 (default 50) — Agents 2, 3, 5

### Frontend
- **7 model options** across 2 backends (not 5 as FEATURES.md states) — Agents 1, 2, 5
- **7-bar waveform** in overlay (FEATURES.md incorrectly says 5, ARCHITECTURE.md says 7) — Agent 1 confirms BAR_COUNT = 7
- **Structured event system**: tracing-based with ring buffer, JSONL persistence, real-time streaming — Agents 2, 3, 4
- **Log viewer is a separate window** (not a modal as FEATURES.md says) with Events + Metrics tabs — Agents 1, 5
- **Auto-updater** with forced updates, min-version enforcement, skip/dismiss, notifications — Agent 2
- **All settings in localStorage**: settings, history, stats, update prefs — Agents 2, 5
- **Dark mode** follows macOS system appearance via Tailwind `dark:` variants — Agent 1

### Events & Commands
- **30 Tauri commands** registered — Agent 3 catalogued all, Agent 4 confirmed independently
- **12 Tauri events** (Rust→Frontend): audio-level, recording-status-changed, transcription-complete, auto-paste-failed, download-progress, double-tap-toggle, hold-down-start, hold-down-stop, keyboard-listener-error, notch-info-changed, app-event, show-about — Agents 2, 3, 4. NOTE: `hold-down-cancel` was listed by Agent 2 as a frontend listener but **revision confirmed it is never emitted from Rust** — the listener in useCombinedToggle.ts is dead code.
- **11 React hooks** — Agent 2 catalogued all with full descriptions

---

## Missing or Incomplete (by agent)

### Agent 1 — Frontend UI
1. **App.tsx not covered** — Expected (out of scope), but App.tsx is the wiring layer. The top-level rendering logic, hook composition, and `modelReady` / `initialized` / `accessibilityGranted` states live there. No agent covered this file.
2. **overlay.html / log-viewer.html entry points** — Not confirmed to exist as separate HTML files. Agent 5 references them from tauri.conf.json but nobody verified the files.
3. **Tray menu items incomplete** — Agent 1 didn't cover tray menu. Agent 3 found "Show Murmur" and "Quit Murmur" items. FEATURES.md lists "Show Window, Toggle Overlay, About, Quit" — **Toggle Overlay and About may have been removed** from the tray menu.

### Agent 2 — Frontend Logic
1. **`hold-down-cancel` event handler**: Agent 2 mentions it as an event name but doesn't describe what happens in the frontend when it fires. Is the recording discarded? Does it call `cancel_native_recording`?
2. **App.tsx wiring not covered**: The dual-mode hook pattern (all hooks called, gated by `enabled`) is documented in ARCHITECTURE.md but not in Agent 2's findings since App.tsx wasn't in their scope.

### Agent 3 — Rust Commands
1. **Events emitted from keyboard.rs not listed**: Agent 3 correctly limited scope to commands/ files. Events like `hold-down-cancel`, `keyboard-listener-error`, `double-tap-toggle` are emitted from `keyboard.rs` (Agent 4's scope). The cross-agent coverage is complete but this could confuse readers of Agent 3's findings alone.
2. **`show-about` event not found**: This event is likely emitted from the tray menu setup in `lib.rs`, not from commands/. Nobody explicitly found where it's emitted.

### Agent 4 — Rust Core
1. **`hold-down-cancel` event missing**: Agent 4 describes the hold-down detector's behavior for short taps but doesn't explicitly list `hold-down-cancel` as an emitted event. Agent 2 found it on the frontend side. Need confirmation this event exists in keyboard.rs.
2. **logging.rs vs telemetry.rs**: The task scope included `logging.rs` but Agent 4 found `telemetry.rs` instead. Need to clarify: does `logging.rs` still exist as a separate file, or has it been completely replaced by `telemetry.rs` + tracing?
3. **`is_recording()` dead code**: Agent 4 notes `#[allow(dead_code)]` on `is_recording()` in audio.rs. Needs verification.

### Agent 5 — Config
1. **Default model discrepancy**: Agent 5 says `moonshine-tiny` is the default in settings.ts, but `DictationState::default()` in state.rs uses `base.en`. These are different defaults at different layers — the frontend default model (what new users see) may differ from the Rust default (fallback if no configure call).
2. **tauri.dev.conf.json not read**: Referenced but not examined. May contain important dev-mode overrides.
3. **entitlements.plist and Info.plist not read**: These define macOS permissions (microphone, accessibility) and are critical for distribution.

---

## Undocumented / New Findings

Features and behaviors found by agents that do NOT appear in FEATURES.md or ARCHITECTURE.md:

1. **VAD (Voice Activity Detection)** — Silero VAD v5.1.2 pre-filters silence. Configurable sensitivity. VAD model co-downloaded with transcription models. Fallback download if missing at transcription time. "downloading-vad" status. (Agents 3, 4)
2. **Structured event system** — Old logging.rs replaced by telemetry.rs using tracing. TauriEmitterLayer intercepts all tracing events → ring buffer → JSONL file → real-time frontend emission. Privacy stripping in release builds. (Agents 3, 4)
3. **Log viewer is now a separate window** with Events tab (stream/level filtering, colored chips, expandable JSON data rows) and Metrics tab (transcription timing charts with Total/Inference/VAD/Paste series). FEATURES.md describes a "modal with last 200 lines." (Agent 1)
4. **Metrics visualization** — SVG line charts for transcription timing, stat cards with trends (up/down/flat), last 20 transcriptions. Completely undocumented. (Agent 1)
5. **cancel_native_recording command** — Discards speculative recordings in "both" mode when a short tap is detected. (Agent 3)
6. **hold-down-cancel event** — Emitted when a hold press is too short (speculative recording). (Agent 2)
7. **Microphone selection setting** — User can choose specific audio input device. Not in FEATURES.md. (Agents 1, 2, 5)
8. **Auto-paste delay setting** — Configurable 10-500ms with UI slider. Not in FEATURES.md. (Agents 1, 2, 5)
9. **VAD sensitivity setting** — Configurable 0-100 with UI slider. Not in FEATURES.md. (Agents 1, 2, 5)
10. **Tray icon is static white** — FEATURES.md says color-coded (gray/red/amber). `update_tray_icon` is now a no-op. (Agents 3, 4)
11. **Close-to-hide for all 3 windows** — Main and log-viewer intercept close requests and hide. (Agents 3, 4)
12. **Privacy stripping** in release builds — All string fields stripped from pipeline events in JSONL/ring buffer. (Agents 3, 4)
13. **Locked mode on overlay** — Double-click to lock recording (persists across single clicks). (Agent 1)
14. **ModelDownloader shows 4 models** (subset of 7) on first launch. Curated selection. (Agent 1)
15. **Default model changed** — Settings.ts defaults to `moonshine-tiny` (FEATURES.md implies `base.en`). (Agent 5)
16. **`process_audio` base64 path** — Legacy command that accepts base64-encoded WAV. Doesn't emit `transcription-complete`. (Agents 3, 4)
17. **Screen change observer** — NSNotification observer for display changes, repositions overlay. (Agents 3, 4)
18. **Configurable paste delay with UI** — Slider only appears when auto-paste is enabled. (Agent 1)
19. **5 Tauri plugins**: opener, autostart, updater, notification, process. (Agents 4, 5)
20. **VAD model fallback download** — `ensure_vad_model` spawns background download if missing at transcription time. (Agent 3)

---

## Revision Round Results

Agents 2, 4, and 5 were re-spawned with targeted questions. Key findings:

### Resolved: `hold-down-cancel` is dead code
- **Agent 4 confirmed**: The string `hold-down-cancel` does NOT exist anywhere in Rust code. It is never emitted.
- **Agent 2 confirmed**: `useCombinedToggle.ts` listens for `hold-down-cancel` and calls `cancel_native_recording`, but the listener is dead code since the event is never fired.
- In hold-down-only mode, short taps go through the full start/stop cycle (the 0.3s minimum recording check handles them).
- In both mode, short taps that aren't promoted to holds simply emit nothing — the recording was never started.

### Resolved: logging.rs → telemetry.rs
- **Agent 4 confirmed**: `logging.rs` no longer exists. The file was entirely replaced by `telemetry.rs`. The CLAUDE.md file map reference to `logging.rs` is outdated. `commands/logging.rs` still exists as a thin command layer delegating to `telemetry.rs` functions.

### Resolved: WhisperState caching mechanism
- **Agent 4 confirmed**: `WhisperState` is stored in the `WhisperBackend` struct (`state: Option<WhisperState>`) and reused across all transcriptions. `create_state()` (which allocates GPU/Metal buffers) is called exactly once on first model load. Only a model change triggers recreation. This is the v0.7.8 optimization that eliminated per-transcription alloc/free cycles.

### Resolved: Default model precedence
- **Agent 5 confirmed**: `moonshine-tiny` (settings.ts) takes precedence over `base.en` (state.rs). The initialization flow calls `configure_dictation` with the frontend default before `initialized` becomes true. No recording can happen with the Rust-side default. The Rust default of `base.en` is effectively dead for the model name field.

### Resolved: tauri.dev.conf.json
- **Agent 5 confirmed**: Exists, overrides only `identifier` (→ `com.localdictation.dev`) and `productName` (→ `Local Dictation Dev`). Ensures dev build installs as a separate app from production.

### Resolved: show-about event
- **Agent 2 confirmed**: Consumed in `App.tsx` via `useShowAboutListener()` hook, which manages the `showAbout` state for the `AboutModal` component.

---

## Stale Documentation to Update

Based on the full crawl, these existing docs are inaccurate:

| Document | Issue |
|----------|-------|
| FEATURES.md | Tagged v0.4.0 — missing VAD, Moonshine, "both" mode, structured events, log viewer window, microphone selection, auto-paste delay, VAD sensitivity, metrics tab. Says 5-bar waveform (actually 7). Says tray icon is color-coded (actually static white). Says log viewer is a modal (now a window). Says 2 recording modes (now 3). Says 5 Whisper models (now 7 total across 2 backends). |
| CLAUDE.md file map | References `logging.rs` which no longer exists (now `telemetry.rs`). Missing `vad.rs`, `resource_monitor.rs`, `telemetry.rs` from the file map. Missing `useCombinedToggle`, `useAutoUpdater`, `useEventStore`, `useHistoryManagement`, `useInitialization`, `useShowAboutListener` from hooks. |
| CHANGELOG | Very stale — last entry is v0.2.0, [Unreleased] section. No entries for v0.3.0 through v0.8.0. |
| ARCHITECTURE.md | Mostly accurate but missing: VAD pipeline, structured event system (telemetry.rs), log viewer window architecture, metrics visualization, WhisperState caching, Moonshine download mechanism (tar.bz2 extraction), privacy stripping, resource monitor. The logging.rs section describes an old file-based system. |
