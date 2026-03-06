# Agent 1 — Frontend UI Findings

## User-Facing Features

### Main Window — StatusHeader
- **App title** "Murmur" displayed in the top-left of the header bar.
- **Status indicator** with colored dot and text showing one of four states:
  - **Initializing...** (stone/gray) — app not yet ready.
  - **Ready** (emerald/green) — app initialized, idle.
  - **Recording Xs** (red, pulsing mic icon) — actively recording; shows elapsed seconds via `recordingDuration` prop.
  - **Processing...** (amber, spinning SVG) — transcription in progress.
- **Settings gear button** in the top-right corner; toggles a side panel. Has `aria-expanded` and `aria-label="Toggle settings"` for accessibility.
- Header is a Tauri drag region (`data-tauri-drag-region`), meaning the user can drag the window by grabbing the header.

### Main Window — RecordingControls
- **Start Recording button** — dark stone/light button with a filled circle icon. Disabled when `initialized` is false or `status === 'processing'`. Text reads "Start Recording" or "Processing..." when processing.
- **Stop Recording button** — red button with a square icon. Only visible when `status === 'recording'`. Disabled when `initialized` is false.
- Callbacks: `onStart` and `onStop` are passed in from the parent (App).

### Main Window — TranscriptionView / HistoryPanel
- **TranscriptionView** is a thin wrapper that renders `HistoryPanel`.
- **Empty state** — when no entries: clock icon, "No transcription history yet" message, and "Your transcriptions will appear here" subtitle.
- **History list** — reverse-chronological (newest first). Each entry is a clickable button that copies the text to clipboard via `navigator.clipboard.writeText()`.
  - Shows timestamp (formatted via `formatTimestamp` from `lib/history`), duration (formatted as `Xs` or `Xm Ys`), and the transcribed text.
  - On copy: the entry highlights green and shows "Copied!" badge for 2 seconds; otherwise shows a copy icon.
  - Text content has `max-h-32` with `overflow-y-auto`, so long transcriptions are scrollable within the card.
- **Clear History button** — at the bottom of the panel. Uses `window.confirm()` dialog ("Are you sure you want to clear all history?") before clearing. Calls `clearHistory()` from `lib/history` and then `onClearHistory` callback.

### Main Window — StatsBar
- Displays four stat chips in a horizontal row:
  - **Total Words** — from `stats.totalWords`.
  - **Avg WPM** — words per minute computed by `getWPM(stats)`. Shows dash when zero.
  - **Recordings** — from `stats.totalRecordings`.
  - **Approx Tokens** — computed by `getApproxTokens(stats)`. Shows dash when zero.
- Stats are loaded from localStorage via `loadStats()` and refresh when `statsVersion` prop changes.
- Each chip is a centered vertical layout: value on top, label beneath.

### Main Window — ResourceMonitor
- **Collapsible panel** — toggled via clicking the header row. Collapse state persisted to localStorage under key `resource-monitor-collapsed`.
- **Header row** always shows:
  - "Resources" label (uppercase, small).
  - Current CPU percentage.
  - Current memory in MB (amber-colored label).
  - Chevron icon that rotates when collapsed vs expanded.
- **Expanded chart** — SVG polyline chart with two lines:
  - CPU % (stone color, CSS var `--cpu-stroke`).
  - Memory MB (amber color, CSS var `--mem-stroke`).
  - Grid lines at 25%, 50%, 75%.
  - Legend beneath with color swatches.
- Uses `useResourceMonitor` hook, only polling when expanded (performance optimization: `!isCollapsed` passed as enable flag).
- Displays up to 60 readings (`MAX_READINGS = 60`).

### Main Window — PermissionsBanner
- **Conditional banner** shown at the top when either microphone or accessibility permission is denied. Hidden when all granted, when dismissed, or while still checking.
- Shows two permission rows:
  - **Microphone** — green dot if granted, red if denied. "Required for recording" text. "Open Settings" link calls `invoke('request_microphone_permission')`.
  - **Accessibility** — green dot if granted, red if denied. "Required for text pasting" text. "Open Settings" link calls `invoke('request_accessibility_permission')`.
