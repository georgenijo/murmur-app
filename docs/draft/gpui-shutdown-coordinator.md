# GPUI 1.8: shutdown coordinator

Implements #992, part of #1009. Draft stack layer 07, based on
`gpui/06-webkit-lifecycle` at `88854edc9fa909b820a7330b88f6e95732749d6b`.
The approved design is the GPUI 1.8 design produced at that revision.
During implementation the base moved to `269dfa39`; this layer was rebased onto
its source-recovery generation and presentation-cache fixes, then rebased again
onto `400a4648` to retain unresolved recovery-evidence pruning fixes. The current
base is `dae05b20`, including guarded presentation sessions, owned-key tombstones,
reload coalescing and content-free reload-failure diagnostics.
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

Linux: TypeScript passes; Vitest is 181 files / 1,861 tests (updated base 1,859; two
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

Every row is pending parent native verification. Also verify early renderer-ready/
chrome intents, explicit hide during delayed startup restoration, and quit during
a delayed startup snapshot read. Run each row with active
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


## Round-one review corrections

The implementation was rebased before editing and again after layer 06's reload
follow-up. Only layer 07 is pushed. The parent owns independent Sol re-checks and
the native GUI matrix; the implementation worker performs no GUI input/app launch.

| Finding | Root cause and fix | Regression / before-after evidence |
|---|---|---|
| I1 | Telemetry recognized two internal IDs and defaulted every other ID to production. All non-production IDs now use an injective hex-encoded namespace. Internal-service/shipper exclusions cover all such IDs. Legacy models/corpus/caption stores and anonymous worker CLI startup use the same identity; implicit shared model fallbacks are disabled. Autostart/updater registration is production-only. | The actual old telemetry resolver fails the shutdown-ID disjointness assertion. Native `shutdown_and_future_bundles_never_resolve_production_logs` and `every_nonproduction_identity_is_disjoint_from_production_and_each_other` cover the shutdown/bench/refactor/future/path-like IDs. Full table: [QA identity audit](../DEVELOPMENT.md#shutdown-qa-identity-isolation-audit). |
| N1 | Mailbox delivery hid an already accepted Quit behind the final writer receipt. Request acceptance now mutates arbitration under the same reducer mutex as `issue`; the channel only carries accepted effects. | `accepted_requests_arbitrate_even_behind_the_final_writer_receipt`: Quit/System/second Restart behind Finished(Writers), one terminal action. Old mailbox acceptance fails; current acceptance passes. Existing deadline/mixed/late-owner cases use the production acceptance seam too. |
| N2 | `prepare` loaded/recovered/published settings synchronously during setup. It now reserves a configuration revision and producer permit before queuing native-source work; shutdown and revision checks precede application, and native geometry remains on common-mode main dispatch. | Native `quit_reaches_terminal_while_startup_settings_io_is_blocked` holds the actual source FIFO, proves off-main execution and a terminal Quit while startup remains blocked. Existing FIFO/producer-barrier tests remain. The original blocking setup path was established by the round-one native review; no GUI launch is performed by this worker. |
| N3 | Reserved restart identifiers did not include the original plugin crate or Tauri's special exit code. The token guard now rejects the original process-plugin crate (including aliased initializer imports) everywhere and reserves RESTART_EXIT_CODE to the adapter's detection seam. | Six plugin/code alias/macro fixtures fail against the old guard and pass now, within five fixture groups. The adapter alone still owns request_restart. |
| N4 | The full self-pipe write leaked EAGAIN into interrupted code. The raw handler saves/restores thread errno, retries EINTR and treats EAGAIN as an existing notification. | `full_pipe_signal_preserves_interrupted_errno` delivers a real thread-targeted SIGUSR1 against a saturated pipe. The old handler fails (EAGAIN replaces EDOM); the new handler passes. No product process is signalled. |
| D1 | Holding replay checked deserialization only; generic snapshots checked only that value existed. Replay now shares complete Holding/document validation with receipt classification. UI, native window and capture-memo recovery require destination validators before any rename; no unvalidated public generic recovery remains. Rejected evidence uses sequenced quarantine/pinning/pruning. | `pending_holding_validates_entries_before_replacing_readable_content` and `snapshot_payload_validation_preserves_destination_and_evidence` fail on the old code and pass now, preserving byte-identical readable destinations. Existing malformed/future source matrices remain. |
| D2 | A crash before pending rename left synced transcript temps outside Clear. Startup/reconciliation removes exact pending/destination temp families. Resolved pending evidence is removed by Clear, Delete, caps and non-retaining producers; unresolved evidence stays pinned until explicit recovery. | `clear_removes_pending_publication_temp` in the baseline fails. `abandoned_pending_temps_and_resolved_evidence_obey_clear_delete_and_caps` covers Clear/Delete/new capped content/retain=false, plus recovery preserving evidence until resolution. Existing retain=false/admission/cap/replay tests remain. |
| D3 | Only Complete failures affected receipts; report/UI/direct mutations could lose their intended value and later receive Committed. Exact failed pending payloads and pre-pending report content now remain with the owner for reconciliation. Source operations retain their exact failed Save. Direct mutation failures retain exact attempted snapshots; a failed retry remains UnpreservedFailure. | `failed_report_and_ui_intents_are_retained_until_exact_publication_recovers` fails before and passes now. `report_failure_before_pending_admission_retains_exact_held_report` covers cap-write failure; `direct_mutation_failures_cannot_receive_committed_until_exact_snapshot_is_retried` covers Clear/Pin before rename, after rename and directory sync. Successful exact retry earns Committed only after publication. |
| D4 | Initialization imported/consumed Holding before pending replay. Pending replay and abandoned-temp cleanup now precede initialization mutations and automatic no-WebKit import. | `startup_replays_held_completion_before_no_webkit_automatic_import` fails before and passes now, retaining exactly one row across another launch. |
| D5 | Quarantined pending evidence blocked every later launch without an exit. Main-only typed `recover_completion_pending` preserves rejected evidence, requires readable current stores and durably records explicit resolution. A valid pending intent always replays. The existing startup recovery screen exposes the operation and success/failure states. | `explicit_pending_recovery_through_public_service_across_two_launches`, `explicit_pending_recovery_survives_restart_without_replaying_rejected_evidence`, and `explicit_recovery_replays_usable_pending_instead_of_discarding_it`; two UI success/failure cases. Procedure: [completion recovery](../DEVELOPMENT.md#completion-pending-recovery). |
| F1 | Successful newest-generation raw publication deleted older failed field patches. Acknowledgement now removes only fields actually submitted at the same field generation. Typed notifications identify acknowledged fields; an unrelated owned-field acknowledgement retains the other patch. A native edit of that field supersedes it. | Both `a successful unrelated save retains the failed %s patch` cases fail against the pre-fix module. The neighboring different-field acknowledgement/native-supersession test passes; the real hook forwards validated acknowledgement metadata. Existing rollback/supersession/generation and layer 06 propagation suites remain. |

No transcript/audio/clipboard/selection payload enters logs or receipts. Retained
in-memory failed intents are not claimed durable until their exact publication
commits or a validated pending copy is synced. They can still be lost on forced
process death during a continuing disk failure; receipts remain degraded then.

The pinned FluidAudio SDK exposes no path override. Its ASR initializer and
speaker diarization are therefore unavailable to non-production bundles to avoid
implicit production-cache access. Synthetic native QA uses isolated Whisper or
Parakeet models. This limitation is deliberate and does not change production.

Final Linux/Mac counts and final-head CI are recorded in the PR body. Baseline
comparisons used a synthetic source archive under the Linux cache, not another
active checkout. No independent review is claimed by this worker.

## Round-two corrections

Renderer source saves now carry a fresh per-webview-session UUID and a monotonic
submission generation, including settings/theme bootstrap saves. All three native
source commands check the session/key generation inside the native FIFO before
running their publication closure. Stale or duplicate requests return the distinct
`superseded` receipt without touching disk, emitting commit events, or acknowledging
owned fields. Failed publications also fence older generations because their exact
payload may already belong to replay; retries receive fresh generations. Settings
acknowledgement events carry their session as well as the original generation and
`acknowledgedFields`, preserving round-one per-field retry ownership.

`flushSourceState` now drains until no intent or owned field remains: joining an
in-flight request with fewer fields cannot count as a completed retry. Identical
content is deduplicated only while its request remains in flight, so repeating a
failed save submits immediately. Superseded receipts retain unacknowledged fields
and retry the latest document rather than acknowledging newer content or reporting
a storage failure. Native quick controls retain the existing locked revision merge.

The three targeted Vitest regressions fail on `ad1e1976` and pass after these fixes.
Additional tests cover superseded settings/themes/other source keys, fresh retry
generations, and old-session acknowledgement isolation. Core Rust tests reject
stale/duplicate generations (both fail with the fence disabled), retain fences after
failures, and scope keys, sessions and owning windows independently. Native adapter tests deterministically construct A
before B but poll B before A and prove the older publication never runs; a settings
integration test combines this ordering with a native quick-control commit and
preserves the deliberate fields and latest language. Existing authoritative reload,
tombstone, coalescing, and content-free warning tests remain unchanged. The parent
owns independent Sol rechecks and GUI verification; this worker launches no app.

Round-two verification at `ca9069a8`: TypeScript and all 182 Vitest files / 1,888
tests pass; all 185 Linux core unit/integration tests and restart policy pass. Mac mini Rust 1.98.1 passes the full native
`cargo test -- --test-threads=1` suite (1,528 library tests, 14 ignored, plus
integration suites) and `cargo clippy --all-targets -- -D warnings
-A clippy::chunks_exact_to_as_chunks -A clippy::byte_char_slices`. The native
reordered-task and quick-control tests both pass. No GUI verification or new
independent review is claimed.

## Round-two isolation and guard corrections

- I1: implicit transcript/audio/report exports select an identity-namespaced
  app-data `exports` directory. Only initialized production selects Documents;
  absent identity/data roots fail closed. Synthetic regression writes production
  file 1 and QA file 1 independently, checking production is unchanged.
- I1: the fixed production capture-agent Mach endpoint is inaccessible from
  non-production/uninitialized CLI dispatch. All probe/recovery/status and
  service actions refuse before launching a helper or invoking SMAppService.
  The shutdown QA matrix needs ordinary capture helpers, not this experiment.
- N2: telemetry installs its subscriber before worker disk startup. History reads,
  ring seeding, file open/rotation, and both JSONL/pretty writes run on one worker;
  early events queue and restored history is prepended to live events. Clear
  invalidates queued output and pending restore generations, preventing deleted
  logs from reappearing after asynchronous startup. A blocked
  synthetic reader regression proves setup returns and early output survives.
  DEVELOPMENT lists the other synchronous setup file operations still present;
  this fix does not establish an all-startup-I/O-off-main claim.
- N3: the restart guard inventories every maintained Cargo.toml recursively,
  including workspace/target/patch package renames, and reserves their Rust crate
  identifiers. Runtime exit uses a conservative literal-zero exit-code allowlist;
  unknown expressions, const aliases, numeric MAX spellings, UFCS and function
  references are refused. Audited pre-Tauri CLI process exits remain supported.
  Fixtures cover renames in every table, inherited workspace dependencies,
  decimal/hex/octal/binary/MAX/alias exit bypasses, and malformed manifests.

Verification at `06a03c31` (combined with the other worker's source-order fixes):

- Linux: all 185 murmur-core unit/integration tests pass; workspace formatting,
  core dependency boundary, targeted-event scanner and restart scanner pass.
  Both policy fixture suites pass (24 tests; nine restart fixture groups).
  The old guard accepts the reviewed dependency alias, `i32::MAX`, decimal MAX
  and const-alias examples; the corrected guard rejects all four.
- Mac mini, Rust 1.98.1: `cargo test -- --test-threads=1` passes both without and
  with `--features shutdown-test-support`: each has 1,533 passing library tests,
  14 pre-existing ignored library tests, passing integration suites and the
  windowless main-dispatch/shutdown executables. Both `cargo clippy --all-targets`
  runs pass with `-D warnings -A clippy::chunks_exact_to_as_chunks
  -A clippy::byte_char_slices` supplied only on the command line.
- An initial native run hit the existing Unix-socket fixture's `SUN_LEN` limit
  with a long temporary root. Shortening TMPDIR to the disposable checkout's
  `t` directory resolves it; the complete matrix above was rerun at the exact
  pushed code revision. No tests were skipped to obtain these passes.

Native checks used only the new `~/Builds/gpui-stack-19` checkout and
`~/Builds/gpui-stack-19-target`; Linux scratch was under the task cache.
No GUI, production capture-agent contact, app launch or merge was performed.
Independent Sol rechecks and native GUI verification remain with the parent.

## Round-two completion data-safety corrections

Completion changes stay in `murmur-core/src/completion/`; no frontend or host
callers change. The stack remains draft and must not be merged. Independent Sol
re-checks remain with the parent task; this implementation worker claims no
independent review or native GUI verification.

- **Published completion retried after pending unlink:** content reconciliation
  checks its stable `completionUuid` in both History/Reports and Holding before
  appending. A published-but-unsynced outcome is synced again without duplication.
  `round_two_unlinked_pending_sync_failure_retries_completion_exactly_once` faults
  the directory-sync boundary after pending deletion, covering both migrated
  History and pre-migration Holding, then verifies one row after restart.
- **Malformed legacy pending shape:** replay requires the current versioned
  destination envelope and its committed import marker. Legacy arrays remain
  import inputs only. A current receipt also cannot downgrade a future destination
  schema. `round_two_legacy_pending_shape_preserves_destination_and_evidence_bytes`
  checks History and Reports byte preservation and exact quarantined evidence for
  legacy arrays, future shapes, and uncommitted envelopes.
- **Complete History mutation:** Holding and History snapshots are both retained
  before the first publication. Recovery finishes those exact documents before
  accepting later relative mutations.
  `round_two_holding_failure_retains_all_history_mutation_targets` covers all nine
  Pin/Edit/Delete × BeforeRename/AfterRename/DirectorySync combinations, including
  another failed retry, final Committed, and restart verification.
- **Next intent behind a failed retry:** the owner retains completion commands and
  lifecycle saves in a shared submission-order FIFO before initialization or
  reconciliation can return early. Exact failed snapshots are reconciled before
  taking the next intent; relative History operations are not reapplied as
  toggles. `round_two_retry_failure_keeps_next_report_in_submission_order` retains
  both reports, and `round_two_retry_failure_keeps_last_ui_value` retains the
  true-then-false UI sequence through shutdown and restart. The existing unreadable
  Holding test now also verifies that its accepted completion publishes exactly
  once after the temporary read error clears.

Before the fixes, all five new regression tests failed against the existing
implementation (using only the added unlink-sync fault seam). After the fixes,
Linux `cargo test` passes all 155 unit and 35 integration tests on the combined
round-two branch, including the
round-one D2 Clear/temp, D4 replay-before-migration, and D5 explicit-recovery tests.
Strict all-target clippy passes with `-D warnings -A clippy::byte_char_slices`.
The requested additional `clippy::chunks_exact_to_as_chunks` exemption is unknown
to this host's Rust/clippy 1.93.1; the literal requested invocation rejects that
unknown lint, and the supported strict invocation above passes without it.
`git diff --check` passes. No public API, frontend, or app host caller changed, so
no frontend checks or Mac host-caller test run is required for this worker's delta.
All fixtures and injected failures are synthetic; no user stores were opened.

## Round-three source ordering corrections

P1-A: UUIDs alone did not order successive renderers. Every source producer now
joins one `register_source_session` handshake before sending writes. Native
registration order establishes a session epoch for each window label: the new
session retires its predecessor across **all** keys, and an old registration
retry cannot reactivate it. Registration, retirement, fences and publication
share the native source FIFO and order mutex. No renderer timestamp decides
precedence. The current/retired session table is the epoch representation; it
needs no persisted counter because renderer sessions cannot outlive the host.

The actual writers are deliberately narrow: main owns settings, theme library
and all generic source snapshots; companion full snapshots remain rejected.
Overlay owns only `autoPaste`/`disabled` field patches. Main and overlay register
independently, so a live overlay is never retired by a main reload (or vice
versa). Diagnostics and dictation preview only read source data. Concurrent
windows' genuine edits are ordered by native queue acceptance for each field,
without comparing their unrelated renderer counters. Reloaded main starts at
generation one; its predecessor's delayed generation 100 is superseded before
publication, including on keys the successor has not yet written.

An intent sent by a renderer that crashes **before native accepts it** has no
native ownership guarantee. If it arrives before that label's next registration
it can be accepted in the old epoch; once the replacement registers it is
superseded without publication. A renderer must complete registration before
sending intents, so an old write cannot masquerade as a new session. An
uncompleted registration is not an accepted edit; registration order itself
defines which session is current. No crash recovery promise depends on renderer
memory or message delivery that native never accepted.

P1-B: settings snapshot fencing no longer discards owned patches. Each deliberate
field has its own original generation, carried unchanged on retries, and its own
native fence. An accepted settings request owns both its snapshot and patches
inside the native queue/WriterPermit lifetime. A superseded snapshot still
publishes its fresh field patches against the **current native document**, leaving
language and other fields intact. Full snapshots always take existing owned
values/revision from native; only deliberate patches can change them. Overlay
quick controls and these patches use the same FIFO, locked merge and full-sync
publisher. Different fields never replace one another. Distinct edits of one
field resolve in native acceptance order across labels; within one renderer its
per-field generation prevents reordered/duplicate edits from reversing a newer
edit. A retry of an already accepted field ID cannot replay it behind a later
quick-control change.

Snapshot receipts remain `applied` or `superseded`. A settings receipt separately
returns `ownedPatchGenerations` for the exact field IDs now owned by native;
these never acknowledge a newer document or another field. Commit events retain
their session/generation and per-field acknowledgements. A failed publication
continues to retain the exact merged value in the durable source owner and
reports failure; a later retry or the shutdown source recovery drains it. Native
shutdown waits accepted writer permits, recovers source publications and seals
the completion owner. It requires no `flushSourceState` call from a renderer.
Renderer flush still drains all retained fields, and an identical retry after
failure still sends again. Retained patches keep their original field IDs even
when the document retry gets a fresh generation.

Regressions use synthetic data and the actual native FIFO: the exact old-session
100/new-session 1 sequence for settings/themes/generic keys; the exact delayed
main Auto Paste patch after an unrelated newer Spanish snapshot and an overlay
disable, followed by native writer recovery and the completion shutdown receipt
with **no renderer flush** (`autoPaste=true`, `disabled=true`, `language=es`);
main/overlay same-field ordering, duplicate field retries, different-field
preservation, overlay reload retirement and continued main edits. Core tests also
cover retirement on untouched keys and independent field fences. Frontend tests
cover the registration barrier, fresh registration on reload, handshake failure,
and separate exact-field receipts on superseded snapshots. Existing per-field
acknowledgement, flush, identical retry and authoritative-cache tests remain.

This implementation worker performs no GUI action, merge or independent review;
the parent retains the independent Sol recheck gate.

The native field owner also retains patches that fail during the read/merge,
before a complete document can reach the durable publisher. A later source
commit combines those retained fields with its own fresh patches; shutdown
retries any remainder under the same writer lock. A later quick control replaces
only its own field in that owner. Configuration revision reservation remains
before disk publication, preserving the existing native override fence.

The new session command is documented in the command reference. Current command
counts in the agent guide, architecture and feature index are 186; historical
rewrite-plan inventory counts remain historical.

Linux verification: `npm ci`, TypeScript, all 183 Vitest files / 1,892 tests,
and all 192 core unit/integration tests pass. Every local command in the CI
`policy` job passes, including reference/link validators, 435 Python unittest
cases, 25 canary tests, restart guard suites and 42 frontend event-guard tests.

Mac mini verification at executable revision `2c69fcaf`, Rust 1.98.1, the
specified build mirror and shared target directory: both full native test runs
(default and `shutdown-test-support`) pass, each with 1,538 passing library tests
and 14 existing ignored tests, plus the integration suites. Both configurations
also pass `cargo clippy --all-targets` with `-D warnings` and only the two
requested command-line allowances. No GUI/app launch or permission grant occurs.

Before/after evidence: the new frontend superseded-field receipt test fails
against the actual `9d69cec8` lifecycle module (it re-sends the already native-owned
Auto Paste field) and all 27 lifecycle tests pass after restoration. Native
defect-restoration checks keep the actual FIFO, regression and publisher intact:
restoring independent-session acceptance makes the exact old-100/new-1 test fail
with `Applied` instead of `Superseded`; restoring snapshot-coupled patch selection
makes the exact shutdown-recovery test fail with `autoPaste=false`. Restoring the
fixes passes both, all four round-three settings regressions, and the round-two
same-session reordered-task regression. These native comparisons restore the
specific old fence/patch defects in the build mirror, rather than claiming that
new registration APIs were present in an older binary. The maintained source is
restored byte-for-byte after each comparison.

The concurrent-window regression also fails when field retries are treated as
fresh patches (the previous behavior without original field IDs): a main retry
undoes the newer overlay Auto Paste value. It passes again with the field fence
restored, including different-field preservation and overlay reload retirement.
The complete CI `policy` shell blocks were rerun locally after the final docs and
regression adjustment; no policy step was omitted.

## Round-three completion durability corrections

Completion changes remain inside `murmur-core/src/completion/`. The base is
unchanged and this draft stack must not be merged. Independent Sol review remains
with the parent task; this implementation worker does not claim that review.

- **Complete mutation durability:** `completion-batch.json` contains the ordered
  Holding and History snapshots in one versioned write-ahead intent. The publisher
  fully syncs the intent file and its directory before changing either target.
  Startup and live recovery replay exact snapshots in order and retire the intent
  only after every target is durable. A failed intent admission retains the whole
  batch for retry and returns an error. Target failures cannot create independent
  partial-batch retries. Unknown retirement outcomes can safely replay the batch.
  A SHA-256 digest binds the complete ordered target set, detecting valid-JSON
  damage after restart. Every target and future-schema guard is checked before
  publication; malformed evidence is quarantined, pinned, and continues blocking
  recovery without replacing readable documents. Live retries validate existing
  evidence against their retained intent and never overwrite damaged or conflicting
  evidence with an in-memory copy.
- **Report identity conflicts:** a matching `completionUuid` deduplicates only
  identical content. Different content with that identity returns the visible
  `IdentityConflict` error, retains the original, and acquires no retry ownership.
  Reports without an identity still receive fresh internal UUIDs, so identical
  independent submissions remain distinct.
- **Bounded retry retention:** completion commands and lifecycle saves share a
  maximum of 64 retained requests and 4 MiB of serialized request payload. Size
  accounting is cached per retained request. Overflow returns `RetentionFull`
  ("submission not accepted") before taking ownership. Storage errors remain
  visible to callers; requests awaiting recovery may remain in this bounded
  in-memory FIFO. Such memory is not durable acceptance, and the existing shutdown
  guard returns `UnpreservedFailure` until all retained requests and snapshots
  drain. Overflow is never replayed. Durable batch/content/snapshot intents are
  used once publication reaches their respective admission point; the memory
  fallback does not promise survival across process exit.

The new synthetic regressions failed before these fixes: the complete process-exit
matrix reproduced **24 lost mutations out of 36 despite Committed**, UUID/content
conflicts returned success, and 256 failed submissions exceeded the FIFO bound.
The corrected 36-case matrix covers Holding/History × BeforeRename/AfterRename/
DirectorySync × initial/retry × Pin/Edit/Delete, discards all owner memory at
injected exits, and checks disk recovery plus the terminal receipt. Additional
cases cover batch admission and retirement faults, damaged evidence preservation
(including live retries and valid-JSON damage after process exit),
future targets, exact report replays and conflicts before/after migration, both
FIFO limits, mixed lifecycle/completion pressure, rejected oversized requests,
failed shutdown while requests remain in memory, and ordered recovery afterward.

After rebasing onto the concurrent worker's `7b9b9358` head, Linux `cargo test`
passes all **166 unit tests and 35 integration tests**, including every round-two
regression and D2/D4/D5. All-target clippy passes with
`-D warnings -A clippy::byte_char_slices` on rustc/clippy 1.93.1; the unsupported
`chunks_exact_to_as_chunks` exemption is omitted as requested. Rustfmt checks and
`git diff --check` pass. The two additional live/restart damage regressions were
also confirmed red against the pre-digest implementation, then green here.
No app host caller or frontend changes, Mac checkout, GUI action, user data, or
user logs are involved in this completion-only delta.
