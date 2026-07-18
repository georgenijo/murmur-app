# Dynamic Island Overlay

## Overview

The overlay is an always-on-top transparent window anchored to the macOS notch area, styled as a "Dynamic Island." It shows recording status, an animated audio waveform, and supports click interactions to start/stop recording. The overlay is a separate Tauri window (`label: "overlay"`) that loads its own HTML entry point and has no shared React context with the main window.

## Notch Detection

Notch dimensions are detected via NSScreen APIs on macOS:

- `safeAreaInsets()` — determines menu bar height
- `auxiliaryTopLeftArea()` and `auxiliaryTopRightArea()` — determines the non-notch menu bar area on each side

Notch width is calculated as: `screen width - left auxiliary area - right auxiliary area`.

Results are cached in `State.notch_info` (a `Mutex<Option<(f64, f64)>>`). The `get_notch_info` command returns the cached dimensions to the frontend.

**Fallback:** When no notch is detected (external monitor, older Mac), the overlay uses 200px wide by 37px tall as default dimensions.

## Window Configuration

The overlay window is configured in `tauri.conf.json`:
- Transparent, borderless, not focusable, not resizable
- Always on top, visible on all workspaces, skips taskbar
- Hidden by default (shown via `show_overlay` command)
- Default size: 260x100

### Window Level

The overlay is raised to **NSMainMenuWindowLevel + 1 (level 25)** via NSWindow APIs, placing it above the menu bar and other always-on-top windows.

### Preventing App Activation

Clicking the overlay should not activate the app (which would unhide the main window). This is achieved using the private API `_setPreventsActivation:`, guarded by `respondsToSelector:` for forward compatibility. If the API is unavailable on a future macOS version, the guard prevents a crash.

### Mouse Events

Tauri's `focusable: false` configuration disables mouse events on macOS. The `show_overlay` command explicitly re-enables them via `setIgnoreCursorEvents(false)`.

## Sizing

The visible pill width adjusts based on recording state and hover:

| State | Width | Notes |
|-------|-------|-------|
| Idle (no hover) | `notchWidth + 28` | Compact, shows only the mic icon |
| Recording / Processing | `notchWidth + 68` | Shows waveform and status indicators |
| Hover (idle or recording) | `notchWidth + 68` | Wide enough for the quick-settings dropdown |

The full overlay window width is `notchWidth + 120` (60px expansion per side), with the visible content area sized within that.

Height matches the menu bar height from notch detection. On hover the window grows by `EXPANDED_DROP` (56px) downward to reveal the dropdown (see Hover-Expand below). The overlay is horizontally centered at the top of the screen (y=0).

Width transitions over 400ms and height over 360ms, both using the spring curve `cubic-bezier(0.34, 1.56, 0.64, 1)`.

## Hover-Expand & Quick Settings

Hovering the pill expands it downward into a quick-settings dropdown. The dropdown is identical regardless of state — only the top bar differs.

- **Expand** on `mouseEnter`; **collapse** 300ms after `mouseLeave` (hover-intent delay to avoid flicker during cursor movement).
- Because a transparent overlay with cursor events enabled captures the mouse across its whole frame, the window is **dynamically resized** rather than pre-allocated tall — otherwise the idle overlay would create a click dead-zone below the notch. Expand grows the window first, then the card animates open; collapse animates the card closed, then shrinks the window ~380ms later so the dropdown is never clipped mid-transition.
- Only the **top bar** is a drag region (`data-tauri-drag-region`); the dropdown buttons are not, so they stay clickable.

### Dropdown controls

| Control | Action |
|---------|--------|
| Speaker-slash | Toggles global disable. Calls `set_app_disabled` directly for an immediate gate. When disabled: red icon (`#ef4444`) on `rgba(239,68,68,0.12)`, auto-paste dims to 35%, top-bar mic fades to 15%. |
| Auto-paste toggle | Reads/writes the `autoPaste` setting in localStorage. |
| Gear | Emits `open-settings` and shows/focuses the main window (`WebviewWindow.getByLabel('main')`). |

During recording + hover, an inline `m:ss` timer appears next to the red dot (top bar only; never shown when collapsed).

### Cross-window settings sync

The overlay runs in a separate window with no shared React context. Writes go to localStorage plus an `emit('settings-changed')`; the main window listens and applies the change (`configure` for auto-paste, `set_app_disabled` for disable) with a diff-guard that prevents an echo loop. The main window also emits `settings-changed` on its own auto-paste/disable changes so an already-expanded overlay updates live.

