# Theme Engine — Implementation Plan

**Status:** Draft plan (ready to ticket / implement)  
**Date:** 2026-07-27  
**Scope:** Local Appearance theming engine for Murmur  
**Out of band:** No code in this document — execution follows the phases below.

---

## 1. Goal

Ship a local **Theme Engine**: versioned `ThemeConfig` documents that resolve onto Murmur’s existing Sonic Canvas semantic token set (`--murmur-*`), with an Appearance Settings page, live multi-window apply, and fail-closed validation.

This is **not** ChatGPT feature-parity. It is a real engine on Murmur’s substrate: mode control, accent (then later bg/fg/contrast), presets, file import/export.

---

## 2. Product intent

| Want | Do not want (v1) |
|------|------------------|
| System / Light / Dark that actually works | Remote theme marketplace / network fetch |
| Named code-defined presets (Sonic default) | Multi-theme library CRUD / id collisions |
| Accent picker with AA fail-closed | Arbitrary UI/code font pickers |
| Later: bg/fg/contrast + file import | Translucent sidebar, dock icon themes |
| Live sync across all Tauri webviews | Clipboard “copy theme” as primary path |
| Overlay / transform island stay dark glass | Light-mode restyle of notch / review chrome |
| Pure frontend theme logic | Stuffing theme into `dictation-settings` |

---

## 3. Substrate (verified)

### 3.1 Tokens today

- Source: `app/src/styles.css`
- 14 `--murmur-*` variables on `:root` (light), dark only under `@media (prefers-color-scheme: dark)`
- `@theme inline` maps them to Tailwind v4 `--color-*` → utilities like `bg-surface`, `text-on-surface`
- Tokens: `background`, `surface`, `surface-container-low`, `surface-container`, `surface-container-high`, `surface-container-lowest`, `surface-container-highest`, `primary`, `primary-dim`, `on-primary`, `on-surface`, `on-surface-variant`, `outline-variant`, `error`
- `color-scheme: light|dark` is set alongside the palettes today
- `body.overlay-window { background: transparent }` — **must preserve** (prior opaque-box regression in CHANGELOG / Sonic reskin)

### 3.2 Settings today

- `app/src/lib/settings.ts` — flat `Settings`, `localStorage` key `dictation-settings`
- Field-by-field migration in `loadSettings`; round-trip key parity tests
- `useSettings.updateSettings` only emits `settings-changed` for a hardcoded field list
- `applyExternalSettings` early-returns unless `disabled` or `autoPaste` changed — **theme must not live here** or main will silently clobber appearance on the next unrelated write

### 3.3 Settings UI

- Pages: `recording`, `transcription`, `transform`, `text-vocabulary`, `delivery`, `performance`, `general`
- New category: **Appearance** (own sidebar entry + `SettingsSection`)
- Docs: `docs/reference/settings.md` (“seven pages”) must be updated

### 3.4 Windows (four independent webviews)

| Window | HTML | Root | CSS | Notes |
|--------|------|------|-----|-------|
| Main | `app/index.html` | `main.tsx` | `styles.css` | Decorated |
| Overlay | `app/overlay.html` | `overlay.tsx` | `styles.css` | Transparent; `overlay-window`; hardcoded dark glass |
| Transform review | `app/transform-review.html` | `transform-review.tsx` | `styles.css` | Same transparency + glass pattern |
| Log viewer | `app/log-viewer.html` | `log-viewer.tsx` | `styles.css` | Decorated; no settings sync today |

No shared React state. Sync = storage + Tauri event.

### 3.5 Dark utilities blocker

- Hundreds of `dark:` Tailwind class usages across ~25 files
- No `@custom-variant dark` today → Tailwind v4 default = `@media (prefers-color-scheme: dark)`
- Without a selector-based dark variant, forced Light/Dark is **visibly broken** (tokens flip, `dark:` utilities stay on OS)

### 3.6 Token coverage gap

- Outside overlay/transform, large amounts of hardcoded `stone-*`, amber/red/emerald, `bg-white`, etc.
- Bg/fg pickers without mapping those to semantic tokens will look broken (purple surfaces + stone chrome)
- **Token debt ticket gates P2 bg/fg**

---

## 4. Locked decisions

