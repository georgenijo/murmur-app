# Per-App Dictation Context

Murmur resolves one immutable `DictationContextSnapshot` for every live recording. The snapshot is created when recording starts from the frontmost application's bundle identifier and the current backend configuration. Final transcription, transformations, file output, clipboard output, and auto-paste all use that same snapshot.

## Resolution and precedence

`dictation_context::resolve` is the only profile resolver. It applies values in this order:

1. Global dictation settings
2. The matching profile's explicit writing style
3. Matching per-app fine-tuning overrides
4. One-session overrides

One-session overrides are an explicit, typed resolver input but no trigger supplies them yet. This keeps the precedence contract ready for future commands without adding a second app-detection or settings path.

Profiles select an optional `writingStyle` and can fine-tune `autoPaste`, transcript cleanup, Smart Formatting, CLI formatting, and local IDE project context. A style and IDE-context opt-in are always explicit user choices; Murmur never infers either one from an app name or bundle identifier.

Settings > Delivery > App Overrides can add a profile from currently running
regular macOS apps or through advanced manual bundle-ID entry. The picker returns
only display name and bundle ID, excludes Murmur and helper/accessory processes,
deduplicates and sorts entries, and caps the list at 64. It is fetched on demand,
kept only in React memory, and never logged or persisted unless the user chooses
an app. Advanced manual bundle-ID entry remains available when an application
is not present in that live macOS list.

Each boolean override is an explicit **Use global setting / Always / Never**
choice mapped to the existing `null / true / false` storage contract. Existing
profiles and every stored field retain their values across the Settings redesign.

| Writing style | Local deterministic behavior |
|---|---|
| Inherit | Preserves the current global/profile behavior byte-for-byte. |
| Conversational | Removes filler and repeated words, tidies capitalization, keeps wording, and disables automatic command formatting. |
| Polished prose | Applies conversational cleanup, deterministic vocabulary correction, and explicitly cued prose structure. |
| Code / technical | Preserves technical surface text, activates enabled developer vocabulary, enables deterministic vocabulary correction, and enables reviewed command formatting. |
| Verbatim | Bypasses cleanup, spoken commands, correction, prose formatting, and command formatting. |
| Notes | Removes filler without forcing sentence capitalization, applies deterministic correction, and formats explicitly cued lists, paragraphs, lines, and symbols. |

These policies use only Murmur's existing reviewed local formatting APIs. They do not call a cloud service or perform open-ended rewriting. The per-profile Clean, Prose, and Commands controls apply after the preset, so users can visibly fine-tune a category. One-session overrides remain highest precedence.

Existing stored profile objects remain valid; missing, `null`, or malformed styles and overrides mean Inherit. CLI defaults to conservative automatic detection; Commands On enables command-shaped unknown tools for that profile, while Off disables implicit detection but preserves the explicit spoken `command` trigger. Verbatim bypasses the command stage entirely unless a later explicit profile/session CLI override fine-tunes it. The settings UI prevents duplicates, but persisted or programmatic configuration can contain them. To preserve legacy behavior exactly, each field uses the first matching profile that provides that field; a `null` value falls through to the next duplicate.

## Snapshot contents and lifetime

The snapshot contains only typed values used by the live pipeline:

- Active app bundle identifier, the first matched profile identity, and the
  private native process-instance evidence needed to verify the delivery target
- Effective transcription, transformation, and delivery settings
- Vocabulary source plus a monotonic configuration version
- The resolved prompt and immutable correction matcher
- Enabled command groups
- Stable resolved writing-style enum
- Context-capture permissions
- An optional ready, memory-only IDE index for the exact matching opted-in profile

`AppState` stores the snapshot with its `recording_id`. Stop and processing paths can retrieve only the matching generation. Cleanup also checks the generation, so a stale pipeline cannot read or clear a newer recording's snapshot. Focus and settings changes after recording starts affect only later recordings' profile resolution. Delivery separately compares the current native identity with a frozen identity for the same `recording_id`; it never re-resolves the profile or adopts a new target.

### Frontmost-app sampling

At the accepted `Idle -> Starting` transition, live dictation takes exactly one native macOS `NSWorkspace` sample. A complete bundle, PID, and launch-instance token from that retained `NSRunningApplication` object become both the immutable profile identity and the private delivery target. The optional focused-window token and content-free activation/Space counters come from the same boundary. This work runs under recording ownership and is included in request-to-first-PCM timing.

