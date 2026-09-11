# Murmur agent guide

Privacy-first macOS voice-to-text app built with Tauri 2, Rust, and React.
Transcription and selected-text rewriting run locally. Optional Voice Query
runs a user-configured CLI that may send content to a cloud service.
`AGENTS.md` is a symlink to this file. Edit this file only.

## Find the relevant context

- [Architecture](docs/ARCHITECTURE.md): module ownership, data flow, and native boundaries.
- [Features](docs/FEATURES.md): feature index and links to detailed behavior.
- [Development](docs/DEVELOPMENT.md): setup, permissions, builds, and troubleshooting.
- [API reference](docs/reference/commands.md): 216 registered commands.

Read the documentation relevant to the task. Search the source for individual
files and symbols instead of maintaining another file inventory here.

## Common commands

Run each command from the repository root:

```bash
python3 scripts/build_local_llm_sidecar.py
(cd app && npm run tauri:dev)
(cd app/src-tauri && cargo check)
(cd app/src-tauri && cargo test -- --test-threads=1)
(cd app && npx tsc --noEmit)
(cd app && npm test)
```

On Apple Silicon macOS, build the bundled helpers before the first Rust check
or Tauri build. Helper binaries are gitignored. See Development for packaging.

## Verify the actual change

Run required CI checks and the narrowest relevant local checks. Frontend changes
need both TypeScript checks and Vitest. Documentation and script changes use
file or CLI checks; they do not require launching the app.

For all manual app verification, Codex uses Computer Use against the running
native macOS app, including settings and other frontend work. Build the changed
revision and confirm that the running app contains it. Click through the affected
flow, enter input, exercise shortcuts where relevant, and inspect the result.
Screenshots support that interaction; a browser preview does not establish native
app behavior.

If Computer Use is unavailable or prohibited, discover and obtain an allowed
native UI control tool, then exercise the same flow. Use Fleet to locate and
operate the Mac when working remotely. Respect tool and OS permission boundaries.
If no allowed native tool works, report the missing verification without claiming
a pass. Automated tests supplement native interaction.

## Optional performance work

Discuss benchmarks only when George mentions them. Run them only when he explicitly
requests a run. Implementation, testing, review, merge, and release requests do
not imply such a request. Do not add benchmark gates, reminders, skipped/N/A
receipts, or follow-up questions to routine work. For a requested run, use
[the performance guide](docs/features/internal-performance-harness.md).

## Constraints worth preserving

- Keep transcript, audio, clipboard, selection, and private-corpus content out of public logs and reports. Follow each feature's consent and retention rules.
- Recheck recording and transform generation ownership before async continuations mutate shared state. Resolve recording configuration once at recording start.
- Keep transcript transforms behind the ordered pipeline entry point. Refuse ambiguous corrections and unprovable secure-field access.
- Rust owns native window geometry. Dispatch `NSWindow` mutations onto the main thread.
- Call `set_is_main_thread(false)` before the rdev listener to avoid macOS TIS/TSM crashes.
- Keep `llama-cpp-2` in the signed LLM sidecar. Linking it into the app conflicts with whisper's ggml.
- Follow the [UI design system](docs/features/ui-design-system.md) for shared controls, spacing, and reduced motion.

## Finish and clean up

Complete verification, required review, and CI before merging. Merge only with
George's authorization, then verify the merge on GitHub before cleaning up.

After the task's PR is confirmed merged:

- Stop the app, dev server, and helpers started for this task, using their exact process identities.
- Remove task-created temporary builds, screenshots, fixtures, and isolated test data. Preserve evidence needed for the PR before deleting its local copy.
- Inspect tracked, untracked, and ignored files before removing the task worktree. Preserve unmerged or unrecognized work; remove a clean task worktree from outside it, then delete its merged local branch.
- Update the primary checkout to current `main` only when it is clean and not in use by another task. Verify final status and the remaining worktrees.

Cleanup covers this task's local state. Preserve production app data, credentials,
shared caches and models, other worktrees, and active sessions. Do not use broad
`git clean`, forced worktree removal, or hard resets to manufacture a clean state.
