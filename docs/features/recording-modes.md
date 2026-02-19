# Recording Modes

The app supports two ways to trigger recording, selectable in Settings.

## Key Combo Mode (default)

Uses Tauri's `global-shortcut` plugin to register system-wide key combinations.

**Behavior:** Press combo to start recording, press again to stop and transcribe.

**Available combos:**
| Setting Value | Combo |
|---------------|-------|
| `shift_l` | Shift + Space |
| `alt_l` | Option + Space |
| `ctrl_r` | Control + Space |

**Code path:**
- `useHotkeyToggle` hook registers/unregisters the shortcut
- `lib/hotkey.ts` wraps `@tauri-apps/plugin-global-shortcut`
- On trigger: calls `toggleRecording()` which invokes `start_native_recording` or `stop_native_recording`

**No special permissions** beyond microphone.

## Double-Tap Mode

Uses `rdev` for low-level keyboard event listening. Detects quick double-taps on bare modifier keys.

**Behavior:** Double-tap modifier to start recording, single tap to stop.

**Available keys:**
| Setting Value | Key |
|---------------|-----|
| `shift_l` | Left Shift |
| `alt_l` | Left Option |
| `ctrl_r` | Right Control |

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

### Threading

- `rdev::listen()` runs on a background thread (spawned once, lives for app lifetime)
- `set_is_main_thread(false)` is called before `listen()` — this is **critical** on macOS because rdev's keyboard translation calls TIS/TSM APIs that Apple requires on the main thread. Without this flag, the app segfaults on key press.
- `AtomicBool` (`LISTENER_ACTIVE`) gates event processing without killing the thread
- `Mutex<Option<DoubleTapDetector>>` holds the detector state, shared between the listener callback and control API

### Code Path

- `useDoubleTapToggle` hook manages lifecycle (start/stop listener, listen for events)
- Hook syncs recording status to backend via `set_double_tap_recording` command
- Rust `keyboard::start_listener()` spawns rdev thread, `keyboard::set_recording_state()` toggles single-tap-to-stop
- On detection: emits `"double-tap-toggle"` event to frontend via `app_handle.emit()`
- Frontend event handler calls `toggleRecording()` — same as key combo mode from there

### Tests

23 unit tests in `keyboard.rs` (`#[cfg(test)] mod tests`). Run with:
```bash
cd ui/src-tauri && cargo test -- --test-threads=1
```

Single-threaded because timing tests use `sleep()`.

## Settings Integration

Both modes share the same `hotkey` setting value (`shift_l`, `alt_l`, `ctrl_r`). The `recordingMode` setting (`'hotkey' | 'double_tap'`) determines which hook is active.

Both hooks are always called (React Rules of Hooks) but only the active one registers listeners, via the `enabled` prop.

Mode switching is disabled while recording (`status !== 'idle'`).