Live dictation does not retry this start sample or adopt a later application. An unavailable, partial, changing, or self-owned sample resolves an unmatched global-only context; app-specific IDE/context reads remain disabled and auto-paste later fails closed. Voice Query retains its own exact native one-sample identity boundary, but no System Events fallback can choose a live recording's profile or paste destination. Literal AppleScript `missing value` is normalized to unavailable in the regression seam rather than treated as a bundle identifier.

Delivery uses a second sample taken at the accepted stop transition, under the
same ownership lock that commits Processing. That stop anchor authorizes
verification only when it is a complete identity; a self-owned, incomplete,
mismatched, or absent stop sample falls back to the recording-start anchor, so a
degraded stop sample can never make delivery more permissive. Profile
resolution is unaffected: per-app context stays bound to the immutable
recording-start identity.

At delivery, a bounded native verifier requires the same application, PID, and
process-instance evidence as the anchored target. A process relaunch therefore
cannot pass merely by reusing a bundle identifier or PID. Different windows in that exact process
instance remain eligible; window relation, activation changes, and Space
changes are recorded only as content-free diagnostic facts. A different app or
process, relaunch, unavailable lookup, incomplete anchored identity, self target,
or unprovable identity remains clipboard-only. Contradictory native PID or
launch-instance evidence is terminal even when an unbundled process withholds
its bundle identity; it is reported content-free as
`partial_identity_mismatch` and cannot be erased by a later sample of the
expected target. A stale recording owner performs no delivery write at all.

## Privacy boundary

Dictation context capture is deny-by-default. A profile may explicitly grant only its bounded local project index. The dictation snapshot never grants reading:

- Selected text
- Nearby or surrounding screen text
- Clipboard contents as general transcription context. A matched snippet may read it only when that exact Voice Command carries explicit clipboard permission.

This policy is separate from delivery. Murmur remains clipboard-first: the completed transcript is still written to the clipboard, and existing auto-paste behavior is unchanged. IDE project context does not change those denials: it reads only user-selected roots through the bounded local index described in [Local IDE Symbols and `@file` Context](ide-context.md). Unmatched profiles and app names that merely look like IDEs remain no-ops.

Voice Query has a separate, visible opt-in context level documented in [Voice Query](voice-query.md). A profile's `queryContextExcluded` flag is deny-only: for a matching bundle ID it forces that pass to attach no app name, window title, or selected text, regardless of the global/preset level. It does not weaken the dictation denials above and cannot enable query context.

Writing styles do not change the ASR model, language, file-saving behavior, clipboard write, auto-paste timing, or destination. The explicit Code / technical style activates the globally enabled developer-vocabulary pool for that matching app; Local IDE project context is the other activation signal. Other styles and unmatched apps never receive scanned or built-in developer terms. Preferred spellings retain their own global/app/project scopes. Style telemetry contains only the stable resolved enum plus the existing matched-profile boolean; bundle identifiers, labels, setting values, and transcript content are never logged.

Vocabulary aliases use this same immutable context. Global aliases always apply. Typed app aliases require the matching snapshot bundle identifier; typed project aliases additionally require the matching profile's enabled local project context and configured root. Settings currently creates global aliases first. No alias path re-detects the frontmost app.

Frontmost-app detection telemetry likewise contains only a numeric outcome code,
retry count, numeric source code, and elapsed milliseconds. Delivery verification
uses the exact all-build `pipeline.delivery_target_verified` schema documented in
[Text Injection](text-injection.md#delivery-target-diagnostics-and-privacy): a
bounded anchor/outcome/source/window-relation vocabulary, equality and
transition booleans, retry/timing numbers, and the positive `recording_id`. Neither path
contains the detected bundle identifier, PID, process-instance token,
application name, window title, profile label, project roots, raw errors,
transcript or clipboard content, or other user content.

## Extension points

Future app-specific model, language, vocabulary, command, formatting, or context-policy fields should be added to the profile schema and folded into `DictationContextSnapshot` by the single resolver. Pipeline stages should consume the snapshot rather than re-reading `DictationState` or detecting the frontmost app again.

Voice Commands already follow this rule: applicable global/app records are selected with the sampled bundle identifier and stored in the snapshot. An app-scoped phrase overrides its global counterpart only for that recording context.
