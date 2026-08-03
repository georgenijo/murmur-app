# Appearance Theme Engine

Murmur's Theme Engine applies a local, versioned appearance document to the
Sonic Canvas semantic token system. It supports System, Light, and Dark modes,
custom accent/background/foreground colors, bounded contrast adjustment, and
local JSON import/export. It never contacts a theme service and never uses the
clipboard.

## Window scope

The main window is the themed webview and maintains a concrete
`data-appearance="light"` or `data-appearance="dark"` attribute even when the
saved preference is `system`.

The overlay and transform-review popover are deliberately outside theme
synchronization. They remain transparent, always-dark glass surfaces. In
particular, the shared `body.overlay-window { background: transparent }`
invariant must never be replaced by an opaque theme background.

## Storage and first paint

Appearance is independent from dictation settings:

- localStorage key: `murmur-appearance`
- schema version: 1
- one active document, with a monotonic revision within each epoch; the last
  writable signed 32-bit revision rolls to `1` and emits `reason: "repair"`
- authoritative fields: mode and theme configuration
- derived fields: exact light/dark resolved-token cache

Every normal write sanitizes the configuration, resolves both palettes, and
writes configuration plus cache together. The cache is strictly validated and
is never authoritative after runtime initialization.

The main HTML entry loads a parser-blocking classic script before its module
entry. The script reads only the bounded appearance document,
resolves System through `matchMedia`, validates the cache version, exact token
allowlist, and hex values, then applies the selected table. It performs no color
conversion. Corrupt, partial, oversized, or unknown-version storage falls back
to Sonic/System without throwing. Runtime initialization recomputes the cache
from authoritative configuration and repairs stale derived data.

Tailwind's `dark:` variant follows `data-appearance`, not the OS media query.
The CSS light defaults and dark media query remain a base-color fallback for
bootstrap failure; Murmur does not promise full no-JavaScript theme parity.

## Synchronization and native chrome

The main webview is the only writer and runtime owner. It emits one revisioned
`appearance-changed` event for user edits, reset, import, cache repair, or the
explicit high-water revision rollover. System-mode OS changes are applied
locally by its `matchMedia` listener and emit no event.

Main also owns application-level native appearance:

- System: application `setTheme(null)`
- Light: application `setTheme('light')`
- Dark: application `setTheme('dark')`

Only the main capability grants `core:app:allow-set-app-theme`, keeping the
decorated main title bar aligned with its themed pixels.

## Color resolution and accessibility

Resolution starts from the exact Sonic light or dark fixture, applies contrast
as a stored integer from -100 through 100 (omitted/default is 0) and
dependency-free OKLab/OKLCH color derivation, applies explicit allowlisted
token overrides, then enforces the semantic contrast matrix. The resolver
returns exactly the supported `--murmur-*` tokens and records automatic
adjustments for the Settings UI.

Mutable palettes first repair the complete surface ladder onto one compatible
black-or-white contrast pole, with enough headroom to retain chromatic
foregrounds. `primary` is solved against its raw surface use, then
`primary-dim` is aligned as a distinct opaque hover/gradient role on the same
pole. Status and surface foregrounds are solved next, including `on-surface`
against the exact 5%, 10%, and 15% primary composites; and `on-primary` is
derived only after the primary pair is stable.
Each solve searches a single OKLCH lightness axis, preserving the requested hue
and as much in-gamut chroma as possible. This avoids sequential repairs that
make one pair accessible by breaking another. The emergency preset fallback
remains a fail-closed postcondition and is not part of ordinary color
resolution.

Text pairs meet WCAG AA (4.5:1), and focus/interactive/status pairs meet 3:1
where required. Explicit overrides cannot bypass the matrix; a value that
cannot be adjusted deterministically falls back to the nearest valid preset
token. Reset restores the exact Sonic fixtures.

Clearing an Accent, Background, or Foreground override removes only that
field. The theme intentionally remains Custom so any other overrides,
contrast, and accessibility adjustments stay active. Only **Reset to Sonic**
returns the preset to Sonic and restores the exact built-in fixtures.

The untouched Sonic preset is intentionally byte-for-byte compatible with the
14 tokens that shipped before status tokens were introduced. Its supported
matrix is validated directly against the exact light and dark fixtures:
`on-surface` and `on-surface-variant` on every surface; untinted `primary` on
every surface; `on-surface` on exact 5%, 10%, and 15% primary-tinted
composites over every possible adjacent surface; `on-primary` on `primary` and
`primary-dim`; `success` and `warning` both directly and on their 10%-tinted
containers on every surface; and all primary and status non-text indicators
on every surface. The added Sonic `success` and `warning` fixtures were chosen
to satisfy those pairs without changing the original 14 tokens.

Only five exact legacy exceptions remain:

- `outline-variant` is decorative; meaningful focus and selection indicators
  use `primary`.
- Sonic `error` text is not supported on `surface-container-highest`.
- Sonic `error` text on a 10%-error-tinted container is not supported when the
  adjacent surface is `surface-container-high` or
  `surface-container-highest`.
- Sonic `primary` is not supported as text on 10% or 15% primary-tinted
  containers. Tinted containers use `on-surface`.
- Sonic `on-surface-variant` is not supported on primary-tinted containers.
  Tinted containers use `on-surface`.

Static UI debt tests enforce those usage boundaries. Any mutable path—accent,
background, foreground, contrast, or an explicit token override—enforces the
complete text and non-text matrices, including all surface rungs, status
tints, and `on-surface` on primary tints. `on-surface-variant` is validated on
raw surfaces only, and `primary-dim` is an opaque hover/gradient background
paired with `on-primary`; neither has invented self-tint requirements. The UI
uses only `on-surface` on primary-tinted containers so exact Sonic and custom
themes share one safe usage rule. This narrow compatibility contract preserves
exact reset parity without allowing a custom theme to bypass accessibility
repair.

The runtime applier sets only:

- `data-appearance`
- `color-scheme`
- allowlisted `--murmur-*` custom properties

It never sets `html`/`body` backgrounds or mutates overlay/popover glass.

## File exchange

The dialog plugin selects a `.json` path. It does not read or write the file.
Two main-window-only Rust commands perform transport:

- `read_theme_file(path) -> String`
- `write_theme_file(path, contents) -> ()`

Both use a 64 KiB UTF-8 byte limit. Reads reject symbolic links and non-regular
files. Writes reject symbolic-link and non-regular destinations, write a unique
sibling temporary file, flush it, atomically rename it over the destination,
then sync the parent directory. New exports use Unix mode `0600`; replacement
exports preserve the existing regular file's permissions. A failed write or
publish removes the temporary file and preserves an existing destination.
Paths and contents are not logged.

The frontend owns schema validation, preview, adjustment reporting, and commit.
Theme files contain authoritative configuration only. Imported revisions and
resolved caches are ignored and regenerated before storage. Malformed,
unsupported, or oversized imports fail before changing the live appearance.
Import and export never read or write the clipboard.

## Privacy and scope

All appearance data remains local. There is no telemetry expansion, remote
marketplace, cloud fallback, arbitrary CSS input, font theming, or
user-managed theme library.
