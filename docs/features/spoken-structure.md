# Spoken Structure

Spoken Structure is the single deterministic owner for explicitly dictated
punctuation and layout. It renders whole-phrase cues such as `period`,
`question mark`, `new paragraph`, `open quote`, `slash`, and `backslash`
without calling a model or reading external context.

## Policy

One immutable policy is resolved at recording start:

- **Off** — neither Voice Commands nor Smart Formatting is enabled.
- **Basic** — Voice Commands is enabled; legacy punctuation, breaks,
  parentheses, and `scratch that` are available.
- **Extended** — Smart Formatting is enabled; the full punctuation, paired
  delimiter, dash, slash, and symbol grammar is available.
- **Union** — both are enabled; the union is scanned once. Legacy
  unpaired-tolerant parenthesis behavior is retained while quotes remain
  paired and bounded.

An IDE-context recording downgrades extended prose behavior but retains Basic
when Voice Commands remains enabled. Code-technical and Verbatim styles turn
both sources off. Transform instructions and imported-file transcription
always skip the stage.

## Arbitration and ownership

Explicit terminal punctuation wins over one adjacent terminal mark emitted by
the recognizer. Thus `ready? question mark.` becomes `ready?`, while two
explicit spoken marks remain intentional: `question mark exclamation mark`
becomes `?!`. Non-terminal punctuation absorbs only the identical adjacent
mark. Arbitration never crosses a newline and consumes at most one ASR mark.

`scratch that` runs in the same left-to-right pass as punctuation and line
breaks, so newly created boundaries are immediately available. User-defined
replacement and snippet output is protected in memory until this pass and is
restored literally; a snippet containing the words `period` or `new line` is
not interpreted as fresh speech.

Higher-level email, URL, enumeration, and backtracking grammars remain in Smart
Formatting and run first. Failed explicit URL grammar and inline `slash
command` cues remain untouched for their authoritative URL/CLI parsers.
Spoken numbers run afterward so `one slash two` becomes `1/2`.

The scanner is ASCII-phrase-based over UTF-8 byte boundaries, is idempotent,
and returns over-16-KiB input unchanged after restoring any protected
command-generated literals.

## Pipeline

```text
cleanup → voice commands → Smart Correction → Smart Formatting
→ Spoken Structure → spoken numbers → IDE context → CLI formatting → delivery
```

The `spoken_structure` stage is included in the persisted performance-stage
contract. Production telemetry records only its name, duration, outcome, and
changed flag—never transcript or command content.

## Source and tests

- Engine and policy: `app/src-tauri/src/spoken_structure.rs`
- Ordered stage: `app/src-tauri/src/transcript_transform.rs`
- Immutable policy resolution: `app/src-tauri/src/dictation_context.rs`
- Persisted metrics contract: `app/src-tauri/src/performance_metrics/types.rs`
- Deterministic fixtures: `app/src-tauri/eval/fixtures/deterministic/corpus-v1.json`
