# Agent Startup — Feature Mode

You are starting a new session on the Murmur project. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project instructions and documentation links (may already be loaded)
- `CHANGELOG.md` — version history, recent changes, naming conventions
- `docs/onboarding.md` — setup, permissions, models, logs
- `docs/reference/settings.md` — full settings schema (read if your ticket touches settings)
- Any file in `docs/features/` relevant to your ticket (recording, transcription, overlay, etc.)

## 2. Health Check (silent)

Run `git status` — check branch and working tree. Surface results only if there are unexpected uncommitted changes. Otherwise say nothing.

## 3. Your Assignment

The issue to work on is injected at the end of this prompt — title, number, and full body are included. Do not re-fetch it.

If any file in `docs/features/` is relevant to the ticket, read that too.

## 4. Plan Mode

Enter plan mode (use the `EnterPlanMode` tool). While in plan mode:
- Read all files relevant to the ticket using sub-agents
- Design your implementation approach
- Write a plan covering: which ticket (issue number + name), files to change, approach, and any risks
- Exit plan mode for user approval

Do not write any code until the user approves the plan.

## 5. Implement

After approval, implement exactly what was planned. No scope creep — do not refactor surrounding code, add comments to unchanged code, or introduce features not in the ticket.

For UI and behavior changes, follow the native verification policy in `CLAUDE.md`. Codex uses Computer Use to click through the changed app, or obtains an allowed native UI tool when Computer Use is unavailable or prohibited.

## 6. Verify

Run the relevant checks before committing, plus required CI checks:
- `cd app/src-tauri && cargo check` — no compile errors or warnings
- `cd app/src-tauri && cargo test -- --test-threads=1` — all unit tests pass
- `cd app && npx tsc --noEmit` — no TypeScript errors
- `cd app && npm test` — frontend tests pass

Exercise affected app flows using the native verification policy in `CLAUDE.md`.

If any check fails, fix the issue before proceeding.

## 7. Commit and PR

1. Stage and commit with a conventional commit message (`feat:`, `fix:`, `chore:`, etc.)
2. Push the branch: `git push -u origin <branch-name>`
3. Open a PR:
   ```bash
   gh pr create --title "<concise title>" --body "Closes #<issue-number>" --repo georgenijo/murmur-app
   ```
4. Report the PR URL.

After an authorized merge is confirmed on GitHub, follow the task cleanup rules
in `CLAUDE.md`.
