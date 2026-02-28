# Onboarding

## Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) (latest stable)
- macOS (tested on 14+)
- A Whisper model file (see below)

## Setup

```bash
cd app && npm install
```

## Development

```bash
# Full Tauri app with hot reload
cd app && npm run tauri dev

# Frontend only (no Rust backend)
cd app && npm run dev

# Production build (outputs .app and .dmg)
cd app && npm run tauri build
```

## Tests

```bash
# Rust unit tests (single-threaded — timing-sensitive tests use sleep)
cd app/src-tauri && cargo test --lib -- --test-threads=1

# Transcription integration tests (requires models on disk, skips if absent)
cd app/src-tauri && cargo test --test transcription_integration -- --test-threads=1

# Frontend unit tests (settings migration)
cd app && npm test

# TypeScript type check
cd app && npx tsc --noEmit
```

Rust unit tests (52) cover keyboard detection, audio RMS, tray icon rendering, and WAV parsing. Frontend tests (7) cover settings migration from legacy formats. Integration tests validate the Whisper and Moonshine transcription pipelines end-to-end — they auto-skip when models aren't installed.

CI runs `cargo check`, `cargo test --lib`, `npx tsc --noEmit`, and `npm test` on every push to main and on PRs.

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
