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

`components/ui/AppSurface.tsx` owns the sans stack at the main, onboarding,
and Diagnostics roots. It resolves `--ui-font-sans` directly so an old font
utility cannot override the application stack. Nested code still uses the
explicit mono token. Home controls retain the shared 8px radius.

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
- `.home-talk` (the Home talk card) / `.history-filter-trigger` / `.history-filter-menu`
- `.personalization-card`

Use these before creating a feature-local control or surface.

## Non-negotiable layout invariants

| Area | Contract |
|------|----------|
| Native chrome | Traffic lights, status, updates, and Settings share one row; recording actions live in the dashboard |
| Status | Minimum 72px; state changes do not remove its container. Home hides the title-bar chip because its talk card already states the dictation status |
| Record | Home owns one talk card, at most 56px tall, with a single 36px Start/Stop button, a plain-language state title, and one short hint; title and hint never truncate at 880×720 or 1120×820. Transcribing a file lives in the history `···` menu |
| Sidebar | Home and Settings share 200px normally and 184px at widths of 760px or less. Labels remain visible; switching surfaces changes the destinations, not the column boundary. Home contains Home, Notetaker, Assistant, and Insights; selection is a surface state, never elevation |
| Customize | The gear and `⌘,` open one ordered five-row hub; detail pages expose Back to Customize and restore the originating row's focus |
| Content | Uses the whole window with 24px desktop and 16px compact insets; no centered max-width shell |
| Toolbar | 28px controls; one line holding the page title, search, the Filter menu, and `···` at 880×720 and wider, wrapping rather than clipping when narrower; no resting entry count |
| History | 5px list gap; cards use an 8px vertical inset |
| Metadata | Time/source only; word count and duration stay out of transcript rows |
| Copy | The non-interactive card surface copies its full transcript; Enter and Space do the same when focused |
| Correct & Teach | Compact muted action on the newest entry only |
| Insights | Durable local analytics use the full content width; Voice Query totals/providers use fixed label/value columns and notes occupy separate rows |
| Secondary pages | Insights, Notetaker, and Assistant use one page header with a visible Back action that returns focus to Home navigation; Assistant retains legacy query history in Previous queries |
| Assistant | Flat transparent reading surface, no nested message shading; one shared left text gutter across speaker labels, bodies, header and composer text. Compact thread rail uses an outlined selection; connection and delete controls live in keyboard-accessible disclosures. Escape dismisses an open disclosure before canceling a turn. |
| Voice Query | Theme-aware flat popover, question then answer on one text gutter; compact state header with Close, bounded scrolling body and pinned actions. Technical summaries live in Details rather than the header or footer. Open in Assistant is one explicit handoff, not a nested save dialog. |
| Charts | Tooltip, plot, and axis are separate regions; each day is keyboard reachable, hover/focus/click share exact values, Escape or blur dismisses, and tooltips never move the plot |

## Input contracts

Destructive hold controls accept only a primary pointer with matching pointer
ownership, or Enter/Space. Two discrete activations arm then confirm; click-only
assistive activation follows the same path without requiring a physical press.
Cancellation disarms rather than confirming. Generated clicks from an already
handled press must not add a second activation.

Settings range controls have an associated label. Paste Delay exposes its draft
value in milliseconds and coalesces pointer-drag steps into one settings commit
on release. Blur, pointer cancellation/capture loss, window blur, and unmount
also save the final visible draft once. Keyboard and assistive value changes
without a pointer drag persist immediately.

## Review and verification

Every main-dashboard change should be checked at the native 1120×820 default,
the 880×720 golden size, and the compact 720×560 target in both appearance modes. Home and Insights must also
be checked with the low-contrast and high-saturation imported-theme fixtures.
Recording transitions must be checked in idle, recording, and processing
states. Native title-bar work must be verified in a bundled Tauri app because a
browser cannot reproduce the macOS traffic-light layout.

Component tests enforce the stable header contracts and confirm click-anywhere
copy, keyboard copy, and nested-action isolation. Native smoke testing covers Settings
navigation, transcript-card actions, and the history overflow menu.

`npm run test:visual` runs Playwright goldens at 880×720 for light and dark
appearances across idle, recording, processing, Insights, and Settings, plus
720×560 compact Home and Insights fixtures; a layout test also checks the talk
card and history toolbar at the 1120×820 window default. Its dashboard theme matrix renders Home and
Insights with Sonic light/dark and repaired low-contrast/high-saturation
Open VSX-like inputs. The 880×720 fixture must retain a 200px Insights rail
beside the history column. Update those baselines only after comparing the
rendered fixture with the Open Design source and repeating the bundled native
smoke test.

## T3 Code typography and floating surfaces (#853)

T3 Code at `cba30591f9` supplies the typography and floating-surface
measurements. Semantic colors still resolve exclusively through `--murmur-*`;
no appearance document or palette changes are part of this contract.

