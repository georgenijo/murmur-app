# Agent 2 — Frontend Logic Findings

## User-Facing Features

### Recording / Dictation
- **Three recording modes**: Hold Down, Double-Tap, and Both (combined). Configurable via `recordingMode` setting (`'hold_down' | 'double_tap' | 'both'`).
- **Hold Down mode**: User presses and holds a trigger key; recording starts on key-down, stops on key-up. Events: `hold-down-start`, `hold-down-stop`. The standalone `useHoldDownToggle` hook does NOT listen for `hold-down-cancel` -- it only listens for `hold-down-start` and `hold-down-stop`. The `hold-down-cancel` event is only handled in `useCombinedToggle` (the "both" mode hook).
- **Double-Tap mode**: User double-taps a trigger key to toggle recording on/off. Event: `double-tap-toggle`.
- **Both mode (combined)**: Hold-down and double-tap run simultaneously on the same key. A `holdActiveRef` flag prevents the eager hold-start from confusing the double-tap state machine (avoids "single tap to stop" on first release of a double-tap). This is the only mode that handles the `hold-down-cancel` event. The handler does two things: (1) resets `holdActiveRef.current = false`, and (2) invokes `cancel_native_recording` (fire-and-forget with `.catch(() => {})`). It does NOT call `onStop` or `onToggle` -- the speculative recording is silently discarded without transcription.
- **Trigger key choices**: Left Shift (`shift_l`), Left Option/Alt (`alt_l`), Right Control (`ctrl_r`). Configurable via `doubleTapKey` setting.
- **Locked mode**: A UI toggle (`toggleLockedMode`) that keeps recording active until explicitly unlocked. When locked, if not already recording it starts; when unlocked, if recording it stops.
- **Recording duration timer**: Displays elapsed seconds while recording. Timer ticks every 1 second via `setInterval`. Resets to 0 when not recording.
- **Live audio level**: A numeric `audioLevel` state updated via the `audio-level` Tauri event, used for waveform visualization in the UI.
- **Transcription display**: The most recent transcription text is stored in `transcription` state and presumably displayed to the user.

### Status Indicators
- **DictationStatus**: Three states visible to the user: `'idle'`, `'recording'`, `'processing'`. The `recording-status-changed` Tauri event keeps the main window in sync when recording is initiated from the overlay.
- **Error display**: Errors are surfaced to the user via the `error` state. Auto-paste failures (`auto-paste-failed` event) show a hint that auto-clears after 5 seconds.

### History
- **Transcription history**: Every successful transcription is stored as a `HistoryEntry` with `id`, `text`, `timestamp`, and `duration`. Persisted to `localStorage` under key `'dictation-history'`.
- **History cap**: Maximum 50 entries. Oldest entries are trimmed.
- **Clear history**: User can clear all history entries (`clearHistory`).
- **Timestamp formatting**: Entries display formatted time via `formatTimestamp` (e.g., "2:30 PM").

### Statistics
- **Total words**: Cumulative count of all transcribed words.
- **Total recordings**: Cumulative count of transcription sessions.
- **Total duration**: Cumulative recording time in seconds.
- **WPM (words per minute)**: Rolling average computed from up to 100 most recent WPM samples. Each sample is `(wordCount / durationSeconds) * 60`.
- **Approximate tokens**: Estimated at `totalWords * 1.3`.
- **Stats reset**: User can reset all stats (`resetStats`). Stats persisted to `localStorage` under key `'dictation-stats'`.
- **Stats version counter**: `statsVersion` state increments on each transcription, likely used to trigger UI re-renders of stats displays.

### Settings
- **Model selection**: 7 model options across two backends:
  - Moonshine: `moonshine-tiny` (~124 MB), `moonshine-base` (~286 MB)
  - Whisper: `tiny.en` (~75 MB), `base.en` (~150 MB), `small.en` (~500 MB), `medium.en` (~1.5 GB), `large-v3-turbo` (~3 GB)
