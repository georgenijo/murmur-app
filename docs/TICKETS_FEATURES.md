# Local Dictation — Feature Tickets

**Created:** 2026-02-19
**Branch base:** `main` (after `feature/add-logging` is merged)

---

## Pre-Work: Commit & Merge Current Branch

Before any feature work begins, commit the unstaged/untracked work on `feature/add-logging` and merge to main.

**Files to commit:**
- `ui/src-tauri/src/keyboard.rs` (untracked — double-tap state machine, fully built + tested)
- `ui/src/lib/hooks/useDoubleTapToggle.ts` (untracked — frontend hook)
- `ui/src/App.tsx` (modified)
- `ui/src/components/settings/SettingsPanel.tsx` (modified — recording mode UI)
- `ui/src/lib/hooks/useHotkeyToggle.ts` (modified — `enabled` prop added)
- `ui/src/lib/settings.ts` (modified — RecordingMode type, double-tap options)
- `ui/src-tauri/src/lib.rs` (modified — keyboard commands wired, logging macros in use)
- `CHANGELOG.md`, `CLAUDE.md`, `README.md`, `Cargo.toml`, `Cargo.lock`

**Then:** PR `feature/add-logging` → `main`, merge.

---

## Dependency Order

```
Pre-Work (merge current branch)
    ↓
FEAT-001: Structural UI Uplift
    ↓
FEAT-002  FEAT-003  FEAT-004  FEAT-005  FEAT-006
(parallelizable after FEAT-001 merges)
```

---

## FEAT-001: Structural UI Uplift

**Priority:** P0 — prerequisite for all features
**Type:** Frontend refactor
**Branch:** `feat/ui-uplift`
**Status:** TODO

### Context
The current UI is a dev prototype: cool gray palette, vertical stack layout, tab navigation (Current/History tabs), overlay settings panel, footer with gear button. The target is a warm neutral redesign (Notion/Bear aesthetic): stone palette, horizontal flex layout with inline collapsible sidebar, single unified content feed (no tabs), no footer. This must land before any other features so they are built on the correct foundation.

Reference: `~/.claude/plans/jolly-tumbling-hippo.md` has the full color table and layout diagram.

### Acceptance Criteria
- [ ] App window is 720×560px (min 520×400px)
- [ ] Root layout is horizontal flex: main content area + inline settings sidebar (280px, animated open/close)
- [ ] No tab navigation — single unified content feed (current transcription card at top, history feed scrolls below)
- [ ] No footer — gear icon moves to header right side
- [ ] Settings sidebar opens/closes inline (not as overlay), with CSS width transition animation
- [ ] Stone palette throughout: `stone-*` replaces all `gray-*`
- [ ] Status colors: recording=`red-500`, processing=`amber-600`, ready=`emerald-600`
- [ ] Primary action button: `stone-800` light / `stone-100` dark
- [ ] `data-tauri-drag-region` attribute on header for native window drag
- [ ] Dark mode works in new layout
- [ ] `TabNavigation.tsx` deleted

### Files to Modify (in order)
1. `ui/src-tauri/tauri.conf.json` — window width: 720, height: 560, minWidth: 520, minHeight: 400
2. `ui/index.html` — `<title>` → "Local Dictation"
3. `ui/src/styles.css` — add `-webkit-font-smoothing: antialiased`
4. `ui/src/App.tsx` — horizontal flex root, remove `activeTab` state, remove `TabNavigation` import, remove footer, move gear icon to `StatusHeader` via props, `SettingsPanel` becomes `<aside>` sibling
5. `ui/src/components/StatusHeader.tsx` — add `onSettingsToggle` + `isSettingsOpen` props, gear icon on right, `data-tauri-drag-region`, amber-600 for processing, emerald-600 for ready
6. `ui/src/components/settings/SettingsPanel.tsx` — remove fixed overlay positioning + backdrop div, become `<aside>` with animated width transition: `${isOpen ? 'w-[280px]' : 'w-0 overflow-hidden'}`, all `gray-*` → `stone-*`
7. `ui/src/components/TranscriptionView.tsx` — remove `activeTab` prop, always render transcription card + `HistoryPanel` below, remove outer card wrapper
8. `ui/src/components/RecordingControls.tsx` — `stone-800` button, `rounded-lg` (not `rounded-xl`)
9. `ui/src/components/history/HistoryPanel.tsx` — all `gray-*` → `stone-*`, copied feedback → emerald, clear button → stone-500 default
10. `ui/src/components/PermissionsBanner.tsx` — `bg-green-500` → `bg-emerald-500`
11. `ui/src/components/AboutModal.tsx` — icon bg → `stone-800`, backdrop → `stone-900/50`
12. **DELETE** `ui/src/components/TabNavigation.tsx`

