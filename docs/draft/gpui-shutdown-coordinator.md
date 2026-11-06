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

### Termination guard threat model

The guard prevents accidental or convenience termination that bypasses the
shutdown coordinator in first-party code. It is a source-level policy check,
not a sandbox. Coverage combines `scripts/check_restart_policy.py`, the shared
Rust source/inclusion inventory, and Clippy's resolved `disallowed_methods`:

- Reserved restart identifiers, the sole adapter `request_restart` call,
  process-plugin Cargo aliases, numeric runtime exit arguments, exact
  function-scoped lint exceptions, and rejection of nested Clippy overrides.
- Resolved std/libc exit and abort, Unix `CommandExt::exec`, Tauri
  AppHandle exit/restart/request_restart, Tauri process restart, and original
  process-plugin init/exit/restart methods, including import aliases, grouped
  imports, method references and macro forwarding.
- Foreign declarations with any literal or default ABI for `exit`, `_exit`,
  `_Exit`, `abort`, `quick_exit`, `exit_group`, `pthread_exit`, `kill` and `raise`;
  terminating linker attributes and unprovable linker aliases. Foreign function
  names or ABIs supplied by macro metavariables fail closed. Conservatively,
  every `extern` token inside a `macro_rules!` definition is rejected, including
  benign `extern "C" fn` callback definitions, so forwarding names through
  invocation arguments cannot evade the check. Macros that never mention
  `extern` remain accepted.
- Exact termination-symbol string, byte-string and C-string literals, including
  raw forms and an optional trailing NUL, anywhere in scanned repository Rust
  sources. This covers literal `dlsym` and `libloading::get` lookups regardless
  of the lookup function's spelling. The sole literal exception is the exact
  `"exit" => app.exit(0),` action arm of the isolated
  `shutdown::test_support::shutdown_test_action` handler; other literals and
  lookups in that function are still checked. Comments, longer strings such as
  `"exit_code"`/`"abort_reason"`, and log messages containing exit remain accepted.

Deliberately obfuscated termination is out of scope: inline asm syscalls,
build-script-generated code, computed symbol names, signals to self via
non-literal arguments, and third-party crate internals. Code review covers those.
Fixtures are checked or compiled only; termination code is never executed.

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

## Round-three tray ownership correction

Disable Murmur now owns a native `disabled` patch. The tray callback acquires a
shutdown writer permit before spawning; the source FIFO resolves each toggle,
commits through `commit_settings_intent`, advances `ownedFieldsRevision`, and
sends the existing owned-field notice. Runtime override reservation remains
before publication. Failed publication retains native ownership for recovery.
Neither correctness nor shutdown recovery needs a renderer echo/flush.

The hook caches `app-disabled-changed` without persisting another intent. The
owned-field notice uses `rebasePendingOwnedSettings` to retire a replaced disable
patch while preserving independently pending Auto Paste edits. Stale snapshots
and duplicate field IDs cannot undo the native value. Runtime synchronization
via `set_app_disabled`, `configure_dictation_control` and setup resume joins the
source FIFO and resolves owned values under the writer lock, including retained patches.
Overlay disable applies runtime state at its native patch boundary. No commands
were added/removed; the count remains 186.

Audited paths: tray; native disabled/control/configure commands; main Settings
and rollback; command palette/hotkeys through `updateSettings`; overlay dropdown,
quick controls and reconciliation; initialization/external mirrors; onboarding;
legacy import and hydration/migration; generic source writes; backup restore;
per-app/mode delivery overrides. Main/overlay edits already carry deliberate
patches. Import fills absent sources, generic writes reject settings, and restore
requires a missing/quarantined destination and a complete supplied backup.

Synthetic regressions use the actual FIFO, merge, publisher, source recovery and
completion shutdown receipt. They cover no-renderer recovery, echo/stale/retried
snapshots, runtime rebasing, publication failure and rapid toggles. Vitest covers
the cache-only hook and retained-field rebasing. The hook regression fails with
the exact `b2155379` hook and passes after restoration. P1-A/P1-B, round-two and
completion durability coverage remains. No GUI, merge or retargeting is performed;
the parent retains the independent Sol recheck.

