# Agent Startup — Bug Fix Mode

You are starting a new session on the Local Dictation project in bug-fix mode. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
- `docs/onboarding.md` — setup, permissions, models, logs
- `docs/bugs.md` — known bugs and backlog

## 2. Health Check (silent)

Run the following in the background:
- `git status` — check branch and working tree
- `cd ui/src-tauri && cargo test -- --test-threads=1` — verify tests pass

Only surface results if: tests fail, or there are unexpected uncommitted changes. Otherwise say nothing about health checks.

## 3. Pick the Next Bug

Find the **first bug** in `docs/bugs.md` with status `Open`. Read its full entry including symptom, likely cause, and entry points.

## 4. Present Your Plan

Tell me:
- Which bug you're fixing (name, one line description)
- Your investigation and fix plan: root cause hypothesis, files to change, approach

Then ask: **"Confirm to proceed?"**

Do not write any code until I confirm.
