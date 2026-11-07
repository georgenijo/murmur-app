# GPUI native host services (#996, layer 21)

The production-only `production-host` feature adds native OS capabilities and
shared-owner admission seams. Gallery/QA entry points keep their injected
synthetic hosts. Layer 22 composes these capabilities with the production
sections and introduces the `ui` executable; this layer does not switch the
shipping Tauri host.

## Native boundaries

- Dialogs use asynchronous NSOpenPanel/NSSavePanel and sheet-based NSAlert.
  The main-thread bridge checks a revocable lease immediately before presenting
  and again on completion. Cancel returns no path and cannot cause a write.
- NSWorkspace opens explicit paths and approved URL schemes. The caller retains
  its lease until the queued main-thread effect. No shell string is evaluated.
- UNUserNotificationCenter receives version-only update notices. Its delegate
  routes clicks into the native update flow. An unbundled executable cannot
  register a notification identity or a login item.
- SMAppService replaces the verified Tauri LaunchAgent. The migration state
  machine requires both a saved enabled preference and an enabled old authority.
  Persist the migration intent, disable the exact old launch job, verify that
  disabled state, remove only the unchanged verified plist, then register the
  main application. No bootout/signal operation can terminate an active app.
  Explicit off wins at every checkpoint. A synchronous intent epoch revokes
  queued main-thread registration before an off future is polled; cloned
  controllers share one serialized operation gate. Approval never triggers repeated
  registration. A crash after the automatic registration attempt cannot override
  a later OS off choice; ambiguous outcomes require an explicit user retry.
- Relaunch is a committed #992 terminal effect. The native shell waits for a
  bounded launcher-ready handshake without blocking its main thread. A kernel
  pipe retained for the parent process lifetime delays the macOS LaunchServices
  opener until parent exit; no PID polling or new-version worker protocol is
  needed. The system helper receives the bundle as a positional argument to a
  constant script. External URLs and paths use NSWorkspace directly.
- Shared service requests run on the installed Tokio runtime. Layer 22 supplies
  per-section read projections and bounded named-consumer subscriptions.
- Window ports carry registration generations. Only main-thread code accesses
  AppKit's weak window registry; workers receive content/frame snapshots and all
  display facts. Rust computes geometry and queues exact-generation mutations.
- The production clipboard provider replaces every GPUI clipboard access,
  including async reads. A dev identity never falls back to the general board.
  This interception is opt-in per application and does not change gallery hosts.

Native preference admission is synchronous. The existing source FIFO retains
accepted jobs and #992 writer permits even when a view drops its receiver.
Field intents merge into the current document under the shared source lock;
configuration/shortcut admission precedes publication. Invalid normalized edits
are rejected. Native source compare/save and History final effects execute in
one completion-owner turn, closing the read-then-Clear race. History uses a
separate content epoch so unrelated Snapshot reads cannot invalidate a current
copy/export intent; even an empty Clear revokes its previous epoch. Long exports must
prepare bytes first and admit only their bounded final publication step.

## Verification

Use Rust 1.98.1 and the isolated task checkout/target on the Mac mini. Build the
capture helpers first. Run GPUI tests and strict all-target Clippy with
`gallery,production-host`, plus shipping Rust checks/tests and the focused core
source/Clear tests. Migration state-machine tests use observations only; path
contracts compute synthetic paths only. No production user directory is opened.

Self-checks do not register OS services, request TCC, use native input or access
the real pasteboard. The GUI QA worker separately exercises the final composition
using a dev or bench bundle, then unregisters test services and verifies cleanup
from a fresh process.

## Native acceptance handoff

| Matrix rows | Concrete final-stack flow |
|---|---|
| D09, D11, H04 | Dictate synthetic text with auto-paste off, copy a History row, and retry delivery from tray/overlay. Inspect only the test pasteboard; verify no duplicate activity/completion and refusal after Clear or generation change. |
| D12, H06, F01, F02 | Choose/cancel a transcript output folder and History save panel; import synthetic audio/video through a multi-select picker and native file drop. Preserve ordered queue/cancel behavior and write nothing on cancellation. |
| D13 | Enable NotchPill mirroring explicitly, test installed/missing adapter states, then disable. Use only the isolated test identity and synthetic final text. |
| O01, O09 | Inspect non-activating overlay and Live Words windows on two displays. Change placement/scale/Dock exposure while recording; keep preview consent and generation fences, click-through behavior and native Rust geometry. |
| S04, N02 | Follow default input, select a stable device, remove/reconnect it, and return from permission settings. Deny/request/reset each permission explicitly. Mounting a section must not prompt or open a capture device. |
| U01, U03, P02, P03 | Visit every section, use native menus and shortcuts with an editable field focused, hide/reopen, resize/move/fullscreen and relaunch. Preserve focus, editing, geometry and native appearance. Open an allowed link through NSWorkspace. |
| M09, M10, M11, M12 | Copy/save each meeting format, captions and a retained-audio bundle; cancel each panel. Request Calendar naming explicitly, review/select/apply, then change an event before apply and verify refusal. |
| K08, K14, A04, G03 | Open bounded synthetic knowledge, spelling, Mode, theme and portable-report files. Review before confirming; change the source/store between preview and commit and verify conflict. Export selected data through save panels. |
| K12, K13, P01 | Cycle Modes from tray, choose/clear Next recording, remove a mode before a stale menu click, and retry delivery. Only the next accepted recording consumes its own pending token. |
| A02 | Select system/light/dark/custom appearance; observe native chrome and all sections, then change OS appearance. OS observation must not publish a source write. |
| P04 | Quit/relaunch through each menu/process/update/system path while capture, meetings, helpers and source writes are active. The shared #992 owner must grant termination before GPUI quit callbacks; failed drains suppress relaunch. |
| P05, V01 | Trigger a due-gated hidden-window update check using the approved test fixture. Observe one native notification; click it to open update details. Denied notifications remain quiet. No transcript or selection content enters the notice. |
| P06 | Test legacy-on, legacy-off, explicit new enable/disable, approval pending and each crash checkpoint in isolated bundle identities. Verify there is never more than one login launch authority, and unregister/verify from a fresh process. |
| V02, V03 | Exercise optional download, Ready/Later, required update, install failure and restart retry with signed synthetic fixture releases. Preserve verification/rollback and use the shared coordinator for every restart. |
| V05 | Compare production/dev/bench paths by calculation-only tests. Confirm each CLI worker creates no NSApplication/Metal state. Packaging, signing/TCC continuity and the installed update/downgrade chain remain the coordinated final-stack acceptance with #994. |

This checklist specifies acceptance flows. It does not claim that compiling or
passing windowless tests establishes interactive acceptance.
