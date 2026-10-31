# Dynamic Island Overlay

## Overview

The overlay is an always-on-top transparent window anchored to the macOS notch area, styled as a "Dynamic Island." It shows recording status, a native Metal voice dot field with Canvas2D fallback, and supports click interactions to start/stop recording. The overlay is a separate Tauri window (`label: "overlay"`) that loads its own HTML entry point (`overlay.html` → `src/overlay.tsx` → `OverlayWidget`) and has no shared React context with the main window.

The frontend is layered bottom-up:

1. **Geometry contract** (Rust → `useOverlayGeometry`) — every pixel dimension.
2. **Expansion controller** (`useOverlayExpansion`) — the hover-expand lifecycle and the single writer to the native resize path.
3. **Runtime hooks** (`useOverlayRecordingStatus`, `useOverlayRuntime`, `useRecordingControls`, `useOverlaySettingsMirror`) — Tauri event subscriptions, click handling, the settings mirror, and settings synchronization.
4. **Presentational components** (`OverlayIsland`, `OverlayPill`, `OverlayDropdown`, `DotField`) — pure rendering, driven by a `deriveVisual()` descriptor.
5. **`OverlayWidget.tsx`** — a composition shell that calls the hooks above and wires refs/handlers into the island container JSX.

## Native field renderer

On macOS the decorative 43×9 field is a transparent, non-interactive MetalKit
view inside the existing overlay window. The webview still owns status priority,
clicks, hover, menus and live words. `useNativeDotField` serializes stage/style,
visibility and Reduced Motion updates; Rust verifies the caller is `overlay`,
resolves geometry, and dispatches every native view mutation on the main thread.
The native view declines hit testing and first-responder status. No text enters
its API. The existing throttled RMS publisher supplies only a scalar level.

`macos/OverlayDotRenderer.swift` implements the same bounded 240Hz simulation
and native display-paced painting. Three Metal buffers protect in-flight frames.
Idle/hidden work pauses, and Reduced Motion settles into one static field. Native
frames do not execute JavaScript or per-frame IPC. Unsupported initialization
falls back to the existing Canvas component. `node scripts/test_overlay_dot_scope.mjs`
compares the real Swift and TypeScript simulations through mixed refresh rates,
all processing styles, restarts and motion changes; CI runs it on macOS.

For explicit local measurements, `MURMUR_OVERLAY_METRICS=1` records bounded,
content-free `MTLDrawable.presentedTime` timing summaries to stderr. The optional
counter is absent from ordinary runs. Screen recordings and process CPU/RSS
measurements remain separate native acceptance evidence.

Hidden WKWebViews can receive a generic settings event before their storage
cache changes. Host persistence therefore sends targeted commit notifications:
preview consent is re-read from disk (a true notification alone cannot enable
words), and overlay controls read a validated snapshot without overwriting a
shared cache. Generation guards discard older responses. Hover refresh and
quick-control writes retain the latest confirmed/optimistic snapshot.

## Notch Detection

Overlay dimensions are detected via NSScreen APIs on macOS:

- `safeAreaInsets()` — determines the physical notch height on notched displays
- `frame()` minus `visibleFrame()` — determines the actual menu-bar height on
  ordinary and external displays
- `auxiliaryTopLeftArea()` and `auxiliaryTopRightArea()` — determines the non-notch menu bar area on each side

Notch width is calculated as: `screen width - left auxiliary area - right auxiliary area`.

The selected screen is matched by its `NSScreenNumber` to CoreGraphics' main
display ID, the same display Tauri uses for `primary_monitor()`. Its notch
measurements and physical display/work-area bounds are captured together when
the display topology changes. Results are cached in `State.notch_info` as a
`NotchInfo` with an explicit physical-notch flag. `get_overlay_geometry`
derives an `OverlayGeometry` from that snapshot.

**No-notch displays:** the default island uses a 185pt pretend notch at the
screen top. Active width is the notch width plus 56pt, not the former 280pt
floating waveform. The measured menu-bar height is retained (24–128pt, 37pt
fallback). App → General → Recording overlay can instead select **Below-menu-bar
pill**, on either physical-notched or external displays. This mode is 6pt below
the usable menu-bar/safe-area top, including fullscreen Spaces.

Horizontal centering depends only on the selected display, never app names,
menu titles or status items. Native calibration and work-area clamps apply to
the resolved placement. A display measurement failure uses the same bounded
pretend-notch contract; geometry is never null.

