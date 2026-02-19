# Agent Prompt Templates

This file contains ready-to-use prompts for spawning subagents to implement each feature ticket. Each prompt is self-contained — the agent can pick it up and work independently.

---

## How to Use

1. Copy the prompt for the ticket you want to work on
2. Spawn a new Claude Code agent (or start a new session) with that prompt
3. The agent will read the necessary files, implement the feature, and open a PR

---

## General Agent Prompt (Pre-Work: Merge Current Branch)

```
You are implementing Pre-Work for the Local Dictation project.

**Task:** Commit all staged/unstaged changes on the `feature/add-logging` branch and open a PR to merge into `main`.

**Setup — read these first:**
- /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md (project overview and conventions)
- /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (Pre-Work section)

**Files to commit:**
- ui/src-tauri/src/keyboard.rs
- ui/src/lib/hooks/useDoubleTapToggle.ts
- ui/src/App.tsx
- ui/src/components/settings/SettingsPanel.tsx
- ui/src/lib/hooks/useHotkeyToggle.ts
- ui/src/lib/settings.ts
- ui/src-tauri/src/lib.rs
- CHANGELOG.md, CLAUDE.md, README.md, ui/src-tauri/Cargo.toml, ui/src-tauri/Cargo.lock

**Steps:**
1. Read CLAUDE.md and the Pre-Work section of TICKETS_FEATURES.md
2. Run `git status` to confirm what is unstaged/untracked
3. Stage and commit all listed files with a clear commit message
4. Verify the build passes: `cd ui && npm run tauri build` (or at minimum `npm run build`)
5. Open a PR from `feature/add-logging` → `main` using `gh pr create`
```

---

## FEAT-001: Structural UI Uplift

```
You are implementing FEAT-001 (Structural UI Uplift) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-001 section)
3. ~/.claude/plans/jolly-tumbling-hippo.md (full color table, layout diagram, and file-by-file instructions)

**Then read the files you will modify:**
- ui/src/App.tsx
- ui/src/components/StatusHeader.tsx
- ui/src/components/settings/SettingsPanel.tsx
- ui/src/components/TranscriptionView.tsx
- ui/src/components/RecordingControls.tsx
- ui/src/components/history/HistoryPanel.tsx
- ui/src/components/PermissionsBanner.tsx
- ui/src/components/AboutModal.tsx
- ui/src-tauri/tauri.conf.json

**Work on branch:** `feat/ui-uplift` (create from main after Pre-Work merges)

**Important constraints:**
- No new features — purely structural and cosmetic changes
- Follow the file modification order in the ticket to avoid breaking imports
- Delete TabNavigation.tsx only after removing all imports of it
- Test both light and dark mode after changes
- Verify layout at minimum window size (520px wide)

**Verification:**
1. `cd ui && npm run tauri dev`
2. App opens at 720×560, horizontal layout with inline sidebar
3. Gear icon in header opens sidebar with animated width transition
4. No tabs visible — single content feed
5. Stone palette in both light and dark mode
6. Recording → processing → idle status colors cycle correctly

Open a PR to `main` when complete.
```

---

## FEAT-002: Status Widget

```
You are implementing FEAT-002 (Status Widget) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-002 section)

**Then read the files you will modify:**
- ui/src-tauri/src/lib.rs
- ui/src-tauri/src/audio.rs
- ui/src-tauri/tauri.conf.json
- ui/src/lib/hooks/useRecordingState.ts

**Work on branch:** `feat/status-widget` (create from main after FEAT-001 merges)

**Key decisions:**
- The overlay window is a separate Tauri window with its own entry point (ui/src/overlay.tsx)
- Audio level data flows: cpal buffer → RMS calculation in audio.rs → app_handle.emit("audio-level", f32) → frontend useRecordingState hook → OverlayWidget waveform bars
- Locked mode is frontend state only — it calls the existing handleStart/handleStop functions
- Use set_is_main_thread(false) pattern already established in keyboard.rs if any main-thread work is needed

**Verification:**
1. Tray icon changes when recording starts/stops
2. Overlay pill appears at bottom-center, always on top
3. Waveform bars animate while recording
4. Double-click overlay → locked mode (recording continues after releasing hotkey)
5. Click overlay again → recording stops

Open a PR to `main` when complete.
```

---

## FEAT-003: Custom Hotkey Binding

