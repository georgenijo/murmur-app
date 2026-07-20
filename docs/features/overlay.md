# Dynamic Island Overlay

## Overview

The overlay is an always-on-top transparent window anchored to the macOS notch area, styled as a "Dynamic Island." It shows recording status, an animated audio waveform, and supports click interactions to start/stop recording. The overlay is a separate Tauri window (`label: "overlay"`) that loads its own HTML entry point and has no shared React context with the main window.

## Notch Detection

Notch dimensions are detected via NSScreen APIs on macOS:

- `safeAreaInsets()` — determines menu bar height
- `auxiliaryTopLeftArea()` and `auxiliaryTopRightArea()` — determines the non-notch menu bar area on each side

Notch width is calculated as: `screen width - left auxiliary area - right auxiliary area`.

Results are cached in `State.notch_info` (a `Mutex<Option<(f64, f64)>>`). The `get_overlay_geometry` command derives an `OverlayGeometry` from the cached notch via `geometry_for()` and returns it to the frontend.

**Fallback:** When no notch is detected (external monitor, older Mac), `geometry_for()` substitutes a synthetic notch of `80×37`, producing a `200×37` overlay window.

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

Every overlay dimension comes from one source: `geometry_for(notch)` in `commands/overlay.rs`, which returns an `OverlayGeometry`. Rust owns all geometry numbers; the frontend only reads the struct (via `get_overlay_geometry` and the `overlay-geometry-changed` event) and never hardcodes pixels.

The visible pill width adjusts based on recording state and hover:

- **Idle (no hover):** `pillIdleW`, centered in the window by a `pillMarginIdle` left margin.
- **Recording / Processing / hover-expanded:** `pillActiveW`, which fills the window (`pillMarginActive = 0`).

The full overlay window is `windowW` wide and is horizontally centered at the top of the screen (y=0).

Height is `collapsedH` at rest. Hover is the only dynamic height change: the window grows to `expandedH`, where `expandedH = collapsedH + dropdownH`.

Width transitions over 400ms and height over 360ms, both using the spring curve `cubic-bezier(0.34, 1.56, 0.64, 1)`.

## Hover-Expand & Quick Settings

Hovering the pill expands it downward into a quick-settings dropdown. The dropdown is identical regardless of state — only the top bar differs.

The entire expand/collapse and native-resize lifecycle is owned by one controller hook, `useOverlayExpansion` (`app/src/lib/hooks/useOverlayExpansion.ts`). Nothing else in the overlay calls `set_overlay_expanded` or owns the dwell/collapse/shrink timers — the controller is the single writer to the native resize path.

### Phase model

The controller runs a four-phase state machine:

| Phase | Meaning |
|-------|---------|
| `collapsed` | Pill only; window at collapsed height. |
| `opening` | Grow requested; **awaiting the resize ack**. The dropdown is not revealed yet. |
| `open` | Ack received; the dropdown is revealed (`expanded` CSS flag = `phase === 'open'`). Also spans the leave-delay. |
| `closing` | Dropdown hidden immediately; the window stays tall until the close animation finishes, then shrinks. |

- **Expand** requires hover intent: the cursor must dwell on the island for `HOVER_OPEN_DWELL_MS` (150ms) before opening — grazing the notch does nothing. **Collapse** begins 300ms after the cursor leaves.
- **Acknowledged ordering:** because a transparent overlay with cursor events enabled captures the mouse across its whole frame, the window is **dynamically resized** rather than pre-allocated tall — otherwise the idle overlay would create a click dead-zone below the notch. Expand enqueues the grow, **awaits the ack from `set_overlay_expanded`** (which returns the applied frame), and only then reveals the card, so CSS can never animate the dropdown into a window that has not yet grown. If the resize is rejected, the controller reverts to `collapsed` without revealing. Collapse hides the card immediately, then shrinks the window `SHRINK_DELAY_MS` later so the dropdown is never clipped mid-transition.
- **Serialized surface writer:** all `set_overlay_expanded` calls flow through one async queue with a generation counter. A newer request supersedes any queued or in-flight older one, and stale acks are dropped, so rapid enter/leave/enter can never apply an out-of-date resize. Re-entry while `closing` cancels the pending shrink and reopens cleanly.
- **Motion tokens:** the transition durations/easings live in `app/src/lib/overlayMotion.ts` (width 400ms, height 360ms, spring `cubic-bezier(0.34,1.56,0.64,1)`, dwell 150ms, collapse-delay 300ms). `SHRINK_DELAY_MS` is **derived** as `OVERLAY_HEIGHT_MS + 20` (= 380ms) rather than a hand-tuned constant, so it can never drift from the height transition it guards. The island's `transition` string is templated from these tokens.
- **Single gated poller:** the overlay is non-activating and sits above the menu bar, so macOS can miss DOM hover events. One 150ms interval branches on phase — strict entry bounds arm the dwell while `collapsed`/`closing`; padded exit bounds collapse the card while `open`. Gating: ticks do **no IPC** (no `outerPosition`/`cursorPosition`) while the overlay is **hidden**; while **disabled**, only the `collapsed` entry detector is skipped (battery) — the exit watchdog stays alive for `open`/`closing`, so clicking the dropdown's own Disable control can never strand the card open on a missed mouseleave. Visibility is tracked via `overlay-visible-changed`, defaulting to visible on mount. A display change (`overlay-geometry-changed`) is authoritative: it cancels timers, forces `collapsed`, and enqueues one corrective collapse resize through the writer (which supersedes any straggler grow and repairs the window).
  - **Note:** `overlay-visible-changed` is emitted by the `show_overlay`/`hide_overlay` commands, which are **not currently invoked in production** — the overlay is shown once at setup (`overlay_win.show()` in `lib.rs`) and stays visible. The visibility ref therefore defaults to `true` so first-hover works from mount, and the `disabled`-phase gate is the active battery saver today; the visibility gate is plumbing that activates if/when show/hide get wired to dynamic callers.
