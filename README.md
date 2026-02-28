# Local Dictation

Privacy-first voice-to-text for macOS. Hold a key, speak, release — your words land in any app. No cloud, no subscriptions, no data leaves your machine.

Built with [Tauri 2](https://tauri.app/) (Rust + React), powered by [whisper.cpp](https://github.com/ggerganov/whisper.cpp) and [Moonshine](https://github.com/usefulsensors/moonshine) running locally with Metal GPU acceleration.

## Features

- **100% local** — all transcription runs on-device via Metal GPU. Zero network calls, works offline
- **Dual transcription backends** — swap between Whisper (GPU-accelerated) and Moonshine (lightweight ONNX) mid-session from a dropdown. 7 models from ~75 MB to ~3 GB
- **Two recording modes** — Hold Down (hold to record, release to stop) or Double-Tap (tap twice to start, tap once to stop)
- **Clipboard-first output** — text always copied to clipboard. Optional auto-paste into your focused app
- **Floating overlay** — always-on-top widget with animated waveform, click to toggle recording
- **In-app model downloader** — first-launch onboarding downloads your chosen model with progress bar
- **Transcription history** — timestamped entries with copy-to-clipboard
- **Stats tracking** — total words, average WPM, total recordings, approximate tokens
- **System tray** — color-coded status icon (idle / recording / processing)
- **Log viewer** — last 200 lines with color-coded levels, per-transcription timing
- **MIT licensed** — use it, fork it, build on it

## Installation

1. Download the latest `.dmg` from the [Releases](https://github.com/georgenijo/murmur-app/releases) page
2. Open the DMG and drag **Local Dictation** to your Applications folder
3. Launch the app — if no model is found, the onboarding screen will guide you through downloading one

### Permissions

Grant these in **System Settings > Privacy & Security** when prompted:

| Permission | Required for |
|------------|-------------|
| Microphone | Recording your voice |
| Accessibility | Keyboard detection + auto-paste |

## Models

Choose a model based on your speed/accuracy tradeoff. Models download automatically on first launch, or you can switch models in Settings at any time.

### Moonshine (ONNX)

| Model | Size | Speed | Notes |
|-------|------|-------|-------|
| Moonshine Tiny | ~124 MB | Fastest | Good for quick dictation |
| Moonshine Base | ~286 MB | Fast | Better accuracy |

### Whisper (Metal GPU)

| Model | Size | Speed | Notes |
|-------|------|-------|-------|
| tiny.en | ~75 MB | Fastest | Fair accuracy |
| base.en | ~142 MB | Fast | Good accuracy |
| small.en | ~488 MB | Medium | Better accuracy |
| medium.en | ~1.5 GB | Slow | Very good accuracy |
| large-v3-turbo | ~3 GB | Slowest | Best accuracy |

## Recording Modes

Configure in the Settings panel:

**Hold Down** — hold a modifier key (Shift, Option, or Control) to record. Release to stop and transcribe.

**Double-Tap** — quickly double-tap a modifier key to start recording. Single tap to stop. The detector rejects held keys, modifier+letter combos, slow taps, and triple-tap spam.

Transcribed text is always copied to your clipboard. Enable **Auto-Paste** in Settings to have it pasted automatically into your focused app.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| App framework | Tauri 2 |
| Backend | Rust |
| Frontend | React 18, TypeScript, Tailwind CSS 4 |
| Transcription | whisper-rs (whisper.cpp + Metal), sherpa-rs (Moonshine + ONNX) |
| Audio capture | cpal |
| Keyboard listener | rdev |
| Clipboard | arboard |
| Auto-paste | osascript |
| Build tool | Vite 6 |

## Building from Source

```bash
git clone https://github.com/georgenijo/murmur-app.git
cd murmur-app/app
npm install
npm run tauri dev        # Dev with hot reload
npm run tauri build      # Production .app and .dmg
```

Requires macOS 12+, [Node.js](https://nodejs.org/) 18+, and [Rust](https://rustup.rs/) (latest stable).

### Running Tests

```bash
cd app/src-tauri && cargo test -- --test-threads=1   # Rust unit tests
cd app && npx tsc --noEmit                            # TypeScript type check
```

## Architecture

```
Hotkey (rdev) → Audio Capture (cpal) → Transcription (whisper-rs / sherpa-rs) → Clipboard (arboard) → Auto-Paste (osascript)
       ↕                    ↕                        ↕                                    ↕
   Frontend (React) ←——— Tauri IPC ———→ Rust Backend ———→ System Tray + Overlay
```

The backend uses a `TranscriptionBackend` trait that both Whisper and Moonshine implement — a shared interface that lets you hot-swap engines at runtime. Adding a new backend is just implementing the same trait.

## License

[MIT](LICENSE)
