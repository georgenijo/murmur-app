# Command Palette

## Overview

Murmur's main window has direct Settings destinations, local workspaces, an updater, and a setup assistant behind it. The palette makes all of it one keystroke away: press **⌘K**, type three letters, press Enter.

It dispatches navigation and explicit actions. Every row carries its own `run` callback supplied by `App.tsx`, so the palette never learns what a command *means* and there is exactly one definition of each action.

## Shortcuts

| Keys | Action |
|------|--------|
| `⌘K` | Toggle the palette |
| `⌘F` | Focus the transcript search box |
| `⌘,` | Open Settings on General |
| `⌘L` | Open Settings → Performance |

Control is accepted in place of Command for keyboard-only setups. Adding Option or Shift always passes the event through — those combinations belong to the focused control, and Murmur must not shadow text-editing shortcuts.

While focus is inside a text field, **only the Command form is accepted**: macOS binds Control+letter to emacs-style editing inside text controls (⌃F forward, ⌃K kill-to-end-of-line), and the search box, settings fields, and knowledge editor must keep those. The mapping is a pure function (`mainWindowShortcut`, with `isEditableTarget` supplying the focus context) so what is bound, and what is deliberately not, is pinned down in tests.

## Commands

| Section | Commands |
|---------|----------|
| Recording | Start / Stop recording · Enable / Disable Murmur |
| History | Paste Last / Retry Delivery · Search transcripts · Copy last transcript\* · Export history to a Markdown file\* |
| Navigation | Go to Record · Go to Transcribe File |
| Settings | One row per main page, plus Keyboard Shortcuts: Recording, Transcription, AI & Models, Text & Vocabulary, Delivery, Benchmark, Performance, Appearance, General |
| Diagnostics | Open performance diagnostics |
| App | Check for updates · About Murmur · Re-run setup assistant |

\* Offered only when history is non-empty, so no row is a dead end.

Rows whose meaning depends on state are labelled with the state, not the toggle: while recording, the row reads *Stop recording*; while Murmur is disabled, *Enable Murmur*.

Opening a settings page uses a `{ page, token }` request rather than a bare page id. The token makes a repeat request for the page you are already on still register, and an unrecognised page id resolves back to General instead of rendering an empty pane. Contextual Text & vocabulary and App overrides requests retain a route back to the Customize hub. The Keyboard Shortcuts command opens the read-only General detail page directly.

## Find transcripts (opt-in)

Settings → Delivery → History → **Find transcripts in the command palette**
(`paletteTranscriptSearch`) is off by default. When it is on, typing in the
palette also lists matching saved transcripts in a **Transcripts** section
after the matching commands, and Enter (or a click) copies the chosen
transcript's full stored text to the clipboard and closes the palette.

- Matching is the History search rule: case-insensitive, and every typed word
  must appear in the transcript or, for an imported file, its file name. The
  palette and History therefore find the same transcripts.
- Nothing is listed until the query has at least two non-space characters, so
  opening the palette never shows transcript text by itself.
- At most five rows, newest first; blank transcripts are skipped. Each row is
  one line: a snippet that starts shortly before the first match (on a word
  boundary, with a leading `…`), the matched words emphasised, and the day and
  time History shows for that entry.
- Transcript rows are never re-ranked against commands; commands keep their
  tiers above them. The placeholder reads *Type a command or search
  transcripts…*, the empty state *No matching command or transcript*, and the
  footer verb reads *copy* while a transcript row is selected.
- Rows are computed from the in-memory history on each keystroke. A deleted,
  expired, or cleared transcript disappears with it; nothing is cached,
  indexed, logged, or sent.

The feature is display-only. It never changes recording, delivered text,
delivery, history writes, statistics, or logs, and the clipboard is written
only when a transcript row is chosen. With the setting off the palette is
unchanged. The rows come from `transcriptPaletteCommands` in
`app/src/lib/paletteTranscripts.ts`, passed to the palette as `extraResults`.

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
- `Tab` is swallowed: the input is the dialog's only focusable element, so focus cannot escape to the page behind it.
- Mouse movement over a row selects it, so keyboard and pointer agree on what Enter would do.
- The list is a `listbox`/`option` tree with `aria-activedescendant` on the input, and the dialog is a labelled `aria-modal`.

## Files

| File | Role |
|------|------|
| `app/src/lib/commandPalette.ts` | `PaletteCommand`, scoring tiers, `filterCommands`, `moveSelection` |
| `app/src/lib/paletteTranscripts.ts` | Opt-in transcript rows: matching, snippet, copy action |
| `app/src/lib/keyboardShortcuts.ts` | Pure `mainWindowShortcut` event→action mapping and shared fixed-binding metadata |
| `app/src/components/settings/KeyboardShortcutsSettings.tsx` | Live read-only reference for global and main-window shortcuts |
| `app/src/components/CommandPalette.tsx` | The dialog: input, list, keyboard handling |
| `app/src/App.tsx` | The command registry and the window-level key listener |
| `app/src/components/settings/SettingsPanel.tsx` | `pageRequest` handling and `resolvePage` |

## Tests

- `app/src/lib/commandPalette.test.ts` — tier ordering, keyword/section matching, token AND, tie-breaking, stability, and selection wrapping.
- `app/src/lib/paletteTranscripts.test.ts` — minimum query, History-rule matching, cap and order, snippets, copy rows.
- `app/src/components/CommandPalette.test.tsx` — extra rows after commands, highlight, footer verb, placeholder and empty state.
- `app/src/App.captureSafety.test.tsx` — the setting off lists no transcripts; on, a chosen row copies its full text.
- `app/visual-tests/palette-transcripts.spec.ts` — row geometry and footer verb in light and dark at two window sizes (`?state=palette-transcripts`).
- `app/src/lib/keyboardShortcuts.test.ts` — every binding, Control equivalence, case-insensitivity, and the pass-through cases.
- `app/src/components/CommandPalette.test.tsx` — filtering, arrow navigation, Enter/Escape, click-to-run, backdrop vs dialog clicks, and reset-on-open.

## Compact presentation

The palette uses 34px result rows in a 520px dialog, a single focused search row,
section headings without repeated category badges, and sans keycaps. Start
**dictation** shows the configured trigger key; Hold (or Double-tap) stays outside
the keycap. The binding follows Recording settings rather than assuming Left
Shift. About and the update dialog use the same 2px blurred backdrop; About uses
the existing sidebar waveform mark. Update actions appear in keyboard/DOM order
Later (or Quit), then the primary action, in an intrinsic-height dialog whose
release notes scroll when needed. Focus trapping and update recovery stay intact.

The recording command's binding follows its current action: idle shows Hold or
Double-tap, double-tap recording shows Tap to stop, and hold-down recording shows
Release. The connecting/cancel state has no start-gesture hint. Combined-mode
recording omits the stop hint because the palette has no recording gesture owner.