Verification: TypeScript and all 183 Vitest files / 1,894 tests pass; all 201
core unit/integration tests pass. On Mac mini with Rust 1.98.1, 31 settings-store
and 12 shutdown library tests plus the windowless integration executables pass;
all-target clippy passes with only the two requested allowances. Restoring the
missing tray owned patch makes the native recovery regression fail with
`disabled=false`; restoring the patch passes. Command/link, workflow, capture,
core-dependency, restart and event policies pass. The isolated Mac checkout and
target are removed after verification.

## Round-four tray ownership without readable settings

Tray Disable/Enable resolves against the current runtime value (or a retained
native patch) inside the source FIFO. Acceptance retains the owned `disabled`
patch and applies the configuration-fenced runtime override **before** reading,
merging or publishing settings. This also applies to deliberate Settings and
overlay disabled edits. An absent initial document, unreadable destination or
quarantine cannot prevent the runtime action. The native adapter updates the
tray checkmark and emits the existing app-state event even if publication fails.
Disabling an active starting/recording dictation schedules cancellation with its
captured recording generation; a delayed continuation cannot cancel a newer
recording after re-enable. Processing is allowed to finish.

A tray failure publishes the existing user-visible dictation-control error. Its
log says that the update was not fully committed and durability is not
guaranteed; it no longer promises that source recovery retained an edit before
acceptance. Runtime truth and durable success are separate: an unsavable choice
remains owned in memory and shutdown reports an uncommitted source fence.
Subsequent legitimate first hydration, return of the original destination, or
explicit complete-backup restore lets the normal merge/publisher persist that
patch. No initial settings document is invented by a tray patch, and damaged
or quarantined evidence is never overwritten or repaired by a toggle.

Tray ownership is an **orderly-shutdown guarantee**, conditional on the source
being readable/publishable by the final source fence. A forced process death
after runtime acceptance but before the publisher retains its durable receipt
can recover the previous value. Closing that window would require a separate
crash-durable patch journal, including replay/identity and quarantine rules;
that is broader than this correction and would add another settings writer.
The existing full-sync publisher and recovery protocol remain unchanged.

Synthetic regressions exercise the actual source FIFO, per-session selection,
owned-field merge, full-sync publisher, source recovery, completion shutdown
receipt and configuration fence. They cover first-launch hydration after one
or two tray clicks; missing, unreadable and quarantined destinations; symmetric
tray and Settings re-enable; unchanged evidence bytes; and generation-bound
recording cancellation. No commands were added or removed (186 total). The draft
PR stays stacked and is neither merged nor retargeted.

Verification: TypeScript and all 184 Vitest files / 1,896 tests pass; all 209
core unit/integration tests pass. Mac mini (Rust 1.98.1) passes 159 selected
settings-store/shutdown/keyboard/dictation-control tests with one test thread,
both default and `shutdown-test-support`; all-target clippy passes in both
configurations with the two requested allowances. All 11 restart-clippy
fixtures, restart/event/frontend/core-dependency policies, rustfmt and diff
checks pass. Reinstating the `f0b53497` tray/commit function bodies (with only an
unused runtime argument for test-call compatibility) makes each new storage
regression fail because runtime remains enabled; restoring the fix passes both.
Only synthetic fixtures were used; no GUI or app was launched. The isolated
Mac checkout/target are removed after verification. Independent Sol re-review
remains with the parent task.

## Round-four completion numeric stability corrections

This completion-only correction addresses P2-1/P2-2 from the round-three Sol
re-check. The draft stack keeps its existing base and must not be merged.
Independent Sol re-check remains with the parent task; this implementation
worker does not claim that review.

- **Integrity binds stored bytes:** the existing schema-1 batch envelope and
  SHA-256 format are preserved. Publication hashes the serialized `snapshots`
  bytes embedded in the envelope. Recovery and live retries locate that exact
  JSON member in the file and hash its original bytes, without reserializing
  parsed values. Whole-batch shape, target ordering, future-schema guards and
  quarantine rules still apply. Duplicate envelope keys are rejected. Even a
  valid-JSON numeric alteration that rounds to the same binary float is detected;
  damaged evidence is preserved and blocks recovery without changing readable
  History.
