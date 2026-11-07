# GPUI 1.3: completion service

Part of #987. Stack 04, based on `gpui/03-dictation-control`; do not merge.

`murmur-core::completion` owns History, statistics, saved Performance Lab
reports and migration v1. A single app-lifetime actor serializes every command;
an exclusive advisory data-root lock rejects a second owner. Dropping its final
handle closes the queue, drains accepted work and joins the owner thread. The
host retains its runtime. The core neither creates windows nor depends on Tauri.

Native successful dictation, file transcription, meeting completion, correction
proposal/confirmation and Paste Last submit at their source. The actor claims
source identities within the app session; a view never submits a completion.
Live recording retains its immutable start-time History policy. File imports
write History only, after the main window accepts the result for its unique
operation UUID; native cancellation suppresses admission before start, during
transcription, and between the result and acceptance. Failed admission keeps the
staged result for retry with the same operation UUID; cancellation after failure
removes its content and prevents later admission. Pending running/ready file
operations are capped at 200. The client cancels abandoned operations on dismissal,
reset and unmount. Acceptance failures reuse the dictation storage diagnostic and
notice, and the existing file queue reports a content-free error. No transcript or
rejected error payload enters diagnostics. Activity events never change dictation counters.

`completion-state.json` contains the next sequence and typed content-free
statistics records with UUIDs and bounded content-free file-admission evidence. `stats.json` carries `applied_through` and all
counters, including legacy transform counters with no current producer. History
is never a statistics replay source. The journal is unbounded while migration
holds it; successful statistics publication allows compaction. Content files
are bounded to 32 MiB consistently on read and write.

The actor writes exclusive mode-0600 temporary siblings, syncs file contents
(`F_FULLFSYNC` on macOS), renames, and fsyncs the parent directory. Every command
recovers from disk and re-establishes directory durability before compacting or
draining; an unknown outcome never advances an in-memory attempted watermark.
Unsupported destination versions and malformed entries are quarantined locally
without content in diagnostics. State loss recovers its sequence from statistics;
holding loss recreates empty lists and records a content-free diagnostic.

History and report envelopes carry the imported marker, clear time, and consumed
or discarded holding generation. Statistics resets commit their watermark with
the empty counters. A clear is one destination write, so a failed second write
cannot resurrect held data. Exact legacy deletions retain id/text digests and
skip one matching legacy row. Holding caps cover the combined original/held view:
200 History entries, 20 pins, ten reports. Snapshots apply held statistics only
in memory and include the Retry notice after repeated uncommitted launches.

The main React startup importer reads only the four targeted legacy keys, sends
v1 to Rust, and removes each destination's keys after a durable receipt, then acknowledges
that removal in the migration file. Interrupted removal repeats on startup. Valid
disk wins; missing/unusable destinations recover from WebKit. Holding generations
drain after the destination commits. `migrations.json` is separate from whole
settings snapshots; losing it safely reruns marker-based decisions. No WebKit
directory commits v1 immediately. Native dictation admission waits for storage
and migration, reloads main once at five seconds, then permits held operation
at ten seconds. A storage initialization failure never permits arming.

React keeps only presentation snapshots, reads the two changed notifications,
and sends exact-row commands. No History/statistics/report writer caches data
in localStorage; both saved-report readers hydrate through the core. Theme
library/settings persistence remains in its existing owner. The standalone
objc2 WKWebView importer, migration v2/v3, and WebKit store retirement belong to
1.5/1.6 and are deliberately outside this change.

## Defined outcome coverage

Each plan row maps to an executable Rust test in
`app/src-tauri/crates/murmur-core/src/completion/tests.rs`.
Tests use temporary roots and synthetic data only. Fault injection runs at the
actual before-rename/after-rename boundaries; the power-loss test exercises both
possible published directory entries. It simulates storage outcomes and does
not claim a physical power-loss test.

