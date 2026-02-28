# Local Dictation — Feature Tickets

**Updated:** 2026-02-19

---

## Status Summary

| Ticket | Description | Priority | Status |
|--------|-------------|----------|--------|
| Pre-Work | Merge feature/add-logging | — | ✅ Done |
| FEAT-001 | Structural UI Uplift | P0 | ✅ Done |
| FEAT-002 | Status Widget | P1 | ✅ Done |
| FEAT-003 | Custom Hotkey Binding | P1 | ✅ Done |
| FEAT-004 | Word Statistics | P2 | ✅ Done |
| FEAT-005 | Logging Viewer | P2 | TODO |
| FEAT-006 | Resource Monitor | P3 | TODO |

---

## FEAT-004: Word Statistics

**Priority:** P2
**Type:** Frontend only
**Branch:** `feat/word-stats`
**Depends on:** FEAT-001

### Context
Whisperflow-style stats visible on the main page. Cumulative over time, stored across restarts in localStorage.

### Acceptance Criteria
- [ ] Stats visible on main page (not behind a tab or button)
- [ ] Metrics: Total Words, Avg WPM, Total Recordings, Approx Tokens
- [ ] Cumulative — persist across restarts (localStorage)
- [ ] Updates immediately after each transcription completes
- [ ] WPM = total words / (total recording seconds / 60), rounded
- [ ] Approx tokens = total words × 1.3, rounded
- [ ] "Reset Stats" button available in settings panel

### Technical Design

**`app/src/lib/stats.ts` (new):**
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
- `app/src/lib/stats.ts` (new) — stats persistence and calculation
- `app/src/components/StatsBar.tsx` (new) — horizontal stats display
- `app/src/lib/hooks/useRecordingState.ts` — call `updateStats` after transcription
- `app/src/App.tsx` — render `<StatsBar />` in main content area
- `app/src/components/settings/SettingsPanel.tsx` — add "Reset Stats" button

---

## FEAT-005: Logging System + In-App Viewer

**Priority:** P2
**Type:** Frontend + Rust
**Branch:** `feat/logging-viewer`
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
- In `stop_native_recording`: record `Instant::now()` before pipeline; after text ready, log transcription stats
- In `on_window_event` `CloseRequested` arm: `log_info!("app closed")`
- New commands: `get_log_contents(lines: usize)`, `clear_logs()`

**`LogViewer.tsx` (new):**
- Modal, fetches log on open via `invoke('get_log_contents', { lines: 500 })`
- Parses lines: `YYYY-MM-DDTHH:MM:SSZ [LEVEL] message`
- Renders with level-colored badges, monospace font, scroll to bottom on open
- "Clear" calls `invoke('clear_logs')` then refetches; "Copy All" copies raw text

### Files to Modify
- `app/src-tauri/src/logging.rs` — add `read_last_lines`, `clear_log`
- `app/src-tauri/src/lib.rs` — transcription timing log, app close log, new commands
- `app/src/components/LogViewer.tsx` (new) — log viewer modal
- `app/src/components/settings/SettingsPanel.tsx` — "View Logs" button

---

## FEAT-006: Live System Resource Monitor

**Priority:** P3
**Type:** Frontend + Rust
**Branch:** `feat/resource-monitor`
**Depends on:** FEAT-001

### Context
Live CPU/memory usage SVG chart that spikes during transcription. Lives as a collapsible inline section in the main area.

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
- `get_resource_usage()` command using `sysinfo::System`
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
- `app/src-tauri/Cargo.toml` — add `sysinfo = "0.32"`
- `app/src-tauri/src/resource_monitor.rs` (new)
- `app/src-tauri/src/lib.rs` — `mod resource_monitor`, register `get_resource_usage` command
- `app/src/lib/hooks/useResourceMonitor.ts` (new)
- `app/src/components/ResourceMonitor.tsx` (new)
- `app/src/App.tsx` — render `<ResourceMonitor />` in main content area
