# Homepage redesign session (Sona UI) — handoff

Status: design exploration, not shippable. Three fixture-only variants exist so a
direction can be picked. The recommended path is a hybrid ported into the
existing components, not promoting any variant file.

## What is on this branch

- `app/src/components/ui/` — newly installed Sona components: fluid-tabs,
  animated-dropdown, bubble-up-button, activity-graph, smart-overflow,
  spotlight-card, expanding-action, ripple-button (plus the earlier
  animated-switch, fluid-tooltip, hold-to-delete-button).
  `activity-graph.tsx` has one local patch (`dataDates.at(-1)` replaced with an
  index lookup; tsconfig lib is ES2020).
- `app/src/components/home/redesign/` — `VariantA.tsx`, `VariantB.tsx`,
  `VariantC.tsx`, their CSS, and `types.ts` (shared props). Fixture-only.
- `app/src/visual-fixtures.tsx` — `?state=redesign-a|redesign-b|redesign-c`
  routing, and `appearance=dark` now applies the real Sonic dark palette
  (previously every "dark" fixture rendered light).
- `app/redesign-shot.mjs` and `app/redesign-interact.mjs` — Playwright capture
  scripts. Run from `app/` with the Vite dev server up on
  `http://127.0.0.1:1420`. Output goes to `/tmp/murmur-redesign/`.

View a variant:

```
cd app && npm run tauri:dev   # or just `npx vite`
http://127.0.0.1:1420/visual-fixtures.html?state=redesign-c&appearance=light
```

## The three variants

| Variant | Idea | Sona components |
|---|---|---|
| A "adopt primitives" | Keep today's layout, swap in Sona pieces | fluid-tabs, animated-dropdown, smart-overflow, bubble-up-button |
| B "rhythm" | Heatmap rail, stat tiles, compact Start control | activity-graph, spotlight-card, expanding-action, fluid-tabs |
| C "quiet focus" | No hero, flat divider list, single rail card | fluid-tabs, activity-graph, smart-overflow, ripple-button |

## Review verdict (fable judge, 2026-09-04)

Scores 1 to 5: A 11, B 8, C 13 (Sona adherence / product fit / implementation
health / component choice).

Shared defects in all three:

1. Each re-implements `HistoryPanel` inline and drops the Correct and Teach
   dialog, the newest-entry-only rule, render batching, copy notices, and
   `HoldToDeleteButton`.
2. ⌘F is dead: `redesign/types.ts` omits `focusSearchToken`.
3. Demo transcripts are generated inside the component when
   `historyEntries.length < 5`. Must move to `visual-fixtures.tsx`.
4. Each redeclares Sona bridge vars locally because `app/src/styles.css` lacks
   the bare `--primary`, `--popover`, `--popover-foreground`, `--tabs-surface`,
   `--tabs-indicator`, `--tabs-indicator-border`, `--radius-lg` that
   fluid-tabs, activity-graph, and expanding-action read at runtime.
5. `--color-danger-foreground` maps to `--murmur-on-primary` (styles.css ~219),
   but Sona menus use it as danger text on a neutral popover. C's Delete item
   renders near-white; A patched it locally with `--murmur-error`.

Per variant:

- A: per-entry Delete calls `onClearHistory` (wipes everything, no hold);
  Correct and Teach is a no-op write on every card; bubble-up-button's
  `mix-blend-difference` fill ignores `--murmur-primary`; `.variant-a-record`
  box-shadow clobbers the focus ring (no visible keyboard focus on Start);
  tab CSS targets `[data-selected]` but Base UI sets `data-active`.
- B: expanding-action makes Start a two-click disclosure; activity-graph date
  range is hardcoded to demo dates even with real data; no clear, export, or
  Correct and Teach path; spotlight-card on static tiles is decoration; rail is
  too wide at 880px.
- C: Recent/Meetings/Queries tabs duplicate the sidebar with placeholder
  content; Delete only hides into local state; h1 and "processed locally"
  subtitle removed from the main column; graph gated on 20 active days so most
  users see "sample" forever. Only variant with zero hardcoded colors and
  correct `--ui-chart-*` mapping. Best 880px read.

Interaction proofs (20 captures, zero console errors) confirmed menus, tabs,
tooltips, expansion, hover reveals, and independent list scroll all work.

## Recommended path to shippable

Hybrid: C's layout (hero row with Start pill and quiet "Transcribe file…"
link, card-less divider list, narrower rail) plus A's toolbar (fluid-tabs
capsule filter, animated-dropdown export menu, smart-overflow per row). Keep
the existing `DashboardAction` for Start (carries `data-testid`, aria label,
waveform, danger tone). Keep the h1 and "everything processed locally" line.

Small PRs, in order:

1. `styles.css`: add the bare Sona tokens listed above to the root bridge;
   remap `--color-danger-foreground` to `--murmur-error` (or add a separate
   on-danger token for filled surfaces).
2. `HistoryPanel.tsx`: replace `.history-filter-track` with
   `FluidTabs variant="capsule"`; replace the hand-rolled export popover with
   `AnimatedDropdown` keeping `HoldToDeleteButton` as the last item; replace
   the per-card button row with `SmartOverflow` (Copy primary, Correct and
   Teach in overflow, newest entry only). No per-entry Delete: no backend.
3. Adopt C's row spacing and rail width in `.home-history .transcript-card`
   and `.home-insights-rail`. Keep `DayChart`. Add `ActivityGraph` only from
   real `HistoryEntry.timestamp` data bucketed by local day (the component
   does UTC math), and only once enough active days exist.
4. Move demo data into `visual-fixtures.tsx`; delete `redesign/` once ported.
5. `cd app && npm test` — `HomeDashboard.test.tsx` record-button, waveform,
   and privacy-line assertions must stay green. `npx tsc --noEmit` too.

Adopt app-wide regardless: fluid-tabs (filters and local view switching,
never sidebar navigation), animated-dropdown (every hand-rolled menu),
smart-overflow (constrained action rows), hold-to-delete-button (only
destructive affordance), activity-graph (Insights view).

Do not adopt: bubble-up-button, ripple-button, spotlight-card,
expanding-action for anything primary.