| Plan case | Rust test |
|---|---|
| Crash after the journal append, before History | `crash_after_journal_before_history_replays_statistics_only` |
| Crash after History, before statistics | `crash_after_history_before_statistics_has_no_duplicate_history` |
| Crash after statistics, before compaction | `crash_after_statistics_before_compaction_does_not_reapply` |
| A meeting, a proposed correction, a taught correction, and a Paste Last, each with a crash before statistics | `activity_events_crash_before_statistics_match_uninterrupted_run` |
| Clock moved backwards; equal timestamps | `clock_backwards_and_equal_timestamps_do_not_change_counters` |
| Statistics write for event N fails, then event N+1 commits | `statistics_n_fails_then_n_plus_one_applies_both_in_order` |
| History write fails | `history_write_fails_statistics_apply_and_no_history_entry` |
| Statistics rename succeeds but the directory fsync fails, then retry | `statistics_unknown_outcome_rereads_and_does_not_compact_until_synced` |
| The same, with power lost before the retry | `power_loss_after_unknown_statistics_write_replays_once` |
| Statistics reset with records pending | `reset_with_pending_records_advances_watermark_in_one_write` |
| Clear History; clear benchmark reports | `clear_history_and_reports_write_explicit_empty_files` |
| Entry deletion; the 200-entry cap; `retainHistory=false` | `deletion_cap_and_retain_false_never_change_statistics` |
| State-file creation fails on first start | `state_first_creation_failure_blocks_all_writes_and_arming` |
| State file deleted or corrupted, before and after v1 commits | `state_deleted_corrupt_or_future_before_and_after_v1_recovers_sequence` |
| Holding file deleted or corrupted before v1 commits | `holding_deleted_corrupt_or_future_does_not_change_import_decision` |
| Stale React settings save while records are pending, then a restart | `stale_settings_save_does_not_touch_journal_or_migration` |
| Shape-valid but malformed disk History (for example `[null]`) or an unsupported version, with a valid WebKit copy | `malformed_history_or_future_schema_quarantined_then_webkit_recovers` |
| Unversioned statistics and legacy History entries on disk | `unversioned_statistics_and_legacy_history_are_valid_destinations` |
| Late v1 import with disk and WebKit holding the same data | `late_import_disk_wins_over_identical_webkit_counters` |
| Late v1 import with WebKit-only data | `late_import_webkit_only_is_below_held_and_added_once` |
| Valid disk statistics, migration timeout, new dictation, statistics file corrupted, relaunch, more dictation and a meeting, late v1 import | `valid_disk_statistics_corrupt_after_timeout_preserves_all_journal_events` |
| The same sequence for History | `original_history_corrupt_after_timeout_recovers_legacy_below_held` |
| Timeout on first launch, new dictation, quit, v1 commits on the next launch | `timeout_then_quit_then_next_launch_import_keeps_webkit_and_held` |
| Clear History, statistics reset, or clearing reports, before a late v1 import | `clear_before_late_import_skips_each_destination_and_removes_its_keys` |
| Crash after the clear's destination write, before the holding-file write | `crash_after_clear_destination_before_holding_drops_old_keeps_later` |
| A clear before v1, then the migration file moved aside | `clear_before_v1_then_migration_file_lost_stays_cleared` |
| Delete a held entry and an entry of the original History file, then corrupt the History file before a late v1 import | `deleted_held_and_original_entries_do_not_return_after_corruption` |
| Crash between a destination's import write and draining the holding file | `crash_after_import_destination_before_drain_adds_content_once` |
| Import write commits, the holding-file write fails, the user deletes an imported entry, relaunch | `failed_import_drain_then_user_delete_and_relaunch_does_not_resurrect` |
| Two legacy entries share an id; the user deletes one before v1; History file corrupted; late v1 import | `duplicate_legacy_ids_delete_skips_exactly_one_matching_text` |
| Repeated timeouts while new dictation and reports accumulate | `repeated_timeouts_cap_combined_history_pins_reports_and_show_retry_on_third_launch` |
| Fresh install with no WebKit data directory; relaunch | `fresh_install_without_webkit_commits_immediately_and_never_holds` |
| Migration file moved aside after the store is retired | `retired_store_migration_file_lost_recommits_without_changing_destinations` |
| Crash between a destination's import write and the v1 commit | `crash_after_destination_import_before_v1_commit_skips_marked_destinations` |
| Migration file moved aside before or after v1 commits | `migration_file_lost_before_or_after_commit_never_imports_twice` |
| Colliding legacy and new History ids | `colliding_legacy_and_new_ids_keep_both_entries` |
| Killed WebContent process during v1 | `killed_webcontent_reload_once_then_arm_holding_and_state_failure_never_arms` |

