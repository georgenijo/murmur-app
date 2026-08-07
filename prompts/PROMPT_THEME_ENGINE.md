# Murmur Theme Engine — End-to-End Codex Delivery Prompt

You are the lead implementation agent for Murmur. Deliver the Theme Engine from the approved converged plan all the way to a verified, ready-to-merge GitHub pull request.

## Repository and source of truth

Run this prompt from the Murmur repository root. Resolve the repository with `git rev-parse --show-toplevel`; do not assume a Linux or macOS home-directory layout.

Canonical approved plan, relative to the repository root:

```text
docs/draft/theme-engine-converged-plan.md
```

Comparison-only plan, relative to the repository root:

```text
docs/draft/theme-engine-plan-v2.md
```

The converged plan is already approved. Do not pause for another plan-approval round. You may summarize your execution plan, but then proceed autonomously.

Use the `murmur-feature` skill and the repository's feature workflow where applicable, with this explicit override:

> DO NOT MERGE THE PULL REQUEST.  
> DO NOT ENABLE AUTO-MERGE.  
> DO NOT TAG OR RELEASE ANYTHING.

Your terminal condition is a non-draft, ready-to-merge PR with all required checks green, full QA completed, evidence attached, no known blockers, and no unresolved actionable review feedback.

## Initial setup

1. Read completely:

   - `AGENTS.md`
   - `prompts/PROMPT.md`
   - `.codex/skills/murmur-feature/SKILL.md`
   - `docs/draft/theme-engine-converged-plan.md`
   - All feature and architecture documentation referenced by that plan
   - Relevant current source and tests

2. Inspect:

   - Current branch, worktree status, remotes, and latest `origin/main`
   - Open GitHub issues and PRs for existing Theme Engine work
   - Current CI health on `main`

3. Preserve existing user files and unrelated changes. In particular:

   - The two theme-plan files may currently be untracked.
   - Do not delete or overwrite either.
   - Carry `theme-engine-converged-plan.md` into the feature branch as the canonical implementation record.
   - Leave `theme-engine-plan-v2.md` unmodified and uncommitted unless it is already tracked for another reason.

4. If an exact tracking issue already exists, use it. Otherwise, this prompt authorizes creating one tracking issue from the converged plan. Use one issue and one PR; treat the four plan tickets as internal delivery phases unless splitting them is technically necessary.

5. Create an isolated feature worktree and branch from the latest `origin/main`. Do not develop directly on `main`.

## Parallel execution

Use all safely available sub-agent capacity. Sub-agents may spawn their own bounded sub-agents when useful.

The lead agent owns architecture, integration, commits, pushing, and the PR. Do not let multiple agents edit the same files concurrently. Establish interfaces and file ownership first.

Suggested parallel workstreams:

### Foundation

- Appearance schema, sanitizer, storage, cache, resolver, and applier
- Selector-based Tailwind dark mode
- CSP-safe parser-blocking bootstrap
- Main/log-viewer synchronization
- Native application theme

### Appearance and accessibility

- Mode and accent UI
- OKLab/OKLCH color math
- Semantic contrast contracts and tests
- Token-debt audit and migration

### Advanced colors and file transport

- Background, foreground, and contrast behavior
- Import/export UI
- Bounded Rust UTF-8 read and atomic-write commands
- Invalid and oversized import behavior

### Independent review and QA

- Adversarial architecture review
- Privacy/security review
- Accessibility review
- Test-gap analysis
- Native UI and regression testing

Parallelize research, tests, documentation, audits, and disjoint implementation aggressively. Respect phase dependencies: the atomic foundation must stabilize before dependent UI is integrated, and token debt must land before advanced background/foreground controls.

## Implementation contract

Implement the converged plan as written. In particular:

- The selector dark-variant conversion and `data-appearance` boot/apply path must land atomically.
- Appearance must use `murmur-appearance`, never `dictation-settings`.
- Main and log-viewer are the themed windows in v1.
- Overlay and transform-review remain transparent, unsynchronized, always-dark glass.
- System-mode changes apply locally in each themed window and emit no Tauri event.
- Main is the only writer and user-change event emitter.
- User-change events are revisioned.
- First paint uses a strictly validated, write-time resolved token cache.
- Use a parser-blocking same-origin external bootstrap, not permanent unsafe inline JavaScript.
- Application-level native `setTheme` is required and owned by main.
- Accent math is dependency-free OKLab/OKLCH.
- Accessibility testing expands to the complete semantic contrast matrix before advanced colors ship.
- Theme import/export must not touch the clipboard.
- File dialogs select paths; bounded Rust commands perform UTF-8 reads and atomic writes.
- Imported resolved caches are discarded and regenerated.
- Preserve the overlay transparent-body invariant.
- No remote services, telemetry expansion, theme marketplace, or cloud behavior.

