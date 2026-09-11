# Voice Query follow-up verification — #733

Implementation commit: `a88e0c80c0e8dc1cb1c0b65ee87a9e96240270b4`  
Base: `bf6c1aca`

The PR description records the evidence commit.

The change adds an explicit Ready-popover follow-up using a fresh query pass and direct CLI child. Only the immediately preceding original question and answer join the new question and optional context in one final argument. Fixed arguments remain separate. The original 32 KiB question limit and final prompt cap remain enforced without truncating the prior exchange. History, telemetry and usage schemas do not gain conversation content or fields.

## Automated checks

| Check | Result |
| --- | --- |
| Rust formatting and canonical `cargo check` | Passed |
| Canonical Rust suite, `--test-threads=1` | 1,410 passed; 10 existing ignored |
| TypeScript | Passed |
| Frontend Vitest | 144 files; 1,379 tests passed |
| Reference and Markdown validation | 31 passed; 203 registered commands documented |
| Independent Astra review | Clean after request-authorization and cancellation-ordering fixes |

Canonical Cargo commands run from `app/src-tauri`, loading its checked-in macOS deployment target and Swift rpath. A prior root-directory `--manifest-path` invocation missed that configuration and failed before test execution because the Swift runtime could not be loaded. Canonical checks passed without a DYLD override; the loader failure was not a code-test failure.

Coverage includes byte-exact literal argv, UTF-8 limits at and beyond the caps, optional-context contribution to the final cap, one-shot review authorization, stale and duplicate allocation refusal, cancellation before allocation and before Connecting delivery, shortcut-disable/stop/dismissal races, exact-owner publication, ephemeral state clearing and independent retained history.

## Synthetic direct-child evidence

[synthetic-cli-history-receipt.json](synthetic-cli-history-receipt.json) exports values from the passing `follow_up_two_real_children_reflect_prior_turn_and_keep_independent_history` test after all its assertions. The ephemeral export patch was removed; tracked source was subsequently confirmed clean at the implementation commit.

Both turns used a new `ManagedChild::spawn_user_cli` child with synthetic input. The fixture recorded one separate `--fixed-argument`, a temporary receipt-file argument and one final literal prompt. The second prompt contains:

- Previous question: `Name a color.`
- Previous answer: `indigo`
- New question: `What color did you just name?`

The second child answered `The previous answer was indigo.` The receipt contains two independent version-1 history entries whose questions are the original new questions, with no conversation or composite-prompt field. Temporary receipt-file paths are redacted without changing argv array lengths. The child's recorded arguments are `sys.argv[1:]`; its Python launcher flags and source are listed separately as `configuredFixedArguments`.

This establishes the tested direct-child prompt/history contract. It does not establish microphone capture, local ASR, native shortcut operation or accepted native follow-up handoff.

## Native observations

The original native `.app` built and launched on the MacBook. [native-original-preflight.png](native-original-preflight.png) records its general Accessibility unavailable banner. Settings separately reported that the global query shortcut requires Accessibility permission, despite relaunch and a user-reported TCC allow setting. A preceding live microphone attempt stalled before first PCM. The actual app's reported state remains the verification boundary.

Native bundle creation succeeds. Subsequent updater packaging reports a missing updater private signing key, so this is evidence of a local app build and launch, not a signed update package or release.

A temporary Ready seed supplied the synthetic question `Name one memorable color for our project.` in place of unavailable capture/ASR input. Ordinary `/usr/bin/printf` ran through the real direct-provider, completion, popover and history paths and returned `The code word is indigo. It is a blue-violet color.` Automatic copying remained off.

The actual **Ask follow-up** button was fully visible and clicked through native Computer Use. It displayed `Could not start a follow-up. Check that Voice Query is enabled, then try again.` The Ready answer remained visible. The popover, command and listener guards were unchanged; no OS permission was bypassed. For native window targeting, the fixture hid the main and overlay windows, leaving the real query-review window visible. Production Murmur was not running during this fixture check.

| Artifact | Verified observation |
| --- | --- |
| [native-ready-fixture.png](native-ready-fixture.png) | Synthetic question completed through the ordinary direct provider and appeared in the Ready popover |
| [native-follow-up-gate.png](native-follow-up-gate.png) | Actual Ask follow-up click respected the unavailable-listener guard and retained the answer |
| [native-history-after-refused-follow-up.json](native-history-after-refused-follow-up.json) | Read-only SQLite check found exactly one original question/answer record, the existing nine columns, and no extra record after the refusal |
| [native-query-history.png](native-query-history.png) | Rebuilt original app shows exactly one of 200 Queries entries: the original question and indigo answer, marked Manual copy, with no extra record |

The temporary seed was removed with its exact reverse patch, and tracked source was confirmed clean at `a88e0c80c0e8dc1cb1c0b65ee87a9e96240270b4`. That original source was rebuilt and launched, and its actual Queries UI was inspected to verify the single retained record. The dev app was then quit. The original dev profile, WebKit storage and cache were restored byte-for-byte, and `/Applications/Murmur.app` was relaunched. The completed fixture check is native partial evidence; it does not establish successful native two-turn capture, transcription or handoff.

## Draft gate

The complete native acceptance flow remains unverified: microphone first PCM, local transcription of both spoken questions, an accepted follow-up handoff through the actual popover, and the second answer reflecting the first exchange. Keep the PR draft until those observations are established. No OS permission or application listener guard has been bypassed.