### Placement refresh

Menu-bar visibility and size can change with the display, the Space, or an
app that hides the menu bar on the current Space, so one worker thread
(`murmur-overlay-placement`) re-measures the whole placement together for each
of these triggers (`PlacementTrigger`):

- startup;
- `NSApplicationDidChangeScreenParametersNotification`;
- `NSWorkspaceDidActivateApplicationNotification` (an app can hide or restore
  the menu bar without a Space change);
- `NSWorkspaceActiveSpaceDidChangeNotification` (a fullscreen Space hides the
  menu bar, a desktop Space restores it).

The activation and Space observers live in `frontmost.rs` and only post a
non-blocking channel message, in addition to their delivery counters. A
re-measure reads only screen geometry; the centred x never depends on the
frontmost application, so an app switch moves the overlay only if menu-bar
visibility or height actually changed. Bursts
are debounced (125ms quiet, 1s cap). After any trigger, the same ladder
re-checks after 600ms, then 1.5s and 3s from the preceding refresh, to catch
the menu bar animating in or out; any new trigger supersedes the remaining
re-checks. Each refresh re-measures the screen on the main thread, builds one
`DisplaySnapshot`, and, only if it differs from the cached one, updates
`State.notch_info`, repositions and resizes the native window while retaining
its calibration intent and recomputing the effective offset, and emits
`overlay-geometry-changed` (the frontend collapses its expansion state).

Each refresh logs a content-free `overlay.placement` event (target `system`)
with the trigger, `physical_notch`, `menu_bar_visible`, `display_left`,
`display_w`, the resolved `band_center_x`, `in_menu_bar_band`, and `changed`.
It carries no app names, bundle identifiers, or titles.

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

`geometry_for_layout(notch, content, layout)` in `commands/overlay.rs` owns the
native target geometry. `OverlayGeometry` includes the window/pill/dropdown
boxes and mandatory `island` metrics. `get_overlay_geometry` and topology events
feed `useOverlayGeometry`; successful native resize transactions separately emit
`overlay-surface-changed`, so activity changes do not reset hover ownership.
Rust tests compare all layout profiles to the JSON consumed by browser fixtures.

- Metrics: pad/inset 7pt, top offset 6pt, notch gap 10pt, spread 28pt; plate height
  26pt; idle/recording/processing plate widths 30/112/76pt; idle/active corners
  8/18pt; field clip radius 11pt.
- Truly idle notch mode fits the notch itself. Active mode is `notchW + 56` wide
  and `notchH + 40` tall. A measured 185×32 notch therefore uses idle 185×32 and
  active 241×72. An external 30pt bar uses active 241×70.
- Below-menu-bar mode has no notch spacer: idle/recording/processing windows are
  44/126/90pt wide and 40pt high. Expanded controls reserve at least 241pt width
  and 64pt dropdown height. Meeting suggestions reserve 320pt width and a 136pt
  dropdown; their text/actions use the production component, not a stand-in.
- Status/refusal/delivery cues replace the decorative field in the visible
  plate. Mode, quick controls and elapsed time remain in the expanded dropdown.
- Logical points are resolved on the selected menu-bar display, including
  nonzero monitor origins and 1×/2× scale transitions. Rust alone mutates native
  geometry on the main thread.
- Click-through live words follow the actual applied overlay frame on every
  placement, centered and clamped to the work area, with a 6pt gap. Native size
  is at most 560×44; height is capped by the remaining work area. Interactive
  Voice Query controls instead reserve conservative expanded clearance, so
  hover does not move actions underneath the pointer.
- State-driven contractions first acknowledge a target while retaining larger
  previous native bounds (`deferShrink`), animate the centered visual surface,
  then serialize an exact-size request after spring settlement. This avoids
  clipping and releases temporary transparent hit area. Hover opening still
  requires native growth acknowledgement before revealing controls.

### Position calibration

The App → General → Advanced settings section can fine-tune the native overlay
window vertically within a maximum range of -12pt through +12pt. The native
command returns the effective range for the current display and window height,
so a pill already at the screen top does not advertise unavailable upward travel.
Calibration uses explicit one-point
up/down controls rather than dragging overlay content: each change previews the
actual macOS window position, **Cancel** restores the position from the start of
the session, **Save position** commits the draft to the durable Settings
document, and **Reset** returns it to zero. Merely opening or closing calibration
never saves a position.