1. **Schema fork B** — one active `ThemeConfig` + code-defined presets + file import/export. No `customThemes[]` library UI in v1.
2. **Own storage** — `localStorage` key `murmur-appearance` (versioned document). Separate `appearance-changed` Tauri event. Do **not** add appearance fields to `Settings` / `useSettings` emit or rollback gates.
3. **Dark variant** — `@custom-variant dark (&:where([data-appearance="dark"], [data-appearance="dark"] *));` and always set concrete `data-appearance="light"|"dark"` on `<html>` (never leave as `system`).
4. **Overlay / transform glass** — remain always-dark in v1 (inline `rgba(20,20,20,0.92)` chrome untouched).
5. **Import/export** — file-based via existing dialog permissions; avoid clipboard as primary (dictation owns clipboard).
6. **Reduce-motion user override** — cut from v1 (already respects OS `prefers-reduced-motion` in CSS + JS).
7. **Color math** — OKLCH/OKLab derivation in JS, **no new npm dependency**; hand-rolled converter + fixtures.
8. **Rust** — no theme *logic*. Optional: one capability + `setTheme` so native title bars follow in-app mode (decide explicitly in P0b).
9. **Applier invariant** — resolver returns only allowlisted keys (`--murmur-*` × 14 + `color-scheme`); applier never sets `background` / `background-color` on `html`/`body`.

---

## 5. Schema

```ts
/** User preference — may be system. Never written to data-appearance. */
export type AppearanceMode = 'system' | 'light' | 'dark';

/** Concrete resolved mode — always written to data-appearance. */
export type ResolvedAppearance = 'light' | 'dark';

export type MurmurTokenName =
  | 'background'
  | 'surface'
  | 'surface-container-low'
  | 'surface-container'
  | 'surface-container-high'
  | 'surface-container-lowest'
  | 'surface-container-highest'
  | 'primary'
  | 'primary-dim'
  | 'on-primary'
  | 'on-surface'
  | 'on-surface-variant'
  | 'outline-variant'
  | 'error';

export type MurmurTokens = Record<MurmurTokenName, string>; // #rrggbb

/** Active theme document (persisted). Presets are code-defined, not stored. */
export interface ThemeConfig {
  version: 1;
  /** Preset id (e.g. "sonic") or "custom" when user overrides / imported. */
  presetId: string;
  /** Optional accent override (#rrggbb). Derives primary / primary-dim / on-primary. */
  accent?: string;
  /** P2 only — gated on token debt. */
  background?: string;
  foreground?: string;
  /** P2 only — 0–100, bounded ladder multiplier. */
  contrast?: number;
  /** Optional explicit token overrides (import / advanced). Last wins. */
  light?: Partial<MurmurTokens>;
  dark?: Partial<MurmurTokens>;
}

export interface AppearanceDocument {
  version: 1;
  mode: AppearanceMode;
  theme: ThemeConfig;
}
```

**Defaults:** `mode: 'system'`, `theme: { version: 1, presetId: 'sonic' }` (no overrides).

**Sanitization:** unknown version → default; oversized payload → default; invalid hex → drop field; contrast clamped; never throw into UI.

**Storage key:** `murmur-appearance`  
**Event:** `appearance-changed` (content-free nudge; each window re-reads storage)

---

## 6. Resolution pipeline

Pure function: `resolveTheme(doc, systemPrefersDark) → { appearance, colorScheme, tokens, adjustments[] }`

1. Resolve mode: `system` → `systemPrefersDark ? dark : light`, else forced.
2. Start from built-in Sonic base palette for that appearance (canonical tables in TS).
3. *(P2)* Apply contrast to surface-container ladder (documented formula + fixtures).
4. *(P1)* If accent set: derive `primary`, `primary-dim`, `on-primary` in OKLCH; pick `on-primary` for AA.
5. *(P2)* Apply background / foreground overrides with AA guarantee (clamp or auto-adjust; record in `adjustments[]`).
6. Apply `light`/`dark` partial token overrides for the active appearance (last wins).
7. Guarantee AA for: `on-surface/background`, `on-surface-variant/background`, `on-primary/primary`, `error/background`. Fail closed by adjusting, never by shipping sub-AA.
8. Return allowlisted record only.

**Apply (`applyResolvedTheme(resolved)`):**

- Set `document.documentElement.dataset.appearance` to concrete light/dark
- Set `document.documentElement.style.colorScheme`
- Set each `--murmur-*` on `documentElement`
- Never touch body background
- Optionally call Tauri `setTheme` (if P0b chooses native follow)

**System listener:** when `mode === 'system'`, subscribe to `matchMedia('(prefers-color-scheme: dark)')` and re-resolve + apply + emit.

