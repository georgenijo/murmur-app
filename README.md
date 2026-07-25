<picture><source media="(prefers-color-scheme: dark)" srcset="docs/banner-dark.svg"><img src="docs/banner.svg" alt="murmur — Local voice-to-text for macOS" width="100%"></picture>

# Murmur

Privacy-first voice-to-text for macOS. Hold a key, speak, release — your words land in any app. No cloud, no subscriptions, no data leaves your machine.

Built with [Tauri 2](https://tauri.app/) (Rust + React), with local transcription through Core ML on the Apple Neural Engine, whisper.cpp on Metal, or sherpa-onnx on CPU.

## Features

- **100% local** — transcription, text rewriting, and benchmarking all run on-device. Nothing leaves the machine
- **Apple Neural Engine by default** — multilingual Parakeet v3 through Core ML for very low-latency transcription; other engines a click away
- **Three recording modes** — Hold Down, Double-Tap, or Both (hold and double-tap on the same key)
- **Clipboard-first output** — text always copied to the clipboard. Optional auto-paste into your focused app, plus optional transcript/audio file output
- **Selected-text Transform** — select text anywhere, hold a second key, say "make this shorter", and review a local LLM's proposal before it replaces anything. Never auto-applies
- **Text that comes out right** — deterministic local cleanup, smart formatting, spoken CLI command grammar, custom vocabulary with sounds-like correction, and typed voice commands with snippets and variables
- **Learns your terms** — correct a transcription once and teach Murmur the exact replacement, scoped globally, per app, or per project
- **Per-app profiles** — writing style and delivery behavior per application, resolved once per recording
- **Floating overlay** — notch-anchored widget with a live waveform and hover quick-settings; never steals focus
- **Performance Lab** — benchmark installed models on identical speech: latency, realtime factor, memory, and three tiers of word error rate
- **Diagnostics** — structured event log, bounded local run history, and CPU/memory timelines, all content-free
- **MIT licensed** — use it, fork it, build on it

## Installation

1. Download the latest `.dmg` from the [Releases](https://github.com/georgenijo/murmur-app/releases) page
2. Open the DMG and drag **Murmur** to your Applications folder
3. Launch the app — the setup assistant walks you through microphone and Accessibility permissions and downloads a model

Requires macOS 14+ on Apple Silicon.

### Permissions

Grant these in **System Settings > Privacy & Security** when prompted:

| Permission | Required for |
|------------|-------------|
| Microphone | Recording your voice |
| Accessibility | Keyboard detection, auto-paste, and reading/replacing the selection for Transform |

## Models

Choose a model based on your speed/accuracy tradeoff. Models download automatically on first launch, or you can switch models in Settings at any time.

| Model | Engine | Accelerator | Size | Notes |
|-------|--------|-------------|------|-------|
| Parakeet v3 | Core ML | Apple Neural Engine | ~470 MB | Default, multilingual, lowest latency |
| Parakeet v2 | sherpa-onnx | CPU | ~1.2 GB | English CPU fallback; also the non-Apple-Silicon path |
| Whisper Tiny | whisper.cpp | Metal GPU | ~75 MB | English |
| Whisper Base | whisper.cpp | Metal GPU | ~150 MB | English |
| Whisper Small | whisper.cpp | Metal GPU | ~500 MB | English |
| Whisper Medium | whisper.cpp | Metal GPU | ~1.5 GB | English |
| Whisper Large Turbo | whisper.cpp | Metal GPU | ~3 GB | Multilingual |

Open **Settings > Performance** to benchmark installed configurations on your
machine. Accuracy is measured as word error rate against bundled speech with
known reference transcripts.

## Recording Modes

Configure in the Settings panel:

**Hold Down** — hold a modifier key (Shift, Option, or Control) to record. Release to stop and transcribe.

**Double-Tap** — quickly double-tap a modifier key to start recording. Single tap to stop. The detector rejects held keys, modifier+letter combos, slow taps, and triple-tap spam.

**Both** — both gestures on the same key, disambiguated by a 200ms hold-promotion window.

Transcribed text is always copied to your clipboard. Enable **Auto-Paste** in Settings to have it pasted automatically into your focused app.

## Selected-text Transform

Select text in any app, hold the transform key (Right Option by default, off until you enable it), and speak an instruction — "make this shorter", "fix grammar", or the name of a preset or saved transform.

A local LLM proposes a rewrite in a small popover with a word diff. Nothing is written until you approve, and Undo restores the original. The model is a pinned Qwen2.5-1.5B-Instruct that runs in a separate signed helper process with no network access. Password fields fail closed.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| App framework | Tauri 2 |
| Backend | Rust |
| Frontend | React 18, TypeScript, Tailwind CSS 4 |
| Transcription | FluidAudio/Core ML, whisper-rs/Metal, sherpa-onnx/CPU |
| Local rewriting | llama.cpp in a signed sidecar process |
| Audio capture | cpal |
| Keyboard listener | rdev |
| Clipboard | arboard |
| Auto-paste | CGEvent (osascript fallback) |
| Local storage | SQLite (rusqlite) |
| Build tool | Vite 6 |

## Building from Source

```bash
git clone https://github.com/georgenijo/murmur-app.git
cd murmur-app

python3 scripts/build_local_llm_sidecar.py   # required first on macOS

cd app
npm install
npm run tauri dev        # Dev with hot reload
npm run tauri build      # Production .app and .dmg
```

Requires macOS 14+ on Apple Silicon, [Node.js](https://nodejs.org/) 18+, [Rust](https://rustup.rs/) (latest stable), and Python 3.

The sidecar step is not optional: the macOS Tauri config declares the sidecar as an `externalBin`, so builds — and even `cargo check` — fail until that binary exists. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

### Running Tests

```bash
cd app/src-tauri && cargo test -- --test-threads=1   # Rust unit tests
cd app && npm test                                    # frontend unit tests
cd app && npx tsc --noEmit                            # TypeScript type check
```

## Architecture

```
Hotkey (rdev) → Audio Capture (cpal) → Transcription Engine → Transcript Pipeline → Clipboard (arboard) → Auto-Paste (CGEvent)
       ↕                  ↕                     ↕                      ↕                                        ↕
   Frontend (React) ←——— Tauri IPC ———→ Rust Backend ———→ Tray + Overlay + Transform Popover ———→ LLM Sidecar (separate process)
```

Transcription engines sit behind one `TranscriptionBackend` trait and one model catalog. Post-recognition text passes through a single ordered pipeline (cleanup → voice commands → correction → formatting → IDE context → CLI). The local rewriting LLM runs out of process, because llama.cpp's ggml ABI clashes with whisper's.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full picture and [docs/FEATURES.md](docs/FEATURES.md) for the feature map.

## License

[MIT](LICENSE)
