# React Hooks Reference

This document lists all 11 custom React hooks in Murmur. Each hook is located under `app/src/lib/hooks/` and is consumed by `App.tsx` or by window-specific entry points (overlay, log viewer).

For the Tauri commands these hooks invoke, see [commands.md](commands.md). For the events they listen to, see [events.md](events.md). For settings managed by `useSettings`, see [settings.md](settings.md).

---

## useAutoUpdater

**File:** `app/src/lib/hooks/useAutoUpdater.ts`

**Parameters:** None.

**Returns:** `UseAutoUpdaterReturn`

```typescript
interface UseAutoUpdaterReturn {
  updateStatus: UpdateStatus;
  checkForUpdate: () => Promise<void>;
  startDownload: () => Promise<void>;
  skipVersion: () => void;
  dismissUpdate: () => void;
}
```

**Responsibilities:**
- Checks for updates on launch and every 24 hours (`CHECK_INTERVAL_MS`). Supports forced updates when the current version is below a remote `min_version` field.
- Manages the full update lifecycle: check, download with progress, install, and relaunch via `@tauri-apps/plugin-updater` and `@tauri-apps/plugin-process`.
- Sends a macOS notification when an update is discovered during a background check.

**Key interactions:**
- Fetches `min_version` from GitHub releases `latest.json` via HTTP.
- Stores skipped version in localStorage under `skipped-update-version`.
- Stores last check timestamp in localStorage under `updater-last-check`.

---

## useCombinedToggle

**File:** `app/src/lib/hooks/useCombinedToggle.ts`

**Parameters:**

```typescript
interface UseCombinedToggleProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  triggerKey: string;
  status: DictationStatus;
  onStart: () => void;
  onStop: () => void;
  onToggle: () => void;
}
```

**Returns:** `void`

**Responsibilities:**
- Combines hold-down and double-tap keyboard detection on a single trigger key. Active when `recordingMode` is `'both'`.
- Tracks whether a hold-down press is currently active via `holdActiveRef` to prevent the eager hold-start from corrupting the double-tap state machine.
- Syncs recording state to the backend double-tap detector via `set_keyboard_recording`, but skips syncing while a hold press is active.

**Key interactions:**
- Listens to events: `hold-down-start`, `hold-down-stop`, `double-tap-toggle`, `keyboard-listener-error`, `hold-down-cancel` (dead code -- this event is never emitted from Rust).
- Invokes commands: `start_keyboard_listener` (mode `"both"`), `stop_keyboard_listener`, `set_keyboard_recording`, `cancel_native_recording` (in the dead `hold-down-cancel` handler).

---

## useDoubleTapToggle

**File:** `app/src/lib/hooks/useDoubleTapToggle.ts`

**Parameters:**

```typescript
interface UseDoubleTapToggleProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  doubleTapKey: string;
  status: DictationStatus;
  onToggle: () => void;
}
```

**Returns:** `void`

**Responsibilities:**
- Manages the double-tap keyboard listener for standalone double-tap mode. Active when `recordingMode` is `'double_tap'`.
- Keeps the backend double-tap detector in sync with the current recording status via `set_keyboard_recording`.
- On `keyboard-listener-error`, waits 2 seconds then attempts to restart the listener.

**Key interactions:**
- Listens to events: `double-tap-toggle`, `keyboard-listener-error`.
- Invokes commands: `start_keyboard_listener` (mode `"double_tap"`), `stop_keyboard_listener`, `set_keyboard_recording`.

---

## useHoldDownToggle

**File:** `app/src/lib/hooks/useHoldDownToggle.ts`

**Parameters:**

```typescript
interface UseHoldDownToggleProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  holdDownKey: string;
  onStart: () => void;
  onStop: () => void;
}
```

**Returns:** `void`

**Responsibilities:**
- Manages the hold-down keyboard listener for standalone hold-down mode. Active when `recordingMode` is `'hold_down'`.
- Calls `onStart` on key press and `onStop` on key release. Does not handle `hold-down-cancel`.
- On `keyboard-listener-error`, waits 2 seconds then attempts to restart the listener.

**Key interactions:**
- Listens to events: `hold-down-start`, `hold-down-stop`, `keyboard-listener-error`.
- Invokes commands: `start_keyboard_listener` (mode `"hold_down"`), `stop_keyboard_listener`.

---

## useHistoryManagement

**File:** `app/src/lib/hooks/useHistoryManagement.ts`

**Parameters:** None.

**Returns:**

```typescript
{
  historyEntries: HistoryEntry[];
  addEntry: (text: string, duration: number) => void;
  clearHistory: () => void;
}
```

**Responsibilities:**
- Manages the transcription history array with a maximum of 50 entries (oldest trimmed).
- Persists history to localStorage under the key `dictation-history`.
- Provides `addEntry` (used by `useRecordingState` via the `transcription-complete` event) and `clearHistory` callbacks.

**Key interactions:**
- Pure frontend state management. No Tauri commands or events. Delegates to `lib/history.ts` for persistence.

---

## useInitialization

**File:** `app/src/lib/hooks/useInitialization.ts`

**Parameters:** `settings: Settings`

**Returns:**

```typescript
{
  initialized: boolean;
  error: string;
}
```

**Responsibilities:**
- Runs the one-time initialization sequence on mount: `initDictation()` then `configure()` with the current model, language, and autoPaste settings.
- Sets `initialized` to `true` on success, which gates the recording controls and keyboard listeners.
- Uses a cancellation flag to prevent stale state updates after unmount.