- **Language**: String setting, defaults to `'en'`.
- **Auto-paste**: Boolean toggle. When enabled, transcribed text is automatically pasted into the active application. Default: `false`.
- **Auto-paste delay**: Configurable delay in milliseconds (`autoPasteDelayMs`). Default: 50 ms.
- **VAD sensitivity**: Numeric value (`vadSensitivity`). Default: 50.
- **Microphone selection**: String setting for input device name. Default: `'system_default'`.
- **Launch at login**: Boolean toggle for macOS autostart. Syncs with OS login item state on mount (detects if user removed it from System Settings). Uses `@tauri-apps/plugin-autostart`. Default: `false`.
- **Recording mode**: Selectable as Hold Down, Double-Tap, or Both. Default: `'hold_down'`.
- **Settings persistence**: All settings saved to `localStorage` under key `'dictation-settings'`.
- **Settings migration**: Legacy `'hotkey'` recording mode is migrated to `'hold_down'`. Legacy `hotkey` field is stripped.
- **Optimistic updates with rollback**: When `configure()` or autostart toggle fails, settings revert to their previous values.

### Auto-Updater
- **Update check on launch**: Background check runs immediately on mount.
- **Periodic update checks**: Every 24 hours (`CHECK_INTERVAL_MS = 86400000`). Uses `setInterval` gated by `isDueForCheck()`.
- **Update phases**: `idle` -> `checking` -> `available` / `up-to-date` / `error`. Then `downloading` (with progress %) -> `ready` -> relaunch.
- **Forced updates**: If current version is below a remote `min_version` field fetched from `latest.json` on GitHub, the update is marked as forced (`isForced: true`). Users cannot skip forced updates.
- **Skip version**: User can skip a specific version; stored in `localStorage` under `'skipped-update-version'`. Skipped versions are suppressed on background checks.
- **Dismiss update**: User can dismiss the update notification without skipping.
- **Download progress**: Percentage progress reported via Tauri's `downloadAndInstall` progress callback.
- **macOS notifications**: Background update availability fires a native notification ("Murmur vX.Y.Z is ready to install") if notification permission is granted.
- **Auto-relaunch**: After successful download and install, the app relaunches automatically via `@tauri-apps/plugin-process`.
- **min_version source**: Fetched from `https://github.com/georgenijo/murmur-app/releases/latest/download/latest.json` with `cache: 'no-store'`.

### About Dialog
- **Show About listener**: The `useShowAboutListener` hook (in `lib/hooks/useShowAboutListener.ts`) listens for the `'show-about'` Tauri event (likely triggered from tray menu) and sets `showAbout` state to `true`. It is consumed in `App.tsx`, which calls `useShowAboutListener()` at line 80 and renders `<AboutModal isOpen={showAbout} onClose={() => setShowAbout(false)} />`. So the event is consumed via the hook in App.tsx, and the AboutModal component receives the state as a prop.

### Resource Monitor
- **CPU and memory readings**: Polls `get_resource_usage` Tauri command every 1 second when enabled.
- **Rolling window**: Keeps up to 60 readings (`MAX_READINGS = 60`), i.e., 1 minute of data.
- **Fresh start**: Clears stale readings when re-enabled.

### Structured Event Log Viewer
- **Event store**: Maintains an in-memory buffer of up to 500 `AppEvent` objects (`MAX_EVENTS = 500`).
- **Hydration from backend**: On mount, fetches existing event history via `get_event_history` Tauri command.
- **Live streaming**: Listens for `app-event` Tauri events and appends them to the buffer in real time.
- **Batched rendering**: Uses `requestAnimationFrame` to coalesce rapid event bursts into a single React state update.
- **Filtering**: Provides `getByStream(stream)` and `getByLevel(level)` filter methods.
- **Clear**: Sends `clear_event_history` to backend and clears local buffer.
- **Event streams**: `'pipeline'`, `'audio'`, `'keyboard'`, `'system'` -- each with distinct color theming.
- **Event levels**: `'trace'`, `'debug'`, `'info'`, `'warn'`, `'error'` -- each with distinct color styling.

