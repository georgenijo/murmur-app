# Dead-code audit, September 4, 2026

The checkout was updated to `origin/main` at
`16ec9b1e43cba6360f6d7d079bf4c2fb092a210e`, including PR #598. The original
checkout was clean. Cleanup branch: `chore/dead-code-audit-2026-09-04`.

The audit covered frontend import reachability, exported symbols, npm imports,
Rust dead-code warnings, Cargo dependency references, registered IPC commands,
CSS class references, and tracked build-output candidates. It does not prove
that every runtime path or every conditional compilation target is reachable.

## Completed cleanup

| Removed | Evidence |
| --- | --- |
| `app/src/components/ResourceMonitor.tsx` and `app/src/lib/hooks/useResourceMonitor.ts` | The component had no importers. It was the hook's only importer. Current resource diagnostics use `usePerformanceDiagnostics`. |
| `app/src/components/TranscriptionView.tsx` | No importers from any window, visual fixture, test, or app tool. Current Home, meeting, and query views have their own composition. |
| `app/src/components/FooterStats.tsx` and its single dedicated test | Only its own test imported the footer. The shipped Insights view and its tests remain. |
| `getApproxTokens` in `stats.ts` | No references. The unused approximation was separate from retained query token accounting. |
| `getCurrentUiLatencyView` in `uiLatency.ts` | No references. Transition tracking and the active view state remain. |
| `appearanceThemeLabel` in `appearance/themeLibrary.ts` | No references. Active theme selection and appearance labels use other code. |
| `semanticNonTextContrastFailures` in `appearance/resolve.ts` | Unused alias of the retained, used `nonTextContrastFailures`. |
| `react-use-measure` | No source, configuration, or script imports. Removed only this package from the manifest and lockfile. |

The removal pass deleted 485 lines before adding the audit script, its npm
command, and this report. Current hook and settings references were updated.
Historical changelogs and archived design documents were preserved.

## Remaining findings, in priority order

### 1. History reformat has a backend but no UI caller

`app/src/lib/historyReformat.ts` is the sole remaining unreachable frontend
module. Its `reformatHistoryText` wrapper invokes `reformat_history_text`,
which is still registered in `app/src-tauri/src/lib.rs` and implemented in
`commands/recording.rs`.

This is a disconnected feature, not evidence that the entire backend operation
is disposable. Decide whether to expose the action again or retire the wrapper,
command registration, implementation, and documentation together. The audit
preserves it because that decision changes feature scope.

`countVocabTokens` in `app/src/lib/dictation.ts` has no callers either. The
command reference still says its backend drives the Whisper prompt budget UI.
Review that claim and the intended UI before retiring the command.

### 2. The dashboard retains unused presentation modes

All remaining callers of `UsageDashboard` pass `displayMode="page"`. The
`inline` and `popover` branches, collapse toggle, `usage-dashboard-collapsed`
storage key, and inline-only CSS remain in `UsageDashboard.tsx` and `styles.css`.

Simplifying this component to the page layout would remove state, localStorage
access, and markup together. Keep the existing page rendering and Insights
visual tests as the acceptance contract.

Two custom CSS class names also have no literal references in tracked frontend
source or HTML: `.main-header` and `.home-nav-section`, both in `styles.css`.
The first shares declarations with the live `.ui-window-header`. Remove only
its selector, not that shared rule. The second has a standalone rule and a
compact-width selector.

### 3. Broad Rust warning suppressions conceal stale code

`selection.rs`, `transform_apply.rs`, and `transform_flow.rs` have module-wide
`#![allow(dead_code)]`. Selection's comment still says no command wires it to
the frontend, although `transform_flow` calls the capture path today.

Forcing the compiler to report dead code produced six warnings covering seven
items in the macOS debug app library:

| Item | Classification |
| --- | --- |
| `selection::log_capture_outcome` | Unused wrapper passing a zero pass ID. Production uses `log_capture_outcome_for_pass`. Remove the wrapper and update its doc references. |
| `TransformSession::new` | Used by unit tests. Production uses `new_for_pass`. Restrict this convenience constructor to tests. |
| `RecordingFlowEffects::emitted` and `secure_flash` | Used by unit tests. Restrict these methods to tests rather than suppressing the whole module. |
| `VocabAccumulator::is_empty` | Used by vocabulary unit tests. |
| `ranked_vocab_terms` and `build_vocab_prompt` | Used by vocabulary unit tests. Production has the directory/accumulator path. Preserve test coverage while narrowing compilation scope. |

The vocabulary helpers have individual suppressions. Platform-specific
injector suppressions, AEC-spike feature gates, and public sidecar test support
have separate reasons; they are not blanket deletion candidates. Check the
supported feature/target combinations before changing suppression scope.

### 4. Registered IPC includes APIs without client references

Twenty of the 188 registered commands have no name references in tracked
non-Rust code. This includes frontend code, scripts, and test tools. Some are
called by Rust itself; all remain callable through registration. The result
is a review list, not a finding that all twenty are dead:

| Area | Commands |
| --- | --- |
| Recording and models | `process_audio`, `check_model_exists`, `get_benchmark_models` |
| Permissions and keyboard | `open_system_preferences`, `update_keyboard_key`, `get_app_disabled` |
| Transform | `apply_transform_result` |
| Knowledge | `get_knowledge`, `resolve_knowledge` |
| Meetings | `get_meeting_store_status`, `prune_meetings` |
| Diagnostics | `get_log_contents`, `clear_logs`, `get_resource_usage` |
| Tray and overlay | `update_tray_icon`, `hide_overlay` |
| Transform window | `get_transform_popover_geometry`, `show_transform_popover`, `hide_transform_popover`, `set_transform_popover_focusable` |

