# UI Development Tickets

## Phase 1: Tauri Project Setup

### TICKET-UI-001: Initialize Tauri project
**Status:** TODO
**Description:**
- Install Rust and Node.js prerequisites
- Run `npm create tauri-app@latest` in project directory
- Choose React + TypeScript template
- Verify `npm run tauri dev` works

**Acceptance Criteria:**
- [ ] Empty Tauri app opens
- [ ] Hot reload works for React changes
- [ ] `npm run tauri build` produces .app bundle

---

### TICKET-UI-002: Set up Python sidecar
**Status:** TODO
**Description:**
- Configure Tauri to bundle Python as sidecar
- Create `dictation_bridge.py` - single entry point for Tauri to call
- Implement JSON-based communication protocol
- Test calling Python from Tauri commands

**Acceptance Criteria:**
- [ ] Tauri can spawn Python process
- [ ] Can send commands (start_recording, stop_recording, get_status)
- [ ] Can receive responses (transcription text, status updates)

---

## Phase 2: Core UI Components

### TICKET-UI-003: Create main window layout
**Status:** TODO
**Description:**
- Design minimal UI layout:
  - Header with app name + status indicator
  - Main area for transcription display
  - Footer with settings button
- Use Tailwind CSS for styling
- Match macOS aesthetic (clean, minimal)

**Acceptance Criteria:**
- [ ] Responsive layout
- [ ] Dark/light mode support
- [ ] Native-feeling design

---

### TICKET-UI-004: Build settings panel
**Status:** TODO
**Description:**
- Model selector dropdown (tiny.en, base.en, small.en, medium.en, large-v3-turbo)
- Hotkey configuration input
- Launch at login toggle
- Show model storage usage
- Save settings to local storage / config file

**Acceptance Criteria:**
- [ ] Settings persist across app restarts
- [ ] Model change takes effect immediately
- [ ] Hotkey change takes effect immediately

---

### TICKET-UI-005: Implement recording status indicator
**Status:** TODO
**Description:**
- Visual indicator showing:
  - Idle (gray mic icon)
  - Recording (red pulsing mic icon)
  - Processing (spinning indicator)
- Show recording duration while active
- Audio waveform visualization (stretch goal)

**Acceptance Criteria:**
- [ ] Status updates in real-time
- [ ] Clear visual distinction between states

---

### TICKET-UI-006: Build transcription history view
**Status:** TODO
**Description:**
- List of recent transcriptions
- Show timestamp, duration, text
- Click to copy to clipboard
- Clear history button
- Persist history to local storage (last 50 entries)

**Acceptance Criteria:**
- [ ] History survives app restart
- [ ] Copy to clipboard works
- [ ] Can clear history

---

## Phase 3: System Integration

### TICKET-UI-007: Add menubar/tray icon
**Status:** TODO
**Description:**
- App lives in menubar when window closed
- Tray icon shows recording status
- Right-click menu: Show Window, Settings, Quit
- Left-click: Toggle window visibility

**Acceptance Criteria:**
- [ ] App stays running when window closed
- [ ] Tray icon updates with recording status
- [ ] Can quit from tray menu

---

### TICKET-UI-008: Implement global hotkey in Tauri
**Status:** TODO
**Description:**
- Register global hotkey (default: Left Shift hold)
- Hotkey works even when app is not focused
- Connect to Python recording start/stop
- Handle hotkey conflicts gracefully

**Acceptance Criteria:**
- [ ] Hotkey works system-wide
- [ ] Can change hotkey in settings
- [ ] Works with window hidden

---

### TICKET-UI-009: Handle macOS permissions
**Status:** TODO
**Description:**
- Check for required permissions on startup
- Prompt user to grant if missing:
  - Microphone
  - Accessibility
  - Input Monitoring
- Show instructions if permissions denied
- Link to System Settings

**Acceptance Criteria:**
- [ ] Clear permission prompts
- [ ] App guides user through setup
- [ ] Graceful handling of denied permissions

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
