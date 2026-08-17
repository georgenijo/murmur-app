# Theme Engine — Converged Implementation Plan

**Status:** Historical draft — implemented by #377; library/marketplace scope superseded by #592
**Date:** 2026-07-28  
**Scope:** Local Appearance theming for Murmur  
**Converges:** `cursor/theme-engine-plan` (v1) and `docs/draft/theme-engine-plan-v2.md`

---

## 1. Plan scorecard

| Plan | Correctness | Completeness | Actionability | Risk control | Overall |
|---|---:|---:|---:|---:|---:|
| v1 — `cursor/theme-engine-plan` | 8.5/10 | 7.5/10 | 8.5/10 | 8/10 | **8.1/10** |
| v2 — `theme-engine-plan-v2.md` | 9.2/10 | 9.2/10 | 9.2/10 | 9/10 | **9.2/10** |

### v1

Strengths:

- Verified the substrate against `main` at v0.21.3.
- Chose separate appearance storage, selector-driven dark mode, local-only behavior, and a token-debt gate.
- Protected overlay transparency and clipboard-first dictation.
- Proposed a sensible resolver, schema, phase structure, and documentation surface.

Why it does not score higher:

- P0a could merge the selector rebind before any code establishes `data-appearance`.
- Pre-hydration did not define where custom resolved tokens came from.
- Import/export omitted actual file transport.
- System listeners and native-theme ownership were undecided.
- The accessibility and behavioral test contracts were incomplete.

### v2

Strengths:

- Collapses the unsafe P0a/P0b boundary.
- Adds a write-time resolved-token cache and math-free bootstrap.
- Uses local, emit-free system listeners in the themed windows.
- Correctly limits visible theme synchronization to main and log-viewer.
- Defines the Knowledge-style Rust file-I/O pattern.
- Adds preset fallback, staged contrast coverage, Strict Mode cleanup, and behavioral tests.
- Produces four coherent, independently actionable tickets.

Remaining improvements incorporated below:

- Replace inline bootstrap code with a parser-blocking same-origin script so future CSP hardening does not require unsafe inline execution.
- Version and strictly validate the derived cache.
- Revision user-change events so consumers can reject stale storage reads.
- Lock application-level native theming instead of permitting a documented mismatch.
- Make file-size, atomic-write, and imported-cache behavior explicit.

### Final convergence decisions

| Question | Final decision |
|---|---|
| P0a versus P0b | One atomic foundation ticket; selector rebinding never merges without attribute boot/apply. |
| Themed windows | Main and log-viewer only in v1. Overlay and transform-review stay unsynchronized, transparent, always-dark glass. |
| System-mode changes | Each themed window applies its local `matchMedia` change without emitting an event. |
| User-change synchronization | Main is the only writer/emitter; events carry a revision so log-viewer can reject stale reads. |
| First paint | A parser-blocking same-origin bootstrap selects a strictly validated, write-time resolved token cache. |
| CSP | Use an external classic script rather than depending on permanent inline-script permission. |
| CSS fallback | Keep Sonic light plus the dark media query as a base-color fallback, not full no-JS parity. |
| Native chrome | Application-level `setTheme` is required and owned by main; a documented mismatch is not acceptable. |
| Accessibility | Test the pairs mutable in each phase; the token-debt ticket establishes the full semantic matrix before advanced colors. |
| Import/export | Dialog selects the path; bounded Rust commands perform UTF-8 reads and atomic writes; imported caches are discarded. |
| Preset fallback | Unknown preset IDs sanitize to Sonic; `custom` explicitly derives from Sonic before overrides. |

---

## 2. Goal

Ship a fully local Theme Engine that resolves a versioned appearance document onto Murmur's Sonic Canvas semantic tokens.

The first release provides:

- System, Light, and Dark modes.
- Sonic as the built-in preset.
- An accessible custom accent.
- Live application across all themed Tauri webviews.
- Native title-bar appearance that follows the selected mode.
- Fail-closed persistence and boot behavior.

Later phases add:

