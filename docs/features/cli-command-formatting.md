# Spoken CLI Command Formatting

Murmur formats likely terminal/developer commands as text after recognition. It never executes them.

## Activation boundary

The CLI stage is conservative and bounded to command-shaped lines at the utterance boundary:

- An explicit leading trigger: `command`, `shell command`, or `terminal command`.
- A known tool at the start of the utterance plus command-shaped evidence, such as a known subcommand or a spoken symbol.
- A matching per-app profile with **CLI: On**. This explicit user choice treats a multi-token utterance as command text, including unknown tools.

**CLI: Off** disables implicit detection for that app; an explicit command trigger still works. Text that does not activate is returned byte-for-byte unchanged. Mentions such as “I use git and cargo every day” and “npm is a package manager” are ordinary prose, not commands.

Physical line endings are immutable boundaries. Each line is considered independently, so a command line can be canonicalized without rewriting adjacent prose, and existing LF/CRLF endings remain unchanged.

## Transformation order

CLI canonicalization is the final deterministic transformation:

```text
raw transcript → cleanup → voice commands → Smart Correction → Smart Formatting → CLI formatting → final text
```

Running after Smart Correction lets vocabulary resolve technical names first. Smart Formatting bypasses utterances activated by the CLI grammar, including already-canonical commands, and the final CLI stage then owns separators, flags, paths, versions, and command-family casing. The pipeline keeps the original and final text together only in memory for reporting and tests; neither value is added to structured logs by this stage.

Explicit spoken vocabulary aliases are part of Smart Correction, not the CLI grammar. They can recover a recognizer error such as `Tori` -> `Tauri`; this final stage can then apply command-family casing (`npm run tauri dev`).

## Grammar and lexicon

The generic spoken-symbol grammar is separate from alias data. It recognizes bounded forms including:

- `at latest` → `@latest`
- `dash b` → `-b`; `dash dash template` → `--template`
- `slash`, `dot`, `colon`, and `equals`
- `pipe`, redirects, single/double quotes, and explicit `new line`

The local lexicon layers:

1. User-approved atom mappings from existing custom voice commands
2. Small built-in tool/package/script aliases
3. Terms captured by the immutable code-vocabulary snapshot

Code-vocabulary scans also read only the package name, script keys, and dependency keys from `package.json`. Script bodies and unrelated manifest prose are not retained. This lets project-local packages and scripts extend the formatter without hard-coded whole-command sentences.

## Examples

| Spoken | Written |
|---|---|
| `npx cc usage at latest` | `npx ccusage@latest` |
| `NPM run Tauri dev` | `npm run tauri dev` |
| `npx create vite at latest my app dash dash template react typescript` | `npx create-vite@latest my-app --template react-ts` |
| `git checkout dash b feature slash streaming` | `git checkout -b feature/streaming` |
| `cargo test dash dash dash test threads equals one` | `cargo test -- --test-threads=1` |

## Safety properties

- Deterministic and fully local; no natural-language model or cloud service.
- Activated spans are formatted, copied, and optionally pasted as text only.
- Canonical commands are idempotent.
- Non-activated whitespace, punctuation, and Unicode remain byte-for-byte unchanged.
- Existing LF/CRLF command boundaries remain byte-for-byte unchanged.
- Imported-file transcription remains raw and does not run CLI formatting.