- **Float-stable storage and identity:** completion storage reads and captured
  failed writes use a local JSON decoder that retains serde's syntax, type and
  duplicate-struct-field checks, restores floats with Rust's correctly rounded
  decimal parser, and preserves integer types, strings and signed zero. Full
  canonical serialized content determines UUID replay equality. Exact replays
  deduplicate; changed content, including adjacent float values and opposite
  signed zero, returns `IdentityConflict` without acquiring retry ownership.
  Independent identical History completions still receive different UUIDs.
  No dependency feature is enabled: neither Cargo manifest nor serde_json feature
  unification across the app/workspace changes. No public command or caller
  signature changes.

Before the fix, all four initial round-four tests failed on `b2155379`: the
reviewer's `1.5287289387243241e-9` History metadata/Pin probe returned `Invalid`,
the `sharedInitMs = 1.0000226757369615` exact report replay returned
`IdentityConflict`, fractional-duration completion reconciliation after a
post-unlink directory-sync failure returned `UnpreservedFailure`, and the
property-style storage/mutation sweep failed to commit.

The corrected sweep exercises **80 finite f64 values in both migrated and
Holding modes** (160 scenarios): subnormals, the smallest normal and its
predecessor, 1e-9-scale numbers, values around 2^53, both signed zeros, many-digit
fractions, finite extremes and 64 deterministic generated bit patterns. Each
scenario submits and replays reports, rejects changed-content collisions,
submits identical independent History completions, pins through a live batch
retry, edits through disk-only batch recovery, deletes and restarts, checking
exact float bits and terminal receipts. A separate 4,096-pattern decoder sweep
checks serialized bytes and binary bits, with integer boundaries, escaped
strings, malformed JSON and duplicate envelope/struct fields. A live/restart
damage probe changes a numeric spelling without changing its f64 and verifies
quarantine bytes and fail-closed recovery.

Linux validation passes all **174 unit tests and 35 integration tests**, including
both reviewer probes, fractional post-unlink reconciliation, the numeric sweeps,
the unchanged 36-case exit matrix, both FIFO bounds and D2/D4/D5. All-target
clippy passes with `-D warnings -A clippy::byte_char_slices` on rustc/clippy
1.93.1. The exact requested command first failed because this toolchain does not
recognize `clippy::chunks_exact_to_as_chunks`; only that unsupported exemption
was omitted for the passing run. Rustfmt checks and `git diff --check` pass.
All test data is synthetic and temporary test roots use `~/.cache/`. No app
caller changes, Mac checkout, GUI action or app launch were needed.

## Round-three isolation, telemetry and guard corrections

- **P1, clipboard/input isolation:** shared clipboard reads, writes and change
  counts now require initialized production identity. All other identities use a
  private, identity-named NSPasteboard; the naming includes an absent-identity
  discriminator and injective UTF-8 encoding. Snippets and transform apply/undo
  share those primitives. A QA-only WebKit plugin routes application Copy/read
  APIs through the same native primitives before frontend startup, in every
  frame (`plugin:qa-clipboard|read_text` / `write_text`), and never falls back
  to the general clipboard. Its native dispatch also refuses production or
  unproven identities; unsupported item types and missing IPC fail closed. Synthetic Vitest tests prove Copy/read routing
  and refusal without an OS clipboard. QA cannot auto-paste, activate a Smart
  destination or post keys to another app. There is no unproven target exemption.
  Production
  retains its existing arboard/general-pasteboard route and delivery policy.
  Pure routing tests cover production, absent/empty, shutdown-test, bench,
  refactor-test, future and path-like IDs, without touching any OS clipboard.
- **P2, resolved process guard:** root `clippy.toml` disallows process exit/abort,
  libc termination, Unix exec replacement, Tauri AppHandle exit/restart and the
  process-plugin exit/restart methods. The existing Python guard still enforces
  numeric runtime exit codes, the sole request_restart adapter and plugin/Cargo
  aliases. It now enforces function-scoped Clippy allowances at an explicit
  CLI/helper/watchdog/adapter and pre-runtime generated-context allowlist,
  rejects broader lint suppressions and nested config overrides, and permits
  unrelated bare functions named exit.
  Compile-only fixtures exercise grouped std/libc aliases, macro-forwarded exit,
  direct zero exit outside CLI, abort, Tauri methods and the unrelated-function
  positive control. They never execute termination code.