- Semantic-token cleanup across the UI.
- Background, foreground, and contrast controls.
- File-based import and export.

No network service, remote marketplace, font picker, dock-icon theming, or multi-theme library is included.

---

## 3. Verified substrate

- `app/src/styles.css` defines 14 `--murmur-*` tokens.
- Light values live on `:root`; dark values currently live only under `prefers-color-scheme: dark`.
- Tailwind v4's `@theme inline` maps those variables to semantic utilities.
- There are 371 `dark:` uses across 25 TypeScript/TSX files.
- There are substantial hardcoded stone, amber, red, emerald, and white utilities.
- Main, overlay, transform-review, and log-viewer are separate webviews with separate React roots.
- Main and log-viewer render themed pixels.
- Overlay and transform-review use hardcoded dark glass and currently consume no semantic tokens or `dark:` utilities. They do not join appearance synchronization in v1.
- `body.overlay-window { background: transparent }` is a release-critical invariant.
- `useSettings.applyExternalSettings` only accepts `disabled` and `autoPaste` changes. Appearance must not enter `dictation-settings`.
- The dialog plugin is installed, but the frontend filesystem plugin is not. A dialog can select a path but cannot itself read or write the selected file.
- Tauri exposes application-level `setTheme`; it requires `core:app:allow-set-app-theme`.
- `tauri.conf.json` currently sets `csp` to `null`. The design must not depend permanently on inline-script permission.

---

## 4. Locked architecture

### 4.1 Storage

Appearance uses its own localStorage document:

- Key: `murmur-appearance`
- Independent from `Settings` and `dictation-settings`
- One active theme document
- Code-defined presets
- No custom-theme library in v1

### 4.2 Concrete DOM mode

The user preference may be `system`, but the DOM always receives:

```html
<html data-appearance="light">
```

or:

```html
<html data-appearance="dark">
```

Tailwind's dark variant becomes selector-based:

```css
@custom-variant dark (&:where([data-appearance="dark"], [data-appearance="dark"] *));
```

The selector conversion and the code that establishes `data-appearance` must ship atomically.

### 4.3 First-paint cache

The stored document contains both:

- The authoritative user configuration.
- A derived cache containing fully resolved light and dark token tables.

The cache is not an import format and is never authoritative after runtime initialization.

Every normal write:

1. Sanitizes the configuration.
2. Resolves both light and dark palettes.
3. Writes the configuration and derived cache together.
4. Emits one revisioned appearance event.

At boot, a tiny parser-blocking script:

1. Reads the stored document.
2. Resolves `system` to a concrete mode with `matchMedia`.
3. Validates the cache version, exact token keys, and hex values.
4. Applies the selected cached token table.
5. Falls back to the matching Sonic table if storage or cache validation fails.

It performs no OKLab/OKLCH conversion and no contrast calculation.

The full TypeScript resolver runs during application initialization, recomputes both cache tables, applies the authoritative result, and repairs stale cache data.

### 4.4 CSP-compatible bootstrap

Use a parser-blocking classic script such as:

```html
<script src="/appearance-boot.js"></script>
```

before the module entry in the two themed HTML files.

The script is a committed, deliberately small artifact under `app/public/`. It remains compatible with a future `script-src 'self'` CSP. A parity test keeps its token allowlist, cache version, and Sonic fallback tables aligned with the TypeScript implementation.

### 4.5 CSS fallback

Keep the current Sonic light defaults and dark media query in CSS.

This is a base-color fallback for corrupt storage or bootstrap failure, not full no-JavaScript theme support. Murmur requires JavaScript, and selector-based `dark:` utilities require `data-appearance`.

Inline token values applied by the bootstrap take precedence over the CSS fallback.

### 4.6 Synchronization ownership

The main webview is the only writer and event emitter. It owns:

- Appearance writes from Settings.
- Global `appearance-changed` emission for user edits, reset, import, and cache repair.
- Application-level native `setTheme`.

Main and log-viewer each own a local `matchMedia` listener while mode is `system`. An OS appearance change re-selects the appropriate cached table locally and emits no Tauri event.

