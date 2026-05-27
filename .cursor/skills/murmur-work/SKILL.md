---
name: murmur-work
description: >-
  Prepares Murmur GitHub issue worktrees and loads prompts/PROMPT.md for
  plan-first implementation. Use when running or mimicking the work command,
  starting an issue by number, creating an issue worktree, or loading issue
  context before coding.
disable-model-invocation: true
---

# Murmur Work

Use this skill when the user asks Cursor to run, mimic, port, or follow the `work` command; start work on a GitHub issue by number; create an issue worktree; or load the Murmur feature prompt plus issue context before implementation.

## Goal

Reproduce George's shell `work issue-number` flow in Cursor:

- Resolve the current GitHub repo.
- Fetch the requested GitHub issue title and body.
- Create or reuse a sibling worktree.
- Create or reuse an issue branch.
- Load `prompts/PROMPT.md`.
- Begin from the repo's plan-first assignment.

Do not launch Claude from Cursor. Prepare the worktree and follow the generated assignment yourself.

## Preferred Helper

If George's Codex skill helper exists, use it from anywhere inside the repo:

```bash
python3 ~/.codex/skills/codex-work/scripts/prepare_issue_worktree.py <issue-number>
```

For a non-mutating check:

```bash
python3 ~/.codex/skills/codex-work/scripts/prepare_issue_worktree.py <issue-number> --dry-run --no-fetch
```

After the helper runs, use the printed `Worktree:` path as the working directory for all subsequent commands.

## Manual Fallback

If the helper is unavailable, perform the same workflow manually.

1. Resolve repo context:

   ```bash
   REPO_ROOT=$(git rev-parse --show-toplevel)
   REPO_NAME=$(gh repo view --json nameWithOwner -q '.nameWithOwner')
   REPO_SLUG=$(basename "$REPO_ROOT")
   DEFAULT_BRANCH=$(gh repo view --json defaultBranchRef -q '.defaultBranchRef.name')
   ```

2. Fetch the default branch without mutating the current checkout:

   ```bash
   git -C "$REPO_ROOT" fetch origin "$DEFAULT_BRANCH"
   ```

3. Fetch issue details:

   ```bash
   gh issue view <issue-number> --repo "$REPO_NAME" --json title,body,url
   ```

4. Create a branch name from the issue title:

   ```bash
   ISSUE_TITLE="<issue title>"
   BRANCH_SLUG=$(printf '%s' "$ISSUE_TITLE" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//;s/-$//')
   BRANCH_NAME="issue/<issue-number>-${BRANCH_SLUG}"
   WORKTREE_DIR="${REPO_ROOT}/../${REPO_SLUG}-issue-<issue-number>"
   ```

5. Create or reuse the worktree:

   ```bash
   if [ -d "$WORKTREE_DIR" ]; then
     echo "Worktree already exists at $WORKTREE_DIR"
   elif git -C "$REPO_ROOT" show-ref --verify --quiet "refs/heads/$BRANCH_NAME"; then
     git -C "$REPO_ROOT" worktree add "$WORKTREE_DIR" "$BRANCH_NAME"
   else
     git -C "$REPO_ROOT" worktree add "$WORKTREE_DIR" -b "$BRANCH_NAME" "origin/$DEFAULT_BRANCH"
   fi
   ```

## Assignment Shape

Load `prompts/PROMPT.md` from the repo root if it exists, then append:

```markdown
## Your Assignment

You are working on GitHub Issue #<issue-number>: <issue title>

<issue body>

Work on this issue and nothing else. Your branch is already created: <branch-name>. Do not create a new branch.
Start by entering plan mode. Present your implementation plan and wait for approval before writing any code.
```

Follow the combined prompt. If it says to plan first or wait for approval, do that before editing files.

## Guardrails

- Work only on the requested issue unless the user explicitly expands scope.
- Do not create an additional branch after the issue branch exists.
- Do not mutate the dirty main checkout; use the issue worktree.
- Before editing, check `git status --short` in the issue worktree and surface unexpected changes.
- Run the verification commands required by the loaded Murmur prompt before committing or opening a PR.
