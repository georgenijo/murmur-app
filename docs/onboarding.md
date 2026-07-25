# Onboarding

## Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) (latest stable)
- Python 3 (sidecar and packaging scripts)
- macOS 14 or newer on Apple Silicon
- A transcription model (the app downloads one on first launch)

## Setup

```bash
# Build the local-LLM sidecar FIRST — on macOS, tauri dev/build and even
# cargo check/test fail on a fresh clone until this binary exists.
python3 scripts/build_local_llm_sidecar.py

cd app && npm install
```

The sidecar lands at
`app/src-tauri/binaries/murmur-llm-sidecar-aarch64-apple-darwin`. It is
gitignored and rebuilt by release CI. The script is a no-op on non-arm64-macOS.

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
(cd app/src-tauri && cargo test --lib -- --test-threads=1)

# Whisper integration tests (requires models on disk, skips if absent)
(cd app/src-tauri && cargo test --test transcription_integration -- --test-threads=1)

# FluidAudio Core ML integration test (explicit opt-in; requires its model cache)
(cd app/src-tauri && MURMUR_COREML_TEST_WAV=/path/to/16khz-mono.wav cargo test --test coreml_transcription_integration -- --ignored)

# Frontend unit tests (settings migration)
(cd app && npm test)

# TypeScript type check
(cd app && npx tsc --noEmit)

# Same-corpus Core ML vs CPU benchmarks (generate fixtures first; install the
# selected model through Murmur's setup screen before running)
./bench/make_audio.sh
(cd app/src-tauri && cargo run --release --example transcription_bench -- --engine coreml --iterations 5)
(cd app/src-tauri && cargo run --release --example transcription_bench -- --engine parakeet --iterations 5)
```

Rust unit tests cover backend dispatch/cache validation, keyboard detection, audio RMS, tray rendering, and WAV parsing. Frontend tests cover settings migration and preservation of existing model selections. Model-backed integration tests are optional so CI never downloads hundreds of megabytes.

## Default Core ML Model

New installs use `parakeet-tdt-0.6b-v3-coreml`, powered by FluidAudio and the Apple Neural Engine. First launch downloads and compiles about 470 MB into `~/Library/Application Support/FluidAudio/Models/parakeet-tdt-0.6b-v3/`. This can take tens of seconds the first time. Whisper and the sherpa-onnx CPU Parakeet model remain available in Settings.

CI runs `cargo check`, `cargo test --lib`, `npx tsc --noEmit`, and `npm test` on every push to main and on PRs.

## Whisper Models

The app requires a ggml `.bin` model file. Download one:

| Model | Size | Speed | Accuracy |
|-------|------|-------|----------|
| `tiny.en` | ~75 MB | Fastest | Basic |
| `base.en` | ~150 MB | Fast | Good |
| `small.en` | ~500 MB | Medium | Better |
| `medium.en` | ~1.5 GB | Slow | Great |
| `large-v3-turbo` | ~3 GB | Fast | Best of the Whisper set, multilingual |

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
| **Microphone** | Audio capture — dictation and transform instructions | Prompted in-app by the setup assistant |
| **Accessibility** | All hotkeys (rdev), auto-paste, and transform selection capture / write-back | System Settings > Privacy & Security > Accessibility |

Under `npm run tauri dev`, grant both to your *terminal app* — dev builds are
re-signed on each rebuild, so a grant to the dev binary never sticks.

## Logs

Release and dev builds write to the same local directory:
`~/Library/Application Support/local-dictation/logs/`. Release files are
`app.log` and `events.jsonl`; debug files are `app.dev.log` and
`events.dev.jsonl`. Structured logs rotate at 5 MB. See
[`features/log-viewer.md`](features/log-viewer.md) for the event architecture and
[`tools/murmur-diag/README.md`](../tools/murmur-diag/README.md) for local MCP
diagnostics.

## Transform model

The selected-text transform uses a separate, optional local LLM: Qwen2.5-1.5B-Instruct Q4_K_M (~1.1 GB), downloaded from **Settings → Transform**. It is pinned by exact size and SHA-256, verified again before every spawn, and stored at:

```text
~/Library/Application Support/local-dictation/models/transform-llm/<sha256>/qwen2.5-1.5b-instruct-q4_k_m.gguf
```

Remove it from the same page. The helper process is shut down first so the file is not open. If repeated faults trip the circuit breaker, **Reset runtime** re-enables it.

## Diagnostics data

Local run history and CPU/memory samples live in `diagnostics/performance.sqlite3` under the Tauri app-data directory (`~/Library/Application Support/com.localdictation/`, or `com.localdictation.dev` in dev builds). They are content-free and bounded (200 runs, 600 samples), and are cleared from the Log Viewer. Explicitly consented transform content captures live beside them under `diagnostics/transforms/` with restrictive permissions, a 3-capture cap, and a 7-day expiry.

## Personal knowledge data

Settings → Knowledge stores replacement rules, vocabulary terms, and snippets only on this Mac under the app data directory in `knowledge/knowledge.sqlite3`. Use the Knowledge page to inspect, edit, disable, export, import, or permanently delete this data. Knowledge content and selected file paths are excluded from logs. See [`features/personal-knowledge-store.md`](features/personal-knowledge-store.md) for migration, backup, and recovery behavior.
