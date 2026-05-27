# Text Injection

## Overview

After transcription, text is always copied to the clipboard. Optionally, the app simulates a paste keystroke into the focused application: `Cmd+V` on macOS via `osascript`, `Ctrl+V` on Linux via `xdotool` (X11) or `wtype` (Wayland).

## Clipboard (`injector.rs`)

Uses `arboard` crate (maintained by 1Password). Text is set via `Clipboard::new()` + `clipboard.set_text()`.

This always happens, regardless of auto-paste setting. The user can always manually Cmd+V.

## Auto-Paste

When `auto_paste` is enabled in settings:

1. Copy text to clipboard
2. Check `AXIsProcessTrusted()` — if accessibility not granted, stop here (text is still in clipboard)
3. Wait for the configurable delay (default 50ms) for window focus to settle
4. Run `osascript -e 'tell application "System Events" to keystroke "v" using command down'`
5. If paste fails, wait 100ms and retry once
6. If both attempts fail, emit `auto-paste-failed` event so the frontend can notify the user

### Delay Rationale

The clipboard write (`arboard::set_text()` → `NSPasteboard`) is synchronous, so no delay is needed for clipboard sync. The delay exists solely to let macOS window focus settle after the transcription pipeline returns. The default of 50ms is sufficient for most systems; users can increase up to 500ms via the settings slider if paste lands in the wrong window.

### Configurable Delay

The paste delay is configurable via a range slider in the settings panel (10–500ms, step 10ms). The slider appears when auto-paste is enabled. The value is sent to the Rust backend via `configure_dictation` and clamped to the 10–500 range.

### Retry Behavior

If the first `osascript` paste attempt fails (non-zero exit), the injector logs a warning, waits 100ms, and retries once. Only after both attempts fail does it return an error. Worst-case blocking on the main thread is ~250ms (50ms delay + paste + 100ms retry delay + retry paste), well within the 2s timeout budget.

### Failure Notification

When paste fails (injection error, sender dropped, or 2s timeout), the Rust pipeline emits an `auto-paste-failed` Tauri event with the message "Text is in your clipboard — press Cmd+V to paste manually." The frontend displays this in the existing error banner and auto-clears it after 5 seconds.

### Why osascript?

Previous approaches tried (`enigo`, `rdev` key simulation) had issues on macOS Sonoma/Sequoia. `osascript` via System Events is the most reliable method for keystroke simulation on modern macOS.

## Linux Auto-Paste

On Linux, `simulate_paste()` uses external tools to simulate `Ctrl+V`. No accessibility permission is required — `is_accessibility_enabled()` always returns `true` on Linux.

### Session Detection

The session type is detected by checking the `WAYLAND_DISPLAY` environment variable:
- **Non-empty** → Wayland session: prefer `wtype`, fall back to `xdotool` (for XWayland apps)
- **Empty or unset** → X11 session: use `xdotool` only

### Wayland path

```
wtype -M ctrl -k v
```

If `wtype` is not installed (`NotFound`), falls back to `xdotool key ctrl+v` to support XWayland-backed applications. If `wtype` runs but exits non-zero (compositor rejected it), the error surfaces for the existing retry-once + `auto-paste-failed` path — no silent swap to `xdotool`.

### X11 path

```
xdotool key ctrl+v
```

### Graceful fallback when tools are missing

If neither `xdotool` nor `wtype` is installed, `simulate_paste()` logs a warning via `tracing` and returns `Ok(())`. The text remains in the clipboard; the caller does **not** emit an `auto-paste-failed` event. This matches the "accessibility not granted" pattern on macOS.

Non-`NotFound` errors (process ran but exited non-zero, permission denied, etc.) still return `Err` and drive the existing retry-once + `auto-paste-failed` banner flow.

### Known limitations

- **Terminal emulators**: `Ctrl+V` does not paste in most terminal emulators (they use `Ctrl+Shift+V`). Users who dictate into terminals should use the clipboard-manual path.
- **Wayland compositor compatibility**: Some compositors (older GNOME/KDE) may reject `wtype`. In that case `wtype` exits non-zero, which triggers the `auto-paste-failed` banner. Disable auto-paste on such systems and use the clipboard.
- **XWayland focus heuristic**: When focused on an XWayland window under a Wayland compositor, `wtype` may target the compositor rather than the XWayland app. The `xdotool` fallback only fires when `wtype` is missing, not when it has no visible effect.

### Threading

`inject_text()` runs on the main thread via `app_handle.run_on_main_thread()` because macOS keyboard APIs require main thread access. On Linux, `std::process::Command` is safe from any thread, so this constraint has no effect.

## Permissions

| Feature | Permission Needed |
|---------|------------------|
| Clipboard copy | None |
| Auto-paste | Accessibility |

The settings panel shows accessibility permission status when auto-paste is enabled, with a "Grant" button that opens System Settings.

## Settings

- `autoPaste: boolean` — enable/disable auto-paste. Persisted to localStorage.
- `autoPasteDelayMs: number` — delay in ms before simulating Cmd+V (default 50, range 10–500). Persisted to localStorage.

Both are sent to the Rust backend via `configure_dictation` command.
