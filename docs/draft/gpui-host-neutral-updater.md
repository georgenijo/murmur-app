# GPUI 1.6: Host-neutral updater (#990)

Stack 08, based on gpui/07. Draft; do not merge. Tracks #990 and #1009.
Native GUI QA belongs to the lead; this implementation worker runs no app.

## Decision and evidence

Use `minisign-verify` 0.3.0 with our own updater, rather than the published
`cargo-packager-updater` 0.2.3. The [decision log](../decisions/DECISIONS.md)
records the choice; no product/cost approval is pending.

| Criterion | Evidence and disposition |
|---|---|
| Wire/key compatibility | Same base64-decoded public-key and signature text, `PublicKey::decode`, `Signature::decode`, `verify(..., true)` as installed tauri-plugin-updater 2.10.0. Tests exercise both legacy Ed and prehashed ED packets. Neither private production key nor release mutation is involved. |
| Feed/platform/policy | latest-v2.json remains unchanged. Select `darwin-aarch64-app`, then `darwin-aarch64`; never a different architecture. Numeric-triple minimum policy retains optional absent/malformed metadata and fail-safe enforcement of malformed string minimums. Cumulative stable release notes remain bounded. |
| Translocation | Exact AppTranslocation path-component test; refuse downloads in read-only translocated locations. Require a regular canonical installed .app/Contents/MacOS/ui path. |
| Relaunch | No library relaunch. The thin Tauri adapter requests `shutdown::Request::Restart`; the same compatible reducer drains owners, applies Quit/System precedence, and alone issues terminal restart. React loses process:allow-restart. |
| Recovery | Same-volume staging, durable receipt, atomic exchange; launch path always contains a complete bundle. A failed verification/extraction/identity check leaves the installed bundle untouched. Startup distinguishes pre-exchange preparation from a confirmed updated binary. Corrupt/future receipts remain visible errors, with evidence preserved. |
| Maintenance/security | minisign-verify 0.3 has no external dependencies and checks canonical Ed25519 S. cargo-packager-updater 0.2.3 depends on 0.2, maps macOS to `macos` and has a remove-then-rename install fallback. We would still need the policy and lifecycle adapters with that library. Own the narrow macOS installer rather than unrelated Windows/Linux install/relaunch branches. |
| Dependency audit | Reuse reqwest/rustls, base64, semver, tar, flate2, plist and ring already in the lockfile. Only verifier upgrades to 0.3; ring is a test-only direct dependency for throwaway signing. CI dependency-audit continues; no exceptions are added. |
| Host boundary | All feed, state, download, verification, transaction, canary and recovery logic lives in murmur-core. `check_core_dependencies.py` scans all targets/features/dev/build edges for Tauri. |
| Phase 2 | No dependency on Tauri bundle markers or its updater plugin. Any packager can supply the same signed single-app tar.gz, identifier, version and ui executable. Phase 2 must still prove signing/notarization, entitlement/TCC and bundle-layout parity. |

