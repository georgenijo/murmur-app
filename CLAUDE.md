# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Local Dictation is a privacy-first macOS desktop app for voice-to-text. Built with Tauri 2 (Rust backend + React frontend), it captures audio via a global hotkey, transcribes locally using whisper.cpp (with Metal GPU acceleration), and copies results to the clipboard. All processing is entirely local — no cloud services.

## Build & Development Commands

All commands run from the `ui/` directory:

```bash
# Development (full Tauri app with hot reload)
cd ui && npm run tauri dev

# Frontend only (no Rust backend)
cd ui && npm run dev

# Production build (outputs .app and .dmg)
cd ui && npm run tauri build

# Install frontend dependencies
cd ui && npm install
```

There are no automated tests in this project currently.

## Architecture

### Data Flow

```
Hotkey press → cpal audio capture → f32 samples in memory →
hotkey release → resample to 16kHz mono → whisper-rs transcription →
clipboard copy (arboard) → optional osascript paste
```

### Rust Backend (`ui/src-tauri/src/`)

- **lib.rs** — Tauri command handlers (`start_native_recording`, `stop_native_recording`, `configure_dictation`, permission checks), tray icon setup, window management. Implements `MutexExt` trait for mutex poison recovery.
- **audio.rs** — Audio capture via cpal on a background thread with channel-based synchronization. Handles multi-channel to mono conversion and resampling to 16kHz.
- **transcriber.rs** — Whisper model loading and inference via whisper-rs. Searches multiple paths for model files (env var, Application Support, cache dirs).
- **injector.rs** — Clipboard write via arboard, optional paste simulation via osascript with 150ms delay.
- **state.rs** — `DictationState` (status, model, language, auto_paste) and `AppState` (mutex-wrapped state + whisper context).

### React Frontend (`ui/src/`)

- **App.tsx** — Main orchestrator: status management, hotkey registration (300ms debounce), recording timer, tab switching.
- **lib/dictation.ts** — Tauri invoke wrappers for all Rust commands.
- **lib/hotkey.ts** — Global shortcut registration/unregistration via `@tauri-apps/plugin-global-shortcut`.
- **lib/settings.ts** — localStorage-based settings persistence (model, hotkey, language, autoPaste).
- **lib/history.ts** — Transcription history in localStorage (max 50 entries).

### Key Design Patterns

- **Clipboard-first**: Text always goes to clipboard. Auto-paste via osascript is optional and requires Accessibility permission.
- **Lazy model loading**: Whisper context initialized on first transcription, not at startup.
- **Mutex poison recovery**: Custom `MutexExt` trait on `lib.rs` allows graceful recovery from panics instead of crashing.
- **Channel-based audio thread**: Audio recording thread signals readiness via channel to prevent race conditions.

## macOS Permissions

- **Microphone** (required): For audio capture via cpal.
- **Accessibility** (optional): Only needed for auto-paste feature (osascript keystroke simulation).

## Whisper Models

Models are ggml `.bin` files. The app searches these locations in order:
1. `$WHISPER_MODEL_DIR` env var
2. `~/Library/Application Support/local-dictation/models/`
3. `~/Library/Application Support/pywhispercpp/models/`
4. `~/.cache/whisper.cpp/`
5. `~/.cache/whisper/`
6. `~/.whisper/models/`

Available models: `tiny.en`, `base.en` (default), `small.en`, `medium.en`, `large-v3-turbo`.

## Known Cleanup Items

- **`rdev`** is listed in `Cargo.toml` but unused in source code (leftover from a previous auto-paste attempt).
- **`ui/src/lib/audioCapture.ts`** is a legacy Web Audio capture module from the Python sidecar era — not imported anywhere.
- **Git remote** is `murmur-app` (the original project name); local directory and app name use `local-dictation`.

## Key Dependencies

- **Rust**: tauri 2, whisper-rs (with Metal), cpal, arboard, hound
- **Frontend**: React 18, Tailwind CSS 4, @tauri-apps/api, @tauri-apps/plugin-global-shortcut, Vite 6, TypeScript
