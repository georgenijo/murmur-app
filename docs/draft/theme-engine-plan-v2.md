# Theme Engine — Implementation Plan (v2, converged)

**Status:** Revised draft (ready to ticket)
**Date:** 2026-07-28
**Supersedes:** `docs/draft/theme-engine-plan.md` (fd7b119, branch `cursor/theme-engine-plan`)
**Revision basis:** Two independent reviews of v1 (see §16). All v1 substrate claims re-verified against main at v0.21.3.

---

## 1. Goal

Ship a local **Theme Engine**: versioned `ThemeConfig` documents that resolve onto Murmur's existing Sonic Canvas semantic token set (`--murmur-*`), with an Appearance Settings page, live apply across themed windows, and fail-closed validation.

This is **not** ChatGPT feature-parity. It is a real engine on Murmur's substrate: mode control, accent (then later bg/fg/contrast), presets, file import/export.

---

## 2. Product intent

| Want | Do not want (v1) |
|------|------------------|
| System / Light / Dark that actually works | Remote theme marketplace / network fetch |
| Named code-defined presets (Sonic default) | Multi-theme library CRUD / id collisions |
| Accent picker with AA fail-closed | Arbitrary UI/code font pickers |
| Later: bg/fg/contrast + file import | Translucent sidebar, dock icon themes |
| Live sync across the themed webviews | Clipboard "copy theme" as primary path |
| Overlay / transform island stay dark glass | Light-mode restyle of notch / review chrome |
| Pure frontend theme logic | Stuffing theme into `dictation-settings` |

---

## 3. Substrate (verified against main, v0.21.3)

### 3.1 Tokens today

- Source: `app/src/styles.css`
- 14 `--murmur-*` variables on `:root` (light), dark only under `@media (prefers-color-scheme: dark)`
- `@theme inline` maps them to Tailwind v4 `--color-*` → utilities like `bg-surface`, `text-on-surface`
- Tokens: `background`, `surface`, `surface-container-low`, `surface-container`, `surface-container-high`, `surface-container-lowest`, `surface-container-highest`, `primary`, `primary-dim`, `on-primary`, `on-surface`, `on-surface-variant`, `outline-variant`, `error`
- `color-scheme: light|dark` is set alongside the palettes today
- `body.overlay-window { background: transparent }` — **must preserve** (prior opaque-box regression)

### 3.2 Settings today

- `app/src/lib/settings.ts` — flat `Settings`, `localStorage` key `dictation-settings`
- `applyExternalSettings` (`useSettings.ts:132`) early-returns unless `disabled` or `autoPaste` changed — **theme must not live in `Settings`** or main silently clobbers appearance on the next unrelated cross-window write

### 3.3 Settings UI

- Pages: `recording`, `transcription`, `transform`, `text-vocabulary`, `delivery`, `performance`, `general`
- New category: **Appearance** (own sidebar entry + `SettingsSection`)
- `docs/reference/settings.md` ("seven pages") must be updated

### 3.4 Windows (four independent webviews — only two are themed)

| Window | HTML | Themed in v1? | Notes |
|--------|------|---------------|-------|
| Main | `app/index.html` | **Yes** | Decorated; Settings lives here |
| Log viewer | `app/log-viewer.html` | **Yes** | Decorated; no settings sync today — gains appearance sync |
| Overlay | `app/overlay.html` | No — always-dark glass | Transparent body; consumes **zero** semantic tokens or `dark:` utilities (verified by grep) |
| Transform review | `app/transform-review.html` | No — always-dark glass | Same: zero token / `dark:` consumers |

Because the overlay and transform-review windows render no themed pixels, **they get no appearance sync and no pre-hydration in v1** — wiring them would be untestable dead code. They join the sync path if and when they ever consume semantic tokens. Their CSS transparency invariants remain under test regardless.

No shared React state. Sync = storage + Tauri event (event is authoritative; `storage` events across WKWebViews are best-effort only and carry no acceptance criteria).

### 3.5 Dark utilities blocker

