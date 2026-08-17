# Appearance Theme Engine

Murmur's Theme Engine applies a local, versioned appearance document to the
Sonic Canvas semantic token system. It supports System, Light, and Dark modes,
custom accent/background/foreground colors, bounded contrast adjustment, and
a durable named-theme library. Local import accepts Murmur JSON plus VS Code
JSON/JSONC themes. An explicit **Browse community** flow can search and download
supported Open VSX color-theme extensions; opening Appearance or using local
themes performs no marketplace request. Theme workflows never use the
clipboard and never execute extension code.

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
- authoritative fields: mode, compiled theme configuration, and optional
  light/dark library-owner selection
- derived fields: exact light/dark resolved-token cache

Saved themes have a separate ownership and failure domain:

- synchronous cache: `murmur-theme-library`
- durable main-window copy: `theme-library.json`
- schema version: 1, revisioned with optimistic concurrency
- bounds: 128 entries and 1 MiB serialized
- each entry has a stable ID, display name, supported light/dark variants,
  theme configuration, and local or Open VSX provenance

Main boot hydrates the library from disk before React renders. Existing
localStorage-only libraries migrate to disk. Invalid entries and duplicates are
dropped by one explicit repair write; corrupt JSON, unsupported schemas, or an
oversized library fail closed without replacing the active appearance. The
small `murmur-appearance` document stays the only parser-blocking first-paint
input, so marketplace metadata is never read by the boot script.

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
`appearance-changed` event for user edits, reset, import, library selection or
update, cache repair, or the explicit high-water revision rollover. System-mode OS changes are applied
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

Selecting a saved palette compiles its resolved tokens into the active
document. Light and dark can name different library owners; a paired entry can
set both at once. This makes utility consumers independent from the larger
library at first paint. Editing any control on a saved or explicitly compiled
palette derives a seed-based Custom theme from the currently rendered colors,
sets both owners to `custom`, and leaves the saved source unchanged. Removing
an active entry falls that appearance back to Sonic. A collection update
recompiles stable active IDs and falls removed active variants back to Sonic.

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

Reads use a 256 KiB UTF-8 byte limit so bounded VS Code JSON/JSONC files can be
converted; exports remain bounded to 64 KiB. Reads reject symbolic links and
non-regular files. Writes reject symbolic-link and non-regular destinations, write a unique
sibling temporary file, flush it, atomically rename it over the destination,
then sync the parent directory. New exports use Unix mode `0600`; replacement
exports preserve the existing regular file's permissions. A failed write or
publish removes the temporary file and preserves an existing destination.
Paths and contents are not logged.

The frontend owns schema validation, adjustment reporting, library persistence,
and commit. A successful local file selection is installed and applied as one
operation; there is no raw token-dump confirmation step. Invalid or unsupported
files still fail before either the library or active appearance changes.
Murmur theme-file v1 remains compatible for active appearance exchange. Named
library-theme v2 files carry only name, supported modes, and authoritative
theme configuration—never library IDs or remote provenance. VS Code imports
read only the supported workbench color allowlist; token rules, fonts, images,
CSS, commands, and extension code are ignored. JSONC comments/trailing commas,
hex alpha, and `color(display-p3 …)` values are converted, then every palette
runs through the normal Murmur accessibility resolver. Imported revisions and
resolved caches are ignored and regenerated before storage. Malformed,
unsupported, or oversized imports fail before changing the live appearance.
Import and export never read or write the clipboard.

## Appearance interaction

Appearance presents System, Light, and Dark as one compact segmented control.
The page is flat rather than wrapped in an additional settings container. The theme
library follows T3 Code's collection model: one imported extension is one
fixed-width, fixed-height tile, not one card per source variant. Tiles wrap from
the left and never stretch to fill unused page width. Each card shows light and
dark preview circles. The ring and sun/moon badge on each circle identify the
effective owner of that appearance without a second Active badge.

Clicking a card applies its default light/dark pair in one commit. A single
light or dark circle is a labelled preview, not a no-op button; the active tile
has a compact check beside its name. Collections with multiple source variants
turn their circles into controls and reveal small radial choices on hover or
keyboard focus. They never insert an expanding selector panel into the page.
Export and removal are
compact icon actions in the title row and stop card-click propagation.

The top-level actions are **Create theme** and **Import theme**. Import opens a
compact choice between a local file and Open VSX discovery. The color editor is
modal, so manual controls do not lengthen the normal Appearance page. There is
no ambiguous “Save current” action; manual color edits are already durable in the
active appearance document, and **Export current** creates a portable file.
Meaningful control and card boundaries use the readable foreground-variant
token, while `outline-variant` is reserved for non-interactive dividers. This
keeps controls discernible even when an imported editor theme supplies a very
subtle outline color.

## Open VSX community themes

The community dialog makes its network boundary visible before search. Search
sends the typed query and ordinary connection metadata directly to
`https://open-vsx.org`; Murmur has no proxy, account, remote theme cache,
background polling, or automatic update request. Results are limited to the
Open VSX `Themes` category and the supported SPDX license set: 0BSD,
Apache-2.0, BSD-2-Clause, BSD-3-Clause, CC0-1.0, ISC, MIT, MPL-2.0, and
Unlicense. Murmur verifies that a result has a color-theme manifest before
showing it.

Choosing **Add** starts a separate bounded download. All manifest, checksum,
and VSIX transport URLs must be credential-free HTTPS URLs on `open-vsx.org`;
fetch uses `credentials: "omit"`. The importer enforces response-size and
timeout limits, verifies Open VSX's SHA-256 digest, rejects ZIP64, traversal,
excess entries, excessive expanded size or compression ratio, and checks the
packaged publisher/name/version/license against the selected result. It reads
only `extension/package.json` and declared color-theme JSON/JSONC files. Theme
`include` inheritance is allowed within the extension root with bounded depth,
file count, size, and cycle detection.

Converted variants receive deterministic IDs derived from extension identity
and declared source paths. An extension's entries form one collection. **Update**
requires confirmation and atomically replaces the expected installed
collection; a concurrent local change refuses the update. No extension script,
grammar, icon, font, binary, activation event, command, or dependency is loaded
or executed. Source links open only after a user click.

## Privacy and scope

Active and saved theme data remains local and is excluded from telemetry,
diagnostics, history, and clipboard flows. Open VSX queries and downloads occur
only after explicit actions in the community dialog and go directly to Open
VSX as disclosed above. There is no cloud inference, Murmur account, background
sync, automatic marketplace traffic, arbitrary CSS input, font theming, or
extension execution.
