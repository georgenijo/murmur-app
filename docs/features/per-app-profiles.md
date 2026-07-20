# Per-App Dictation Context

Murmur resolves one immutable `DictationContextSnapshot` for every live recording. The snapshot is created when recording starts from the frontmost application's bundle identifier and the current backend configuration. Incremental transcription, batch fallback, transformations, file output, clipboard output, and auto-paste all use that same snapshot.

## Resolution and precedence

`dictation_context::resolve` is the only profile resolver. It applies values in this order:

1. Global dictation settings
2. The matching profile's explicit writing style
3. Matching per-app fine-tuning overrides
4. One-session overrides

One-session overrides are an explicit, typed resolver input but no trigger supplies them yet. This keeps the precedence contract ready for future commands without adding a second app-detection or settings path.

Profiles select an optional `writingStyle` and can fine-tune `autoPaste`, transcript cleanup, Smart Formatting, CLI formatting, and local IDE project context. A style and IDE-context opt-in are always explicit user choices; Murmur never infers either one from an app name or bundle identifier.

| Writing style | Local deterministic behavior |
|---|---|
| Inherit | Preserves the current global/profile behavior byte-for-byte. |
| Conversational | Removes filler and repeated words, tidies capitalization, keeps wording, and disables automatic command formatting. |
| Polished prose | Applies conversational cleanup, deterministic vocabulary correction, and explicitly cued prose structure. |
| Code / technical | Preserves technical surface text, enables deterministic vocabulary correction, and enables reviewed command formatting. |
| Verbatim | Bypasses cleanup, spoken commands, correction, prose formatting, and command formatting. |
| Notes | Removes filler without forcing sentence capitalization, applies deterministic correction, and formats explicitly cued lists, paragraphs, lines, and symbols. |

These policies use only Murmur's existing reviewed local formatting APIs. They do not call a cloud service or perform open-ended rewriting. The per-profile Clean, Prose, and Commands controls apply after the preset, so users can visibly fine-tune a category. One-session overrides remain highest precedence.

Existing stored profile objects remain valid; missing, `null`, or malformed styles and overrides mean Inherit. CLI defaults to conservative automatic detection; Commands On enables command-shaped unknown tools for that profile, while Off disables implicit detection but preserves the explicit spoken `command` trigger. Verbatim bypasses the command stage entirely unless a later explicit profile/session CLI override fine-tunes it. The settings UI prevents duplicates, but persisted or programmatic configuration can contain them. To preserve legacy behavior exactly, each field uses the first matching profile that provides that field; a `null` value falls through to the next duplicate.

## Snapshot contents and lifetime

The snapshot contains only typed values used by the live pipeline:

- Active app bundle identifier and the first matched profile identity
- Effective transcription, transformation, and delivery settings
- Vocabulary source plus a monotonic configuration version
- The resolved prompt and immutable correction matcher
- Enabled command groups
- Stable resolved writing-style enum
- Context-capture permissions
- An optional ready, memory-only IDE index for the exact matching opted-in profile

`AppState` stores the snapshot with its `recording_id`. Stop and processing paths can retrieve only the matching generation. Cleanup also checks the generation, so a stale pipeline cannot read or clear a newer recording's snapshot. Focus changes and settings changes after recording starts affect only later recordings.

### Frontmost-app sampling

At recording start, Murmur queries the native macOS `NSWorkspace` frontmost application first. An unavailable or empty native result is retried twice at 10 ms intervals, then the timeout-bounded System Events query is attempted once as a compatibility fallback. The nominal worst-case query budget is 270 ms: 20 ms of retry delay plus the fallback's 250 ms timeout.

The first successful sample wins and is resolved into the immutable snapshot exactly once. If focus changes after a successful native sample, the original app remains active for that recording. If an early native sample is unavailable while the user switches apps, the first later successful native or fallback sample becomes active for the recording. If every query fails, Murmur resolves an unmatched global-only context; app-specific IDE/context reads remain disabled.

## Privacy boundary

Context capture is deny-by-default. A profile may explicitly grant only its bounded local project index. The snapshot never grants reading:

- Selected text
- Nearby or surrounding screen text
- Clipboard contents as transcription context

This policy is separate from delivery. Murmur remains clipboard-first: the completed transcript is still written to the clipboard, and existing auto-paste behavior is unchanged. IDE project context does not change those denials: it reads only user-selected roots through the bounded local index described in [Local IDE Symbols and `@file` Context](ide-context.md). Unmatched profiles and app names that merely look like IDEs remain no-ops.

Writing styles also do not change the ASR model, language, vocabulary inputs, prompt, file-saving behavior, clipboard write, auto-paste timing, or destination. Style telemetry contains only the stable resolved enum plus the existing matched-profile boolean; bundle identifiers, labels, setting values, and transcript content are never logged.

Frontmost-app detection telemetry likewise contains only a numeric outcome code, retry count, numeric source code, and elapsed milliseconds. It never contains the detected bundle identifier, application name, profile label, project roots, or user content.

## Extension points

Future app-specific model, language, vocabulary, command, formatting, or context-policy fields should be added to the profile schema and folded into `DictationContextSnapshot` by the single resolver. Pipeline stages should consume the snapshot rather than re-reading `DictationState` or detecting the frontmost app again.
