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

The shared small-type ramp is 11/16, 12/16, 13/18, and 14/20px, as pinned
by #853 below. Components use the corresponding tokens rather than inventing
local sizes. Prose statistics, eyebrows, and history date labels use the sans
stack with tabular numerals; monospace is reserved for technical text such as
shortcuts, code, paths, and transcript timestamps.

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

`lib/windowLifecycle.tsx` owns each workspace webview's `data-fullscreen`
attribute, initialized from the native window and refreshed on native resize.
In fullscreen, `windowLifecycle.css` sets `--ui-window-controls-inset` to zero;
ordinary windows retain the zoom-compensated inset. AppKit owns the hover-reveal
strip and restores the real controls on exit. Main and Diagnostics mount the
lifecycle in React so every native resize subscription is disposed on unmount.

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
| Native chrome | Traffic lights, Home sidebar toggle, status, updates, and Settings share one row. The toggle starts after the zoom-compensated 80pt controls inset and never acts as a drag region; recording actions live in the dashboard. Collapsed Home and narrow Settings leave the header background unpainted; expanded header paint aligns with its sidebar |
| Status | Minimum 72px; state changes do not remove its container. Home hides the title-bar chip because its talk card already states the dictation status |
| Record | Home owns one talk card with a single talk-row action and a passive month strip, a plain-language state title, and one short hint; title and hint never truncate at 880×720 or 1120×820. Transcribing a file lives in the history `···` menu |
| Sidebar | App owns Home collapsed state, resizing, and a 208–384px expanded width. Below 768 CSS px Home defaults to a 48px icon rail; explicit expansion is available. The header owns its toggle and restores focus there when a focused resize separator disappears. Sidebar heading retains the brand. Settings uses the separate #848 contract below: 208px / 56px rail. Home contains Home, Notetaker, and Insights; selection is a surface state with no shadow, a vertically centred expanded marker, and no collapsed marker |
| Customize | The gear and `⌘,` open General; Customize is an unordered set of independent destinations, and detail pages expose Back to Customize and restore the originating row's focus |
| Content | Home uses the whole window with 24px desktop and 16px compact insets. Settings uses a centered 728px shell with 680px of content |
| Toolbar | 28px controls; one line holding the page title, search, the Filter menu, and `···` at 880×720 and wider, wrapping rather than clipping when narrower; no resting entry count |
| History | Home uses 28px above and 8px below day labels; rows use an 8px vertical inset, with aligned hover fills and within-day hairlines |
| Metadata | Time/source only; word count and duration stay out of transcript rows |
| Copy | The non-interactive card surface copies its full transcript; Space does the same when focused |
| Correct & Teach | Compact muted action on the newest entry only |
| Secondary pages | Insights uses a page header with a Back action. Notetaker uses a title/capture header with sidebar navigation; see the Notetaker contract below |
| Charts | Tooltip, plot, and axis are separate regions; each day is keyboard reachable, hover/focus/click share exact values, Escape or blur dismisses, tooltips never move the plot, and heatmaps expose date-range/month/weekday labels; Insights exposes daily values in assistive-technology tables, while other heatmaps retain the Daily values disclosure |

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

`npm test` runs the UI design ratchet in `app/src/styles.test.ts`. It scans
production `.tsx` and `.css` under `app/src` for literal colors (including
Tailwind palette utilities), font sizes outside `var(--ui-font-*)`, arbitrary
pixel Tailwind values, and prose monospace. Use semantic colors and type tokens;
statistics use sans with `tabular-nums`. Monospace is permitted on `code`, `pre`,
`kbd`, and `samp` and their descendants. Other existing uses remain baselined.
CSS custom-property definitions, tests, and visual fixtures are exempt. This is
a static source scan; constructed runtime classes are not fully evaluated.

The committed `app/src/test-utils/design-lint-baseline.json` identifies each
legacy violation by file, rule, pattern, structural context, and occurrence
within that context. TSX context includes enclosing component/binding names,
element ancestry, stable attributes, and non-violating class neighbours; CSS
context includes resolved nested selectors and enclosing at-rules. Line shifts
do not change identity, and moving a violation to a different context fails. New violations fail with their file and
rule; removing a violation also fails until its stale baseline entry is removed.
After fixing violations, run `(cd app && npm run design-lint:shrink)` and commit
the baseline diff with the fix. Regeneration must **only shrink**: this command
refuses new violations rather than accepting them. Never add entries to silence
a failure. Inspect the baseline diff, then run the normal tests without the env
flag. The initial baseline records existing debt without changing rendered UI.

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

