# Local IDE Symbols and `@file` Context

Murmur can use an explicitly selected local project root to correct project symbols and canonicalize spoken file references for one configured app profile. The feature is off by default and fully local. Murmur never classifies an app as an IDE from its name or bundle identifier.

## Opt-in and profile scope

In Settings → Application Profiles, enable **Local IDE symbols & @file** on a specific profile and add up to four project roots with the folder picker. The frontmost application's bundle identifier must exactly match that configured profile at recording start. Unsupported, unmatched, disabled, or rootless profiles do not run this stage and preserve the preceding text byte-for-byte.

The selected root paths are visible locally in Settings and are persisted as profile configuration. As with every persisted setting, they remain present in the transparent `dictation-settings` JSON if it is inspected or backed up. Filenames, symbols, source snippets, and scan results are never persisted.

## Index boundary

Each configured root is canonicalized once per scan. Murmur then performs a bounded, read-only walk under that canonical root:

- At most 4 roots (4,096 bytes per configured root), 1,000 files, 32 MiB total, 512 KiB per file, 10,000 candidate symbols with 500 retained, 20,000 visited directory entries, 512 bytes per root-relative path, 10 seconds per scan, and 16 KiB per transformed transcript
- Only allowlisted source files and `package.json` manifests
- No symlinks, sockets, devices, hidden files/directories, version-control directories, dependency/vendor directories, build output, or cache directories
- No path that cannot be represented as a clean descendant of its canonical root

The source text exists only while its file is being parsed. Files are opened without following symlinks and revalidated against the canonical root before parsing. The finished index contains only the bounded correction matcher and relative-file aliases in process memory. A ready generation expires after 60 seconds. Root/profile changes, manual refresh, manual clear, cancellation, or expiry invalidate the prior generation before a new scan can be adopted. A cancelled, timed-out, or cap-truncated scan publishes no partial index.

Indexing telemetry contains only generation IDs, counts, capped sizes, elapsed time, and outcomes. It never includes roots, basenames, relative or absolute paths, symbols, or source content.

## Deterministic formatting

Project symbols reuse the exact, ambiguity-safe local correction matcher. A spoken form is eligible only when it maps to one unique written symbol in the active generation.

File canonicalization requires the explicit word `mention`:

```text
mention recording dot rs → @src/recording.rs
mention src slash recording dot rs → @src/recording.rs
```

Output is always the canonical root-relative text beginning with `@`; absolute paths are never emitted. If two roots contain the same basename or the same relative candidate, that reference remains unchanged. A longer relative qualification is used only when it resolves uniquely. Existing canonical `@file` text is idempotent, and a filename without the explicit trigger is not rewritten.

## Pipeline and delivery

The live pipeline order is:

```text
cleanup → voice commands → Smart Correction → Smart Formatting → IDE context → CLI formatting
```

Explicit IDE opt-in disables Smart Formatting for that recording so prose structure cannot interfere with code-oriented text. Generic correction resources still run first, and the reviewed CLI stage remains final and authoritative. Imported-file transcription never runs the IDE stage.

The immutable recording-start snapshot may capture only the ready memory index for the exact matching opted-in profile. It does not grant selected-text, screen-text, or clipboard reads. Delivery remains unchanged and final-only: one final string reaches optional file output, clipboard, paste, history, and stats.

## Clear versus remove

**Clear index** immediately discards the current memory generation while keeping the selected roots and opt-in. **Remove** beside a root changes persisted configuration and removes that root from future scans. **Refresh index** invalidates the old generation first and builds a new one from the still-configured roots.
