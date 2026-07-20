# Personal Knowledge Store

Murmur keeps reusable replacement rules, vocabulary terms, and snippets in a local SQLite database. The store remains an issue-independent repository boundary. Settings manages every record, while Correct and Teach creates explicitly confirmed learned replacement rules through a narrow command and Smart Correction consumes enabled replacement records through an immutable matcher snapshot.

No knowledge record is uploaded, added to telemetry, or mirrored into `localStorage`. The database lives under the app data directory at `knowledge/knowledge.sqlite3`; its `backups/` and `quarantine/` directories are contained beneath the same root.

## Record model

Every record has a stable ID, enabled flag, revision, provenance, timestamps, one payload, and one visibility scope:

- `replacement_rule`: heard `source` plus written `replacement`
- `vocabulary_term`: canonical `written` form plus bounded spoken `aliases`
- `snippet`: spoken `trigger` plus text `body`
- `global`, `app { bundleId }`, or `project { bundleId, root }` scope
- `manual`, `import`, `learned_correction`, or `code_scan` provenance

Settings creates and edits manual records. Imported records retain their content and scope but use `import` provenance. The repository resolver is deterministic: exact normalized trigger, then project over app over global, manual over import over learned correction over code scan, newest update, then stable ID. Correct and Teach uses that order inside Smart Correction; Voice Commands remain a separate, earlier stage.

## Schema and migrations

SQLite `PRAGMA user_version` is the schema version. Version 1 creates metadata and normalized records. Version 2 adds lookup indexes and the FTS5 search table. Each ordered migration is transactional. Before migrating an older schema, Murmur creates and integrity-checks a SQLite backup, retaining the three newest backups.

Connections enable foreign keys, WAL, `synchronous=FULL`, `secure_delete`, and a bounded busy timeout. Writes use optimistic record revisions, and destructive delete-all requires the current store revision.

## Recovery

Startup runs `quick_check`. If the database is corrupt, Murmur quarantines it inside the knowledge root and restores the newest valid local backup. If no valid backup exists, Murmur creates a new empty store. The Settings banner distinguishes ready, recovered, reinitialized, and unavailable states; unavailable stores can be retried without restarting.

Recovery messages and logs contain only status, schema version, and counts—not record text, chosen import/export paths, project roots, or bundle IDs.

## Settings management

Settings → Knowledge provides:

- server-bounded search and filters for type, enabled state, and scope
- pages of 50 records, with at most 100 returned per repository request
- create, inspect, edit, enable/disable, and individually confirmed delete
- visible scope, provenance, and update time per record
- atomic JSON export and inspected import preview
- typed `DELETE` confirmation for delete-all

Import files are limited to 8 MiB and 10,000 records. Validation happens before writes. Semantic duplicates are skipped; same-ID/different-content conflicts reject the entire import; existing records are never overwritten. Export writes a temporary sibling before atomic rename. Delete-all removes records, SQLite free pages, recovery backups, and quarantined databases, but never touches an export outside the store.

## Test boundaries

Rust tests use temporary, issue-specific directories for persistence, scope precedence, migration backup, corrupt-database recovery, import/export, delete-all, and privacy-safe unavailable errors. Native destructive smoke must use a unique test bundle identifier so its database cannot overlap a normal Murmur installation.