- **P2, telemetry drain:** the worker handle and sender are retained. Writers
  starts an independent waiter alongside source/completion draining. It retains
  final owner events until those writers finish or the existing +10 s absolute
  deadline, then seals and flushes with only the remaining budget. A blocked
  content owner cannot postpone telemetry accounting. The +5/+7/+10 s driver never
  waits on the disk worker. A worker content-free stderr receipt accounts for
  pending, failed and rejected records even when startup history remains stalled.
  Tests cover timeout accounting, final owner events, and successful early-event
  drain after release.
- **P2, Clear precedence:** history restoration receives the generation captured
  at schedule time. A Clear before the worker starts, or during its read, wins.
  Clear IPC invalidates the in-memory generation before waiting for disk and
  does its filesystem work off main. A new regression covers Clear-before-start.
- **N2, startup dispatch:** completion initialization, legacy cleanup,
  capture-health replay, performance/diagnostics, knowledge and meetings run on
  retained startup workers. Completion readiness/initial IPC do not block main;
  recording remains gated. Capture-health and repository initialization do not
  hold main-visible locks during file I/O. Capture-health retains live events
  after restored history, avoids replaying current-instance events twice, and
  honors Clear during its stalled read. Synthetic tests prove live observations
  remain available and the quit adapter can issue termination at +10 s while a
  startup reader remains blocked. DEVELOPMENT documents the required native
  main-thread registration work and its OS-controlled timing boundary.

The identity audit now includes shared clipboard/pasteboard access and OS-managed
crash reports, explicitly qualified as having no application-owned crash reader.
No base-branch change, GUI launch, permission grant, production clipboard access
or merge is part of this worker's scope. Independent Sol rechecks remain with
the parent implementation task.

Verification at implementation head `f34f8ff2` (including the concurrent
tray-ownership and numeric-integrity corrections through `b87a9c93`):

- Linux: all **209 core tests** pass (174 unit and 35 integration). TypeScript
  and all **184 Vitest files / 1,896 tests** pass, including the two private
  WebKit clipboard regressions. Two unchanged presentation failure-storm tests
  timed out under the initial concurrent load; their 13-test file passes in
  isolation and the complete suite passes with `--maxWorkers=4`, without
  changing timeouts or skipping cases. All 14 restart policy fixture groups pass.
  Core dependency, targeted Rust-event, capture-boundary, reference/link and
  workspace formatting checks pass; the reference count remains 186 because
  the QA-only bridge is a separate plugin and does not change the app handler.
- Mac mini, Rust 1.98.1: full `cargo test -- --test-threads=1` passes both without
  and with `shutdown-test-support`: each has **1,552 passing library tests**, 14
  existing ignored library tests, passing integration suites and the windowless
  main-dispatch/AppKit shutdown executables. Existing hardware integration
  ignores remain unchanged. Both `cargo clippy --workspace --all-targets` runs
  pass with `-D warnings` and only the requested command-line allowances
  `-A clippy::chunks_exact_to_as_chunks -A clippy::byte_char_slices`.
- All **11 compile-only Clippy fixtures** pass on the Mac: the ten terminating
  cases are actually rejected by `clippy::disallowed_methods`; the unrelated
  function named `exit` compiles. This includes grouped std/libc aliases,
  macro-forwarded exit, direct zero exit outside CLI, abort, AppHandle methods
  and the original process-plugin init alias. No fixture is executed.
- SHA-256 comparison confirms **350 native build input files** match this
  worktree. Regenerated schema outputs also confirm the QA bridge exposes only
  its two text commands to local app windows. The routing tests use mocks and
  pure identity selectors; no OS pasteboard is read or written by those tests.

Native checks used only the new `~/Builds/gpui-stack-22` checkout, external
`~/Builds/gpui-stack-22-target`, and task cache paths. The helper preparation
script's default-output assumption was accommodated inside that disposable
checkout; application checks retained the requested external Cargo target.
No GUI, app launch, OS permission grant, production data access or merge was
performed. Independent Sol review and CI integration remain with the parent.

Cleanup verified: the disposable Mac checkout, external target and synthetic
runtime/cache scratch were removed after all checks; no task process remained.
Shared models, production data, other checkouts and shared caches were preserved.

