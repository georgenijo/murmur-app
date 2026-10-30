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
| Copy | The non-interactive card surface copies its full transcript; Space does the same when focused |
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

## Copy and interaction

Adapted from [Hex’s design system](https://github.com/anomalyco/hex/blob/a07f994/docs/design-system.md#rules)
(lines 24–131, MIT). T3 Code remains the source for visual measurements;
these rules govern Murmur’s copy and interactions.

- Use sentence case for labels, buttons, titles, and section headings. Preserve
  proper nouns (Murmur, macOS, Finder, Calendar, Open VSX, model/provider names)
  and native macOS menu title case. Named features such as Voice Query and
  Smart Auto retain their names; generic commands, modes, and settings do not.
- Use one `…` character for progress and actions that open a further choice:
  `Saving…`, `Choose folder…`, `Create knowledge…`. Use normal punctuation when
  mentioning an action inside a sentence.
- A row description is one direct phrase without a closing period. Necessary
  multi-sentence instructions, permission disclosures, and error explanations
  keep their grammar. Put help beside its control; omit page introductions,
  repeated labels, and state words beside switches. A switch’s checked state
  communicates on/off; its accessible name identifies the setting.
- Each screen has at most one primary action. Ongoing recording, cancellation,
  and update-available states use secondary actions; readiness is a status.
  Danger styling is reserved for destructive actions and armed confirmations.
- Show one to three common actions in a detail header. Move additional actions
  into the existing overflow menu. Put destructive actions last, append `…`,
  and require a second deliberate activation or confirmation. Preserve typed
  confirmation for bulk deletion. Do not add “cannot be undone” warnings beside
  an existing two-step confirmation. Editing an unsaved draft is recoverable.
- Space activates the focused button. Plain Return activates only an explicitly
  designated, enabled default confirming button in a dialog, setup step, or
  temporary recoverable confirmation. A standing pane action and an irreversible
  action are never defaults. Repeated/composing Return must not confirm. Textareas,
  contenteditable fields, native selects, and menu/listbox selection keep their
  own editing/selection keys. A destructive control still suppresses Return
  inside a menu. Existing dedicated command shortcuts retain their contracts.
- Use `defaultButtonKeyDown` on the interaction boundary. A recoverable dialog
  uses `data-default-button` on its actual confirming button; an inline
  confirmation also declares `data-default-scope`. Disabled buttons stay inert.
  `ConfirmActionButton` and `noDefaultButtonKeyDown` protect destructive controls.
  Verify actual Space/Return behavior in the browser and native app.
- Hover feedback belongs to the interactive target, never a passive settings
  row. Use shared controls and semantic colors; copy changes must not introduce
  typography or theme tokens.

See the [copy and interaction audit](../copy-interaction-audit.md) for the
issue #850 checklist, coverage, and native verification handoff.
