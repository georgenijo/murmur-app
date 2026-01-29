# Local Voice Dictation

A privacy-first voice-to-text tool for macOS, inspired by Wispr Flow. All processing happens locally - no data leaves your machine.

## Features

- 100% local processing - no cloud APIs, no data collection
- Whisper-powered transcription (OpenAI's open-source model)
- Optional LLM cleanup via Ollama (removes filler words, fixes grammar)
- Global hotkey activation (hold to record, release to transcribe)
- Works with any application - text is pasted into the focused app

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
