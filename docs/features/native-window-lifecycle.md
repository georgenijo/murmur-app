# Workspace window lifecycle

Main and Diagnostics default to 1100×780 logical pixels, with an 840×620
minimum. Playwright measures the real Settings layout at that minimum in both
schemes: content scrolls vertically, while pane controls stay within the window.
The separate minimum-size goldens are Darwin browser evidence, not native or
Linux evidence.

`--ui-window-header-height` remains the sole header-height source. The renderer
measures it through layout, including rem/calc values, and sends the result to
Rust. Rust places native traffic lights at x=16 points and
`round(headerHeight × pageZoom / 2 − 7)` points from the top. AppKit changes run
on the main thread. Cmd/Ctrl + `+` or `=` increases page zoom by 10%, `-`
decreases it, and `0` resets it; the supported range is 50–200%. Display backing
scale is independent from page zoom. The lifecycle bridge also publishes
`--ui-window-page-zoom`; `lib/windowLifecycle.css` divides the existing
`--ui-window-controls-inset` token by that zoom. Thus the reserved native-control
width stays 80 window points even at 50% page zoom, while the CSS token remains
the sole inset measurement. Disposal restores the previous root property.

Workspace windows start hidden with an opaque configuration fallback. The last
resolved native background is restored before reveal; the current resolved CSS
background replaces it before main is shown. Main reveals after a committed
React tree and two animation frames. An independent five-second Rust timeout
reveals main if renderer initialization fails. Theme, scheme, and zoom changes
synchronize native chrome without reopening a closed window. Diagnostics uses
its own measurements and zoom, follows shared appearance read-only, and stays
hidden until requested. Transparent overlays and popovers retain their own
geometry and visibility.

Rust writes `window-state.json` in the bundle's application-data directory,
500 ms after the last geometry/background event and again at exit. It stores
whole physical-pixel outer bounds, inner size, their backing scale, the last
ordinary bounds while maximized, the maximized flag, and resolved backgrounds.
Restore normalizes saved bounds using their saved scale and each connected
display using its own scale before checking containment in logical desktop
coordinates. Older state without a saved scale uses the candidate display's
scale. The entire outer rectangle must fit one connected display and the inner
size must meet the 840×620 logical minimum. Restored geometry rounds to whole
pixels at the target display's scale and is passed to Tao as logical position
and size. This avoids Tao converting physical arguments with the launch
window's potentially different scale. Changing the target display's backing
scale between launches preserves the logical geometry. Missing, invalid,
undersized, or off-screen state falls back to the default size centered on the current display. Fullscreen and
minimized geometry do not replace normal bounds.

## Lead's native acceptance checklist

Build an isolated bundle on the Mac mini with identifier
`com.localdictation.epic846.window854` (Tauri's `--config` accepts a JSON object
with this `identifier` and a distinct `productName`). Do not launch production
Murmur or use the MacBook. Confirm the running bundle contains the tested
revision. Native interaction and independent Sol review belong to the lead.

- Record cold launches in explicit Light and Dark, then System and a custom
  imported theme. Verify the first visible background matches the webview,
  and changing theme/scheme updates the native background without flashing.
- Check main Settings and pop-out Diagnostics traffic lights at 100%, 50%
  (Cmd + `-` five times from reset), 120% (reset, then Cmd + `+` twice), and
  reset (Cmd + `0`); verify the first header content stays clear of all three
  buttons at 50% and the reserved native-control width remains 80 points. The
  native x stays 16 points and y follows the actual CSS header height. Resize, maximize/unmaximize, enter/exit fullscreen, and
  check that buttons stay clickable and centered.
- Move and resize both workspaces, wait at least 500 ms, quit/relaunch, and
  verify normal bounds and maximized state separately. Diagnostics must not
  open automatically at launch. Close main; theme changes must not reopen it.
  With lead-managed mixed-scale displays available, save normal and maximized
  bounds on the 1× display and relaunch from the 2× display, then reverse the
  direction. Verify the same logical inner size and outer origin, including a
  display left of or above the primary. Changing a target display's scale must
  retain logical bounds, rounded to whole pixels at its new scale. Do not alter
  the shared display configuration or use the MacBook for this check.
- At 840×620, inspect General, Recording, Delivery, Meetings, Appearance,
  Customize/editor pages, all three AI detail pages, and Diagnostics. Verify
  controls fit horizontally and all vertical content remains scrollable.
- For off-screen fallback, quit the isolated app, back up
  `/Users/george-mac-mini/Library/Application Support/com.localdictation.epic846.window854/window-state.json`,
  set `windows.main.bounds.x` and `.y` to `999999`, then relaunch. Expect
  1100×780 centered on a connected display. Restore/delete only this isolated
  test file afterward. Repeat with `windows.diagnostics`; an actual disconnected
  monitor is an additional native case.
- Exercise the safety timeout with a deliberately broken renderer in an
  isolated test build; main must appear within five seconds of Rust setup.

Native recordings, exact-revision native Retina proof, and Linux browser proof
remain release gates for the lead. The lead has established a 2× virtual display
on the Mac mini (1920×1080 logical, 3840×2160 physical); this worker did not alter
it or exercise the native build. The mini's checked Docker context points at its
local OrbStack socket, but that daemon was unavailable; no Linux screenshots were
produced. Existing visual CI remains macOS. No Retina waiver was granted. Do not
operate the MacBook to satisfy these gates. Home sidebar collapse/resize remains a lead-owned follow-up and is not
implemented in this change.

## S2/S3 regression evidence

Pure Rust lifecycle tests simulate both launch/target scale directions, a
backing-scale change between launches, negative monitor origins, disconnected
and spanning windows, target-pixel rounding, legacy state, invalid scales,
minimum inner size, and maximized-state retention. Headless Playwright tests in
`window-lifecycle-zoom.spec.ts` render the real Settings and Diagnostics headers
in both schemes, drive the production bridge to 50%, check actual content and
inset layout in CSS coordinates converted to window points, change the CSS
inset token, and check reset/120%. The 50% case doubles the CSS viewport to
represent the native window's fixed point size; it does not operate a native
webview or establish native traffic-light clickability. Existing goldens are
unchanged. The lead's native delta is the 50% header flow and mixed-scale restore
cases above; review recheck is limited to S2/S3 behavior, with no approval claim.