`show_overlay` only shows the existing window and re-enables mouse input; it does
not first jump the overlay back to its default coordinates. The overlay root
initializes native ownership from the confirmed setting once. Native placement
refreshes then retain the requested offset—including an active unsaved draft—and
recompute what the current anchor can apply, so Space or display changes
cannot reset it. The native command clamps again
and records both target and actual physical coordinates in structured,
content-free telemetry.

Physical-notch calibration intentionally retains the full ±12pt fine-tuning
range; the screen-top clamp applies only to a pretend-notch island. Below-menu-bar calibration is clamped to the
usable safe area and may advertise a smaller effective range.

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

While closing, a brief hover leaves the native shrink scheduled until the open
dwell actually commits. The cursor watchdog can cancel that dwell when macOS
misses a DOM leave; cancelling the shrink at hover entry would otherwise strand
the collapsed pill in an expanded, transparent click window. Pointer handlers
belong to the visible island, not its transparent full-window wrapper.

- **Expand** requires hover intent: the cursor must dwell on the island before opening — grazing the notch does nothing. **Collapse** begins some time after the cursor leaves.
- **Acknowledged ordering:** because a transparent overlay with cursor events enabled captures the mouse across its whole frame, the window is **dynamically resized** rather than pre-allocated tall — otherwise the idle overlay would create a click dead-zone below the notch. Expand enqueues the grow, **awaits the ack from `set_overlay_expanded`** (which returns the applied frame), and only then reveals the card, so CSS can never animate the dropdown into a window that has not yet grown. If the resize is rejected, the controller reverts to `collapsed` without revealing. Collapse hides the card immediately, then shrinks the window one guarded interval later so the dropdown is never clipped mid-transition.
- **Serialized surface writer:** all `set_overlay_expanded` calls flow through one async queue with a generation counter. A newer request supersedes any queued or in-flight older one, and stale acks are dropped, so rapid enter/leave/enter can never apply an out-of-date resize. The native acknowledgment wait is bounded at two seconds, so a hung IPC request cannot wedge every later resize. Leaving during `opening` immediately supersedes the pending grow with a collapse, so a late grow ack cannot reveal the dropdown. Sustained re-entry while `closing` cancels the pending shrink once the open dwell commits and reopens cleanly. A rejected or timed-out collapse is retried twice inside the same generation-aware queue before the controller gives up, reducing the chance that a transient native resize failure leaves a tall transparent hit area.
- **Motion tokens** — see [Motion tokens](#motion-tokens).
- **Single gated poller:** the overlay is non-activating and sits above the menu bar, so macOS can miss DOM hover events. One interval branches on phase — strict entry bounds arm the dwell while `collapsed`/`closing`; padded exit bounds collapse the card while `open`. Gating: ticks do **no IPC** (no `outerPosition`/`cursorPosition`) while the overlay is **hidden**; while **disabled**, DOM and poller entry are blocked, but the exit watchdog stays alive during an active interaction so clicking the dropdown's own Disable control cannot strand the card open on a missed mouseleave. Visibility is tracked via `overlay-visible-changed`, defaulting to visible on mount; hiding also cancels timers, resets the phase, and serializes a collapse. In-flight cursor results are generation-guarded so they cannot reopen after a visibility or display reset. A display change (`overlay-geometry-changed`) is authoritative: it cancels timers, forces `collapsed`, and enqueues one corrective collapse resize through the writer (which supersedes any straggler grow and repairs the window).
  - **Note:** `overlay-visible-changed` is emitted by the `show_overlay`/`hide_overlay` commands, which are **not currently invoked in production** — the overlay is shown once at setup (`overlay_win.show()` in `lib.rs`) and stays visible. The visibility ref therefore defaults to `true` so first-hover works from mount, and the `disabled`-phase gate is the active battery saver today; the visibility gate is plumbing that activates if/when show/hide get wired to dynamic callers.
- The anchored overlay is not a native drag region. Relocation is explicit through
  Settings calibration, so an unpersisted free drag cannot be undone by the next
  native placement refresh. The dropdown is labeled as the `Quick settings` group
  and is `aria-hidden` until the expanded-frame acknowledgment reveals it.

### Motion tokens

The width/height/corner CSS easing is sampled from the same closed-form
`Spring` oscillator used by the dot field: 0.3s duration, 0.15 bounce. Field scale
is 0.9→1 and blur 3→0; opacity/blur use the zero-bounce 0.3s spring.
`OVERLAY_HEIGHT_MS` is the calculated settlement time, not the response duration;
`SHRINK_DELAY_MS` adds 20ms. Hover dwell is 150ms and leave intent is 300ms.
Reduce Motion snaps geometry/scale/blur and uses only a field opacity fade,
then sleeps even while recording. No traveling trace, processing sweep or Done
wipe runs in that mode.

## Frontend Hooks

`OverlayWidget.tsx` owns the shared `status` state (subscribing to `recording-status-changed` directly) plus `disabled`/`showHotkeyMiss`, and composes the following, in roughly this order (later hooks depend on earlier ones' output):

| Hook | Owns |
|------|------|
| `useOverlayGeometry` | Fetches/subscribes to `OverlayGeometry` (see [Geometry Contract](#geometry-contract)). |
| `useOverlaySettingsMirror` | The localStorage settings snapshot the overlay needs (`autoPaste`, `fileOutputEnabled`, `overlayVerticalOffset`, `overlayPlacement`, `overlayProcessingStyle`), `applySettingsSnapshot`/`refresh`, the `settings-changed` listener, and the three quick-control actions (toggle auto-paste with rollback-on-failure, toggle global disable, open Settings). |
| `useOverlayRuntime` | The `recording-cancelled` (red-X flash), `hotkey-tap-rejected` (amber flash), typed `recording-initialization-failed`/generic `recording-interrupted` microphone cues, `dictation-generation-started` ownership floor, generation-ordered `dictation-delivery-outcome` (clipboard-only `⌘V` cue), inline retry command and authoritative returned-result presentation, and `app-disabled-changed` listeners, plus the transient flash timers. `disabled`/`showHotkeyMiss`/`hotkeyMissFeedbackRef` are created in the composition shell (not inside this hook or the settings mirror) because both hooks write into them synchronously and neither can be constructed from the other's return value without an artificial call-order dependency; this hook attaches behavior and re-exposes them. |
| `useOverlayExpansion` (pre-existing, see [Expansion Controller](#expansion-controller)) | The hover-expand lifecycle. |
| `DotField` | Ref-only audio levels, bounded fixed-step simulation and visibility-gated canvas painting (see [Dot field](#dot-field)). |
| `useRecordingControls` | Click/double-click/mousedown disambiguation (250ms debounce) and "locked mode" (see [Click Interactions](#click-interactions)). Reads the microphone override via `loadSettings()` — no raw localStorage parsing. |

Pure, React-free logic lives alongside the presentational components in `app/src/components/overlay/`:

- **`deriveVisual.ts`** — runtime state → `OverlayVisual`. Encodes the top-bar indicator priority, including typed microphone failures, exactly once; locked by its exhaustive base-state matrix and focused priority tests in `deriveVisual.test.ts`.

Presentational components are driven by props:

- **`OverlayPill.tsx`** — the plate (priority status, decorative dot field or delivery action), including accessible transient cues. A terminal `device_unavailable` uses a mic-off glyph with actionable Settings guidance; other capture failures retain a generic warning.
- **`OverlayDropdown.tsx`** — the three quick-settings buttons (Power, auto-paste toggle, gear). Icons (`PowerIcon`, `ClipboardPasteIcon`, `SlidersIcon`) are colocated in this file rather than split one-per-file.

`OverlayIsland` owns the fixed-black surface and sizing. `OverlayWidget` wires its hover handlers and `islandRef`, then composes the pill and dropdown as siblings.

### Dropdown controls

| Control | Action |
|---------|--------|
| Power | Toggles global disable. Calls `set_app_disabled` directly for an immediate gate. When disabled: red icon (`#ef4444`) on a red-tinted background, auto-paste dims to 35%, top-bar mic fades to 15%. Global disable is also a "Disable Murmur" check item in the tray menu; the command keeps the tray check state in sync and the main window persists tray-driven changes. |
| Auto-paste toggle | Reads/writes the `autoPaste` setting via `loadSettings()`/`saveSettings()`. |
| Gear | Emits `open-settings` and shows/focuses the main window (`show_main_window`). |

The dropdown has a centered control row and a separate status row. The status
row shows the recording `m:ss` timer, connection/recovery text, or "Tap missed"
when applicable. These are visible only while expanded; the transient no-hover
cues remain on the pill.

### Cross-window settings sync

The overlay runs in a separate window with no shared React context. Writes go through `saveSettings()` plus an `emit('settings-changed')`; the main window listens and applies the change (`configure` for auto-paste, `set_app_disabled` for disable) with a diff-guard that prevents an echo loop. The main window also emits `settings-changed` on its own auto-paste/disable/calibration changes so an already-open overlay updates live. `useOverlaySettingsMirror` also re-applies the snapshot whenever the expansion controller's `phase` becomes `'opening'`, so the dropdown always shows current settings by the time it is revealed. The Settings calibration control separately emits `overlay-calibration-changed { active }` to switch the overlay into and out of its non-interactive calibration visual while the native window is previewed.

## Visual States

The overlay's top-bar indicator is a pure function of status + transient flags + global-disable (`deriveVisual()`); `status` itself is driven by the `recording-status-changed` event.

### Transform indicators (issue #312)

While a selected-text transform is in **Thinking** (sidecar running), the overlay can show a **transforming…** indicator (`deriveVisual` priority: cancelled > secure-field flash > hotkey-miss > recording > processing > transforming > idle). Secure/password fields refused by the transform capture path emit a content-free `transform-secure-field` flash via `useOverlayRuntime` — no selection text is ever read or shown.

### Idle
The notch rest shrinks to the notch and contains no animated plate. The below-menu-bar rest shows a small mic at 40% white opacity (15% when disabled). Neither keeps an idle canvas loop.

### Recording
The braided voice field occupies the plate below the notch (or the standalone
pill). The `m:ss` timer remains in the expanded dropdown. No transcript appears
inside the island; optional Live words use the separate click-through window.

### Starting and Recovering

Starting uses a blue pulse; after the backend's 5-second signal it becomes an
amber "Still connecting" cue. Clicking or toggling again cancels. Recovering
uses an amber spinner and dropdown label during cancellation recovery; the app
returns to Idle only after the exclusively owned Core Audio thread exits.
Terminal microphone failure gets a five-second, non-interactive red status cue
without opening controls or focusing the overlay; it retains the active plate size. `device_unavailable`
uses a mic-off glyph whose accessibility label directs the user to Settings to
choose another input; permission denial points to microphone settings, ordinary
capture failures recommend retry, and an interrupted partial capture says to
wait for transcription. A newer
recording generation clears the prior cue immediately so successful retries
cannot remain covered by stale failure feedback. A normal
recording stop whose teardown remains blocked surfaces persistent guidance in
the main recording-error area.

### Processing
A 76pt field runs the selected Sweep, Flow or Pulse style. Processing→idle holds Done for 400ms: dots clear from the center outward, then fade. A new recording cancels the hold. Priority refusal/failure/delivery cues hide the field immediately. No provisional words remain while processing.

### Clipboard Only (transient)

Whenever a dictation is confirmed on the clipboard without a completed
automatic paste—including disabled or file-output-suppressed auto-paste,
missing Accessibility permission, a safe focus refusal, or a paste failure—the
collapsed pill shows an accessible green `⌘V` cue for 5 seconds. A visible
**Try again** action in the right wing calls the existing secure Paste Last
command. Its mouse-down is cancelled before invocation so it does not focus or
activate Murmur, and the original auto-hide timer is cancelled while the retry
runs. Automatic paste shows **Pasted**, an empty delivery shows **Nothing**, and
clipboard-only, busy, or failed results keep **Try again** available for a fresh
bounded interval. A retry's clipboard-only result uses a neutral retry symbol,
because it can mean the pasteboard changed and does not prove the retained text
is still there. The cue never appears after a failed/unconfirmed clipboard write
or a completed automatic paste. Active
recording, processing, meeting, transform, and higher-priority failure states
remain truthful and take visual precedence; the cue appears once those states
return to idle. The action never focuses Murmur or expands controls; the cue retains the active plate. Monotonic recording IDs make
duplicate and stale delivery events unable to extend or resurrect the cue; a
separate generation-start event advances the stale-event floor before delivery.

### Cancelled (transient)
An 800ms red-X flash, triggered by `recording-cancelled`. Takes priority over every other indicator.

### Hotkey Timing Miss (optional, transient)
When `hotkeyMissFeedback` is enabled and the backend emits `hotkey-tap-rejected` for an expired second-tap window, the pill shows an amber outlined exclamation (`!`) in the left wing plus an amber border glow. The `Tap missed` text label renders in the dropdown row (below notch height, since text is wider than a wing) and is therefore visible while the card is expanded; the `!` and border glow are the always-visible transient cue. Takes priority over every indicator except cancelled. The setting is off by default.

The idle dropdown omits shortcut guidance. Recording, connection, recovery,
and hotkey-miss feedback use the separate status row.

**Styling:** The island is intentionally solid black in every theme, with rounded
bottom corners for notch mode and all corners for below-menu-bar mode. Live words
instead use semantic theme background, foreground and hairline tokens.

## Reload recovery

`useOverlayRecordingStatus` subscribes to `recording-status-changed` before
reading `get_status`. Reloading the overlay restores the current native state,
including an ongoing recording or transcription. A status event received during
the read takes precedence over the snapshot. The hook updates the click-handler
status ref synchronously and discards callbacks after unmount.

Reloading the webview does not stop native capture. To stop a recording, click
the recording indicator or use the recording shortcut.

## Dot field

`DotField` paints Canvas2D from reused typed arrays; no React state update,
native RPC, or explicit array/object allocation occurs per animation frame.
The grid is 43×9 at 2.6pt pitch with two opposed 1.6-turn traces and
center-weighted falloff. The 6.5Hz amplitude oscillator releases over 0.15s;
its history travels outward at 150pt/s with a 0.6s phosphor tail. Auto-level
uses a 40ms peak attack, room floor down/up 0.3/6s, 16dB minimum span and
−60dB noise floor. Simulation is fixed at 240Hz, capped at 12 catch-up steps;
painting follows display refresh. Audio older than 150ms is silence.

Idle/finished fields sleep. Document/native hiding cancels animation frames;
showing resumes with a fresh timestep rather than accumulating hidden work.
Deterministic synthetic fixtures exercise processing styles, Done, themes,
placement, priority cues and Reduce Motion. They do not establish native
performance, focus or real-dictation acceptance.

### Optional live words

App → General → **Live words** is off by default; the retired preview flag never
opts an install in. The 32pt pill is attached 6pt below the native frame, in a
560×44 click-through window. UI type is 13px with 14px horizontal padding.
Stable segments head-clip and tentative segments tail-ellipsis. Existing APIs
provide only whole provisional strings, so production renders tentative text
only: it does not infer committed words or invent cadence. See
[Live preview](transcription.md#live-preview) for the unchanged backend cadence.

The generation/status/cancellation listeners remain active even while disabled,
so re-enabling cannot resurrect a stopped generation. Stop, cancellation, newer
generation, setting-off and rejected native presentation clear the content.
No preview copy/paste/log/persist/export path exists. The native show gate
revalidates the exact generation on the main thread immediately before display.

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

The overlay seeds its presentation with validated cached settings, then reads the durable host snapshot on committed-setting notifications. Quick controls configure their owned backend field and call `patch_overlay_setting`: the host merges only Auto-paste or Disabled into the latest document under its writer lock. Main applies that one committed field to state/cache without a full-file echo. A stale webview snapshot cannot restore revoked Live Words consent or overwrite a newer processing style.

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

- Calls: `get_status`, `get_overlay_geometry`, `set_overlay_expanded`, `show_main_window`, `start_native_recording`, `stop_native_recording`, `retry_last_delivery`, `set_app_disabled`, `configure_dictation`.
- Listens: `overlay-geometry-changed`, `overlay-surface-changed`, `overlay-visible-changed`, `recording-status-changed`, `recording-cancelled`, `dictation-generation-started`, `dictation-delivery-outcome`, `hotkey-tap-rejected`, `app-disabled-changed`, `audio-level`, `settings-changed`.

`set_overlay_expanded` **returns the applied frame** as `AppliedSurface { windowW, windowH }`; the expansion controller awaits this value as the resize ack before revealing the dropdown. `show_overlay`/`hide_overlay` emit `overlay-visible-changed(true|false)`, which gates the controller's cursor poller so it does no IPC while the overlay is hidden.

## Transparent window caveat

The shared `styles.css` gives `body` an opaque surface background. The overlay's body carries the `overlay-window` class (`overlay.html`), which scopes it back to transparent — without it the whole overlay window frame paints as a dark box around the island.

The Appearance Theme Engine deliberately does not synchronize this window.
Overlay glass remains always dark, and its body remains transparent in System,
Light, and Dark modes. Appearance application must never set an `html` or
`body` background that can override this invariant.

The expanded Mode control shows an amber **Next: <Mode>** label while a one-recording
override is pending. The regular resolved Mode label returns after consumption
or explicit clearing, without changing the app binding. See [Modes](modes.md).
