# GPUI 1.5: Rust-owned state and WebKit lifecycle

Closes nothing; part of #1009 / implements #989. Stack 06, based on
`gpui/05-targeted-events`; draft only, never merge this layer to main.

## Ownership and key inventory

Inventory: `rg 'localStorage|sessionStorage|indexedDB' app/src`, followed by
`StorageLike`, `DurableBlobStore`, `getItem`, `setItem`, `removeItem`, and wrapper
callers. Production has no sessionStorage or IndexedDB stores. Test-only storage
keys are synthetic fixtures. WebKit caches may remain while React lives, but no
v2 source is authoritative there. The updater retains its current owner until
1.6; it is explicitly excluded from this release's migration.

| Key | Owner | Disposition | Migration version |
|---|---|---|---|
| `dictation-history` | completion actor, `history.json` | Existing v1 import; remove only after destination receipt, never cache again | v1 |
| `dictation-stats` | completion actor, `stats.json` | Existing v1 import; remove after destination receipt, never cache again | v1 |
| `murmur-benchmark-report` | completion actor, `benchmark-reports.json` | Existing v1 single-report import; remove with report destination receipt | v1 |
| `murmur-benchmark-reports` | completion actor, `benchmark-reports.json` | Existing v1 import, ten-report cap and Clear; remove; both readers use the actor | v1 |
| `dictation-settings` | core settings schema, `settings.json` | Fill absent disk source; Rust settings v7 migrations; React presentation cache | v2 |
| `murmur-theme-library` | core source schema, `theme-library.json` | Fill absent disk source; normalize entries/revisions and reserved IDs in Rust | v2 |
| `murmur-appearance` | core source schema, `ui-state.json` | Preserve mode, complete custom theme, both selections; regenerate cache in renderer | v2 |
| `murmur_discovery_v1` | core source schema, `ui-state.json` | Fill absent source; explicit null is a durable reset | v2 |
| `murmur.settings-disclosure.v1` | core source schema, `ui-state.json` | Fill absent disclosure state | v2 |
| `usage-dashboard-collapsed` | core source schema, `ui-state.json` | Fill absent boolean, including explicit false | v2 |
| `murmur-microphone-startup-benchmark-reports` | core source schema, `ui-state.json` | Additional key discovered by inventory; bounded saved reports, explicit empty Clear | v2 |
| `murmur_onboarding_complete` | core settings launch metadata | Read once; fill absent `launchOnboardingComplete`, preserve disk gate; React cache mirrors settings | v2 |
| `murmur-overlay-vertical-offset` | core settings migration | Obsolete calibration source is dropped after v2; settings v2 resets the broken offset | v2 |
| `murmur-ui-latency-v1` | session presentation state | Drop after v2; stop persistence so it cannot refill | v2 |
| `updater-last-check` | existing React updater; future core updater | Remains writable by its current owner until #990 ships its import | v3, reserved |
| `pending-update-release-notes` | existing React updater; future core updater | Same contract, including explicit empty/dismissed state | v3, reserved |

`loadBenchmarkReports` is removed. `savedBenchmarkReports` reads only the actor
snapshot and cannot rewrite old keys. Synthetic benchmark tests exercise saved
report publication and relaunch without running the performance harness.

## Schemas, publication and concurrency

`murmur-core::state` owns durable source normalization. The settings defaults and
catalog are checked-in Rust schema data; the frontend consumes those defaults.
118 synthetic legacy settings cases port the previous migration outcomes into
Rust, including nested vocabulary, profiles, modes, browser rules and scan
summaries. TypeScript no longer migrates or publishes settings while reading.
Appearance source normalization discards only derived cache data, preserving a
custom theme without a library card. Reserved library IDs carry a durable rename
map so interrupted library publication can reconcile selected halves on retry.

The existing completion owner and advisory root lock serialize v1 destinations,
v2 state and migration receipts. Tauri source transports share `WRITE_LOCK`
with imports. Full settings saves and quick-control patches use that lock; a
durable owned-field revision prevents delayed/retried full snapshots from undoing
newer overlay Disable or Auto Paste commits. Every revision notification carries both owned values, so a missed event cannot
stamp a stale Disable value with a newer revision. Native configuration is built from
the merged committed document. Frontend retry intents retain their newest per-key and per-owned-field
generations. Main-save acknowledgements echo their originating generation; rebasing
committed fields preserves deliberate edits newer than that acknowledgement.
Configuration rollback supersedes the rejected owned-field patch. Recording generation ownership is unchanged.

