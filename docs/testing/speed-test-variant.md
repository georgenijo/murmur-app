# Murmur Speed Test variant

A separate, clearly labelled app for trying a dictation-speed change on a real
machine for a day without touching the production install. It is built from a
branch, never released, and never replaces `/Applications/Murmur.app`.

| | Production | Speed Test |
|---|---|---|
| Bundle identifier | `com.localdictation` | `com.localdictation.refactor-test` (the existing internal test identifier) |
| Product name | Murmur | Murmur Speed Test |
| Icon | green rings | orange rings with a **SPEED** banner |
| Data directory | `~/Library/Application Support/com.localdictation` | `~/Library/Application Support/com.localdictation.refactor-test` |
| Logs | `~/Library/Application Support/local-dictation/logs/` | `~/Library/Application Support/local-dictation-refactor-test/logs/` |
| Log shipper | on | off (internal bundle) |
| Updates | OTA | none (see below) |
| Login item | user setting | hidden; never registered |
| In-app label | none | "Speed Test build" badge in the main window header; menu bar tooltip and menu read "Murmur Speed Test" |

The log files (`app.log`, `events.jsonl`) have the same names and event codes as
production, so `pipeline.dictation_completed` and
`pipeline.dictation_stage_gaps` compare directly. The diagnostics database is
`~/Library/Application Support/com.localdictation.refactor-test/diagnostics/performance.sqlite3`.

## Why the refactor-test identifier

The log directory is chosen by bundle identifier in `telemetry.rs`, and only the
production, `com.localdictation.refactor-test` and `com.localdictation.bench`
identifiers are mapped to their own directories. Any other identifier logs into
production's directory and ships its logs. The variant therefore reuses the
refactor-test identifier, which also turns the log shipper off. Do not run the
Speed Test variant and a Refactor Test bundle on one Mac at the same time: they
share data, logs and macOS permission grants. (A distinct identifier needs a
`telemetry.rs` change, which `scripts/check_shadow_diff.py` currently rejects for
any PR because of two `dictation-shadow` twins in that file that its table does
not list.)

## What is shared with production

Only read-mostly assets keep their fixed production paths, because they are not
keyed by bundle identifier in the code: transcription models
(`local-dictation/models`, the FluidAudio Core ML cache), the VAD model, and the
opt-in NotchPill caption file (`local-dictation/latest-caption.json`, written
only when "Mirror to NotchPill" is on). Nothing else, including settings,
history, transcripts, vocabulary and diagnostics, is shared. Do not copy
anything but `settings.json` from production into the variant.

## Never self-updates

- `tauri.speed-test.conf.json` sets `createUpdaterArtifacts` to `false` and
  points the updater at `https://speed-test-updates-disabled.invalid/…` (`.invalid` never resolves; Tauri only accepts https). The
  production release feed is not in the bundle, so even a manual check cannot
  find a release to install.
- `VITE_MURMUR_SPEED_TEST=1` compiles the frontend with
  `NO_SELF_MANAGEMENT_BUILD`: no background checks, no update indicator, no
  Updates row, no Launch at login row.

## Build

Run from the repository root on a Mac with an Apple Development identity, as in
[Development](../DEVELOPMENT.md#stable-signing-for-dev-bundles). The frontend flag must be
set for the whole build:

```bash
python3 scripts/build_capture_helpers.py --release
export MURMUR_DEV_SIGNING_IDENTITY="Apple Development: <name> (<id>)"   # name or hash
VITE_MURMUR_SPEED_TEST=1 python3 scripts/build_dev_bundle.py --require-identity \
  --skip-helpers --config app/src-tauri/tauri.speed-test.conf.json
```

The script prints `DEV_APP=…`. Install it by copying the bundle with
`ditto` (which keeps the signature) to `/Applications/Murmur Speed Test.app`.

## Permissions

macOS stores grants per bundle identifier and signing identity, so the variant
asks for the `com.localdictation.refactor-test` Microphone, Accessibility and
Input Monitoring grants once.
Signed with the same Apple Development certificate, a rebuild keeps them.

Quit production Murmur before launching the variant. Both listen for the same
hotkey, so running both double-triggers it.
