# Text Injection

## Overview

After transcription, text is always copied to the clipboard. Optionally, the app simulates Cmd+V to paste into the focused application.

## Clipboard (`injector.rs`)

Uses `arboard` crate (maintained by 1Password). Text is set via `Clipboard::new()` + `clipboard.set_text()`.

This always happens, regardless of auto-paste setting. The user can always manually Cmd+V.

## Auto-Paste

When `auto_paste` is enabled in settings:

1. Copy text to clipboard
2. Check `AXIsProcessTrusted()` — if accessibility not granted, stop here (text is still in clipboard)
3. Wait 150ms for clipboard sync and window focus to settle
4. Run `osascript -e 'tell application "System Events" to keystroke "v" using command down'`

### Why osascript?

Previous approaches tried (`enigo`, `rdev` key simulation) had issues on macOS Sonoma/Sequoia. `osascript` via System Events is the most reliable method for keystroke simulation on modern macOS.

### Threading

`inject_text()` runs on the main thread via `app_handle.run_on_main_thread()` because macOS keyboard APIs require main thread access.

## Permissions

| Feature | Permission Needed |
|---------|------------------|
| Clipboard copy | None |
| Auto-paste | Accessibility |

The settings panel shows accessibility permission status when auto-paste is enabled, with a "Grant" button that opens System Settings.

## Settings

`autoPaste: boolean` in `Settings` interface. Persisted to localStorage. Sent to Rust backend via `configure_dictation` command.
