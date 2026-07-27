# History Workspace

## Overview

Transcription history is where a dictated line goes to be found again. The workspace turns the plain reverse-chronological list into something you can work in: search it, narrow it, keep the entries that matter, and take a set of them out of the app as Markdown, plain text, or JSON.

Everything here is local. History lives in `localStorage` under `dictation-history`, and the only thing that leaves the app is an export the user explicitly asks for.

## Search

The search box filters as you type.

- Matching is case-insensitive and searches the transcript text plus, for imported files, the source file name.
- Multiple words are **ANDed** — `tauri release` keeps only entries containing both. Narrowing a query never widens the result set.
- Matches are highlighted in place. `matchSegments` splits an entry into alternating plain/matched runs, merging overlapping and adjacent ranges so two tokens that overlap (`there` and `here` in "therein") produce one highlight rather than nested ones. Original casing is preserved, and the segments always reassemble to the original text exactly.
- Highlight ranges are capped per entry, so a one-character query cannot emit thousands of spans for a long transcript.
- Escape clears the query when it is non-empty.

## Filters

Four chips: **All**, **Mic**, **File**, **Pinned**. Entries saved before the `source` field existed count as `Mic`. The pinned chip carries the current pin count. Filters compose with the search query.

The counter on the right reads `N of M` while anything is filtered, and just the total otherwise.

## Pinning

Pin an entry to keep it. Pinning has three consequences:

1. **Pinned entries are exempt from the rolling trim.** Ordinary entries keep a 50-entry budget; pinned entries have their own budget of 25. Both are enforced by index (not by id) in `trimHistory`, so entries created inside the same millisecond can't confuse it.
2. **Pinned entries sort to the top**, newest first inside each group.
3. **"Clear history" skips them.** With pins present, the primary button becomes `Clear N unpinned` and a separate `Clear all` appears next to it. Both are two-step confirms that disarm after four seconds — deliberately not `window.confirm`, since a native modal steals focus from the main window.

Pinning past the ceiling is refused rather than silently dropping the oldest pin: `togglePinned` returns the same array, and the panel says so (`Pin limit reached (25). Unpin something first.`). Unpinning is always allowed.

## Export

The **Export** menu acts on **exactly what is currently shown** — filters and search included — so "copy today's CLI notes" is a search plus two clicks.

| Format | Shape |
|--------|-------|
| Markdown | `# Murmur transcript history`, an export stamp, then one `##` section per entry with source, duration, and pin state |
| Plain text | `[timestamp · source · duration]` header line per entry, blank-line separated |
| JSON | `{ schema: "murmur.history.v1", exportedAt, count, entries: [...] }` |

Each format can go to the clipboard or to a file. The two menu groups repeat the same format names, so each is a labelled `role="group"` and every item carries the verb in its accessible name ("Copy 5 shown as Markdown", "Save 5 shown as Markdown"). Entries are ordered exactly as displayed (pinned first, then newest first), and timestamps are rendered as local `YYYY-MM-DD HH:MM:SS` so an export reads the same on every machine.

**`teachingContext` is never exported.** The bundle id and project root captured at recording start are local scope metadata for Correct-and-Teach, not part of a transcript the user is sharing. All three formats are asserted against this in tests.

### Saving to a file

`Save to file` opens the native save dialog (`dialog:allow-save`, already granted to the main window) with a suggested name of `murmur-history-<YYYY-MM-DD-HHMM>.<ext>`, then hands the chosen path and the rendered payload to the `save_text_export` Rust command.

That command is deliberately narrow — it is a document sink, not a general file-write primitive:

- the path must be absolute, must not be an existing directory, and its parent directory must already exist;
- the file name must not start with a dot;
- the extension must be one of `.json`, `.md`, `.txt` (case-insensitive) — anything else, including a missing extension, is refused;
- the payload is capped at 8 MB;
- the write is atomic: a temp sibling in the destination directory, then a rename, with the temp file removed if either the write or the rename fails.

Cancelling the dialog is a no-op — no command call, no message.

## Correct and Teach

Correct-and-Teach still targets **the newest entry in the whole history**, not the first row on screen. Pinning and filtering reorder the list, so the button is anchored to `entries[entries.length - 1]` and travels with that entry wherever it is displayed.

## Files

| File | Role |
|------|------|
| `app/src/lib/history.ts` | Entry shape, trim/pin/filter/sort, match segmentation, export rendering |
| `app/src/lib/historyExport.ts` | Clipboard and save-dialog wrappers around the pure renderer |
| `app/src/lib/hooks/useHistoryManagement.ts` | State + persistence, including pin and pinned-safe clear |
| `app/src/components/history/HistoryPanel.tsx` | Search, chips, cards, export menu, clear actions |
| `app/src-tauri/src/commands/export.rs` | `save_text_export` — validation and atomic write |

## Tests

- `app/src/lib/history.test.ts` — trim budgets, duplicate ids, pin ceiling, filters, sorting, match segmentation (including regex metacharacters and reassembly), all three export formats, and the teaching-context exclusion.
- `app/src/components/history/HistoryPanel.test.tsx` — search/filter/pin interaction, the pin-ceiling message, export scope, dialog cancellation, and the two-step clears.
- `app/src-tauri/src/commands/export.rs` — the full validation matrix plus atomic overwrite and temp-file placement.