- `dark:` Tailwind utilities across ~25 files
- No `@custom-variant dark` today → Tailwind v4 default = `@media (prefers-color-scheme: dark)`
- Without a selector-based dark variant, forced Light/Dark is visibly broken (tokens flip, `dark:` utilities stay on OS)
- **Sequencing hazard:** rebinding `dark:` to `data-appearance` and setting that attribute must land in the **same ticket** — the variant rebind alone disables dark utilities app-wide (see §10, Foundation)

### 3.6 Token coverage gap

- Outside overlay/transform, hardcoded `stone-*`, amber/red/emerald, `bg-white`, etc.
- Bg/fg pickers without mapping those to semantic tokens will look broken
- **Token debt ticket gates advanced colors**

### 3.7 CSP

- `tauri.conf.json` has `"csp": null` today, so inline pre-hydration scripts are not blocked.
- **Guard note:** if CSP is ever hardened, the pre-hydration scripts must be hashed/nonced or theming silently dies at boot. The foundation ticket adds a comment at the CSP config site and a test that fails loudly if the scripts are removed.

### 3.8 File I/O pattern (for import/export)

- `package.json` has `@tauri-apps/plugin-dialog` but **no** `@tauri-apps/plugin-fs`.
- Existing precedent: Knowledge export (`KnowledgeManager.tsx` → `save()` dialog for the path, then bounded Rust commands `exportKnowledgeToFile` / `inspectKnowledgeImport` do the actual I/O).
- Theme import/export **must use this same pattern**: dialog selects the path, a bounded Rust command reads/writes the file. No frontend fs permissions.

---

## 4. Locked decisions

1. **Schema fork B** — one active `ThemeConfig` + code-defined presets + file import/export. No `customThemes[]` library UI in v1.
2. **Own storage** — `localStorage` key `murmur-appearance` (versioned document). Separate `appearance-changed` Tauri event. Never routed through `Settings` / `useSettings`.
3. **Dark variant** — `@custom-variant dark (&:where([data-appearance="dark"], [data-appearance="dark"] *));` and always set concrete `data-appearance="light"|"dark"` on `<html>` (never `system`). Lands together with the code that sets the attribute (§10 Foundation).
4. **Overlay / transform glass** — remain always-dark in v1; no sync wiring, no pre-hydration in those windows (§3.4). Inline `rgba(20,20,20,0.92)` chrome untouched.
5. **Import/export** — dialog-for-path + bounded Rust command I/O, per the Knowledge pattern (§3.8). Clipboard is never the primary path (dictation owns the clipboard).
6. **Reduce-motion user override** — cut from v1 (OS `prefers-reduced-motion` already respected).
7. **Color math** — OKLCH/OKLab derivation in JS, **no new npm dependency**; hand-rolled converter + fixtures.
8. **Rust** — no theme *logic*. Native chrome follows the app via **one** application-level `setTheme` call (capability `core:app:allow-set-app-theme`), invoked only from the Settings flow in main — not per window. If rejected during implementation, document the native/app mismatch instead.
9. **Applier invariant** — resolver returns only allowlisted keys (`--murmur-*` × 14 + `color-scheme`); applier never sets `background` / `background-color` on `html`/`body`.
10. **Resolved-token cache** *(new in v2)* — resolution runs at **write time**; the stored document carries fully resolved light *and* dark token tables. The pre-hydration script is math-free: read, pick by mode, apply. The full resolver still re-runs on module load as the authoritative path, and a parity test asserts bootstrap output ≡ resolver output for the same document.
11. **System-mode changes are local, not coordinated** *(new in v2)* — each themed window owns a `matchMedia('(prefers-color-scheme: dark)')` listener and re-applies **locally, with no emit**. The `appearance-changed` event fires only for user edits, which originate in exactly one place (Settings, main window). This deletes the multi-coordinator problem instead of managing it.
12. **Preset fallback** *(new in v2)* — `presetId` is `'sonic' | 'custom'`. Sanitization maps any unknown id to `'sonic'`. Custom/imported themes explicitly base on the Sonic palette for the active appearance before overrides apply.
13. **No-JS / first-paint fallback** *(decided in v2)* — the existing CSS (`:root` Sonic light + `@media (prefers-color-scheme: dark)` Sonic dark) stays as the pre-script fallback. Since empty storage resolves to Sonic/system, CSS and script agree by construction; the media query affects only token values, never the `dark:` variant (which is selector-bound). The parity test encodes this.

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
  /** Sanitization maps unknown ids to 'sonic'. */
  presetId: 'sonic' | 'custom';
  /** Optional accent override (#rrggbb). Derives primary / primary-dim / on-primary. */
  accent?: string;
  /** Advanced-colors phase only — gated on token debt. */
  background?: string;
  foreground?: string;
  /** Advanced-colors phase only — 0–100, bounded ladder multiplier. */
  contrast?: number;
  /** Optional explicit token overrides (import / advanced). Last wins. */
  light?: Partial<MurmurTokens>;
  dark?: Partial<MurmurTokens>;
}