---

## FEAT-002: Status Widget

**Priority:** P1
**Type:** Frontend + Rust
**Branch:** `feat/status-widget`
**Status:** TODO
**Depends on:** FEAT-001

### Context
User wants a Whisperflow-style status indicator. Primary preference: dynamic tray icon that changes per recording state. Additionally: a small always-on-top floating overlay window at bottom-center of screen showing recording state with animated waveform. "Locked mode" allows pinning recording without holding a key — double-click the overlay to toggle it.

### Acceptance Criteria
- [ ] Tray icon updates per state: gray mic (idle), red pulsing mic (recording), spinner (processing)
- [ ] Overlay window: small pill at bottom-center of screen, always on top, no decorations, transparent background
- [ ] Overlay shows: idle (dark static waveform), recording (animated waveform bars driven by live audio level), processing (spinner)
- [ ] Double-clicking overlay enters "locked mode" — recording stays active without holding hotkey
- [ ] In locked mode, clicking overlay again stops recording
- [ ] Overlay visible/hidden toggle from tray menu
- [ ] Audio level from `cpal` capture is emitted to frontend to drive waveform

### Technical Design

**Tray icon updates (Rust `lib.rs`):**
- Create 3 icon variants embedded as PNG bytes
- `update_tray_icon(state: &str)` Tauri command swaps the icon
- Frontend calls this whenever `status` changes

**Overlay window:**
- Add second window `overlay` in `tauri.conf.json`: decorations=false, alwaysOnTop=true, transparent=true, skipTaskbar=true, width=200, height=60, bottom-center position
- `show_overlay` / `hide_overlay` Tauri commands
- `ui/src/overlay.tsx` — separate entry point for overlay window

**Audio level for waveform (Rust `audio.rs`):**
- Compute RMS of each ~50ms audio buffer chunk
- `app_handle.emit("audio-level", rms_value: f32)` during capture
- Frontend subscribes to `audio-level` events, drives bar heights via CSS transitions

**Locked mode (Frontend):**
- `lockedMode: boolean` state in `useRecordingState`
- Double-click overlay → `toggleLockedMode()` → if entering, call `handleStart()`; if exiting, call `handleStop()`

### Files to Create/Modify
- `ui/src-tauri/tauri.conf.json` — add overlay window config
- `ui/src-tauri/src/lib.rs` — `update_tray_icon`, `show_overlay`, `hide_overlay` commands
- `ui/src-tauri/src/audio.rs` — emit `audio-level` events during capture
- `ui/src/lib/hooks/useRecordingState.ts` — subscribe to `audio-level`, expose `lockedMode`, `toggleLockedMode`
- `ui/src/components/OverlayWidget.tsx` (new) — pill UI, waveform bars, lock indicator
- `ui/src/overlay.tsx` (new) — overlay window entry point

---

## FEAT-003: Custom Hotkey Binding

**Priority:** P1
**Type:** Frontend + Rust
**Branch:** `feat/custom-hotkey`
**Status:** TODO
**Depends on:** FEAT-001

### Context
Currently hotkey is a dropdown with 3 fixed combos. User wants to capture any key or combo freely. Conflict detection with macOS system shortcuts is deferred to v2.

### Acceptance Criteria
- [ ] Hotkey field in settings is a key-capture input: click/focus it, press any combo, it registers
- [ ] Supports modifier+key combos (e.g. `Cmd+Shift+D`, `Option+F1`)
- [ ] "Disable" option clears the binding (no active hotkey)
- [ ] Double-tap mode key selector also uses key-capture input
- [ ] Persists across restarts
- [ ] On hotkey change: old shortcut unregistered before new one registered (no double-registration)

### Technical Design

