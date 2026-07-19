# Per-App Dictation Context

Murmur resolves one immutable `DictationContextSnapshot` for every live recording. The snapshot is created when recording starts from the frontmost application's bundle identifier and the current backend configuration. Incremental transcription, batch fallback, transformations, file output, clipboard output, and auto-paste all use that same snapshot.

## Resolution and precedence

`dictation_context::resolve` is the only profile resolver. It applies values in this order:

1. Global dictation settings
2. Matching per-app profile overrides
3. One-session overrides

One-session overrides are an explicit, typed resolver input but no trigger supplies them yet. This keeps the precedence contract ready for future commands without adding a second app-detection or settings path.

Profiles currently override `autoPaste`, transcript cleanup, Smart Formatting, and CLI formatting. Existing stored profile objects remain valid; missing or `null` overrides still mean "use the global/automatic value." Smart Formatting is a separate opt-in prose stage, so CLI/code/verbatim profiles can explicitly leave it off without changing CLI canonicalization. CLI defaults to conservative automatic detection; On enables command-shaped unknown tools for that profile, while Off disables implicit detection but preserves the explicit spoken `command` trigger. The settings UI prevents duplicates, but persisted or programmatic configuration can contain them. To preserve legacy behavior exactly, each field uses the first matching profile that provides that field; a `null` override falls through to the next duplicate.

## Snapshot contents and lifetime

The snapshot contains only typed values used by the live pipeline:

- Active app bundle identifier and the first matched profile identity
- Effective transcription, transformation, and delivery settings
- Vocabulary source plus a monotonic configuration version
- The resolved prompt and immutable correction matcher
- Enabled command groups
- Context-capture permissions

`AppState` stores the snapshot with its `recording_id`. Stop and processing paths can retrieve only the matching generation. Cleanup also checks the generation, so a stale pipeline cannot read or clear a newer recording's snapshot. Focus changes and settings changes after recording starts affect only later recordings.

## Privacy boundary

Context capture is deny-by-default. The current snapshot never grants reading:

- Selected text
- Nearby or surrounding screen text
- Clipboard contents as transcription context

This policy is separate from delivery. Murmur remains clipboard-first: the completed transcript is still written to the clipboard, and existing auto-paste behavior is unchanged. A future context feature must add an explicit user setting/profile override and grant the corresponding capture permission in the resolver before any new read path is introduced.

## Extension points

Future app-specific model, language, vocabulary, command, formatting, or context-policy fields should be added to the profile schema and folded into `DictationContextSnapshot` by the single resolver. Pipeline stages should consume the snapshot rather than re-reading `DictationState` or detecting the frontmost app again.
