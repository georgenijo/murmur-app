# Recording Modes

The app supports Hold-Down, Double-Tap, and a combined Both mode, selectable in Settings. All use `rdev` for low-level keyboard event listening and require Accessibility permission.

## Transform hold key (issue #312)

Separate from dictation modes: a dedicated **transform hold key** (`transformHoldKey`: `alt_r` / `ctrl_l` / `shift_r`) drives the selected-text transform flow. It uses the **same shared rdev listener thread** as dictation (one thread rule), with an independent detector and `start_transform_listener` / `stop_transform_listener` / `set_transform_key` commands. Dictation hold keys are rejected for the transform shortcut so the two never share a physical key. See [selected-text-transform.md](selected-text-transform.md).

## Hold-Down Mode (default)

Hold a modifier key to record, release to stop and transcribe.

**Behavior:** Press and hold modifier to start recording. Release to stop recording and begin transcription.

**Available keys:**

| Setting Value | Key |
|---------------|-----|
| `shift_l` | Left Shift |
| `alt_l` | Left Option |
| `ctrl_r` | Right Control |

**Requires Accessibility permission** (rdev needs it for global keyboard events).

### State Machine (`HoldDownDetector` in `keyboard.rs`)

To start:

```text
Idle → KeyPress(target) → Held  (emit hold-down-start)
```

To stop:

```text
Held → KeyRelease(target) → Idle (emit hold-down-stop)
```

### Rejection Rules

- **Key repeat** while held: Ignored (stays in Held state)
- **Modifier + letter** (e.g. Shift+A): Cancels hold, emits stop
- **Cooldown**: 300ms after stop before re-trigger is allowed

### Code Path

- `useHoldDownToggle` hook manages lifecycle (start/stop listener, listen for events)
- Listens for two distinct events: `hold-down-start` and `hold-down-stop`
- Rust `keyboard::start_listener(app_handle, hotkey, "hold_down")` spawns rdev thread
- On key press: emits `"hold-down-start"` event → frontend calls `handleStart()`
- On key release: emits `"hold-down-stop"` event → frontend calls `handleStop()`

## Double-Tap Mode

Uses `rdev` for low-level keyboard event listening. Detects quick double-taps on bare modifier keys.

**Behavior:** Double-tap modifier to start recording, single tap to stop.

**Available keys:** Same as Hold-Down mode (Left Shift, Left Option, Right Control).

**Requires Accessibility permission** (rdev needs it for global keyboard events).

### State Machine (`DoubleTapDetector` in `keyboard.rs`)

To start (when not recording):

```text
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 200ms → WaitingSecondDown
WaitingSecondDown → KeyDown(target) within 400ms → WaitingSecondUp
WaitingSecondUp → KeyUp(target) within 200ms → FIRE
```

To stop (when recording):

```text
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 200ms → FIRE
```

### Rejection Rules

- **Held key** (>200ms): Resets to Idle
- **Modifier + letter** (e.g. Shift+A): Resets on non-modifier KeyPress
- **Slow gap** between taps (>400ms): A timer resets to Idle at expiry, without waiting for another keyboard event
- **Triple-tap spam**: 50ms cooldown after firing
- **Key repeat events**: Ignored while within hold duration

### Code Path

- `useDoubleTapToggle` hook manages lifecycle (start/stop listener, listen for events)
- Hook syncs recording status to backend via `set_keyboard_recording` command
- Rust `keyboard::start_listener(app_handle, hotkey, "double_tap")` spawns rdev thread
- On detection: emits `"double-tap-toggle"` event to frontend via `app_handle.emit()`
- Frontend event handler calls `toggleRecording()`

### Optional Timing-Miss Feedback

The `hotkeyMissFeedback` setting is off by default. When enabled, expiration of the 400ms second-tap window in Double-Tap or Both mode emits `hotkey-tap-rejected` with `{ reason: "second_tap_expired", mode }`. The overlay shows a distinct amber `Tap missed` flash for 500ms.

Only the expired second-tap window is surfaced. Existing structured diagnostics still record other rejection reasons, but the UI stays silent for long holds, modifier+letter combinations, processing skips, Both mode's first short tap, and valid double-taps. This prevents ordinary modifier use from producing feedback noise.

## Shared Infrastructure

### Threading

- Both modes share a single `rdev::listen()` background thread (spawned once, lives for app lifetime)
- `set_is_main_thread(false)` is called before `listen()` — this is **critical** on macOS because rdev's keyboard translation calls TIS/TSM APIs that Apple requires on the main thread. Without this flag, the app segfaults on key press.
- rdev is pinned to Murmur's fork by commit revision. Its macOS listener derives modifier press/release directly from the physical keycode and device-specific flag (no cached global modifier state), automatically re-enables a disabled event tap, listens only for key events, and skips key-name translation for modifier events.
- `AtomicBool` (`LISTENER_ACTIVE`) gates event processing without killing the thread
- `DetectorMode` enum (`DoubleTap` | `HoldDown`) determines which detector processes events
- Separate `Mutex`-wrapped detectors: `DOUBLE_TAP_DETECTOR` and `HOLD_DOWN_DETECTOR`

### Tests

46 unit tests in `keyboard.rs` (`#[cfg(test)] mod tests`). Run with:
```bash
cd app/src-tauri && cargo test -- --test-threads=1
```

Single-threaded because timing tests use `sleep()`.

## Settings Integration

All modes share the `doubleTapKey` setting (`shift_l`, `alt_l`, `ctrl_r`). The `recordingMode` setting (`'hold_down' | 'double_tap' | 'both'`) determines which hook is active.

All three hooks are always called (React Rules of Hooks) but only the active one registers listeners, via the `enabled` prop.

Mode switching is disabled while recording (`status !== 'idle'`).
