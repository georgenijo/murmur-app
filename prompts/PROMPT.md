# Agent Startup — Feature Mode

You are starting a new session on the Local Dictation project. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
- `docs/onboarding.md` — setup, permissions, models, logs

## 2. Health Check (silent)

Run `git status` — check branch and working tree. Surface results only if there are unexpected uncommitted changes. Otherwise say nothing.

## 3. Pick the Next Ticket

The issue to work on is injected at the end of this prompt. Run:
```bash
gh issue view <number> --json title,body --repo georgenijo/murmur-app
```

Use the issue body as the full ticket spec. If a matching doc exists under `docs/features/`, read that too.

## 4. Plan Mode

Enter plan mode (use the `EnterPlanMode` tool). While in plan mode:
- Read all files relevant to the ticket using sub-agents
- Design your implementation approach
- Write a plan covering: which ticket (issue number + name), files to change, approach, and any risks
- Exit plan mode for user approval

Do not write any code until the user approves the plan.
