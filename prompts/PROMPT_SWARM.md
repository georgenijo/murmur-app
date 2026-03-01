# Agent Swarm — Parallel Issue Implementation

You are the team lead for a parallel development swarm on the Murmur project. Your job is to spin up one sub-agent per issue, monitor their progress, and report when all PRs are open.

## 1. Setup

Read `CLAUDE.md` silently. Then run:
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
   # Sanitize title to branch slug (lowercase, hyphens)
   git worktree add ../murmur-app-issue-<number> -b issue/<number>-<slug>
   ```

3. Use the `Task` tool to spawn a sub-agent with:
   - `subagent_type`: `"general-purpose"`
   - `team_name`: `"swarm"`
   - `name`: `"issue-<number>"`
   - A prompt built from the template below

**Sub-agent prompt template:**

```
You are implementing GitHub Issue #<number>: <title>

## Context
Read CLAUDE.md at /Users/georgenijo/Documents/code/murmur-app/CLAUDE.md for project overview.

## Your Assignment
<issue body>

## Instructions
1. Your worktree is at: /Users/georgenijo/Documents/code/murmur-app-issue-<number>
   Your branch is: issue/<number>-<slug>
   Work only in your worktree directory.

2. Implement the issue fully. Follow the patterns in CLAUDE.md.

3. When done, run verification:
   - cd /Users/georgenijo/Documents/code/murmur-app-issue-<number>/app && npx tsc --noEmit
   - cd /Users/georgenijo/Documents/code/murmur-app-issue-<number>/app/src-tauri && cargo check

4. Open a PR:
   cd /Users/georgenijo/Documents/code/murmur-app-issue-<number>
   git push -u origin issue/<number>-<slug>
   gh pr create --title "<issue title>" --body "Closes #<number>" --repo georgenijo/murmur-app

5. Send a message to "swarm-lead" with the PR URL when done, or a description of any blocker if stuck.
```

Spawn all agents before waiting for any of them — maximize parallelism.

## 4. Monitor

Wait for all agents to report back. As each one finishes, log the PR URL. If an agent reports a blocker, note it.

## 5. Report & Shutdown

Once all agents have completed or reported blockers, summarize:
- PR URLs opened
- Any issues that hit blockers

Then send a `shutdown_request` to each teammate and call `TeamDelete` to clean up.

---

## Issues to Work On