## Test port and exit check

Rust replacements passed before retiring client persistence assertions. Pure
search, formatting, export, derived-entry and statistics presentation helpers
retain their TypeScript coverage.

| Retired TypeScript ownership | Replacement |
|---|---|
| `history.test.ts` persistence block, `useHistoryManagement.test.tsx` storage assertions | Schema/quarantine, cap/pin/delete, clear, marker and holding tests above; client test now checks exact-row IPC and absence of WebKit writes. |
| `activityStats.test.ts` counter persistence/reset, `durableUserData.test.ts` ordering/restart | Activity crash replay, journal sequencing, reset watermark, prototype-label and legacy-counter tests; pure insight derivation tests remain. |
| `useLocalStats.test.tsx` producer subscriptions and de-duplication | Native producers plus headless completion/activity tests; client test checks that multiple readers subscribe to no producer. |
| `useRecordingState.test.tsx` completion writers; `useFileTranscription.test.tsx` History insertion | Headless completion and retention/file-statistics assertions; client tests assert those hooks cannot write counts or History. |
| `benchmark.test.ts` localStorage reports | Rust report schema/cap/import/clear tests; comparison retains session-import tests and reads a synthetic core snapshot. |

`headless_fake_host_can_arm_start_stop_cancel_autostop_and_paste` now attaches a
real completion actor through the same `completion::producers::Producers`
adapters used by native capture/delivery and activity sources. It covers successful
stop and auto-stop, cancellation, retention-off, file transcription, meeting,
taught correction and Paste Last; duplicate attachment adds nothing. Reopening
readers and relaunching the service preserve History and counters. Additional
actor tests cover concurrent producers, rejection of a second owner, exact-row
edits and pin limits, legacy WPM ordering and transform counters, report metadata,
UUID de-duplication, removal acknowledgements, and unknown journal outcomes.
Snapshot revisions prevent delayed IPC responses from replacing newer data.
A source-wiring contract also checks the native adapter callers. Temporarily
removing the shared dictation submission fails the effect-driven exit test;
removing its native caller fails the wiring contract. Neither mutation is kept.
Additional review regressions cover unreadable holding data, initialization fsync
ordering, no-WebKit initialization retries, edited-original tombstones, file
cancellation races and duplicate-ID client edits. The main-window importer test
checks actual legacy-key absence after each destination's durable clear receipt.

Run Linux core tests/Clippy, TypeScript/Vitest, core dependency and repository
reference/markdown policy validators. macOS full-crate and native observations
are recorded separately in the stack PR; headless effects do not establish OS
paste or native GUI behavior. Row 37 exercises the migration gate only; killing
a real WebContent process and observing its one reload remains a native check.

## File acceptance re-check

`file_acceptance_storage_failure_retains_result_for_retry_or_cancellation` replaces
each of the synthetic holding/state files with an unreadable directory. Failed
acceptance retains the result; restoring storage and accepting twice writes one
History entry and no statistics. Cancelling after failure admits nothing, including
on repeated acceptance, and cannot restart the cancelled identity. The regression
fails on `cfd20dd4`, which removes the staged result before submission.
`file_staging_is_bounded_and_cancellation_releases_capacity` verifies admission
backpressure and capacity release without evicting a retryable result.

The client regression also fails on `cfd20dd4`: rejection leaves the item marked
Done. It now reports the existing queue error, ignores the rejected payload, and
covers cancellation after failure plus dismissal/reset/unmount cleanup. Acceptance
still starts in the same turn as result publication and the cancellation check,
without opening a cancellable UI gap during native admission.

Dictation, meeting, correction and Paste Last adapters and native callers were
inspected: none removes a staged completion before submission. Their existing
journal/source-claim handling and failed-History-write policy remain unchanged.

The later publication-failure re-check found that `Complete` claimed file sources
before History publication and suppressed its error. File acceptance now uses a
separate `AcceptFile` command: staging assigns a stable completion UUID; the actor
claims the source only after the History or holding write is durable. Publication
errors reach the existing host `completion_write_failed` diagnostic and
`completion-storage-error` notice, without content. The producer keeps its staged
result and the same UUID on failure. No actor-side deferred retry can admit a
cancelled result. The state machine below supersedes the shared-mutex
cancellation protocol: intent now reaches the owner independently of staging.
The 200-operation staging cap is unchanged.

