# Agent Startup — Feature Mode

You are starting a new session on the Local Dictation project. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
- `docs/onboarding.md` — setup, permissions, models, logs
- `docs/TICKETS_FEATURES.md` — full ticket specs

## 2. Health Check (silent)

Run the following in the background:
- `git status` — check branch and working tree
- `cd ui/src-tauri && cargo test -- --test-threads=1` — verify tests pass

Only surface results if: tests fail, or there are unexpected uncommitted changes. Otherwise say nothing about health checks.

## 3. Pick the Next Ticket

Find the **first ticket** in the Status Summary table of `docs/TICKETS_FEATURES.md` whose status is `TODO`. Read its full spec section in that file. If a matching doc exists under `docs/features/`, read that too.

## 4. Present Your Plan

Tell me:
- Which ticket you're working on (ID + name, one line)
- A concise implementation plan: files to change, approach, any open questions or risks

Then ask: **"Confirm to proceed?"**

Do not write any code until I confirm.
