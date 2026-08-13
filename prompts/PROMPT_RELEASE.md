# Agent Startup — Release Mode

You are starting a release session for the Murmur project. Work autonomously through preparation, the trusted release build, and automatic promotion. Treat the version-bump push as the release authorization: after that exact trusted build succeeds, GitHub automatically creates the tag and publishes the release.

## 1. Load Context

Read silently:
- `app/src-tauri/tauri.conf.json` — current version
- `app/src-tauri/Cargo.toml` — current version (must stay in sync)
- `app/package.json` — current frontend package version (must stay in sync)
- `app/package-lock.json` — current frontend lockfile version (must stay in sync)
- `CHANGELOG.md` — version history

## 2. Assess Current State

Run:
- `git status` — must be on `main` with a clean working tree. If not, stop and report.
- `git fetch origin && git log origin/main --oneline -5` — confirm local main is up to date with remote.

## 3. Determine Version Bump

Run:
- `git tag --sort=-version:refname | head -5` — find the last release tag
- `git log {last_tag}..HEAD --oneline` — all commits since that tag
- `git diff {last_tag}..HEAD --stat` — files changed

Analyse the commits using these rules (in priority order):
- Any commit with `feat!:`, `BREAKING CHANGE`, or a major architectural change → **major bump**
- Any commit with `feat:` → **minor bump**
- Only `fix:`, `chore:`, `docs:`, `refactor:`, `test:` → **patch bump**

Determine the new version by applying the bump to the current version in `tauri.conf.json`.

## 3b. Set the Updater Policy

Check if any commits since the last tag contain:
- Security fixes
- Breaking changes to the update mechanism itself
- Data format changes that make old versions incompatible

Set `.github/updater-policy.json` before preparing the version bump:
- Default/optional update: `"min_version": null` (users can skip or defer).
- Critical update: `"min_version": "{new_version}"` (older versions receive a
  non-dismissable forced-update modal).

If the criticality is unclear, ask: **"Is this a critical update? Should
min_version be set to this release?"** Never edit or replace the published
`latest.json` afterward. Trusted promotion validates the source-controlled
policy and emits the immutable updater manifest.

Include the min_version decision in the release summary.

Never set `min_version` to force an update until the currently shipped public
version has passed the post-release OTA canary. The first public
canary-capable build is a one-time bootstrap exception: physically install it
in the dedicated canary location and verify `--dry-run`; mandatory OTA gating
starts with the next release. After publication, run
`python3 scripts/murmur_canary_fleet.py --tag v{new_version}` on the trusted
Mac mini and do not announce the release unless it exits zero. The one-time
canary installation setup and result schema are in
`docs/features/auto-updater.md`.

## 4. Run the Pre-Release Murmur Bench Gate

Before asking for release authorization, compare the previous release tag with
the exact `origin/main` release candidate on the trusted benchmark Mac:

```bash
python3 scripts/murmur_bench_fleet.py \
  --baseline v{previous_version} \
  --candidate origin/main \
  --preset standard
```

Use `thorough` instead of `standard` when any commit since the tag can change
recognition latency, accuracy, delivered-text output, or memory. This includes
VAD, transcription backends, model runtime, transcript transforms, benchmarked
execution paths, and performance-sensitive Rust dependencies.

This gate is mandatory for every release. Do not use `--no-fail`. If the
comparison fails, rerun once with `--candidate-first` to expose order/thermal
bias. A repeated regression blocks the release. Mixed results are inconclusive
and also block the release until investigated or explicitly accepted by the
user. If the trusted Mac, corpus, or comparable baseline is unavailable, stop
and report the missing prerequisite rather than silently skipping the gate.

Raw reports can contain personal reference and recognized transcript text.
Leave them on the trusted benchmark Mac. The release summary may contain only
content-free provenance and results: exact refs, candidate SHA where applicable,
preset, model names, configured thresholds, aggregate deltas, and pass/fail.
Murmur Bench replays saved WAV files, so it does not replace the post-release
production check for live Core Audio startup, first PCM, device behavior,
clipboard, or paste.

## 5. Summarise the Build Plan

Present a concise release summary:
- Current version → New version (and why: major/minor/patch)
- Bullet list of what's included (one line per meaningful commit, skip chores/docs)
- Murmur Bench preset, compared refs, and aggregate pass/fail result
- Explain that pushing the version-bump commit starts the signed `Release Build`
  and that a successful build automatically creates `v{new_version}` and publishes
  its exact artifacts. A failed build never creates a tag or release.
- Ask: **"Ready to release v{new_version}? This will push the version bump to main; if the signed build succeeds, GitHub will automatically tag and publish it."**

Stop and wait for confirmation. This is the release confirmation: it authorizes
the version bump, main push, and automatic tag/publish after all gates pass.

## 6. Build Trusted Artifacts

Run these steps in order:

