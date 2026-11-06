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
same due-gated intent. Retained verified bytes survive install retries within
the process. Download interruption loses only the incomplete in-memory buffer;
a later launch checks/downloads again.

The Rust owner persists updater-last-check and pending-update-release-notes
through the completion actor's serialized lifecycle. Migration v3 fills absent
disk keys only, publishes a UI-state v3 marker before the migrations.json v3
receipt, and never reimports after commit. Dismissal stores an explicit null.
React removes just the two legacy keys after the durable receipt, retrying
failed removals on later launches. The standalone WebKit importer reads and
removes them too. Existing v1/v2 schemas remain readable for release jumps.

The installer verifies bytes again, validates one bundle's identifier, version
and ui executable, rejects escaping paths/symlinks, and stages beside the
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
