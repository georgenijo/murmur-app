# Layer 24 packaged production QA

This draft packages the integrated production app as `ui`. It is based on
`gpui/23-startup-integration` at `1db65bfa`. The final executable changes were
built at `b2a41b37`; later evidence/docs commits do not change executable behavior.
The checkout and paired Cargo target were reused with Rust 1.98.1.

The tests used `com.localdictation.gpui-host-qa110`, ad-hoc signatures and synthetic
HOME directories. The production-only build used `production-host`; the legacy
and loopback-updater fixtures used `native-host-qa` through that same `ui` main.
[Production binary receipt](qa/gpui24/production-build.json) and
[fixture build receipt](qa/gpui24/build.json) record hashes. No production signing
or release workflow was used. VoiceOver and system settings were left unchanged.
Only the named test pasteboard was eligible for app editing/delivery.

## Completed native subflows

These PASS results apply to the stated subflows. They do not complete a matrix
row whose other required subcases remain blocked in the table below.

| Subflow | Result | Evidence |
|---|---|---|
| Production entry and version | PASS | The packaged `ui` opens setup and production sections, not the removed shell. About shows 0.62.1 from the shared config. `--version` succeeds with a nonexistent HOME and creates no profile. [About](qa/gpui24/production-about.png), [CLI receipts](qa/gpui24/cli.json). |
| Passive production sections | PASS | Native AX clicks visited Home, Meetings, Insights, Models, Knowledge, Modes, Customize, Appearance and Settings. No permission request was made. [Navigation receipts](qa/gpui24/navigation.json). |
| Missing model/setup recovery | PASS | Fresh and incomplete profiles route to setup with no installed model. The production build presents the denied microphone state without requesting a grant. [Permission screen](qa/gpui24/production-permissions.png). Completion with a real installed model remains blocked. |
| Native AX roles/actions | PASS | Named controls, tabs, progress, headings and palette options are exposed. AX Pick on palette results opens About and Diagnostics; closing About restores focus to its invoker. The final heading is `AXHeading`. [Final AX tree](qa/gpui24/native-ax.txt), [Diagnostics](qa/gpui24/production-diagnostics.png). Spoken VoiceOver announcements were not certified. |
| Appearance editing/persistence | PASS | Light and Dark apply; a synthetic custom palette saves and survives a fresh process. An editor opened against a changing appearance revision refuses the stale save. [Persisted appearance](qa/gpui24/production-appearance.png). |
| Native panel cancellation | PASS | Folder selection and theme-file selection were opened and cancelled through native AX. No file was chosen or exported. Attempts to place the picker at the isolated fixture directory did not establish safe file selection. [Panel receipt](qa/gpui24/panel-scope.json). |
| Main hide/reopen | PASS | AX Close leaves the same test PID alive with zero visible windows. Opening the exact test bundle restores the same native window/PID. No other bundle was targeted. |
| Fullscreen and compact relaunch | PASS | Entering fullscreen produced a 1920×1080 native window. A separate compact 760×632 frame at (200,150), with 760×600 content, persisted and restored exactly after an orderly SIGTERM/fresh launch. [Restored window](qa/gpui24/compact-restored.png), [before](qa/gpui24/geometry-before.json), [after](qa/gpui24/geometry-restored.json). Fullscreen shortcut/exit and display-topology changes remain unverified. |
| Optional update and Later | PASS | A loopback feed signed with a disposable synthetic Ed25519 key reaches Ready. Close/reopen preserves Ready. [Ready](qa/gpui24/updater-ready.png). |
| Update payload/signature failures | PASS | Validly signed malformed bundle metadata is rejected before replacing the running bundle; retry returns to discovery. A bad signature is rejected. [Metadata failure](qa/gpui24/updater-invalid.png), [signature failure](qa/gpui24/updater-signature.png). |
| Required update and native Quit | PASS | Required policy has Download and Quit with no Close/Later. A failed download retains Quit/retry. The native Quit button exits the exact process through the service coordinator. [Required offer](qa/gpui24/updater-required.png). |
| Legacy import/fresh processes | PASS | Two contract launches reach startup stage 110, all passive projections valid, one imported row and migrated setting acknowledged. [Receipts](qa/gpui24/migration.json). No transcript contents are retained in this report. |
| Interrupted legacy child/retry | PASS | The exact owned legacy child was interrupted before acknowledgement. Startup stopped at stage 20; a new process recovered, and retry plus repeat each reported the one-time import contract true. [Receipts](qa/gpui24/migration-interrupt.json). |
| Worker admission/identity | PASS | Invalid diarization/Core ML/helper requests exit 64; the non-production capture-agent request is refused with 77 before service access. Each CLI HOME remains absent. The dispatch-order tests and calculation-only native path contract pass. [CLI receipts](qa/gpui24/cli.json). Successful permission/model-dependent worker execution was not certified. |

## Complete Phase 2 row disposition

The current adapters checklist contains **35 distinct IDs**, rather than the 34
named in the handoff. Every ID is included here. **BLOCKED means the full row is
not accepted**, including where permitted subflows above passed. No blocked row
is replaced by automated-test evidence or an AX event merely being dispatched.

