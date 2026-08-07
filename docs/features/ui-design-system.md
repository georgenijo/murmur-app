# UI design system

Murmur's front end uses one constrained geometry layer across the main
workspace, Settings, onboarding, editors, diagnostics, dialogs, and auxiliary
surfaces. The Open Design package for the redesign is the visual source of
truth; this document records the code contract that keeps later work from
drifting away from it.

## Token ownership

`app/src/styles.css` has two distinct token groups:

- `--murmur-*` variables are semantic appearance colors. The appearance engine
  may update these at runtime for System, Light, Dark, and custom themes.
- `--ui-*` variables are stable geometry and type contracts: caption/body
  scale, spacing rhythm, control heights, radii, chrome inset, history density,
  elevation, and motion.

Feature components must not replace semantic colors with literal palette
values. New recurring measurements belong in `--ui-*`; one-off arbitrary
Tailwind measurements in redesigned surfaces should be treated as a review
failure.

## Shared primitives

`components/ui/WindowHeader.tsx` owns the native-overlay title bar. It reserves
the macOS traffic-light inset and fixes the height and typography used by the
main window, onboarding, and Settings editors. Production never renders fake
traffic lights.

The reusable CSS contracts are:

- `.ui-window-header` / `.ui-window-wordmark`
- `.ui-status-chip`
- `.ui-record-pill`
- `.ui-icon-button`
- `.ui-filter-chip`
- `.history-toolbar` / `.history-search` / `.history-list`
- `.transcript-card` / `.transcript-meta` / `.transcript-text`
- `.transcript-copy` / `.transcript-teach`

Use these before creating a feature-local control or surface.

## Non-negotiable layout invariants

| Area | Contract |
|------|----------|
| Native chrome | Traffic lights, wordmark, status, hint, Record, and Settings share one row |
| Status | Minimum 72px; state changes do not remove its container |
| Record | Minimum 72px border-box width; timer remains mounted and changes visibility |
| Toolbar | 28px controls; search is 180px and expands to 260px on focus |
| History | 5px list gap; cards use an 8px vertical inset |
| Metadata | Time/source left; words/duration flush right |
| Copy | Absolutely positioned at card bottom-right; never participates in metadata layout |
| Correct & Teach | Compact muted action on the newest entry only |
| Footer | Inline statistics in a single 32px row |

## Review and verification

Every UI change should be checked at the canonical 880×720 main-window size in
both appearance modes. Recording transitions must be checked in idle,
recording, and processing states. Native title-bar work must be verified in a
bundled Tauri app because a browser cannot reproduce the macOS traffic-light
layout.

Component tests enforce the stable header contracts and confirm that Copy is
outside the transcript counts row. Native smoke testing covers Settings
navigation, transcript-card actions, and the history overflow menu.

`npm run test:visual` runs Playwright goldens at the canonical 880×720 size for
light and dark appearances across idle, recording, processing, and Settings.
Update those baselines only after comparing the rendered fixture with the Open
Design source and repeating the bundled native smoke test.