Source inspected: installed tauri-plugin-updater 2.10.0 `verify_signature`,
static-platform selection and macOS installation; published
[cargo-packager-updater 0.2.3](https://docs.rs/crate/cargo-packager-updater/0.2.3/source/src/lib.rs)
and its [dependencies](https://docs.rs/crate/cargo-packager-updater/0.2.3/source/Cargo.toml);
[minisign verifier](https://github.com/jedisct1/rust-minisign-verify).

A read-only compatibility check also verified the exact published v0.62.1
Murmur.app.tar.gz (25,760,760 bytes) against that release's latest-v2.json and
sidecar signature with the embedded key. Archive SHA-256:
`4d3e8dbfee212bf7c16afaf8e17b9210c0340fcad86ce33428bf0d6408846816`.
This verifies existing release bytes, not OTA installation or relaunch.

## Ownership and flow

`murmur-core::updater::Updater` serializes actions independently of revisioned
view snapshots. React subscribes to `updater-state-changed` and forwards typed
intents. It retains no Update resource, downloaded bytes, manifest parser,
scheduling policy or localStorage updater writes. Rust owns the six-hour due
check, 15-minute timer, setup/automatic gate, two-attempt discovery and
five-minute pre-download refresh. Focus/visibility/wake signals forward the
same due-gated intent. Retained verified bytes survive filesystem replacement retries within
the process; payload verification, extraction or validation failures discard them. Download interruption loses only the incomplete in-memory buffer;
a later launch checks/downloads again.

The Rust owner persists updater-last-check and pending-update-release-notes
through the completion actor's serialized lifecycle. Migration v3 fills absent
disk keys only, publishes a UI-state v3 marker before the migrations.json v3
receipt, and never reimports after commit. Dismissal stores an explicit null.
React removes just the two legacy keys after the durable receipt, retrying
failed removals on later launches. The standalone WebKit importer reads and
removes them too. Existing v1/v2 schemas remain readable for release jumps.

The installer verifies bytes again, validates one bundle's identifier, version
and all four required executable files and effective execute access, rejects escaping paths/symlinks, and stages beside the
installed app. The receipt is durable before atomic `renameatx_np(RENAME_SWAP)`.
Linux uses renameat2 exchange solely for synthetic recovery tests. There is no
remove-then-rename fallback. Old bundle staging remains until the new process
confirms its own version, then only that transaction's staging is removed.
A non-writable install fails with the current app intact. Unsupported filesystem
exchange requires manual reinstall; privileges are never escalated.

## Failure paths and shutdown

Discovery errors are distinct from download/install/relaunch failures. Unknown
or missing platform/signature never becomes Ready. HTTP and redirects require
HTTPS; only the isolated feature accepts loopback HTTP. Feed/download/expanded
archive sizes and entry counts are bounded. A successful download remains
Ready until explicit install. Forced updates cannot be dismissed. Pending notes
appear only for the exact installed identity including prerelease/build suffix.

Shutdown closes updater admission and the writer phase joins its current owner
until the existing absolute deadline. Late network completions cannot publish
source state after admission closes. The swap checks admission immediately
before replacement. Atomic replacement and its durable receipt cover a forced
interruption; such interruption is never reported as a completed drain. The
adapter has no direct process termination or relaunch API.

## Isolated native QA fixture

Build with `shutdown-test-support` and `tauri.shutdown-test.conf.json` only.
The feature forwards `murmur-core/updater-test-support`. Its Main-only
`updater-test:allow-updater-test-run` capability exists only in that QA config.
The adapter accepts MURMUR_UPDATER_FIXTURE only for
com.localdictation.shutdown-test; production identity ignores it even when a
QA feature is compiled. Production builds compile no override path. A generated-context ACL test also
checks the real production/QA authorities: fixture access is Main-only in QA,
with no updater-plugin download or process-relaunch access in either config. Normal
internal/dev identities cannot consume the production feed. Policy tests in
`scripts/test_updater_fixture_policy.py` enforce these seams and exact trust
parity with the existing configuration; CI runs them.

`scripts/updater_test_fixture.py` takes a synthetic QA bundle and a new
~/.cache directory. It generates a throwaway minisign key, signs its tar.gz,
removes the private test key, and serves only feed/ on 127.0.0.1. fixture.json
contains the local feed URL and throwaway public key. `--failure signature`
tampers after signing; `--failure archive` signs a malformed archive. Neither
can affect a production bundle. Requires the public minisign CLI on PATH. Generator preparation was exercised
with minisign 0.12 for normal, tampered-signature and signed-invalid-archive
fixtures, and rejection of production identity; no app executable was run.

## Native procedure for the lead (not executed by this worker)

1. Build two QA bundles from this revision with distinct actual package and
   CFBundleShortVersionString versions (for example 0.62.1 and 0.62.2), both
   the shutdown-test identity/feature. Preserve ui and all packaged helpers;
   use a secretless local signature if needed. Keep bundles and scratch under
   ~/.cache; never use /Applications/Murmur.app or production profile paths.
2. Serve the new bundle with `python3 scripts/updater_test_fixture.py --bundle
   <new.app> --directory ~/.cache/updater-fixture-unique`. Retain its exact
   server PID. Set MURMUR_UPDATER_FIXTURE to fixture.json when launching the
   old disposable bundle; retain the old PID. The existing shutdown synthetic
   profile supplies recording/completion fixture data, never real content.
3. Use the allowed native UI controller to click Check, Download, Later,
   reopen, and Restart. Alternatively invoke the QA-only
   `plugin:updater-test|updater_test_run` with action kinds check/background:false,
   download, install while recording native UI proof. Observe the real download
   and Ready states, then the coordinator's dispatcher hold and drain receipt.
4. Confirm old PID terminates and exactly one replacement PID reaches ready,
   reports the offered actual version and shows the exact What's New notes.
   Verify synthetic history/statistics remain exactly once and the installed
   launch path never disappears. Quit the replacement and verify one drain.
5. Repeat with each failure mode. Signature failure must never become Ready;
   signed malformed archive must never replace the bundle; retry through a
   corrected server must succeed. Also interrupt download, and simulate process
   interruption before/after exchange using a disposable identity, then verify
   recovery and that no second exchange reinstalls the old bundle.
6. Exercise optional/forced dismissal, duplicate clicks, wake/focus, setup
   suppression, post-update notes dismissal/relaunch, translocated refusal,
   install permissions failure, Quit during install and Quit overriding pending
   relaunch. Preserve PID/version/receipt evidence. Stop only fixture-owned
   processes and remove only their isolated data/staging afterwards.

## Real OTA acceptance remains gated

MURMUR_UPDATER_CANARY and the updater_canary read/write command retain the
existing six-stage JSON contract. Rust drives discovery, dry-run policy,
download, signature verification, install and coordinated relaunch; the new
binary alone marks relaunch passed after validating the offered version.
The release workflow and scripts/murmur_canary_fleet.py are unchanged.

Two real signed OTA canaries in the Tauri app are still required before Phase
2. They require George's explicit authorization to publish signed releases.
This draft stack creates no tags, releases or public feeds and requests no
production signing private key. Headless unit checks and the local fixture do
not satisfy that real-OTA acceptance gate.

## Automated implementation evidence

Executable behavior at 5d952a58: TypeScript and 1,827 frontend tests pass;
core default/QA-feature suites pass 237/238 unit and integration tests. Mac
mini Rust 1.98.1 app checks pass 1,581/1,587 tests with default/QA configs
(16 opt-in tests ignored in each), plus the shutdown and main-dispatch
harnesses. Both all-target clippy configurations pass with the two requested
allowances, as do 18 foreign-source and 11 resolved restart guard fixtures.
All 21 native updater cases pass, including a deliberate symlinked cache-root
run; native core clippy passes. The Mac checkout, target and synthetic temp
data were removed after preserving evidence. CI results are recorded on the
draft PR; native GUI QA, independent Sol review and real OTA acceptance remain
with the lead. This evidence includes no app launch or production-profile use.


## Review corrections and compatibility audit

The review-fix branch first merged `origin/gpui/07-shutdown-coordinator` at
`3ae3061f` with a merge commit, including the bounded native QA receipt fixes.
This remains a draft stack PR; no retargeting, merge, release or app launch.

### Bundle acceptance before replacement

`tauri.macos.conf.json` and `scripts/finalize_macos_bundle.py` establish the
current bundle layout: `Contents/MacOS/ui`, `murmur-capture-agent`,
`murmur-capture-helper`, and `murmur-capture-worker`. All four are mandatory,
regular nonsymlink files with at least one execute mode bit **and** effective
`access(X_OK)` permission. `Contents` and `MacOS` must be real directories;
`Info.plist` must name `ui` as `CFBundleExecutable` and match identifier and
version. An alternate main executable is refused, rather than accepted without
validation. Extraction and every acceptance check run before journaling and
atomic exchange. A signed `0644` main or helper, missing helper, symlinked
executable, or invalid main identity leaves the old app intact, removes the
staging directory, drops the rejected bytes, and requests no relaunch.

The installed **tauri-plugin-updater 2.10.0** source was inspected at
`src/updater.rs::verify_signature` and its macOS `install_inner`: it verifies
the minisign archive signature, unpacks tar permissions, moves the old/new
bundles, and touches the installed bundle. It performs **no independent
codesign verification, Gatekeeper assessment, quarantine inspection or
quarantine removal**. The new owner preserves that trust boundary: the
embedded production minisign public key authenticates the complete archive;
release packaging still signs, verifies and notarizes the app and helpers.
It exceeds the old runtime flow with pre-swap identity, path and executable
checks and a continuously present atomic launch path. It does not claim
runtime Apple code-signature/notarization validation, repair quarantine or
prove compatibility of an independently packaged Phase 2 app. Those remain
release/Phase 2 acceptance obligations. No production signing key is involved.

### Retry, notes and presentation behavior

The download cache records the selected feed entry's **version, URL and
signature**, never just its version. Any selected-entry mismatch invalidates
it. A stale pre-download refresh can select corrected bytes at the same
version as well as a newer version; absent/older/failed refreshes retain the
last signed offer. As before, a downloaded Ready update stays stable through
ordinary Check and duplicate Download clicks. A failed verify/extract/validate
removes failed preparation staging and discards cached bytes, so Check can
read a corrected feed and Download can fetch it without a process restart.
The rejection error offers Check in the existing dialog, rather than an
Install retry with no downloaded bytes.
Filesystem replacement errors retain valid bytes for retry. An uncertain
post-exchange receipt retains both bundles for recovery and never swaps back.

Configure preserves future pending notes, including a Ready update's notes.
Only obsolete or malformed records are cleared automatically; a new download
supersedes the pending version and explicit DismissCompleted durably consumes
matching installed notes. Exact prerelease/build identity remains required.
The replacement exposes What's New until dismissal, focuses once per process,
and keeps notes even if native activation fails. Dismissal survives another
launch. The legacy stale-notes test still clears older pending versions;
clearing future pending versions was the bug, not the intended behavior.

All trigger sources now follow these rules:

| Trigger | Failure behavior |
|---|---|
| Startup Configure with automatic checks enabled | Error state and diagnostic only; no dialog opens. |
| 15-minute native timer | Same quiet error; six-hour due gate still applies. |
| Window focus | Same quiet error and due gate. |
| Visible visibility-change | Same quiet error and due gate; hidden documents send no action. |
| Native wake event | Same quiet error and due gate. |
| Configure/setup/model-readiness/storage recovery | Quiet; preserve pending/downloaded notes. |
| Explicit manual Check | Two discovery attempts; persistent failure opens the recovery dialog. |
| Explicit Download | Failure opens the download/install-stage recovery dialog. |
| Explicit Install | Failure opens the restart-stage recovery dialog. |
| Opt-in canary launch command/marker | Failure opens the dialog and records the first unproven canary stage. |

An already-open dialog is never closed by a quiet error. Optional background
availability remains quiet and notifies once per version; forced availability
opens only while setup/automatic checks remain enabled. Suppression is checked
again after discovery. Setup transitions do not repeat startup checks. Manual
checks remain available during setup and can join an in-flight owner. A
background request never competes with a current download. A withdrawn offer
removes stale availability and closes its dialog. Stable diagnostic event
codes for current version, download start/ready, and failures are retained.

The old WebView `Update.close()` resource and fallible `getVersion()` calls no
longer exist: Rust owns bytes and the immutable native package version. The
old async notification-permission race is also removed: the macOS native
notification plugin sends synchronously (its permission API reports Granted).
The core rechecks readiness immediately before invoking this native callback;
there is no awaited permission continuation to publish after setup restarts.
Relaunch ownership is the shutdown coordinator's single terminal request,
with Quit/System precedence, rather than a second WebView process call. Tests
cover the replacement seams below; GUI/real OTA acceptance is still separate.

### Deleted-test mapping

Audited all **78 test declarations** (including their parameterized cases) in
`origin/gpui/07-shutdown-coordinator`:
`useAutoUpdater.test.tsx` (42), `useAutoUpdater.onboarding.test.tsx` (7), and
`updater.test.ts` (29). Each row below is one old declaration; parameterized
forced/optional, absent/older and malformed-input cases remain parameterized
or explicit matrices in the replacement tests. Rust names below live in
`crates/murmur-core/src/updater/tests.rs` unless prefixed with `host:`;
frontend names live in `app/src/lib/hooks/useAutoUpdater.test.tsx`.

Test key:

| Key | Replacement test |
|---|---|
| A | `stale_refresh_selects_newer_or_corrected_same_version_and_retains_no_newer` |
| B | `freshness_failure_keeps_prior_signed_offer_and_shutdown_fences_continuations` |
| C | `real_http_owner_checks_downloads_retries_installs_and_confirms_notes_on_new_launch` |
| D | `configure_preserves_downloaded_notes_then_relaunch_shows_once_after_dismissal` |
| E | `host: completed_notes_activate_once_and_survive_activation_failure` |
| F | `pending_notes_preserve_future_clear_obsolete_and_are_superseded_on_new_download` |
| G | `canary_dry_run_preserves_pending_stages_and_actual_flow_marks_only_new_version_passed` |
| H | `canary_policy_download_archive_and_completion_failures_preserve_exact_stage` |
| I | `canary_receipt_roundtrip_requires_matching_installed_version_to_confirm` |
| J | `host: canary_command_preserves_read_write_wire_schema` |
| K | `startup_timer_focus_visibility_and_wake_errors_are_quiet_but_manual_actions_open` |
| L | `transient_feed_failure_retries_once_and_canary_marker_absence_is_inert` |
| M | `semver_policy_and_notes_cover_legacy_edge_matrix` |
| N | `preparing_progress_ready_and_translocated_recovery_follow_the_real_owner` and `download_without_content_length_remains_indeterminate_until_ready` |
| O | `rust_schedule_gates_setup_six_hours_ready_and_forced_dismissal` |
| P | `concurrent_checks_downloads_and_installs_have_one_owner_and_preserve_dialog` |
| Q | `uncertain_install_adopts_new_bundle_and_pre_exchange_failure_retries_without_swapback` |
| R | `background_policy_optional_forced_notification_once_and_withdrawal` |
| S | `in_flight_background_setup_suppresses_presentation_then_due_checks_resume` |
| T | `manual_check_joins_busy_owner_without_another_fetch` |
| U | `stable_diagnostics_cover_current_retry_policy_download_ready_and_failures` |
| V | `notes_include_complete_stable_upgrade_range_and_pending_requires_exact_identity` |
| W | `frontend: ignores legacy skipped-version storage and leaves native policy authoritative` |
| X | `frontend: forwards setup gating to Rust and keeps manual checks available` |
| Y | `frontend: does not forward hidden visibility and removes lifecycle listeners during setup and unmount` |
| Z | `rust_owner_requires_v3_before_network_and_generic_sources_cannot_seed_invalid_updater_values` |

**useAutoUpdater.test.tsx**

| Old test | Replacement |
|---|---|
| refreshes a stale update and downloads and installs the newer version with its cumulative notes | A, D |
| forces and downloads the fresh update when the installed version is unavailable | A, M |
| uses the retained update if a stale update refresh fails | B |
| uses the retained update when a stale refresh returns no newer version | A |
| does not refresh a recently discovered update or an already downloaded update | A, C |
| shows and focuses the main window after confirming the installed update | E, D |
| does not request focus on an ordinary launch | E |
| does not request focus when the pending update was not installed | E, F |
| preserves the cumulative upgrade range through download and relaunch | D, V |
| keeps release notes available if native activation fails | E |
| keeps optional availability after Later and can reopen it from the pill | C |
| does not suppress an available update from a legacy skipped-version value | W |
| logs the stable current-version event code when no update is available | U |
| leaves the canary path inert when the launch marker is absent | L |
| uses the Rust command boundary schema for canary reads and writes | J, I |
| canary uses the real download, install, and relaunch path | G |
| canary dry-run launches discovery and policy but never downloads or relaunches | G |
| records production stages as pending until install evidence is available | G |
| records policy failure without fabricating production stages | H |
| records a canary download failure at the download stage | H |
| records a canary relaunch failure after install evidence | H |
| recovers a pending canary only after the installed version matches | I, G |
| opens a recovery modal after a failed manual check | K |
| recovers from a transient updater-feed failure before showing an error | L, U |
| offers a verified update as optional when policy cannot be verified | M, R |
| uses an absent policy from the native response without a webview fetch | M, X |
| opens a required update when the verified policy is above the installed version | R |
| blocks installation before download when Gatekeeper translocated the app | N |
| downloads to a ready state and waits for explicit restart before installing | N, C, U |
| shows indeterminate progress and opens the modal for a direct download | N |
| keeps a forced ready update open | O |
| keeps the downloaded resource stable when a manual check is requested | C, P |
| reports a download failure and permits a retry | C, U |
| allows only one restart owner | P |
| retries installation without downloading again | C, Q |
| retries only relaunch after the update has installed | Q, G |
| allows only one download owner while environment verification is pending | P |
| waits out an in-flight check instead of dropping the download click | P |
| keeps the download dialog open when a background check finishes first | P |
| ignores a manual check while download owns the updater | P, T |
| ignores a due native wake check while download owns the updater | P, O |
| removes withdrawn availability after an authoritative background check | R |

**useAutoUpdater.onboarding.test.tsx**

| Old test | Replacement |
|---|---|
| stays quiet throughout setup and starts the normal background check when ready | O, R, X, Y |
| preserves explicit manual update checks while setup defers background work | X, K |
| keeps an in-flight background result quiet after setup restarts (forced: %s) | S |
| notifies a previously suppressed update after setup completes, once per version (forced: %s) | S, R |
| rechecks readiness after waiting for native notification permission | S (permission continuation removed; synchronous native boundary) |
| honors a manual request joining a background check after setup restarts | S, T |
| resumes due checks after setup is run again without repeating the startup check | S, O, Y |

**updater.test.ts**

| Old test | Replacement |
|---|---|
| parses valid semver strings | M |
| parses semver with pre-release and build metadata | M |
| handles v prefix and whitespace | M |
| returns null for invalid strings | M |
| detects equal versions | M |
| detects less-than | M |
| detects greater-than | M |
| returns null for unparseable versions | M |
| returns true when current < min | M |
| returns false when current >= min | M |
| returns true (force update) when versions are unparseable | M |
| returns 0 when no timestamp stored | O |
| round-trips a timestamp | O |
| isDueForCheck returns true when never checked | O |
| isDueForCheck returns false right after setting timestamp | O |
| isDueForCheck returns true when timestamp is old enough | O |
| returns the saved notes for the exact installed version | V, D |
| accepts an equivalent v-prefixed installed version | M |
| removes stale notes when the installed version does not match | F |
| does not confuse prerelease and final builds with the same core semver | M, V |
| rejects invalid versions even when both strings match | M, Z |
| fails closed and removes malformed storage | M, Z |
| can be dismissed permanently | D |
| distinguishes an intentionally absent policy | M |
| returns a present policy | M |
| does not collapse missing or malformed native metadata into absence | M |
| includes every missed release newest first and excludes installed/future/preview versions | V |
| uses numeric version ordering and accepts v prefixes | M |
| keeps single-release notes and falls back for missing or malformed history | M |

Additional review regressions: `nonexecutable_missing_symlinked_helpers_or_invalid_main_are_refused_before_swap`,
`signed_bad_archive_can_be_corrected_at_same_version_without_restarting`,
`cache_identity_requires_version_url_and_signature`, and
`request_fixture_waits_for_fragmented_headers_and_rejects_truncation` cover
findings 1, 2 and 5. Finding 3 is D; finding 4 is K plus the frontend
`forwards %s failures as quiet background checks` focus/visibility/wake cases.
The native timer and startup use the same background action tested by K.
`download_click_presents_dialog_while_waiting_for_background_discovery` additionally
asserts the immediate dialog intent in P before the network gate is released.
`scripts/test_updater_fixture_policy.py::test_installer_requires_every_packaged_executable`
keeps required executables aligned with the actual packaged helper list.
