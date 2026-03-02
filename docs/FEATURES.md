## Murmur v0.8.0 — Feature Map

### Core: Voice-to-Text
- Record voice, transcribe locally, output text to clipboard
- Two transcription backends: Whisper (Metal GPU via whisper-rs) and Moonshine (CPU via sherpa-onnx/sherpa-rs)
- 7 models across both backends:
  - Moonshine: moonshine-tiny (~124 MB), moonshine-base (~286 MB)
  - Whisper: tiny.en (~75 MB), base.en (~150 MB), small.en (~500 MB), medium.en (~1.5 GB), large-v3-turbo (~3 GB)
- Default model: moonshine-tiny (fastest, sub-20ms for typical dictation)
- Greedy inference, single-segment, blank suppression (Whisper)
- Lazy model loading: model context created on first transcription, cached across subsequent runs
- WhisperState caching: GPU/Metal buffers allocated once and reused (v0.7.8 optimization)
- Moonshine models use ONNX int8 quantized inference on CPU
- Zero cloud dependencies — fully offline

### Voice Activity Detection (VAD)
- Silero VAD v5.1.2 pre-filters silence before transcription
- Prevents Whisper hallucination loops on silent/noisy audio
- Configurable sensitivity: 0-100 scale (default 50), exposed as slider in settings
- Higher sensitivity keeps more audio; lower trims silence more aggressively
- VAD model (~1.8 MB) co-downloaded automatically alongside any transcription model
- Fallback: if VAD model is missing at transcription time, a background download is started for next time
- If VAD detects no speech, transcription is skipped entirely (empty string returned)

### Recording Activation (3 modes)
- **Hold Down** — hold a modifier key to record; release to stop and transcribe
- **Double-Tap** — tap a modifier key twice (within 400ms, each hold under 200ms) to start; single tap to stop
- **Both** — hold-down and double-tap run simultaneously on the same key. Uses 200ms deferred hold promotion: short taps feed double-tap detection; only presses held beyond 200ms start a recording
- Trigger key choices: Left Shift, Left Option/Alt, Right Control
- Recordings shorter than 0.3 seconds are silently discarded as phantom triggers
- Both detectors reject modifier+letter combos to avoid triggering during normal typing

### Text Output
- Clipboard copy always (via arboard)
- Optional auto-paste via osascript Cmd+V simulation
- Configurable paste delay: 10-500ms (default 50ms), slider in settings (only shown when auto-paste enabled)
- Auto-paste retries once on failure with 100ms backoff
- On paste failure: "auto-paste-failed" event with hint to paste manually (auto-clears after 5 seconds)
- Auto-paste requires accessibility permission
- Empty/whitespace-only text is silently skipped

### UI — Main Window
- 720x560 default size, 520x400 minimum, centered on launch
- Dark/light theme follows macOS system appearance (Tailwind dark: variants)
- Header with "Murmur" title, status indicator, settings gear button; acts as Tauri drag region
- Status indicator: Initializing (gray), Ready (green), Recording Xs (red, pulsing mic), Processing (amber, spinner)
- Start/Stop recording buttons (disabled while not initialized or processing)
- Transcription history: reverse-chronological, click to copy, timestamped, duration displayed
- History capped at 50 entries, clear with confirmation dialog
- Stats bar: Total Words, Avg WPM, Recordings, Approx Tokens
- Error display banner; auto-paste failure hint with 5-second auto-dismiss
- Permissions banner: microphone + accessibility status with grant buttons, auto-rechecks on window focus, dismissable
- Close-to-hide: close request hides the window instead of destroying it

### Settings Panel (side drawer, 280px)
- Slides in/out from the right with width transition
- Organized into collapsible sections with animated expand/collapse

#### Section: Transcription
- Model selector dropdown with two groups: "Moonshine (Fast, CPU)" and "Whisper (Metal GPU)"
- Each option shows label and file size
- Inline model download: warning when model not downloaded, progress bar, retry on error
- Microphone selector: lists all audio input devices via system query, "System Default" option, warning if saved device not found

#### Section: Recording
- VAD Sensitivity slider: 0-100%, step 5, draft value while dragging
- Recording Trigger mode: three toggle buttons (Hold Down, Double-Tap, Both)
- Accessibility permission warning with grant link when not granted
- Trigger Key selector: label changes per mode (Hold Key / Double-Tap Key / Trigger Key), contextual help text

#### Section: Output
- Auto-Paste toggle with accessibility status indicator
- Paste Delay slider: 10-500ms, step 10ms (only visible when auto-paste enabled)
- Launch at Login toggle, syncs with macOS login item state on mount

#### Section: About (collapsed by default)
- Model info display: name, backend, size
- Reset Stats button with two-click confirmation (auto-resets after 3 seconds)
- View Logs button (opens log viewer window)
- Check for Updates button with status text
- Version display

### Model Downloader (first-launch onboarding)
- Full-screen view shown when no model is installed
- 4 curated model cards: moonshine-tiny, moonshine-base, large-v3-turbo, base.en
- Each card shows label, size, description
- Default selection: moonshine-tiny
- Download button with progress bar (percentage + byte counter)
- Error display with retry
- Selection disabled during download
- Checks both backends: download screen skipped if any model exists (Whisper or Moonshine)

