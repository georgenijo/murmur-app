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
write History only. Activity events never change dictation counters.

`completion-state.json` contains the next sequence and typed content-free
statistics records with UUIDs. `stats.json` carries `applied_through` and all
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
| Crash after the journal append, before History | `dictation` |
| Crash after History, before statistics | `legacy` |
| Crash after statistics, before compaction | `crash_after_journal_before_history_replays_statistics_only` |
| A meeting, a proposed correction, a taught correction, and a Paste Last, each with a crash before statistics | `crash_after_history_before_statistics_has_no_duplicate_history` |
| Clock moved backwards; equal timestamps | `crash_after_statistics_before_compaction_does_not_reapply` |
| Statistics write for event N fails, then event N+1 commits | `activity_events_crash_before_statistics_match_uninterrupted_run` |
| History write fails | `clock_backwards_and_equal_timestamps_do_not_change_counters` |
| Statistics rename succeeds but the directory fsync fails, then retry | `statistics_n_fails_then_n_plus_one_applies_both_in_order` |
| The same, with power lost before the retry | `history_write_fails_statistics_apply_and_no_history_entry` |
| Statistics reset with records pending | `statistics_unknown_outcome_rereads_and_does_not_compact_until_synced` |
| Clear History; clear benchmark reports | `power_loss_after_unknown_statistics_write_replays_once` |
| Entry deletion; the 200-entry cap; `retainHistory=false` | `reset_with_pending_records_advances_watermark_in_one_write` |
| State-file creation fails on first start | `clear_history_and_reports_write_explicit_empty_files` |
| State file deleted or corrupted, before and after v1 commits | `deletion_cap_and_retain_false_never_change_statistics` |
| Holding file deleted or corrupted before v1 commits | `state_first_creation_failure_blocks_all_writes_and_arming` |
| Stale React settings save while records are pending, then a restart | `state_deleted_corrupt_or_future_before_and_after_v1_recovers_sequence` |
| Shape-valid but malformed disk History (for example `[null]`) or an unsupported version, with a valid WebKit copy | `holding_deleted_corrupt_or_future_does_not_change_import_decision` |
| Unversioned statistics and legacy History entries on disk | `stale_settings_save_does_not_touch_journal_or_migration` |
| Late v1 import with disk and WebKit holding the same data | `malformed_history_or_future_schema_quarantined_then_webkit_recovers` |
| Late v1 import with WebKit-only data | `unversioned_statistics_and_legacy_history_are_valid_destinations` |
| Valid disk statistics, migration timeout, new dictation, statistics file corrupted, relaunch, more dictation and a meeting, late v1 import | `late_import_disk_wins_over_identical_webkit_counters` |
| The same sequence for History | `late_import_webkit_only_is_below_held_and_added_once` |
| Timeout on first launch, new dictation, quit, v1 commits on the next launch | `valid_disk_statistics_corrupt_after_timeout_preserves_all_journal_events` |
| Clear History, statistics reset, or clearing reports, before a late v1 import | `original_history_corrupt_after_timeout_recovers_legacy_below_held` |
| Crash after the clear's destination write, before the holding-file write | `timeout_then_quit_then_next_launch_import_keeps_webkit_and_held` |
| A clear before v1, then the migration file moved aside | `clear_before_late_import_skips_each_destination_and_removes_its_keys` |
| Delete a held entry and an entry of the original History file, then corrupt the History file before a late v1 import | `crash_after_clear_destination_before_holding_drops_old_keeps_later` |
| Crash between a destination's import write and draining the holding file | `clear_before_v1_then_migration_file_lost_stays_cleared` |
| Import write commits, the holding-file write fails, the user deletes an imported entry, relaunch | `deleted_held_and_original_entries_do_not_return_after_corruption` |
| Two legacy entries share an id; the user deletes one before v1; History file corrupted; late v1 import | `crash_after_import_destination_before_drain_adds_content_once` |
| Repeated timeouts while new dictation and reports accumulate | `failed_import_drain_then_user_delete_and_relaunch_does_not_resurrect` |
| Fresh install with no WebKit data directory; relaunch | `duplicate_legacy_ids_delete_skips_exactly_one_matching_text` |
| Migration file moved aside after the store is retired | `repeated_timeouts_cap_combined_history_pins_reports_and_show_retry_on_third_launch` |
| Crash between a destination's import write and the v1 commit | `fresh_install_without_webkit_commits_immediately_and_never_holds` |
| Migration file moved aside before or after v1 commits | `retired_store_migration_file_lost_recommits_without_changing_destinations` |
| Colliding legacy and new History ids | `crash_after_destination_import_before_v1_commit_skips_marked_destinations` |
| Killed WebContent process during v1 | `migration_file_lost_before_or_after_commit_never_imports_twice` |

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
real completion actor to the fake capture/delivery effects. It covers successful
stop and auto-stop, cancellation, retention-off, file transcription, meeting,
taught correction and Paste Last; duplicate attachment adds nothing. Reopening
readers and relaunching the service preserve History and counters. Additional
actor tests cover concurrent producers, rejection of a second owner, exact-row
edits and pin limits, legacy WPM ordering and transform counters, report metadata,
UUID de-duplication, removal acknowledgements, and unknown journal outcomes.
Snapshot revisions prevent delayed IPC responses from replacing newer data.

Run Linux core tests/Clippy, TypeScript/Vitest, core dependency and repository
reference/markdown policy validators. macOS full-crate and native observations
are recorded separately in the stack PR; headless effects do not establish OS
paste or native GUI behavior.