## Round-four isolation, startup and guard corrections

This delta addresses the four remaining isolation re-check P2s on the existing
layer-07 draft. It keeps the stack base, tray corrections through `9676fc4b`,
Clear/capture-agent refusal fixes and round-four completion behavior intact.
Independent Sol re-check remains with the parent; no merge or retargeting is
part of this implementation worker's work.

- **QA clipboard ACL:** the private clipboard capability is inline only in
  `tauri.shutdown-test.conf.json`, outside the production auto-loaded capability
  directory. Both the native plugin and its build-time permission manifest are
  compiled only with `shutdown-test-support`. Production generated schemas omit
  the QA permissions. The generated-context regression resolves both commands
  for all four actual window labels in each build: denied in production,
  allowed with the QA feature/config. The native Copy seam writes a synthetic
  string to a unique identity-named NSPasteboard, reads it back, then clears
  that private board. It has no general-pasteboard path. Existing WebKit Copy/read
  tests retain their fail-closed IPC routing assertions.
- **Knowledge startup ordering:** refreshes reserve a process-local generation
  before SQLite reads and compare it under the same publication lock afterward.
  A stale read cannot replace entries, rebuild a matcher or bump the settings
  revision after a newer refresh. Entries/matcher publication also shares the
  dictation lock with settings rebuilds and recording snapshots. Recording and
  silent first-tap preparation refuse admission until a matcher has actually
  been published; startup work never blocks window setup. Failed initialization
  stays refused until a successful retry. Native and toolbar controls report that rules
  are still loading and points to the knowledge-store retry. Deterministic channel
  regressions stall the startup read, publish a newer synthetic edit, release
  the old read, and check matcher identity/output and revision. A separate case
  proves admission remains closed during the stall and after a failed read,
  opens only after publication, and retains an immutable recording matcher.
- **Telemetry outcome:** the Writers adapter consumes the bounded telemetry
  drain result; failure, join failure or timeout changes the receipt to
  `UnpreservedFailure`, preserving successful content-owner classifications only
  when telemetry drains. The existing independent deadline driver remains
  unchanged. A real queued synthetic event is lost under an injected file-create
  failure after rotation: `pending=0`, `failed=1`, drain=false. Feeding that result
  through the actual writer-outcome combiner and coordinator suppresses restart.
  The stalled-worker regression checks the same outcome and returns within its
  short timeout before releasing the worker; success retains both Committed and
  PreservedForReplay outcomes.
- **Foreign termination guard:** the lexer-aware source policy rejects foreign
  declarations for exit, `_exit`, `_Exit`, abort, quick_exit, kill, raise,
  pthread_exit and exit_group with any ABI, plus terminating linker attribute
  values (`link_name`, including conditional/raw/escaped forms, and `link`).
  Unprovable macro/constant linker aliases fail closed. Ordinary externs,
  comments/strings and unrelated Rust functions named exit remain accepted.
  Eight static fixtures supplement the original eleven compile-only Clippy
  fixtures and Python regressions. CI's `rust-lint` job now explicitly runs both
  the textual guard and the fixture runner; none of the termination fixtures
  are executed.

Verification uses synthetic data only, no GUI/app launch and no production
application/profile, WebKit directory or general pasteboard. The Mac checkout
and target are the task-owned `~/Builds/gpui-stack-26` and
`~/Builds/gpui-stack-26-target`; its home and temporary directory are isolated
under `~/.cache/gpui-stack-26`. Direct Cargo QA checks require both the feature
and `TAURI_CONFIG` populated from the QA config, as documented in DEVELOPMENT.

Linux verification: TypeScript and all **184 Vitest files / 1,897 tests** pass;
all **209 core unit/integration tests** pass. The restart policy passes **17
Python tests**, preserving all fourteen previous cases. All five new terminating
foreign fixtures are accepted by the actual pre-delta guard at `9676fc4b` and
rejected by the corrected guard; all three benign extern fixtures pass. Reference
(186 commands), Markdown links, workflow, core-dependency, formatting and diff
checks pass. The actual default Mac build regenerated all four production schema
files byte-identically to their tracked versions; none contains `qa-clipboard`.