**`KeyCaptureInput` component (new):**
- Renders current binding as formatted text (e.g. "⌘⇧D")
- On focus: listens for `keydown`, captures modifier flags + key, prevents default
- On non-modifier `keyup`: saves binding, loses focus
- Formats to `@tauri-apps/plugin-global-shortcut` string format (e.g. "CmdOrCtrl+Shift+D")
- "Clear" button resets to disabled state

**Settings changes:**
- `HotkeyOption` type in `settings.ts` changes from union of 3 strings to `string`
- Remove `HOTKEY_OPTIONS` and `DOUBLE_TAP_KEY_OPTIONS` fixed arrays
- Update defaults to use string format

**Hook changes:**
- `useHotkeyToggle`: unregister old shortcut before registering new one on hotkey change

### Files to Create/Modify
- `ui/src/lib/settings.ts` — `HotkeyOption` → `string`, remove fixed option arrays, update defaults
- `ui/src/components/KeyCaptureInput.tsx` (new) — key capture UI component
- `ui/src/components/settings/SettingsPanel.tsx` — replace dropdowns with `KeyCaptureInput`
- `ui/src/lib/hooks/useHotkeyToggle.ts` — handle unregister before re-register

---

## FEAT-004: Word Statistics

**Priority:** P2
**Type:** Frontend only
**Branch:** `feat/word-stats`
**Status:** TODO
**Depends on:** FEAT-001

### Context
User wants Whisperflow-style stats visible on the main page. Cumulative over time, stored across restarts in localStorage.

### Acceptance Criteria
- [ ] Stats visible on main page (not behind a tab or button)
- [ ] Metrics: Total Words, Avg WPM, Total Recordings, Approx Tokens
- [ ] Cumulative — persist across restarts (localStorage)
- [ ] Updates immediately after each transcription completes
- [ ] WPM = total words / (total recording seconds / 60), rounded
- [ ] Approx tokens = total words × 1.3, rounded
- [ ] "Reset Stats" button available in settings panel

### Technical Design

**`ui/src/lib/stats.ts` (new):**
```typescript
interface DictationStats {
  totalWords: number;
  totalRecordings: number;
  totalRecordingSeconds: number;
  totalTokensApprox: number;
}
// loadStats(), saveStats(), updateStats(text, durationSeconds), resetStats(), getWPM()
```

**`StatsBar` component (new):**
- Horizontal row of 4 stat chips below header, above transcription area
- Each chip: small label + large number + unit
- Stone palette, matches FEAT-001 design language

**Wiring:**
- `useRecordingState` calls `updateStats(text, recordingDuration)` after each successful transcription

### Files to Create/Modify
- `ui/src/lib/stats.ts` (new) — stats persistence and calculation
- `ui/src/components/StatsBar.tsx` (new) — horizontal stats display
- `ui/src/lib/hooks/useRecordingState.ts` — call `updateStats` after transcription
- `ui/src/App.tsx` — render `<StatsBar />` in main content area
- `ui/src/components/settings/SettingsPanel.tsx` — add "Reset Stats" button

---

## FEAT-005: Logging System + In-App Viewer

**Priority:** P2
**Type:** Frontend + Rust
**Branch:** `feat/logging-viewer`
**Status:** TODO
**Depends on:** FEAT-001

### Context
`logging.rs` is already built — writes to `~/Library/Application Support/local-dictation/logs/app.log` with ISO timestamps and 5MB rotation. The macros `log_info!`, `log_warn!`, `log_error!` are used in `lib.rs`. Missing pieces: per-transcription timing, app close event, a Tauri command to read logs, and the in-app viewer UI.

### Acceptance Criteria
- [ ] App close is logged on `CloseRequested` window event
- [ ] Each transcription logs: recording duration (s), transcription latency (ms), word count, approx token count
- [ ] `get_log_contents(lines: usize)` Tauri command returns last N lines of `app.log`
- [ ] `clear_logs()` Tauri command truncates `app.log`
- [ ] "View Logs" button in settings panel opens a log viewer modal
- [ ] Log viewer renders each line with: timestamp, colored level badge (INFO=stone, WARN=amber, ERROR=red), message
- [ ] Log viewer has "Clear" and "Copy All" buttons