Keyboard events from Computer Use and Ghosthands produced no observable edit or
shortcut effect, including Go to Folder in an AppKit panel. AX value entry,
focus, buttons and palette Pick actions worked. Japanese IME composition is also
blocked because the enabled input-source list contains only the U.S. layout.
[Editing attempts](qa/gpui24/edit-qa.json) preserve failed read-back assertions
without field or clipboard contents; these are not counted as product passes.

| Row | Result | Evidence / remaining constraint |
|---|---|---|
| D09 | BLOCKED | Microphone denied; no recording/model fixture. External injection is intentionally refused for this isolated identity. Clipboard-provider/identity tests pass. |
| D11 | BLOCKED | No accepted synthetic recording/final delivery to retry; overlay/tray/shortcut retry cannot be certified. |
| H04 | BLOCKED | No interactive retained-history copy/expand/key flow; key delivery was not established. Private-board unit coverage passes. |
| D12 | BLOCKED | Folder-panel cancellation passed; synthetic capture, sequential transcript/audio output and nonfatal write-failure flow require recording permission/model. |
| H06 | BLOCKED | Complete filtered History and pending-export/Clear require a populated native History fixture and reliable isolated panel selection. |
| D13 | BLOCKED | No isolated NotchPill install/IPC fixture. The real external integration was not enabled. |
| O01 | BLOCKED | Active overlay calibration/placement and two-display capture require permission/model and a second display. |
| O09 | BLOCKED | Live Words provisional/revoked-consent behavior requires active synthetic capture. |
| F01 | BLOCKED | Safe isolated multi-file selection was not established through the native picker. No real files were imported. |
| F02 | BLOCKED | Ordered transcription/drop/cancel requires synthetic media plus an installed model; no file drop was certified. |
| S04 | BLOCKED | Passive Recording settings visited, System default shown with stale-inventory status. Device removal/default switching and frozen recording configuration require permission and a controllable input fixture. |
| N02 | BLOCKED | Denied microphone state observed. Grant/reset/return, System Audio, AX and Calendar permission matrices would require interactive grants or system changes outside this constrained pass. |
| U01 | BLOCKED | All production sections, compact layout and native chrome exercised; Home start/stop and editor destinations tied to capture remain blocked. |
| U03 | BLOCKED | AX text entry/focus and palette focus return pass. Native Edit operations, actual shortcut delivery, selection/undo and Japanese IME remain blocked as described above. |
| M09 | BLOCKED | No retained synthetic meeting created: recording/System Audio permissions unavailable. Label/metadata/playback/copy flow not accepted. |
| M10 | BLOCKED | Meeting text/JSON export and stale-selection cancellation lack a retained meeting and reliable isolated picker navigation. |
| M11 | BLOCKED | Caption/audio ZIP export and retained-audio playback lack a synthetic retained meeting/audio fixture. |
| K08 | BLOCKED | Knowledge editor open/cancel and AX text entry pass. File import/export, malformed/oversized imports and source/file revision races require reliable isolated picker selection. |
| K14 | BLOCKED | Spelling/Mode/theme surfaces visited; full file import/export and bulk/recovery conflict flows remain blocked by picker/input limitations. |
| A04 | BLOCKED | Custom palette compilation/save, stale-editor refusal and persistence pass. Local-file/community import/export and malformed/oversized file matrix remain blocked. |
| K12 | BLOCKED | Mode library and next-recording palette options exposed; complete tray cycling/stale-menu and active recording snapshot flow not certified. |
| K13 | BLOCKED | Select/clear/consume pending token and stale mode removal require the full tray and accepted recording flow. |
| P01 | BLOCKED | Scoped app AX tools did not provide a complete tray left/right-click/menu proof. No ambiguous status item or production bundle was driven. |
| A02 | BLOCKED | System/Light/Dark controls, custom palette, stale source refusal and relaunch persistence pass. Changing the OS appearance and observing it across all sections was not performed. |
| P02 | BLOCKED | App/Edit/Services menu structure, native About, native Settings, palette actions and panel cancellation observed. Edit commands and external URL effects were not certified; no Apple event was sent to another app. |
| P03 | BLOCKED | Close/hide/reopen, resize/move, fullscreen entry and compact relaunch pass. Dock/tray reopen, fullscreen exit shortcut, zoom/IME/hit testing and changed display topology remain unverified. |
| P04 | BLOCKED | Exact-PID SIGTERM exits 0; required-update Quit and contract exits pass. Menu/key/Dock/tray, active-capture/meeting/pending-write delay, failed-drain and updater replacement/relaunch paths are not fully certified. |
| P05 | BLOCKED | Incomplete setup remains passive and manual fixture checks work. Allowed/denied native notification delivery and clicks require notification permission/setup completion. |
| V01 | BLOCKED | Manual synthetic updates share sidebar/Settings state; incomplete setup suppresses automatic work. Completed-setup launch/focus/wake six-hour coalescing not exercised interactively. |
| P06 | BLOCKED | Launch at login remains off across isolated launches. The unknown QA identity does not retire a legacy authority. Registration/approval/crash/migration matrix requires a separately controlled launch-authority fixture. |
| V02 | BLOCKED | Signed Ready/Later, malformed payload refusal and stage recovery pass. Successful version-matching replacement and restart/restart-failure chain were not performed. |
| V03 | BLOCKED | Required non-dismissible policy, failed download, bad signature and coordinator Quit pass. Absent/malformed policy and actual translocation/reinstall guidance were not fully exercised natively. |
| V04 | BLOCKED | Synthetic notes are displayed. Multi-version installed transition, matching-version What's New, quit-before-dismiss and one-time focus handoff need the version-matching replacement chain. |
| V05 | BLOCKED | Ad-hoc layout/signature/link checks, metadata version, identity/path tests, worker refusal and synthetic migration pass. Signed Tauri→GPUI→GPUI/TCC continuity and supported downgrade require a separately authorized signed installation rehearsal. |
| G03 | BLOCKED | Native Compare view opens. Importing two compatible plus invalid/oversized/incompatible reports is blocked by reliable isolated picker selection. No report execution was started. |