Every core publication uses exclusive mode-0600 temporary siblings, file sync
plus macOS F_FULLFSYNC, rename and parent-directory fsync. An unknown rename
outcome is reread only after directory sync. Settings/theme transports now use
that publisher too. Source writes that fail retain retry intents and show a
content-free notice. Startup import failures preserve the source and display a
recovery screen with retry and explicit backup restore instead of seeding defaults.

Malformed source JSON is distinct from a missing key: transports send raw
strings, and storage read failures abort import. Malformed disk envelopes are
quarantined, with no content in diagnostics. A missing/quarantined required destination
with a committed v2 receipt blocks default publication. Future source schemas
stay in place and reject older launches repeatedly. Unknown keys are rejected
at the native source-write boundary. React has no retirement command.

## State machine and importer forms

`migrations.json` extends the existing schema-1 v1 record; it does not replace
1.3's destination markers, holding generation, clear receipts or journal.

1. **Import v1:** existing completion policy, per destination. A 1.3 Clear is a
   valid imported empty destination. Import never resurrects its WebKit copy.
2. **Import v2:** validate source, fill absent disk values, publish settings and
   library first, then `ui-state.json` with `v2_imported`, then `lifecycle.v2`.
   On replay, disk values and destination markers win, including empty values.
3. **Remove transcripts:** independently inspect v1 destination receipts, remove
   only their exact keys, verify absence, then durably acknowledge destinations.
   This runs even if v2 or source reads fail. It never clears the entire store.
