# GPUI 1.8: shutdown coordinator

Implements #992, part of #1009. Draft stack layer 07, based on
`gpui/06-webkit-lifecycle` at `88854edc9fa909b820a7330b88f6e95732749d6b`.
The approved design is the GPUI 1.8 design produced at that revision.
During implementation the base moved to `269dfa39`; this layer was rebased onto
its source-recovery generation and presentation-cache fixes, then rebased again
onto `400a4648` to retain unresolved recovery-evidence pruning fixes.
Independent implementation review and the native Tauri matrix belong to the parent.

## Policy and ownership

`murmur-core::shutdown::Coordinator` is a Tauri-free reducer. It accepts
monotonic milliseconds, requests, and `(epoch, step)`-tagged receipts. The single
process-local epoch progresses through Running, ClosingAdmission,
DrainingCapture, ConfirmingHelpers, FlushingWriters, ReadyToTerminate,
TerminationIssued, and Exited. Duplicate, mismatched, expired, and old callbacks
cannot advance a completed phase. Repeated requests cannot extend deadlines.
Quit suppresses a pending restart; system termination does too. After terminal
commit there is no rearming. A replacement process constructs its own coordinator.

`src/shutdown.rs` is the Tauri adapter. It closes an atomic process latch before
queueing a request. A dedicated deadline thread runs independently of owner
workers and disk waits. Effects use AppContext background/main dispatch. Owner
work stays retained if its waiter times out; no timed-out future is described as
a cancelled OS call. RunEvent::Exit only records the content-free terminal
receipt. It does no teardown, disk publication, inference, or joins.

| Phase | Absolute deadline from first request | Evidence / failure |
|---|---|---|
| Admission | Immediately; transition fence target +250 ms | Permanent latch at controller and actual helper spawn boundaries; late configuration cannot reopen it |
| Capture / meetings | +5 s | Original dictation generation stops through its existing pipeline; Processing is joined; meeting completion wait retains its owner on timeout |
| Helpers | +7 s | Exact ManagedChild owners cancel/stop; only direct-child reap plus empty group releases the global outstanding-owner receipt |
| Writers | +10 s | Registered source/file/correction/Paste Last operations drain; FIFO source, window and memo barriers; completion seal/reconciliation |
| Terminal | Completed phases or +10 s | One committed action; missing evidence remains degraded and suppresses relaunch |

Capture start, preparation, passive Smart Auto and every ManagedChild spawn check
the permanent latch. A spawn fence crosses previously admitted spawns before
helper receipts are inspected. The coordinator never searches helper names,
steals children, or signals already-confirmed numeric PGIDs. Inventory threads,
preview/standby, preparation, diarization, installer and meeting-suggestion
owners retain their existing exact termination mechanisms. Installer supervision
observes shutdown during its polling loop. Unconfirmed groups prohibit restart.

Meeting shutdown returns a completion receipt rather than clearing
`meeting_active` after an ignored wait. New inference claims stop immediately
on shutdown; already durable SQLite rows and WAV spools remain pending for the
existing startup recovery. Existing inference may finish, but optional backlog
is not consumed merely to quit. No persistent dictation audio is added.
An unfinished utterance at the deadline is represented by a failed capture
receipt, not by a recovery promise.

## Writer contract

The completion actor has asynchronous `flush`, `seal_and_flush`, and
`shutdown_receipt` barriers. Sealing is ordered with enqueue under its admission
lock. The host registers accepted producer/source work before asynchronous
continuations and waits for those permits before sealing. Dictation and meeting
owners also have their generation/completion fences. Managed service handles
remain retained off main even when the deadline stops waiting.

`completion-pending.json` contains a single private immutable publication
snapshot for History, reports or pre-migration holding data. It is separate from
the content-free statistics journal. Existing stable completion UUIDs and the
snapshot's destination envelope identify the publication. Pending file sync
precedes destination publication; destination and directory sync precede pending
removal. Replay overwrites the same envelope rather than appending again.
Every subsequent command recovers before admitting new work. An unresolved
pending destination rejects admissions rather than evicting accepted work.