## Visual States

The overlay has three visual states driven by `recording-status-changed` Tauri events:

### Idle
Small mic SVG icon at 40% white opacity. Compact width.

### Recording
Expanded width. Red pulsing dot on the left, animated 7-bar waveform on the right. The waveform responds to real-time audio levels.

### Processing
Same expanded width. Spinning circle on the left, dimmed waveform on the right.

### Hotkey Timing Miss (optional)

When `hotkeyMissFeedback` is enabled and the backend emits `hotkey-tap-rejected` for an expired second-tap window, the pill briefly expands for 500ms with an amber outlined exclamation, amber border glow, and `Tap missed` label. This is visually distinct from the red recording dot and the red cancellation X. The setting is off by default.

**Styling:** Dark background (`rgba(20, 20, 20, 0.92)`), 40px backdrop blur, rounded bottom corners.

## Waveform Animation

7 bars (`BAR_COUNT = 7`) animate via `requestAnimationFrame` with direct DOM manipulation (no React state updates per frame).

- Audio levels arrive via the `audio-level` Tauri event and are stored in a ref
- The rAF loop reads the ref and sets `el.style.height` on each bar element
- Bar heights are computed from: baseline (random jitter), center-weighted envelope (middle bars taller), audio level (scaled x16, capped at 1), and a squared boost with random factor for organic movement
- Animation only runs when `status === 'recording'`; bars reset to 2px when idle

## Click Interactions

The overlay supports both single-click and double-click, disambiguated by a 250ms debounce timer.

**Single click** (after 250ms with no second click):
- If recording: stops recording. Exits locked mode if active.

**Double click** (second click within 250ms cancels the pending single-click timer):
- Toggles "locked mode"
- First double-click starts recording via `invoke('start_native_recording', { deviceName })`
- Second double-click stops recording via `invoke('stop_native_recording')`

### Locked Mode

A boolean tracking whether recording was initiated from the overlay (vs. keyboard). When locked mode is active, single clicks stop recording and exit locked mode. When status returns to `idle`, locked mode is automatically reset to `false`.

### Settings Access

The overlay reads the microphone setting from `localStorage` directly (parsing the full settings object from `STORAGE_KEY`) because it runs in a separate window with no shared React context. This creates a coupling to the localStorage schema — if the settings structure changes, the overlay's direct parsing could break.

## Screen Change Observer

An `NSApplicationDidChangeScreenParametersNotification` observer is registered at startup to handle:
- Monitor plug/unplug
- Lid open/close
- Display configuration changes

When triggered, the observer:
1. Re-detects notch dimensions via NSScreen APIs
2. Updates the cached `State.notch_info`
3. Repositions the overlay window
4. Emits `notch-info-changed` event to the frontend with updated dimensions (or `null` if no notch)

The frontend overlay listens for `notch-info-changed` and updates its internal `notchWidth` state accordingly.

The observer is intentionally leaked (`std::mem::forget`) for app-lifetime observation.

## Commands

| Command | Description |
|---------|-------------|
| `show_overlay` | Positions, sizes, and shows the overlay window. Re-enables mouse events. |
| `hide_overlay` | Hides the overlay window. Gracefully handles missing window. |
| `set_overlay_expanded` | Resizes the overlay height for hover-expand (`base_h + EXPANDED_DROP` when expanded, notch height when collapsed). Width unchanged, top anchored. |
| `get_notch_info` | Returns cached `{ notch_width, notch_height }` or `null`. |

## Events

| Event | Payload | Description |
|-------|---------|-------------|
| `recording-status-changed` | String | Drives visual state transitions |
| `audio-level` | Number (RMS 0.0-1.0) | Real-time audio level for waveform |
| `notch-info-changed` | `{ notch_width, notch_height }` or `null` | Display configuration changed |
| `app-disabled-changed` | Boolean | Global-disable state changed (updates the top-bar mic + speaker-slash) |
| `settings-changed` | (none) | Overlay-relevant settings changed in another window; listeners re-read localStorage |
| `hotkey-tap-rejected` | `{ reason: "second_tap_expired", mode: "double_tap" \| "both" }` | Drives the opt-in amber timing-miss flash |
| `open-settings` | (none) | Overlay gear asks the main window to open the Settings panel |

The entire overlay surface is a Tauri drag region (`data-tauri-drag-region`), allowing the user to reposition it. Overlay position save/restore is currently disabled (TODO: re-enable after notch positioning is stable).