- **Re-check permissions** link button at bottom to manually re-check.
- **Dismiss button** (X icon) hides the banner for the session (not persisted).
- Microphone check uses `navigator.mediaDevices.getUserMedia({ audio: true })`.
- Accessibility check uses `invoke('check_accessibility_permission')`.
- Automatically re-checks on window focus events.

### Settings Panel (side drawer)
- **Slides in/out** from the right side; width transitions from `w-0` to `w-[280px]` with overflow hidden/auto.
- **Close button** (X icon) in the panel header.
- Organized into collapsible `SettingsSection` components with title + optional subtitle:

#### Section: Transcription (Model, microphone)
- **Transcription Model selector** — custom `Select` dropdown with two option groups:
  - "Moonshine (Fast, CPU)" group: options from `MOONSHINE_MODELS`.
  - "Whisper (Metal GPU)" group: options from `WHISPER_MODELS`.
  - Each option shows label + size (e.g., "66MB").
  - Disabled while recording is active; shows amber warning "Stop recording before changing model."
  - Help text: "Moonshine runs on CPU; Whisper uses Metal GPU. Larger models are more accurate but slower."
- **Inline model download** — when the selected model is not downloaded (`modelAvailable === false`):
  - Shows amber warning "Model not downloaded" with a "Download" link.
  - While downloading: progress bar with percentage and "Downloading..." text.
  - On error: red error banner with the error message and a "Retry" link.
  - Uses `invoke('check_specific_model_exists', { modelName })` to check availability.
  - Uses `invoke('download_model', { modelName })` and listens to `download-progress` event for progress updates.
- **Microphone selector** — custom `Select` dropdown.
  - Default option: "System Default" (`system_default`).
  - Lists all audio devices fetched via `invoke('list_audio_devices')` (fetched whenever the panel opens).
  - Disabled while recording is active.
  - Shows amber warning if the saved device is not found in the device list: "Selected device not found -- will use System Default."

#### Section: Recording (Trigger mode, shortcut key)
- **Voice Detection / VAD Sensitivity slider** — range 0-100%, step 5. Draft value shown while dragging; committed on pointer up.
  - Help: "Higher = keeps more audio. Lower = trims silence more aggressively."
- **Recording Trigger mode selector** — three toggle buttons rendered from `RECORDING_MODE_OPTIONS`:
  - Modes are: hold-down, double-tap, and both (inferred from code).
  - Disabled while recording; shows amber warning.
- **Accessibility permission warning** — shown when `accessibilityGranted === false`. "Accessibility permission required for keyboard detection" with "Grant" link that calls `invoke('request_accessibility_permission')`.
- **Trigger Key selector** — custom `Select` dropdown from `DOUBLE_TAP_KEY_OPTIONS`.
  - Label changes based on mode: "Hold Key" (hold-down), "Double-Tap Key" (double-tap), or "Trigger Key" (both).
  - Help text changes similarly:
    - Hold: "Hold to start recording, release to stop"
    - Double-tap: "Double-tap to start recording, single tap to stop"
    - Both: "Hold to record, or double-tap to start and single tap to stop"
  - Disabled while recording.

#### Section: Output (Auto-paste, launch at login)
- **Auto-Paste toggle** — styled switch (`role="switch"`, `aria-checked`). Toggles `settings.autoPaste`.
  - Description: "Automatically paste transcription (requires Accessibility permission)".
  - When enabled, shows accessibility permission status: green "Accessibility permission granted" or amber "Accessibility permission required" with "Grant" link.
  - When enabled, reveals the **Paste Delay slider** — range 10-500ms, step 10ms. Draft value shown while dragging; committed on pointer up.
    - Help: "Delay before paste. Increase if paste lands in the wrong window."
- **Launch at Login toggle** — styled switch (`role="switch"`, `aria-checked`). Toggles `settings.launchAtLogin`.
  - Description: "Automatically start when you log in".

#### Section: About (Stats, logs, updates) — collapsed by default
- **Model Info** — static display showing: Model name, Backend (Moonshine CPU or Whisper Metal GPU), Size.
- **Reset Stats button** — two-click confirmation pattern: first click shows "Confirm Reset" (red-styled, auto-resets after 3 seconds), second click resets.
- **View Logs button** — calls `onViewLogs` callback.
- **Check for Updates button** — calls `onCheckForUpdate` callback. Disabled when update status is `checking` or `downloading`.
  - Shows status text: "Checking...", "You're up to date", "vX.Y.Z available", or "Update check failed".
