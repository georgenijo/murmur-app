# Issue #38 — Overlay Widget: Session Notes

## What was done

### Core fix: Mouse events on non-focusable window
- `focusable: false` in tauri.conf.json causes macOS to set `ignoresMouseEvents = true`
- Fix: call `overlay.set_ignore_cursor_events(false)` from Rust after window creation
- Applied in 3 places: `setup()` closure, `show_overlay` command, `toggle_overlay` tray handler
- This re-enables mouse events while keeping the window non-activating (no focus steal)

### Dynamic Island-style notch integration
- **Notch detection**: Uses `objc2-app-kit` NSScreen APIs (`auxiliaryTopLeftArea`, `auxiliaryTopRightArea`, `safeAreaInsets`) to detect the exact notch width and height in logical points
- **Window level 25**: Set via `NSWindow.setLevel(25)` (mainMenu + 1) so the overlay renders above the menu bar and can overlap the notch area
- **Window auto-sizing**: `set_size(notch_width + 10, 52)` to match the notch with a 5px border on each side
- **Positioning**: `y = notch_height - 8` (overlaps 8px behind the notch, rest protrudes below)

### Overlay capabilities
- Created `ui/src-tauri/capabilities/overlay.json` with permissions for event listening, dragging, positioning
- The overlay window previously had zero IPC permissions

### Frontend (OverlayWidget.tsx)
- **`data-tauri-drag-region`** on outer container for native dragging
- **Dynamic Island animation**: idle = 4px black nub (hidden under notch), active = expands to 44px with spring animation
- **Pure black background** with `border-radius: 0 0 14px 14px` (flat top merges with notch, rounded bottom)
- **Waveform**: 7 bars, 16x audio amplification, center-peaked EQ envelope, ~60fps updates (throttle reduced from 33ms to 16ms in audio.rs)

### Config changes (tauri.conf.json)
- `macOSPrivateApi: true` — required for window transparency on macOS
- `visibleOnAllWorkspaces: true` — overlay follows across Spaces (3-finger swipe)
- Overlay auto-shows on app launch (in setup closure)

## What still needs work

### Notch positioning (main remaining issue)
- The widget is close but not pixel-perfect aligned with the physical notch
- The 8px overlap creates a visible seam in some cases
- Attempted a full-window approach (y=0, content pinned to bottom) but it regressed — reverted
- **Next approach to try**: Study boring.notch's full-screen transparent panel approach — create a large transparent window covering the full screen, use CSS to position the notch shape within it. This avoids coordinate math entirely (SwiftUI spacers center it automatically; we'd use CSS flexbox equivalently)
- Reference repos: [boring.notch](https://github.com/TheBoredTeam/boring.notch), [mew-notch](https://github.com/monuk7735/mew-notch), [DynamicNotchKit](https://github.com/MrKai77/DynamicNotchKit)

### Position persistence (temporarily disabled)
- localStorage save/restore of overlay position is commented out (`// TODO: re-enable after notch positioning is stable`)
- The `onMoved` debounced listener for saving position is still active but the restore on mount is disabled
- Re-enable once notch positioning is finalized

### Dragging
- `data-tauri-drag-region` is in place but dragging conflicts with notch-locked positioning
- Need to decide: allow free dragging (loses notch alignment) or lock to notch position only

### Click/double-click interaction
- Handlers exist (single click = stop recording in locked mode, double click = toggle locked mode)
- These work when cursor events are enabled but haven't been extensively tested with the notch positioning

## Key files

| File | What it does |
|------|-------------|
| `ui/src-tauri/src/lib.rs` | `detect_notch_info()`, `raise_window_above_menubar()`, `position_overlay_default()`, `show_overlay`, tray toggle |
| `ui/src/components/OverlayWidget.tsx` | Dynamic Island UI, waveform animation, click/drag handlers, position persistence |
| `ui/src-tauri/src/audio.rs` | Audio level throttle (16ms / ~60fps) |
| `ui/src-tauri/tauri.conf.json` | Overlay window config, macOSPrivateApi, visibleOnAllWorkspaces |
| `ui/src-tauri/capabilities/overlay.json` | IPC permissions for the overlay window |
| `ui/src-tauri/Cargo.toml` | objc2, objc2-app-kit, objc2-foundation dependencies |

## Key technical notes

- `NSWindow.setLevel(25)` = mainMenu + 1. This is what boring.notch and mew-notch use to render above the menu bar.
- The notch width on a 14" MacBook Pro is ~185 logical points.
- `safeAreaInsets.top` gives the notch/menu bar height (~37pt on notched Macs).
- `auxiliaryTopLeftArea` and `auxiliaryTopRightArea` are the menu bar regions on either side of the notch.
- The `focusable: false` + `set_ignore_cursor_events(false)` combo gives us mouse events without focus stealing — the two NSWindow properties (`canBecomeKey` and `ignoresMouseEvents`) are independent.

## PR
https://github.com/georgenijo/murmur-app/pull/63