Mac mini verification, Rust **1.98.1**: full default and QA-feature/config
`cargo test -- --test-threads=1` runs each pass **1,561 library tests** with the
same fourteen existing ignored tests, plus their integration suites and windowless
main-dispatch/AppKit executables. Both `cargo clippy --workspace --all-targets`
runs pass with `-D warnings` and only the two requested allowances. All **11
compile-only Clippy fixtures** and **8 foreign-source fixtures** pass. SHA-256
comparison confirms **451 native/guard input files** match this worktree.
The generated runtime ACL checks and native private-board Copy test pass in both
configurations; production schema output excludes the QA plugin, while the QA
permission manifest exposes it. The generated directory capability inventory
naturally omits inline capabilities; the runtime ACL regression verifies the
actual QA grants. The final frontend readiness-message regression also passes.

The task's Mac checkout, external target, synthetic home/temp/cache and generated
helpers were removed after preserving check receipts. No task process remained.
No product app was launched, no production data or general pasteboard was accessed,
and no merge, retargeting or independent-review pass is claimed.

### Round-five tray durability warning

The native pending disabled patch is now queryable through the main-only,
read-only `get_dictation_settings_unsaved` command. Its lifetime follows the
existing native ownership: failed writes retain the latest choice, a failed
superseding choice still warns, and successful publication (including native
retry or an unrelated settings write that merges the patch) clears it.
Settings producers invalidate the warning after success or failure.

`DictationSettingsNotice` subscribes before querying on mount and ignores older
query responses. Both hydrated main and startup `SourceRecovery` mount it,
using the existing `MainErrorBanner` and content-free fixed messages. Its
window-level placement stays below native chrome and separate from the ordinary
workspace recording/delivery error banner. A tray failure before hydration therefore survives either startup outcome. Recording
status transitions cannot erase this separate notice. No GUI or app launches
were used; all verification uses synthetic data.

The two pre-hydration regressions fail on `9676fc4b` and pass with this change;
six frontend cases also cover recovery-time changes and late initial reads.
The Rust lifecycle case covers retention, failed retry, failed supersession,
merged successful publication, saved tray supersession, and native retry.

Verification after rebasing onto isolation/startup/guard work through `58b44b39`:
TypeScript and all 185 Vitest files / 1,903 tests pass; all 209 core tests pass.
On the Mac mini with Rust 1.98.1, settings-store (34), shutdown (16), and
dictation-control (7) tests pass single-threaded in both default and
`shutdown-test-support` configurations. Workspace/all-target strict Clippy passes
in both with only the two requested command-line allowances. All 345 native
input files matched this checkout. Reference/command-count, targeted-event,
core-dependency, capture-boundary, restart-policy and formatting checks pass.
Regenerated schemas were restored; `gpui-stack-27`, its external target and
synthetic temp data were removed. Independent Sol recheck remains with the parent.

### Round-six warning reconciliation

Retained owned fields now record their current intent's candidate publication
revision. Recovery and publication retire only fields matching both that revision
and value under the source writer lock. A newer edit invalidates the old proof,
including repeated boolean values; unrelated fields reconcile independently.
The warning remains while the latest disabled choice is undurable.

Reconciliation covers hydration, runtime rebasing, later saves, explicit backup
recovery, WebKit status/import/onboarding writes, shutdown, and lifecycle retries
during unrelated completion commands or native producers. Consumers receive
warning invalidation after the lock releases, even when another source fails.
Shutdown does not add a revision when replay already committed the choice.

All five synthetic Rust regressions fail against the `25a573a4` behavior with
transport/publisher test seams extracted, and pass with the fix. They use the
real core publisher and exact receipt replay. TypeScript, 1,903 frontend tests
and all 209 core tests pass. Mac mini / Rust 1.98.1 passes settings-store (39),
shutdown (17), dictation-control (7) and completion adapter (2) tests with one
test thread, plus workspace/all-target strict Clippy, in both default and
`shutdown-test-support` configurations with only the two requested allowances.
All 427 tracked native inputs match this checkout. Schemas were restored and
`gpui-stack-29`, its target and synthetic temp data were removed. No GUI or app
launches were used. Independent Sol recheck remains with the parent.

### Round-seven lifecycle publication ordering