- **Version display** — app version string fetched via `getVersion()`.

### Model Downloader (initial setup screen)
- **Full-screen download view** shown before any model is available.
- Presents a curated subset of 4 models: `moonshine-tiny`, `moonshine-base`, `large-v3-turbo`, `base.en`.
- Each model shown as a selectable card with: label, size (monospace, right-aligned), description text.
  - Descriptions: "Fastest -- sub-20ms for typical dictation", "Better accuracy, still very fast", "Highest accuracy, slower (1-2 seconds)", "Good balance of speed and accuracy".
- Default selection: first model (moonshine-tiny).
- **Download button** — text changes: "Download" -> "Downloading..." -> "Retry Download" on error.
- **Progress bar** — shows percentage, "Starting..." before total is known, and "X.X / Y.Y MB" bytes counter.
- **Error display** — red banner with error message.
- Selection disabled during download.
- On completion: calls `onComplete` callback.

### About Modal
- **Modal dialog** with backdrop overlay (semi-transparent dark).
- Microphone SVG icon in a dark rounded square.
- App name "Murmur", version string fetched from `getVersion()`.
- Description: "Privacy-first voice-to-text powered by Whisper AI. All processing happens locally on your device."
- Copyright: "(c) 2026 Murmur".
- Close button. Backdrop click also closes.

### Update Modal
- **Modal dialog** for app updates with four phases:
  - **available** — "Update Available" (or "Required Update" if forced). Shows version number, optional release notes rendered as Markdown (using `react-markdown` + `rehype-sanitize`). Buttons: "Update Now", "Skip This Version" (non-forced only), "Later" (non-forced only).
  - **downloading** — progress bar with percentage text and "Downloading..." label.
  - **ready** — "Installing and relaunching..." text.
  - **error** — red error banner with message. "Retry" button.
- **Forced update behavior**: backdrop click disabled, no Skip/Later buttons, shows "Quit" button (calls `exit(0)` from `@tauri-apps/plugin-process`), amber warning "This update is required to continue using the app."
- Close button (X) shown only on non-forced available and non-forced error states.
- Non-visible phases (`idle`, `checking`, `up-to-date`) return `null`.

### Overlay Widget (Dynamic Island / notch overlay window)
- **Separate window** that renders a "Dynamic Island"-style UI anchored to the macOS notch area.
- **Notch-aware sizing**: fetches notch dimensions via `invoke('get_notch_info')` on mount. Listens to `notch-info-changed` event for display configuration changes (monitor plug/unplug).
- **Three visual states**:
  - **Idle** — small mic SVG icon (dimmed, white/40% opacity). Width = `notchWidth + 28`.
  - **Recording** — expands width to `notchWidth + 68`. Red pulsing dot on left, animated waveform bars on right.
  - **Processing** — same expanded width. Spinning circle on left, dimmed waveform on right.
- **Waveform animation** — 7 bars (`BAR_COUNT = 7`), animated via `requestAnimationFrame` loop with direct DOM manipulation (no React state). Uses `audio-level` events from Rust for reactive audio visualization. Center bars are taller (envelope shaping). Random jitter for organic feel.
- **Island styling**: dark background (`rgba(20, 20, 20, 0.92)`), 40px blur, rounded bottom corners, spring-like transition (`cubic-bezier(0.34,1.56,0.64,1)` over 500ms).
- **Mouse interactions**:
  - **Single click** (250ms debounced): stops recording if currently recording. Exits locked mode.
  - **Double click**: toggles "locked mode." First double-click starts recording (`invoke('start_native_recording', { deviceName })`), second double-click stops recording (`invoke('stop_native_recording')`). Reads microphone setting from localStorage (since overlay has no React settings context).
  - **mousedown** logging for diagnostics.
- **Locked mode** state: tracks whether recording was initiated by double-click on the overlay.
- Drag region: entire overlay is a Tauri drag region (`data-tauri-drag-region`).
- Logs extensively via `flog` (info-level).

