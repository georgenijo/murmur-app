# Issue #38 — Overlay Widget: Session Notes

## Session 1 — Core implementation

### Core fix: Mouse events on non-focusable window
- `focusable: false` in tauri.conf.json causes macOS to set `ignoresMouseEvents = true`
- Fix: call `overlay.set_ignore_cursor_events(false)` from Rust after window creation
- Applied in: `setup()` closure, `show_overlay` command
- This re-enables mouse events while keeping the window non-activating (no focus steal)

### Dynamic Island-style notch integration
- **Notch detection**: Uses `objc2-app-kit` NSScreen APIs (`auxiliaryTopLeftArea`, `auxiliaryTopRightArea`, `safeAreaInsets`) to detect the exact notch width and height in logical points
- **Window level 25**: Set via `NSWindow.setLevel(25)` (mainMenu + 1) so the overlay renders above the menu bar and can overlap the notch area
- **Shadow disabled**: `NSWindow.setHasShadow(false)` removes the default macOS window shadow

### Config changes (tauri.conf.json)
- `macOSPrivateApi: true` — required for window transparency on macOS
- `visibleOnAllWorkspaces: true` — overlay follows across Spaces (3-finger swipe)
- Overlay auto-shows on app launch (in setup closure)
- Default overlay height 100px (Rust overrides to notch height at runtime)

---

## Session 2 — Horizontal Dynamic Island layout + bug fixes

### Design iteration (current state)
Through visual iteration with screenshots, the overlay evolved from a vertical-expanding notch extension to a **horizontal Dynamic Island** layout:

- **Idle state**: Pill extends slightly left of the physical notch (`notchWidth + 28`), shifted left via `marginLeft: 32`. Shows a small **mic icon** (12px SVG, 40% white opacity) to indicate the app is alive.
- **Active state**: Pill expands **rightward only** to `notchWidth + 68`. Left edge stays fixed. Left side shows a **fast-blinking red dot** (0.8s pulse) during recording or a **spinner** during processing. Right side shows **waveform bars** (7 bars, rAF-driven direct DOM updates).
- **Transition**: Spring animation `cubic-bezier(0.34, 1.56, 0.64, 1)` over 500ms.
- **Styling**: `rgba(20, 20, 20, 0.92)` background with `backdrop-filter: blur(40px)`. Border radius `0 0 12px 12px` (same for both states).

### Notch positioning (resolved)
- **Window at y=0**: Anchored at the very top of the screen, eliminating vertical coordinate guessing
- **Window height = notch height**: `overlay_h = notch_height` (~37pt), no vertical extension
- **Window width = notch + expansion room**: `NOTCH_EXPAND = 120.0` (60px each side)
- **CSS handles pill positioning**: `marginLeft: 32` positions the pill relative to the notch within the window
- No more "purple gap" — the pill's black background merges seamlessly with the physical notch

### Tray icon removed
- Removed the entire tray icon (no colored circle, no menu)
- No more amber/red/gray status indicator in the menu bar
- App accessible via dock icon and keyboard shortcuts

### Bug fixes (from CodeRabbit review)
- **MainThreadMarker UB fixed**: `detect_notch_info()` now called only once during `setup()` (guaranteed main thread). Result cached in `State.notch_info: Mutex<Option<(f64, f64)>>`. All callers (`get_notch_info` command, `show_overlay`, `position_overlay_default`) read from cache.
- **Monitor mismatch fixed**: `position_overlay_default()` uses `current_monitor()` instead of `primary_monitor()`
- **Stale comments fixed**: audio.rs "~30 fps" updated to "~60 fps" (matches 16ms constant)
- **Waveform perf fixed**: Replaced 60fps React state updates (`setAudioLevel` + `setBarHeights`) with `audioLevelRef` + `barRefs` + `requestAnimationFrame` loop for direct DOM updates. Zero React reconciliation during recording.
- **Position persistence disabled**: Both save (onMoved) and restore removed while notch positioning is being iterated. TODO tracks re-enabling.
- **Error logging**: `set_size` and `set_position` failures now logged with context instead of silently swallowed.
- **Unused state**: `_notchHeight` state converted to `notchHeightRef` ref.

### Position persistence (deliberately disabled)
- Both save (onMoved) and restore are disabled
- Will re-enable with user-drag filtering once notch alignment is finalized
- TODO at line 62-63 in OverlayWidget.tsx tracks this

---

## Known bugs (discovered at end of session 2)

### Bug 1: Overlay click activates app / brings main window to front
- **Symptom**: Clicking the overlay briefly morphs it (recording starts), but then the main window appears and the overlay resets to idle state and stops responding to clicks
- **Likely cause**: Clicking the overlay activates the macOS application, which may trigger the main window to show. The `focusable: false` prevents the overlay from becoming key window, but the app still gets activated.
- **Investigation needed**: Check if macOS `applicationShouldHandleReopen:hasVisibleWindows:` is firing and showing the main window. May need to handle the reopen event in Tauri to prevent this.
- **Related**: We removed the tray icon — the dock icon click handler may also be relevant.

### Bug 2: Transcription not showing in main window UI when initiated from overlay
- **Symptom**: When recording is started from the overlay (double-click) and stopped, the transcribed text may not appear in the main window's history list
- **Investigation needed**: Check if the transcription result event reaches the main window. The recording/transcription pipeline should be the same regardless of where it's initiated — both call `start_native_recording`/`stop_native_recording`. Check the main window's event listeners.

---

## Key files

| File | What it does |
|------|-------------|
| `app/src-tauri/src/lib.rs` | `detect_notch_info()`, `raise_window_above_menubar()`, `position_overlay_default()`, `show_overlay`, notch caching in `State` |
| `app/src/components/OverlayWidget.tsx` | Dynamic Island UI, rAF waveform animation, click/drag handlers, `get_notch_info` call |
| `app/src-tauri/src/audio.rs` | Audio level throttle (16ms / ~60fps) |
| `app/src-tauri/tauri.conf.json` | Overlay window config, macOSPrivateApi, visibleOnAllWorkspaces |
| `app/src-tauri/capabilities/overlay.json` | IPC permissions for the overlay window |
| `app/src-tauri/Cargo.toml` | objc2, objc2-app-kit, objc2-foundation dependencies |

## Key technical notes

- `NSWindow.setLevel(25)` = mainMenu + 1. This is what boring.notch and mew-notch use to render above the menu bar.
- The notch width on a 14" MacBook Pro is ~185 logical points.
- `safeAreaInsets.top` gives the notch/menu bar height (~37pt on notched Macs).
- `auxiliaryTopLeftArea` and `auxiliaryTopRightArea` are the menu bar regions on either side of the notch.
- The `focusable: false` + `set_ignore_cursor_events(false)` combo gives us mouse events without focus stealing — the two NSWindow properties (`canBecomeKey` and `ignoresMouseEvents`) are independent.
- Tauri 2 custom commands (`#[tauri::command]`) are available to all windows by default, but core/plugin commands require explicit capability permissions (see `capabilities/overlay.json`).
- Notch info is cached at setup time (main thread safe) — never call `detect_notch_info()` from command threads.

## PR
https://github.com/georgenijo/murmur-app/pull/63
