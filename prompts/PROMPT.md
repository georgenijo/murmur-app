# Agent Startup — Feature Mode

You are starting a new session on the Murmur project. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
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

**For UI/frontend changes:** If the dev server is running at `http://localhost:1420`, use the Playwright MCP to screenshot your changes after each significant edit. Call `browser_navigate` then `browser_take_screenshot`, evaluate the result visually, and iterate until it looks right. Do not skip this step for visual work.

## 6. Verify

Run all of these before committing:
- `cd app/src-tauri && cargo check` — no compile errors or warnings
- `cd app/src-tauri && cargo test -- --test-threads=1` — all unit tests pass
- `cd app && npx tsc --noEmit` — no TypeScript errors

If any check fails, fix the issue before proceeding.

## 7. Commit and PR

1. Stage and commit with a conventional commit message (`feat:`, `fix:`, `chore:`, etc.)
2. Push the branch: `git push -u origin <branch-name>`
3. Open a PR:
   ```bash
   gh pr create --title "<concise title>" --body "Closes #<issue-number>" --repo georgenijo/murmur-app
   ```
4. Report the PR URL.
