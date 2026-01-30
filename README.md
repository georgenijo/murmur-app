# Local Voice Dictation

A privacy-first voice-to-text tool for macOS, inspired by Wispr Flow. All processing happens locally - no data leaves your machine.

## Features

- 100% local processing - no cloud APIs, no data collection
- Whisper-powered transcription (OpenAI's open-source model)
- Optional LLM cleanup via Ollama (removes filler words, fixes grammar)
- Global hotkey activation (hold to record, release to transcribe)
- Works with any application - text is pasted into the focused app

---

## Desktop App (Recommended)

Local Dictation now includes a native macOS desktop app with a modern UI, system tray integration, and easy configuration.

### App Overview

The desktop app provides a polished user experience with:

- **System Tray Integration** - Lives in your menubar for quick access
- **Visual Recording Indicator** - See when you're recording with a duration timer
- **Settings Panel** - Configure hotkeys and Whisper models without command line
- **Transcription History** - View and copy past transcriptions
- **One-Click Installation** - No Python environment setup required

<!-- TODO: Add screenshot -->
![App Screenshot](docs/images/app-screenshot.png)

### Installation

#### Option 1: Download DMG (Recommended)

1. Download the latest `.dmg` file from the [Releases](https://github.com/yourusername/local-dictation/releases) page
2. Open the DMG and drag **Local Dictation** to your Applications folder
3. Launch the app and grant the required permissions when prompted

#### Option 2: Build from Source

See [Building from Source](#building-the-desktop-app-from-source) below.

### Usage Guide

#### Starting the App

Launch **Local Dictation** from your Applications folder. The app will appear in your menubar.

<!-- TODO: Add menubar screenshot -->
![Menubar Icon](docs/images/menubar-icon.png)

#### Recording

1. Press and hold your configured hotkey (default: **Shift+Space**)
2. Speak your text
3. Release the hotkey
4. The transcribed text will be pasted into your focused application

#### Hotkey Options

Configure your preferred hotkey in Settings:

| Hotkey | Description |
|--------|-------------|
| **Shift+Space** | Default, easy to reach |
| **Option+Space** | Alternative for Shift conflicts |
| **Control+Space** | Alternative option |

#### Settings Panel

Click the menubar icon and select **Settings** to configure:

- **Whisper Model** - Choose transcription accuracy vs speed
- **Hotkey** - Select your preferred activation key
- **Recording Duration** - View current recording length

<!-- TODO: Add settings screenshot -->
![Settings Panel](docs/images/settings-panel.png)

#### Transcription History

View your recent transcriptions by clicking the menubar icon. Each entry shows:

- Transcribed text
- Timestamp
- One-click copy to clipboard

<!-- TODO: Add history screenshot -->
![History Panel](docs/images/history-panel.png)

### Download Whisper Model

The app requires a Whisper model file for transcription. Download one of these models:

| Model | Size | Speed | Accuracy | Download |
|-------|------|-------|----------|----------|
| `large-v3-turbo` | 1.6GB | Fast | Best | [Download](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin) |
| `base.en` | 142MB | Fastest | Good | [Download](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin) |
| `small.en` | 466MB | Medium | Better | [Download](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin) |

**Installation:**

```bash
# Create the models directory
mkdir -p ~/Library/Application\ Support/local-dictation/models

# Move your downloaded model (example for large-v3-turbo)
mv ~/Downloads/ggml-large-v3-turbo.bin ~/Library/Application\ Support/local-dictation/models/
```

The app searches for models in these locations:
- `~/Library/Application Support/local-dictation/models/`
- `~/.cache/whisper.cpp/`
- Custom path via `WHISPER_MODEL_DIR` environment variable

### macOS Permissions

The app requires the following permissions (you'll be prompted on first launch):

| Permission | Why |
|------------|-----|
| **Microphone** | To record your voice |
| **Accessibility** | To paste text into apps |

Go to **System Settings → Privacy & Security** to grant permissions if needed.

### Building the Desktop App from Source

#### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) (latest stable)
- [pnpm](https://pnpm.io/) package manager

#### Build Steps

```bash
# Clone the repository
git clone https://github.com/yourusername/local-dictation.git
cd local-dictation/ui

# Install dependencies
pnpm install

# Development mode (with hot reload)
pnpm tauri dev

# Production build
pnpm tauri build
```

The built app will be in `ui/src-tauri/target/release/bundle/dmg/`.

#### Project Structure (UI)

```
ui/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── hooks/              # Custom React hooks
│   └── App.tsx             # Main app component
├── src-tauri/              # Tauri backend (Rust)
│   ├── src/                # Rust source code
│   ├── sidecar/            # Python transcription sidecar
│   └── tauri.conf.json     # Tauri configuration
└── package.json
```

---

## Requirements

- macOS (tested on macOS 14+)
- Python 3.11+
- Homebrew
- ~3GB disk space (for models)

## Installation

### 1. Install System Dependencies

```bash
brew install python@3.11 ffmpeg portaudio
```

### 2. Install Ollama (for LLM cleanup)

```bash
brew install ollama
brew services start ollama
ollama pull llama3.2:3b
```

### 3. Set Up Python Environment

```bash
cd /path/to/local-dictation
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
```

### 4. Grant macOS Permissions

The app needs three permissions to function. Go to **System Settings → Privacy & Security**:

| Permission | Why |
|------------|-----|
| **Microphone** | To record your voice |
| **Accessibility** | To paste text into apps |
| **Input Monitoring** | To detect the global hotkey |

Add **Terminal** (or your IDE) to each of these lists.

> You may need to restart Terminal after granting permissions.

## Usage

### Basic Usage

```bash
source venv/bin/activate
python main.py
```

Then:
1. Hold the **Right Option** key
2. Speak
3. Release the key
4. Text appears in your focused app

### Command Line Options

```
python main.py [OPTIONS]

Options:
  --hotkey KEY        Hotkey to trigger recording (default: alt_r)
  --no-cleanup        Disable LLM cleanup (use raw transcription)
  --model MODEL       Ollama model for cleanup (default: llama3.2:3b)
  --whisper-model M   Whisper model: tiny.en, base.en, small.en, turbo
  --type              Use typing instead of clipboard paste
```

### Examples

```bash
# Use raw transcription (no LLM cleanup)
python main.py --no-cleanup

# Use a different hotkey (left option key)
python main.py --hotkey alt_l

# Use the turbo Whisper model for better accuracy
python main.py --whisper-model turbo

# Use typing instead of paste (slower but doesn't touch clipboard)
python main.py --type
```

### Available Hotkeys

- `alt_r` / `option_r` - Right Option key (default)
- `alt_l` / `option_l` - Left Option key
- `ctrl_r` / `ctrl_l` - Control keys
- `cmd_r` / `cmd_l` - Command keys
- `f13`, `f14`, `f15` - Function keys (if available)
- `caps_lock` - Caps Lock key

## Whisper Models

| Model | Size | Speed | Accuracy |
|-------|------|-------|----------|
| `tiny.en` | 39MB | Fastest | Basic |
| `base.en` | 74MB | Fast | Good (default) |
| `small.en` | 244MB | Medium | Better |
| `turbo` | 809MB | Fast | Best |

Models download automatically on first use to `~/.cache/whisper/`.

## Troubleshooting

### "Permission denied" or hotkey not working

- Ensure Terminal has Accessibility and Input Monitoring permissions
- Restart Terminal after granting permissions
- Try running with `sudo` if issues persist

### Ollama not available

If you see "Ollama: Not available", the app will still work but without text cleanup.

```bash
# Check if Ollama is running
curl http://localhost:11434/api/tags

# Start Ollama if needed
brew services start ollama
```

### No audio recorded

- Check Microphone permission in System Settings
- Verify your microphone is working: `python audio_recorder.py`

### Slow transcription

- Use a smaller Whisper model: `--whisper-model tiny.en`
- The first transcription is slower (model loading)

### Text not pasting

- Check Accessibility permission
- Try `--type` flag to use keystroke simulation instead

## Project Structure

```
local-dictation/
├── main.py              # Main application
├── audio_recorder.py    # Microphone recording
├── transcriber.py       # Whisper transcription
├── llm_cleanup.py       # Ollama text cleanup
├── text_injector.py     # Paste/type into apps
├── hotkey_listener.py   # Global hotkey detection
├── requirements.txt     # Python dependencies
└── README.md
```

## License

MIT
