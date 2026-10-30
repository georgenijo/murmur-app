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
| Sidebar | Home uses 200px normally and 184px at widths of 760px or less. Settings uses the separate #848 contract below: 240px / 56px rail. Home contains Home, Notetaker, Assistant, and Insights; selection is a surface state, never elevation |
| Customize | The gear and `⌘,` open one ordered five-row hub; detail pages expose Back to Customize and restore the originating row's focus |
| Content | Home uses the whole window with 24px desktop and 16px compact insets. Settings uses the 896px shell documented below |
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

## T3 Code settings geometry (#848)

Settings uses the pinned T3 Code `cba30591f9` geometry by default. The tokens
in `styles/settings.css` map onto the active `--murmur-*` colors; Sonic,
OpenCode, imported themes, library ownership, and appearance persistence remain
owned by the appearance engine. No T3 palette is installed over a saved theme.
The search-match colors below are the explicit highlight exception. Settings
scopes the existing header paint-boundary token to its own 240/56px navigation;
Home and native window lifecycle remain separate.

| Tokens / component | Contract |
|---|---|
| `--settings-nav-width`, `--settings-nav-rail` | 240px; 56px below a 768px viewport; 1px end border |
| `--settings-nav-group-pad`, `--settings-nav-group-gap`, `--settings-nav-item-gap` | 8px group inset, 16px between groups, 4px between items |
| Navigation labels | 12px/500 group headings; muted role at 80%; 14/20/500 items, 32px tall, 8px radius, 10px horizontal inset, 8px icon gap, 16px icons; selected uses the card/foreground roles |
| `--settings-search-radius` | Search is 32px tall with 6px corners; a labelled search button opens it from the narrow rail |
| `--settings-content-max`, `--settings-content-x`, `--settings-content-top`, `--settings-content-bottom` | 896px maximum including padding; 24px horizontal (20px below 640px), 24px top, 48px bottom |
| `--settings-section-gap`, `--settings-heading-gap`, `--settings-heading-height` | 32px between sections; 10px heading-to-card; heading 16/24/600, minimum 28px, horizontal inset 16px |
| Page title | One H1 in the content top bar; section headings are H2. The existing #853 page-title contract remains 20/26/600 |
| `--settings-card-border`, `--settings-card-bg`, `--settings-card-shadow` | 12px radius; 1px border at 60%; card role at 40%; `0 1px 2px rgb(0 0 0 / 5%)`; no card-body padding; plain groups have a 4px gap |
| `--settings-row-pad`, `--settings-row-x`, `--settings-divider` | 12px vertical / 16px horizontal (12px below 640px); square row corners; subsequent rows have a 1px top divider at 50%, across the full interior card width |
| `--settings-copy-gap`, `--settings-description-max` | Title 14/20/500; description 12/18/400, 4px below, max 672px, balanced wrapping, no clamp |
| `--settings-control-gap`, `--settings-control-min` | Grid `minmax(0,1fr) minmax(160px,auto)`, 32px gap; controls align right with 8px between them |
| `--settings-control-max` | Picker content is bounded at 320px so long option labels and microphone disclosures cannot consume the copy column |
| Narrow row | Named inline-size container query below 512px of row content; one column, 12px gap, full-width control area. This follows T3's padded container measurement rather than a viewport breakpoint |
| `--settings-reset-size`, `--settings-reset-icon` | Reserved 20×20px slot; changed values show a 12px Undo in a 4px-radius ghost button with “Reset to default” tooltip. Resets call the existing owner handler, including shortcut/provider validation |
| `--ui-disabled-opacity`, `--settings-unavailable-bg` | Disabled rows 64% with muted copy; unavailable group uses muted surface at 20%, 12/18 notice and 14px warning icon; opacity applies once |
| `--ui-switch-*` | Standard 30×18px / 14px knob / 2px inset / 12px travel; small 26×16 / 12 / 2 / 10. Primary when on, input when off, background knob; 200ms track and 150ms slide, pressed knob stretches to 110% |
| `--ui-control-standard`, `--ui-control-compact`, `--ui-control-xs`, `--ui-select-min` | Select and button sizes 32/28/24px; select minimum 144px, 14px text/chevron (chevron 70%) |
| `--ui-input-height`, `--ui-control-inset`, `--ui-control-border`, `--ui-control-bg` | Input 30px; inputs/selects use 11px horizontal inset, 8px radius, 1px input-role border; background role in light, input role at 32% in dark |
| `--ui-control-shadow`, `--ui-control-focus` | 5% shadow-xs plus inner `0 1px` at black 4% (dark: `0 -1px` white 6%); focus border uses primary/ring and a 3px ring at 24% |
| Buttons | 8px radius, 1px border, 14/20/500; standard/small/xs horizontal padding `calc(12px - 1px)` / `calc(10px - 1px)` / `calc(8px - 1px)`; pressed scale 0.97 |
| `--ui-segment-*` | Segmented control 32px / 8px radius; selected background role and shadow-xs 10%, dark input role at 72% |
| `--settings-search-pulse-*` | Destination pulses twice, 650ms ease-in-out, inset 2px primary ring at 70%; reduced motion suppresses the pulse |
| `--settings-search-match-*` | Light `oklch(.915 .11 85)` / `oklch(.216 .028 77.4)`; dark `oklch(.55 .1 85)` / `oklch(.970 0 0)` |

`SettingsRow`, `SettingsCard`, and `SettingsUnavailableGroup` own row/card
presentation. `SettingToggle` uses that row and reserves reset space even for
unchanged values. `FormControls` wraps Base UI Button/Input; the existing Select
and Base UI AnimatedSwitch consume the same control tokens. Legacy action and
input elements receive the shared geometry through workspace-scoped CSS;
feature behavior and folding are not restructured by this change.

`settings-measurements.spec.ts` asserts the computed styles of these production
components, including both sides of the container-query boundary, reset/disabled
states, input focus, active buttons, semantic colors, and search motion/highlights.
`settings-rhythm.spec.ts` measures real cards across ten default/expanded panes.
`settings-geometry.spec.ts` navigates the real SettingsPanel for 22 destinations
in both modes. It complements the existing imported-theme contrast matrix and
all-pane overflow audit. Goldens are kept separately for Darwin and Linux;
Linux CI uses `mcr.microsoft.com/playwright:v1.62.1-noble`. These browser checks
do not replace the lead's serial native WKWebView all-pane and Retina checks.

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
