# Agent Startup — Bug Fix Mode

You are starting a new session on the Local Dictation project in bug-fix mode. Follow these steps exactly and in order.

## 1. Load Context

Read these files silently:
- `CLAUDE.md` — project overview, file map, key patterns (may already be loaded)
- `docs/onboarding.md` — setup, permissions, models, logs

## 2. Health Check (silent)

Run the following in the background:
- `git status` — check branch and working tree
- `cd app/src-tauri && cargo test -- --test-threads=1` — verify tests pass

Only surface results if: tests fail, or there are unexpected uncommitted changes. Otherwise say nothing about health checks.

## 3. Pick the Next Bug

Run:
```bash
gh issue list --label "bug" --state open --json number,title,labels --repo georgenijo/murmur-app
```

From the results, pick the open issue with the highest priority label (p1 > p2 > p3). If no issues carry a p1/p2/p3 label, run `gh issue list --label "bug" --state open --sort updated --limit 1 --repo georgenijo/murmur-app` and pick the most recently updated open issue; if that also returns nothing, stop and report "no open bug issues found" with no further action. Then run:
```bash
gh issue view <number> --json title,body --repo georgenijo/murmur-app
```

Use the issue body as the full bug spec.

## 4. Present Your Plan

Tell me:
- Which bug you're fixing (issue number + name, one-line description)
- Your investigation and fix plan: root cause hypothesis, files to change, approach

Then ask: **"Confirm to proceed?"**

Do not write any code until I confirm.