export interface AppearanceDocument {
  version: 1;
  mode: AppearanceMode;
  theme: ThemeConfig;
  /**
   * Write-time resolution output. Read by the pre-hydration bootstrap only;
   * regenerated on every save. Runtime always re-resolves from `theme` and
   * a parity test asserts the two agree.
   */
  resolved: { light: MurmurTokens; dark: MurmurTokens };
}
```

**Defaults:** `mode: 'system'`, `theme: { version: 1, presetId: 'sonic' }`, `resolved` = Sonic tables.

**Sanitization (fail closed, never throw into UI):** unknown version → default document; oversized payload → default; unknown `presetId` → `'sonic'`; invalid hex → drop field; `contrast` clamped; missing/malformed `resolved` → regenerate from `theme` (or default if that fails).

**Storage key:** `murmur-appearance`
**Event:** `appearance-changed` (content-free nudge; each themed window re-reads storage). Emitted **only** on user edits from Settings — never from system-mode `matchMedia` reactions (§4.11).

---

## 6. Resolution pipeline

Pure function: `resolveTheme(doc, systemPrefersDark) → { appearance, colorScheme, tokens, adjustments[] }`

1. Resolve mode: `system` → `systemPrefersDark ? dark : light`, else forced.
2. Start from built-in Sonic base palette for that appearance (canonical tables in TS). `presetId: 'custom'` also starts here (§4.12).
3. *(Advanced)* Apply contrast to surface-container ladder (documented formula + fixtures).
4. *(Accent phase)* If accent set: derive `primary`, `primary-dim`, `on-primary` in OKLCH; pick `on-primary` for AA.
5. *(Advanced)* Apply background / foreground overrides with AA guarantee (clamp or auto-adjust; record in `adjustments[]`).
6. Apply `light`/`dark` partial token overrides for the active appearance (last wins).
7. **AA contract, staged:**
   - *Foundation + accent phases:* guarantee `on-surface/background`, `on-surface-variant/background`, `on-primary/primary`, `error/background`. (Accent can only move the `primary` triple, so this covers everything mutable.)
   - *Token debt + advanced phases:* extend to the full semantic matrix — `on-surface` and `on-surface-variant` over each of the six surface/container tokens, status colors over their placement surfaces, and focus/outline contrast — because bg/fg/contrast can move every surface. The matrix is defined and tested in the token-debt ticket, before any picker ships.
   - Fail closed by adjusting, never by shipping sub-AA.
8. Return allowlisted record only.

**Write path (`saveAppearance(doc)`):** sanitize → resolve both appearances → store document *with* `resolved` tables → emit `appearance-changed`.

**Apply (`applyResolvedTheme(resolved)`):**

- Set `document.documentElement.dataset.appearance` to concrete light/dark
- Set `document.documentElement.style.colorScheme`
- Set each `--murmur-*` on `documentElement`
- Never touch body background

**System listener (per themed window, local only):** when `mode === 'system'`, subscribe to `matchMedia('(prefers-color-scheme: dark)')`; on change, re-resolve + apply locally. **No emit** (§4.11). Listener cleanup must survive React Strict Mode double-invocation.

**Native chrome:** on user mode edits, main additionally calls app-level `setTheme` once (§4.8).

---

## 7. CSS refactor

1. Add `@custom-variant dark` bound to `[data-appearance="dark"]` — in the same ticket as the attribute-setting bootstrap (§10 Foundation).
2. Keep `:root` Sonic light + `@media (prefers-color-scheme: dark)` Sonic dark as the pre-script/no-JS fallback (§4.13). The bootstrap overrides them.
3. Preserve `body.overlay-window { background: transparent }` with equal/higher specificity.
4. Keep `@theme inline` Tailwind bridge unchanged (still reads `--murmur-*`).
5. Respect existing `prefers-reduced-motion` rules; no theme cross-fade animations that ignore them.

---

## 8. Persistence, boot, sync

### 8.1 Module layout

- `app/src/lib/appearance/types.ts` — types
- `app/src/lib/appearance/palettes.ts` — Sonic light/dark tables (canonical)
- `app/src/lib/appearance/color.ts` — OKLab/OKLCH + contrast helpers (no deps)
- `app/src/lib/appearance/resolve.ts` — pure resolver
- `app/src/lib/appearance/apply.ts` — DOM applier + allowlist assert
- `app/src/lib/appearance/storage.ts` — load/save/sanitize (write path regenerates `resolved`)
- `app/src/lib/appearance/presets.ts` — code presets (`sonic`)
- `app/src/lib/hooks/useAppearance.ts` — main Settings driver (only emitter; native `setTheme` caller)
- `app/src/lib/hooks/useAppearanceSync.ts` — boot + `appearance-changed` + local system listener, for any themed window

### 8.2 Pre-hydration bootstrap

Synchronous inline `<script>` in the **two themed** HTML files (`index.html`, `log-viewer.html`), before the module entry:

- Read `murmur-appearance`; JSON-parse defensively (any failure → do nothing, CSS fallback holds)
- Resolve mode against `matchMedia` (mode only — **no color math**)
- Pick `resolved.light` or `resolved.dark`; set `data-appearance`, `color-scheme`, and each `--murmur-*` on `documentElement`

Tests: (a) presence in both HTML files, (b) **output parity** — for a corpus of stored documents, bootstrap-applied tokens ≡ `resolveTheme` output, (c) corrupted-storage no-op.

Residual: native window chrome may still flash before WKWebView paints — documented as known; optional static `backgroundColor` in Tauri config.

### 8.3 Sync

- Authoritative: Tauri `appearance-changed` → each themed window re-reads storage + applies
- `storage` event: best-effort only; no acceptance criterion depends on it
- Wire `useAppearanceSync` into: `main`, `log-viewer` only (§3.4)

---

## 9. Appearance UI

**Settings → Appearance** (new sidebar category)

**Foundation + accent ship:**

- Mode cards: System / Light / Dark
- Accent color control
- Reset to Sonic
- Short note that island / review chrome stay dark

**Advanced-colors ship (after token debt):**

- Background / Foreground pickers
- Contrast slider
- Import / Export theme file (Rust-command I/O, §3.8)
- Surface "contrast adjusted" indicator when resolver clamps

**Preview:** optional scoped preview container with local CSS vars (must not mutate live `documentElement` without commit). Low priority.

**Not in v1 UI:** font pickers, translucent sidebar, dock icons, multi-theme library.

---

## 10. Phases & tickets (four)

> v1's P0a/P0b split is collapsed: rebinding `dark:` to `data-appearance` without the code that sets the attribute would ship an app whose dark utilities never fire. Variant + bootstrap + apply land atomically.

### T1 — Foundation: dark variant, resolver, storage, boot, sync

**Deliverables**

- `@custom-variant dark` on `data-appearance` **and** the pre-hydration bootstrap + `useAppearanceSync`, in one ticket
- TS palette tables; CSS↔TS parity test; pure `resolveTheme` with exact current Sonic hex fixtures
- `murmur-appearance` storage + sanitize + write-time `resolved` cache
- `applyResolvedTheme`; bootstrap in `index.html` + `log-viewer.html`; sync wired into both roots
- `appearance-changed` event (user-edit-only); docs in `events.md` / `hooks.md`
- Local per-window system listener, Strict-Mode-safe cleanup
- Native chrome: app-level `setTheme` + `core:app:allow-set-app-theme` (or documented rejection)
- Overlay/transform transparency invariant tests retained; CSP guard note + test (§3.7)
- Mode cards + Reset in a minimal Appearance page (so the ticket is user-visible and testable end to end)

**AC**

1. No dark styling depends on `prefers-color-scheme` as its only source; forced Light on a dark OS and forced Dark on a light OS render correctly in Settings, Onboarding, Performance Lab, history, log viewer.
2. Resolver key allowlist ⊆ 14 tokens + `color-scheme`; fixture equality with today's Sonic light/dark hexes.
3. `data-appearance` always concrete; `color-scheme` set with it.
4. Bootstrap present in both themed HTML files; bootstrap/runtime **output parity** test passes; corrupted storage falls back to defaults without error.
5. Appearance never written through `Settings` / `useSettings` gates.
6. Both themed windows update within one `appearance-changed` cycle; OS theme change in system mode updates each window with **zero** events emitted.
7. Overlay/transform bodies remain transparent; glass unchanged; applier never sets body background.
8. System-listener cleanup verified under React Strict Mode.
9. Native chrome policy implemented (single app-level call) and documented.
10. `npm test` + `npx tsc --noEmit` green.

### T2 — Accent + Appearance page polish

**Deliverables**

- OKLCH accent derivation + AA table-driven tests (accent grid)
- Appearance page: accent control + adjustment indicator
- `docs/features/appearance.md`, settings reference update, `DECISIONS.md` entries (§13)

**AC**

11. Documented formula; no new npm dependency.
12. Accent grid cannot produce sub-AA resolved palettes; UI shows adjustment when clamped.
13. Docs complete for shipped surface.

### T3 — Token debt (gate)

**Deliverables**

- Map hardcoded `stone-*` / status colors onto semantic tokens; add `success` / `warning` tokens if needed
- Define + implement the **full semantic contrast matrix** (§6.7): text tokens over all six surface/container tokens, status colors, focus/outline contrast — with AA tests

**AC**

14. Background/foreground pickers must not ship until this lands.
15. Contrast matrix tests pass for Sonic light and dark.

### T4 — Advanced colors: bg / fg / contrast + file import

**Deliverables**

- Background / foreground / contrast in resolver + UI
- Import/export: dialog-for-path + bounded Rust commands (Knowledge pattern, §3.8), versioned, size-bounded, fail-closed, with Rust-side tests
- Contrast ladder algorithm + fixture table

**AC**

16. Contrast bounded; full matrix AA at both extremes.
17. Invalid import rejected with a visible reason; Reset restores Sonic fixture hexes exactly.
18. Import/export never touches the clipboard; file I/O happens only in the bounded Rust commands.

---

## 11. Explicit cuts (v1)

- Multi-theme library (`customThemes[]`, rename, delete-active edge cases)
- Reduce-motion as an in-app setting
- Overlay / transform-review light theming **and** appearance sync wiring for those windows
- Font pickers, translucent sidebar, dock icons
- Live ChatGPT-style code-diff theme preview
- Remote / shared themes

---

## 12. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Variant rebind ships without attribute-setter → dark utilities dead | Single foundation ticket (T1); no partial ship |
| Theme in `Settings` → silent revert | Own storage + event (verified gate at `useSettings.ts:132`) |
| FOUC / wrong theme flash | Math-free bootstrap reading the `resolved` cache; parity-tested |
| Bootstrap drifts from resolver | Write-time cache + bootstrap/runtime output-parity test |
| Overlay opaque box | Allowlisted CSS vars only; transparency tests; no sync wiring in overlay |
| Sub-AA custom colors | Staged AA contract; full matrix gated in T3 before pickers |
| Bg/fg before token debt | T3 gates T4 |
| One OS flip → event storm across windows | System-mode handling is local, emit-free (§4.11) |
| `storage` flaky across WKWebViews | Tauri event authoritative |
| Clipboard steals dictation paste buffer | Rust-command file I/O; clipboard never involved |
| No fs plugin for import/export | Knowledge pattern: dialog path + bounded Rust commands (§3.8) |
| Future CSP hardening kills boot theming | Guard note at config + loud test (§3.7) |
| Native title bar mismatch | Single app-level `setTheme` with explicit capability |
| TS/CSS palette drift | Parity test (or generate CSS from TS) |
| Unknown/corrupt stored document | Sanitize to defaults; unknown `presetId` → `sonic`; never throw into UI |

---

## 13. Docs & decisions to write when implementing

- `docs/features/appearance.md` — user-facing behavior, privacy (local only), overlay exception
- `docs/reference/settings.md` — Appearance page in the page list (storage is separate)
- `docs/reference/events.md` — `appearance-changed` (user-edit-only semantics)
- `docs/reference/hooks.md` — `useAppearance` / `useAppearanceSync`
- `docs/decisions/DECISIONS.md` entries:
  - Appearance stored outside `Settings`
  - Island / review chrome stay dark in v1; no sync wiring in unthemed windows
  - Token debt gates background/foreground
  - Schema B (single active theme, no library)
  - Write-time resolved-token cache; math-free bootstrap
  - System-mode changes local and emit-free

---

## 14. Test plan (summary)

- **Unit:** sanitize (incl. unknown `presetId`, corrupt/oversized payloads, malformed `resolved`), resolve Sonic fixtures, accent grid AA, contrast ladder, allowlist keys
- **Parity:** CSS↔TS palettes; bootstrap output ≡ resolver output over a document corpus
- **CSS string tests:** custom variant present; `overlay-window` transparent rule present
- **HTML string tests:** bootstrap present in both themed entry files
- **Hook/component:** Appearance page mode + accent + reset; emit-on-edit path; system listener applies locally without emitting; Strict-Mode cleanup
- **Behavioral:** corrupted storage boots to defaults; forced Light on dark OS and forced Dark on light OS
- **Manual / smoke (native):** all four windows on both OS appearances × three modes — confirm overlay/transform stay dark glass with transparent bodies, main + log viewer follow the app appearance, native chrome follows `setTheme`
- **CI:** `cd app && npm test` and `npx tsc --noEmit`; T4 adds Rust tests for the import/export commands (`cargo test -- --test-threads=1`)

---

## 15. GitHub issue split

1. **Theme engine T1 — Foundation:** selector dark variant + resolver + storage + bootstrap + two-window sync + mode UI
2. **Theme engine T2 — Accent:** OKLCH derivation + Appearance page polish + docs
3. **Theme engine T3 — Token debt:** semantic-token migration + full contrast matrix (gates T4)
4. **Theme engine T4 — Advanced colors:** bg/fg/contrast + Rust-command file import/export

Each issue pastes its AC block from §10.

---

## 16. Plan review history

- v1 (fd7b119): substrate exploration + adversarial revision; established own-storage, selector dark variant, overlay protection, token-debt gate, no-deps OKLCH, file import.
- v2 (this doc) converges two independent reviews of v1:
  - **Review A (George):** collapsed P0a/P0b (variant rebind cannot ship without the attribute-setter); import/export needs the Knowledge Rust-I/O pattern (no fs plugin exists); bootstrap must be parity-tested, not presence-tested; AA needs a full semantic matrix (staged into T3 here); single native `setTheme` owner with explicit capability; preset fallback defined; behavioral tests added (corrupt storage, forced modes, Strict Mode cleanup, native smoke).
  - **Review B (Claude):** write-time resolved-token cache making the bootstrap math-free (adopted as the parity mechanism); system-mode changes local and emit-free instead of a coordinator window; overlay/transform verified token-free → sync wiring trimmed to the two themed windows; CSP-null guard; §7.2 fallback decision forced (keep CSS media-query fallback); `presetId` narrowed to a union type.

This document is the single source of truth for implementing the theme engine.
