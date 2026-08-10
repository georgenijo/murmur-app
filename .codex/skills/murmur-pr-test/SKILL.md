---
name: murmur-pr-test
description: >-
  Tests and validates Murmur pull requests in isolated worktrees with cargo,
  TypeScript checks, and native Tauri smoke tests. Use when testing a PR,
  merging a verified PR, picking the next PR from the queue, or continuing
  PR validation for georgenijo/murmur-app. For issue-to-merge delivery, use
  murmur-feature (/feature) instead.
---

# Murmur PR Test

Use this skill when testing, validating, or merging Murmur pull requests, especially when the user asks to pick the next PR, test a PR in the real native app, merge it if it looks good, or continue through the PR queue.

## Goal

Make PR validation repeatable, isolated, and merge-safe:

- Test each PR in its own worktree.
- Prefer the real native Tauri app for user-facing Murmur behavior.
- Run the required compile and test checks before merge.
- Merge only after the PR is verified and the user has asked to merge.
- Skip and report PRs with clear blockers.

## Before Testing

1. Inspect open PRs with `gh pr list --repo georgenijo/murmur-app`.
2. Pick the requested PR, or choose a small, testable PR if the user asks for the next one.
3. Read the PR body with `gh pr view <number> --repo georgenijo/murmur-app`.
4. Do not refetch the issue unless the user asks. Use the PR title/body and changed files.
5. If the PR body lists known blockers, do not test deeply; report the blocker and pick another PR if requested.

## Worktree Isolation

Never test PRs in the dirty main checkout. Use a separate worktree:

```bash
git fetch origin pull/<number>/head:codex/test-pr-<number>
git worktree add ../murmur-app-pr-<number> codex/test-pr-<number>
```

If the worktree already exists, inspect it before reusing it:

```bash
git -C ../murmur-app-pr-<number> status --short
git -C ../murmur-app-pr-<number> branch --show-current
```

## Choose The Test Path

Inspect changed files:

```bash
git diff --stat origin/main...HEAD
git diff --name-only origin/main...HEAD
```

Use the narrowest useful smoke test first, but always run the required checks before merge.

- Rust/backend changes: run Rust checks and tests.
- Frontend/settings changes: run TypeScript and native app smoke tests.
- Overlay or app UI behavior: test the real native app with Computer Use.
- Docs-only changes: no native app required unless behavior is unclear.
- Workflow/CI changes: inspect YAML and run local checks that approximate the workflow.
- Benchmark-sensitive changes (VAD, transcription backends, model runtime,
  transcript transforms, benchmarked execution paths, or performance-sensitive
  Rust dependencies): run the Murmur Bench gate after resolving the exact
  pushed PR head ref.

## Required Checks Before Merge

Run all of these from the PR worktree:

```bash
cd app/src-tauri && cargo check
cd app/src-tauri && cargo test -- --test-threads=1
cd app && npx tsc --noEmit
```

All checks must pass before merging. Fix only issues required to make the PR mergeable and verified.

## Murmur Bench Performance Gate

When the PR can change recognition latency, accuracy, delivered-text output, or
memory, resolve the immutable pushed PR-head commit and run the private
benchmark from the PR worktree against that SHA:

```bash
PR_HEAD_SHA="$(gh pr view --json headRefOid --jq .headRefOid)"
python3 scripts/murmur_bench_fleet.py \
  --baseline origin/main \
  --candidate "$PR_HEAD_SHA" \
  --preset quick
```

Before running, fetch `origin` on the trusted benchmark Mac and verify that it
resolves `PR_HEAD_SHA`; do not substitute a moving branch name. Record this
same candidate SHA in the validation receipt.

Use `standard` for shared cross-model or pipeline changes. Do not use
`--no-fail`. If the comparison fails, rerun once with `--candidate-first`; a
repeated regression blocks merge, while mixed results are inconclusive and
require investigation or explicit user acceptance. Raw reports can contain
personal transcript text and must remain on the trusted benchmark Mac. Put only
a content-free receipt containing the exact refs, candidate SHA, preset, model
names, thresholds, aggregate deltas, and pass/fail in the PR. Any later push,
rebase, merge from main, or conflict
resolution invalidates the result and requires a rerun.

For an unrelated PR, record `Murmur Bench: N/A — <reason>` rather than silently
omitting the gate. Murmur Bench replays saved WAV files and does not replace a
native smoke test for capture startup, device switching, clipboard, or paste.

## Native App Testing

When the PR affects Murmur behavior or UI, test the real app, not the Vite web page.

Build the dev app:

```bash
cd app
npx tauri build --debug --config src-tauri/tauri.dev.conf.json
```

If the command exits nonzero only because `TAURI_SIGNING_PRIVATE_KEY` is missing, check whether the `.app` was still produced. That signing failure is not itself a native smoke-test blocker.

Launch the app:

```bash
open -n app/src-tauri/target/debug/bundle/macos/Local\ Dictation\ Dev.app
```

Use native-app automation to verify visible behavior. Do not substitute browser testing for native-app testing when the user asks for the real app.

## Merge Preparation

If GitHub says the PR is dirty or not mergeable:

1. Fetch current main.
2. Merge `origin/main` into the PR worktree.
3. Resolve only necessary conflicts.
4. Preserve both the PR behavior and already-merged main behavior.
5. Rerun all required checks.
6. Commit the conflict resolution with a focused message:

```bash
git commit -m "chore: merge main into <short-pr-name> PR"
```

Push to the PR head branch:

```bash
git push origin HEAD:<headRefName>
```

After pushing the changed candidate, rerun the Murmur Bench gate when
applicable. Do not merge using a result from the pre-resolution commit.

## Merge

Only merge when:

- The user asked to merge passing PRs.
- Native smoke testing passed when applicable.
- `cargo check` passed.
- `cargo test -- --test-threads=1` passed.
- `npx tsc --noEmit` passed.
- Murmur Bench passed when applicable, or the PR has a justified N/A receipt.
- No unresolved/inconclusive benchmark regression remains unless the user
  explicitly accepted the measured risk.
- There are no known blockers.

Merge with:

```bash
gh pr merge <number> --repo georgenijo/murmur-app --merge
```

Use `--admin` only when the user has authorized merging despite branch protection and the local verification is clean.

## Reporting

Report:

- PR number and title.
- Worktree path.
- What was tested in the native app.
- Exact checks run and whether they passed.
- Murmur Bench preset and aggregate result, or N/A with reason.
- Any blockers, with the command error or file conflict.
- Merge commit or PR URL if merged.

Keep the report concise. If a PR is blocked, say why and move to the next PR only if the user asked to continue the queue.

## Related

- **murmur-feature** (`.codex/skills/murmur-feature`) — full `/feature <issue>` pipeline: worktree, plan, implement, review, native smoke, PR, merge.