### Log Viewer Window (LogViewerApp)
- **Separate window** for viewing structured application events.
- **Two tabs**: "Events" and "Metrics".
- **Events tab**:
  - **Stream filter chips** (StreamChips) — toggle which event streams to show. Default active: `pipeline`, `audio`, `system`. Uses colored chips from `STREAM_COLORS`.
  - **Level filter** (LevelFilter) — toggle which log levels to show: `info`, `warn`, `error`. All active by default.
  - **Event list** — scrollable, monospace font, auto-scrolls to bottom when new events arrive (disengages auto-scroll if user scrolls up; re-engages when user scrolls near bottom within 40px).
  - Each **EventRow** shows: timestamp (time portion only), stream chip (colored), level label (uppercase, colored), summary text. If event has data, row is clickable/expandable to show JSON data in a `<pre>` block.
  - **Copy All button** — copies all filtered events as text lines: `{timestamp} [{stream}] {LEVEL} {summary}`.
  - **Clear button** — clears all events via `useEventStore().clear()`.
  - Empty state: "No events to display".
- **Metrics tab** (MetricsView):
  - Extracts transcription timing metrics from pipeline events where `summary === 'transcription complete'`.
  - Shows up to last 20 transcriptions.
  - **Toggleable series legend** — four series: Total, Inference, VAD, Paste. Click to show/hide (at least one must remain visible).
  - **Stat cards** — one per visible series showing: latest value, average, trend indicator (up arrow red, down arrow green, or dash for flat). Trend threshold: 10% deviation from average.
  - **Two line charts** (SVG):
    - Upper: Total + Inference (taller, 150px).
    - Lower: VAD + Paste (shorter, 120px).
    - Y-axis: auto-scaled with "nice" round numbers. Three tick marks (0, mid, max).
    - X-axis: transcription index (1-based).
    - Color coding: Total = stone-600, Inference = amber-500, VAD = stone-400, Paste = slate-500.
    - Polylines with dots at each data point.
  - Empty state: "No transcription data yet. Complete a recording to see metrics."

### UI Primitives

#### Select Component (`ui/Select.tsx`)
- Custom dropdown/combobox replacement for `<select>`.
- Supports flat options (`SelectOption[]`) or grouped options (`SelectGroup[]`).
- **Full keyboard navigation**: Enter/Space to open/select, ArrowUp/Down to navigate, Home/End for first/last, Escape to close, Tab to close and move focus.
- **ARIA attributes**: `role="combobox"`, `aria-expanded`, `aria-haspopup="listbox"`, `aria-activedescendant`, `role="option"`, `aria-selected`, `role="group"`, `aria-labelledby`.
- Click-outside-to-close via `mousedown` listener on `document`.
- Highlighted option scrolls into view automatically.
- Selected option shows a checkmark icon.
- Chevron rotates when open.
- Supports `disabled` state (grayed out, `cursor-not-allowed`).
- Supports `placeholder` text.

#### SettingsSection Component (`settings/SettingsSection.tsx`)
- Collapsible accordion section.
- Props: `title`, optional `subtitle` (shown when collapsed), `defaultExpanded` (default `true`), `children`.
- Animated collapse/expand using CSS `grid-template-rows` transition.
- Overflow handling: overflow set to `visible` only after expand transition ends (prevents clipping during animation).
- ARIA: `aria-expanded`, `aria-controls`, `role="region"`, `aria-labelledby`.
- Chevron icon rotates on toggle.

## Internal Systems

### Overlay Widget State Machine
- Three states: `idle`, `recording`, `processing` (type `DictationStatus`).
- Status driven by `recording-status-changed` Tauri events.
- When status goes to `idle`, `lockedMode` is automatically reset to `false`.
- `lockedMode` is an internal boolean tracking whether the overlay initiated the recording (vs. keyboard-initiated).

### Overlay Audio Visualization Pipeline
- Listens to `audio-level` Tauri event (payload: `number`).
- Stores audio level in a ref (no React re-render).
- `requestAnimationFrame` loop reads the ref and updates 7 bar elements via direct DOM manipulation (`el.style.height`).
- Bar heights computed from: baseline (random jitter), center-weighted envelope, audio level (scaled x16, capped at 1), and a squared boost with random factor.
- Animation only runs when `status === 'recording'`; bars reset to 2px otherwise.

