# Correct and Teach

Correct and Teach lets the user explicitly turn one correction to the newest local history entry into a reusable Smart Correction rule. It does not watch typing outside Murmur and does not retrain Whisper, Parakeet, Core ML, or any other speech model.

## Review and consent

The newest history card exposes **Correct and Teach**. Editing the transcript has no persistence side effect until the review step. Murmur computes one bounded proposal and shows the exact heard phrase, written replacement, scope, current before/after example, and affected occurrence count.

The review has separate actions:

- **Save correction only** updates the local history entry without storing knowledge.
- **Remember correction** is the only action that creates a learned rule.
- Cancel, Escape, backdrop dismissal, and Back never store a rule.

History edits cannot change text that was already copied, pasted, or saved, and they do not recalculate usage statistics.

## Bounded rule extraction

Proposal generation is deterministic and local. Inputs are capped at 8,192 Unicode characters and 512 lexical tokens. A candidate must contain exactly one replacement hunk, with non-empty source and replacement spans of at most eight tokens and 256 characters each.

Casing, names, code identifiers, and short phrase replacements are supported. Insert-only, delete-only, punctuation-only, whitespace-only, oversized, reordered, and multiple-disjoint edits fail closed. An unsafe edit may still be saved to history, but Murmur does not guess a reusable rule.

Built-in and configured Voice Command phrases are rejected as learned corrections. Voice Commands remain earlier and unchanged in the pipeline.

## Scope and precedence

Global scope is always available. App scope is offered only when the frontmost bundle identifier was captured in the immutable recording-start context. Project scope additionally requires the exact matching profile to have local project context enabled with one configured root. Multiple roots are ambiguous and do not produce an invented active-project scope.

Enabled replacement records are compiled outside the transcript hot path. A recording captures one immutable matcher generation, so knowledge edits affect the next recording and cannot change an in-flight result. Exact precedence is:

```text
Voice Commands
→ explicit vocabulary aliases
→ replacement knowledge (project → app → global; then provenance/update/ID)
→ derived/exact vocabulary
→ fuzzy vocabulary
→ Smart Formatting
→ IDE context
→ CLI formatting
```

Within replacement knowledge, the repository's provenance order remains manual → import → learned correction → code scan. Exact same-scope conflicts must be reviewed in Settings → Knowledge rather than silently overwritten.

## Knowledge management and privacy

Confirmed rules use the Rust-owned personal knowledge SQLite store with `learned_correction` provenance. Settings → Knowledge provides inspect, edit, enable/disable, export, and confirmed deletion. Store changes rebuild the next correction matcher generation.

Proposal examples remain in the local UI/history data already stored on this Mac. The knowledge record persists only the source phrase, replacement, scope, enabled state, timestamps, revision, and provenance—not the full transcript or examples. Telemetry contains only character counts, outcome booleans, scope kind, and provenance; it never logs transcript text, rule values, bundle identifiers, or project paths. No network service or account is involved.

## Source and tests

- Proposal bounds and consent state: `app/src-tauri/src/correct_and_teach.rs`
- Persistence commands: `app/src-tauri/src/commands/correct_and_teach.rs`
- Matcher precedence: `app/src-tauri/src/correction.rs` and `app/src-tauri/src/vocabulary_alias.rs`
- History review UI: `app/src/components/history/CorrectAndTeachDialog.tsx`
- Knowledge management: `app/src/components/settings/KnowledgeManager.tsx`