Every retry first re-establishes directory durability, then rereads the atomic
UUID receipts in History and holding envelopes. An unknown outcome with a receipt
is acknowledged without another append, even if its row was subsequently removed;
a failed directory sync still returns an error and retains staging. A pre-publication cancellation prevents later retries
and migration from admitting the result. Cancellation after successful publication
keeps the already-admitted entry, as before.

Three production-adapter regressions fail on `043f4b10` and pass with the fix:

- `file_history_publication_failure_retains_result_and_retries_once` injects
  `BeforeRename`, checks the surfaced error and retained staging, then retries to
  exactly one History entry with no statistics.
- `file_history_unknown_publication_rereads_before_retry_without_duplicates`
  injects `AfterRename`, fails the next directory sync, and verifies that recovery
  acknowledges the reread UUID without republishing or duplicating it.
- `file_history_cancel_after_publication_failure_never_admits_on_retry` cancels
  after `BeforeRename` failure and verifies that repeated acceptance and relaunch
  admit nothing.

Each runs against both `history.json` and pre-v1 `completion-holding.json`, and
checks migration/relaunch plus unchanged statistics. Dictation continues to use
its durable statistics journal as the commit point: a History publication failure
records `history_write_failed`, applies statistics, and adds no History entry
(plan row 7). Meeting, correction and Paste Last also use that journal: a failed
pre-rename append leaves the source unclaimed; an unknown append claims it only
if a reread finds its UUID. Recovery must sync before applying or compacting those
records. These producers have no staged History-only result to discard. Their
journal/source-claim behavior is unchanged.


## File acceptance state machine and interleavings

One file operation is identified by its staging UUID. The following states are
logical states of that operation, derived from staging, cancellation intent and
durable receipt together; no attempted in-memory write is treated as durability.
**Staged** has a ready result; **accepting** is queued, preparing or publishing;
**preparing** writes UUID-only evidence without publishing this file’s content;
**published-unknown** is a returned error whose publication may have renamed;
**admitted** has a synced receipt; **cancelled** has intent and no new publication;
**failed-retryable** has a proven pre-rename failure; **uncertain** has an unknown
publication without a surviving receipt or admitted evidence. Running capture
precedes staged and is covered by the existing cancellation/cap tests.

Admission uses a UUID-only write-ahead record in `completion-state.json`, with
an `admitted` flag, before publishing transcript content. The History/holding
publication still atomically includes its UUID receipt. Recovery syncs first,
then promotes any matching pending record to admitted **before** draining a
holding generation. Delete, clear, edits, caps and migration preserve state
records. This places admission evidence in the existing recovery-critical journal
instead of trusting a losable holding file alone; no transcript is added to the
journal and no new durable file or deletion-specific protocol is needed.

The live intent distinguishes PREPARING from ATTEMPTED. Writing pending evidence
sets only PREPARING; the publication CAS replaces it with ATTEMPTED/PUBLISHING.
A preparation failure at any boundary leaves the transcript staged and retryable.
The next command syncs the directory and rereads state, resolving an unknown
state rename before reusing or recreating that UUID's pending record. It never
adds a second pending record. A proven non-publication returns to PREPARING
**before** evidence cleanup, so failed cleanup cannot strand an unpublished file.
A pre-v1 cap write changes only old History rows; even its unknown rename cannot
have published the new file, which goes only into the subsequent holding write.
A content post-rename failure keeps pending evidence: a synced receipt proves
admission, but an absent receipt cannot prove non-publication. It stays uncertain
and acceptance returns the content-free storage error/notice without appending.
The staged intent also remembers that publication was attempted, so loss of the
state evidence cannot turn that same live operation into a fresh admission.
Admitted evidence acknowledges retry without touching rows, even after receipt
loss. State loss retains the plan's diagnostic/reconstruction behavior; matching
surviving receipts still prove admission. Loss of all evidence for a live attempted
operation stays uncertain. Production relaunch abandons staging and never retries
it; tests additionally preserve the synthetic staging/intent across actor relaunch.

