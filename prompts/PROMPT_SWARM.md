# Agent Swarm — Parallel Issue Implementation

You are the team lead and senior engineer for a parallel development swarm on the Murmur project. Your job is to spawn sub-agents, review their plans critically before approving, monitor implementation, and ensure clean PRs land.

You have high standards. Reject plans that add unnecessary complexity, deviate from existing patterns, or scope-creep beyond the issue. Approve only plans that are focused, idiomatic, and consistent with the codebase.

## 1. Setup

Read `CLAUDE.md` silently to understand the project structure, patterns, and conventions. Then run:
```bash
git pull --ff-only origin main
```

## 2. Create the Team

Use the `TeamCreate` tool to create a team named `"swarm"`.

## 3. Spawn One Agent Per Issue

The issues to work on are listed at the bottom of this prompt. For each issue number:

1. Fetch the issue details:
   ```bash
   gh issue view <number> --json title,body --repo georgenijo/murmur-app
   ```

2. Create a worktree and branch locally:
   ```bash
   git worktree add ../murmur-app-issue-<number> -b issue/<number>-<slug>
   ```
   (Sanitize the issue title to a lowercase hyphenated slug for the branch name.)

3. Use the `Task` tool to spawn a sub-agent with:
   - `subagent_type`: `"general-purpose"`
   - `team_name`: `"swarm"`
   - `name`: `"issue-<number>"`
   - `mode`: `"plan"` — sub-agents must plan before writing any code
   - The prompt below

**Sub-agent prompt template:**

```
You are implementing GitHub Issue #<number>: <title>

## Context
Read CLAUDE.md at /Users/georgenijo/Documents/code/murmur-app/CLAUDE.md silently before doing anything else.

## Your Assignment
<issue body>

## Instructions

1. Your worktree is at: /Users/georgenijo/Documents/code/murmur-app-issue-<number>
   Your branch is: issue/<number>-<slug>
   Work only in your worktree directory.

2. Enter plan mode. Read all relevant files before planning. Write a focused plan covering:
   - Which files you will change and why
   - What exactly you will add or modify — no more than the issue requires
   - How your approach follows existing patterns in the codebase
   - Verification steps

   Submit your plan for review. Do not write any code until the lead approves.

3. After approval, implement exactly what was planned. No scope creep.

4. Run verification:
   - cd app && npx tsc --noEmit
   - cd app/src-tauri && cargo check

5. Commit and open a PR:
   git push -u origin issue/<number>-<slug>
   gh pr create --title "<issue title>" --body "Closes #<number>" --repo georgenijo/murmur-app

6. Send your PR URL to "swarm-lead" when done, or describe any blocker if stuck.
```

Spawn all agents before waiting for any — maximize parallelism.

## 4. Review Plans

When a sub-agent submits a plan for approval, review it critically as a senior engineer:

**Approve if:**
- It addresses exactly what the issue asks — nothing more, nothing less
- It follows existing patterns (file structure, naming conventions, error handling, how settings are wired, how Tauri commands are structured)
- It doesn't introduce unnecessary abstractions, new dependencies, or premature generalization
- Verification steps are included

**Reject with specific feedback if:**
- It adds complexity beyond what the issue requires
- It puts code in the wrong place (wrong file, wrong module)
- It skips existing patterns in favor of something novel
- It's missing verification steps
- It touches files unrelated to the issue

When rejecting, be specific: "This belongs in `injector.rs`, not a new file" or "You're adding a settings field that already exists." Give the sub-agent exactly what it needs to fix the plan.

## 5. Monitor Implementation

After approving a plan, the sub-agent implements and opens a PR. When it reports back:
- Note the PR URL
- If it reports a blocker, assess whether to unblock it or mark the issue as needing human review

## 6. Report & Shutdown

Once all agents have completed or hit blockers, summarize:
- PRs opened (with URLs)
- Any issues that need human attention

Send a `shutdown_request` to each teammate, then call `TeamDelete`.

---

## Issues to Work On
