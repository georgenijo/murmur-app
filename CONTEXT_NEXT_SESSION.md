# Next Session Context - Tauri UI App

## Quick Start Prompt
```
Read these files to understand the project:
1. PROJECT_SUMMARY.md - What the dictation tool does
2. CONTEXT_NEXT_SESSION.md - This file, current state and goals
3. TICKETS_UI.md - Tasks for building the Tauri UI

Then start working on the tickets in order.
```

## Current State

### What Works (CLI)
- **Hold Left Shift → Speak → Release → Text appears at cursor**
- All transcription backends working:
  - `openai` - Python Whisper
  - `cpp` - whisper.cpp (fastest)
  - `deepgram` - Cloud API
- Shell aliases configured: `d` (accurate) and `df` (fast)

### Best Model (from benchmarks)
```
cpp / large-v3-turbo: 1.12s, 100% accuracy
cpp / small.en: 0.46s, 95.2% accuracy (fast alternative)
```

### File Structure
```
local-dictation/
├── main.py                 # CLI entry point - orchestrates everything
├── audio_recorder.py       # Records from mic at 16kHz mono
├── transcriber.py          # OpenAI Whisper backend
├── transcriber_cpp.py      # whisper.cpp backend (USE THIS)
├── transcriber_deepgram.py # Deepgram API backend
├── llm_cleanup.py          # Optional Ollama cleanup (not useful)
├── text_injector.py        # Pastes via clipboard + Cmd+V
├── hotkey_listener.py      # Global hotkey detection
├── requirements.txt        # Python dependencies
├── tests/                  # Benchmark tests with accuracy
│   ├── test_openai.py
│   ├── test_cpp.py
│   ├── test_deepgram.py
│   ├── accuracy.py
│   └── run_all.py
├── PROJECT_SUMMARY.md      # Full project documentation
├── CONTEXT_NEXT_SESSION.md # This file
└── TICKETS_UI.md           # UI development tasks
```

## What We're Building Next

### Goal
Convert CLI tool into a **Tauri desktop app** with:
- Menubar presence (mic icon shows recording status)
- Settings UI (model selection, hotkey config)
- Transcription history
- Bundled as `Dictation.app`

### Tech Stack
- **Tauri 2** - Rust-based app framework (~5MB bundle)
- **React + TypeScript** - Frontend UI
- **Python backend** - Keep existing transcription code
- Communication: Tauri commands call Python subprocess or sidecar

### Architecture
```
┌─────────────────────────────────────────┐
│           Tauri App (Rust)              │
│  ┌───────────────────────────────────┐  │
│  │      React UI (TypeScript)        │  │
│  │  - Settings panel                 │  │
│  │  - Status indicator               │  │
│  │  - Transcription history          │  │
│  └───────────────────────────────────┘  │
│                   │                     │
│           Tauri Commands                │
│                   │                     │
│  ┌───────────────────────────────────┐  │
│  │     Python Sidecar/Subprocess     │  │
│  │  - audio_recorder.py              │  │
│  │  - transcriber_cpp.py             │  │
│  │  - hotkey_listener.py             │  │
│  │  - text_injector.py               │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

### Key Decisions Made
1. **Tauri over Electron** - Lighter, faster (Wispr Flow uses Electron and is 800MB)
2. **Keep Python backend** - Transcription code already works, no rewrite needed
3. **cpp/large-v3-turbo as default** - 100% accuracy, 1.1s transcribe time
4. **Local-first** - No cloud dependency (unlike Wispr Flow)

## Model Storage Locations
```
~/.cache/whisper/                              # OpenAI Python models (2.2 GB)
~/Library/Application Support/pywhispercpp/   # whisper.cpp models (6.5 GB)
```

## Dependencies
```
# Python (in venv)
openai-whisper, pywhispercpp, sounddevice, pynput, pyperclip, noisereduce, psutil, jiwer

# For Tauri UI (to install)
Node.js, npm, Rust, Tauri CLI
```

## macOS Permissions Required
- Microphone access
- Accessibility (for hotkey + text injection)
- Input Monitoring
