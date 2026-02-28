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
- **[docs/features/recording-modes.md](docs/features/recording-modes.md)** — Hold-down and double-tap modes, state machine, rdev threading
- **[docs/features/transcription.md](docs/features/transcription.md)** — Audio capture, whisper pipeline, status flow
- **[docs/features/text-injection.md](docs/features/text-injection.md)** — Clipboard, auto-paste, osascript

## File Map

### Rust (`app/src-tauri/src/`)

| File | Purpose |
|------|---------|
| `lib.rs` | App wiring: mod declarations, `State`, `MutexExt`, `run()` |
| `tray.rs` | Tray icon rendering (`make_tray_icon_data`, `update_tray_icon`) |
| `overlay.rs` | Notch detection, overlay positioning, show/hide commands |
| `commands/mod.rs` | Re-exports command sub-modules |
| `commands/recording.rs` | `IdleGuard`, transcription pipeline, 7 recording/status commands |
| `commands/permissions.rs` | 6 permission and audio device commands |
| `commands/keyboard.rs` | 4 keyboard listener commands |
| `commands/logging.rs` | 3 logging commands |
| `commands/models.rs` | Model download pipeline and existence checks |
| `keyboard.rs` | Hold-down and double-tap detectors, shared rdev listener thread |
| `audio.rs` | cpal capture, mono conversion, 16kHz resampling |
| `transcriber/` | whisper-rs and moonshine model loading and inference |
| `injector.rs` | Clipboard (arboard) + auto-paste (osascript) |
| `state.rs` | `DictationState`, `AppState` with mutex-wrapped state |
| `logging.rs` | File-based logging with rotation |

### Frontend (`app/src/`)

| File | Purpose |
|------|---------|
| `App.tsx` | Main orchestrator, wires hooks together |
| `lib/settings.ts` | Settings types, defaults, localStorage persistence |
| `lib/hooks/useHoldDownToggle.ts` | Hold-down mode (rdev press/release events) |
| `lib/hooks/useDoubleTapToggle.ts` | Double-tap mode (rdev events) |
| `lib/hooks/useRecordingState.ts` | Recording status, transcription, toggle logic |
| `components/settings/SettingsPanel.tsx` | Settings UI with mode switching |

## Key Patterns

- **Dual recording modes**: Both hooks always called (Rules of Hooks), gated by `enabled` prop
- **Clipboard-first**: Text always goes to clipboard; auto-paste is optional
- **Lazy model loading**: Whisper context created on first transcription
- **Mutex poison recovery**: `MutexExt` trait recovers from panics
- **rdev thread safety**: `set_is_main_thread(false)` before `listen()` — prevents macOS TIS/TSM segfault

## Dependencies

- **Rust**: tauri 2, whisper-rs (Metal), cpal, arboard, hound, rdev (git main branch)
- **Frontend**: React 18, Tailwind CSS 4, @tauri-apps/api, Vite 6, TypeScript
