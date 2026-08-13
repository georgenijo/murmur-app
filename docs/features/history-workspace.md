# History Workspace

## Overview

History has three deliberately separate workspaces: Transcripts, Meetings, and
Queries. Transcription history turns the plain reverse-chronological list into
something you can search, narrow, and export. Meetings use their own searchable
SQLite sessions. Queries expose only explicitly retained Voice Query content;
they never join transcript export or Correct and Teach.

Everything here is local. The transcript durable source of truth is `history.json` in the
per-bundle app data directory; `localStorage` under `dictation-history` is a
synchronous cache. The main window hydrates that cache before React renders,
and upgrades migrate an existing cache to disk once when no durable file
exists. History keeps its rolling 200-entry cap (`trimHistory` drops the oldest
beyond it, by index so same-millisecond ids cannot confuse it), and the only
thing that leaves the app is an export the user explicitly asks for. An entry
worth keeping past the cap belongs in an export or the knowledge store, not in
a special history state.

Settings → Model & Output → Output includes **Save Transcription History**. Turning it off makes the single `addEntry` boundary discard new microphone and imported-file transcripts before they reach React state, localStorage, or `history.json`. Current transcription delivery and content-free usage statistics continue normally. Previously saved entries remain visible until the user explicitly clears them, so changing a preference never silently deletes data.

Voice Query has a different, off-by-default boundary. Settings → Text → Voice
Query includes **Keep Voice Query history on this Mac**. When enabled for a
pass, Rust stores the original question and answer with bounded metadata in a
separate local SQLite database. History → Queries loads at most 50 records at a
time, filters by provider, and offers **Delete all query history** as a direct
one-click purge. The store keeps at most 200 newest entries. Turning the toggle
off affects new passes only; it never silently deletes old records. A pass that
actually appends app/window/selection context to its provider prompt is
display-only and skips the entire history row, because raw or structured
provider output may quote that context. Context configured but unavailable or
excluded for the active app is never appended and does not suppress history.

Query records are never cached in localStorage or merged into
`dictation-history`. They are not editable, teachable, searchable through the
transcript index, copied into transcript exports, or included in saved files.
Optional app/window/selection context, composed prompts, stderr/error detail,
paths, argv, and environment values are excluded from the store. Structured
Claude/Codex raw-fallback passes also skip the whole row because their raw
archives can contain user frames that echo the composed prompt. The provider
filter and pagination cross only main-window-gated IPC, and live change events
contain no content.

Writes publish atomically through Rust with owner-only file permissions and
never log transcript content. A
history file that is oversized, invalid UTF-8, invalid JSON, or not an array is
quarantined beside the original path instead of deleted. The frontend then
falls back to its cache and repairs the durable copy during the same startup
when one exists. Clear History removes both the cache and durable file.

## Search

Search rests as a compact icon so it does not dominate the history toolbar. Hover previews the field; clicking the icon, focusing the input, or invoking the app-wide search shortcut pins it open. An active non-empty query also keeps the field visible after focus leaves.

- Leaving a hover-only preview collapses it.
- Leaving while the input is focused does not collapse it or steal focus.
- Clicking elsewhere collapses an empty search; a non-empty search remains visible so an active filter is never hidden.
- Escape or the close button clears the query, releases focus, and collapses the field. If the pointer is still over the field, hover is suppressed until it genuinely leaves so the just-closed field cannot immediately reopen.
- Motion uses the app's reduced-motion contract; width and content transitions are removed when reduced motion is requested.

- Matching is case-insensitive and searches the transcript text plus, for imported files, the source file name.
- Multiple words are **ANDed** — `tauri release` keeps only entries containing both. Narrowing a query never widens the result set.
- Matches are highlighted in place. `matchSegments` splits an entry into alternating plain/matched runs, merging overlapping and adjacent ranges so two tokens that overlap (`there` and `here` in "therein") produce one highlight rather than nested ones. Original casing is preserved, and the segments always reassemble to the original text exactly.
- Highlight ranges are capped per entry, so a one-character query cannot emit thousands of spans for a long transcript.

## Filters

Three chips: **All**, **Mic**, **File**. Entries saved before the `source` field existed count as `Mic`. Filters compose with the search query.

The counter on the right reads `N of M` while anything is filtered, and just the total otherwise.

## Clearing

`Clear History` removes everything. It is a two-step confirm that disarms after four seconds — deliberately not `window.confirm`, since a native modal steals focus from the main window.

## Export

The **Export** menu acts on **exactly what is currently shown** — filters and search included — so "copy today's CLI notes" is a search plus two clicks.

| Format | Shape |
|--------|-------|
| Markdown | `# Murmur transcript history`, an export stamp, then one `##` section per entry with source and duration |
| Plain text | `[timestamp · source · duration]` header line per entry, blank-line separated |
| JSON | `{ schema: "murmur.history.v1", exportedAt, count, entries: [...] }` |

Each format can go to the clipboard or to a file. The two menu groups repeat the same format names, so each is a labelled `role="group"` and every item carries the verb in its accessible name ("Copy 5 shown as Markdown", "Save 5 shown as Markdown"). Entries are ordered exactly as displayed (newest first), and timestamps are rendered as local `YYYY-MM-DD HH:MM:SS` so an export reads the same on every machine.

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

Correct-and-Teach still targets **the newest entry in the whole history**, not the first row on screen. Sorting and filtering reorder the list, so the button is anchored to `entries[entries.length - 1]` and travels with that entry wherever it is displayed.

## Files

| File | Role |
|------|------|
| `app/src/lib/history.ts` | Entry shape, trim/filter/sort, match segmentation, export rendering |
| `app/src/lib/historyExport.ts` | Clipboard and save-dialog wrappers around the pure renderer |
| `app/src/lib/hooks/useHistoryManagement.ts` | State + retention boundary: add, update, clear |
| `app/src/lib/durableUserData.ts` | Boot hydration, localStorage migration, write-through and clear bridge |
| `app/src-tauri/src/commands/settings_store.rs` | Bounded atomic durable blobs and corruption quarantine |
| `app/src/components/history/HistoryPanel.tsx` | Search, chips, cards, export menu, clear actions |
| `app/src/lib/hooks/useQueryHistory.ts` | Active-workspace-only paged query-history reads, provider filter, live refresh, and purge |
| `app/src/components/history/QueryHistoryPanel.tsx` | Local question/answer cards and one-click purge; no export or edit path |
| `app/src-tauri/src/commands/export.rs` | `save_text_export` — validation and atomic write |

## Tests

- `app/src/lib/history.test.ts` — the 200-entry trim, duplicate ids, filters, sorting, match segmentation (including regex metacharacters and reassembly), all three export formats, and the teaching-context exclusion.
- `app/src/components/history/HistoryPanel.test.tsx` — search/filter interaction, export scope, dialog cancellation, and the two-step clear.
- `app/src-tauri/src/commands/export.rs` — the full validation matrix plus atomic overwrite and temp-file placement.
- `app/src-tauri/src/commands/settings_store.rs` — history/stats/settings shape
  bounds, atomic overwrite, quarantine, and idempotent clear.
- `app/src/lib/durableUserData.test.ts` — disk-authoritative hydration,
  one-time localStorage migration, isolated failure, write-through, and clear.
- `app/src/lib/hooks/useHistoryManagement.test.tsx` — disabled retention keeps
  existing entries while rejecting new transcript content.