## Internal Systems

### Dictation Pipeline (dictation.ts)
- `initDictation()` -> invokes `'init_dictation'` Tauri command. Called once on mount.
- `configure(options)` -> invokes `'configure_dictation'` with model, language, autoPaste, autoPasteDelayMs, vadSensitivity.
- `startRecording(deviceName?)` -> invokes `'start_native_recording'`. If deviceName equals `'system_default'`, sends `null` to backend.
- `stopRecording()` -> invokes `'stop_native_recording'`. Returns transcription result.
- `getStatus()` -> invokes `'get_status'`. Returns current dictation status.
- `DictationResponse` type: `{ type, state?, text?, model?, error? }`.

### Initialization Pipeline (useInitialization)
- Runs once on mount (empty dependency array, ESLint rule suppressed).
- Sequence: `initDictation()` -> `configure({ model, language, autoPaste })` -> sets `initialized = true`.
- On failure: sets `error` string.
- Cancellation flag prevents state updates after unmount.

### Keyboard Listener Lifecycle
- All three keyboard hooks (hold-down, double-tap, combined) share the same pattern:
  1. Gate on `enabled && initialized && accessibilityGranted`.
  2. Set up Tauri event listeners for keyboard events.
  3. Invoke `start_keyboard_listener` with hotkey and mode (`'hold_down'`, `'double_tap'`, or `'both'`).
  4. On `keyboard-listener-error` event: wait 2 seconds, then attempt restart.
  5. Cleanup: unlisten all events + invoke `stop_keyboard_listener`.
- Callbacks are stored in refs to avoid stale closures.
- All setup is async with a `cancelled` flag to handle race conditions during cleanup.

### Backend State Synchronization
- `set_keyboard_recording` is invoked whenever recording status changes, to keep the Rust double-tap detector's internal state in sync.
- In combined mode, the sync is skipped while a hold press is active (`holdActiveRef`) to prevent the eager hold-start from corrupting double-tap state machine transitions.

### Recording State Machine (useRecordingState)
- States: `idle` -> `recording` -> `processing` -> `idle`.
- Guard refs prevent concurrent `handleStart` / `handleStop` calls (`isStartingRef`, `isStoppingRef`).
- `toggleRecording`: If `processing`, no-op. If `recording`, stop. Otherwise, start. Reads status from ref for stability.
- History and stats updates are handled exclusively by the `transcription-complete` event listener to avoid race-condition duplicates (not in `handleStop`).
- The `recording-status-changed` event can externally drive status transitions (e.g., overlay-initiated recording). It also seeds `recordingStartTime` if the recording was started externally.

### Frontend Logging (log.ts)
- `flog.info/warn/error(tag, message, data?)` — fire-and-forget calls to `'log_frontend'` Tauri command.
- Formats: `[tag] message` or `[tag] message {json}`.
- Non-blocking: catch errors silently.

### Settings Persistence and Sync
- `loadSettings()`: Reads from localStorage, merges with defaults, applies migrations.
- `saveSettings()`: Writes full settings object to localStorage.
- `useSettings` hook: Wraps load/save with React state; pushes changes to backend via `configure()`.
- Autostart sync: On mount, queries OS autostart state and reconciles with stored setting.
- Sequential autostart operations: Uses a promise chain (`lastAutostartOp`) to serialize enable/disable calls to the OS.
- Versioned configure calls: `configureVersionRef` prevents stale rollbacks from overwriting newer settings.