**Key interactions:**
- Invokes commands: `init_dictation`, `configure_dictation`.
- Note: Does not pass `autoPasteDelayMs` or `vadSensitivity` during initial configure. Those values are sent to the backend when `useSettings.updateSettings` is called.

---

## useRecordingState

**File:** `app/src/lib/hooks/useRecordingState.ts`

**Parameters:**

```typescript
interface UseRecordingStateProps {
  addEntry: (text: string, duration: number) => void;
  microphone: string;
}
```

**Returns:**

```typescript
{
  status: DictationStatus;
  transcription: string;
  recordingDuration: number;
  error: string;
  setError: (error: string) => void;
  handleStart: () => Promise<void>;
  handleStop: () => Promise<void>;
  toggleRecording: () => Promise<void>;
  audioLevel: number;
  lockedMode: boolean;
  toggleLockedMode: () => Promise<void>;
  statsVersion: number;
}
```

**Responsibilities:**
- Core recording state machine (`idle` -> `recording` -> `processing` -> `idle`) with guard refs to prevent concurrent start/stop calls.
- Handles history and stats updates exclusively through the `transcription-complete` event listener to avoid race-condition duplicates.
- Manages locked mode (overlay-initiated persistent recording), audio level tracking, recording duration timer (1-second ticks), and auto-paste error display (5-second auto-clear).

**Key interactions:**
- Listens to events: `recording-status-changed` (syncs status from overlay), `transcription-complete` (single source of truth for history/stats), `auto-paste-failed` (error display), `audio-level` (waveform data).
- Invokes commands: `start_native_recording`, `stop_native_recording`.

---

## useResourceMonitor

**File:** `app/src/lib/hooks/useResourceMonitor.ts`

**Parameters:** `enabled: boolean`

**Returns:** `ResourceReading[]`

```typescript
interface ResourceReading {
  cpu_percent: number;
  memory_mb: number;
}
```

**Responsibilities:**
- Polls `get_resource_usage` every 1 second when enabled. Stops polling when disabled.
- Maintains a rolling window of up to 60 readings (1 minute of data).
- Clears stale readings when re-enabled so the chart starts fresh.

**Key interactions:**
- Invokes command: `get_resource_usage`.
- No events listened.

---

## useSettings

**File:** `app/src/lib/hooks/useSettings.ts`

**Parameters:** None.

**Returns:**

```typescript
{
  settings: Settings;
  updateSettings: (updates: Partial<Settings>) => void;
}
```

**Responsibilities:**
- Wraps settings load/save with React state. Loads from localStorage on initialization, applies migrations.
- Pushes relevant setting changes to the Rust backend via `configure_dictation` (model, language, autoPaste, autoPasteDelayMs, vadSensitivity). Uses versioned configure calls to prevent stale rollbacks.
- Synchronizes `launchAtLogin` with the OS autostart state on mount (detects if user removed login item from System Settings). Serializes autostart enable/disable calls via a promise chain.

**Key interactions:**
- Invokes commands: `configure_dictation` (via `lib/dictation.ts`).
- Uses `@tauri-apps/plugin-autostart` for `enable()`, `disable()`, `isEnabled()`.
- See [settings.md](settings.md) for the full settings schema.

---

## useShowAboutListener

**File:** `app/src/lib/hooks/useShowAboutListener.ts`

**Parameters:** None.

**Returns:**

```typescript
{
  showAbout: boolean;
  setShowAbout: (value: boolean) => void;
}
```

**Responsibilities:**
- Listens for the `show-about` Tauri event (emitted from the tray menu) and sets `showAbout` to `true`.
- Consumed in `App.tsx` to control the AboutModal visibility.

**Key interactions:**
- Listens to event: `show-about`.
- No commands invoked.

---

## useEventStore

**File:** `app/src/lib/hooks/useEventStore.ts`

**Parameters:** None.

**Returns:**

```typescript
{
  events: AppEvent[];
  getByStream: (stream: StreamName) => AppEvent[];
  getByLevel: (level: LevelName) => AppEvent[];
  clear: () => void;
}
```

**Responsibilities:**
- Manages an in-memory buffer of up to 500 `AppEvent` objects. Hydrates from the backend on mount via `get_event_history`.
- Streams new events in real-time via the `app-event` Tauri event. Coalesces rapid event bursts into a single React state update using `requestAnimationFrame`.
- Provides filter methods (`getByStream`, `getByLevel`) that read directly from the mutable ref buffer (always up-to-date but not reactive on their own).

**Key interactions:**
- Listens to event: `app-event`.
- Invokes commands: `get_event_history` (on mount), `clear_event_history` (on clear).

---

## Hook Activation in App.tsx

All hooks are always called (Rules of Hooks), but their behavior is gated by props:

| Hook | Active When |
|------|-------------|
| `useHoldDownToggle` | `recordingMode === 'hold_down'` |
| `useDoubleTapToggle` | `recordingMode === 'double_tap'` |
| `useCombinedToggle` | `recordingMode === 'both'` |
| `useAutoUpdater` | Always |
| `useInitialization` | Always (runs once on mount) |
| `useRecordingState` | Always |
| `useHistoryManagement` | Always |
| `useSettings` | Always |
| `useShowAboutListener` | Always |
| `useResourceMonitor` | When the resource monitor panel is expanded |
| `useEventStore` | Used in the log-viewer window |