The completion owner now acquires a Tauri-free core `SOURCE_WRITE_LOCK` for the
entire lifecycle recovery/read/merge/publication critical section. The host's
`settings_store::WRITE_LOCK` is an alias to that same mutex. This includes
onboarding saves retried by unrelated completion commands, native producers,
barriers and shutdown recovery. Observing publication afterward still reconciles
round-six ownership and warning state, but publication itself is now serialized.
A tray or Settings writer cannot commit between a lifecycle read and its receipt
publication. Hydration, restore, source recovery and host settings/theme writers
continue using the same lock and full-sync publisher.

File-backed settings/theme snapshots are rejected by lifecycle `Save` before
FIFO acceptance, matching the host command's existing restriction. They must use
the host's source-ordered merge API. The obsolete raw `save_settings` API had no
production callers and is removed; its sole test uses the shared publisher.
This closes a second way to retain a stale
full snapshot outside P1-A/P1-B ownership fences. Import remains fill-only against
current durable sources; the onboarding operation patches current settings under
the lock and preserves the onboarding gate. The existing source-recovery safety
fixtures use the host publisher for file creation and interrupted publication.

Lock order is explicit: source FIFO/order mutex (host writers only) -> shared
source write mutex -> retained owned-field mutex. The completion actor takes the
shared source write mutex inside `lifecycle_inner`; it never acquires the host
source FIFO/order mutex or retained-field mutex. Admission enqueues and releases
its mutex before waiting. Host status/import/save and final shutdown drain wait
for the actor **without** the shared source write mutex; afterward they reacquire
it only for reconciliation, including on errors. No host path waits for or joins
the actor while holding that mutex, so there is no write-lock-to-actor edge to
complete a cycle. Absent-WebKit initialization uses a locked recursive helper
rather than acquiring the same mutex twice. Shutdown first drains accepted host
writers, recovers their sources, then seals/drains completion without the lock;
the retained actor may finish safely if the coordinator deadline stops waiting.

Synthetic deterministic regressions retain an actual interrupted re-enable
receipt, retain an onboarding save behind a failed UI-state read, then pause its
retry at the existing test-only receipt `BeforeRename` hook after settings
recovery/read. A competing host writer probes the same mutex: with the defect it
publishes revision two before the retry resumes and is overwritten by revision
one; with the fix it waits and publishes the final revision two. Variants cover
tray Disable, Auto Paste, a Settings language edit, native dictation production
(including duplicate submission), and shutdown recovery. The existing warning,
source-order, tray and completion durability coverage remains. A native adapter
test checks both lifecycle and shutdown reconciliation finish within a bounded
wait, catching reintroduced lock inversion. No GUI, app launch, or production
data is used.

Before/after: the four requested interleavings all fail on actual `b0ffc7ba`
source with `disabled=false` after the newer host success. The only baseline
additions are the regression module and its test-only host mutex declaration;
no baseline publication behavior is changed. All four and the producer variant
pass after the fix. Verification results follow below.

Verification: TypeScript and all 1,903 frontend tests pass. All 215 core unit
and integration tests pass, including round-four numeric identity, the 36-case
process-exit matrix and retained FIFO bounds. All six new core regressions pass
on both Linux and Mac mini; the five race variants fail on `b0ffc7ba`, and the
sixth confirms that baseline still accepts bypassing file snapshots. On Mac
mini / Rust 1.98.1, single-threaded native settings-store (39), shutdown (18),
completion (11) and dictation-control (7) selections pass in both default and
`shutdown-test-support` configurations. Workspace/all-target strict Clippy passes
in both, with only the two requested command-line allowances. The final removal
of the unused raw writer also passes its replacement test and both Clippy
configurations. All 11 resolved restart-Clippy fixtures and 18 foreign-source
fixtures pass. Core-dependency, restart, targeted-event, reference/link,
formatting and diff checks pass. Regenerated schemas were restored; all seven
changed inputs matched the Mac checkout before cleanup. Synthetic logs were
preserved, and `gpui-stack-30`, its target and synthetic scratch were removed.
No GUI or app was launched. Independent Sol recheck remains with the parent;
PR #1025 remains draft, stacked on its original base, unmerged and unretargeted.