Overlay and transform-review do not join appearance synchronization in v1. Their transparency and always-dark glass remain explicit test invariants.

Event payload:

```ts
interface AppearanceChangedEvent {
  revision: number;
  reason: 'user' | 'repair' | 'reset' | 'import';
}
```

For user writes, log-viewer reloads storage and ignores older revisions. A bounded retry handles the event arriving before the new localStorage revision becomes visible.

For system changes, the stored configuration does not change and no event is emitted.

Both themed roots maintain the same concrete `data-appearance` and `color-scheme`.

### 4.7 Native appearance

Native appearance is applied by main during initialization and after every mode write:

- `system` calls application-level `setTheme(null)`; Tauri then follows OS changes.
- Forced Light calls `setTheme('light')`.
- Forced Dark calls `setTheme('dark')`.

Only the main capability receives:

```json
"core:app:allow-set-app-theme"
```

Decorated main and log-viewer windows must not display system-dark title bars around forced-light content, or the inverse. A documented mismatch is not an acceptable fallback.

### 4.8 File I/O

Theme logic remains frontend-only, but advanced-phase file transport uses narrow Rust commands, following the existing Knowledge import/export pattern:

- `read_theme_file(path) -> String`
- `write_theme_file(path, contents)`

Rust responsibilities are limited to:

- A 64 KiB file/content limit.
- UTF-8 input.
- Atomic writes.
- Clear I/O errors.

The frontend owns schema validation, import preview, resolution, and cache generation. Imported cache data is discarded.

---

## 5. Schema

```ts
export type AppearanceMode = 'system' | 'light' | 'dark';
export type ResolvedAppearance = 'light' | 'dark';

export const MURMUR_TOKEN_NAMES = [
  'background',
  'surface',
  'surface-container-low',
  'surface-container',
  'surface-container-high',
  'surface-container-lowest',
  'surface-container-highest',
  'primary',
  'primary-dim',
  'on-primary',
  'on-surface',
  'on-surface-variant',
  'outline-variant',
  'error',
] as const;

export type MurmurTokenName = (typeof MURMUR_TOKEN_NAMES)[number];
export type MurmurTokens = Record<MurmurTokenName, `#${string}`>;

export const BUILTIN_PRESET_IDS = ['sonic'] as const;
export type BuiltinPresetId = (typeof BUILTIN_PRESET_IDS)[number];

export interface ThemeConfigV1 {
  version: 1;
  presetId: BuiltinPresetId | 'custom';
  accent?: string;
  background?: string;
  foreground?: string;
  contrast?: number;
  light?: Partial<MurmurTokens>;
  dark?: Partial<MurmurTokens>;
}

export interface ResolvedThemeCacheV1 {
  version: 1;
  light: MurmurTokens;
  dark: MurmurTokens;
}