This deliberately uses an owner publication snapshot instead of a second
per-entry append queue: the existing single actor already serializes the
destination and its clear/deletion/migration receipts. Replay runs before those
mutations, so a subsequent Clear or Delete cannot resurrect content. Existing
History/report caps also bound the snapshot. Retention-off producers never
submit a transcript entry and never create transcript-bearing pending work.
The immutable recording retention decision remains resolved at recording start.
Pending content is private local storage, excluded from diagnostics and exports.

A failed immediate History publication still reports the existing content-free
diagnostic. A synced pending copy can yield PreservedForReplay; failed pending
publication records UnpreservedFailure even if later counts or snapshots flush.
No timeout changes in-memory work into durable evidence. Non-completion writer
failures conservatively suppress restart even if some pending documents exist;
they are not classified as preserved without a complete owner receipt.

Settings and theme writes keep WRITE_LOCK, normalized source schemas and
owned-field revision merging. Source IPC queues immediately to one native FIFO;
React no longer waits for a prior IPC acknowledgement before submitting the
new edit. Retry presentation state remains generation-aware. Immediate requests contain
only their newly deliberate owned-field patches; older acknowledged patches
are removed from retained retries so they cannot overwrite a native quick control. Source commands
execute disk work off main and carry writer permits. UI and window snapshots
use synced `.pending` documents with replay before load. The window worker
clones state under its mutex, then releases it before I/O; its flush is FIFO.
Capture memo saves use one tracked serial writer and a flush acknowledgement.
Window startup reads/recovery and memo recovery also run on workers. Window
restoration is dispatched to main; early renderer-ready/chrome intents are retained
until that restoration settles. Request timestamps are captured before enqueue,
so delayed delivery to the deadline driver cannot extend the shutdown budget.

The standalone WebKit importer exposes `preserve_for_shutdown`: it freezes
new importer work and retains partial progress for retry. It fabricates neither
migration bits nor key-removal acknowledgements. The current Tauri host still
uses the React importer; no new production native importer is created here.

## Native termination and restart

The owned tao 0.35.3 source patch retains upstream LICENSE and LICENSE.spdx and
keeps tao's delegate. Its added `applicationShouldTerminate:` returns the typed
NSUInteger-backed NSApplicationTerminateReply, not BOOL. Registration is main-only
and once per process; the callback is cloned before invoking the host.
`TaoApp.terminate:` relays to its superclass via a common-mode CF run-loop block,
outside the original dispatch callback. MainThreadQueue retains FIFO and
outside-lock execution but uses the same common-mode scheduling.

The host maintains one AppKit pending-reply latch. Native quit requests are
conservatively system-priority: they all suppress relaunch, without retaining
Apple-event payloads. Once the reducer commits a terminal permit, the adapter
replies true on main only if AppKit previously returned Later. Otherwise it
issues the runtime exit/restart. It never sends a spurious AppKit reply.
The default app menu's final predefined Quit is replaced with a custom Quit
item and Cmd-Q accelerator; other standard menus remain intact. Tray Quit uses
the same request path. Main-window close continues to hide. Actual destruction
of the final runtime window and ordinary app.exit reach ExitRequested, where
prevent_exit is synchronous until permitted.

The app-owned compatibility plugin is named `process`. Its exit/restart IPC
names and process permission manifest remain compatible; only Main is accepted.
Only the terminal adapter calls request_restart, after the reducer's drained
permit. AppHandle::restart and other restart identifiers are forbidden by a
Rust-token policy with nested-comment/raw-string handling, including aliases,
UFCS, references and macro token trees. This is a conservative source guard,
not a proof over arbitrary generated Rust. The owned tao source is an exact
third-party exclusion shared with the transport guard; maintained nested vendor
paths remain scanned. A non-process AEC helper formerly called restart is now
named reset_stream to keep the reserved-name policy simple.

SIGTERM/SIGINT handlers only write a byte to a nonblocking self-pipe. A normal
reader thread requests system shutdown. No Rust locks, allocation or AppKit
calls run in the raw handler. SIGKILL, Force Quit, abort/crashes and OS forced
termination cannot be coordinated; durable state and helper parent-watchdogs
provide recovery/containment, not a successful drain receipt.