### Technical Design

**`logging.rs` additions:**
- `pub fn read_last_lines(n: usize) -> String` — reads file tail, returns joined string
- `pub fn clear_log()` — truncates `app.log`

**`lib.rs` additions:**
- In `stop_native_recording`: record `Instant::now()` before pipeline; after text ready, `log_info!("transcription: {}ms latency, {} words, ~{} tokens", ...)`
- In `on_window_event` `CloseRequested` arm: `log_info!("app closed")`
- New commands: `get_log_contents(lines: usize)`, `clear_logs()`

**`LogViewer.tsx` (new):**
- Modal, fetches log on open via `invoke('get_log_contents', { lines: 500 })`
- Parses lines: `YYYY-MM-DDTHH:MM:SSZ [LEVEL] message`
- Renders with level-colored badges, monospace font, scroll to bottom on open
- "Clear" calls `invoke('clear_logs')` then refetches; "Copy All" copies raw text

### Files to Modify
- `ui/src-tauri/src/logging.rs` — add `read_last_lines`, `clear_log`
- `ui/src-tauri/src/lib.rs` — transcription timing log, app close log, new commands, register commands
- `ui/src/components/LogViewer.tsx` (new) — log viewer modal
- `ui/src/components/settings/SettingsPanel.tsx` — "View Logs" button

---

## FEAT-006: Live System Resource Monitor

**Priority:** P3
**Type:** Frontend + Rust
**Branch:** `feat/resource-monitor`
**Status:** TODO
**Depends on:** FEAT-001

### Context
User wants a live CPU/memory usage graph in the app that visibly spikes when the Whisper pipeline runs. Lives as a collapsible inline section in the main area (no new tab).

### Acceptance Criteria
- [ ] `get_resource_usage` Tauri command returns current `{ cpu_percent: f32, memory_mb: u64 }`
- [ ] Frontend polls every 1 second, maintains rolling 60-point history
- [ ] SVG area/line chart renders in app — no external chart library
- [ ] Two lines: CPU% (stone-600) and memory MB (amber-600)
- [ ] Current values displayed as text above graph
- [ ] Visible CPU spike during transcription pipeline
- [ ] Section is collapsible — chevron toggle button
- [ ] Collapsed state persists in localStorage

### Technical Design

**`resource_monitor.rs` (new):**
- Add `sysinfo = "0.32"` to `Cargo.toml`
- `get_resource_usage()` command using `sysinfo::System` — refresh CPU/memory on each call
- Returns `ResourceUsage { cpu_percent: f32, memory_mb: u64 }`

**`useResourceMonitor` hook (new):**
- `setInterval` at 1000ms calling `invoke('get_resource_usage')`
- Maintains `readings: ResourceReading[]` capped at 60 points
- Cleans up interval on unmount

**`ResourceMonitor.tsx` (new):**
- Collapsible section with chevron button
- SVG chart: normalize readings to chart height, render as `<polyline>` paths
- Two colored lines, y-axis min/max labels, current values as text

### Files to Create/Modify
- `ui/src-tauri/Cargo.toml` — add `sysinfo = "0.32"`
- `ui/src-tauri/src/resource_monitor.rs` (new)
- `ui/src-tauri/src/lib.rs` — `mod resource_monitor`, register `get_resource_usage` command
- `ui/src/lib/hooks/useResourceMonitor.ts` (new)
- `ui/src/components/ResourceMonitor.tsx` (new)
- `ui/src/App.tsx` — render `<ResourceMonitor />` in main content area

---

## Ticket Status Summary

| Ticket | Description | Priority | Status | Depends On |
|--------|-------------|----------|--------|------------|
| Pre-Work | Merge feature/add-logging | — | TODO | — |
| FEAT-001 | Structural UI Uplift | P0 | TODO | Pre-Work |
| FEAT-002 | Status Widget | P1 | TODO | FEAT-001 |
| FEAT-003 | Custom Hotkey Binding | P1 | TODO | FEAT-001 |
| FEAT-004 | Word Statistics | P2 | TODO | FEAT-001 |
| FEAT-005 | Logging Viewer | P2 | TODO | FEAT-001 |
| FEAT-006 | Resource Monitor | P3 | TODO | FEAT-001 |
