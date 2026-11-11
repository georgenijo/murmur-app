# Spoken Structure

Spoken Structure is the single deterministic owner for explicitly dictated
punctuation and layout. It renders whole-phrase cues such as `period`,
`question mark`, `new paragraph`, `open quote`, `slash`, and `backslash`
without calling a model or reading external context.

## Policy

One immutable policy is resolved at recording start:

- **Off** — neither Spoken formatting nor Smart Formatting is enabled.
- **Basic** — Spoken formatting is enabled; the built-in spoken commands below
  (punctuation, breaks, parentheses, and `scratch that`) are available.
- **Extended** — Smart Formatting is enabled; the full punctuation, paired
  delimiter, dash, slash, and symbol grammar is available.
- **Union** — both are enabled; the union is scanned once. Legacy
  unpaired-tolerant parenthesis behavior is retained while quotes remain
  paired and bounded.

**Spoken formatting** (Settings → Text & vocabulary → Advanced) is the switch
behind Basic. It is stored as `voiceCommandsEnabled` for compatibility with
existing settings files.

An IDE-context recording downgrades extended prose behavior but retains Basic
when Spoken formatting remains enabled. Code-technical and Verbatim styles turn
both sources off. Transform instructions and imported-file transcription
always skip the stage.

## Built-in spoken commands

The Basic policy recognizes these whole phrases:

| Say | Result |
| --- | --- |
| `period` | `.` |
| `comma` | `,` |
| `question mark` | `?` |
| `open paren` / `close paren` | `(` / `)` |
| `new line` | a line break |
| `new paragraph` | a blank line |
| `scratch that` | deletes the sentence you just said; it stops at a line break |

Matching is case-insensitive and word-bounded, so `periodic` is left alone.
There are no user-defined commands: Murmur once supported custom phrase →
replacement pairs and snippets, and they were retired. Vocabulary aliases cannot
reuse a built-in phrase; saving an alias that does is rejected with "is a
built-in spoken formatting phrase". The same list is shared by the engine
(`BASIC_COMMAND_PHRASES`) and the frontend alias validator.

## Arbitration and ownership

Explicit terminal punctuation wins over one adjacent terminal mark emitted by
the recognizer. Thus `ready? question mark.` becomes `ready?`, while two
explicit spoken marks remain intentional: `question mark exclamation mark`
becomes `?!`. Non-terminal punctuation absorbs only the identical adjacent
mark. Arbitration never crosses a newline and consumes at most one ASR mark.

`scratch that` runs in the same left-to-right pass as punctuation and line
breaks, so newly created boundaries are immediately available.

Higher-level email, URL, enumeration, and backtracking grammars remain in Smart
Formatting and run first. Failed explicit URL grammar and inline `slash
command` cues remain untouched for their authoritative URL/CLI parsers.
Spoken numbers run afterward so `one slash two` becomes `1/2`.

The scanner is ASCII-phrase-based over UTF-8 byte boundaries, is idempotent,
and returns over-16-KiB input unchanged.

## Pipeline

```text
cleanup → Smart Correction → Smart Formatting → Spoken Structure
→ spoken numbers → IDE context → CLI formatting → delivery
```

The `spoken_structure` stage is included in the persisted performance-stage
contract. Production telemetry records only its name, duration, outcome, and
changed flag—never transcript content.

## Source and tests

- Engine and policy: `app/src-tauri/src/spoken_structure.rs`
- Ordered stage: `app/src-tauri/src/transcript_transform.rs`
- Immutable policy resolution: `app/src-tauri/src/dictation_context.rs`
- Persisted metrics contract: `app/src-tauri/src/performance_metrics/types.rs`
- Deterministic fixtures: `app/src-tauri/eval/fixtures/deterministic/corpus-v1.json`