---

## 7. CSS refactor

1. Add `@custom-variant dark` bound to `[data-appearance="dark"]`.
2. Stop relying on `@media (prefers-color-scheme: dark)` as the **only** dark token source. Keep media query only as a no-JS FOUC fallback *or* remove once pre-hydration script is mandatory — parity test must encode the chosen approach.
3. Keep Sonic hexes as CSS defaults for first paint before script (or script runs first — prefer script-first in all four HTML files).
4. Preserve `body.overlay-window { background: transparent }` with equal/higher specificity.
5. Keep `@theme inline` Tailwind bridge unchanged (still reads `--murmur-*`).
6. Respect existing `prefers-reduced-motion` rules; do not add theme cross-fade animations that ignore them.

---

## 8. Persistence, boot, sync

### 8.1 Module layout (proposed)

- `app/src/lib/appearance/types.ts` — types
- `app/src/lib/appearance/palettes.ts` — Sonic light/dark tables (canonical)
- `app/src/lib/appearance/color.ts` — OKLab/OKLCH + contrast helpers (no deps)
- `app/src/lib/appearance/resolve.ts` — pure resolver
- `app/src/lib/appearance/apply.ts` — DOM applier + allowlist assert
- `app/src/lib/appearance/storage.ts` — load/save/sanitize
- `app/src/lib/appearance/presets.ts` — code presets (`sonic`, …)
- `app/src/lib/hooks/useAppearance.ts` — main Settings driver
- `app/src/lib/hooks/useAppearanceSync.ts` — boot + `appearance-changed` listener for any window

### 8.2 Pre-hydration

Synchronous inline `<script>` in **all four** HTML files, before the module entry:

- Read `murmur-appearance`
- Resolve mode against `matchMedia`
- Set `data-appearance`, `color-scheme`, and `--murmur-*` on `documentElement`
- Tiny, duplicated or build-generated; test asserts presence in each HTML file

Residual: native window chrome may still flash before WKWebView paints — document as known; optional static `backgroundColor` in Tauri config.

### 8.3 Sync

- Authoritative: Tauri `appearance-changed` → each window `load` + `apply`
- `storage` event: best-effort only; **no acceptance criterion depends on it**
- Wire listeners into: `main`, `overlay`, `transform-review`, `log-viewer` (last two have none today)

---

## 9. Appearance UI

**Settings → Appearance** (new sidebar category)

**P0/P1 ship:**

- Mode cards: System / Light / Dark
- Accent color control (P1)
- Reset to Sonic
- Short note that island / review chrome stay dark

**P2 ship (after token debt):**

- Background / Foreground pickers
- Contrast slider
- Import / Export theme file
- Surface “contrast adjusted” when resolver clamps

**Preview:** optional scoped preview container with local CSS vars (must not mutate live `documentElement` without commit). Fake code-diff chrome is optional and low priority.

**Not in v1 UI:** font pickers, translucent sidebar, dock icons, multi-theme library dropdown of user themes (presets-only is fine).

---

## 10. Phases & tickets

### P0a — Dark variant + token plumbing

**Deliverables**

- `@custom-variant dark` on `data-appearance`
- Pure `resolveTheme` for Sonic light/dark with exact current hex fixtures
- TS palette tables; CSS↔TS parity test
- Update `styles.test.ts` for new structure; AA assertions still pass for Sonic

**AC**

1. No dark styling depends on `prefers-color-scheme` as its *only* source.
2. Resolver key allowlist ⊆ 14 tokens + `color-scheme`.
3. Fixture equality with today’s Sonic light/dark hexes.

### P0b — Apply + sync + boot

**Deliverables**

- `murmur-appearance` storage + sanitize
- `applyResolvedTheme` + pre-hydration script in all four HTML files
- `useAppearanceSync` in all four roots
- `appearance-changed` event; docs in `events.md` / `hooks.md`
- Overlay transparency invariant tests
- Native chrome decision: follow app via `setTheme` (+ one capability) **or** stay on system and document mismatch

**AC**

4. `data-appearance` always concrete; `color-scheme` set with it.
5. Pre-hydration present in all four HTML files (string test).
6. Appearance never written through `Settings` / `useSettings` gates.
7. All four windows update within one `appearance-changed` cycle (dev-verified).
8. Overlay/transform bodies remain transparent; glass unchanged in light mode; applier never sets body background.
9. Native chrome policy implemented and documented.
10. `npm test` + `npx tsc --noEmit` green.

