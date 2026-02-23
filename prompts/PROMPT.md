# Agent Startup — Feature Mode

You are starting a new session on the Local Dictation project. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
- `docs/onboarding.md` — setup, permissions, models, logs

## 2. Health Check (silent)

Run the following in the background:
- `git status` — check branch and working tree
- `cd ui/src-tauri && cargo test -- --test-threads=1` — verify tests pass

Only surface results if: tests fail, or there are unexpected uncommitted changes. Otherwise say nothing about health checks.

## 3. Pick the Next Ticket

Run:
```bash
gh issue list --label "enhancement" --state open --json number,title,labels --repo georgenijo/murmur-app
```

From the results, pick the open issue with the highest priority label (p1 > p2 > p3). If no issues carry a p1/p2/p3 label, run `gh issue list --label "enhancement" --state open --sort updated --limit 1 --repo georgenijo/murmur-app` and pick the most recently updated open issue; if that also returns nothing, stop and report "no open enhancement issues found" with no further action. Then run:
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
