# Development Setup

## Prerequisites

- macOS 12+ (Apple Silicon recommended for Metal GPU)
- Node.js 18+
- Rust (install via rustup)

## Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/georgenijo/murmur-app.git
   cd murmur-app
   ```

2. Install Node dependencies:
   ```bash
   cd ui
   npm install
   ```

3. Download a Whisper model (if not already present):
   ```bash
   mkdir -p ~/Library/Application\ Support/local-dictation/models
   # Download from https://huggingface.co/ggerganov/whisper.cpp
   # Place ggml-base.en.bin (or other model) in the models folder
   ```

4. Run in development mode:
   ```bash
   cd ui
   npm run tauri dev
   ```

## macOS Permissions

Grant these permissions to your terminal app (e.g., Ghostty, iTerm, Terminal):

1. **System Settings → Privacy & Security → Microphone** - Add your terminal
2. **System Settings → Privacy & Security → Accessibility** - Add your terminal

After granting permissions, restart the dev server.

## Project Structure

```
murmur-app/
├── ui/                     # Tauri desktop app
│   ├── src/                # React frontend
│   │   ├── components/     # UI components
│   │   ├── lib/           # Utilities
│   │   └── App.tsx        # Main component
│   └── src-tauri/         # Rust backend
│       ├── src/
│       │   ├── lib.rs     # Tauri commands
│       │   ├── audio.rs   # Audio capture
│       │   ├── transcriber.rs
│       │   ├── injector.rs
│       │   └── state.rs
│       └── Cargo.toml
└── docs/                   # Documentation
```

## Building for Production

```bash
cd ui
npm run tauri build
```

Output:
- `ui/src-tauri/target/release/bundle/macos/Local Dictation.app`
- `ui/src-tauri/target/release/bundle/dmg/Local Dictation_x.x.x_aarch64.dmg`

## Viewing Logs

During development, check the terminal running `npm run tauri dev` for Rust println! output.

## Common Issues

### Permission Popup Every Rebuild
Dev builds have different signatures each time. Grant Accessibility permission to your terminal app instead of the dev build.

### "No input device available"
Grant Microphone permission to your terminal app, then restart the dev server.

### Model Not Found
Ensure a Whisper model exists in one of these locations (checked in order):
- `WHISPER_MODEL_DIR` environment variable (recommended)
- `~/Library/Application Support/local-dictation/models/`
- `~/Library/Application Support/pywhispercpp/models/`
- `~/.cache/whisper.cpp/`
- `~/.cache/whisper/`
- `~/.whisper/models/`