export interface AppearanceDocumentV1 {
  version: 1;
  revision: number;
  mode: AppearanceMode;
  theme: ThemeConfigV1;
  cache: ResolvedThemeCacheV1;
}
```

Runtime parsing still treats `presetId` as untrusted input.

Rules:

- Unknown document or theme version: fall back to Sonic/System.
- Unknown preset ID: fall back to Sonic.
- `custom` uses Sonic as its explicit base before overrides.
- Invalid mode: `system`.
- Invalid color: drop that field.
- Invalid token key: discard it.
- Missing token key in the cache: discard the entire cache.
- Oversized storage payload: fall back without parsing.
- Contrast: clamp to its documented range.
- Unknown fields: ignore on local load and remove on the next write.
- Imports: reject unsupported versions and show the error before committing.
- Sanitization never throws into React rendering.

---

## 6. Resolution

Pure API:

```ts
resolveTheme(
  theme: ThemeConfigV1,
  appearance: ResolvedAppearance,
): {
  appearance: ResolvedAppearance;
  colorScheme: ResolvedAppearance;
  tokens: MurmurTokens;
  adjustments: ThemeAdjustment[];
}
```

Order:

1. Validate and select the built-in base preset.
2. Apply contrast to the surface ladder when supported.
3. Derive accent tokens in OKLCH when an accent is present.
4. Apply background and foreground controls when supported.
5. Apply explicit light/dark token overrides.
6. Enforce the semantic contrast matrix.
7. Return exactly the allowlisted token record.

The applier:

- Sets concrete `data-appearance`.
- Sets `documentElement.style.colorScheme`.
- Sets only allowlisted `--murmur-*` custom properties.
- Never sets `background` or `background-color` on `html` or `body`.
- Never mutates overlay glass colors.

---

## 7. Accessibility contract

AA is defined over semantic usage pairs, not only four colors. Coverage expands before a phase can mutate additional tokens.

Foundation and accent phases guarantee at least 4.5:1 for:

- `on-surface` against `background`.
- `on-surface-variant` against `background`.
- `on-primary` against `primary` and `primary-dim`.
- `error` against `background`.

The token-debt phase expands the matrix, before advanced colors ship:

- `on-surface` against `background`, `surface`, and every surface-container token.
- `on-surface-variant` against every surface token on which the utility is used.
- `on-primary` against `primary` and `primary-dim`.
- `error` against every surface on which error text is used.
- Future `success` and `warning` foregrounds against their supported surfaces.

Non-text UI pairs must meet at least 3:1 where WCAG requires it:

- Focus indicators against adjacent surfaces.
- Interactive outlines against adjacent surfaces.
- Status indicators whose color is the only state cue.

Resolution records every automatic adjustment. The Appearance UI explains when a selected value was adjusted.

Explicit token overrides cannot bypass these guarantees. Values that cannot be adjusted deterministically fall back to the nearest valid preset token.

---

## 8. Appearance UI

Settings receives an Appearance page.

Foundation:

- System, Light, and Dark mode cards.
- Sonic preset.
- Reset to Sonic.
- Note that overlay and transform-review glass stay dark.

Accent phase:

- Accent picker.
- Validated hex input.
- Adjustment notice.
- Immediate live application after commit.

Advanced phase:

- Background and foreground controls.
- Contrast slider.
- Import Theme and Export Theme.
- Import preview with validation and adjustment summary.

Preview is optional. If added, it uses scoped CSS variables and never mutates the live document before commit.

---

## 9. Delivery phases

### Ticket 1 — Atomic theme foundation

This is one merge unit. It may use multiple internal commits, but selector conversion must not merge without boot/apply support.

Deliverables:

- Appearance types, Sonic palettes, sanitizer, storage, and cache.
- Sonic resolver for both concrete modes.
- Selector-based Tailwind dark variant.
- Parser-blocking bootstrap in main and log-viewer.
- Runtime applier in main and log-viewer.
- Local, emit-free system listeners in both themed roots.
- Revisioned user-change events emitted only by main.
- Main-only application `setTheme` permission and calls.
- Appearance Settings page with mode cards and Reset.
- CSS fallback decision documented.
- Overlay transparency and dark-glass invariants.
- Reference documentation for storage, event, hook, and native behavior.

Acceptance criteria:

1. No merged state exists where `dark:` depends on `data-appearance` but the attribute is absent.
2. Empty storage reproduces the existing Sonic/System appearance exactly.
3. Forced Light works on a dark OS; forced Dark works on a light OS.
4. Main and log-viewer content and native chrome agree.
5. Both themed roots expose the same concrete mode after one user-change event.
6. Overlay and transform-review bodies remain transparent and their glass remains dark.
7. Bootstrap and runtime Sonic token tables are identical.
8. Corrupt, oversized, or partial storage fails to Sonic without throwing.
9. Appearance never enters `Settings` or `dictation-settings`.
10. React Strict Mode does not duplicate listeners after cleanup.

### Ticket 2 — Accessible accent

Deliverables:

- Dependency-free OKLab/OKLCH conversion.
- Accent derivation for `primary`, `primary-dim`, and `on-primary`.
- Accent picker and adjustment feedback.
- Cache regeneration after accent writes.
- Feature documentation.

Acceptance criteria:

1. A table-driven accent gamut produces valid hex tokens.
2. All accent-related semantic contrast pairs pass.
3. Bootstrap first paint uses the selected accent without a Sonic-to-custom flash.
4. Reset restores exact Sonic fixtures.

### Ticket 3 — Semantic token debt

Deliverables:

- Replace hardcoded neutral palette utilities with semantic tokens.
- Define explicit success and warning tokens if their current usage requires them.
- Document which foregrounds may render on which surfaces.
- Remove redundant `dark:` utilities where semantic tokens already encode appearance.

Acceptance criteria:

1. Background/foreground controls remain unavailable until this ticket lands.
2. The semantic contrast matrix covers every supported foreground/surface combination.
3. Main Settings, Onboarding, Performance Lab, history, and log viewer remain visually coherent in both forced modes.

### Ticket 4 — Advanced colors and file exchange

Deliverables:

- Background, foreground, and bounded contrast controls.
- Deterministic surface-ladder derivation.
- Dialog-based file selection.
- Bounded Rust read/write commands.
- Versioned theme-file import/export.
- Import preview and adjustment summary.

Acceptance criteria:

1. Contrast remains valid at both slider extremes.
2. Imported cache data is ignored and recomputed.
3. Unsupported, malformed, or oversized files are rejected before commit.
4. Export writes atomically and never uses the clipboard.
5. Importing then exporting a valid theme preserves its authoritative configuration.
6. Existing dictation clipboard contents are unchanged.

---

## 10. Test plan

### Unit

- Document and cache sanitization.
- Unknown versions, presets, fields, modes, and token keys.
- Oversized storage and file payloads.
- Exact Sonic fixture parity.
- OKLab/OKLCH conversion fixtures.
- Accent gamut and semantic contrast matrix.
- Contrast ladder extremes.
- Resolver allowlist.
- Imported-cache stripping.

### Bootstrap behavior

- Execute the real bootstrap artifact against:
  - Empty storage.
  - Valid forced-light storage on dark OS.
  - Valid forced-dark storage on light OS.
  - Valid custom accent cache.
  - Invalid JSON.
  - Missing cache keys.
  - Invalid token values.
- Assert script ordering before both themed module entries.
- Assert bootstrap token output equals runtime output for shared fixtures.

### CSS and components

- Compile or render enough Tailwind output to prove `dark:` follows `data-appearance`, not only a source-string assertion.
- Appearance mode, accent, adjustment, import, export, and reset behavior.
- Listener registration and cleanup under React Strict Mode.
- Overlay transparency and hardcoded-glass invariants.

### Native smoke

- Forced Light on a dark OS.
- Forced Dark on a light OS.
- System-mode transition while all four windows exist.
- Main and log-viewer native title bars.
- Overlay transparency with no opaque-box regression.
- Transform-review transparency.
- Accent persistence across a full app restart.

### Required checks

```bash
cd app && npm test
cd app && npx tsc --noEmit
cd app/src-tauri && cargo test -- --test-threads=1
```

For frontend work, visually verify with the configured browser tooling. For native window, title-bar, overlay, and restart behavior, run a native Tauri smoke test.

---

## 11. Explicit cuts

- Remote themes or marketplace.
- User-managed multi-theme library.
- Font controls.
- Translucent sidebar.
- Dock-icon themes.
- Light overlay or transform-review glass.
- In-app reduce-motion override.
- Clipboard-based theme exchange.
- Theme-controlled arbitrary CSS properties.

---

## 12. Definition of done

The Theme Engine is complete when:

- Mode and accent survive restart without a wrong-theme flash.
- Forced appearance is consistent across semantic tokens, Tailwind variants, and native chrome.
- Every themed webview maintains a concrete appearance state without event loops.
- Overlay and transform-review transparency remain intact.
- Invalid local or imported data cannot apply arbitrary properties or inaccessible colors.
- Advanced colors do not ship before semantic token debt is resolved.
- Import/export is local, bounded, atomic, and clipboard-independent.
- Tests and documentation encode the architecture rather than relying on manual memory.
