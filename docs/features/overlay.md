# Dynamic Island Overlay

## Overview

The overlay is an always-on-top transparent window anchored to the macOS notch area, styled as a "Dynamic Island." It shows recording status, an animated audio waveform, and supports click interactions to start/stop recording. The overlay is a separate Tauri window (`label: "overlay"`) that loads its own HTML entry point (`overlay.html` → `src/overlay.tsx` → `OverlayWidget`) and has no shared React context with the main window.

The frontend is layered bottom-up:

1. **Geometry contract** (Rust → `useOverlayGeometry`) — every pixel dimension.
2. **Expansion controller** (`useOverlayExpansion`) — the hover-expand lifecycle and the single writer to the native resize path.
3. **Runtime hooks** (`useOverlayRuntime`, `useRecordingControls`, `useOverlaySettingsMirror`, `useWaveform`) — Tauri event subscriptions, click handling, the settings mirror, and the waveform animation.
4. **Presentational components** (`OverlayPill`, `OverlayDropdown`) — pure rendering, driven by a `deriveVisual()` descriptor.
5. **`OverlayWidget.tsx`** — a composition shell (~150 lines) that calls the hooks above and wires refs/handlers into the island container JSX.

## Notch Detection

Overlay dimensions are detected via NSScreen APIs on macOS:

- `safeAreaInsets()` — determines the physical notch height on notched displays
- `frame()` minus `visibleFrame()` — determines the actual menu-bar height on
  ordinary and external displays
- `auxiliaryTopLeftArea()` and `auxiliaryTopRightArea()` — determines the non-notch menu bar area on each side

Notch width is calculated as: `screen width - left auxiliary area - right auxiliary area`.

Results are cached in `State.notch_info` (a `Mutex<Option<(f64, f64)>>`). The `get_overlay_geometry` command derives an `OverlayGeometry` from the cached notch via `geometry_for()` and returns it to the frontend.

**No-notch displays:** Murmur substitutes only the 80pt synthetic center width;
the collapsed height always comes from the display's measured menu-bar height.
This prevents a fixed synthetic notch from extending onto the desktop when the
menu bar is shorter. If native screen measurement itself is unavailable,
`geometry_for()` retains the fully synthetic `80×37` last-resort fallback.
`get_overlay_geometry` and the `overlay-geometry-changed` event never return
null.

## Window Configuration

The overlay window is configured in `tauri.conf.json`:
- Transparent, borderless, not focusable, not resizable
- Always on top, visible on all workspaces, skips taskbar
- Hidden by default (shown via `show_overlay` command)
- Default size: 260x100 (immediately superseded by the geometry contract on setup)

### Window Level

The overlay is raised to **NSMainMenuWindowLevel + 1 (level 25)** via NSWindow APIs, placing it above the menu bar and other always-on-top windows.

### Preventing App Activation

Clicking the overlay should not activate the app (which would unhide the main window). This is achieved using the private API `_setPreventsActivation:`, guarded by `respondsToSelector:` for forward compatibility. If the API is unavailable on a future macOS version, the guard prevents a crash.

### Mouse Events

Tauri's `focusable: false` configuration disables mouse events on macOS. The `show_overlay` command explicitly re-enables them via `setIgnoreCursorEvents(false)`.

## Geometry Contract

Every overlay dimension comes from one source: `geometry_for(notch)` in `commands/overlay.rs`, which returns an `OverlayGeometry` (`windowW`, `collapsedH`, `expandedH`, `pillIdleW`, `pillActiveW`, `pillMarginIdle`, `pillMarginActive`, `dropdownH`). Rust owns every geometry number; the frontend only reads the struct — via `get_overlay_geometry` (`useOverlayGeometry`, with retry-with-backoff on the initial fetch) and the `overlay-geometry-changed` event — and never hardcodes pixels. No overlay component holds a geometry literal.