4. **Import v3 (future #990):** updater owner must stop its React writers in the
   same release, fill absent updater values, durably publish its own imported
   destination receipt, then set `lifecycle.v3`. 1.5 never sets this bit.
5. **Retirement prepare:** only a native host holding the no-React/no-webview
   barrier may request it. Requires v1, v2, v3 and all three removal receipts.
6. **Retirement finish:** purge all types through WKWebsiteDataStore, fetch and
   verify zero records, release store/view references, remove the exclusive
   bundle directory, sync its parent, then commit `lifecycle.retired`.

The main React entry runs the source importer before hydrating caches or
rendering. Targeted removal is in an independent finally path, including
snapshot/read/import failures. Companions read Rust snapshots; they cannot
publish imports or retire the store. Existing typed EventSink transports remain
in use; no native broadcast is introduced.

`murmur-webkit` is a separate objc2 library with no Tauri/wry dependency. A future
native host reads the lifecycle receipt before opening WebKit; when the store is
already retired it must not create any WebKit object. Otherwise the production adapter obtains a store capability from the actual
Tauri/wry WKWebView configuration, checks pointer identity against the default
store, and passes that exact store to an invisible WKWebView with a fixed `tauri://localhost/` scheme
handler. Its fixed page reads the original origin via WebKit APIs. It sends
bounded raw messages, performs durable operations off the main thread and polls
results on the main loop. Partial imports still remove individually receipted
keys. The host retains the importer until completion, before showing user data
or creating React child views, and drops it before retirement. No private WebKit
on-disk layout is parsed.

If no WebKit data directory exists, current versions commit without opening a
webview. Missing v3 is never treated as committed. The retirement coordinator
ships now but cannot run in 1.5: v3 is deliberately absent. Its unsafe constructor
documents the host's exclusive directory and no-live-webview preconditions.

## Failure matrix

| State | Crash/failure point | Recovery |
|---|---|---|
| Unmigrated v1 | Any completion publication boundary | Existing 1.3 journal/holding/destination receipts resume |
| v1 marked, v2 unreadable | Snapshot, getItem, source decode or import | Preserve v2; independently remove only marked transcript destinations |
| v2 importing | Before settings/library rename | Old destination remains; retry fill-only import |
| v2 importing | After destination rename, before directory sync | Sync directory, reread; valid disk destination wins |
| v2 importing | After library write, before appearance write | Durable rename map reconciles source selections on retry |
| v2 importing | After UI destination marker, before migration marker | Skip source; commit migration marker from the destination |
| Removal authorized | During individual removeItem calls | Recheck destination receipts; repeat exact removal; no broad clear |
| Keys absent | Before/after acknowledgement rename | Repeat absent-key removal and durable acknowledgement |
| Corrupt source | Decode/schema validation | Preserve WebKit; no import receipt for rejected data; no retirement |
| Corrupt disk UI document | Quarantine before retry | Preserve quarantine; committed v2 prevents empty replacement |
| Future source schema | First or later older launch | Leave file in place, reject; do not seed defaults |
| Cleared on 1.3 | Update/import or migration-file loss | Destination marker keeps explicit empty state; legacy content stays removed |
| v3 absent | Retirement requested | Reject before any purge/directory removal |
| All receipts, no React | Before retirement prepare rename | Retry prepare; store untouched |
| Retirement prepared | During purge or empty-record verification | Repeat purge; do not remove directory until empty verified |
| Store purged | Before directory removal or parent sync | Repeat API purge/verification and directory removal |
| Directory absent | Before/after retirement marker rename | Repeat harmless removal; sync parent and commit marker |
| Retired | Later startup | Host skips WebKit entirely; source state remains on disk |
| First source save | After pending receipt or required marker, before destination publication | Replay the exact durable pending document before status/import/read/merge; finish marker and destination sync |
| Required source genuinely missing/quarantined | No valid pending publication | Startup stays blocked visibly; restore a user-selected complete backup; keep quarantine, never seed defaults |
| Source edit queued | Commit fails or a newer generation arrives | Keep newest intent; retry; never clear newer intent on old acknowledgement |

Quarantine and diagnostics contain only local data files or content-free
codes/counts respectively. No transcript, clipboard or selection content is
printed. These tests simulate crash publication boundaries; they do not claim a
physical power-loss test.

## Outcome-to-test mapping

| Required outcome | Executable coverage |
|---|---|
| Rust-owned user state; importer and marker ship | `legacy_settings_contract_fixtures_now_owned_by_rust` (118 cases); `source_theme_library_normalizes_reserved_ids_and_revisions_without_react`; React `webkitLifecycle.test.ts`; macOS `murmur-webkit/tests/synthetic_store.rs` |
| Unmigrated and migrated stores | `unmigrated_direct_062_import_preserves_custom_source_without_library`; `interrupted_import_and_already_imported_fill_only_and_marker_loss` |
| Interrupted import | Same test injects before/after rename at settings, UI destination and migration files; existing v1 failure matrix remains covered |
| Interrupted removal | `interrupted_removal_ack_retries_without_importing_cleared_content`; React exact-key removal tests; custom-store native removal/relaunch |
| Interrupted retirement | `sequential_versions_and_jump_to_future_v3_contract_retirement_recovery` injects prepare/finish marker failures; native synthetic purge/empty-record/directory removal |
| Corrupt and already-imported stores | `corrupt_source_and_future_disk_preserve_data_and_forbid_retirement`; `future_source_schema_stays_in_place_across_multiple_older_launches`; React raw-corrupt and already-imported cases |
| v1 → v2 → v3 and direct jump | `sequential_versions_and_jump_to_future_v3_contract_retirement_recovery`; v3 is a synthetic future updater receipt, not a shipped updater migration |
| Clear History on 1.3, then update | `clear_on_13_then_update_does_not_resurrect_even_corrupt_legacy`; existing 1.3 clear and marker-loss tests |
| Direct 0.62 → first native/post-Phase-4 host | Core direct-import test; standalone native fixture imports legacy History/custom appearance without React; same library is retained after the host flip |
| Complete appearance source, mixed selections, empty/pre-library | `appearance_pre_library_and_empty_library_keep_complete_theme_source`; custom-source and reserved-ID tests |
| Synthetic benchmark/relaunch leaves all four keys absent | `completionData.test.ts` synthetic report save/relaunch; standalone custom-store relaunch; reports still use the existing completion actor |
| Concurrent writer/failed-write ordering | Native `delayed_or_retried_full_settings_cannot_undo_a_newer_overlay_disable`; frontend retry/rebase test |

Native GUI verification is pending a dedicated worker. The implementation
worker sends no keyboard/mouse input and opens no app window on the shared Mac.
All WebKit tests use a custom nonpersistent data store; Rust fixtures and the
retirement directory are in temporary roots. No real app installation, data,
WebKit store, logs, transcripts or performance harness are accessed.

## Review regressions and proof boundaries

Retirement and transcript removal require readable destination envelopes with
`v1_imported`; the global flag cannot authorize either operation. A no-WebKit
install now creates explicit empty destination envelopes before committing v1.
The v2 UI receipt records the settings/library destinations actually imported,
including empty values. Later creations have separate durable `.required`
receipts and exact `.pending` publication documents. Missing expected files and quarantine stay errors across repeated
reads and relaunches; status, import replay and retirement all check them. Destination-receipt metadata is mandatory; a missing field fails visibly too. An exact pending publication is fully synced before the required marker, replayed
before reads/merges on restart, and removed only after destination directory sync.
Missing or quarantined destinations without a pending document require an explicit
user-selected complete current-version backup through the startup recovery screen
and main-only `restore_source_backup`. Quarantines are preserved. No defaults are
published to replace an expected missing destination.

React queues Rust intents before attempting cache writes. This includes settings,
theme-library and small UI writers. Deliberate main-window Disable and Auto Paste
edits accompany full settings writes as field patches; incidental snapshots still
respect the durable owned-field revision. `save_settings_blob` returns the merged
committed document. The renderer retains defensive library parsing for presentation
and import previews, but does not publish library repairs on load. Rust publishes
reserved-ID migrations. Appearance cache regeneration and JSON key order do not
advance revisions or emit repair events.

The standalone importer has a 30-second terminal deadline, navigation failure and
WebContent termination delegates, and JavaScript completion error inspection.
`PreservedForRetry` is terminal for that launch; hosts must present Rust-owned
data and retry WebKit later. No removal acknowledgement is sent without the
exact-key reply. `Importer::start` checks retirement before invoking the store
factory. `start_from_app_webview` derives the capability from the actual host
view; the unrestricted constructor is private. Tauri setup checks the actual
wry view against the default store. Retirement requires that capability and the
host's exclusive-directory/no-live-view safety preconditions. Synthetic capability
construction refuses persistent stores. The native-main host flip is future work;
this release continues to run the React importer in production.

| Failure/outcome | Executable test and scope |
|---|---|
| v1 destination missing/quarantined, repeated read and restart | `missing_or_quarantined_v1_destinations_never_authorize_removal_or_retirement`, public CompletionService API |
| Imported/created settings or empty library lost or corrupt | `imported_and_created_separate_destinations_remain_required_after_loss_or_quarantine` |
| Repeated corrupt UI with committed v2 | `repeated_corrupt_ui_recovery_with_committed_v2_is_never_empty_success` |
| Library before/after rename and directory-sync boundaries; library written before appearance | `library_rename_and_appearance_publication_failure_matrix`, six injected actual publication faults |
| Individual key-removal interruptions | React `retries interrupted individual transcript removals before acknowledgement`, injected Storage exceptions at each exact key |
| Every source form: wrong container/type, truncated, oversized, future schema | `corrupt_source_matrix_rejects_every_source_form_without_v2_receipt` (all eight v2 forms); `corrupt_transcript_source_matrix_preserves_all_four_keys` (four v1 forms) |
| WebKit fails after durable import | Native `synthetic_store`: actual JS evaluation error; injected navigation/termination delegate callbacks and expired deadline; terminal failure, no acknowledgement, Rust Snapshot remains available. Does not kill an OS WebContent process |
| Purge/verification failure and relaunch | Native `synthetic_store`: synthetic phase failure before purge and injected nonempty-verification result, followed by real API purge/empty-record verification on retry. Partial proof; does not force WebKit itself to fail its purge API |
| Interrupted directory removal/parent sync; absent-directory recovery | Native `synthetic_store`: actual partial synthetic-directory deletion or removal followed by injected sync failure, service relaunch and successful retry |
| Interrupted retirement marker | `sequential_versions_and_jump_to_future_v3_contract_retirement_recovery`, synthetic future-v3 receipt and real marker publication faults |
| Retired later startup creates no WebKit | Native `synthetic_store`, store factory panics if invoked after retirement |
| In-flight source acknowledgement vs failure/new generation | React `in-flight generation cannot discard the newer edit on failure=%s`, both completion outcomes |
| Quota failure still queues authoritative saves | React `quota errors cannot suppress Rust writes for UI settings or themes` |
| Missed revision then explicit re-enable | Native `missed_revision_event_does_not_undo_explicit_main_reenable` |
| No-WebKit status versus source writer | Native `absent_webkit_status_serializes_with_source_writer` |
| Settings migration disk publication/version and alias/scope restart | `settings_migrations_publish_version_and_alias_scope_round_trip_on_restart` |
| Literal defaults/consents/smart destination/malformed fuzzy | `literal_defaults_consents_smart_destination_and_correction_fuzzy`, expectations independent of defaults JSON |
| Complete models/Core ML default | `complete_model_catalog_preserves_every_supported_model` |
| Browser catalog and legacy host edge cases | `every_supported_browser_and_exact_legacy_host_contract` |
| Exact legacy string bounds | `unicode_bounds_preserve_each_fields_legacy_units` and `supplementary_boundaries_for_every_string_limit`: UTF-16 Modes/roots/theme labels/source metadata, code-point vocabulary (including odd surrogate boundaries), UTF-8 device IDs |
| Idle catalog, seven built-in identities/names, stale companion migrations | React `literal catalogs and companion ownership` |
| Obsolete offset key, with/without settings | React `removes obsolete calibration key after v2, settings-present=%s` |

The legacy 118-case contract now compares `settingsVersion` and onboarding
metadata instead of stripping them. Native readback independently evaluates all
four transcript keys after relaunch, rather than inferring absence from receipts.
All native fixtures remain nonpersistent stores and temporary synthetic paths.
These injected faults establish state-machine recovery, not physical power-loss
or production-default-store execution. Production identity is a runtime assertion
against the actual wry view, deliberately not exercised against user data here.

## Round-two implementation regressions

| Finding/outcome | Executable coverage |
|---|---|
| DS1: crash after first required marker, restart, then save | `interrupted_first_source_publication_replays_exact_intent_before_merge` injects real before/after rename and directory-sync faults for both destinations, restarts via public CompletionService, checks exact synthetic values, saves again; native `settings_merge_resumes_first_save_after_marker_before_destination_crash` exercises the production read/merge |
| DS1: genuine destination loss/quarantine | Native `explicit_backup_restore_requires_complete_supplied_data_and_preserves_quarantine`; React `SourceRecovery.test.tsx` drives backup selection and success/failure notice. No automatic reset or production data access; `malformed_pending_publication_never_seeds_defaults_on_restart` covers repeated damaged-receipt failures |
| DS2/F1: Auto Paste true→false; Disable→re-enable while first save is pending | React `older %s acknowledgement preserves the newer deliberate field intent` executes the real queue/rebase module with delayed native acknowledgement and checks final native patch for both fields |
| F2: configuration rollback supersedes rejected Auto Paste | `configuration rollback supersedes the rejected Auto Paste native patch`, real hook and source queue, synthetic native persistence plus configure failure |
| F3: exact pre-PR string units | `unicode_bounds_preserve_each_fields_legacy_units`, `supplementary_boundaries_for_every_string_limit`; audited against fa2be313 |
| F4: theme quota failure then successful next operation | `successive library edits use the controller document after cache quota recovers`, real controller install/install/remove sequence |
| F5: onboarding complete/reset with cache exceptions | `public onboarding complete and reset persist despite cache write and removal errors`; core `onboarding_patch_preserves_authoritative_settings_across_restart`. All source-persistence helper callers swept for cache-first suppression |
| F7: independent defaults and disk migration expectations | 118-case fixture uses frozen `settings-expected-defaults.json`, separate from production defaults; `literal_fallbacks_do_not_depend_on_production_defaults`; `settings_migrations_publish_literal_unchanged_delay_smart_destination_and_shortcut_removal` |

Restack receipt: before `8ba32dce8042bcb7f2ae6a91848d63bd491c54ec`,
after `792123132eb178174909e9b1046b9c4d45c91809`, onto
`8f976c342612b63014cd7f0067ed21f8a334d850`. No conflicts; all seven commits
compare equal in `git range-diff fa2be313..8ba32dce 8f976c34..79212313`.
Restack checks: TypeScript, core tests, 178 files / 1,787 Vitest tests passed
(previous 176 files / 1,756 tests; +31 tests inherited from layer 1.4).

Round-two Linux verification: TypeScript, 179 files / 1,795 Vitest tests
(+8 regressions after restack, no removals), 139 core tests (111 unit + 28
integration), strict core Clippy, Tauri-free dependency guard, both event guards
and their CI-invoked suites (15 Python / 42 Node), and reference/Markdown
validators passed. Native validation evidence and final-head CI status are
recorded in the PR body. Independent re-check and GUI verification remain with
the parent and dedicated verifier.