- Only the **top bar** is a drag region (`data-tauri-drag-region`); the dropdown buttons are not, so they stay clickable.

### Dropdown controls

| Control | Action |
|---------|--------|
| Power | Toggles global disable. Calls `set_app_disabled` directly for an immediate gate. When disabled: red icon (`#ef4444`) on `rgba(239,68,68,0.12)`, auto-paste dims to 35%, top-bar mic fades to 15%. Global disable is also a "Disable Murmur" check item in the tray menu; the command keeps the tray check state in sync and the main window persists tray-driven changes. |
| Auto-paste toggle | Reads/writes the `autoPaste` setting in localStorage. |
| Gear | Emits `open-settings` and shows/focuses the main window (`WebviewWindow.getByLabel('main')`). |

During recording, an inline `m:ss` timer remains visible next to the red dot in the left wing without requiring hover.

### Cross-window settings sync

The overlay runs in a separate window with no shared React context. Writes go to localStorage plus an `emit('settings-changed')`; the main window listens and applies the change (`configure` for auto-paste, `set_app_disabled` for disable) with a diff-guard that prevents an echo loop. The main window also emits `settings-changed` on its own auto-paste/disable changes so an already-expanded overlay updates live.

## Visual States

The overlay has three visual states driven by `recording-status-changed` Tauri events:

### Idle
Small mic SVG icon at 40% white opacity. Compact width.

### Recording
Expanded width. The red pulsing dot and elapsed timer occupy the visible left wing, while the animated 7-bar waveform occupies the right. No transcript text is displayed while recording.

### Processing
Same expanded width. Spinning circle on the left; the waveform is hidden (visible only while recording). No transcript text is displayed while processing.

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
4. Emits `overlay-geometry-changed` to the frontend carrying a full `OverlayGeometry` (never null — `geometry_for()` always resolves, using the synthetic fallback notch when none is present)

The frontend `useOverlayGeometry` hook listens for `overlay-geometry-changed` and updates its geometry state accordingly.

The observer is intentionally leaked (`std::mem::forget`) for app-lifetime observation.

## Commands

| Command | Description |
|---------|-------------|
| `show_overlay` | Positions, sizes, and shows the overlay window. Re-enables mouse events. Emits `overlay-visible-changed(true)` after showing. |
| `hide_overlay` | Hides the overlay window. Gracefully handles missing window. Emits `overlay-visible-changed(false)` after hiding. |
| `set_overlay_expanded` | Switches between the collapsed and expanded frames while keeping the window top anchored, then **returns the applied frame** as `AppliedSurface { windowW, windowH }`. The expansion controller awaits this value as the resize ack before revealing the dropdown. Sizes are derived from `geometry_for()`. |
| `get_overlay_geometry` | Returns the current `OverlayGeometry` (never null) derived from the cached notch via `geometry_for()`. |

`set_overlay_expanded` and `position_overlay_default` both size the window from `geometry_for()`, so they stay consistent.

## Events

| Event | Payload | Description |
|-------|---------|-------------|
| `recording-status-changed` | String | Drives visual state transitions |
| `audio-level` | Number (RMS 0.0-1.0) | Real-time audio level for waveform |
| `overlay-geometry-changed` | `OverlayGeometry` | Display configuration changed; carries the recomputed geometry (never null). Authoritative reset for the expansion controller (forces `collapsed`). |
| `overlay-visible-changed` | Boolean | Overlay window shown (`true`) / hidden (`false`); gates the expansion controller's cursor poller so it does no IPC while hidden |
| `app-disabled-changed` | Boolean | Global-disable state changed (updates the top-bar mic + speaker-slash) |
| `settings-changed` | (none) | Overlay-relevant settings changed in another window; listeners re-read localStorage |
| `hotkey-tap-rejected` | `{ reason: "second_tap_expired", mode: "double_tap" \| "both" }` | Drives the opt-in amber timing-miss flash |
| `open-settings` | (none) | Overlay gear asks the main window to open the Settings panel |

Only the top bar is a Tauri drag region (`data-tauri-drag-region`); the dropdown controls remain clickable. Overlay position save/restore is currently disabled (TODO: re-enable after notch positioning is stable).

## Transparent window caveat

The shared `styles.css` gives `body` an opaque surface background. The overlay's body carries the `overlay-window` class (`overlay.html`), which scopes it back to transparent — without it the whole overlay window frame paints as a dark box around the island.
