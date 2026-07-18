# Agent Startup — Release Mode

You are starting a release session for the Murmur project. Work autonomously through preparation and the trusted release build. Stop for a separate explicit final confirmation before creating or pushing the tag, because the tag promotes and publishes the release.

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
- Explain that pushing the version-bump commit starts a signed, non-publishing
  `Release Build`; the tag/publish confirmation comes only after that build is green.
- Ask: **"Ready to prepare the signed v{new_version} build on main? This will not create a tag or publish a release."**

Stop and wait for confirmation. This confirmation authorizes only the version
bump, main push, and non-publishing build. Do not create or push a tag yet.

## 5. Build Trusted Artifacts

Run these steps in order:

1. Bump `"version"` in `app/src-tauri/tauri.conf.json`
2. Bump `version` (package field only) in `app/src-tauri/Cargo.toml`
3. Commit: `git add app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml && git commit -m "chore: bump version to {new_version}"`
4. Push: `git push origin main`
5. Wait for the `Release Build` workflow on that exact commit to succeed.
6. Verify its `typecheck`, `release-macos`, and `release-linux` jobs, signed
   artifacts named with the exact 40-character commit SHA, package smoke tests,
   and cache summaries. Do not continue if any job or artifact is missing.

If the build fails, use the cold fallback in `docs/release.md`; do not tag.

## 6. Final Tag and Publish Confirmation

Present the exact commit SHA, successful Release Build run URL/timings, artifact
names, and proposed tag. Ask:

**"The signed artifacts are ready. Confirm pushing v{new_version}; this will promote and publish the GitHub Release."**

Stop and wait for explicit confirmation. A prior confirmation to prepare or
build the release does not authorize the tag.

## 7. Promote Release

Only after the final confirmation:

1. Tag the already-built commit: `git tag v{new_version}`
2. Push the tag: `git push origin v{new_version}`
3. Wait for the tag-triggered `Release` workflow to validate and publish the
   immutable artifacts (normally under two minutes), then update its notes:
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

## 8. Hand Off

Tell the user:
- Tag pushed — GitHub Actions is promoting the already-built signed artifacts
- Where to watch: `https://github.com/georgenijo/murmur-app/actions`
- Release will publish automatically at: `https://github.com/georgenijo/murmur-app/releases`
