# Recording Modes

The app supports two ways to trigger recording, selectable in Settings. Both use `rdev` for low-level keyboard event listening and require Accessibility permission.

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
```
Idle → KeyPress(target) → Held  (emit hold-down-start)
```

To stop:
```
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
```
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 300ms → WaitingSecondDown
WaitingSecondDown → KeyDown(target) within 400ms → WaitingSecondUp
WaitingSecondUp → KeyUp(target) within 300ms → FIRE
```

To stop (when recording):
```
Idle → KeyDown(target) → WaitingFirstUp
WaitingFirstUp → KeyUp(target) within 300ms → FIRE
```

### Rejection Rules

- **Held key** (>300ms): Resets to Idle
- **Modifier + letter** (e.g. Shift+A): Resets on non-modifier KeyPress
- **Slow gap** between taps (>400ms): Resets to Idle
- **Triple-tap spam**: 500ms cooldown after firing
- **Key repeat events**: Ignored while within hold duration

### Code Path

- `useDoubleTapToggle` hook manages lifecycle (start/stop listener, listen for events)
- Hook syncs recording status to backend via `set_keyboard_recording` command
- Rust `keyboard::start_listener(app_handle, hotkey, "double_tap")` spawns rdev thread
- On detection: emits `"double-tap-toggle"` event to frontend via `app_handle.emit()`
- Frontend event handler calls `toggleRecording()`

## Shared Infrastructure

### Threading

- Both modes share a single `rdev::listen()` background thread (spawned once, lives for app lifetime)
- `set_is_main_thread(false)` is called before `listen()` — this is **critical** on macOS because rdev's keyboard translation calls TIS/TSM APIs that Apple requires on the main thread. Without this flag, the app segfaults on key press.
- `AtomicBool` (`LISTENER_ACTIVE`) gates event processing without killing the thread
- `DetectorMode` enum (`DoubleTap` | `HoldDown`) determines which detector processes events
- Separate `Mutex`-wrapped detectors: `DOUBLE_TAP_DETECTOR` and `HOLD_DOWN_DETECTOR`

### Tests

46 unit tests in `keyboard.rs` (`#[cfg(test)] mod tests`). Run with:
```bash
cd ui/src-tauri && cargo test -- --test-threads=1
```

Single-threaded because timing tests use `sleep()`.

## Settings Integration

Both modes share the `doubleTapKey` setting (`shift_l`, `alt_l`, `ctrl_r`). The `recordingMode` setting (`'hold_down' | 'double_tap'`) determines which hook is active.

Both hooks are always called (React Rules of Hooks) but only the active one registers listeners, via the `enabled` prop.

Mode switching is disabled while recording (`status !== 'idle'`).
