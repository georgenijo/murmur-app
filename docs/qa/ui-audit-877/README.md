# UI audit #877 verification

The implementation revalidated the older audit against v0.59.0 (`fe6a1f4e`) and integrated main's updater restart fix (`86230751`). No recognition, capture, transcript-transform, storage-schema, or Rust product code changed.

## Implementation and review

- `da53e423`: settings accessibility and navigation, dialog/draft/deletion behavior, review contrast, meeting editor layout, calendar labels, palette grouping, and scoped help/privacy copy.
- `b4348186`: restore Create theme focus when Reset to Sonic removes the original Custom-card opener.
- `ff9d0075`: ignore unchanged Home media-query notifications and reconcile a breakpoint crossed before subscription.
- Independent `gpt-6.1-sol` high reviews found and resolved Select placement during disclosure animation, Custom-provider test help, and disconnected theme-opener focus. Focused rechecks passed, including the Home delta.

Current-main behavior was retained and regression-tested where already fixed: Paste Delay keyboard persistence (#883), overlay quick-control geometry (#886), Diagnostics wrapping (#887), the named search omissions (#890), selected-fragment Copy handling (#894), and the replacement Home checklist (#897).

## Local checks

On the integrated sources at `c2db9695` plus the unchanged Home delta subsequently committed as `ff9d0075`:

- TypeScript passed.
- Vitest: 2,048 tests in 198 files passed.
- Darwin Playwright: 388 tests passed, with zero failed, flaky, or skipped classifications; product/test and Darwin snapshot hashes were stable throughout.
- Rust: `cargo check` passed; 1,775 unit and 43 integration tests passed, with 17 ignored. Rust sources are unchanged by the epic.
- Reference documentation validation passed for 243 registered commands; Markdown links passed for 76 maintained documents.
- Release workflow policy validation and 86 release artifact/version/policy tests passed.

Linux snapshots are generated and inspected in the CI-matching amd64 Playwright 1.62.1 Noble image with Node 20, not copied from Darwin. The affected 219 cases and full 388-case Linux suite passed with zero skips or flakes after five further inspected Linux snapshots were reconciled. Product/test hashes were identical before and after the final run.

## Native evidence and scope

The isolated packaged app uses `com.localdictation.epic877`. Its supported minimum is **840×620**; **720×560 is browser stress coverage only**. Native build 7 contains product revision `ff9d0075`; executable SHA-256:

`0fe49041e14a28841058990a04ab28fbf26e65c091203cd62fbb77d9860cbbb6`

Observed with actual WKWebView accessibility, keyboard/pointer interaction, and screenshots:

- Settings search has one operative clear control in Light and Dark. Clearing preserves input focus. App overrides remains visible after its search highlight ends.
- Sensitivity exposes its name, percentage, and Off value. Paste Delay keyboard changes persist across navigation and relaunch; pointer changes remain functional.
- Named Select controls retain current values; End/Home navigate and scroll edge-positioned choices, and Return selects.
- Command New/Edit/Duplicate, dirty-draft preservation/discard, and successful Save restore the correct invoker. The disabled-setting link focuses the exact off switch without enabling it.
- About has named dialog semantics, contained keyboard focus, Escape dismissal, and scoped local/provider copy. Theme actions remain visible; resetting Custom to Sonic restores the persistent Create theme control.
- Three disposable synthetic query records were filtered, cancelled, then explicitly deleted. The backend readback reported zero records and clear epoch 1. The separate synthetic meeting remained unchanged by deletion.
- Synthetic meeting Cancel retained original review values; Save durably changed task/owner/date and incremented review revision to 2. A hash over all canonical transcript columns remained unchanged.
- Home checklist text, unnumbered Customize destinations, wrapped Diagnostics stream chips, onboarding privacy copy, Insights calendar anchors, and its accessible Daily values table were inspected at minimum size. Diagnostics filters were toggled and restored. Setup was completed using Skip for ungranted permissions.

These synthetic history fixtures test UI/backend behavior; they are not recording or provider-execution evidence. Native AX evidence is not a VoiceOver spoken-announcement result. A separate synthetic AppKit/WKWebView host also confirmed Query Light/Dark and long-answer footer fit at 440×340 points, and opaque rewrite surface/diff/actions at 420×220. Its source components are real but its drivers are synthetic; this is not Tauri transport, provider, or clipboard evidence. Actual runtime clipboard/selection checks and multi-display overlay placement remain pending. Optional/forced/busy updater and query-clear failure branches have automated coverage; they were not all forced in the native app. The isolated app has no microphone or Accessibility grants, and George approved microphone and Accessibility grants for this isolated app. Computer Use refuses the protected UserNotificationCenter microphone alert and LocalAuthenticationRemoteService authentication UI. Neither grant has been observed; manual system actions remain necessary despite authorization.

## Release policy

George explicitly waived all benchmarks and hosted PR CI for this delivery. Neither is represented as passing. The signed release workflow, exact artifact/manifest verification, post-publication OTA canary, and installed-build checks remain required. This receipt does not claim release completion.

Detailed task-local reports, raw test results, hashes, synthetic fixture verification, and the 24-ticket/96-criterion ledger are retained in `/tmp/murmur-epic-877/` on the trusted Mac.
