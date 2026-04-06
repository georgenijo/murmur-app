# CLAUDE.md

Privacy-first macOS voice-to-text app. Tauri 2 (Rust + React), local whisper.cpp transcription with Metal GPU, clipboard-first output. No cloud services.

## Commands

```bash
cd app && npm run tauri dev        # Dev with hot reload
cd app && npm run tauri build      # Production .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1  # Rust unit tests
cd app && npx tsc --noEmit         # TypeScript check
```

## Docs

Read these before working on a feature:

- **[docs/onboarding.md](docs/onboarding.md)** — Setup, permissions, model installation, logs
- **[docs/install-linux.md](docs/install-linux.md)** — Linux end-user install (deb / rpm / AppImage), runtime deps, Wayland notes
- **[docs/features/recording-modes.md](docs/features/recording-modes.md)** — Hold-down and double-tap modes, state machine, rdev threading
- **[docs/features/transcription.md](docs/features/transcription.md)** — Audio capture, whisper pipeline, status flow
- **[docs/features/text-injection.md](docs/features/text-injection.md)** — Clipboard, auto-paste, osascript
- **[docs/features/vad.md](docs/features/vad.md)** — VAD speech filtering
- **[docs/features/overlay.md](docs/features/overlay.md)** — Dynamic Island overlay
- **[docs/features/log-viewer.md](docs/features/log-viewer.md)** — Structured event system and log viewer
- **[docs/features/auto-updater.md](docs/features/auto-updater.md)** — Auto-update system
- **[docs/features/models.md](docs/features/models.md)** — Model management and download

## File Map

### Rust (`app/src-tauri/src/`)

| File | Purpose |
|------|---------|
| `lib.rs` | App wiring: mod declarations, `State`, `MutexExt`, `run()` |
| `commands/mod.rs` | Re-exports command sub-modules |
| `commands/recording.rs` | `IdleGuard`, transcription pipeline with VAD, 7 recording/status commands |
| `commands/permissions.rs` | 6 permission and audio device commands |
| `commands/keyboard.rs` | 4 keyboard listener commands |
| `commands/logging.rs` | 4 logging commands, delegates to telemetry.rs |
| `commands/models.rs` | Model download pipeline and existence checks |
| `commands/tray.rs` | Tray icon rendering (`make_tray_icon_data`, `update_tray_icon`) |
| `commands/overlay.rs` | Notch detection, overlay positioning, show/hide commands |
| `keyboard.rs` | Hold-down and double-tap detectors, shared rdev listener thread |
| `audio.rs` | cpal capture, mono conversion, 16kHz resampling |
| `transcriber/` | whisper-rs model loading and inference |
| `injector.rs` | Clipboard (arboard) + auto-paste (osascript) |
| `state.rs` | `DictationState`, `AppState` with mutex-wrapped state |
| `telemetry.rs` | Structured event system: TauriEmitterLayer, ring buffer, JSONL, privacy stripping |
| `vad.rs` | Silero VAD speech filtering via whisper-rs |
| `resource_monitor.rs` | System CPU/memory monitoring via sysinfo |

### Frontend (`app/src/`)

| File | Purpose |
|------|---------|
| `App.tsx` | Main orchestrator, wires hooks together |
| `lib/settings.ts` | Settings types, defaults, localStorage persistence |
| `lib/events.ts` | Event types, stream/level definitions, color constants |
| `lib/history.ts` | History entry types and localStorage persistence |
| `lib/stats.ts` | Usage metrics: words, WPM, recordings, tokens |
| `lib/dictation.ts` | Tauri command wrappers for dictation pipeline |
| `lib/updater.ts` | Semver parsing, min-version checking, update utilities |
| `lib/log.ts` | Frontend logging via Rust tracing (flog utility) |
| `lib/hooks/useHoldDownToggle.ts` | Hold-down mode (rdev press/release events) |
| `lib/hooks/useDoubleTapToggle.ts` | Double-tap mode (rdev events) |
| `lib/hooks/useCombinedToggle.ts` | Both mode (hold-down + double-tap simultaneous) |
| `lib/hooks/useRecordingState.ts` | Recording status, transcription, toggle logic |
| `lib/hooks/useAutoUpdater.ts` | OTA updates, min-version enforcement |
| `lib/hooks/useHistoryManagement.ts` | Transcription history with localStorage persistence |
| `lib/hooks/useInitialization.ts` | One-time init sequence (initDictation + configure) |
| `lib/hooks/useShowAboutListener.ts` | Listens for show-about tray event |
| `lib/hooks/useEventStore.ts` | Structured event log buffer with live streaming |
| `lib/hooks/useResourceMonitor.ts` | CPU/memory polling with rolling buffer |
| `components/settings/SettingsPanel.tsx` | Settings UI with mode switching |
| `components/log-viewer/LogViewerApp.tsx` | Structured event viewer with Events + Metrics tabs |
| `components/OverlayWidget.tsx` | Dynamic Island notch overlay |

## Key Patterns

- **Dual recording modes**: Both hooks always called (Rules of Hooks), gated by `enabled` prop
- **Clipboard-first**: Text always goes to clipboard; auto-paste is optional
- **Lazy model loading**: Whisper context created on first transcription
- **Mutex poison recovery**: `MutexExt` trait recovers from panics
- **rdev thread safety**: `set_is_main_thread(false)` before `listen()` — prevents macOS TIS/TSM segfault

## MCP Tools

- **Playwright** (`@playwright/mcp`): Browser automation for UI work. When making frontend/UI changes, use `browser_navigate` to `http://localhost:1420` and `browser_take_screenshot` to visually verify your changes. Requires `npm run tauri dev` to be running. Screenshots return inline as images — evaluate them and iterate until the UI looks right.

## Dependencies

- **Rust**: tauri 2, whisper-rs (Metal), cpal, arboard, hound, rdev (git main branch)
- **Frontend**: React 18, Tailwind CSS 4, @tauri-apps/api, Vite 6, TypeScript