## Additional checklist rows and carried acceptance

| Source / row | Result | Evidence / remaining constraint |
|---|---|---|
| Host services M12 Calendar naming | BLOCKED | No Calendar grant or synthetic event-store fixture; no calendar enumeration or permission request performed. |
| Startup U01/P03 fresh/missing model | BLOCKED | Fresh loading→setup passes; full setup, selected-model removal and hide-during-pending-startup completion need a model/controlled delay fixture. |
| Startup V05 migration | PASS | Synthetic nonpersistent worker, fresh-process repeat and exact-child interruption/retry receipts above. Production path equality uses calculation only. |
| Startup P03/U03 zoom | BLOCKED | 50/100/200/300% input/IME/AX frame and Retina/non-Retina movement require working native shortcuts/IME and another display. |
| Startup P05/V01 automatic checks | BLOCKED | Incomplete setup and manual checks covered; complete-setup wake/focus cadence not accepted. |
| Startup V02–V04 updater fixture | BLOCKED | Signed synthetic Ready/Later, failures and required Quit covered; replacement/restart/What's New remainder above. |
| #1035 IME/Edit menu/shortcuts | BLOCKED | Enabled sources and attempted keyboard/read-back evidence above; no grants or input-source changes made. |
| #1035 services/panels/relaunch | BLOCKED | Services submenu exposed; native cancellation/About/Diagnostics and fresh-process persistence covered. Full service actions, file matrices and coordinator relaunch remain unverified. |
| #1035 VoiceOver AX tree | PASS | Native roles, accessible labels, value/focus updates, palette Pick actions and corrected `AXHeading` observed. VoiceOver was not toggled. Spoken announcements and keyboard VoiceOver navigation are not certified. |

## Defects fixed on layer 24

- **Layer 20 test setup:** Tauri mock/IPC tests lacked host-feature guards.
  Native-only lib tests therefore failed to compile. Guard the host-specific
  tests, retain shared tests, and provide a test-only Tokio runtime; production
  still requires explicit runtime installation. Geometry conversion tests use
  shared dpi types. The model-path test recognizes missing native path injection.
- **Layer 20 native linkage:** direct native test executables lacked the Swift
  runtime rpath. Native builds now emit the same `/usr/lib/swift` rpath used by
  the GPUI executable; the suite and contract run without a DYLD override.
- **Layer 20/21 compact geometry:** native supported sizes were rejected by the
  Tauri minimum on relaunch. Native restore now accepts its 720×560 content
  minimum and Diagnostics uses that minimum too; Tauri keeps 840×620. The new
  regression plus actual compact relaunch pass.
- **Layer 9 accessibility adapter:** headings exposed the nonstandard `Heading`
  role. The final native focus test caught it; use `AXHeading`, with adapter
  regression and vendored patch round-trip coverage. The native snapshot now
  reports the standard role.

The newly integrated layer 22 settings transaction compiled without a merge
repair. No `settings_store.rs` merge fix was needed.

## Verification and cleanup

Final check receipts are in [checks.json](qa/gpui24/checks.json). The relevant
commands include workflow policy, packaging Python tests, patch round-trip tests,
GPUI check/library tests/strict all-target Clippy/formatting, native-host service
check/library tests, geometry tests, and the calculation-only host contract.
The default Tauri library test build was also compiled to ensure its mock tests
remain available; it was not executed as part of this native pass.

Independent Sol review and GitHub CI belong to the lead. This PR remains draft,
targets `gpui/23-startup-integration`, and must not be merged by this worker.

The test app and exact owned fixture-server session were stopped. Disposable
bundles, synthetic HOME/legacy stores, signing fixture/key, screenshots not
retained here and temporary fixtures were removed. The lane checkout and its
paired target remain for reuse. No production app/data/agent was opened, no real
pasteboard contents were read, and no OS setting or permission grant was changed.