| Contract | Rendered value |
|---|---|
| Sans stack | `-apple-system, BlinkMacSystemFont, system-ui, sans-serif` |
| Mono stack | `"SF Mono", "SFMono-Regular", Menlo, monospace` |
| Small type | 14/20, 13/18, 12/16, 11/16 px; long-form prose may opt into its own leading |
| Weight | 600 for headings and statistics, 500 for labels and controls |
| Tracking | 0 below 18px, including small uppercase labels; -0.01em for large titles; shortcut hints alone use 0.1em |
| Page titles | 20px / 26px / 600; whole-pixel leading avoids fractional scroll-edge clipping |
| Lucide | 1.75 stroke, 16px default, 14px in compact controls; size utilities can retain larger illustrations |
| Numeric / prose | Tabular counters and statistics; pretty paragraph wrapping; balanced tooltips |
| Radius | 8px controls/popovers, 12px cards, 4px menu items, 6px tooltips; explicit pill variants remain pills |
| Control height | Existing 28px compact / 32px standard |
| Filled button | White 16% inner top highlight, 5% shadow-xs, scale 0.97 while pressed |
| Bordered button padding | `calc(N - 1px)` horizontal inset compensates for the 1px border |
| Menu popup | 4px padding, 8px radius, scrolls within Base UI's `--available-height`, uses `--transform-origin`, no-drag |
| Menu item | Minimum 28px; 8px horizontal / 4px vertical padding, 8px gap, 14/20 text |
| Menu decoration | 16px muted icons at 80%; 1px separators with 8px horizontal / 4px vertical margins; disabled opacity 64% |
| Glass | 80% popover surface, 10% foreground border; light blur 12px / saturation 1.14, dark 16px / 1.08 |
| Popup shadow | Light `0 16px 40px -18px rgb(0 0 0 / 55%)`; dark `0 18px 44px -18px rgb(0 0 0 / 80%)` |
| Tooltip motion | Scale 0.98 plus opacity on entry and exit |
| Scrollbar | 6px, transparent track, 3px thumb radius; light 217 gray / hover 191 gray; dark white 8% / hover 12% |

`styles/menus.css` defines `.ui-menu-popup`, `.ui-menu-item`,
`.ui-menu-label`, `.ui-menu-separator`, `.ui-menu-shortcut`, `.ui-popover`, and
`.ui-tooltip`. AnimatedDropdown, SmartOverflow, history filters, the microphone
popover, and FluidTooltip consume these contracts. Keyboard focus uses
`:focus-visible`. The body disables overscroll. A universal reduced-motion
rule collapses CSS animations/transitions while preserving completion events;
Motion components also read `useReducedMotion`.

`MiddleTruncate` retains the end of a path, clips at whole glyphs, refits on
resize or changed text, and leaves the full value in the DOM and title. File
output and IDE-root paths use it. Its T3 MIT attribution is preserved in
`THIRD_PARTY_NOTICES.md`. Diagnostics stream chips wrap so their labels remain
reachable in compact Settings, without changing settings-row/control geometry.

`visual-tests/typography.spec.ts` renders production primitives through
`visual-fixtures.html?state=typography` and asserts computed measurements,
keyboard/pointer focus, active-button scale, menu states, scrollbar styles,
and reduced motion. The browser normalizes BlinkMacSystemFont to a quoted
system-ui alias on macOS; the test checks both the declared stack and the
browser's resolved value. `typography-overflow.spec.ts` audits small-text
tracking and horizontal clipping across every settings category/tool and AI /
Customize drill-in, plus exposed selects and history menus, at 880×720 and
720×560 in both modes. Intentional ellipsis and horizontally scrollable tab
strips are distinguished from unreachable clipped text.

Dedicated light/dark goldens cover Home (`state=idle`), General
(`state=settings-general`), Appearance (`state=settings-appearance`), and an
open Home filter menu. The existing dashboard, settings, updater and theme
matrix goldens also change with the shared typography. These are Chromium
browser fixtures, not native WKWebView evidence. CI currently runs on macOS;
Linux proof must be generated and identified separately and must never reuse
Darwin screenshots as Linux results.

Before merge, the lead must verify the exact integrated revision in an
isolated native Mac mini build, including Home, Settings → General,
Settings → Appearance, Home → Filter / history overflow, and an Insights chart
tooltip, in Light and Dark. Exercise pointer and keyboard navigation, Escape,
menu selection, compact scrolling, and Reduce Motion. Required Retina
WKWebView and native T3 side-by-side evidence remain gates until captured;
a 1× capture or browser device-scale emulation does not satisfy Retina proof.
The MacBook must not be operated for this task.

On Linux, run `npm ci`, `npx playwright install chromium`, then
`npx playwright test typography.spec.ts typography-overflow.spec.ts` from
`app/`. The dedicated `*-linux.png` baselines live beside their Darwin
counterparts. This focused Linux gate complements the existing full macOS
suite; it does not claim that macOS-only native behavior was tested on Linux.
