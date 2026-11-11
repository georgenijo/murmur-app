# Murmur agent guide

Privacy-first macOS voice-to-text app built with Tauri 2, Rust, and React.
Dictation and meeting transcription run locally.
`AGENTS.md` is a symlink to this file. Edit this file only.

## Find the relevant context

- [Architecture](docs/ARCHITECTURE.md): module ownership, data flow, and native boundaries.
- [Features](docs/FEATURES.md): feature index and links to detailed behavior.
- [Development](docs/DEVELOPMENT.md): setup, permissions, builds, and troubleshooting.
- [API reference](docs/reference/commands.md): 150 registered commands.

Read the documentation relevant to the task. Search the source for individual
files and symbols instead of maintaining another file inventory here.

## Common commands

Run each command from the repository root:

```bash
python3 scripts/build_capture_helpers.py
(cd app && npm run tauri:dev)
(cd app/src-tauri && cargo check)
(cd app/src-tauri && cargo test -- --test-threads=1)
(cd app && npx tsc --noEmit)
(cd app && npm test)
```

On Apple Silicon macOS, build the bundled helpers before the first Rust check
or Tauri build. Helper binaries are gitignored. See Development for packaging.

## Reproduce behaviour bugs first, not screenshots

A screenshot that shows the problem is the reproduction. When a report is about
how something looks (layout, spacing, wording, colour, a control's style, too
much on a page) and the screenshot shows it, go straight to the change. Do not
build and launch the app to look at the same screen again. The native check
comes once, after the change, under the next section.

Reproduce in the running native app before changing code when the report is
about what the app does: something fails, works only sometimes, jumps or
flickers, loses or corrupts data, or depends on timing, state, permissions,
audio, or another app. A screenshot does not replace this, because a still image
cannot show a timing or state problem. Do the same for an appearance report that
has no screenshot or whose screenshot does not show the problem.

For those bugs, write down the exact steps, what you expected, what happened,
and the revision you saw it on. Find the cause before changing code; a report
such as "status is not working" can be a layout, data, or timing problem, and
watching it happen decides which. After the fix, repeat the same steps on the
fixed build and record the result. If you cannot reproduce it, say so before
proposing a fix, and do not ship a guess as a fix.

## Verify the actual change

Run required CI checks and the narrowest relevant local checks. Frontend changes
run `npx tsc --noEmit`, `npm test`, and `npm run test:visual` from `app/`.
No screenshot baselines. Do not add `toHaveScreenshot` or commit baseline images.
When fixing a layout bug, add a geometry or DOM assertion that would catch it.
Documentation and script changes use
file or CLI checks; they do not require launching the app.

For manual app verification, use Computer Use or another allowed native UI
control tool against the running native macOS app. Build the changed revision
and confirm that the running app contains it. For behaviour, native chrome,
window, or overlay changes, click through the affected flow, enter input,
exercise shortcuts where relevant, and inspect the result. For appearance-only
changes such as CSS, copy, or spacing, inspect the changed screen in light and
dark in the built native app and attach a screenshot to the PR; click-through
is not required. A browser preview does not establish native app behavior.

Native UI work runs on a Mac. On a host that cannot build or run the app, use
Fleet to locate a Mac and do the reproduction and verification there. Respect
tool and OS permission boundaries. If no allowed native tool works, report the
missing verification without claiming a pass, and open a user-visible UI change
as a draft PR: it is not ready to merge until it has been exercised in the
native app. Automated tests supplement native interaction.

## Optional performance work

Discuss benchmarks only when George mentions them. Run them only when he explicitly
requests a run. Implementation, testing, review, merge, and release requests do
not imply such a request. Do not add benchmark gates, reminders, skipped/N/A
receipts, or follow-up questions to routine work. For a requested run, use
[the performance guide](docs/features/internal-performance-harness.md).

## Constraints worth preserving

- Keep transcript, audio, clipboard, selection, and private-corpus content out of public logs and reports. Follow each feature's consent and retention rules.
- Recheck recording generation ownership before async continuations mutate shared state. Resolve recording configuration once at recording start.
- Keep transcript transforms behind the ordered pipeline entry point. Refuse unprovable secure-field access.
- Rust owns native window geometry. Dispatch `NSWindow` mutations onto the main thread.
- Call `set_is_main_thread(false)` before the rdev listener to avoid macOS TIS/TSM crashes.
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