### System Tray
- Static white waveform icon: 66x66 RGBA (3x for 22pt Retina menu bar), 5 vertical capsule bars
- Menu: "Show Murmur" and "Quit Murmur" only
- Left-click on tray icon shows and focuses the main window
- Hide-on-close (doesn't quit)

### Overlay Widget (Dynamic Island / notch overlay)
- Separate always-on-top window (260x100), transparent, borderless, not focusable, visible on all workspaces
- Positioned over the MacBook notch area, window level above menu bar (NSMainMenuWindowLevel + 1)
- Notch-aware sizing: detects notch dimensions via NSScreen APIs (safe area insets, auxiliary areas)
- Fallback dimensions: 200px wide, 37px tall when no notch detected
- Three visual states: idle (dimmed mic icon), recording (red dot + waveform), processing (spinner + dimmed waveform)
- 7-bar animated waveform driven by real-time audio levels via requestAnimationFrame with direct DOM manipulation
- Center bars taller (envelope shaping), random jitter for organic feel
- Single click (250ms debounced): stops recording if active, exits locked mode
- Double-click: toggles locked mode; starts/stops recording
- Locked mode persists recording across single clicks until explicitly unlocked
- Screen change observer: re-detects notch on monitor plug/unplug or lid open/close
- Uses `_setPreventsActivation:` private API (guarded by respondsToSelector:) to prevent overlay clicks from activating the app
- Entire overlay is a Tauri drag region

### Log Viewer Window
- Separate window (800x600, min 600x400), titled "Murmur -- Log Viewer"
- Close-to-hide behavior (not destroyed)
- Two tabs: Events and Metrics

#### Events Tab
- Stream filter chips: toggle pipeline, audio, keyboard, system streams (colored)
- Level filter: toggle info, warn, error levels
- Scrollable event list with monospace font and auto-scroll (disengages on scroll up, re-engages near bottom)
- Each row: timestamp, stream chip, level label, summary text
- Expandable rows with JSON data detail view
- Copy All button (filtered events as text lines)
- Clear button (clears frontend buffer and backend ring buffer)

#### Metrics Tab
- Extracts timing from pipeline events where summary is "transcription complete"
- Last 20 transcriptions displayed
- Toggleable series legend: Total, Inference, VAD, Paste
- Stat cards per visible series: latest value, average, trend indicator (up/down/flat, 10% threshold)
- Two SVG line charts: upper (Total + Inference, 150px), lower (VAD + Paste, 120px)
- Auto-scaled Y-axis with round tick marks, X-axis by transcription index
- Polylines with dots at each data point

### Structured Event System
- All logging via tracing crate with two layers: pretty-printed log file + structured JSONL + Tauri event emission
- TauriEmitterLayer intercepts all tracing events, converts to AppEvent structs
- In-memory ring buffer: 500 events max, FIFO eviction, seeded from existing JSONL on startup
- JSONL persistence: events.jsonl (release) / events.dev.jsonl (dev), rotated at 5 MB
- Real-time streaming: every tracing event broadcast to all frontend windows via app-event
- Four streams: pipeline, audio, keyboard, system
- Five levels: trace, debug, info, warn, error
- Privacy stripping: in release builds, all string fields stripped from pipeline stream events in data object
- Frontend logging: log_frontend command routes frontend messages through Rust tracing

### Auto-Updater
- Background update check on launch and every 24 hours
- Endpoint: GitHub releases latest.json
- Forced updates: if current version is below remote min_version, update is required (no skip/dismiss)
- Skip version: user can skip a specific version (persisted to localStorage)
- Dismiss: user can dismiss without skipping
- Download progress with percentage
- Auto-relaunch after successful download and install
- Native macOS notification when update available (background check)
- Update modal phases: available (with markdown release notes), downloading (progress), ready (installing), error (retry)

### About Modal
- App name "Murmur", version, description, copyright
- Microphone icon, close button, backdrop click to close

### Resource Monitor (dev builds only)
- Collapsible CPU% + Memory MB panel
- Dual-line SVG chart: CPU (stone) and Memory (amber), 60-point rolling history
- Grid lines at 25%, 50%, 75%
- Polls every 1 second only when expanded (performance optimization)
- Collapse state persisted to localStorage

### Permissions
- Microphone: required for recording
- Accessibility: required for keyboard listener (all modes) and auto-paste
- In-app status checks with grant buttons that open System Settings
- Automatic re-check on window focus
- Dismissable banner (session-scoped)

### Three-Window Architecture
- Main window (720x560): full app UI, settings, recording controls, history
- Overlay window (260x100): notch-anchored Dynamic Island
- Log viewer window (800x600): structured event browser and metrics
- Each window has separate Tauri capabilities following least privilege
- Overlay reads settings from localStorage directly (no shared React context across windows)
- Close-to-hide for main and log-viewer windows

### Platform and Distribution
- macOS only (12+), Apple Silicon optimized
- Metal GPU acceleration for Whisper backend
- Developer ID signed + notarized
- DMG installer via GitHub Actions (tag-triggered)
- Binary size optimized: opt-level "s", LTO, strip, codegen-units 1, panic abort
- Dev build has separate bundle identifier (com.localdictation.dev) and name (Local Dictation Dev)

### CI/CD
- TypeScript type checking on push/PR
- Omen code quality analysis on PRs
- Claude Code automated PR review
- Claude Code issue/PR comment bot (@claude)
- Rust tests: single-threaded (cargo test -- --test-threads=1)
- Frontend tests: vitest with jsdom

### Dependencies
- **Rust**: Tauri 2, whisper-rs (Metal), sherpa-rs (ONNX/Moonshine), cpal, arboard, rdev (git main), reqwest (rustls-tls), sysinfo, tracing/tracing-subscriber/tracing-appender, objc2/objc2-app-kit (macOS), tar/bzip2
- **Frontend**: React 18, Tailwind CSS 4, Vite 6, TypeScript ~5.6, react-markdown, rehype-sanitize, @tauri-apps/api and plugins (autostart, updater, notification, process, opener)
- **5 Tauri plugins**: opener, autostart (LaunchAgent), updater, notification, process
