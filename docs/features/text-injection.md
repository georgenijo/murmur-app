# Text Injection

## Overview

After transcription, text is always copied to the clipboard. Optionally, the app simulates a paste keystroke into the focused application: native CoreGraphics `Cmd+V` events on macOS, `Ctrl+V` via `xdotool` (X11) or `wtype` (Wayland) on Linux.

## Clipboard (`injector.rs`)

Uses `arboard` crate (maintained by 1Password). Text is set via `Clipboard::new()` + `clipboard.set_text()`.

This always happens, regardless of auto-paste setting. The user can always manually Cmd+V.

## Auto-Paste

When `auto_paste` is enabled in settings:

1. Copy text to clipboard
2. Check `AXIsProcessTrusted()` — if accessibility not granted, stop here (text is still in clipboard)
3. Wait for the configurable delay (default 0ms) for window focus to settle
4. Compare the current frontmost PID with the PID frozen at recording start. If the user moved to a different application, stop here and surface a clipboard-only notification rather than pasting into the new target.
5. Resolve the frontmost process with `NSWorkspace` and query its focused element role with the macOS Accessibility API. Native AX timeout (`-25204`) returns `Unknown` immediately and skips the fallback (allow-paste). A native no-value response (`-25212`) also skips the fallback for non-Finder apps because web-backed editors can accept Cmd+V without exposing a focused AX element; Finder retains the compatibility query so its desktop/file-view guard remains effective. Other native failures fall back to the previous System Events `osascript` query.
6. Skip auto-paste only when the focused role is on the confirmed non-editable denylist; unknown roles still allow paste
7. Post Command-modified `V` key-down and key-up events through the CoreGraphics HID event tap. If event construction fails, fall back to the previous System Events `osascript` paste
8. If the paste attempt reports a failure, wait 100ms and retry once
9. If both attempts fail, emit `auto-paste-failed` so the frontend can notify the user

### Delay Rationale

The clipboard write (`arboard::set_text()` → `NSPasteboard`) is synchronous, so no delay is needed for clipboard sync. The delay exists solely to let macOS window focus settle after the transcription pipeline returns. The zero-delay default is sufficient for the native path; users can increase up to 500ms via the settings slider for applications that move focus asynchronously. The recording-target PID check runs after this delay, immediately before the focused-field query.

### Configurable Delay

The paste delay is configurable via a range slider in the settings panel (0–500ms, step 10ms). The slider appears when auto-paste is enabled. The value is sent to the Rust backend via `configure_dictation` and clamped to the 0–500 range. This configured delay is one component of the broader **Clipboard / paste** performance stage, which also measures the clipboard write, target/focus safety queries, Cmd+V event, and small dispatch overhead.

### Retry Behavior

CoreGraphics event posting has no delivery result, so a successful native post completes immediately. Event construction failures use the `osascript` compatibility path, whose non-zero exit status is observable. Each AppleScript fallback is forcibly terminated after 250ms. If a paste attempt returns an error, the injector logs a warning, waits 100ms, and retries once. Only after both attempts fail does it return an error; the caller also enforces a 2s timeout for the complete injection operation.

### Failure Notification

When paste fails (injection error, sender dropped, or 2s timeout), the Rust pipeline emits an `auto-paste-failed` Tauri event. A recording-target mismatch uses the specific message "App focus changed. Text is in your clipboard; paste it when ready." Other failures retain the manual-paste hint. The frontend displays this in the existing error banner and auto-clears it after 5 seconds.

### Native path and compatibility fallback

The primary path avoids launching System Events twice per dictation: `NSWorkspace` and `AXUIElement` inspect focus in-process, while `CGEvent` posts Cmd+V in-process. The previous `osascript` implementation remains as a compatibility fallback because earlier `enigo` and `rdev` key simulation approaches had reliability issues on macOS Sonoma and Sequoia.

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

`inject_text()` runs on the main thread via `app_handle.run_on_main_thread()` so its AppKit focus lookup and macOS keyboard APIs execute in the expected context. On Linux, `std::process::Command` is safe from any thread, so this constraint has no effect.

## Permissions

| Feature | Permission Needed |
|---------|------------------|
| Clipboard copy | None |
| Auto-paste | Accessibility |

Settings > Delivery shows accessibility permission status when effective
auto-paste is enabled, with a "Grant" button that opens System Settings.

## Settings

- `autoPaste: boolean` — enable/disable auto-paste. Persisted to localStorage.
- `autoPasteDelayMs: number` — delay in ms before simulating Cmd+V (default 0, range 0–500). Persisted to localStorage.

Both are sent to the Rust backend via `configure_dictation` command.

## Save to File

Live hotkey dictation can optionally persist its output to disk via two independent toggles in Settings > Delivery:

- `saveTranscript: boolean` — write each transcription to a sequentially numbered `.txt`.
- `saveAudio: boolean` — write each recording to a matching `.wav` (16kHz mono, 16-bit PCM).
- `outputDir: string` — destination folder; empty means the default `Documents/Murmur` (created on first write).

Writing happens in `file_output.rs`, called from `run_transcription_pipeline` after the cancellation checkpoints and before injection. The WAV is written from the original (pre-VAD) 16kHz samples; the `.txt` is only written when the transcript is non-empty. A short sequential base name (`murmur-0001`, `murmur-0002`, …) is shared by the pair. The next number is the highest existing `murmur-NNNN` in the folder plus one (older timestamped names are ignored when numbering).

**Interaction with auto-paste:** when either toggle is on, the recording is treated as a "capture to file" action — the clipboard write still happens (clipboard-first is unconditional), but auto-paste is suppressed (`effective_auto_paste = auto_paste && !(save_transcript || save_audio)`). With both toggles off, behavior is unchanged. Write failures are non-fatal: they are logged and surfaced to the UI via the `file-output-failed` event (text remains in the clipboard).

The UI mirrors this effective state without mutating the stored `autoPaste`
preference: the switch appears off and unavailable while file output is active.
When the stored preference is on, the copy identifies it as paused and says it
will resume when both file toggles are off; when it is already off, the copy
says it remains off.

**Known limitation:** recordings the VAD classifies as no-speech return early before the write step, so they save neither file.
