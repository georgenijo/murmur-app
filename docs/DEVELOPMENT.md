# Development Setup

## Prerequisites

- macOS 14+ on Apple Silicon (Core ML/ANE and the local-LLM sidecar are Apple Silicon only)
- Node.js 18+
- Rust (install via rustup)
- Python 3 (for the sidecar and packaging scripts)

## Setup

```bash
git clone https://github.com/georgenijo/murmur-app.git
cd murmur-app

# 1. Build the local-LLM sidecar FIRST (macOS only — no-op elsewhere)
python3 scripts/build_local_llm_sidecar.py

# 2. Install Node dependencies
cd app && npm install

# 3. Run
npm run tauri dev
```

> **Step 1 is not optional on macOS.** `tauri.macos.conf.json` declares the
> `murmur-llm-sidecar` externalBin, so `tauri dev`, `tauri build`, **and even
> `cargo check` / `cargo test`** fail on a fresh clone until the binary exists at
> `app/src-tauri/binaries/murmur-llm-sidecar-aarch64-apple-darwin`. The binary is
> gitignored; release CI builds it before bundling. If you only need the Rust
> crate to compile, a stub file at that path is enough.

A transcription model is downloaded through the app's first-launch setup
assistant — you do not need to place model files by hand. The pinned transform
LLM is a separate, optional download from **Settings → Transform**.

## Commands

```bash
# Dev
cd app && npm run tauri dev        # full app, hot reload
cd app && npm run dev              # frontend only (no Rust backend)
cd app && npm run tauri build      # production .app and .dmg

# Tests
cd app/src-tauri && cargo test -- --test-threads=1   # Rust (timing-sensitive; keep single-threaded)
cd app && npm test                                   # frontend vitest
cd app && npx tsc --noEmit                           # TypeScript
```

CI runs `cargo check`, `cargo test`, `npx tsc --noEmit`, **and** `npm test` on
every push to main and on PRs. A tsc-only check is not sufficient verification —
several regressions have shipped past it.

Model-backed integration tests are opt-in so CI never downloads hundreds of
megabytes:

```bash
cd app/src-tauri && cargo test --test transcription_integration -- --test-threads=1
cd app/src-tauri && MURMUR_COREML_TEST_WAV=/path/to/16khz-mono.wav \
    cargo test --test coreml_transcription_integration -- --ignored
```

## macOS Permissions

Murmur needs **Microphone** and **Accessibility**. Which process needs them
depends on how you are running it:

- **`npm run tauri dev`** — grant them to your *terminal app* (Ghostty, iTerm,
  Terminal). Dev builds are re-signed on every rebuild, so granting the dev
  binary itself is futile. Restart the dev server after granting.
- **A built `.app`** — grant them to the app itself.

## Project Structure

```text
murmur-app/
├── app/
│   ├── src/                     # React frontend
│   │   ├── components/          # UI (settings, overlay, log-viewer, transform-review, …)
│   │   └── lib/                 # hooks, settings, pure logic
│   └── src-tauri/
│       ├── src/                 # Rust backend (see docs/ARCHITECTURE.md for the module map)
│       ├── sidecars/local-llm/  # the signed murmur-llm-sidecar crate
│       ├── crates/              # local-llm-protocol
│       ├── binaries/            # built sidecar (gitignored)
│       └── tests/               # integration tests
├── bench/                       # benchmark audio fixtures
├── docs/                        # architecture, features, references, decisions
├── prompts/                     # agent prompt files (work / chat / release / bug / swarm)
├── scripts/                     # sidecar build, notarization, release artifacts
├── tests/                       # release-artifact and workflow-policy tests (Python)
└── tools/murmur-diag/           # local MCP log-diagnostics tool
```

## Building for Production

```bash
cd app && npm run tauri build
```

Output:
- `app/src-tauri/target/release/bundle/macos/Murmur.app`
- `app/src-tauri/target/release/bundle/dmg/Murmur_x.x.x_aarch64.dmg`

The `build` shell function installs the result into `/Applications` and resets
the stale Accessibility grant for you (see below).

## Logs

Both build flavors write to `~/Library/Application Support/local-dictation/logs/`:

| File | Build |
|------|-------|
| `app.log` / `events.jsonl` | release |
| `app.dev.log` / `events.dev.jsonl` | dev |

Structured JSONL rotates at 5 MB. In-app: **Settings → General → View Logs**, or
the Log Viewer window. See [features/log-viewer.md](features/log-viewer.md) and
[`tools/murmur-diag/README.md`](../tools/murmur-diag/README.md).

## Common Issues

### Sidecar binary not found

```
failed to bundle project: `murmur-llm-sidecar-aarch64-apple-darwin` not found
```

Run `python3 scripts/build_local_llm_sidecar.py`. This also breaks plain
`cargo check`/`cargo test`, because Tauri's build script validates externalBin
entries.

### Permission prompt on every rebuild

Dev builds get a different signature each time. Grant Accessibility to your
terminal app instead of the dev binary.

### Accessibility silently broken after a local production build

When you copy a freshly built `.app` over the old one, macOS keeps the old
Accessibility entry — but it is stale, because the new binary has a different
signature. Microphone carries over fine; anything keyboard-related (double-tap,
auto-paste, transform hold key) silently fails.

```bash
tccutil reset Accessibility com.localdictation
```

The `build` shell function does this automatically. **Note:** the
Accessibility-denied banner cannot be reproduced under `tauri dev` — you need a
built `.app` to exercise that path.

### "No input device available"

Grant Microphone to your terminal app, then restart the dev server.

### Transform crashes / popover misbehaves only in a real build

The transform popover's `NSWindow` treatment is raw AppKit and must run on the
main thread. `?mock=1` and `tauri dev` do not exercise that path — press the real
transform hold key on a built `.app` to smoke-test it (this is what reproduced
[#325](https://github.com/georgenijo/murmur-app/issues/325) immediately).

### Model not found

Use the in-app setup assistant. If you are placing files manually, Whisper
`.bin` models are searched in this order:

1. `$WHISPER_MODEL_DIR`
2. `~/Library/Application Support/local-dictation/models/`
3. `~/Library/Application Support/pywhispercpp/models/`
4. `~/.cache/whisper.cpp/`
5. `~/.cache/whisper/`
6. `~/.whisper/models/`

Core ML models are managed by FluidAudio under
`~/Library/Application Support/FluidAudio/Models/`.

## Release

Releases are tag-based off a trusted `main` version-bump commit, with signed and
notarized artifacts promoted only when the commit SHA, run ID, and artifact
hashes all match. See [release.md](release.md).
