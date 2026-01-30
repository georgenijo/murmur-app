# Architecture

## Overview

Local Dictation is a macOS desktop app for voice-to-text transcription using whisper.cpp.

## Components

### Tauri Desktop App (ui/)
- React/TypeScript frontend
- Rust backend with system tray and global hotkey
- Communicates with Python sidecar via JSON over stdin/stdout

### Python Sidecar (root level)
These files MUST remain at the project root (imported by dictation_bridge.py):
- `dictation_bridge.py` - Entry point, JSON protocol handler
- `audio_recorder.py` - Microphone recording with noise reduction
- `text_injector.py` - Clipboard paste functionality

## Communication Protocol

Tauri spawns Python and communicates via JSON lines:

Commands: `start_recording`, `stop_recording`, `get_status`, `configure`, `shutdown`

Example:
```json
{"cmd": "start_recording"}
{"type": "recording_started"}
{"type": "transcription", "text": "Hello world"}
```

## Build Requirements

- Node.js 18+
- Rust (via rustup)
- Python 3.11+ with venv at `./venv`