Only acceptance consults an operation's unresolved evidence. Snapshot, migration,
dictation, meetings, corrections and Paste Last continue independently. Missing or
unusable files recover as the plan specifies. Read errors still block actual
storage access. Every acceptance compacts state evidence and envelope receipts
against the sole producer's complete live ready UUID set, at most 200; Snapshot
and launch never discard live evidence. Normal statistics compaction preserves
these fields. Unknown evidence is retired only when its staged ownership ends.

The intent registry has explicit active ownership plus the most recent 200
content-free operation-ID tombstones. Begin creates ownership; acceptance,
completed cancellation and discard retire it. A cancelled running native job pins
its intent until finish/discard, preventing a delayed finish from finding a new
intent after tombstone eviction. Running ownership, including cancelled jobs not
yet finished, counts toward the 200-operation cap. Ready cancellation releases
capacity and drops the staged transcript; queued accepts retain the original `Arc`.
Staging is memory-only; no transcript-bearing staging file needs deletion. Operation UUIDs are never
reused; older retired IDs leaving the bounded window cannot have native work
remaining. Unknown cancel IDs create bounded tombstones, not active intents.

Cancellation records an atomic intent without waiting for the files mutex or owner. The
actor's compare-and-exchange immediately before the first publication write is
the linearization point. Intent recorded earlier prevents publication. Once that
step starts, cancellation may discard staging but cannot undo a durable publish;
a pre-rename failure still leaves no entry, while a post-rename failure requires
a directory sync and disk reread. A cancel arriving after durable publication
leaves the entry in History. Cancellation never deletes History implicitly.

Lock order: `files` may briefly acquire the intent registry, then release it
before enqueueing/waiting on the owner. Cancel acquires/releases the registry,
marks the atomic intent and only **tries** `files`. Every production files guard
drains **all** cancelled operations on release: ready values are dropped and
retired, while running jobs become Cancelled and retain ownership until finish or
discard. The guard holds the registry through unlocking files, closing the race
between the last drain and a new cancellation: the canceller either joins that
drain or acquires free files and drains itself. It never waits for the owner or
files. Discard may wait for files after releasing the registry. The actor uses only atomics, never either producer mutex. Concurrent
accepts serialize on files; neither blocks cancellation dispatch. The root lock
remains actor-owned.

Preparation and publication have separate transitions (F is the 180-case fault
matrix below; V is the deferred-cancel capacity regression):

| Phase / event | Evidence and live intent | Result |
|---|---|---|
| Preparation write succeeds | Pending UUID durable; PREPARING | Publication CAS may proceed (F) |
| Preparation fails before rename | No content attempted; PREPARING | Retry after recovery, or cancel/drop staging (F) |
| Preparation fails after rename or directory fsync | Pending UUID may exist; PREPARING | Sync and reread state, then retry without uncertainty (F) |
| Content publication CAS | PREPARING becomes ATTEMPTED/PUBLISHING unless cancelled | Earlier cancellation prevents publication (Q/P) |
| Pre-v1 old-History cap write fails at any boundary | New content not published | Return to PREPARING; retry or cancel (F) |
| Content write fails before rename; cleanup write also fails | Pending UUID may remain; PREPARING restored before cleanup | Sync/reread, retry or cancel; never stuck uncertain (F) |
| Content write fails after rename or directory fsync | ATTEMPTED; possible content publication | Sync/reread receipts; acknowledge or remain uncertain only if evidence was lost (F/X) |
| Content succeeds; final admitted-state write fails | Content receipt survives; ADMITTED live intent | Recovery promotes pending evidence, acknowledges exactly once (F) |
| Cancel while another operation holds files | Atomic intent recorded; no files/owner wait | Guard drains all cancelled ready results when released (V) |
| Cancel running job during deferred drain | Cancelled native ownership remains | Finish/discard drops result and retires owner (O) |

The following table partitions the remaining events by state. Rename and fsync
failures in its accepting column refer to **content publication**; preparation
failures follow the table above. Test keys below apply to each
cell carrying that key. **—** means impossible: a publication callback belongs
only to an accepting operation. **=** means no file-state transition: History
mutations cannot themselves submit an acceptance (tested by the adjacent key).
For published-unknown, all commands first sync and reread; a failed sync changes
nothing and permits no mutation.

