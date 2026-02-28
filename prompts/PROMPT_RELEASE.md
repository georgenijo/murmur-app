# Agent Startup — Release Mode

You are starting a release session for the Local Dictation project. Work autonomously through every step. Only stop to confirm the final release action before pushing the tag.

## 1. Load Context

Read silently:
- `app/src-tauri/tauri.conf.json` — current version
- `app/src-tauri/Cargo.toml` — current version (must stay in sync)

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

## 4. Summarise and Confirm

Present a concise release summary:
- Current version → New version (and why: major/minor/patch)
- Bullet list of what's included (one line per meaningful commit, skip chores/docs)
- Ask: **"Ready to release v{new_version}? Confirm to proceed."**

Stop and wait for confirmation.

## 5. Execute Release

On confirmation, run these steps in order:

1. Bump `"version"` in `app/src-tauri/tauri.conf.json`
2. Bump `version` (package field only) in `app/src-tauri/Cargo.toml`
3. Commit: `git add app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml && git commit -m "chore: bump version to {new_version}"`
4. Push: `git push origin main`
5. Tag: `git tag v{new_version}`
6. Push tag: `git push origin v{new_version}`
7. Wait for the GitHub release to be created by CI (~1–2 min), then update its notes:
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

## 6. Hand Off

Tell the user:
- Tag pushed — GitHub Actions is now building the signed DMG (~15–20 min)
- Where to watch: `https://github.com/georgenijo/murmur-app/actions`
- Release will publish automatically at: `https://github.com/georgenijo/murmur-app/releases`
