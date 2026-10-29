# Explicit Spoken Vocabulary Aliases

Custom Vocabulary stores structured entries with one canonical written form and zero or more exact spoken aliases. For example, both `Tori` and `Tory` can map to `Tauri`. Canonical terms continue to bias Whisper; aliases are post-model rules, so they work identically with Whisper, sherpa-onnx, and FluidAudio Core ML.

## Matching and precedence

Aliases are deterministic, local, Unicode-aware, case-insensitive exact phrase matches. They preserve surrounding punctuation and use longest-match-first ordering. They never enable fuzzy correction for an ordinary word.

The live transformation order remains:

```text
cleanup -> Voice Commands -> explicit aliases -> built-in contextual variants -> derived/exact vocabulary -> fuzzy correction -> Smart Formatting -> Spoken Structure -> spoken numbers -> IDE context -> CLI formatting
```

Explicit alias output is literal to spoken-number rendering. Aliases whose spoken form contains a digit are matched again at the end of the spoken-numbers stage, but only against digits that stage just rendered and never against text an alias already wrote. A variant such as `GPT 5` therefore still matches when the recognizer said `GPT five`, and no rule applies twice.

Voice Commands intentionally trigger insertions/actions and remain separate. Settings reject an alias that collides with a built-in or custom Voice Command phrase. Already-canonical terms are protected. Explicit user aliases outrank future learned rules, built-in vocabulary, derived exact forms, and generic fuzzy matching. IDE symbols remain context-specific after generic correction, and CLI formatting remains final and authoritative. Thus `npm run Tori dev` becomes `npm run Tauri dev` in correction and then `npm run tauri dev` in the CLI stage.

Ambiguous aliases, canonical collisions, and direct or indirect cycles are rejected rather than resolved by insertion order. Disabled entries and entries outside the immutable recording-start app/project scope do not participate.

## Context-cued variants

Some recognizer variants are real words in other contexts. `Tori` is a name, and `Maine` is a state. A context-cued variant is rewritten only when the immutable recording-start context or nearby words establish the intended meaning. Otherwise the recognized text is left alone. Matching stays deterministic, local, and bounded.

A requirement can enable a variant in three ways:

- **Register.** The frozen Mode register (Everyday, Messages, Email, Notes, Technical, Terminal, or Verbatim) is in its allowed list.
- **Project context.** The recording has Local IDE project context.
- **Cue words.** A cue word occurs within a small window of words on either side, inside the same clause.

**Blocking words** within the window refuse the rewrite. A cue and a counter-cue together are ambiguous, so the phrase stays as recognized. Two built-in variants for the same phrase that both apply are also ambiguous and fail closed.

Cue and blocker windows stop at `.`, `!`, `?`, `;`, and line breaks. A `.` counts only when followed by whitespace, so `app.json` stays one clause. `Ship the app. Tori is coming over for dinner.` keeps `Tori`. Variants match Unicode-aware whole words, so `Torié` and mixed-script names such as `Toriд` are never rewritten.

Murmur ships these built-in variants. They apply whenever Smart Correction runs and are separate from the Developer Terms pool:

| Recognized | Written | Applies when |
|---|---|---|
| `Maine`, `me` | `main` | adjacent `branch`, or the observed version-control shapes (`latest changes from Maine`, `check out the changes on me`). Geography words block it. |
| `Tori`, `Tory` | `Tauri` | Technical/Terminal, project context, or a framework cue within 8 words (`rust`, `cargo`, `plugin`, `app`, `build`, `tauri`, `npm`, `webview`, ...). Person cues within 4 words (`she`, `friend`, `hey`, `thanks`, ...) block it, and so do politics cues for `Tory`. |
| `Postgres SQL`, `Postgre SQL` | `PostgreSQL` | Technical/Terminal or project context |
| `npxtsc` | `npx tsc` | Technical/Terminal or project context |
| `CoreML` | `Core ML` | Technical/Terminal or project context |

```text
Everyday:  Tori is coming over for dinner.      → unchanged
Everyday:  The Tori plugin needs a rust update. → The Tauri plugin needs a rust update.
Everyday:  My friend Tori is building an app.   → unchanged (cue and counter-cue)
Technical: open the Tori config                 → open the Tauri config
Terminal:  npm run Tori dev                     → npm run tauri dev
```

Explicit user aliases and learned replacements outrank built-in variants. For example, an explicit `Tori` → `Tori` entry keeps the name in every register.

Context requirements are internal data on the built-in pack. Vocabulary entries have no user-authored `context` field. User-authored context rules are a follow-up: they need Settings support and a storage shape that an older build cannot read as an unrestricted alias.

## Scope and migration

The Settings editor creates global entries. The persisted schema also has typed app and project scope variants so existing `DictationContextSnapshot` app/profile/project context can select rules without another frontmost-app resolver. Project rules require the matching bundle identifier, an IDE-enabled matching profile, and the configured root.

Older comma/newline-separated `customVocabulary` strings migrate to enabled global entries with no aliases. Their written terms continue to feed the Whisper prompt and Smart Correction, preserving prior behavior.

## Backup and transfer

Use `Export saved spellings` in Settings → Text & Vocabulary to save every
global, app-scoped, and project-scoped entry as a versioned JSON file. `Import
saved spellings` validates the complete file locally, then shows new entries,
exact duplicates, and conflicts before applying anything. Conflicting IDs or
written terms are never overwritten; invalid, corrupted, or oversized files
are rejected without changing Settings.

## Preview and privacy

Settings includes a local in-memory preview that uses the production Rust matcher and can optionally include final CLI formatting. Preview input, output, aliases, canonical terms, bundle identifiers, and project roots are never logged. Configuration telemetry contains counts and booleans only. Alias processing does not read the clipboard or screen and does not change final-only clipboard/paste delivery.

## Local evaluation

The deterministic cases in `bench/vocabulary-aliases.json` cover Tauri/Tori/Tory command composition, casing, punctuation, false positives, idempotence, and context-cued variants. A case may set `mode`, `projectContext`, and `aliases: false`, which leaves out the explicit Tori/Tory aliases so the built-in variant is exercised on its own. Run them with:

```bash
cd app/src-tauri
cargo test transcript_transform::tests::vocabulary_alias_eval -- --exact
```