### Overlay Click Disambiguation
- Uses a 250ms debounce timer (`clickTimerRef`) to distinguish single-click from double-click.
- `handleClick` sets a 250ms timeout for single-click behavior.
- `handleDoubleClick` cancels any pending single-click timer before executing.
- Single-click: always stops recording (if recording).
- Double-click: toggles locked mode and starts/stops recording accordingly.

### Model Availability Checking (Settings Panel)
- On model selection change: calls `invoke('check_specific_model_exists', { modelName })`.
- Tracks availability in `modelAvailable` state (null = checking, true/false).
- Stale-request protection: uses a `stale` flag in the effect cleanup.

### Model Download Pipeline (Settings Panel + ModelDownloader)
- Both components use the same pattern:
  - Call `invoke('download_model', { modelName })`.
  - Listen to `download-progress` event with payload `{ received: number; total: number }`.
  - Track download state: `idle` -> `downloading` -> (success or `error`).
  - Clean up event listener on unmount via ref.
- Settings panel additionally tracks which model the download was initiated for (`downloadModelRef`), preventing stale progress updates from a previously selected model.

### Permission Checking (PermissionsBanner)
- Microphone: checked via Web API `navigator.mediaDevices.getUserMedia({ audio: true })`. Stream tracks are immediately stopped after confirmation.
- Accessibility: checked via Tauri command `check_accessibility_permission`.
- Re-checked automatically on window `focus` event.
- Can be manually re-triggered via "Re-check permissions" link.

### Resource Monitoring
- Delegates to `useResourceMonitor` hook.
- Only polls when the panel is expanded (the `enabled` parameter controls polling).
- Maintains a rolling window of up to 60 `ResourceReading` objects.
- Each reading contains `cpu_percent` and `memory_mb`.

### Stats System
- `loadStats()` and related functions from `lib/stats` provide `DictationStats`.
- Computed values: `getWPM(stats)` for words-per-minute, `getApproxTokens(stats)` for estimated token count.
- Stats refresh is driven by a `statsVersion` prop (bumped externally).

### Event Store (Log Viewer)
- `useEventStore` hook provides `{ events, clear }`.
- Events are `AppEvent` objects with: `timestamp`, `stream`, `level`, `summary`, `data`.
- Events are filtered by both stream and level toggles.
- Auto-scroll logic: tracks whether user is scrolled near bottom (within 40px threshold).

### Update System
- `UpdateStatus` type (from `lib/updater`) drives the update modal with phases: `idle`, `checking`, `up-to-date`, `available`, `downloading`, `ready`, `error`.
- `available` phase includes: `version`, `notes` (markdown string), `isForced` boolean.
- `downloading` phase includes: `version`, `progress` (percentage number).
- `ready` phase includes: `version`.
- `error` phase includes: `message`, `isForced`.
- Forced updates block dismissal and offer only "Update Now" / "Quit".

## Commands / Hooks / Events

### Tauri Commands (invoke)
- `download_model({ modelName: string })` — Downloads a transcription model file.
- `check_specific_model_exists({ modelName: string })` — Returns `boolean` indicating if model file is present on disk.
- `check_accessibility_permission()` — Returns `boolean` for macOS accessibility permission status.
- `request_accessibility_permission()` — Opens macOS accessibility settings prompt.
- `request_microphone_permission()` — Opens macOS microphone settings prompt.
- `list_audio_devices()` — Returns `string[]` of available audio input device names.
- `get_notch_info()` — Returns `{ notch_width: number; notch_height: number } | null` for the current display's notch dimensions.
- `start_native_recording({ deviceName: string | null })` — Starts audio recording with optional specific device.
- `stop_native_recording()` — Stops the active audio recording.

### Tauri Events (listen)
- `download-progress` — Payload: `{ received: number; total: number }`. Emitted during model download.
- `recording-status-changed` — Payload: `string` (validated via `isDictationStatus`). Emitted when recording state changes.
- `notch-info-changed` — Payload: `{ notch_width: number; notch_height: number } | null`. Emitted when display configuration changes.
- `audio-level` — Payload: `number`. Emitted during recording with current audio input level.

