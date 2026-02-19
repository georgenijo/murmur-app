# Local Dictation

A privacy-first voice-to-text app for macOS. Speak, and your words appear in any app — no cloud, no internet, everything processed on-device.

Built with Tauri 2 + Rust, powered by [whisper.cpp](https://github.com/ggerganov/whisper.cpp) with Metal GPU acceleration.

---

## Installation

1. Download the latest `.dmg` from the [Releases](https://github.com/georgenijo/murmur-app/releases) page
2. Open the DMG and drag **Local Dictation** to your Applications folder
3. Launch the app and grant permissions when prompted

---

## Whisper Model Setup

The app requires a Whisper model file. Download one and place it in `~/Library/Application Support/local-dictation/models/`:

```bash
mkdir -p ~/Library/Application\ Support/local-dictation/models
mv ~/Downloads/ggml-large-v3-turbo.bin ~/Library/Application\ Support/local-dictation/models/
```

| Model | Size | Notes |
|-------|------|-------|
| [large-v3-turbo](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin) | 1.6 GB | Best accuracy |
| [small.en](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin) | 466 MB | Good balance |
| [base.en](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin) | 142 MB | Fastest |

---

## Permissions

Grant these in **System Settings → Privacy & Security**:

| Permission | Required for |
|------------|-------------|
| Microphone | Recording your voice |
| Accessibility | Double-tap mode |

---

## Recording Modes

Configure in the Settings panel. Two modes available:

**Key Combo** — press a hotkey to start, press again to stop. Customize the key combo freely (e.g. `Alt+D`, `Ctrl+Shift+R`). A modifier key is required.

**Double-Tap** — quickly double-tap Shift, Option, or Control to start recording; single tap to stop. Requires Accessibility permission.

Transcribed text is always copied to your clipboard. Enable **Auto-Paste** in Settings to have it pasted automatically into your focused app.

---

## Building from Source

```bash
git clone https://github.com/georgenijo/murmur-app.git
cd murmur-app/ui
npm install
npm run tauri build
```

Requires [Node.js](https://nodejs.org/) 18+ and [Rust](https://rustup.rs/) (latest stable).

---

## License

MIT