### UI PR checklist

- Check native controls clearance and the header centreline independently of sidebar width.
- Keep the shared type scale; use sans prose and tabular statistics, mono technical text.
- Exercise expanded, collapsed, narrow and attainable 0.5/1.0/1.1/1.2 zoom states.
- Compare light and dark rendering and inspect platform-specific golden diffs.
- Verify the signed bundled native app: pointer/keyboard toggle, separator focus restore, window drag and title-bar double-click; include narrow Settings at 1.1.

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
scopes the existing header paint-boundary token to its own 208/56px navigation;
Home and native window lifecycle remain separate.

| Tokens / component | Contract |
|---|---|
| `--settings-nav-width`, `--settings-nav-rail` | 208px; 56px below a 768px viewport; 1px end border |
| `--settings-nav-group-pad`, `--settings-nav-group-gap`, `--settings-nav-item-gap` | 8px group inset, 16px between groups, 4px between items |
| Navigation labels | 12px/500 group headings; muted role at 80%; 13/18/500 items, 32px tall, 8px radius, 10px horizontal inset, 8px icon gap, 16px icons; selected uses the card/foreground roles |
| `--settings-search-radius` | Search is 32px tall with 6px corners; a labelled search button opens it from the narrow rail |
| `--settings-content-max`, `--settings-content-x`, `--settings-content-top`, `--settings-content-bottom` | 728px maximum including padding (680px content), centered; 24px horizontal (20px below 640px), 24px top, 48px bottom |
| `--settings-section-gap`, `--settings-heading-gap`, `--settings-heading-height` | 32px between legacy sections; 16px between named card groups; 10px legacy heading-to-card; group labels 11/16/500, muted, 16px horizontal inset and 6px below; legacy section-head wrappers retain their 28px minimum |
| Page title | One H1 in the content top bar; section headings are H2. Settings uses 18/24/600 |
| `--settings-card-border`, `--settings-card-bg`, `--settings-card-shadow` | 12px radius; 1px border using the muted foreground at 17%; card role at 40%; no shadow; no card-body padding; plain groups have a 4px gap |
| `--settings-row-pad`, `--settings-row-x`, `--settings-divider` | 7px vertical / 16px horizontal (12px below 640px); 52px minimum row height; square interior row corners; subsequent rows have a 1px top divider using the same 17% hairline, across the full interior card width |
| `--settings-copy-gap`, `--settings-description-max` | Title 13/18/500; description 12/16/400, 2px below, max 672px, pretty wrapping, no clamp |
| `--settings-control-gap`, `--settings-control-min` | Grid `minmax(0,1fr) minmax(160px,auto)`, 32px gap; controls align right with 8px between them |
| `--settings-control-max` | Picker content is bounded at 320px so long option labels and microphone disclosures cannot consume the copy column |
| Narrow row | Named inline-size container query below 512px of row content; one column, 12px gap, full-width control area. This follows T3's padded container measurement rather than a viewport breakpoint |
| `--settings-reset-size`, `--settings-reset-icon` | Reserved 20×20px slot; changed values show a 12px Undo in a 28×28px, 4px-radius ghost button without changing the slot with “Reset to default” tooltip. Resets call the existing owner handler, including shortcut/provider validation |
| `--ui-disabled-opacity`, `--settings-unavailable-bg` | Disabled rows 64% with muted copy; unavailable group uses muted surface at 20%, 12/18 notice and 14px warning icon; opacity applies once |
| `--ui-switch-*` | Standard 30×18px / 14px knob / 2px inset / 12px travel; small 26×16 / 12 / 2 / 10. Primary when on, input when off, background knob; 200ms track and 150ms slide, pressed knob stretches to 110% |
| `--ui-control-standard`, `--ui-control-compact`, `--ui-control-xs`, `--ui-select-min` | Select and button sizes 32/28/24px; select minimum 144px, 14px text/chevron (chevron 70%) |
| `--ui-input-height`, `--ui-control-inset`, `--ui-control-border`, `--ui-control-bg` | Input 30px; inputs/selects use 11px horizontal inset, 8px radius, 1px input-role border; background role in light, input role at 32% in dark |
| `--ui-control-shadow`, `--ui-control-focus` | 5% shadow-xs plus inner `0 1px` at black 4% (dark: `0 -1px` white 6%); focus border uses primary/ring and a 3px ring at 24% |
| Buttons | 8px radius, 1px border, 14/20/500; standard/small/xs horizontal padding `calc(12px - 1px)` / `calc(10px - 1px)` / `calc(8px - 1px)`; pressed scale 0.97 |
| `--ui-segment-*` | Settings control: 34px sunken pill track, 2px padding and gap; 28px selected control with 8px radius, raised fill and `--ui-shadow-1`, matching Insights |
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
  and native macOS menu title case. Named features such as
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

