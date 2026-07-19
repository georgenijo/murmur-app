# Agent Startup — Release Mode

You are starting a release session for the Murmur project. Work autonomously through preparation, the trusted release build, and automatic promotion. Treat the version-bump push as the release authorization: after that exact trusted build succeeds, GitHub automatically creates the tag and publishes the release.

## 1. Load Context

Read silently:
- `app/src-tauri/tauri.conf.json` — current version
- `app/src-tauri/Cargo.toml` — current version (must stay in sync)
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

## 3b. Assess min_version

Check if any commits since the last tag contain:
- Security fixes
- Breaking changes to the update mechanism itself
- Data format changes that make old versions incompatible

If any of the above apply, ask: **"Is this a critical update? Should min_version be set to this release?"**
- Default: No (optional update — users can skip or defer)
- If yes: after the release publishes, download `latest.json` from the GitHub release assets, add `"min_version": "{new_version}"` to the JSON, then re-upload with `gh release upload v{new_version} latest.json --clobber` to replace the asset. Users running versions older than min_version will see a non-dismissable forced update modal.

Include the min_version decision in the release summary.

## 4. Summarise the Build Plan

Present a concise release summary:
- Current version → New version (and why: major/minor/patch)
- Bullet list of what's included (one line per meaningful commit, skip chores/docs)
- Explain that pushing the version-bump commit starts the signed `Release Build`
  and that a successful build automatically creates `v{new_version}` and publishes
  its exact artifacts. A failed build never creates a tag or release.
- Ask: **"Ready to release v{new_version}? This will push the version bump to main; if the signed build succeeds, GitHub will automatically tag and publish it."**

Stop and wait for confirmation. This is the release confirmation: it authorizes
the version bump, main push, and automatic tag/publish after all gates pass.

## 5. Build Trusted Artifacts

Run these steps in order:

1. Bump `"version"` in `app/src-tauri/tauri.conf.json`
2. Bump `version` (package field only) in `app/src-tauri/Cargo.toml`
3. Update the `ui` package version in `app/src-tauri/Cargo.lock`
4. Commit all three version files with: `chore: bump version to {new_version}`
5. Push: `git push origin main`
6. Wait for the `Release Build` workflow on that exact commit to succeed.
7. Verify its `typecheck`, `release-macos`, and `release-linux` jobs, signed
   artifacts named with the exact 40-character commit SHA, package smoke tests,
   and cache summaries. Do not continue if any job or artifact is missing.

If the build fails, use the cold fallback in `docs/release.md`. Automation will
not create a tag or release for a failed build.

## 6. Verify Automatic Promotion

Wait for the `Release` workflow started by the completed `Release Build`. Verify
that it used the exact build run ID and commit SHA, created `v{new_version}` at
that commit, validated both immutable artifacts and updater signatures, and
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

## 7. Hand Off

Tell the user:
- Exact commit, build run, promotion run, tag, and release URLs
- The signed build passed and GitHub automatically promoted its exact artifacts
- The release is published at: `https://github.com/georgenijo/murmur-app/releases`
