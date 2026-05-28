---
name: murmur-feature
description: >-
  End-to-end Murmur feature delivery in Codex: GitHub issue worktree (work
  flow), plan and implement from prompts/PROMPT.md, self-review, native app
  smoke test via Computer Use, PR creation, validation, and merge when green.
  Use when the user invokes /feature, ships a feature from an issue number,
  or wants plan + build + review + test + merge in one workflow.
disable-model-invocation: true
---

# Murmur Feature — Plan → Ship → Merge

Single Codex workflow combining **codex-work** (issue worktree + `PROMPT.md`) and **murmur-pr-test** (checks, native app, merge). Do not spawn another agent.

**Repo:** `georgenijo/murmur-app`

## Invocation

- `/feature <issue-number>` — full pipeline from issue to merge (when green)
- `/feature` — ask for issue number, or resume in an existing `../murmur-app-issue-<n>` worktree

**Stop early** only if the user asks (e.g. "plan only", "no merge"). Otherwise run all phases in order.

---

## Phase 1 — Prepare (codex-work)

From anywhere in the repo:

```bash
python3 .codex/skills/murmur-feature/scripts/prepare_issue_worktree.py <issue-number>
```

- Use `--dry-run --no-fetch` to inspect without changes.
- Read everything between `ASSIGNMENT START` and `ASSIGNMENT END`.
- **All later commands run in the printed worktree path** (sibling dir `../murmur-app-issue-<n>`).
- Do not create another branch.

Record: issue #, title, worktree path, branch name.

---

## Phase 2 — Plan (`prompts/PROMPT.md`)

Follow the assignment and **PROMPT.md** sections 1–4:

1. Load context (`CLAUDE.md`, `CHANGELOG.md`, relevant `docs/features/`, etc.).
2. Silent health check (`git status` in worktree).
3. Read issue body from the assignment — do not re-fetch unless the user asks.
4. **Plan mode:** design approach (files, risks, tests). **Wait for user approval before any code.**

No scope creep beyond the issue.

---

## Phase 3 — Implement

After approval, implement exactly the plan (PROMPT.md §5):

- Match project patterns in `CLAUDE.md` / `AGENTS.md`.
- **Settings UI:** Playwright MCP against `http://localhost:1420` only when dev server is running and the change is the settings webview — screenshot and iterate.
- **Native Murmur behavior** (overlay, hotkeys, dictation, tray): do not treat Vite alone as sufficient; you will verify in Phase 5 with the real `.app`.

Commit in focused chunks on the issue branch.

---

## Phase 4 — Self-review (pre-PR)

Review your own diff before opening a PR:

```bash
git diff origin/main...HEAD
git diff --stat origin/main...HEAD
```

Check for:

| Area | Murmur-specific |
|------|-----------------|
| Correctness | Mutex/rdev threading, recording state machine, audio pipeline |
| Security / privacy | No cloud leakage; telemetry stripping; clipboard handling |
| Scope | Only issue changes; no drive-by refactors |
| Tests | Rust tests for backend logic; TS types clean |
| Platform | macOS vs Linux `#[cfg]` paths when touching OS code |

Fix blocking issues. Skip style nits linters already cover.

---

## Phase 5 — Verify (required checks)

From the **worktree**, all must pass:

```bash
cd app/src-tauri && cargo check
cd app/src-tauri && cargo test -- --test-threads=1
cd app && npx tsc --noEmit
```

If any fail, fix and re-run before a PR.

### Native app smoke (when behavior or UI changed)

Build dev bundle:

```bash
cd app
npx tauri build --debug --config src-tauri/tauri.dev.conf.json
```

If exit code is nonzero only due to missing `TAURI_SIGNING_PRIVATE_KEY`, confirm the `.app` still exists — that alone is not a blocker.

Launch:

```bash
open -n app/src-tauri/target/debug/bundle/macos/Local\ Dictation\ Dev.app
```

**Use Computer Use** (Codex desktop automation) to exercise the feature in the real app: hotkeys, overlay, transcription flow, settings that affect native behavior. Do not substitute browser-only testing for native Murmur behavior.

Report what you exercised and what you observed.

---

## Phase 6 — Pull request

When checks (and native smoke, if applicable) pass:

```bash
git push -u origin <branch-name>
gh pr create \
  --title "<concise title>" \
  --body "$(cat <<'EOF'
## Summary
<1-3 bullets>

## Test plan
- [ ] cargo check / test / tsc
- [ ] native app smoke (if applicable)
- [ ] <issue-specific steps>

Closes #<issue-number>
EOF
)" \
  --repo georgenijo/murmur-app
```

Note the PR number and URL.

---

## Phase 7 — PR validation & merge (murmur-pr-test)

Treat your PR like any other Murmur PR:

1. `gh pr view <number> --repo georgenijo/murmur-app`
2. If not mergeable, merge `origin/main` into the worktree, resolve conflicts, re-run Phase 5 checks, push.
3. Re-read CI / checks on GitHub if present.

**Merge only when:**

- Phase 5 checks passed (re-run after any merge-from-main).
- Native smoke passed when the feature touched user-visible behavior.
- No known blockers in the PR or issue.
- User has not said "stop before merge" — default for `/feature` is to **merge when green**.

```bash
gh pr merge <number> --repo georgenijo/murmur-app --merge
```

Use `--admin` only if the user explicitly authorized bypassing branch protection and local verification is clean.

---

## Final report

Keep it short:

- Issue # and title
- Worktree path and branch
- Plan summary (one line)
- Checks run (pass/fail)
- Native smoke (what was tested, or N/A)
- PR URL
- Merge result (commit SHA or "not merged" + why)

---

## Partial runs

| User intent | Stop after |
|-------------|------------|
| "plan only" / "work 152" style | Phase 2 (after approved plan or at plan presentation) |
| "implement only" | Phase 3–5, no PR/merge |
| "test PR N" | Phase 7 only (use **murmur-pr-test** skill) |
| Full `/feature N` | Phase 7 merge when green |

## Related skills

- **codex-work** (`~/.codex/skills/codex-work`) — prepare worktree only; same script also lives under this skill's `scripts/`.
- **murmur-pr-test** (`.codex/skills/murmur-pr-test`) — validate/merge existing PRs without building from an issue.
