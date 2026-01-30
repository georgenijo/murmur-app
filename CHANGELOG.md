# Changelog

All notable changes to Local Dictation will be documented in this file.

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