### React Hooks (referenced from components)
- `useResourceMonitor(enabled: boolean)` — Returns `ResourceReading[]`. From `lib/hooks/useResourceMonitor`.
- `useEventStore()` — Returns `{ events: AppEvent[]; clear: () => void }`. From `lib/hooks/useEventStore`.

### Tauri APIs Used
- `getVersion()` from `@tauri-apps/api/app` — Returns app version string.
- `invoke()` from `@tauri-apps/api/core` — Calls Rust backend commands.
- `listen()` from `@tauri-apps/api/event` — Subscribes to Rust-emitted events.
- `exit(0)` from `@tauri-apps/plugin-process` — Quits the application (used in forced update quit button).

### External Libraries Used in Components
- `react-markdown` with `rehype-sanitize` — Used in UpdateModal for rendering release notes.

## Gaps / Unclear

### Overlay Position Persistence (Disabled)
- Two TODO comments in `OverlayWidget.tsx` (lines 46-62, 64-65) indicate that overlay position save/restore is intentionally disabled: "re-enable after notch positioning is stable." Both saving (on window move) and restoring (on mount) are commented out. References a `POSITION_KEY` constant and `PhysicalPosition` API that are not currently imported.

### Overlay Reads Settings from localStorage Directly
- `OverlayWidget.tsx` reads `localStorage.getItem(STORAGE_KEY)` and parses the microphone setting manually (lines 165-174) because it notes "overlay has no React settings context." This creates a tight coupling to the localStorage schema and means overlay uses the raw `STORAGE_KEY` and `DEFAULT_SETTINGS` imports. If the settings schema changes, the overlay could silently break.

### No Notch Fallback Width Consistency
- When notch info is null (no notch detected), `notchWidth` defaults to `185` on initial state (line 15) but is set to `140` when the notch is removed at runtime via the `notch-info-changed` event (line 97). These two fallback values differ without explanation.

### Click Debounce Timing
- The overlay's 250ms click debounce (line 208) is a hardcoded magic number. This could cause issues on slower input devices or accessibility contexts where click timing differs.

### No Error Handling for Clipboard Copy in HistoryPanel
- `handleCopy` in HistoryPanel (line 12-19) catches errors but only logs them to `console.error`. No user-visible feedback is shown on copy failure.

### PermissionsBanner Uses Web getUserMedia for Mic Check
- The microphone permission check uses `navigator.mediaDevices.getUserMedia` (browser API) rather than a Tauri command. This is inconsistent with how accessibility is checked (via Tauri command) and may not reflect the actual system-level microphone permission status accurately in all cases (e.g., when the WebView permission differs from macOS permission).

### SettingsPanel Reset Confirmation Timer
- The 3-second auto-reset for the "Confirm Reset" state (line 104) uses `setTimeout` but the timeout ref is not cleared in the component's cleanup/unmount. If the panel unmounts during the 3-second window, this could cause a React state update on an unmounted component.

### ModelDownloader Limited Model Subset
- `DOWNLOAD_MODEL_KEYS` (line 14) is a hardcoded subset of 4 models from `MODEL_OPTIONS`. If new models are added to `MODEL_OPTIONS`, they will not automatically appear on the download screen.

### No Loading State for Audio Device Enumeration
- When `list_audio_devices` is being fetched (SettingsPanel line 180-183), there is no loading indicator. The dropdown may momentarily show only "System Default" before devices populate.

### MetricsView Hardcoded 20-Transcription Window
- `extractMetrics` (MetricsView.tsx line 25) takes only the last 20 transcription-complete events (`.slice(-20)`). This limit is not configurable.

### EventRow Key Strategy
- EventRow uses `key={${event.timestamp}-${i}}` which relies on index, potentially causing React reconciliation issues if events are cleared and new events have the same timestamp at the same index.

### Update Modal Markdown Rendering
- Release notes are rendered with `react-markdown` and `rehype-sanitize`. The sanitization config uses defaults from `rehype-sanitize` — no custom schema is provided. This should be safe but means any custom HTML in release notes would be stripped.

## Notes