| Event | staged | accepting | published-unknown | admitted | cancelled | failed-retryable |
|---|---|---|---|---|---|---|
| accept | accepting (B) | serialized retry (Q) | recover, acknowledge or stay uncertain (U/X) | no append (B) | no append (C) | accepting (B) |
| retry | accepting (B) | serialized retry (Q) | recover, acknowledge or stay uncertain (U/X) | no append (B) | no append (C) | accepting (B) |
| cancel | cancelled (C) | intent wins before publication; otherwise finish write (Q/P) | discard staging, preserve any published row (L) | stays admitted (L) | stays cancelled (C) | cancelled (C) |
| publish ok | — | admitted, retire staging (B/P) | — | — | — | — |
| fail before rename | — | failed-retryable; retain staging unless cancelled (B/P) | — | — | — | — |
| fail after rename | — | published-unknown (U/P) | — | — | — | — |
| failed directory fsync | — | published-unknown (D) | stays unknown; no mutation (U) | — | — | — |
| recovery / Snapshot | = (B) | ordered behind acceptance (Q/P) | synced receipt -> admitted; absent after unknown publication -> uncertain (X) | = (L) | no new row (C/P) | = (B) |
| delete | = (D) | ordered before/after write (D/Q) | receipt survives, retry cannot resurrect (D) | =; row removed (D) | =; cannot admit (C/D) | =; still retryable (B/D) |
| clear | = (H) | ordered before/after write (H/Q) | receipt survives (H) | =; row removed (H) | =; cannot admit (C/H) | =; still retryable (B/H) |
| cap eviction | = (E) | ordered before/after write (E/Q) | receipt survives (E/K) | =; row may leave (E) | =; cannot admit (C/E) | =; still retryable (B/E) |
| edit | = (T) | ordered before/after write (T/Q) | receipt survives, edited text retained (T) | = (T) | =; cannot admit (C/T) | = (B/T) |
| relaunch | abandon memory (R) | drain accepted owner jobs before shutdown (R) | retain receipt, abandon memory; synthetic preserved retry is safe (R) | receipt/row retained (L/R) | never replay staging (C/R) | abandon memory (R) |
| unmount | cancel via native IPC (W) | same publication boundary (W/Q/P) | discard staging; durable row retained (W/L) | no implicit delete (W/L) | no-op (W/C) | discard staging (W/C) |
| WebContent loss | no automatic accept; bounded staging until process exit (K) | queued command finishes under native ownership (Q/P) | bounded staging/receipt; no automatic retry (K/U) | native receipt retained (R) | cannot admit (C) | bounded until cancel/exit (K/B) |

Test keys:

- B: `file_history_publication_failure_retains_result_and_retries_once` and
  `file_acceptance_storage_failure_retains_result_for_retry_or_cancellation`.
- U: `file_history_unknown_publication_rereads_before_retry_without_duplicates`.
- C: `file_history_cancel_after_publication_failure_never_admits_on_retry` and
  `file_cancellation_before_during_after_transcription_and_after_acceptance`.
- D/H/E/T: `file_receipt_delete_then_retry_never_resurrects`,
  `file_receipt_clear_then_retry_never_resurrects`,
  `file_receipt_cap_evict_then_retry_never_resurrects`, and
  `file_receipt_edit_then_retry_keeps_edit`. Each exercises History and pre-v1
  holding, both after-rename and directory-fsync failure, Snapshot, mutation,
  retry, migration and relaunch. They assert statistics stay unchanged.
- R: `file_receipt_relaunch_then_retry_never_resurrects` plus the existing
  owner-drain, fresh-install and migration relaunch coverage. Process exit drops
  transcript-bearing producer memory by Rust ownership; no extra UI test needed.
- Q: `file_queued_retries_cancel_before_publication_wins` blocks the actual owner,
  starts two production acceptance retries after a pre-rename failure, waits for
  cancellation intent, and only then releases the owner. Neither retry publishes.
- P: `file_cancel_during_publication_preserves_only_published_outcomes` pauses
  the actual write before rename, records cancellation, then releases success,
  before-rename failure or after-rename failure. No retry resurrects content.
- L: `file_cancel_after_durable_publish_keeps_history` covers lost response,
  Snapshot establishing durability, cancel and relaunch, without deleting the row.