- **Left-anchored compact idle width.** `windowW == pillActiveW == notchW + 2·WING`; while truly idle and not hovered, `pillIdleW == notchW + WING`. Both margins are `0`, so the mic-side left edge never moves: compact idle tucks the empty right wing beneath the physical notch, and hover reveals that wing by growing only the right edge. Recording and processing always retain the full active width so their right-side indicators remain visible. `WING = 36` fits the left status icon and right waveform with a little slack.
- **Notched vs. no-notch.** Notched (notch `185×32`): `windowW 257`, `collapsedH 32`, `expandedH 76`, `dropdownH 44`. A typical 30pt external-display menu bar uses the synthetic 80pt center width but the measured height: `windowW 152`, `collapsedH 30`, `expandedH 74`. The fully synthetic `80×37` geometry is reserved for native measurement failure.
- **Window width** (`windowW`) is fixed and horizontally centers the overlay at the top of the primary/menu-bar display using that monitor's physical origin and scale factor.
- **Size transition.** Height grows from `collapsedH` to `expandedH` (`= collapsedH + dropdownH`) while the hover dropdown opens. At the same time, an idle island grows from `pillIdleW` to `pillActiveW`; the fixed left edge makes that width reveal happen only on the right. The native window stays top-anchored and at `windowW`, so recording/processing can use the full top bar immediately.
- **Nothing renders under the physical notch.** The wings hold ONLY the status indicator (left) and the waveform (right). Anything wider than a wing renders below notch height, in the dropdown row: the recording `m:ss` timer (shown when expanded + recording) and the "Tap missed" hotkey-miss label. The amber `!` badge and the amber border glow stay on the pill.
- **Motion tokens** — durations and easing for the width/height transition — live in `app/src/lib/overlayMotion.ts` as the single source; see [Motion tokens](#motion-tokens) below rather than restating numbers here.

### Position calibration

The App → General → Advanced settings section can fine-tune the native overlay
window vertically from -12pt through +12pt. Calibration uses explicit one-point
up/down controls rather than dragging overlay content: each change previews the
actual macOS window position, **Cancel** restores the position from the start of
the session, **Save position** commits the draft to the durable Settings
document, and **Reset** returns it to zero. Merely opening or closing calibration
never saves a position.

`show_overlay` only shows the existing window and re-enables mouse input; it does
not first jump the overlay back to its default coordinates. Startup and display
changes establish the default top-center position, after which the overlay root
applies the confirmed setting. The native command clamps again and records both
target and actual physical coordinates in structured, content-free telemetry.

Settings schema v2 resets offsets produced by the former pointer-drag flow once
and removes its standalone `murmur-overlay-vertical-offset` key. That flow could
persist its +48pt limit even though its preview moved only DOM content, which is
why affected installs could reopen far below the notch.

## Expansion Controller

Hovering the pill expands it downward into a quick-settings dropdown. The dropdown is identical regardless of state — only the top bar differs.

The entire expand/collapse and native-resize lifecycle is owned by one controller hook, `useOverlayExpansion` (`app/src/lib/hooks/useOverlayExpansion.ts`). Nothing else in the overlay calls `set_overlay_expanded` or owns the dwell/collapse/shrink timers — the controller is the single writer to the native resize path. It exposes `{ phase, expanded, expandedRef, islandRef, onHoverStart, onHoverEnd }`; the composition shell attaches `islandRef` to the outer island `<div>` (the poller measures its bounds) and reads `expandedRef` wherever a synchronous "is the card up" check is needed (e.g. the double-click guard in `useRecordingControls`).

### Phase model

The controller runs a four-phase state machine:

| Phase | Meaning |
|-------|---------|
| `collapsed` | Pill only; window at collapsed height. |
| `opening` | Grow requested; **awaiting the resize ack**. The dropdown is not revealed yet. |
| `open` | Ack received; the dropdown is revealed (`expanded` CSS flag = `phase === 'open'`). Also spans the leave-delay. |
| `closing` | Dropdown hidden immediately; the window stays tall until the close animation finishes, then shrinks. |

- **Expand** requires hover intent: the cursor must dwell on the island before opening — grazing the notch does nothing. **Collapse** begins some time after the cursor leaves.
- **Acknowledged ordering:** because a transparent overlay with cursor events enabled captures the mouse across its whole frame, the window is **dynamically resized** rather than pre-allocated tall — otherwise the idle overlay would create a click dead-zone below the notch. Expand enqueues the grow, **awaits the ack from `set_overlay_expanded`** (which returns the applied frame), and only then reveals the card, so CSS can never animate the dropdown into a window that has not yet grown. If the resize is rejected, the controller reverts to `collapsed` without revealing. Collapse hides the card immediately, then shrinks the window one guarded interval later so the dropdown is never clipped mid-transition.
- **Serialized surface writer:** all `set_overlay_expanded` calls flow through one async queue with a generation counter. A newer request supersedes any queued or in-flight older one, and stale acks are dropped, so rapid enter/leave/enter can never apply an out-of-date resize. The native acknowledgment wait is bounded at two seconds, so a hung IPC request cannot wedge every later resize. Leaving during `opening` immediately supersedes the pending grow with a collapse, so a late grow ack cannot reveal the dropdown. Re-entry while `closing` cancels the pending shrink and reopens cleanly. A rejected or timed-out collapse is retried twice inside the same generation-aware queue before the controller gives up, reducing the chance that a transient native resize failure leaves a tall transparent hit area.
- **Motion tokens** — see [Motion tokens](#motion-tokens).
- **Single gated poller:** the overlay is non-activating and sits above the menu bar, so macOS can miss DOM hover events. One interval branches on phase — strict entry bounds arm the dwell while `collapsed`/`closing`; padded exit bounds collapse the card while `open`. Gating: ticks do **no IPC** (no `outerPosition`/`cursorPosition`) while the overlay is **hidden**; while **disabled**, DOM and poller entry are blocked, but the exit watchdog stays alive during an active interaction so clicking the dropdown's own Disable control cannot strand the card open on a missed mouseleave. Visibility is tracked via `overlay-visible-changed`, defaulting to visible on mount; hiding also cancels timers, resets the phase, and serializes a collapse. In-flight cursor results are generation-guarded so they cannot reopen after a visibility or display reset. A display change (`overlay-geometry-changed`) is authoritative: it cancels timers, forces `collapsed`, and enqueues one corrective collapse resize through the writer (which supersedes any straggler grow and repairs the window).
  - **Note:** `overlay-visible-changed` is emitted by the `show_overlay`/`hide_overlay` commands, which are **not currently invoked in production** — the overlay is shown once at setup (`overlay_win.show()` in `lib.rs`) and stays visible. The visibility ref therefore defaults to `true` so first-hover works from mount, and the `disabled`-phase gate is the active battery saver today; the visibility gate is plumbing that activates if/when show/hide get wired to dynamic callers.
- Only the **top bar** is a drag region (`data-tauri-drag-region`, set in `OverlayPill.tsx`); the dropdown buttons are not, so they stay clickable. The dropdown is labeled as the `Quick settings` group and is `aria-hidden` until the expanded-frame acknowledgment reveals it.

### Motion tokens

The transition durations/easings live in `app/src/lib/overlayMotion.ts` as the single source: `OVERLAY_HEIGHT_MS`, `OVERLAY_SPRING`, `HOVER_OPEN_DWELL_MS`, `COLLAPSE_DELAY_MS`. `OVERLAY_ISLAND_TRANSITION` applies the same spring to width and height so the right wing and dropdown open as one surface. `SHRINK_DELAY_MS` is **derived** as `OVERLAY_HEIGHT_MS + 20` rather than a hand-tuned constant, so it cannot drift from the transition it guards. With `prefers-reduced-motion: reduce`, CSS transitions are removed (see `styles.css`) and the controller shrinks immediately after the leave-intent delay, avoiding both motion and a residual transparent hit area.

## Frontend Hooks

`OverlayWidget.tsx` owns the shared `status` state (subscribing to `recording-status-changed` directly) plus `disabled`/`showHotkeyMiss`, and composes the following, in roughly this order (later hooks depend on earlier ones' output):

| Hook | Owns |
|------|------|
| `useOverlayGeometry` | Fetches/subscribes to `OverlayGeometry` (see [Geometry Contract](#geometry-contract)). |
| `useOverlaySettingsMirror` | The localStorage settings snapshot the overlay needs (`autoPaste`, `fileOutputEnabled`, `overlayVerticalOffset`), `applySettingsSnapshot`/`refresh`, the `settings-changed` listener, and the three quick-control actions (toggle auto-paste with rollback-on-failure, toggle global disable, open Settings). |
| `useOverlayRuntime` | The `recording-cancelled` (red-X flash), `hotkey-tap-rejected` (amber flash), `auto-paste-failed` (clipboard-only `⌘V` cue), and `app-disabled-changed` listeners, plus the transient flash timers. `disabled`/`showHotkeyMiss`/`hotkeyMissFeedbackRef` are created in the composition shell (not inside this hook or the settings mirror) because both hooks write into them synchronously and neither can be constructed from the other's return value without an artificial call-order dependency; this hook attaches behavior and re-exposes them. |
| `useOverlayExpansion` (pre-existing, see [Expansion Controller](#expansion-controller)) | The hover-expand lifecycle. |
| `useWaveform` | The `audio-level` listener and the rAF bar-height animation (see [Waveform Animation](#waveform-animation)). |
| `useRecordingControls` | Click/double-click/mousedown disambiguation (250ms debounce) and "locked mode" (see [Click Interactions](#click-interactions)). Reads the microphone override via `loadSettings()` — no raw localStorage parsing. |

Pure, React-free logic lives alongside the presentational components in `app/src/components/overlay/`:

- **`deriveVisual.ts`** — `(status, showCancelled, showHotkeyMiss, disabled) → OverlayVisual`. Encodes the top-bar indicator priority (cancelled > hotkey-miss > recording > processing > idle-with-disabled-dimming) exactly once; locked by an exhaustive matrix test (`deriveVisual.test.ts`) over every status × flag combination.

Presentational components, both driven entirely by props (no hooks beyond `OverlayPill`'s own local elapsed-timer state):

- **`OverlayPill.tsx`** — the top bar (status indicator slot, inline `m:ss` timer, waveform bars). Owns the elapsed-timer effect (keyed on the `status` prop it already needs for rendering — the smallest-plumbing home for it).
- **`OverlayDropdown.tsx`** — the three quick-settings buttons (Power, auto-paste toggle, gear). Icons (`PowerIcon`, `ClipboardPasteIcon`, `SlidersIcon`) are colocated in this file rather than split one-per-file.

The island **container** (sizing, hover handlers, `islandRef`) stays in `OverlayWidget.tsx` itself, since it wraps both `OverlayPill` and `OverlayDropdown` as siblings.

### Dropdown controls

| Control | Action |
|---------|--------|
| Power | Toggles global disable. Calls `set_app_disabled` directly for an immediate gate. When disabled: red icon (`#ef4444`) on a red-tinted background, auto-paste dims to 35%, top-bar mic fades to 15%. Global disable is also a "Disable Murmur" check item in the tray menu; the command keeps the tray check state in sync and the main window persists tray-driven changes. |
| Auto-paste toggle | Reads/writes the `autoPaste` setting via `loadSettings()`/`saveSettings()`. |
| Gear | Emits `open-settings` and shows/focuses the main window (`show_main_window`). |

The dropdown row also carries content too wide for a wing, in a left-anchored slot (absolutely positioned so the buttons stay centered): the recording `m:ss` timer while recording, or the "Tap missed" label during a hotkey-miss flash. Both are only visible while the card is expanded — the transient no-hover cues (red dot / amber `!` badge and border glow) stay on the always-visible pill.

### Cross-window settings sync

The overlay runs in a separate window with no shared React context. Writes go through `saveSettings()` plus an `emit('settings-changed')`; the main window listens and applies the change (`configure` for auto-paste, `set_app_disabled` for disable) with a diff-guard that prevents an echo loop. The main window also emits `settings-changed` on its own auto-paste/disable/calibration changes so an already-open overlay updates live. `useOverlaySettingsMirror` also re-applies the snapshot whenever the expansion controller's `phase` becomes `'opening'`, so the dropdown always shows current settings by the time it is revealed. The Settings calibration control separately emits `overlay-calibration-changed { active }` to switch the overlay into and out of its non-interactive calibration visual while the native window is previewed.

## Visual States

The overlay's top-bar indicator is a pure function of status + transient flags + global-disable (`deriveVisual()`); `status` itself is driven by the `recording-status-changed` event.

### Transform indicators (issue #312)

While a selected-text transform is in **Thinking** (sidecar running), the overlay can show a **transforming…** indicator (`deriveVisual` priority: cancelled > secure-field flash > hotkey-miss > recording > processing > transforming > idle). Secure/password fields refused by the transform capture path emit a content-free `transform-secure-field` flash via `useOverlayRuntime` — no selection text is ever read or shown.

### Idle
Small mic SVG icon at 40% white opacity (dimmed further to 15% when globally disabled). Compact width.

### Recording
The red pulsing dot occupies the visible left wing and the animated 7-bar waveform occupies the right wing. The `m:ss` elapsed timer renders in the dropdown row (visible on hover-expand), not in a wing. No transcript text is displayed while recording. (Width is constant across all states.)

### Starting and Recovering

Starting uses a blue pulse; after the backend's 5-second signal it becomes an
amber "Still connecting" cue. Clicking or toggling again cancels. Recovering
uses an amber spinner and dropdown label during cancellation recovery; the app
returns to Idle only after the exclusively owned Core Audio thread exits.
Initialization failure gets a bounded red warning flash. A normal
recording stop whose teardown remains blocked surfaces persistent guidance in
the main recording-error area.

### Processing
Spinning circle in the left wing; the waveform is hidden (visible only while recording). No transcript text is displayed while processing.

### Clipboard Only (transient)

When automatic paste is refused or fails after the clipboard write succeeds,
the collapsed pill shows an accessible green `⌘V` cue for 5 seconds. Active
recording, processing, meeting, transform, and higher-priority failure states
remain truthful and take visual precedence; the cue appears once those states
return to idle. It is non-interactive and never focuses Murmur, expands the
surface, resizes the window, or retries the paste.

### Cancelled (transient)
An 800ms red-X flash, triggered by `recording-cancelled`. Takes priority over every other indicator.

### Hotkey Timing Miss (optional, transient)
When `hotkeyMissFeedback` is enabled and the backend emits `hotkey-tap-rejected` for an expired second-tap window, the pill shows an amber outlined exclamation (`!`) in the left wing plus an amber border glow. The `Tap missed` text label renders in the dropdown row (below notch height, since text is wider than a wing) and is therefore visible while the card is expanded; the `!` and border glow are the always-visible transient cue. Takes priority over every indicator except cancelled. The setting is off by default.

**Styling:** Dark background (`rgba(20, 20, 20, 0.92)`), 40px backdrop blur, rounded bottom corners.

## Waveform Animation

7 bars (`BAR_COUNT` in `useWaveform.ts`) animate via `requestAnimationFrame` with direct DOM manipulation (no React state updates per frame).

- Audio levels arrive via the `audio-level` Tauri event and are stored in a ref
- The rAF loop reads the ref and sets `el.style.height` on each bar element
- Bar heights are computed from: baseline (random jitter), center-weighted envelope (middle bars taller), audio level (scaled x16, capped at 1), and a squared boost with random factor for organic movement
- Animation only runs when `status === 'recording'`; bars reset to 2px when idle

## Click Interactions

`useRecordingControls` disambiguates single-click and double-click with a 250ms debounce timer.

**Single click** (after 250ms with no second click):
- If recording: stops recording. Exits locked mode if active.

**Double click** (second click within 250ms cancels the pending single-click timer):
- Ignored while the dropdown is expanded or opening (`expandedRef`), or while `processing`, or while globally disabled and idle.
- Toggles "locked mode"
- First double-click starts recording via `invoke('start_native_recording', { deviceName })`
- Second double-click stops recording via `invoke('stop_native_recording')`

### Locked Mode

A boolean tracking whether recording was initiated from the overlay (vs. keyboard). When locked mode is active, single clicks stop recording and exit locked mode. When status returns to `idle`, locked mode is automatically reset to `false`.

### Settings Access

The overlay reads settings from `localStorage` via the validated `loadSettings()` API (never a raw `JSON.parse` of the stored blob) because it runs in a separate window with no shared React context. `loadSettings()` sanitizes/migrates the stored shape, so a malformed or stale blob degrades to defaults rather than surfacing a parse error.

## Screen Change Observer

An `NSApplicationDidChangeScreenParametersNotification` observer is registered at startup to handle:
- Monitor plug/unplug
- Lid open/close
- Display configuration changes

Notifications enter a trailing 125ms debounce. After the burst, Murmur compares
the full display snapshot (notch/menu-bar geometry, monitor bounds and origin,
and scale). Only a changed snapshot updates `State.notch_info`, repositions the
overlay, and emits `overlay-geometry-changed`; isolated but identical
notifications are logged once and otherwise ignored.

The frontend `useOverlayGeometry` hook listens for `overlay-geometry-changed` and updates its geometry state accordingly; `useOverlayExpansion` treats the same event as an authoritative reset (see [Expansion Controller](#expansion-controller)).

The observer is intentionally leaked (`std::mem::forget`) for app-lifetime observation.

## Commands and Events

See [docs/reference/commands.md](../reference/commands.md) (Overlay section) and [docs/reference/events.md](../reference/events.md) (Overlay Events section) for the authoritative, up-to-date list. Summary of what the overlay itself calls/listens to:

- Calls: `get_overlay_geometry`, `set_overlay_expanded`, `show_main_window`, `start_native_recording`, `stop_native_recording`, `set_app_disabled`, `configure_dictation`.
- Listens: `overlay-geometry-changed`, `overlay-visible-changed`, `recording-status-changed`, `recording-cancelled`, `auto-paste-failed`, `hotkey-tap-rejected`, `app-disabled-changed`, `audio-level`, `settings-changed`.

`set_overlay_expanded` **returns the applied frame** as `AppliedSurface { windowW, windowH }`; the expansion controller awaits this value as the resize ack before revealing the dropdown. `show_overlay`/`hide_overlay` emit `overlay-visible-changed(true|false)`, which gates the controller's cursor poller so it does no IPC while the overlay is hidden.

## Transparent window caveat

The shared `styles.css` gives `body` an opaque surface background. The overlay's body carries the `overlay-window` class (`overlay.html`), which scopes it back to transparent — without it the whole overlay window frame paints as a dark box around the island.

The Appearance Theme Engine deliberately does not synchronize this window.
Overlay glass remains always dark, and its body remains transparent in System,
Light, and Dark modes. Appearance application must never set an `html` or
`body` background that can override this invariant.
