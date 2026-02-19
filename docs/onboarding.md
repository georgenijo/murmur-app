# Onboarding

## Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) (latest stable)
- macOS (tested on 14+)
- A Whisper model file (see below)

## Setup

```bash
cd ui && npm install
```

## Development

```bash
# Full Tauri app with hot reload
cd ui && npm run tauri dev

# Frontend only (no Rust backend)
cd ui && npm run dev

# Production build (outputs .app and .dmg)
cd ui && npm run tauri build
```

## Tests

```bash
# Rust unit tests (single-threaded — timing-sensitive tests use sleep)
cd ui/src-tauri && cargo test -- --test-threads=1

# TypeScript type check
cd ui && npx tsc --noEmit
```

There are no frontend tests yet. Rust tests cover the double-tap detector state machine (23 tests in `keyboard.rs`).

## Whisper Models

The app requires a ggml `.bin` model file. Download one:

| Model | Size | Speed | Accuracy |
|-------|------|-------|----------|
| `tiny.en` | ~75 MB | Fastest | Basic |
| `base.en` | ~150 MB | Fast | Good |
| `small.en` | ~500 MB | Medium | Better |
| `medium.en` | ~1.5 GB | Slow | Great |
| `large-v3-turbo` | ~1.6 GB | Fast | Best (recommended) |

Download from: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model}.bin`

Install example (macOS):
```bash
mkdir -p ~/Library/Application\ Support/local-dictation/models
mv ~/Downloads/ggml-large-v3-turbo.bin ~/Library/Application\ Support/local-dictation/models/
```

The app searches these locations in order:
1. `$WHISPER_MODEL_DIR` env var
2. `~/Library/Application Support/local-dictation/models/`
3. `~/Library/Application Support/pywhispercpp/models/`
4. `~/.cache/whisper.cpp/`
5. `~/.cache/whisper/`
6. `~/.whisper/models/`

## macOS Permissions

| Permission | Required For | How to Grant |
|------------|-------------|--------------|
| **Microphone** | Audio capture (always required) | Prompted on first use |
| **Accessibility** | Double-tap recording mode + auto-paste | System Settings > Privacy & Security > Accessibility |

## Logs

App logs are at `~/Library/Application Support/local-dictation/logs/app.log` with automatic rotation at 5 MB.
