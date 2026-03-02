# Changelog

All notable changes to Murmur will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Fixed
- Strip recording-status-changed emissions from `ensure_vad_model` to reduce event noise

## [0.8.0] - 2026-03-02

### Added
- **Structured event system** with `TauriEmitterLayer`, ring buffer, JSONL export, and privacy stripping (`telemetry.rs`)
- **Log viewer window** with Events and Metrics tabs for real-time structured event inspection

## [0.7.8] - 2026-03-01

### Fixed
- Cache `WhisperState` to eliminate per-transcription alloc/free cycles, improving latency

## [0.7.7] - 2026-03-01

### Added
- **Collapsible accordion sections** for the settings panel
- **Pre-VAD RMS logging** and VAD sensitivity slider for tuning speech detection

## [0.7.6] - 2026-02-28

### Fixed
- CI: set `CMAKE_OSX_DEPLOYMENT_TARGET=11.0` to fix `std::filesystem` errors with Xcode 16.4
- CI: add ARM i8mm flags to rust check job

## [0.7.5] - 2026-02-28

### Added
- **Silero VAD pre-processing** to filter silence and prevent whisper hallucination loops (`vad.rs`)
- **Configurable auto-paste delay** with retry logic and failure notification

### Fixed
- Discard phantom recordings and add transcription logging
- Reposition overlay on display configuration change

### Changed
- Split `lib.rs` into focused single-responsibility modules (`state.rs`, `audio.rs`, etc.)
- Split `keyboard.rs` into focused submodules
- Rename `ui/` to `app/` at repo root

## [0.7.0] - 2026-02-27

### Added
- **"Both" recording mode** — simultaneous hold-down + double-tap (`useCombinedToggle.ts`)

### Fixed
- Allow scrolling within long transcription history entries
- Restore tray icon and fix overlay click surfacing main window

## [0.6.7] - 2026-02-27

### Changed
- **Rebrand to Murmur** — app rename with new icon

## [0.6.5] - 2026-02-26

### Added
- **OTA auto-updater** with min-version enforcement (`useAutoUpdater.ts`, `lib/updater.ts`)
- Custom styled select dropdowns replacing native selects

### Fixed
- Log accessibility permission status in keyboard listener start/stop

## [0.6.2] - 2026-02-26

### Added
- **Microphone device selection** in settings
- **Launch at login** toggle

## [0.6.0] - 2026-02-26

### Added
- **Interactive overlay** with Dynamic Island notch integration (`OverlayWidget.tsx`, `commands/overlay.rs`)

## [0.5.3] - 2026-02-24

### Added
- Group model selector by backend (Moonshine / Whisper)
- CI: Rust tests and settings migration tests in CI pipeline
- CI: post-build smoke test in release workflow

### Fixed
- Statically link sherpa-rs to fix launch crash

## [0.5.0] - 2026-02-23

### Added
- **Moonshine transcription backend** via sherpa-rs as an alternative to Whisper
- **Hold-Down recording mode** replacing Key Combo mode (press to start, release to stop)
- `TranscriptionBackend` trait extracted from `transcriber.rs` for backend abstraction

### Fixed
- Eliminate auto-paste toggle race conditions and silent failures
- Surface Control shortcut failures and warn in settings

## [0.4.0] - 2026-02-20

### Added
- **In-app model downloader** for first-launch onboarding
- Per-phase timing instrumentation for the transcription pipeline

### Fixed
- Surface rdev listener failures and add heartbeat logging

## [0.3.2] - 2026-02-19

### Fixed
- Auto-paste toggle shrinks and loses track in dark mode
- Set `signingIdentity` so local builds use Developer ID cert
- Use draft-then-publish pattern in release workflow

## [0.3.0] - 2026-02-19

### Added
- **Live resource monitor** with CPU/memory chart (`resource_monitor.rs`, `useResourceMonitor.ts`)
- **Logging viewer** for inspecting app logs in real time
- **Double-tap modifier key recording mode** — double-tap Shift/Option/Control to start recording, single tap to stop
- **Recording mode setting** — choose between "Key Combo" and "Double-Tap" modes in Settings
- Unit tests (23 tests) for the `DoubleTapDetector` state machine
- `keyboard.rs` module for double-tap detection and rdev listener management

### Fixed
- Settings help text incorrectly described recording behavior
- rdev macOS crash: switched to git `main` branch and added `set_is_main_thread(false)` to prevent TIS/TSM segfaults

### Changed
- Accessibility permission now also required for double-tap recording mode

## [0.2.0] - 2026-02-19

### Added
- Native audio capture via cpal (replaced Web Audio + Python sidecar)
- Pure Rust transcription pipeline via whisper-rs with Metal GPU acceleration
- Auto-paste toggle with osascript Cmd+V simulation
- File-based logging with rotation (`~/Library/Application Support/local-dictation/logs/`)
- Word statistics with stats bar and localStorage persistence
- Custom hotkey binding
- Status widget — tray icon, overlay pill, audio waveform
- Warm neutral UI redesign

### Removed
- Python sidecar dependency — all processing is now pure Rust
- Web Audio capture module (`audioCapture.ts`)

## [0.1.0] - 2026-02-19

### Added
- Tauri desktop app with React/TypeScript frontend
- System tray integration (menubar icon)
- Global hotkey support (Shift+Space, Option+Space, Control+Space)
- Settings panel (model selection, hotkey configuration)
- Transcription history with copy-to-clipboard
- Recording status indicator with duration timer
- macOS permissions guidance
- About window with version info
- Production build with DMG installer
- Python sidecar for transcription (whisper.cpp)
- JSON-based communication protocol
- Support for multiple Whisper models (tiny.en to large-v3-turbo)
- Local processing with no cloud dependencies