- K: `file_receipts_compact_only_after_staging_retires` keeps an unknown staging
  UUID through 205 later admissions/evictions, verifying bounded UUID-only
  receipts and no resurrection. `file_staging_is_bounded_and_cancellation_releases_capacity`
  checks the 200-operation cap even without a client. WebContent loss cannot
  generate a native command by itself; no synthetic browser crash is needed for
  this ownership invariant (the real migration reload remains a native check).
- W: existing client rejection, dismissal, reset and unmount tests in
  `useFileTranscription.test.tsx`; Q/P/L exercise their native cancellation target.

Receipt-compaction failure is itself retryable: it changes no History rows and
staging remains intact. If compaction renamed before failing, recovery rereads it;
it removed only UUIDs whose staging had already retired. A power loss after a
file publication has the same two directory-entry outcomes as U/D: old envelope
means pending evidence and conservative uncertainty; new envelope means admitted
receipt and no duplicate. None of these paths changes dictation's row-7 journal commit,
the content-free storage-error notice, or activity producer semantics.

The delete, clear, cap-eviction and relaunch regressions and the queued-retry
cancellation regression all fail against `ffe2fcb2` with only the test pause/fault
seams added. The edit and cancel-after-durable-publication cases are preservation
controls: those outcomes already worked and must remain unchanged.


### Evidence recovery and native dispatch (fourth re-check)

The uncertain state retains staging until cancellation or process exit and never
retries a publication automatically. The following rows extend the transition
table above. X is `file_unknown_publication_loss_matrix_never_duplicates_or_resurrects`;
I is `file_uncertainty_isolated_from_all_other_producers`; O is
`file_intent_ownership_and_tombstones_are_bounded`; N is
`file_cancel_ipc_completes_while_accept_owner_is_blocked`.

| Event after unknown publication | Transition/outcome | Evidence |
|---|---|---|
| Holding receipt file deleted, corrupt or future schema before recovery | Recreate holding; pending journal evidence stays uncertain; no append | X, all three losses |
| Holding receipt file lost after migration, including after exact-row delete | Admitted journal evidence survives; acknowledge without resurrecting | X |
| History receipt file deleted, corrupt or future schema | Admitted evidence acknowledges; pending evidence with no receipt stays uncertain | X |
| State evidence file deleted, corrupt or future schema | Recreate per plan; surviving receipt proves admission; otherwise attempted live intent refuses append | X |
| State and both possible receipt files lost/corrupt/future | Same live operation stays uncertain, never authorizes append | X |
| Migration file lost/corrupt/future | Rerun marker-based import, preserving journal evidence | X |
| Unknown publication plus any loss above, then cancel and relaunch | Cancel drops staging; no replay, duplicate or resurrection | X |
| Uncertain file plus dictation/meeting/proposed/taught correction/Paste Last | Other producers still complete through their unchanged journal path | I |
| 1,000 completed/cancelled operations, plus a cancelled job with delayed finish | At most 200 tombstones plus live owners; delayed finish cannot admit | O |
| Accept awaiting owner while cancel arrives on the same IPC dispatcher | Async accept returns dispatch promptly; cancel completes before owner resumes | N |

The adversarial enumeration in X crosses History vs pre-v1 holding publication,
missing/corrupt/future files, loss before recovery vs import-and-delete first,
actor relaunch with preserved synthetic staging, retry, cancellation, migration,
and a second relaunch. The joint-loss case removes state **and both** receipt
stores. Storage errors never authorize another append. Content-free pending
records do not affect statistics or migration decisions (I). Existing U/P tests
also cover failed directory sync and cancellation during actual publication.
There is no automatic restoration of content lost under the plan's accepted
state/holding/destination loss rules.

