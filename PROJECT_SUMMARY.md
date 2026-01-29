# Local Voice Dictation - Project Summary

## What We Built
A privacy-first voice dictation tool for macOS, inspired by Wispr Flow. Hold a key, speak, release - text appears wherever your cursor is.

## Session Timeline

### Phase 1: Core Setup (Tickets 001-004)
- Set up Python 3.11 virtual environment
- Installed Whisper (OpenAI's speech-to-text)
- Installed Ollama with Llama 3.2 3B for text cleanup
- Created `requirements.txt`

### Phase 2: Module Development (Tickets 005-009)
Built 5 modules in parallel:
- `audio_recorder.py` - Records from microphone at 16kHz mono, includes noise reduction
- `transcriber.py` - OpenAI Whisper transcription (Python/PyTorch)
- `llm_cleanup.py` - Optional text cleanup via Ollama
- `text_injector.py` - Pastes text via clipboard + Cmd+V
- `hotkey_listener.py` - Global hotkey detection (default: Left Shift)

### Phase 3: Main Application (Ticket 010)
- Created `main.py` - orchestrates all modules
- Added command-line arguments for configuration
- Added resource monitoring (CPU, RAM, timing stats)
- Added JSON logging to `dictation_log.jsonl`

### Phase 4: Optimizations
1. **whisper.cpp backend** - Added faster C++ implementation
   - Created `transcriber_cpp.py`
   - 2-4x faster than Python Whisper
   - Uses quantized models (smaller, faster)

2. **Deepgram API backend** - Added cloud transcription option
   - Created `transcriber_deepgram.py`
   - 200 hours free tier
   - Fastest option (no local processing)
   - API key stored in `.env` file

## Current Architecture

```
[Hold Left Shift]
       ↓
   Record Audio (16kHz mono WAV)
       ↓
   Noise Reduction
       ↓
┌──────────────────────────────────────────────┐
│         TRANSCRIPTION BACKEND                │
│                                              │
│  --backend openai   → Python Whisper         │
│  --backend cpp      → whisper.cpp (faster)   │
│  --backend deepgram → Deepgram API (cloud)   │
└──────────────────────────────────────────────┘
       ↓
   [Optional: --cleanup → Ollama LLM]
       ↓
   Paste to focused app
```

## File Structure

```
local-dictation/
├── main.py                 # Main application entry point
├── audio_recorder.py       # Microphone recording + noise reduction
├── transcriber.py          # OpenAI Whisper backend
├── transcriber_cpp.py      # whisper.cpp backend (faster)
├── transcriber_deepgram.py # Deepgram API backend (cloud)
├── llm_cleanup.py          # Ollama text cleanup (optional)
├── text_injector.py        # Clipboard paste
├── hotkey_listener.py      # Global hotkey detection
├── requirements.txt        # Python dependencies
├── .env                    # API keys (git ignored)
├── .gitignore              # Git ignore rules
├── dictation_log.jsonl     # Transcription logs
└── README.md               # User documentation
```

## Usage

### Basic (Local - OpenAI Whisper Turbo)
```bash
source venv/bin/activate
python main.py
```

### Faster Local (whisper.cpp)
```bash
python main.py --cpp
```

### Cloud (Deepgram - 200hrs free)
```bash
python main.py --deepgram
```

### With LLM Cleanup
```bash
python main.py --cleanup
```

### All Options
```bash
python main.py \
  --backend openai|cpp|deepgram \
  --whisper-model tiny.en|base.en|small.en|turbo|large-v3 \
  --cleanup \
  --hotkey shift_l|alt_r|f13|etc
```

## Performance Comparison (observed)

| Backend | Model | Transcribe Time | RAM | Accuracy |
|---------|-------|-----------------|-----|----------|
| openai | base.en | ~0.3s | ~800MB | Good |
| openai | turbo | ~2.5s | ~3.6GB | Excellent |
| cpp | base.en | ~0.2s | ~500MB | Good |
| deepgram | nova-2 | ~0.5s* | ~0MB | Excellent |

*Plus network latency

## Key Learnings

1. **Whisper turbo** is 10x slower than base.en but much more accurate
2. **whisper.cpp** is the same model as OpenAI Whisper, just faster C++ code
3. **LLM cleanup** wasn't very useful - Whisper turbo already outputs clean text
4. **Repetitive words** can cause Whisper to hang (known bug)
5. **Deepgram free tier** (200 hrs) is generous for personal use

## Configuration Files

### .env
```
DEEPGRAM_API_KEY=your-key-here
```

### Logging
All transcriptions logged to `dictation_log.jsonl`:
```json
{
  "timestamp": "2026-01-29T14:35:22",
  "raw_text": "...",
  "cleaned_text": "...",
  "whisper_time_s": 2.53,
  "total_time_s": 2.79,
  "gpu": "MPS (Apple GPU)"
}
```

## macOS Permissions Required
- System Settings → Privacy & Security → **Microphone** → Terminal
- System Settings → Privacy & Security → **Accessibility** → Terminal
- System Settings → Privacy & Security → **Input Monitoring** → Terminal

## Dependencies
- Python 3.11+
- openai-whisper
- pywhispercpp
- deepgram-sdk
- sounddevice
- pynput
- pyperclip
- noisereduce
- psutil
- python-dotenv

## Known Issues
1. **Deepgram API flaky** - Sometimes times out or returns empty. Network dependent.
2. **Repetitive words hang Whisper** - Saying "stop stop stop" can cause 14s+ delays
3. **LLM cleanup not useful** - Often doesn't remove filler words, sometimes refuses content

## Latest Eval Results (whisper.cpp)
```
Model        Init       Transcribe   Total
tiny.en      ~0.05s     ~0.10s       ~0.15s   (fastest, less accurate)
base.en      ~0.08s     ~0.18s       ~0.26s   (best balance)
small.en     ~0.15s     ~0.40s       ~0.55s   (more accurate)
```

**Recommendation:** Use `python main.py --cpp --whisper-model base.en` for best speed/accuracy balance.

## Code Patterns

All transcribers follow the same interface:
```python
class SomeTranscriber:
    def __init__(self, model_name: str = "default"):
        self.model_name = model_name
        self._load_model()

    def _load_model(self):
        # Load model into memory
        pass

    def transcribe(self, audio_path: str, language: str = "en") -> str:
        # Return transcribed text, empty string on error
        pass
```

To add a new backend:
1. Create `transcriber_newbackend.py` following the pattern above
2. Add to `main.py` backend selection (around line 54)
3. Add CLI flag in argparse section

## Future Ideas (not implemented)
- [ ] Code mode (convert speech to camelCase/code syntax)
- [ ] Streaming transcription (start transcribing while speaking)
- [ ] Menubar app / standalone executable
- [ ] Auto-timeout to save RAM when idle
- [ ] Native Fn key support via IOKit
- [ ] Fix Deepgram reliability or remove it
