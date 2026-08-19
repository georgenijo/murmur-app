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

The type ramp is intentionally limited to three reusable sizes: 12px captions,
13px controls, and 14px body text. Components should use those tokens instead
of inventing local font sizes.

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
- `.home-sidebar` / `.home-nav-item`
- `.home-dashboard` / `.home-dashboard-grid`
- `.home-recording-bar` / `.home-record-button`
- `.dashboard-card` / `.personalization-card`

Use these before creating a feature-local control or surface.

## Non-negotiable layout invariants

| Area | Contract |
|------|----------|
| Native chrome | Traffic lights, status, updates, and Settings share one row; recording actions live in the dashboard |
| Status | Minimum 72px; state changes do not remove its container |
| Record | Home owns one 64px row with a stable Start/Stop action, truthful status, shortcut, and file action |
| Sidebar | 216px normally, 64px icon rail at compact width; every item routes to a real surface |
| Content | Uses the whole window with 24px desktop and 16px compact insets; no centered max-width shell |
| Toolbar | 28px controls; search is 180px and expands to 260px on focus |
| History | 5px list gap; cards use an 8px vertical inset |
| Metadata | Time/source left; words/duration flush right |
| Copy | Absolutely positioned at card bottom-right; never participates in metadata layout |
| Correct & Teach | Compact muted action on the newest entry only |
| Insights | Counts come from durable local stats; personalization uses explicit milestones and never an opaque score |

## Review and verification

Every main-dashboard change should be checked at 1180×760 and the compact
720×560 target in both appearance modes. Recording transitions must be checked
in idle, recording, and processing states. Native title-bar work must be
verified in a bundled Tauri app because a browser cannot reproduce the macOS
traffic-light layout.

Component tests enforce the stable header contracts and confirm that Copy is
outside the transcript counts row. Native smoke testing covers Settings
navigation, transcript-card actions, and the history overflow menu.

`npm run test:visual` runs Playwright goldens at 1180×760 for light and dark
appearances across idle, recording, processing, Insights, and Settings, plus a
720×560 compact Home fixture. Update those baselines only after comparing the
rendered fixture with the Open Design source and repeating the bundled native
smoke test.