Pinned sources inspected locally: `Cargo.lock` pins Tauri 2.11.2, tauri-macros
2.6.2 and Wry 0.55.1. The macro's
[`body_blocking` / `body_async`](https://docs.rs/crate/tauri-macros/2.6.2/source/src/command/wrapper.rs)
call synchronous commands inline, while async commands use
[`respond_async_serialized_inner`](https://docs.rs/crate/tauri/2.11.2/source/src/ipc/mod.rs),
which spawns the future. Wry's macOS
[`WryWebViewDelegate::did_receive`](https://docs.rs/crate/wry/0.55.1/source/src/wkwebview/class/wry_web_view_delegate.rs)
calls `ipc_handler` inline in the WKScriptMessageHandler callback. Accept is now
an async Tauri command and moves the blocking owner wait to `spawn_blocking`;
cancel remains a short synchronous command with no owner/files wait.

N uses `tauri::test::MockRuntime`, the actual registered command wrappers and
`Webview::on_message`, dispatching accept then cancel on one thread. A FIFO in
a synthetic temporary data root blocks the real core owner while it processes acceptance;
a watchdog releases it after five seconds so a synchronous regression fails
instead of hanging. The FIFO handshake proves acceptance holds staging and awaits its owner.
Cancellation must return successfully before that release,
and acceptance must eventually leave History empty. This proves production
Tauri wrapper dispatch and cancellation independence from the blocked owner. It
does not run WKWebView's Objective-C callback or establish native GUI behavior;
its inline behavior is established by the pinned source inspection above.

All producer fixtures used for exact-row mutation now include native
`schemaVersion: 2`, `source` and `pinned`. Delete/clear assert an empty History,
eviction asserts 200 entries without the target, and edit asserts the changed
text **before** retry. The cap fixture seeds 199 additional native-shaped rows
in one storage write, asserts the full store still contains the target, then
uses one real dictation submission to trigger and verify eviction. The other
completion fixture helper already carries
`schemaVersion: 2`; intentionally legacy fixtures retain their legacy shape.

Baseline checks on archived `91dc0f25`, with only new test functions added,
fail X (first missing History evidence), I (holding loss allows admission), and
O (registry exceeds the tombstone bound). Adding only the new pre-retry mutation
assertions to that baseline makes delete and edit fail: neither mutation had
actually run. Switching those fixtures to the native shape makes all five
receipt mutation controls pass, confirming the fixture repair rather than a
production mutation change.

## Fifth re-check regressions

- `review_evidence_before_rename_failure_remains_retryable` and
  `review_evidence_after_rename_failure_remains_retryable` start from the reviewer's
  scratch reproductions. Before/after v1, each checks empty History after the
  preparation fault, actor restart with preserved synthetic live phase, repeated
  acceptance, migration and relaunch: exactly one History entry, zero statistics.
- V: `review_cancel_ready_while_other_accept_holds_files_releases_capacity`
  fills all 200 slots with ready synthetic transcripts, blocks acceptance of 0,
  cancels 1–199 without releasing the owner, then verifies zero remaining staged
  results and all 200 replacement slots available. The next begin still rejects
  at the cap; discarding those running jobs releases all slots again.
- F: `file_acceptance_write_fault_matrix_is_retryable_or_admitted_once` covers
  **180 combinations**: before/after v1; each of BeforeRename, AfterRename and
  DirectorySync; cancel or retry; actor relaunch or same actor. Write sites are
  state preparation, content History/holding, final admitted state, pre-v1
  old-History cap, and state/holding/History receipt compaction. A paired-fault
  scenario fails content before rename and then fails the state cleanup at each
  boundary. The state file is the existing statistics journal, not an additional
  file. Every injected fault must be consumed at its intended write. Each case
  checks the immediate published count, three retries, migration and another
  relaunch, with zero statistics and exactly one entry (or zero if cancelled
  before any publication). No case without potential content publication is
  permitted to remain uncertain. Existing P/Q tests cover cancellation queued
  before and racing actual content writes; X retains evidence-loss coverage.

The three reviewer-derived regressions fail on `a2dfaa0c` with only the tests
added. The matrix also fails on that baseline with only its test fault seam:
preparation BeforeRename leaves an empty History permanently Uncertain. The
production changes preserve dictation's plan row 7 and the content-free error
notice. No other producer or durable schema changes.

## 1.8 shutdown handoff

Explicit asynchronous flush/seal barriers now replace reliance on final-handle
Drop. The host drains registered source/producer work before sealing. A private
completion-pending.json publication envelope preserves History/report content
across destination failures; replay runs before subsequent mutations and never
appends a second copy. Clear/Delete and caps cover pending copies; retention-off
producers submit no transcript. The earlier immediate-History-failure test still
checks that no row was published at that failure; new tests additionally verify
relaunch recovery. A lost pending publication remains UnpreservedFailure rather
than being hidden by successful statistics flush. See the
[shutdown contract](gpui-shutdown-coordinator.md) for deadlines and native gates.
