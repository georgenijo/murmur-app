# GPUI identity, update chain, and downgrade rehearsal

Issue [#997](https://github.com/georgenijo/murmur-app/issues/997), rewrite plan
[§2.4](gpui-rewrite-plan.md#phase-2-gpui-becomes-the-host). This draft implements
the secretless policy and synthetic compatibility checks. It does **not** claim
installed update, signing, notarization, native UI, or real TCC acceptance.
George must run the signed checklist below before the host flip.

## Codified contract through Phase 3

`scripts/gpui-identity-policy.json` freezes the current Tauri host and every
bundled helper's identity inputs: the main config, macOS externalBin list, Info
plists (including permission descriptions and version placeholders), launchd
registration and exact entitlements. GPUI reads those same Tauri inputs and uses
the same finalizer. Packaging tests compare the GPUI finalizer arguments to the
Tauri release workflow, including its `APPLE_TEAM_ID` slot. They exercise
codesign argument construction with synthetic values only: fixed helper IDs,
hardened runtime, timestamps, per-executable entitlements, and no custom `-r`
requirement override. The host ID is `com.localdictation`; helpers are
`com.localdictation.capture-agent`, `.capture-helper`, and `.capture-worker`.

Developer ID codesign's designated requirement shape is identifier + Apple
generic anchor + Developer ID intermediate/leaf OIDs + leaf Team ID. Tests reuse
the existing canonical requirement validator with a synthetic `ABCDE12345`
placeholder for the host and every helper, rejecting broadened/changed
requirements. The helper runtime peer requirement is separately pinned to its
identifier + Apple generic anchor + the running host's Team ID. A placeholder
and matching inputs cannot prove the actual certificate/DR or TCC continuity;
compare the installed signatures for all three builds in the signed rehearsal.
Do not substitute ad-hoc identities, QA bundle IDs, or reset TCC for that proof.

`.github/updater-policy.json` must remain `{"min_version": null}`. The generated
v2 manifest currently omits `min_version` for null, which the shared updater
interprets as no forced minimum. The migration policy in
`.github/gpui-migration-policy.json` requires retention of the last Tauri arm64
DMG through Phase 3. `last_tauri_release` deliberately remains null in this
unshipped draft: George must name the **last Tauri build with the Phase 1.5 Rust
loader and these additive compatibility fixes**, then name the same tag in the
flip release notes. Pre-Phase-1 releases such as the plan's 0.62.0 baseline are
installation-chain starting points, not supported downgrade targets.

`validate_workflow_policy.py` runs both identity and update config checks.
`tests.test_gpui_packaging` also generates a real synthetic updater manifest and
checks flip readiness against a synthetic release inventory, rejecting a forced
minimum, unnamed target, draft/missing release or absent DMG. Before the signed
flip, supply a fresh GitHub release inventory (including assets and their
uploaded state/size/download URL), generated manifest and exact release notes:

```sh
python3 scripts/gpui_migration_policy.py \
  --manifest /synthetic-rehearsal/latest-v2.json \
  --releases /synthetic-rehearsal/releases-with-assets.json \
  --notes /synthetic-rehearsal/flip-notes.md
```

This is a read-only readiness check. It intentionally fails while the downgrade
tag is unset. The inventory proves only recorded metadata; signed acceptance
must actually download the old DMG. Existing promotion still excludes GPUI
artifacts; this PR neither enables that lane nor publishes/deletes releases.
Any future flip promotion must call this readiness check with fresh inventory
before publishing; retention is an ongoing release obligation, not a guarantee
that this offline policy can prevent external asset deletion.

Settings stay at version 7, History and UI-state envelopes at schema 1, and
appearance/theme-library at version 1. `murmur-core` retains same-version opaque
keys on normalized settings (including nested records), History entries and
metadata, appearance documents/themes/selections, UI-state envelopes, and theme
libraries. Invalid known fields still follow existing validation; obsolete
appearance caches, local source metadata and profile scan payloads still follow
their existing removal rules. Explicit delete/reset/replacement actions retain
their meaning; retention does not resurrect deleted objects. Unknown fields are
opaque durable data, not newly executable settings. GPUI theme, mode and spelling
exports use explicit projections of the existing known fields, so retaining
local opaque fields does not widen their export schemas or break strict import.
History exports still select their existing allowed fields; consent is unchanged.

The synthetic Rust tests write with the GPUI-used core APIs, edit/pin/reopen,
then read through the shared Tauri completion/lifecycle loaders. A Tauri adapter
test also exercises `load_settings_blob_locked` and the theme-library disk
loader on GPUI-written fixtures. All fixtures are checked in and synthetic.
These run as library tests in `rust-macos`; they do not require opening either
app or reading user data. Shared loaders avoid inventing a second parser. Keep
these fixes in the final Tauri downgrade build before the flip; an older strict
History envelope parser cannot promise preservation of future metadata.

## Manual signed acceptance — George owns every row

Use an isolated macOS test account/volume with synthetic settings, History and
appearance data. Record the installed pre-Phase-1 Tauri tag, last compatible
Tauri downgrade tag, first GPUI tag and next GPUI tag, macOS build, source SHAs,
Team ID, and artifact hashes. Fill the release tags before starting. Use the
production Developer ID/notarized installed bundles through the normal approved
release process. This implementation task must never access signing private
keys or notarization secrets, dispatch `release-build.yml`, or publish releases.
Never use the production app/data/pasteboard for this rehearsal.

Every row is pending and **requires signed builds**. Repeat the chain for an
account with permissions granted and an account with permissions denied; do not
reset TCC between versions. Record pass/fail and content-free evidence for each
permission and transition. Keep transcript/audio/selection/clipboard content
out of logs, reports, screenshots, and recordings.

| Check | Signed-build requirement | Pass evidence (pending) |
|---|---|---|
| Establish installed pre-Phase-1 Tauri baseline; grant/deny Microphone, Accessibility, System Audio Recording, Calendars and Apple Events independently | Requires signed builds | Tags/hashes, per-permission observed state and synthetic behavior; no TCC reset |
| Compare host and all bundled helpers across Tauri, first GPUI and next GPUI | Requires signed builds | Same identifiers, Team ID, canonical designated requirements and exact entitlements; Developer ID, notarization, staple and Gatekeeper verification |
| Install/update baseline Tauri → first GPUI using the supported updater path | Requires signed builds | Existing grants continue; denied permissions stay denied and show the correct recovery UI; no duplicate permission identity |
| Relaunch first GPUI; exercise permitted/denied dictation, meeting audio, calendar and Apple Events flows | Requires signed builds | Same TCC state after quit/relaunch; no unexpected prompts; no recording without consent |
| Update first GPUI → next GPUI, then relaunch and repeat every permission flow | Requires signed builds | Successful installed update, persisted state and same grants/denials without resets |
| Simulate failed/corrupt/interrupted update on each edge | Requires signed builds | Signature/verification failure leaves a runnable previous app; restart and retry recover without lost settings/History/appearance or changed grants |
| Verify hosted update policy and retained downgrade installer | Requires signed builds | Served manifest has no forced minimum; exact named Tauri arm64 DMG downloads successfully and matches its recorded hash |
| Run the real update canary for both installed update edges | Requires signed builds | Canary reports source/target/version/outcome and sanitized failure diagnostics; no user content; inspect both success and failed-update report |
| Write synthetic settings, pinned/edited History and custom appearance with next GPUI; install the named last Tauri release | Requires signed builds | Tauri loads all three without reset, quarantine, or loss of known/opaque fields; settings, pin/edit and appearance survive relaunch |
| Exercise all five granted/denied permission paths after downgrade, then recovery back to GPUI | Requires signed builds | Correct behavior under existing TCC decisions, stable data and successful recovery; capture per-permission results |

The issue's real-data acceptance is still a separate George-owned gate; synthetic
CI proves compatibility of codified formats only. Real-data rehearsal requires
George's explicit handling/consent and must keep all content private. After
Phase 4, remove downgrade support only through an explicit policy change and
state that downgrade is unsupported in release notes and the old DMG page.
