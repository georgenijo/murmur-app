# Changelog

All notable changes to Local Dictation will be documented in this file.

## [Unreleased]

### Added
- **Double-tap modifier key recording mode** — double-tap Shift/Option/Control to start recording, single tap to stop. Uses rdev for low-level keyboard event listening with a state machine that rejects held keys, modifier+letter combos, slow taps, and triple-tap spam.
- **Recording mode setting** — choose between "Key Combo" (Shift+Space etc.) and "Double-Tap" modes in Settings.
- **Unit tests** — 23 tests for the DoubleTapDetector state machine covering all edge cases (held keys, combos, cooldowns, single-tap-to-stop).
- `keyboard.rs` module for double-tap detection and rdev listener management.

### Fixed
- Settings help text incorrectly said "Hold this key to record, release to transcribe" — corrected to reflect toggle/double-tap behavior.
- rdev macOS crash: switched from crates.io v0.5 to git `main` branch and added `set_is_main_thread(false)` to prevent TIS/TSM thread-safety segfaults.

### Changed
- Accessibility permission is now also required for double-tap recording mode (rdev needs it for global keyboard event listening).

## [0.2.0] - 2025-XX-XX

### Added
- Native audio capture via cpal (replaced Web Audio + Python sidecar)
- Pure Rust transcription pipeline via whisper-rs with Metal GPU acceleration
- Auto-paste toggle with osascript Cmd+V simulation
- File-based logging with rotation (`~/Library/Application Support/local-dictation/logs/`)

### Removed
- Python sidecar dependency — all processing is now pure Rust
- Web Audio capture module (`audioCapture.ts`)

## [0.1.0] - 2024-XX-XX

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

### Backend
- Python sidecar for transcription (whisper.cpp)
- JSON-based communication protocol
- Support for multiple Whisper models (tiny.en to large-v3-turbo)

### Technical
- Built with Tauri 2, React 18, TypeScript, Tailwind CSS
- ~1.5MB DMG, ~25MB installed app size
- Local processing - no cloud dependencies