### Updater Logic (updater.ts)
- Semver parsing: Handles `v` prefix, whitespace, pre-release/build metadata (strips them).
- Semver comparison: Returns -1/0/1/null. Null means unparseable.
- `isBelowMinVersion`: Fail-safe -- unparseable versions return `true` (force update).
- Skipped version: localStorage key `'skipped-update-version'`.
- Check interval: localStorage key `'updater-last-check'`, 24-hour interval.
- `fetchMinVersion`: HTTP fetch to GitHub releases `latest.json`, extracts `min_version` field.

### Event System (events.ts)
- `AppEvent` type: `{ timestamp: string, stream: StreamName, level: LevelName, summary: string, data: Record<string, unknown> }`.
- Four streams: `pipeline`, `audio`, `keyboard`, `system`.
- Five levels: `trace`, `debug`, `info`, `warn`, `error`.
- Color constants: Each stream has `bg`, `text`, `dot` Tailwind classes. Each level has a `text` Tailwind class.
- Pure data/type definitions -- no logic, no event emission. Used by `useEventStore` and presumably by log viewer UI components.

## Commands / Hooks / Events

### Tauri Commands (invoked from frontend)
- `init_dictation` — Initialize the dictation subsystem on app startup.
- `configure_dictation` — Send configuration (model, language, autoPaste, autoPasteDelayMs, vadSensitivity) to the Rust backend.
- `start_native_recording` — Begin audio capture with optional device name.
- `stop_native_recording` — Stop audio capture and trigger transcription.
- `cancel_native_recording` — Cancel a speculative recording without transcribing (hold-down short tap).
- `get_status` — Query the current dictation status from the backend.
- `start_keyboard_listener` — Start the rdev keyboard listener with a hotkey and mode (hold_down, double_tap, or both).
- `stop_keyboard_listener` — Stop the rdev keyboard listener.
- `set_keyboard_recording` — Inform the backend whether a recording is active (for double-tap state machine sync).
- `log_frontend` — Write a log line to the frontend log file on the Rust side.
- `get_resource_usage` — Get current CPU percent and memory MB usage.
- `get_event_history` — Fetch all stored structured events from the backend.
- `clear_event_history` — Clear all stored structured events on the backend.

### React Hooks
- `useAutoUpdater()` — Manages the full auto-update lifecycle: periodic checks, download with progress, forced updates, skip/dismiss, macOS notifications.
- `useCombinedToggle({ enabled, initialized, accessibilityGranted, triggerKey, status, onStart, onStop, onToggle })` — Combined hold-down + double-tap keyboard listener; manages both event types on a single key with hold-active tracking. This is the only hook that listens for `hold-down-cancel`; the handler resets `holdActiveRef` and invokes `cancel_native_recording`.
- `useDoubleTapToggle({ enabled, initialized, accessibilityGranted, doubleTapKey, status, onToggle })` — Double-tap keyboard listener; starts rdev listener in double_tap mode and toggles recording on double-tap events.
- `useHoldDownToggle({ enabled, initialized, accessibilityGranted, holdDownKey, onStart, onStop })` — Hold-down keyboard listener; starts rdev listener in hold_down mode and calls onStart/onStop on key press/release. Listens only for `hold-down-start` and `hold-down-stop`; does NOT handle `hold-down-cancel`.
- `useHistoryManagement()` — Manages transcription history array with localStorage persistence; exposes addEntry and clearHistory.
- `useInitialization(settings)` — Runs one-time init sequence (initDictation + configure) and returns initialized/error state.
- `useRecordingState({ addEntry, microphone })` — Core recording state machine with start/stop/toggle, audio level tracking, locked mode, error handling, and stats integration.
- `useResourceMonitor(enabled)` — Polls CPU/memory usage every second and maintains a rolling 60-reading buffer.
- `useSettings()` — Manages settings state with localStorage persistence, OS autostart sync, and backend configuration pushes.
- `useShowAboutListener()` — Listens for the show-about event from the tray and manages showAbout boolean state.
- `useEventStore()` — Manages structured event log buffer with backend hydration, live streaming, batched rendering, and filter/clear operations.