Until #990, the existing updater installs before requesting the compatibility
plugin's restart. The core exposes a process-local RestartTicket handoff. Installation transaction ownership, v3 migration and removal
of process:allow-restart remain #990 work. This layer closes its relaunch bypass;
it does not claim installation has moved to the native updater.

## Verification and native matrix

Linux: TypeScript passes; Vitest is 180 files / 1,835 tests (updated base 1,833; two
source-submission/owned-field regressions added, none removed). Core unit/integration,
strict Clippy, policy fixtures and final CI results are recorded in the PR.
Core tests cover phase order, deadlines, arbitration, duplicate/late receipts,
restart prerequisites, pending-content publication failures at before rename,
after rename and directory sync, replay once, Delete/Clear/retention, capacity,
explicit barriers and unpreserved-vs-preserved receipts. Existing source-publication
fault tests retain their real Storage fault injection.

Mac verification uses only `~/Builds/gpui-stack-18` and its dedicated
`~/Builds/gpui-stack-18-target`, Rust 1.98.1, via `fleet exec --via opti mac-mini`.
The windowless harness=false `shutdown_appkit` executable uses actual patched
tao and AppKit, no product data/windows/input. It holds termination for 6.5 s
and requires 65 main-thread FIFO callbacks before replying. A watchdog fails
if modal dispatch stalls. This establishes the patch/queue seam, not the full
Tauri UI matrix below. Exact Mac check/test/Clippy results belong in the PR.

For the parent: build the explicit shutdown-test-support feature with
`tauri.shutdown-test.conf.json`. It requires the distinct
`com.localdictation.shutdown-test` identity and synthetic data only. The feature
adds `plugin:shutdown-test|shutdown_test_action`, gated to Main and that identity,
with quit/relaunch/exit/destroy-last actions. It holds the final release for
6.5 s after real owners, services mixed requests during the hold, and records
195 expected callbacks across AppContext, Tauri dispatch and CF common-mode
heartbeat. Optional per-phase `MURMUR_SHUTDOWN_DELAY_{ADMISSION,CAPTURE,HELPERS,WRITERS}_MS`
values (capped at 20 s) inject synthetic stalls only for this identity.

Use the PID-validated sender in `scripts/send_shutdown_test_quit.py` for system
reasons; it verifies the isolated bundle ID and actual process executable twice,
never addresses loginwindow, and sends no-reply events with a 10 s sender timeout.
See [native QA](../DEVELOPMENT.md#shutdown-coordinator-native-qa).

Every row is pending parent native verification. Run each row with active
synthetic dictation, an active synthetic meeting, and pending synthetic writes.

| Quit path | Native drive |
|---|---|
| App menu Quit | Invoke custom application menu item through AX |
| Cmd-Q | Real keyboard chord with isolated app active |
| Dock Quit | Real HID click on Dock context-menu Quit; AXPress alone is insufficient |
| Plain Apple-event quit | Address isolated bundle only |
| Simulated logout | PID-only sender, reason logout |
| Simulated system restart / shutdown | PID-only sender, reason restart/shutdown; never actual machine actions |
| Tray Quit Murmur | Native status-item menu |
| Updater relaunch | Isolated update fixture; observe old PID exit and one new PID ready |
| Process-plugin restart | Authorized existing Main relaunch path |
| Programmatic exit | Isolated test action exit |
| Actual final-window destruction | Isolated test action destroy-last |
| SIGTERM / SIGINT | Verified isolated PID only |
| Repeated/mixed requests | Repeat Quit, restart during Quit and Quit during restart hold |

For each run require: new recording/meeting admission refused; original capture
generation stopped; no fallback/passive/standby respawn; helper groups empty or
an explicit failure; accepted completed content replayed once; meeting rows and
spools recoverable; source/theme/window values committed or replayed; Clear and
retention effective; main dispatch running throughout hold; one terminal action;
one replacement for restart; replacement's own quit drains once. Separately
exercise helper death, WebContent death and forced process death as recovery
scenarios. No full native matrix pass or independent review is claimed here.
