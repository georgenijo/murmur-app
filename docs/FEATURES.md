## Local Dictation v0.4.0 — Feature Map

### Core: Voice-to-Text
- Record voice → transcribe locally via whisper.cpp → text to clipboard
- Metal GPU acceleration on Apple Silicon
- 5 Whisper models: tiny, base.en, small.en, medium, large-v3-turbo
- Configurable language (default: English)
- Greedy inference, single-segment, blank suppression
- Zero cloud dependencies — fully offline

### Recording Activation (2 modes)
- **Key Combo** — configurable global shortcut (e.g. Shift+Space), captured via free-form key input with macOS symbols
- **Double-Tap** — tap a modifier key twice to start, once to stop. Rejects held keys, combos, slow taps, triple-tap spam

### Text Output
- Clipboard copy (always)
- Optional auto-paste via osascript Cmd+V simulation (150ms delay)
- Auto-paste requires accessibility permission

### UI
- Main window (720x560, dark/light theme follows macOS)
- Start/Stop recording buttons
- Transcription history (timestamped, copy-to-clipboard, clear all)
- Stats bar: total words, avg WPM, total recordings, approx tokens
- Error display banner
- Permissions banner (microphone + accessibility status, grant buttons)
- Dev mode banner

### Settings Panel
- Model selector dropdown with sizes
- Recording mode toggle (Key Combo / Double-Tap)
- Key capture input (hotkey mode) or modifier dropdown (double-tap mode)
- Auto-paste toggle with accessibility status
- Reset stats button (two-click confirm)
- View logs button
- Version footer

### System Tray
- Color-coded icon: gray (idle), red (recording), amber (processing), amber (idle in dev mode)
- Menu: Show Window, Toggle Overlay, About, Quit
- Click to show window
- Hide-on-close (doesn't quit)

### Overlay Widget
- Floating always-on-top 200x60 window
- 5-bar animated waveform driven by real-time audio levels
- Click to toggle recording, double-click for locked mode
- Color-coded states: stone (idle), red (recording), amber (processing)

### Model Downloader
- First-launch onboarding screen if no model found
- Downloads from Hugging Face with streaming progress bar
- 3 model choices with size info
- Atomic download (.tmp → rename)

### Log Viewer
- Modal with last 200 lines, auto-refreshes every 2s
- Color-coded level badges (INFO/WARN/ERROR)
- Clear and Copy All buttons
- Per-transcription timing: audio parse, inference, injection, total pipeline

### Resource Monitor (dev only)
- Collapsible CPU% + Memory MB panel
- Dual-line SVG chart, 60-point rolling history
- Polls only when expanded

### Permissions
- Microphone: required for recording
- Accessibility: required for double-tap mode + auto-paste
- In-app status checks, grant buttons, opens System Settings

### Platform & Distribution
- macOS only (12+), Apple Silicon optimized
- Developer ID signed + notarized
- DMG installer via GitHub Actions (tag-triggered)
- ~1.5MB DMG, ~25MB installed

### CI/CD
- TypeScript type checking on push/PR
- Omen code quality analysis on PRs
- Claude Code automated PR review
- Claude Code issue/PR comment bot (@claude)

### Developer
- File-based logging with 5MB rotation
- Heartbeat logging for keyboard listener (60s)
- Per-phase timing instrumentation
- Debug tray icon color (amber)