```
You are implementing FEAT-003 (Custom Hotkey Binding) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-003 section)

**Then read the files you will modify:**
- ui/src/lib/settings.ts
- ui/src/components/settings/SettingsPanel.tsx
- ui/src/lib/hooks/useHotkeyToggle.ts
- ui/src/lib/hotkey.ts (understand current shortcut registration format)

**Work on branch:** `feat/custom-hotkey` (create from main after FEAT-001 merges)

**Key decisions:**
- The KeyCaptureInput component captures keydown events, builds a shortcut string in the format expected by @tauri-apps/plugin-global-shortcut
- MacOS modifier symbols for display: ⌘ Cmd, ⌥ Option, ⌃ Ctrl, ⇧ Shift
- A "Disable" state means settings.hotkey = "" and useHotkeyToggle skips registration
- Double-tap mode key is still selected from a fixed set (modifier keys only) — only hotkey combo mode gets free-form capture

**Verification:**
1. Click hotkey field → press Cmd+Shift+D → field shows "⌘⇧D", recording toggles with that combo
2. Click "Clear" → hotkey disabled, pressing old combo does nothing
3. Setting persists after app restart
4. Switching hotkeys unregisters old one before registering new one

Open a PR to `main` when complete.
```

---

## FEAT-004: Word Statistics

```
You are implementing FEAT-004 (Word Statistics) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-004 section)

**Then read the files you will modify:**
- ui/src/App.tsx
- ui/src/lib/hooks/useRecordingState.ts
- ui/src/lib/history.ts (understand the localStorage pattern to follow the same approach)
- ui/src/components/settings/SettingsPanel.tsx

**Work on branch:** `feat/word-stats` (create from main after FEAT-001 merges)

**Key decisions:**
- Follow the exact same localStorage pattern as history.ts for stats.ts
- Word count = text.trim().split(/\s+/).filter(Boolean).length
- Token count = Math.round(wordCount * 1.3)
- WPM = Math.round(totalWords / (totalRecordingSeconds / 60))
- StatsBar is always visible (not collapsible) — it lives between the header and the transcription area
- Use stone palette from FEAT-001 for all styling

**Verification:**
1. Dictate something → stats update immediately (word count increases, recording count increases)
2. Close and reopen app → stats are preserved
3. Reset Stats button in settings → all stats return to zero
4. WPM value is reasonable (120–200 for normal speech)

Open a PR to `main` when complete.
```

---

## FEAT-005: Logging Viewer

```
You are implementing FEAT-005 (Logging System + In-App Viewer) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-005 section)

**Then read the files you will modify:**
- ui/src-tauri/src/logging.rs (understand current implementation — read_last_lines and clear_log need to be added)
- ui/src-tauri/src/lib.rs (understand stop_native_recording to know where to add timing)
- ui/src/components/settings/SettingsPanel.tsx

**Work on branch:** `feat/logging-viewer` (create from main after FEAT-001 merges)

**Key decisions:**
- Transcription timing: use std::time::Instant before calling run_transcription_pipeline, calculate elapsed after it returns
- Word count from transcription text: text.split_whitespace().count()
- Token count: (word_count as f32 * 1.3).round() as usize
- Log line format is already set: "YYYY-MM-DDTHH:MM:SSZ [LEVEL] message"
- Parse log lines in the frontend using this format for badge coloring
- The log viewer is a modal (not a page/tab) — opened from a button in the settings panel

**Verification:**
1. Open app → check log file at ~/Library/Application Support/local-dictation/logs/app.log — should have "app setup" entry
2. Dictate something → log should show transcription timing entry
3. Close app → log should have "app closed" entry
4. Open settings → click "View Logs" → modal shows log entries with colored badges
5. "Clear" button empties the log file
6. "Copy All" copies raw log text to clipboard

Open a PR to `main` when complete.
```

---

## FEAT-006: Resource Monitor

```
You are implementing FEAT-006 (Live System Resource Monitor) for the Local Dictation project.

**Setup — read these first:**
1. /Users/georgenijo/Documents/code/local-dictation/CLAUDE.md
2. /Users/georgenijo/Documents/code/local-dictation/docs/TICKETS_FEATURES.md (FEAT-006 section)

**Then read the files you will modify:**
- ui/src-tauri/Cargo.toml (to add sysinfo dependency)
- ui/src-tauri/src/lib.rs (to add mod and register command)
- ui/src/App.tsx

**Work on branch:** `feat/resource-monitor` (create from main after FEAT-001 merges)

**Key decisions:**
- sysinfo version: "0.32" — use System::new_with_specifics with only CpuRefresh and MemoryRefresh enabled for performance
- CPU refresh requires two calls with a delay (sysinfo limitation) — cache the System instance in a Mutex<Option<System>> static, call refresh_cpu_usage() between polls rather than creating new System each time
- SVG chart: 60 data points, chart width/height from component props, use polyline not path for simplicity
- Normalize CPU to 0–100, normalize memory to 0–maxMemoryMB (from sysinfo total_memory)
- Collapsed state key in localStorage: "resource-monitor-collapsed"
- Do NOT poll when the section is collapsed (pause the interval)

**Verification:**
1. Open app → resource monitor section visible with live updating graph
2. Start a transcription → CPU line spikes visibly during whisper processing
3. Collapse section → graph stops updating (interval paused)
4. Expand section → graph resumes
5. Collapsed state remembered after app restart

Open a PR to `main` when complete.
```