### P1 — Accent + Appearance page

**Deliverables**

- OKLCH accent derivation + AA table-driven tests
- Appearance page: mode + accent + Reset
- `docs/features/appearance.md`, settings reference update, `DECISIONS.md` entry

**AC**

11. Documented formula; no new npm dependency.
12. Accent grid cannot produce sub-AA resolved palettes; UI shows adjustment when clamped.
13. Docs complete for shipped surface.

### Token debt (gate)

**Deliverables**

- Map hardcoded `stone-*` / status colors onto semantic tokens
- Add `success` / `warning` tokens if needed
- AA tests for new pairs

**AC**

14. Background/foreground pickers must not ship until this lands.

### P2 — Bg / fg / contrast + file import

**Deliverables**

- Background / foreground / contrast in resolver + UI
- File import/export (versioned, size-bounded, fail-closed)
- Contrast ladder algorithm + fixture table

**AC**

15. Contrast bounded; AA at both extremes.
16. Invalid import rejected; Reset restores Sonic fixture hexes exactly.
17. Import/export does not use clipboard as the primary path.

---

## 11. Explicit cuts (v1)

- Multi-theme library (`customThemes[]`, rename, delete-active edge cases)
- Reduce-motion as an in-app setting
- Overlay / transform-review light theming
- Font pickers, translucent sidebar, dock icons
- Live ChatGPT-style code-diff theme preview (optional later)
- Remote / shared themes

---

## 12. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Forced mode half-themes (`dark:` still OS) | P0a `@custom-variant` first |
| Theme in `Settings` → silent revert | Own storage + event |
| FOUC / wrong theme flash | Pre-hydration script in all 4 HTML files |
| Overlay opaque box | Allowlisted CSS vars only; transparency tests |
| Sub-AA custom colors | Resolver guarantees + UI adjustment signal |
| Bg/fg before token debt | Gate P2 on debt ticket |
| `storage` flaky across WKWebViews | Tauri event authoritative |
| Clipboard steals dictation paste buffer | File import/export |
| Native title bar mismatch | Explicit `setTheme` decision in P0b |
| TS/CSS palette drift | Parity test (or generate CSS from TS) |

---

## 13. Docs & decisions to write when implementing

- `docs/features/appearance.md` — user-facing behavior, privacy (local only), overlay exception
- `docs/reference/settings.md` — Appearance page in the page list (even though storage is separate)
- `docs/reference/events.md` — `appearance-changed`
- `docs/reference/hooks.md` — `useAppearance` / `useAppearanceSync`
- `docs/decisions/DECISIONS.md` entries:
  - Appearance stored outside `Settings`
  - Island / review chrome stay dark in v1
  - Token debt gates background/foreground
  - Schema B (single active theme, no library)

---

## 14. Test plan (summary)

- Unit: sanitize, resolve Sonic fixtures, accent grid AA, contrast ladder, allowlist keys
- CSS string tests: custom variant present; overlay-window transparent rule present; palette parity
- HTML string tests: pre-hydration script in all four entry HTML files
- Hook/component: Appearance page mode + accent + reset; sync emit path
- Manual / smoke: force Light on dark OS and Dark on light OS in Settings, Onboarding, Performance Lab, log viewer, overlay (confirm no opaque box), transform popover
- CI: `cd app && npm test` and `npx tsc --noEmit` (no Rust theme tests required unless `setTheme` capability added — then capability config review only)

---

## 15. Suggested GitHub issue split

1. **Theme engine P0a:** selector-based dark + Sonic resolve plumbing  
2. **Theme engine P0b:** persistence, pre-hydration, four-window sync, overlay invariants  
3. **Theme engine P1:** accent derivation + Appearance Settings page  
4. **Theme token debt:** replace hardcoded palette utilities with semantic tokens  
5. **Theme engine P2:** bg/fg/contrast + file import/export  

Each issue should paste the matching phase AC from §10.

---

## 16. Plan review history

- Substrate exploration of settings, CSS, windows, sync patterns.
- Draft plan reviewed adversarially; verdict **Revise**.
- Incorporated blockers: `dark:` variant, own storage (avoid `applyExternalSettings` clobber), pre-hydration, `color-scheme`, native chrome honesty, AA contract, OKLCH/no-deps, token-debt gate, Tauri-event-authoritative sync, file import, schema B, cut reduce-motion from v1.

This document is the single source of truth for implementing the whole theme engine.
