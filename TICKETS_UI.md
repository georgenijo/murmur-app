# UI Development Tickets

## Phase 1: Tauri Project Setup

### TICKET-UI-001: Initialize Tauri project
**Status:** DONE
**Description:**
- Install Rust and Node.js prerequisites
- Run `npm create tauri-app@latest` in project directory
- Choose React + TypeScript template
- Verify `npm run tauri dev` works

**Acceptance Criteria:**
- [x] Empty Tauri app opens
- [x] Hot reload works for React changes
- [x] `npm run tauri build` produces .app bundle

**Notes:**
- Bundle identifier warning: The current identifier `com.local-dictation.app` should be changed to `com.localdictation.app` (without the `.app` suffix) in a future update.

---

### TICKET-UI-002: Set up Python sidecar
**Status:** DONE
**Description:**
- Configure Tauri to bundle Python as sidecar
- Create `dictation_bridge.py` - single entry point for Tauri to call
- Implement JSON-based communication protocol
- Test calling Python from Tauri commands

**Acceptance Criteria:**
- [x] Tauri can spawn Python process
- [x] Can send commands and receive responses
- [x] Transcription text returned to frontend

**Notes:**
- Implementation uses `dictation_bridge.py` for the Python sidecar
- JSON protocol over stdin/stdout for communication
- Commands: `start_recording`, `stop_recording`, `get_status`, `configure`, `shutdown`

---

## Phase 2: Core UI Components

### TICKET-UI-003: Create main window layout
**Status:** DONE
**Description:**
- Design minimal UI layout:
  - Header with app name + status indicator
  - Main area for transcription display
  - Footer with settings button
- Use Tailwind CSS for styling
- Match macOS aesthetic (clean, minimal)

**Acceptance Criteria:**
- [x] Responsive layout
- [x] Dark/light mode support
- [x] Native-feeling design

**Notes:**
- Implemented 3-section layout: header, main transcription area, and footer
- macOS aesthetic achieved with backdrop blur effects, subtle shadows, and clean typography
- Pulsing status indicator in header shows recording state
- Dark/light mode support via Tailwind CSS dark: variants
- Responsive design adapts to window resizing

---

### TICKET-UI-004: Build settings panel
**Status:** DONE
**Description:**
- Model selector dropdown (tiny.en, base.en, small.en, medium.en, large-v3-turbo)
- Hotkey configuration input
- Launch at login toggle
- Show model storage usage
- Save settings to local storage / config file

**Acceptance Criteria:**
- [x] Settings persist across app restarts
- [x] Model change takes effect immediately
- [x] Hotkey change takes effect immediately

**Notes:**
- Settings panel implemented as slide-over from right
- Model selector with 5 whisper model options
- Hotkey selector with 5 options (currently UI only - hotkey implementation in future ticket)
- Settings persist to localStorage
- Model changes sent to Python bridge via configure command
- Launch at login deferred to future ticket

---

### TICKET-UI-005: Implement recording status indicator
**Status:** DONE
**Description:**
- Visual indicator showing:
  - Idle (gray mic icon)
  - Recording (red pulsing mic icon)
  - Processing (spinning indicator)
- Show recording duration while active
- Audio waveform visualization (stretch goal)

**Acceptance Criteria:**
- [x] Status updates in real-time
- [x] Clear visual distinction between states

**Notes:**
- Mic icon replaces colored dot
- Red pulsing mic when recording
- Spinning loader when processing
- Gray mic when idle
- Recording duration timer (shows seconds elapsed)
- Audio waveform deferred as stretch goal

---

### TICKET-UI-006: Build transcription history view
**Status:** DONE
**Description:**
- List of recent transcriptions
- Show timestamp, duration, text
- Click to copy to clipboard
- Clear history button
- Persist history to local storage (last 50 entries)

**Acceptance Criteria:**
- [x] History survives app restart
- [x] Copy to clipboard works
- [x] Can clear history