Do not silently cut scope. If the plan contains a genuine contradiction, investigate it, choose the safest minimal resolution, document the decision, and continue. Stop only if new user authority is genuinely required.

## Required engineering checks

Continuously run focused tests while implementing. Before QA, run the complete relevant suite:

```bash
cd app && npm test
cd app && npx tsc --noEmit
cd app/src-tauri && cargo test -- --test-threads=1
cd app && npm run tauri build
```

Also run formatting, linting, workflow validation, or repository-specific checks required by current CI.

Add tests for at least:

- Empty, corrupt, oversized, partial, and unknown-version storage
- Unknown presets and invalid token keys/values
- Resolved-cache validation and repair
- Bootstrap/runtime output parity
- Forced Light on dark OS
- Forced Dark on light OS
- System-mode OS appearance changes with zero emitted events
- Revisioned cross-window user-change synchronization
- React Strict Mode listener cleanup
- Exact Sonic reset fixtures
- Accent gamut and contrast
- Full semantic contrast matrix
- Contrast slider extremes
- Imported-cache stripping
- Malformed and oversized file imports
- Atomic export behavior
- Clipboard preservation
- Overlay and transform-review transparency
- Compiled Tailwind selector behavior, not only source-string presence

## Self-review

After implementation and before native QA:

1. Spawn fresh review agents that did not author the relevant code.
2. Review the complete diff against `origin/main`.
3. Check architecture, race conditions, boot behavior, storage migration, accessibility, CSP compatibility, Tauri capabilities, Rust path handling, privacy, and regression risk.
4. Fix every actionable finding.
5. Repeat review until no material findings remain.

## Full QA and Computer Use

Start the native Tauri application and perform a deliberate end-to-end QA session. Use the configured Playwright/browser tooling for frontend verification and Computer Use/native UI automation for macOS behavior.

Do not treat compilation or unit tests as sufficient.

Exercise and visually inspect:

- Main window
- Appearance Settings
- General Settings
- Onboarding
- History
- Performance Lab
- Log viewer
- Overlay
- Transform-review popover

Test this appearance matrix:

- OS Light × System
- OS Light × forced Light
- OS Light × forced Dark
- OS Dark × System
- OS Dark × forced Light
- OS Dark × forced Dark

Verify:

- Themed pixels and native title bars agree.
- Main and log-viewer update together after user changes.
- System mode responds correctly to live OS appearance changes.
- No duplicate events or listener loops occur.
- No wrong-theme first paint or visible Sonic-to-custom flash occurs after restart.
- Custom accent persists across restart.
- Reset restores exact Sonic appearance.
- Background, foreground, and contrast controls remain accessible at extremes.
- Import/export works for valid files.
- Invalid, unsupported, malformed, and oversized files fail visibly and safely.
- Import/export never changes the clipboard.
- Overlay never becomes an opaque rectangle.
- Transform-review remains transparent and dark glass.
- Reduced-motion behavior remains intact.
- Keyboard navigation, focus indication, labels, and error messaging work.
- Light and dark screenshots show no hardcoded-palette visual breakage.
- Logs contain no unexpected errors or event storms.

If microphone permissions and an installed model are available, complete at least one real recording/transcription cycle and confirm clipboard-first delivery still works. If unavailable, document the environmental limitation and perform the strongest safe substitute.

Capture screenshots and concise QA evidence. Iterate on every visual or functional defect found. Do not stop at the first successful smoke test.

## PR delivery

1. Keep commits intentional and logically grouped.
2. Update documentation, decisions, references, tests, and changelog as appropriate.
3. Re-sync with the latest `origin/main` before final validation.
4. Push the feature branch.
5. Open a draft PR early if useful, then convert it to a normal ready-for-review PR only after all validation is complete.
6. Include in the PR description:

   - User-facing behavior
   - Architecture summary
   - Privacy statement
   - Ticket/phase coverage
   - Test commands and results
   - Native QA matrix and screenshots
   - Known limitations
   - Risk and rollback notes

7. Monitor GitHub Actions until every required check reaches a terminal state.
8. Diagnose and fix all failures.
9. Inspect and address any actionable PR review feedback.
10. Confirm the PR is mergeable, current with main, non-draft, green, and has no unresolved blockers.

## Final stop condition

Stop only when the PR is genuinely ready for a human to press Merge.

Report:

- PR URL
- Final commit SHA
- Exact checks and QA completed
- Screenshot/evidence locations
- Any non-blocking limitations
- Explicit confirmation that the PR was not merged and auto-merge was not enabled

Under no circumstances press Merge, enable auto-merge, create a release, or push a release tag.
