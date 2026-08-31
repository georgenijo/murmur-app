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
- `--ui-*` variables are component-system contracts. Most are stable geometry
  and type values: caption/body scale, spacing rhythm, control heights, radii,
  chrome inset, history density, elevation, and motion. Dashboard color aliases
  are the exception: CSS derives them from the active repaired `--murmur-*`
  palette and never persists them as theme data.

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

`components/ui/DashboardPrimitives.tsx` owns bounded variants for dashboard
surfaces, actions, navigation rows, section headings, statistic groups, and
secondary-page headers. Feature callers choose a named variant and cannot pass
arbitrary style classes through the primitive API. `components/ui/DayChart.tsx`
owns the day-summary bar, line, and heatmap structures, including keyboard
targets, a fixed tooltip region, a separate plot region, and a separate weekday
axis.

The reusable CSS contracts are:

- `.ui-window-header` / `.ui-window-wordmark`
- `.ui-status-chip`
- `.ui-record-pill`
- `.ui-icon-button`
- `.ui-filter-chip`
- `.ui-dashboard-surface` / `.ui-dashboard-action`
- `.ui-main-nav-item` / `.ui-workspace-page-header`
- `.ui-dashboard-section-header` / `.ui-dashboard-stats`
- `.ui-day-chart-tooltip` / `.ui-day-chart-plot` / `.ui-day-chart-axis`
- `.history-toolbar` / `.history-search` / `.history-list`
- `.transcript-card` / `.transcript-meta` / `.transcript-text`
- `.transcript-copy-feedback` / `.transcript-teach`
- `.home-sidebar`
- `.home-dashboard` / `.home-dashboard-grid`
- `.home-recording-bar` with shared primary and secondary dashboard actions
- `.personalization-card`

Use these before creating a feature-local control or surface.

## Non-negotiable layout invariants

| Area | Contract |
|------|----------|
| Native chrome | Traffic lights, status, updates, and Settings share one row; recording actions live in the dashboard |
| Status | Minimum 72px; state changes do not remove its container |
| Record | Home owns one 64px row with a stable Start/Stop action, truthful status, shortcut, and file action |
| Sidebar | 160px normally, 56px icon rail below 760px; it contains only Home, Notetaker, Queries, and Insights; selection is a surface state, never elevation |
| Customize | The gear and `⌘,` open one ordered four-row hub; detail pages expose Back to Customize and restore the originating row's focus |
| Content | Uses the whole window with 24px desktop and 16px compact insets; no centered max-width shell |
| Toolbar | 28px controls; search is 180px and expands to 260px on focus |
| History | 5px list gap; cards use an 8px vertical inset |
| Metadata | Time/source only; word count and duration stay out of transcript rows |
| Copy | The non-interactive card surface copies its full transcript; Enter and Space do the same when focused |
| Correct & Teach | Compact muted action on the newest entry only |
| Insights | Durable local analytics use the full content width; Voice Query totals/providers use fixed label/value columns and notes occupy separate rows |
| Secondary pages | Insights, Notetaker, and Queries use one page header with a visible Back action that returns focus to Home navigation |
| Charts | Tooltip, plot, and axis are separate regions; each day is keyboard reachable, hover/focus/click share exact values, Escape or blur dismisses, and tooltips never move the plot |

## Review and verification

Every main-dashboard change should be checked at the native 880×720 size and
the compact 720×560 target in both appearance modes. Home and Insights must also
be checked with the low-contrast and high-saturation imported-theme fixtures.
Recording transitions must be checked in idle, recording, and processing
states. Native title-bar work must be verified in a bundled Tauri app because a
browser cannot reproduce the macOS traffic-light layout.

Component tests enforce the stable header contracts and confirm click-anywhere
copy, keyboard copy, and nested-action isolation. Native smoke testing covers Settings
navigation, transcript-card actions, and the history overflow menu.

`npm run test:visual` runs Playwright goldens at 880×720 for light and dark
appearances across idle, recording, processing, Insights, and Settings, plus
720×560 compact Home and Insights fixtures. Its dashboard theme matrix renders Home and
Insights with Sonic light/dark and repaired low-contrast/high-saturation
Open VSX-like inputs. The 880×720 fixture must retain a 200px Insights rail
beside the history column. Update those baselines only after comparing the
rendered fixture with the Open Design source and repeating the bundled native
smoke test.