Trace internal callers and documented diagnostic use before retiring an IPC
entry. Remove registration separately from shared native logic where needed.

### 5. Two Cargo dependencies need a separate build check

`serde_json` has no direct crate references in `sidecars/local-llm`.
`libc` has none in `spikes/moonshine-bench`. These are direct-dependency removal
candidates. Removing the sidecar's direct `serde_json` entry would still leave
the protocol crate's transitive JSON dependency.

The capture worker's `coreaudio-rs` dependency is used under its actual crate
name, `coreaudio`; its apparent name mismatch is a false positive. No app-crate
dependency lacked a literal reference. The sidecar and Moonshine dependency
removals were not compiled or applied during this pass.

### 6. Export counts mostly describe API scope, not dead implementations

After cleanup, 199 frontend exports have no external symbol references:
159 types and 40 values. Of those values, 37 are referenced within their own
module. Removing these implementations would break live code. Making selected
declarations private is optional API cleanup, not a performance improvement.

The remaining three values without local or external references are
`countVocabTokens`, `reformatHistoryText`, and the Sona component
`AnimatedDropdownTriggerIndicator`. The last is an optional registry component
export. Review the registry update convention before trimming its public API.

The import graph correctly retains all six packaged windows, the visual
fixture entry, dynamically imported Open VSX code, and test fixtures.
`components/log-viewer/testFixtures.ts` is intentionally test-only.

### 7. Design handoff and architecture descriptions have drifted

`docs/design/homepage-redesign-session.md` opens with a design-exploration
status and paths to removed variant components and `redesign-interact.mjs`.
It also contains later implementation notes. Its historical and current state
need clearer labels. The surviving `app/redesign-shot.mjs` is a manual capture
tool, not an app entry point; its usage comment names `shot.mjs`.

`docs/ARCHITECTURE.md` says "Four-Window Architecture", lists five windows,
omits Query Review, and gives Main's minimum 720×560 size as its default.
`tauri.conf.json` declares six windows and a default Main size of 880×720.

No tracked `dist`, `target`, `node_modules`, Python bytecode, or temporary
build-output files were found. The tracked WAV files are benchmark fixtures.
No worktrees, ignored data, historical evidence, or installed apps were deleted.

## Reproduce and interpret the checks

Run from the repository root:

```sh
node scripts/audit_frontend_dead_code.mjs > /tmp/murmur-dead-code.json
```

The equivalent npm command is `cd app && npm run audit:dead-code`. The script
uses the repository's installed TypeScript compiler. It reads tracked files
from the working tree and reports the base revision plus whether the checkout
is dirty. It does not modify source or return failure for review candidates.

The scanner counts type imports, literal dynamic imports, re-exports, all HTML
window roots, tests, and app tools. It distinguishes wholly unreachable files
from modules reachable only through tests. It does not expand glob imports or
infer runtime-computed consumers. The only detected glob is the existing raw
source privacy check in `no-mic-probe.test.ts`.

The Rust probe was:

```sh
cd app/src-tauri
cargo rustc --lib -- --force-warn dead_code
```

Rust's warning does not establish that public library APIs are used. This probe
covered the macOS default debug app library, not all feature combinations.

## Verification

| Check | Result |
| --- | --- |
| Production frontend build, including TypeScript | Passed before and after cleanup. Existing main-chunk size warning remains, approximately 1 MB minified. |
| Frontend unit tests after cleanup | 123 files, 1,118 tests passed. |
| Playwright visual suite after cleanup | 42 of 42 passed against existing goldens; none updated. |
| Light and dark Home at 880×720 | Screenshots inspected using bundled Chromium. |
| Strict Rust Clippy | `cargo clippy --workspace --exclude murmur-llm-sidecar --all-targets -- -D warnings` passed. Rust source was unchanged. |
| Reference documentation | All 188 registered commands have matching command-reference rows. |
| Capture-boundary validator | Passed; production HAL crates remain worker-only. |
| Final frontend reachability | One unreachable file, the preserved history-reformat wrapper. One intentional test-fixture module. Zero npm dependencies without module imports. |

The browser MCP could not initialize because Google Chrome is absent. The
repository's installed Playwright Chromium supplied the passing visual suite
and inspected screenshots. Native audio capture, signed helper packaging, Rust
test execution, and Murmur Bench were not run. Native source and dependencies
were unchanged. This audit does not include a merge or release.

## Follow-up verification, September 4, 2026

PR #675's initial CI run failed five Insights screenshots after midnight UTC.
Both the PR at `cc131bf` and its unchanged base at `8181b61` reproduced all five
failures with `TZ=UTC`. Their actual `light-insights.png` images had the same
SHA-256, `1b62a7f16ca5ebde13f8146d40803c0f176437642eb520ada7eae1925ea26dc4`.
The differences were the calendar cells and current-month recording count.
The fixtures used the wall-clock date, so they had advanced to September 5
while the checked-in screenshots represented September 4.

The visual suite now fixes the browser date to September 4 at 16:00 in
`America/New_York`. Playwright's fixed-date clock leaves timers running.
No production code, screenshot goldens, or comparison thresholds changed.
All 42 visual tests passed under both `TZ=UTC` and `TZ=Pacific/Auckland`.
The production frontend build and all 1,118 frontend tests also passed again.