### Tauri Events (listened to from frontend)
- `hold-down-start` — Emitted by Rust when the trigger key is pressed down (hold mode).
- `hold-down-stop` — Emitted by Rust when the trigger key is released (hold mode).
- `hold-down-cancel` — Emitted by Rust when a hold press is too short (speculative recording discarded). The exact string in the `listen()` call is `'hold-down-cancel'` (hyphens, not underscores). Only `useCombinedToggle` listens for this event; `useHoldDownToggle` does not. The handler resets `holdActiveRef` to `false` and invokes `cancel_native_recording`.
- `double-tap-toggle` — Emitted by Rust when a double-tap is detected.
- `keyboard-listener-error` — Emitted by Rust when the rdev listener thread crashes; payload is an error string.
- `recording-status-changed` — Emitted by Rust when dictation status changes; payload is a DictationStatus string. Used to sync main window with overlay-initiated recording.
- `audio-level` — Emitted by Rust with a numeric payload representing current audio input level for waveform visualization.
- `auto-paste-failed` — Emitted by Rust when an auto-paste attempt fails; payload is an error message string displayed to the user for 5 seconds.
- `transcription-complete` — Emitted by Rust when transcription finishes; payload is `{ text: string, duration: number }`. Single source of truth for history/stats updates.
- `show-about` — Emitted by Rust (likely from tray menu) to open the About dialog.
- `app-event` — Emitted by Rust for structured logging; payload is an `AppEvent` object consumed by the event store.

## Gaps / Unclear

1. **`getStatus()` in dictation.ts is exported but never called** from any file in `app/src/lib/`. It may be used elsewhere (e.g., components) or may be dead code.

2. **`useInitialization` ignores current settings for configure**: It runs `configure()` with the initial settings only once on mount (empty dep array). If settings change before initialization completes, or if the user changes settings before the configure call finishes, those changes could be lost. The ESLint exhaustive-deps rule is explicitly suppressed.

3. **History entry ID uses `Date.now().toString()`**: Two entries created in the same millisecond would have duplicate IDs. This is unlikely in practice (one transcription at a time), but the ID generation is not strictly unique.

4. **`handleStop` still accepts `addEntry` in its dependency array** but does not call it directly. The comment says history updates are handled by the `transcription-complete` listener, but `addEntry` remains in the deps array of `handleStop` via the outer hook closure -- this is harmless but potentially confusing.

5. **`useRecordingState` transcription-complete listener is the sole writer of history/stats**, yet `handleStop` also sets `transcription` text. There is a potential for a brief flash where `handleStop` sets `transcription` from its `stopRecording()` response, then the event listener overwrites it again with the same text. No data corruption, but redundant state update.

6. **`useDoubleTapToggle` and `useHoldDownToggle` are actively used standalone hooks**: All three hooks are called in `App.tsx` (lines 77-79), each gated by the `recordingMode` setting: `useHoldDownToggle` is enabled when mode is `'hold_down'`, `useDoubleTapToggle` when `'double_tap'`, and `useCombinedToggle` when `'both'`. Exactly one is active at a time. However, `useHoldDownToggle` does not listen for `hold-down-cancel`, meaning in pure hold-down mode, if the Rust backend emits `hold-down-cancel` (short tap), the frontend has no handler for it. The speculative recording started by `hold-down-start` would proceed through `onStop` (via `hold-down-stop`) and attempt transcription rather than being cancelled. This is a potential behavioral gap: cancel-on-short-tap only works in "both" mode, not in standalone hold-down mode.

7. **`configure()` in `useInitialization` does not pass `autoPasteDelayMs` or `vadSensitivity`**: It only passes `{ model, language, autoPaste }`. These two settings would only be sent to the backend when `useSettings.updateSettings` is called later. If the user has non-default values saved, they would not be applied until the user changes any triggering setting. This could cause the first transcription to use wrong autoPasteDelayMs/vadSensitivity.

