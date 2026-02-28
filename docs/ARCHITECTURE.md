# Architecture

## Overview

Local Dictation is a cross-platform desktop app for voice-to-text transcription using whisper.cpp, built entirely in Rust with a React frontend. Metal GPU acceleration is available on macOS.

## Tech Stack

- **Frontend**: React + TypeScript + Vite + Tailwind CSS
- **Backend**: Pure Rust (Tauri 2)
- **Transcription**: whisper-rs (whisper.cpp with Metal GPU acceleration)
- **Audio Capture**: cpal (native audio)
- **Clipboard**: arboard

## Components

### Tauri Desktop App (app/)

```text
app/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── lib/               # Utilities (dictation.ts)
│   └── App.tsx            # Main app component
└── src-tauri/
    └── src/
        ├── lib.rs         # Tauri commands, app setup, tray
        ├── audio.rs       # Native audio capture with cpal
        ├── transcriber.rs # Whisper model loading & transcription
        ├── injector.rs    # Clipboard + keyboard simulation
        └── state.rs       # App state, constants
```

## Data Flow

```text
Hotkey pressed
    ↓
cpal captures audio from microphone
    ↓
Hotkey released
    ↓
Audio resampled to 16kHz mono
    ↓
whisper-rs transcribes (Metal GPU)
    ↓
arboard copies text to clipboard
    ↓
User pastes with Cmd+V (manual)
```

## Key Design Decisions

1. **Pure Rust Backend** - No Python subprocess, faster startup, smaller bundle
2. **Native Audio Capture** - Uses cpal instead of WebView's navigator.mediaDevices
3. **Channel Synchronization** - Audio thread signals readiness via channel (no race conditions)
4. **Mutex Poison Recovery** - App recovers gracefully from panics
5. **Clipboard-Only Injection** - Text copied to clipboard for user to paste manually

## Permissions Required (macOS)

| Permission    | Purpose             | Settings Location                    |
|---------------|---------------------|--------------------------------------|
| Microphone    | Audio capture       | Privacy & Security → Microphone      |

## Build Outputs

```bash
npm run tauri build
```

Produces:
- `target/release/bundle/macos/Local Dictation.app`
- `target/release/bundle/dmg/Local Dictation_x.x.x_aarch64.dmg`
