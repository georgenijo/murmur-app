# Session Bootstrap

You are starting a new session on the Local Dictation project. Before doing anything else, complete these steps in order:

## Step 1: Load Core Context

Read these files:
- `CLAUDE.md` — project overview, file map, build commands, key patterns
- `docs/onboarding.md` — setup, permissions, models, logs

## Step 2: Check Project State

Run these commands and summarize the results:
- `git status` — current branch, uncommitted changes, staged files
- `git log --oneline -10` — recent commit history
- `git diff --stat HEAD~1` — what changed in the last commit
- `cd ui/src-tauri && cargo test -- --test-threads=1` — verify tests pass

## Step 3: Report Back

Give me a brief summary covering:
- What branch we're on and whether the tree is clean
- What the last few commits did
- Whether tests pass
- Any issues you noticed

## Step 4: Wait for Instructions

After reporting, wait for me to tell you what we're working on. Do not start writing code until I give you a task.

## Ticket Status

| Ticket | Description | Status |
|--------|-------------|--------|
| Pre-Work | Commit & merge feature/add-logging | ✅ Done |
| FEAT-001 | Structural UI Uplift | ✅ Done |
| FEAT-002 | Status Widget (tray icon + overlay pill) | ✅ Done |
| FEAT-003 | Custom Hotkey Binding (free-form key capture) | ✅ Done |
| FEAT-004 | Word Statistics (stats bar, localStorage) | TODO |
| FEAT-005 | Logging Viewer (in-app log viewer) | TODO |
| FEAT-006 | Resource Monitor (CPU/memory graph) | TODO |

FEAT-004 through FEAT-006 are all unblocked and parallelizable. Full specs in `docs/TICKETS_FEATURES.md`.

## Available Feature Docs

When working on a specific area, read the relevant doc first:

| Area | Doc |
|------|-----|
| Hotkey and double-tap recording | `docs/features/recording-modes.md` |
| Audio capture and whisper pipeline | `docs/features/transcription.md` |
| Clipboard and auto-paste | `docs/features/text-injection.md` |

## Available References

| File | When to read |
|------|-------------|
| `CHANGELOG.md` | When asked about version history or what's shipped |
| `README.md` | When updating user-facing documentation |
| `docs/onboarding.md` | When adding dependencies, permissions, or setup steps |