### Dark Mode Support
- Every single component implements dark mode via Tailwind `dark:` variant classes. The entire UI is fully dark-mode compatible.

### Accessibility (a11y)
- The custom `Select` component has thorough ARIA support: combobox role, listbox, options, groups, active descendant tracking, full keyboard navigation (arrows, Home, End, Escape, Enter, Space, Tab).
- `SettingsSection` has proper accordion ARIA: `aria-expanded`, `aria-controls`, `role="region"`, `aria-labelledby`.
- Toggle switches use `role="switch"` and `aria-checked`.
- LevelFilter and StreamChips use `aria-pressed` on toggle buttons.
- EventRow uses `role="button"`, `tabIndex`, and keyboard handlers for expandable rows.
- Some components are less accessible: `RecordingControls` buttons lack `aria-label`, `HistoryPanel` entries are buttons but have no role annotation beyond implicit button semantics.

### Styling Patterns
- Consistent use of Tailwind CSS with stone color palette for neutral tones.
- Transitions on almost all interactive elements.
- CSS custom properties (`--cpu-stroke`, `--mem-stroke`) used for theme-aware SVG stroke colors in ResourceMonitor.
- Backdrop blur used in StatusHeader and OverlayWidget.
- Spring-like cubic-bezier animations in OverlayWidget for the Dynamic Island expand/collapse.

### Performance Considerations
- OverlayWidget uses direct DOM manipulation (refs + rAF) for audio visualization to avoid React re-renders on every animation frame.
- ResourceMonitor only polls when expanded.
- Audio level events stored in ref (not state) in OverlayWidget to avoid re-renders.
- MetricsView line chart is pure SVG, computed in render — no canvas or external charting library.

### Component Structure
- 10 top-level components in `components/`.
- 1 subdirectory for history: `history/HistoryPanel.tsx` + barrel `index.ts`.
- 1 subdirectory for settings: `settings/SettingsPanel.tsx` + `settings/SettingsSection.tsx` + barrel `index.ts`.
- 1 subdirectory for UI primitives: `ui/Select.tsx`.
- 1 subdirectory for log viewer: `log-viewer/LogViewerApp.tsx` + `EventRow.tsx` + `LevelFilter.tsx` + `MetricsView.tsx` + `StreamChips.tsx`.
- Total: 21 files across 4 subdirectories.

### Multi-Window Architecture
- The app has at least 3 distinct window contexts:
  1. **Main window** — StatusHeader, RecordingControls, TranscriptionView, StatsBar, ResourceMonitor, PermissionsBanner, SettingsPanel, AboutModal, UpdateModal, ModelDownloader.
  2. **Overlay window** — OverlayWidget (Dynamic Island).
  3. **Log viewer window** — LogViewerApp (with EventRow, LevelFilter, MetricsView, StreamChips).
- The overlay window reads settings from localStorage directly (no shared React context across windows).

### Event System Types
- `AppEvent` type from `lib/events` has: `timestamp`, `stream`, `level`, `summary`, `data`.
- `StreamName` and `LevelName` types are defined in `lib/events`.
- `STREAMS`, `STREAM_COLORS`, `LEVEL_COLORS` constants are defined in `lib/events`.
- Stream colors have `bg`, `text`, and `dot` properties for styling.
- Default active streams in log viewer: `pipeline`, `audio`, `system`.

### Settings Types Referenced
- `Settings` — main settings object.
- `RecordingMode` — enum/union for recording trigger mode.
- `ModelOption` — type for model selection values.
- Constants: `DEFAULT_SETTINGS`, `STORAGE_KEY`, `MODEL_OPTIONS`, `MOONSHINE_MODELS`, `WHISPER_MODELS`, `DOUBLE_TAP_KEY_OPTIONS`, `RECORDING_MODE_OPTIONS`.
- `DictationStatus` — type for recording state (`'idle' | 'recording' | 'processing'`), validated by `isDictationStatus`.
- `UpdateStatus` — discriminated union for update phases.
- `HistoryEntry` — type with `id`, `text`, `timestamp`, `duration` fields.
- `DictationStats` — type with at least `totalWords`, `totalRecordings` fields.
- `ResourceReading` — type with `cpu_percent`, `memory_mb` fields.