1. Set and review `.github/updater-policy.json` using the decision from Step 3b.
2. Run `python3 scripts/release_version.py prepare {new_version}`. This updates
   `tauri.conf.json`, `Cargo.toml`, `Cargo.lock`, `package.json`, and
   `package-lock.json`, cuts the current `[Unreleased]` notes into a dated
   `{new_version}` section, and opens a fresh empty `[Unreleased]` section.
3. Review the version-file, CHANGELOG, and updater-policy diff, then run
   `python3 scripts/release_version.py check {new_version}`.
4. Commit the synchronized version files, CHANGELOG, and updater policy with:
   `chore: bump version to {new_version}`.
5. Push: `git push origin main`
6. Wait for the `Release Build` workflow on that exact commit to succeed.
7. Verify its `typecheck` and `release-macos` jobs, signed macOS artifact named
   with the exact 40-character commit SHA, package smoke tests, and cache
   summary. Do not continue if either job or the artifact is missing.

If the build fails, use the cold fallback in `docs/release.md`. Automation will
not create a tag or release for a failed build.

## 7. Verify Automatic Promotion

Wait for the `Release` workflow started by the completed `Release Build`. Verify
that it used the exact build run ID and commit SHA, created `v{new_version}` at
that commit, validated the immutable macOS artifact and updater signature, and
published the GitHub Release.

If automatic promotion fails after the build succeeded, fix or rerun it. The
tag-triggered workflow remains the recovery path: only push the matching tag
manually after confirming the exact successful trusted-main build and source SHA.

Then update its notes:
   ```
   gh release edit v{new_version} --repo georgenijo/murmur-app --notes "$(cat <<'EOF'
   ## What's New
   - bullet per `feat:` commit (human-readable, not the raw commit message)

   ## Improvements
   - bullet per `perf:` / `refactor:` commit (omit section if none)

   ## Fixes
   - bullet per `fix:` commit (omit section if none)

   ## Full Changelog
   https://github.com/georgenijo/murmur-app/compare/v{previous_version}...v{new_version}
   EOF
   )"
   ```
   Write the notes yourself from the commit list in Step 3 — use clear, user-facing language (not raw commit messages). Omit any section that has no entries. Skip `chore:`, `docs:`, `test:` commits.

## 8. Validate Post-Release Production Latency

For any release that changes capture, transcription, delivery, model runtime,
or performance-sensitive dependencies, the release is published but its
performance validation remains pending until natural production use exists.

1. Ask the user to make at least three normal prompts in the updated production
   app. Do not generate synthetic prompts, drive another Mac's UI, or run a
   second app build for this check.
2. With the user's authorization to read that machine, confirm the installed
   app version and isolate the exact session beginning at
   `app setup — Murmur v{new_version}` in:
   - `~/Library/Application Support/local-dictation/logs/events.jsonl`
   - `~/Library/Application Support/com.localdictation/diagnostics/performance.sqlite3`
3. From content-free events, collect per recording:
   - helper resolve/signature/spawn time;
   - stream-open and first-callback phases;
   - start-to-first-retained-PCM and total audio-readiness time;
   - stop-to-worker-exit;
   - fallback, capture failure, zero-sample, stale-worker, or overlapping-owner
     evidence.
4. From `completed_runs.payload_json`, collect the matching app-version's
   successful dictation stages:
   - capture finalization, VAD, model queue/load, inference/decode;
   - transcript transform, clipboard/paste, and total post-stop processing;
   - warm/cold state, audio/output-size bucket, and bounded resource summaries.
5. Compare only compatible same-machine production cohorts. Match run kind,
   model/backend/accelerator, warm state, microphone transport/selection class
   where available, and input/output-size bucket. Do not compare dev with
   production, simultaneous app runs, different machines, or incompatible
   configurations as if they were a regression result.
6. Report sample count and every raw value when fewer than 20 compatible runs
   exist. A median is allowed for a small sample, but label it preliminary.
   Never report or compare p95/p99 with fewer than 20 runs per cohort.
7. Flag a preliminary median regression only when it is both greater than
   20 ms absolute and 15% relative. With at least 20 compatible runs, flag a
   p95 regression when it is both greater than 30 ms absolute and 20% relative.
   Any new capture failure, fallback, zero-sample success, stale worker, or
   overlapping helper is a regression regardless of sample count.
8. Record the comparison in the release handoff and link it to
   [#430](https://github.com/georgenijo/murmur-app/issues/430) until the in-app
   production-version comparison is authoritative.

The Diagnostics Reports tab is not a substitute for this check: it compares
Performance Lab or evaluation reports, not retained production dictation runs.
Follow `docs/features/performance-diagnostics.md` for the local run contract and
privacy boundaries. Never inspect or report transcript, clipboard, or audio
content while doing this comparison.

## 9. Hand Off

Tell the user:
- Exact commit, build run, promotion run, tag, and release URLs
- The signed build passed and GitHub automatically promoted its exact artifacts
- The release is published at: `https://github.com/georgenijo/murmur-app/releases`
- The pre-release Murmur Bench refs, preset, and aggregate result
- The production-latency comparison result, or clearly state that it is pending
  natural prompts on an explicitly authorized updated machine
