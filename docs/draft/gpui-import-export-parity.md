# GPUI import/export and bulk-action feedback (#1061)

The GPUI production host shows the shipped Tauri wording for file exchange and
bulk changes. Failure explanations come from the services' own static literals
through the allow list in `app/gpui/src/host/services/explain.rs`; a failure
that is not on it stays the generic unavailable state, so a path, an id or
record content never reaches the screen. Tests keep each allow-listed line equal
to a literal in the owning service.

## Intentional differences from shipped (699514c6)

- Knowledge import and export notices keep their counts ("Imported N; skipped M
  duplicate(s).", "Exported N records."). Shipped overwrote them with a final
  "Knowledge imported/exported." line.
- Knowledge import keeps the owner's stale-revision check ("Knowledge changed
  after import review"). Shipped had no check, so a changed store was replaced
  silently.
- A bulk Knowledge failure names the record title and the store's reason when
  the reason is on the allow list; otherwise it reads "Murmur could not change
  this record." The notice is bounded to 512 bytes with an "...and N more
  failed." tail.
- The Modes conflict list is capped at 50 lines plus "...and N more conflicts."
- A History export that cannot be saved shows the shipped line without the raw
  error text.
- Theme failures read "Failed to import theme: <reason>" and "Failed to export
  theme: <reason>" without the "Error: " prefix that JavaScript adds when it
  stringifies an error object.
- A theme exported from the library is offered as "murmur-theme.json"; shipped
  offered "<id>.murmur-theme.json".
- Open VSX / VS Code name humanizing is not ported; the file's own name is used.