## Settings progressive disclosure (#849)

The behavior layer, complete 22-pane before/after audit, search/permission
boundaries, UI-only persistence contract and verification map are documented in
[Settings progressive disclosure](settings-progressive-disclosure.md). Geometry
continues to use the #848 measurements above.

## Approved Home and secondary controls

`styles/buttons.css` owns secondary-button paint for DashboardAction secondary,
FormControls outline, meeting header actions, Settings quiet/back/header actions,
and secondary dialog/model actions. New consumers can use `.ui-secondary-button`.
Use this family rather than defining a local border or fill. It has a 1px
`--ui-control-border` border, `--ui-control-bg` fill, on-surface text,
8px control radius and `--ui-control-shadow`. Hover/pressed use dashboard
hover/pressed tokens. Disabled applies 0.64 opacity once, retaining resting
paint with no shadow, transform or hover feedback. Focus is a 2px primary
outline with a 2px offset. Keep 28px compact and 32px standard geometry.

Home uses one scroll range and a 208px initial/minimum expanded sidebar.
Its footer is the one-line “Audio and transcripts stay local.”; the icon rail
retains the full accessible note. The talk row owns Start/Stop dictation and
recording tint; its neutral border and passive month summary do not react to
recording or hover. Empty Home omits the usage strip. Only confirmed microphone
denial replaces the talk row with an inline recovery action; unknown and
not-determined permission still allow the native first-use prompt.

Home fixture states are `idle`, `recording`, `processing`, `home-empty`,
`home-mic-denied`, and `home-sidebar-collapsed`. Use an actual pointer hover on
the second transcript row for the row-action state; talk-row hover is exercised
separately. Fixtures run without capture-time CSS or DOM rewriting. Verify each
appearance at 1100×780 and 840×620; the combined native build remains the final
manual acceptance gate.

The shared secondary defaults use `[class]:where(...)` with specificity 0,1,0, keeping the element-level
font reset below the shared defaults. Geometry
rules must not assign a neutral border color to shared or destructive consumers. Explicit danger consumers
(`data-tone="danger"`, confirming/destructive variants, `.settings-modes-danger`
and error text utilities) receive shared geometry/focus and retain their own
paint at rest, on hover and when armed. Computed-style coverage exercises actual
meeting deletion, mode deletion, knowledge actions and General setup/reset
buttons in both appearances.

### Notetaker redesign

The Notetaker title and capture action share a 32px header. Its status area is
57px high at standard widths and 75px at window widths of 900px or less,
independent of permission/capture state. Idle markers are neutral; only live
source flags use their semantic channel colors. The three source/access markers
share an 8px dot. Check access has a 24px visual height and 28px pointer target.

The list is 264px wide, or 208px in compact windows. Detail headings use
16/22px; the page title uses 18/24px. Copy, Export, More, Speakers, the player,
and transcript rules share the detail pane's right edge. Secondary actions use
the shared FormControls appearance; feature styles own geometry only. Compact
headers reserve a separate action row and give Copy a labelled 28px icon target.
The player is 40px high. Transcript rows reserve 56px for time and 48px for the
speaker, separated by 8px gaps, with 8px vertical insets. Play glyphs reveal on
hover or keyboard focus in an absolute slot, keeping timestamps aligned.

### Approved Settings redesign

Settings uses one muted 11/16px section-heading style and grouped cards on
General, Shortcuts, and Appearance; other pages retain their content with the
same shared row/card/control-column geometry. The page title is 18/24px. The window title is centered
independently of navigation width. Done consumes the shared secondary button.
Settings' segmented control is local CSS: a 34px sunken pill track, 2px padding,
28px selected surface with an 8px radius and `--ui-shadow-1`, matching Insights.
The redesign's browser fixture evidence covers 1100×780 and 840×620 in both
appearances. Native verification remains a separate integration requirement.
