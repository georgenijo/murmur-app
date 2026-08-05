# Spoken Number Formatting

Murmur renders English number words as decimal digits during live dictation. This is a deterministic, local pipeline stage and is enabled by default independently of Smart Formatting.

```text
one, two, three, four, five, six
1, 2, 3, 4, 5, 6

eight hundred fifty-seven
857

ten million one hundred three thousand four hundred forty-five
10,103,445
```

## Grammar

The stage supports:

- cardinal words from `zero` through `nineteen` and the tens through `ninety`;
- `hundred` plus descending `thousand`, `million`, `billion`, `trillion`, and `quadrillion` scales;
- optional `and` inside a compound number and hyphenated forms such as `forty-five`;
- `a hundred` / `a million`, colloquial groups such as `twelve hundred`, and `negative` / `minus`;
- spoken decimals such as `three point one four` and `point oh five`.

Adjacent unit words stay separate unless the grammar proves they are one compound number, so `one two three` becomes `1 2 3`, not `6` or `123`. Punctuation outside a number span is preserved. Spoken integers of 1,000 or more use thousands separators.

The stage also groups standalone digit runs of five or more digits, so an engine-produced `35455034` becomes `35,455,034`. Four-digit digit runs and leading-zero strings stay unchanged to avoid rewriting likely years and identifiers. Long decimal fractions are not grouped.

An isolated `one` attached to ordinary prose stays spelled out as a determiner or pronoun, including `one thing`, `one idea`, `one day`, and `that one`. This also repairs an engine-produced `1` in those contexts. Explicit numeric uses continue to render as digits: a bare `one`, sequences such as `one, two, three`, compound values, and labels such as `number one`, `step one`, and `version one`.

Scales must be ordered from larger to smaller. A malformed sequence is converted only through its last unambiguous bounded group; Murmur does not guess a single value for invalid number grammar. Each number phrase is capped at 64 words, decimal fractions at 32 digits, and the complete input at 16 KiB.

## Pipeline and scope

The live order is:

```text
cleanup → voice commands → Smart Correction → Smart Formatting
→ Spoken Structure → spoken numbers → IDE context → CLI formatting → final text
```

Running after Spoken Structure allows numeric separators to disambiguate fractions (`one slash two` → `1/2`). Running before IDE and CLI formatting gives those authoritative stages numeric tokens when a code or command utterance includes a spoken number.

The pass is English-only and reads no external context. It runs for normal live dictation, including default and code-oriented profiles. The explicit Verbatim profile disables it. Imported-file transcription and selected-text transform instructions also leave number words unchanged.

Only final transformed text reaches file output, clipboard/paste, history, and stats. Stage diagnostics contain timing, outcome, and a changed flag but never transcript content.

## Source and tests

- Grammar: `app/src-tauri/src/spoken_numbers.rs`
- Pipeline integration: `app/src-tauri/src/transcript_transform.rs`
- Default/profile resolution: `app/src-tauri/src/dictation_context.rs`
