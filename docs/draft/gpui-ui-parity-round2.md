# GPUI UI parity — part B / round 2

Part B builds on part A (`73ea70ab`). Implementation: `17d970bb`; updater
input-containment fix: `4e181f54`. PR #1044 remains a draft stacked on
`gpui/25-identity-update-chain`; no merge, retarget, or reviewer launch.

| Item | Result |
| --- | --- |
| Insights | Shared Home navigation, adjacent Back link, four-column label/value/caption tiles, heatmap and seven-day chart visible at 880×720. |
| Notetaker | Shared navigation, status and primary Start, date-grouped cards, Delete all in overflow, compact naming/export controls, timestamp playback, coloured speaker/text grid. |
| Settings / General | Wordmark, icon navigation, bottom Tools group, Done pill, fixed heading separator, one General card in React order. |
| Customize | Description/chevron cards for exactly the four current React destinations. The older five-card golden is stale. |
| Appearance | Selected scheme check, theme-card grid, light/dark gradient orbs, selected ring, accessible variant names and icon actions. |
| Site modes | Lookup/rule toggles, inline browser/host/Add and editable rule rows, display names, quiet red removal with confirmation. |
| Onboarding | Centred column, eight-dot progress indicator, icon tile/status chip, split footer, quiet reset, retained denied-state System Settings guidance. |
| Updates | Shared centred modal, scrim, icon/version/close header, release-note body and adjacent pills. Scrim input cannot activate the workspace underneath. |
| Pill | Plain recording has a vector dot and compact waveform without a visible disclosure; quick settings remain accessible. QA root matches production transparency. |
| Part A leftovers | Active Home marker is at the row edge; imported-audio badges include a file icon. |

Verification on the Mac mini, with builds at `nice -n 15` and
`CARGO_TARGET_DIR=~/Builds/gpui-stack-target`:

- Formatting, locked check, locked Rust tests: **239 passed**.
- CI strict Clippy: `--all-targets --features gallery,production-host -- -D warnings`.
- CI accessibility harness: **371 passed**, plus the native table adapter.
- Python harness: **23 passed**.
- CI offscreen renderer: **780 cases**, two byte-identical renders each at
  `17d970bb`. The later modal-only input fix has its own regression and native
  recheck; it changes no golden-case rendering.
- Native Computer Use exercised General/Customize, theme variants and schemes,
  site toggle/add/confirmed removal, meeting naming/overflow/Escape/SubRip/copy,
  onboarding Back/Continue/Skip, pill expansion/collapse, and update dismissal.
  The updater click-through was reproduced, fixed and rechecked natively.

All **19** requested native keys, same round-1 sizes, are in
`~/Builds/gpui-council/round2/`. `manifest.json` records per-image binary hashes,
features, environment, revision and window geometry; `verification.json` and
`native-interaction.json` contain the checks. Supplemental images cover populated
live captions, SubRip export, and the dismissed updater. Captures were inspected
against the retained Darwin React goldens; text, dates, statistics and installed
collections remain synthetic. Meeting review is scrolled to expose all four
segment states, as in round 1.

Both round-1 and round-2 pill PNGs have transparent extreme corners (alpha zero);
the apparent black squares were a display/capture artifact. Round 2 also removes
the fringe by giving the QA host the same transparent root as production.

No UI spec items were skipped. The updater capture uses `update-qa`, which shares
the shipping modal and actual Home view, with service-free callbacks; it does not
start production services or download an update. Only synthetic, non-production
QA hosts were used. Task-created GUI processes stopped by exact PID and temporary
bundles/data removed; production data, pasteboard, permissions and settings were
not touched.
