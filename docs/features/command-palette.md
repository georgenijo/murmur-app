# Command Palette

## Overview

Murmur's main window has seven settings pages, two tabs, a log viewer, an updater, and a setup assistant behind them. The palette makes all of it one keystroke away: press **⌘K**, type three letters, press Enter.

It is a navigation surface only. Every row carries its own `run` callback supplied by `App.tsx`, so the palette never learns what a command *means* and there is exactly one definition of each action.

## Shortcuts

| Keys | Action |
|------|--------|
| `⌘K` | Toggle the palette |
| `⌘F` | Focus the transcript search box |
| `⌘,` | Open Settings |
| `⌘L` | Open the log viewer |

Control is accepted in place of Command for keyboard-only setups. Adding Option or Shift always passes the event through — those combinations belong to the focused control, and Murmur must not shadow text-editing shortcuts. The mapping is a pure function (`mainWindowShortcut`) so what is bound, and what is deliberately not, is pinned down in tests.

## Commands

| Section | Commands |
|---------|----------|
| Recording | Start / Stop recording · Enable / Disable Murmur |
| History | Search transcripts · Copy last transcript* · Export history to a Markdown file* |
| Navigation | Go to Record · Go to Transcribe File |
| Settings | One row per page: Recording, Transcription, Transform, Text & Vocabulary, Delivery, Performance, General |
| Diagnostics | Open log viewer |
| App | Check for updates · About Murmur · Re-run setup assistant |

\* Offered only when history is non-empty, so no row is a dead end.

Rows whose meaning depends on state are labelled with the state, not the toggle: while recording, the row reads *Stop recording*; while Murmur is disabled, *Enable Murmur*.

Opening a settings page uses a `{ page, token }` request rather than a bare page id. The token makes a repeat request for the page you are already on still register, and an unrecognised page id resolves back to the first page instead of rendering an empty pane.

## Matching and ranking

Matching is explicit tiers rather than a generic fuzzy library — with a small fixed command set, predictable ranking makes the first row feel obvious and is easy to test.

Per query token, highest first:

| Tier | Example (`del`) |
|------|-----------------|
| exact title | — |
| title prefix | `Delivery settings` |
| word prefix inside the title | `Settings: Delivery` |
| title substring | `Zzz delivery` |
| section or keyword substring | a command tagged `clipboard` |
| subsequence of the title | `Dump event log` |

Tokens are **ANDed** — `settings del` narrows to the Delivery page rather than matching every settings row. A command matches only if *every* token matches something. Scores sum across tokens, shorter titles win ties, and declaration order breaks the remainder, so results are fully deterministic.

## Interaction

- Opens with the query cleared and the first row selected; focus moves to the input on the next frame so the palette wins over whatever had focus.
- `↑` / `↓` (and `⌃P` / `⌃N`) move the selection and wrap at both ends. The selection resets to the top whenever the query changes.
- `Enter` runs the selected row and closes. With no results it does nothing.
- `Escape`, or a click on the backdrop, closes without running anything. A click inside the dialog does not.
- Mouse movement over a row selects it, so keyboard and pointer agree on what Enter would do.
- The list is a `listbox`/`option` tree with `aria-activedescendant` on the input, and the dialog is a labelled `aria-modal`.

## Files

| File | Role |
|------|------|
| `app/src/lib/commandPalette.ts` | `PaletteCommand`, scoring tiers, `filterCommands`, `moveSelection` |
| `app/src/lib/keyboardShortcuts.ts` | Pure `mainWindowShortcut` event→action mapping |
| `app/src/components/CommandPalette.tsx` | The dialog: input, list, keyboard handling |
| `app/src/App.tsx` | The command registry and the window-level key listener |
| `app/src/components/settings/SettingsPanel.tsx` | `pageRequest` handling and `resolvePage` |

## Tests

- `app/src/lib/commandPalette.test.ts` — tier ordering, keyword/section matching, token AND, tie-breaking, stability, and selection wrapping.
- `app/src/lib/keyboardShortcuts.test.ts` — every binding, Control equivalence, case-insensitivity, and the pass-through cases.
- `app/src/components/CommandPalette.test.tsx` — filtering, arrow navigation, Enter/Escape, click-to-run, backdrop vs dialog clicks, and reset-on-open.
