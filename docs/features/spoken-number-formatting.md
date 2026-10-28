# Spoken Number Formatting

Murmur renders English number words as decimal digits during live dictation. This is a deterministic, local pipeline stage. It is enabled by default and runs independently of Smart Formatting. How eagerly it converts depends on the recording's Mode.

```text
eight hundred fifty-seven
857

ten million one hundred three thousand four hundred forty-five
10,103,445
```

## Mode-aware policy

The stage reads one content-free register that is frozen at recording start: the built-in Mode family that the effective writing style resolves to (see [Modes](modes.md#text-register)). Custom Modes and app profiles map by their writing style. Murmur never infers the policy from transcript text.

| Register | Policy |
|---|---|
| Everyday, Messages, Email, Notes | **Prose** |
| Technical, Terminal, or any recording with Local IDE project context | **Technical** |
| Verbatim | Stage off; text is byte-for-byte unchanged |

### Prose

Small standalone counts from zero through nine stay words when they are attached to ordinary language. Coordinated small counts stay words together.

```text
two of them              → two of them
zero tolerance           → zero tolerance
two or three days        → two or three days
We need two, three, or four people → unchanged
```

Prose still renders digits for:

- numbers of ten or more (`twelve bags` → `12 bags`);
- a count followed by a unit, currency, or percent word, or preceded by a currency symbol (`five percent` → `5 percent`, `two or three dollars` → `2 or 3 dollars`). Durations such as minutes, hours, and days are not units here: `five minutes` stays words;
- a small count coordinated with a value that needs digits (`nine or ten` → `9 or 10`);
- comma-separated sequences and bare utterances that are only numbers (`one, two, three` → `1, 2, 3`; `two` → `2`), which keeps the explicit number-dictation contract;
- times, dates, years, labels, and dotted values, as described below.

Unit words are not rewritten into symbols. `five percent` becomes `5 percent`, not `5%`.

### Technical

Every cardinal becomes digits (`two of them` → `2 of them`). In addition:

- spaced single-digit runs join: `eight zero eight zero` → `8080`, `four oh four` → `404`;
- dotted values join, including IP-like addresses: `one ninety two point one sixty eight point one point one` → `192.168.1.1`. `dot` also separates segments;
- ordinals of ten or more use digits: `the twenty-first item` → `the 21st item`;
- existing digit runs keep their exact digits. Technical identifiers such as `123456` are not regrouped.

## Rules in every non-Verbatim policy

- **Times.** An hour from one to twelve plus minutes becomes `H:MM` when followed by `AM`/`PM` (`a.m.`/`p.m.`) or preceded by `at`, `by`, `until`, `till`, `from`, `before`, `after`, or `since`: `four thirty PM` → `4:30 PM`, `at two thirty` → `at 2:30`, `from ten oh five` → `from 10:05`. An hour alone becomes digits before `AM`/`PM` or `o'clock`: `one PM` → `1 PM`, `five o'clock` → `5 o'clock`. Murmur does not assume a cue-less pair is a time. `meet at two` stays words in prose.
- **Years.** `eighteen`, `nineteen`, or `twenty` followed by a two-digit part is a year: `twenty twenty six` → `2026`, `nineteen eighty four` → `1984`, `twenty oh five` → `2005`. A unit second half stays a cardinal: `twenty one` → `21`. A spoken thousand is a year only after a year cue (`in`, `since`, `year`, `until`, `till`, `from`, `before`, `after`, `during`, `circa`) or a date: `in two thousand twenty six` → `in 2026`. Without a cue it is a count: `two thousand twenty six` → `2,026`.
- **Dates.** A month name followed by a day renders the day as digits: `September seventeenth` → `September 17`, `March twenty-first` → `March 21`. A following year gets a comma: `September seventeenth twenty twenty six` → `September 17, 2026`. `the fifth of November` → `the 5th of November`. `March`, `May`, and `August` are also ordinary words, so they must be capitalized and followed by an ordinal day: `May first` is a date, `you may two` is not.
- **Labels.** A value after a label uses digits, and a spoken digit run after a label joins: `step two` → `step 2`, `room four oh two` → `room 402`, `port eight zero eight zero` → `port 8080`, `PR seven nine one` → `PR 791`. Labels include number, step, option, version, chapter, page, line, issue, ticket, PR, port, room, section, figure, item, episode, season, level, phase, and error. Labels that are also common verbs (build, release, model, rank, table, part, track, pin) label a value only when it is dotted or ends the phrase: `in build two` → `in build 2`, `release two point one` → `release 2.1`, but `build two apps` stays words in prose.
- **Dotted values.** `X point Y point Z` becomes a dotted value with spoken segments: `version two point seventeen point two` → `version 2.17.2`, `python three point eleven` → `python 3.11`.
- **Hyphenated compounds stay words.** A number hyphen-joined to an ordinary word is left unchanged: `two-factor`, `one-on-one`, `two-by-four`, `zero-copy`, `forty-five-minute`. Compound numbers such as `fifty-seven` still convert.
- **Ordinals.** Ordinals become digits only where digits are chosen: dates (`September first` → `September 1`) and, in Technical, ordinals of ten or more. Prose keeps `the twenty-first century`, and `first` through `ninth` never change outside a date. `second` is also a unit of time, so `a twenty second timeout` stays a cardinal.
- **Bare `point`.** `point five` is a decimal only at a clause start or after a word that introduces a value (`is`, `of`, `and`, `minus`, `about`, ...): `the ratio is point five` → `the ratio is 0.5`. `at this point five people` keeps the noun.
- **The determiner `one`.** An isolated `one` attached to ordinary prose stays spelled out, including `one thing`, `that one`, and `the one point I want to make`. This also repairs an engine-produced `1` in those contexts. Labels, times, units, currency, compounds, and sequences keep `one` numeric.

## Grammar

The cardinal parser supports:

- `zero` through `nineteen` and the tens through `ninety`;
- `hundred` plus descending `thousand`, `million`, `billion`, `trillion`, and `quadrillion` scales;
- optional `and` inside a compound number and hyphenated forms such as `forty-five`;
- `a hundred` / `a million`, colloquial groups such as `twelve hundred`, and `negative` / `minus`;
- spoken decimals such as `three point one four` and `point oh five`.

Adjacent unit words stay separate unless the grammar proves they are one compound number, a Technical digit run, or a labelled value. So `one two three` becomes `1 2 3` in prose and `123` in Technical. Punctuation outside a number span is preserved. Spoken integers of 1,000 or more use thousands separators, except years.

In prose, the stage also groups standalone digit runs of five or more digits, so an engine-produced `35455034` becomes `35,455,034`. Four-digit runs, leading-zero strings, labelled values, and every Technical digit run stay unchanged.

Scales must be ordered from larger to smaller. A malformed sequence is converted only through its last unambiguous bounded group; Murmur does not guess a single value for invalid number grammar. Each number phrase is capped at 64 words, decimal fractions at 32 digits, dotted values at 8 segments, and the complete input at 16 KiB.

## Pipeline and scope

The live order is:

```text
cleanup → voice commands → Smart Correction → Smart Formatting
→ Spoken Structure → spoken numbers → IDE context → CLI formatting → final text
```

Running after Spoken Structure allows numeric separators to disambiguate fractions (`one slash two` → `1/2`). Running before IDE and CLI formatting gives those authoritative stages numeric tokens when a code or command utterance includes a spoken number.

Correct and Teach learns from final history text, where numbers are already digits. When Smart Correction is on, explicit aliases and learned rules whose spoken form contains a digit (for example `GPT 5` → `GPT-5`) are therefore matched again at the end of this stage, against the rendered digits. See [Correct and Teach](correct-and-teach.md#scope-and-precedence).

The pass is English-only. It runs for normal live dictation, including default and code-oriented profiles. The explicit Verbatim profile disables it. Imported-file transcription and selected-text transform instructions also leave number words unchanged.

Only final transformed text reaches file output, clipboard/paste, history, and stats. Stage diagnostics contain timing, outcome, and a changed flag but never transcript content or the register.

## Source and tests

- Grammar and policy: `app/src-tauri/src/spoken_numbers.rs`
- Register plumbing and pipeline integration: `app/src-tauri/src/transcript_transform.rs`
- Register resolution: `app/src-tauri/src/state.rs` (`TextRegister`) and `app/src-tauri/src/dictation_context.rs`
- Mode-aware evaluation fixtures: `app/src-tauri/eval/fixtures/deterministic/corpus-v1.json`
