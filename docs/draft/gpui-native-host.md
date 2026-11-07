# GPUI native host (#996)

Layer 19 provides the native host shell. Layer 20 extracts service ownership;
layer 21 supplies native OS services and composes the production views. The
shipping Tauri host remains in place until Phase 4. Each layer is independently
buildable and remains a draft. Packaging belongs to parallel layer 18 (#994).
The final executable is named `ui`, matching the existing updater contract.

The shell accepts explicit production composition, a #992 lifecycle, and a
terminal receiver. It never imports a fixture. It creates persistent GPUI Main
and Diagnostics windows, GPUI App/Edit/Dock menus and an objc2-owned NSStatusItem.
Closing a persistent window hides it. Native next-recording commands carry the
menu revision and must still pass the service owner's revision check before
admission. A left tray click opens Main; a right click opens its menu.

The pinned GPUI delegate has applicationWillTerminate but no
applicationShouldTerminate. The host adds only the missing selector to that
existing delegate, rejecting an existing implementation. It never swizzles or
replaces the delegate. AppKit's NSTerminateLater keeps main-thread dispatch
running while the #992 reducer drains owners on background workers. Only its
committed terminal action may reply or ask GPUI to quit. The reply is queued
outside the GPUI AppCell borrow. GPUI's 200 ms shutdown window performs final
cleanup only. Owner failures retain #992's restart suppression.

`DataPaths` computes the existing Tauri app-data roots without touching disk.
Legacy model/storage paths preserve the production root only for the explicit
production identifier; other identifiers use the same UTF-8-hex isolation as
the shipping host. Tests use `/synthetic-home` and never open these directories.
`worker::dispatch` short-circuits diarization, Core ML installer, capture-agent
and capture-helper handling before the executable constructs GPUI or Metal.

## Native acceptance handoff

Use an isolated dev or bench identity and synthetic data. No synthetic input is
used by the implementation worker. Interactive acceptance belongs to the native
GUI QA worker, after the final service composition is built.

| Matrix row | Concrete flow and required observation |
|---|---|
| P01, D11, K12, K13 | Left tray click shows Main; right click opens menu. Disable/re-enable changes the real admission owner. Paste Last before a result reports Nothing; after a synthetic result retries once. Cycle Mode, select Next recording, clear it, then reopen menus after changes; stale menu actions cannot alter the new revision. |
| P02, U03 | Open App, Edit and Dock menus. Settings and About route to native views; Cut/Copy/Paste/Undo/Redo/Select All target the focused synthetic field; app shortcuts preserve ordinary editing. Open an allowed external link through the native opener. |
| P03, U01 | Resize/move Main and Diagnostics, enter/exit fullscreen, close each, reopen from Dock and tray. Windows hide and preserve state; Reopen respects visible windows and does not steal activation from overlay interaction. Relaunch restores geometry. |
| P04 | Exercise menu Quit, Dock Quit, exact-PID synthetic quit Apple event and terminate/relaunch action with idle, dictation, meeting and pending-write fixtures. Observe original-generation drain, exact helper death, durable write receipts and no replacement PID after failed drain. GPUI quit callbacks must occur after the terminal permit. |
| V05 (host portion) | Compare production/dev/bench path calculations by unit test only. Run each worker mode in an isolated CLI process and prove no NSApplication/Metal initialization. Signed identity, package layout, TCC carryover and installed update/downgrade rehearsal belong to the coordinated final-stack acceptance with #994. |

Layer 21 extends this checklist with the remaining Phase 2 rows; this table is
not a claim that production service wiring or native acceptance has passed.
