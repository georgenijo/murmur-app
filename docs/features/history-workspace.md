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

Stored transcript entries use schema version 2. New live-dictation entries keep
the exact raw backend recognition separately from the delivered text, together
with the local model ID, recording correlation, resolved Mode/profile identity,
and content-free transform-stage outcomes. Legacy entries migrate to v2 without
copying delivered text into the unknown raw-recognition field. Normal dictation
audio is never part of history.

Settings → Model & Output → Output includes **Save Transcription History**. Turning it off makes the single `addEntry` boundary discard new microphone and imported-file transcripts before they reach React state, localStorage, or `history.json`. Current transcription delivery and content-free usage statistics continue normally. Previously saved entries remain visible until the user explicitly clears them, so changing a preference never silently deletes data.

Voice Query has a different, off-by-default boundary. Settings → Text → Voice
Query includes **Keep Voice Query history on this Mac**. When enabled for a
pass, Rust stores the original question and answer with bounded metadata in a
separate local SQLite database. History → Queries loads at most 50 records at a
time, filters by provider, and offers **Delete all query history** as a direct
one-click purge. The store keeps at most 200 newest entries. Turning the toggle
off affects new passes only; it never silently deletes old records. Every
recognized query is retained when this explicit local-history consent is on,
including a query that appended app/window/selection context to its provider
prompt or used raw structured-provider fallback. Context is not stored as a
separate field, but a retained answer may quote context that was sent to its
CLI.

Query records are never cached in localStorage or merged into
`dictation-history`. They are not editable, teachable, searchable through the
transcript index, copied into transcript exports, or included in saved files.
There is no separate app/window/selection context or composed-prompt field in
the store. Stderr/error detail, paths, argv, and environment values are also
excluded. Because a saved answer can quote context or raw provider output,
enabling Voice Query history is explicit local consent to retain the complete
question-and-answer result. The provider filter and pagination cross only
main-window-gated IPC, and live change events contain no content.

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

## Transcript rows

Rows keep the transcript itself primary: their compact metadata contains only
the timestamp and source, without per-entry word counts, durations, or repeated
mode controls. Clicking any non-interactive part of a row copies the entry's
full stored text, including text hidden behind **Show more**. A focused row does
the same with Enter or Space and briefly announces **Copied**. Nested actions
such as **Show more** and **Correct & Teach** retain their own behavior and do
not trigger a copy.

The newest 30 matching transcript rows are mounted initially. **Show older**
adds the next batch of at most 30 without changing sort order. Search and source
filters still evaluate the complete retained history before this presentation
window is applied, and exports include the complete filtered result set rather
than only the currently mounted batch.

## Clearing

`Clear History` removes everything. It is a two-step confirm that disarms after four seconds — deliberately not `window.confirm`, since a native modal steals focus from the main window.

## Export

The **Export** menu acts on **exactly what is currently shown** — filters and search included — so "copy today's CLI notes" is a search plus two clicks.

| Format | Shape |
|--------|-------|
| Markdown | `# Murmur transcript history`, an export stamp, then one `##` section per entry with source and duration |
| Plain text | `[timestamp · source · duration]` header line per entry, blank-line separated |
| JSON | `{ schema: "murmur.history.v2", exportedAt, count, entries: [...] }`; v2 entries include raw recognition and recording metadata |

Each format can go to the clipboard or to a file. The two menu groups repeat the same format names, so each is a labelled `role="group"` and every item carries the verb in its accessible name ("Copy 5 shown as Markdown", "Save 5 shown as Markdown"). Entries are ordered exactly as displayed (newest first), and timestamps are rendered as local `YYYY-MM-DD HH:MM:SS` so an export reads the same on every machine.

**`teachingContext` is never exported.** The bundle id and project root captured at recording start are local scope metadata for Correct-and-Teach, not part of a transcript the user is sharing. All three formats are asserted against this in tests.

Markdown and plain-text exports remain delivered-text views. Raw recognition,
model/profile correlation, and stage outcomes leave history only through the
user's explicit JSON export. No export contains audio. Telemetry continues to
receive only content-free counts, stable codes, durations, and stage outcomes;
neither raw nor delivered dictated text is logged or shipped.

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

## Reformatting

Entries that retain v2 raw recognition can be explicitly reformatted with any
enabled compatible Mode. This runs text through the local deterministic Mode
pipeline; it is never described as audio retranscription and does not access
or recreate audio. The original entry remains immutable. A new entry stores
the source entry ID, selected Mode ID, creation time, and content-free stage
outcomes as provenance.

Reformatting never injects text, increments dictation statistics, or invokes
Correct-and-Teach learning. Raw and derived content stay behind the existing
history-retention boundary and enter exports only through the user's explicit
history export action.

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
- `app/src/components/history/HistoryPanel.test.tsx` — search/filter interaction, bounded render batches, export scope, dialog cancellation, and the two-step clear.
- `app/src/lib/hooks/useFileTranscription.test.tsx` — newly enqueued files publish synchronously and enter the sequential drain exactly once.
- `app/src-tauri/src/commands/export.rs` — the full validation matrix plus atomic overwrite and temp-file placement.
- `app/src-tauri/src/commands/settings_store.rs` — history/stats/settings shape
  bounds, atomic overwrite, quarantine, and idempotent clear.
- `app/src/lib/durableUserData.test.ts` — disk-authoritative hydration,
  one-time localStorage migration, isolated failure, write-through, and clear.
- `app/src/lib/hooks/useHistoryManagement.test.tsx` — disabled retention keeps
  existing entries while rejecting new transcript content.
