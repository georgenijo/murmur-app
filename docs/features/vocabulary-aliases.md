# Explicit Spoken Vocabulary Aliases

Custom Vocabulary stores structured entries with one canonical written form and zero or more exact spoken aliases. For example, both `Tori` and `Tory` can map to `Tauri`. Canonical terms continue to bias Whisper; aliases are post-model rules, so they work identically with Whisper, sherpa-onnx, and FluidAudio Core ML.

## Matching and precedence

Aliases are deterministic, local, Unicode-aware, case-insensitive exact phrase matches. They preserve surrounding punctuation and use longest-match-first ordering. They never enable fuzzy correction for an ordinary word.

The live transformation order remains:

```text
cleanup -> Voice Commands -> explicit aliases -> derived/exact vocabulary -> fuzzy correction -> Smart Formatting -> IDE context -> CLI formatting
```

Voice Commands intentionally trigger insertions/actions and remain separate. Settings reject an alias that collides with a built-in or custom Voice Command phrase. Already-canonical terms are protected. Explicit user aliases outrank future learned rules, built-in vocabulary, derived exact forms, and generic fuzzy matching. IDE symbols remain context-specific after generic correction, and CLI formatting remains final and authoritative. Thus `npm run Tori dev` becomes `npm run Tauri dev` in correction and then `npm run tauri dev` in the CLI stage.

Ambiguous aliases, canonical collisions, and direct or indirect cycles are rejected rather than resolved by insertion order. Disabled entries and entries outside the immutable recording-start app/project scope do not participate.

## Scope and migration

The Settings editor creates global entries. The persisted schema also has typed app and project scope variants so existing `DictationContextSnapshot` app/profile/project context can select rules without another frontmost-app resolver. Project rules require the matching bundle identifier, an IDE-enabled matching profile, and the configured root.

Older comma/newline-separated `customVocabulary` strings migrate to enabled global entries with no aliases. Their written terms continue to feed the Whisper prompt and Smart Correction, preserving prior behavior.

## Preview and privacy

Settings includes a local in-memory preview that uses the production Rust matcher and can optionally include final CLI formatting. Preview input, output, aliases, canonical terms, bundle identifiers, and project roots are never logged. Configuration telemetry contains counts and booleans only. Alias processing does not read the clipboard or screen and does not change final-only clipboard/paste delivery.

## Local evaluation

The deterministic cases in `bench/vocabulary-aliases.json` cover Tauri/Tori/Tory command composition, casing, punctuation, false positives, and idempotence. Run them with:

```bash
cd app/src-tauri
cargo test transcript_transform::tests::vocabulary_alias_eval -- --exact
```