8. **No error recovery for `initDictation` failure**: If `initDictation()` fails, the error is set but there is no retry mechanism. The user would need to restart the app.

9. **`fetchMinVersion` does a raw HTTP fetch**: This bypasses any Tauri proxy or authentication. It fetches from a public GitHub URL, so this works for public repos but would fail for private repos.

10. **`useResourceMonitor` suppresses errors in dev mode only**: Uses `import.meta.env.DEV` guard on `console.debug`. In production, errors are fully silenced.

11. **Event store `getByStream` and `getByLevel` read from `bufRef.current` directly**: These functions reference the mutable ref, not the React state. This means they always return the most up-to-date data but are not reactive -- components calling them would not re-render when new events arrive unless they also depend on the `events` state array.

12. **`STREAM_COLORS` and `LEVEL_COLORS` use Tailwind classes with dark mode variants**: This implies the app supports dark mode, but there is no dark mode toggle in settings. Dark mode is presumably driven by the OS system appearance.

13. **`updater.ts` CHECK_INTERVAL_MS is used as the `setInterval` period in `useAutoUpdater`**: This means the interval callback fires every 24 hours, but inside it also checks `isDueForCheck()`. This double-gating is redundant but harmless.

14. **Settings type `vadSensitivity` is a bare `number`**: No min/max bounds are enforced in the frontend code. Validation presumably happens on the Rust side, but invalid values could be persisted to localStorage.

15. **`autoPasteDelayMs` default is 50ms**: This is a very short delay. No UI validation is visible in `lib/` code to prevent a user from setting it to 0 or a very large value.

## Notes

1. **Dual model backend architecture**: The settings system supports both Whisper and Moonshine transcription backends, with `MODEL_OPTIONS` explicitly tagging each model's backend. The `TranscriptionBackend` type (`'whisper' | 'moonshine'`) and filtered arrays (`MOONSHINE_MODELS`, `WHISPER_MODELS`) suggest the UI may present these in separate sections.

2. **Clipboard-first design**: The `dictation.ts` module has no clipboard logic -- that lives entirely in Rust. The frontend only controls `autoPaste` as a boolean toggle. Text always goes to clipboard; auto-paste is additive.

3. **Overlay-aware architecture**: Multiple patterns in `useRecordingState` explicitly handle the case where recording is started/stopped from an overlay window rather than the main window. The `recording-status-changed` event and `transcription-complete` event are the synchronization mechanisms.

4. **Strict cancellation patterns**: Every async `useEffect` setup uses a `cancelled` flag and checks it after each await. This is a consistent and thorough approach to preventing stale state updates on unmount.

5. **Ref-based callback pattern**: All keyboard hooks store callbacks in refs (`onStartRef`, `onStopRef`, `onToggleRef`) and update them in separate `useEffect` hooks. This prevents the main listener setup from re-running when callbacks change identity.

6. **Test coverage**: Two test files exist: `settings.test.ts` (7 tests covering defaults, migration, persistence) and `updater.test.ts` (comprehensive tests for semver parsing, comparison, skipped version storage, and check interval logic). No tests exist for `history.ts`, `stats.ts`, `dictation.ts`, `log.ts`, `events.ts`, or any hooks.

7. **All state persistence uses localStorage**: Settings, history, stats, skipped update version, and last check timestamp all use `localStorage`. No IndexedDB, no backend-side persistence for user data.

8. **Error handling is consistently defensive**: Every `localStorage` read/write is wrapped in try/catch. Every Tauri invoke has error handling. Errors are logged to console or surfaced to the user, never thrown to crash the app.

9. **No internationalization (i18n)**: All user-facing strings in lib/ are hardcoded in English. There is no i18n framework or string externalization.

10. **`configure()` is called from two places**: Once in `useInitialization` (on mount) and again in `useSettings.updateSettings` (on any relevant setting change). The initialization call is a subset of the full configuration -- it omits `autoPasteDelayMs` and `vadSensitivity` (see Gap #7).