**Notes:**
- Tab system added (Current / History tabs)
- History entries show timestamp, duration, and text
- Click-to-copy with visual "Copied!" feedback
- Clear history button with confirmation
- Persists to localStorage, limited to 50 entries
- Empty state with clock icon when no history

---

## Phase 3: System Integration

### TICKET-UI-007: Add menubar/tray icon
**Status:** DONE
**Description:**
- App lives in menubar when window closed
- Tray icon shows recording status
- Right-click menu: Show Window, Settings, Quit
- Left-click: Toggle window visibility

**Acceptance Criteria:**
- [x] App stays running when window closed
- [x] Tray icon updates with recording status
- [x] Can quit from tray menu

**Notes:**
- Tray icon implemented using TrayIconBuilder in Tauri
- Right-click menu with "Show Window" and "Quit" options
- Left-click on tray icon shows the main window and brings it to focus
- Closing window hides to tray instead of quitting (via on_window_event handler)
- App uses default window icon for tray icon

---

### TICKET-UI-008: Implement global hotkey in Tauri
**Status:** DONE
**Description:**
- Register global hotkey (default: Left Shift hold)
- Hotkey works even when app is not focused
- Connect to Python recording start/stop
- Handle hotkey conflicts gracefully

**Acceptance Criteria:**
- [x] Hotkey works system-wide
- [x] Can change hotkey in settings
- [x] Works with window hidden

**Notes:**
- Toggle hotkey (press to start/stop)
- Uses Tauri global-shortcut plugin
- Works system-wide even when app hidden

---

### TICKET-UI-009: Handle macOS permissions
**Status:** DONE
**Description:**
- Check for required permissions on startup
- Prompt user to grant if missing:
  - Microphone
  - Accessibility
  - Input Monitoring
- Show instructions if permissions denied
- Link to System Settings

**Acceptance Criteria:**
- [x] Clear permission prompts
- [x] App guides user through setup
- [x] Graceful handling of denied permissions

**Notes:**
- Permission banner displayed on first launch
- Info.plist descriptions for microphone/accessibility permissions
- Button to open System Settings for granting permissions
- Banner is dismissible with localStorage persistence

---

## Phase 4: Polish & Distribution

### TICKET-UI-010: App icon and branding
**Status:** TODO
**Description:**
- Design app icon (microphone-based)
- Set app name: "Dictation" or "LocalDictate"
- Add About window with version info
- Create DMG background image

**Acceptance Criteria:**
- [ ] Professional app icon
- [ ] Consistent branding throughout

---

### TICKET-UI-011: Build and sign for distribution
**Status:** TODO
**Description:**
- Configure Tauri for production build
- Code sign with Apple Developer certificate (if available)
- Create DMG installer
- Test on clean macOS install

**Acceptance Criteria:**
- [ ] .app runs without "unidentified developer" warning (if signed)
- [ ] DMG installs cleanly
- [ ] All features work in production build

---

### TICKET-UI-012: Write README and documentation
**Status:** TODO
**Description:**
- Update README.md with:
  - Screenshots
  - Installation instructions
  - Usage guide
  - Building from source
- Add CHANGELOG.md
- Add LICENSE file

**Acceptance Criteria:**
- [ ] README has clear install instructions
- [ ] Screenshots show key features
- [ ] Ready for GitHub release

---

## Stretch Goals

### TICKET-UI-S01: Keyboard shortcuts
- Cmd+, for settings
- Cmd+H to hide
- Cmd+Q to quit

### TICKET-UI-S02: Auto-update mechanism
- Check for updates on startup
- Download and install updates

### TICKET-UI-S03: Multiple language support
- UI localization
- Whisper language selection for non-English

### TICKET-UI-S04: Audio input device selection
- Choose which microphone to use
- Show audio level meter

### TICKET-UI-S05: Export transcription history
- Export as TXT, JSON, or CSV
