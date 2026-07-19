# Smart Formatting and Same-Utterance Backtracking

Smart Formatting is an opt-in, local post-recognition stage for live dictation. It applies a small deterministic prose grammar; it does not call a model, inspect other applications, rewrite selected text, or revisit text that Murmur has already delivered.

## Enablement and context

The global **Smart Formatting** switch is off by default. Per-app writing styles can select it as part of a transparent local policy, and the profile's Prose control can independently inherit, enable, or disable it. Murmur resolves that value once at recording start, so focus and setting changes apply only to the next recording.

The stage is skipped for imported-file transcription. CLI/code/verbatim contexts can bypass it through the per-app Smart Formatting override, and any utterance activated by the authoritative CLI grammar bypasses prose rules automatically. CLI canonicalization still runs after Smart Formatting.

## Deterministic grammar

All matching is local and bounded. Text that does not complete a rule is returned unchanged.

### Enumerations

Two to ten consecutive ordinal markers (`first` through `tenth`) become a numbered list. The utterance must start with `first` or use an explicit list-like prefix ending in words such as `are`, `include`, `steps`, `tasks`, or `priorities`. Each item is limited to 24 words.

```text
The three priorities are first reliability, second latency, third accuracy

The three priorities are:
1. Reliability
2. Latency
3. Accuracy
```

Missing, repeated, or out-of-order ordinal markers fail closed.

### Same-utterance backtracking

The explicit restatement cues `actually, make that`, `I mean`, `or rather`, and `rather` can replace only the final abandoned term in the current utterance. `I mean` and `rather` forms require a preceding correction separator. Replacement text is limited to four words and 64 characters; discourse forms such as `what I mean is` remain unchanged.

```text
Ship it Friday—actually, make that Monday

Ship it Monday.
```

Backtracking never reaches an earlier utterance, history entry, clipboard delivery, or pasted text.

### Explicit structured tokens

- Email formatting requires an earlier `email` cue plus bounded `at` / `dot` tokens. Local-part `dot`, `underscore`, `dash`, `hyphen`, and `plus` tokens are supported within the 12-token address bound.
- URL formatting requires a leading `URL` or `web address` cue, an explicit `dot`, and no more than 20 tokens. `http(s) colon slash slash`, path slashes, dashes, and colons are deterministic.
- Quotes and parentheses require a matched `open ...` / `close ...` pair containing at most 240 characters.
- Paragraph, line, punctuation, dash, and symbol tokens are bounded whole phrases such as `new paragraph`, `question mark`, `em dash`, `plus sign`, or `equals sign`.
- The stage fails closed for utterances larger than 16 KiB, bounding all grammar scans before allocating token lists or transformed output.

No email or URL structure is inferred from ordinary prose without its cue. Unpaired quote/parenthesis markers and over-limit structures remain literal.

## Pipeline, delivery, and privacy

The live order is:

```text
cleanup → voice commands → Smart Correction → Smart Formatting → CLI formatting → final text
```

Only the final transformed text reaches optional file output, the clipboard, auto-paste, history, and stats. Clipboard and paste therefore still happen once, after all stages complete. Imported audio keeps the existing raw-ASR behavior with every transformation stage skipped.

For tests and pipeline diagnostics, the pipeline result holds original and final strings only in memory for the lifetime of that result. Stage telemetry contains only the stage name, duration, changed flag, outcome, and failure policy. Transcript contents, cues, replacements, addresses, URLs, commands, and before/after values never enter logs. There is intentionally no persisted recovery, undo, or original-versus-transformed history UI.

## Source and tests

- Grammar: `app/src-tauri/src/smart_formatting.rs`
- Stage order and context bypass: `app/src-tauri/src/transcript_transform.rs`
- Immutable profile resolution: `app/src-tauri/src/dictation_context.rs`
- Setting and migration: `app/src/lib/settings.ts`

The Rust fixtures cover golden outputs, conservative false positives, Unicode, every rule's bounds, idempotence, CLI/profile bypass, imported-file behavior, stage ordering, and in-memory-only original/final comparison.
