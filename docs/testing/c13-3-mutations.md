# C13.3 revert and mutation rows

The dictation-core refactor gates each family cutover on a revert check and a
mutation check for every in-scope historical bug (appendix C13.3). The rows live in
`scripts/c13_3_mutations/*.json` and `scripts/c13_3_mutations.py` re-runs them, so a
cutover PR can show that its family's rows still fail when the fix is removed.

## Rows

One JSON file per row. A row names the historical bug, the family it belongs to,
one or more `[file, old, new]` edits (paths relative to `app/src-tauri`, each `old`
occurring exactly once), and the cargo commands that must fail with the edit applied.

| Family | Rows |
|---|---|
| `admission` | `M6`, `M6r`, `M7`, `M7r`, `M7r2`, `M8`, `M8r` |
| `lifecycle` | `M384`, `M388a`, `M535`, `M535b` |
| `process_audio` | `M117` |
| `start` | `M117b`, `M216b`, `M388b`, `M535a` |
| `stop` | `M216` |
| `cancel` | none: no C13.3 bug is in the cancel family |

`python3 scripts/c13_3_mutations.py --list` prints the current table.

## Running them

Use a scratch worktree on the revision under test. The runner edits tracked source
in place, runs the commands (`CARGO_BUILD_JOBS=4`, from `app/src-tauri`), and writes
each file back from its original bytes, including after Ctrl-C. It refuses a file
with uncommitted changes.

```bash
python3 scripts/c13_3_mutations.py --family stop            # one family's rows
python3 scripts/c13_3_mutations.py --row M216-stop-queued-behind-start
python3 scripts/c13_3_mutations.py --all --keep-going       # everything
python3 scripts/c13_3_mutations.py --all --check            # edits still apply; no cargo
python3 scripts/c13_3_mutations.py --family stop --dry-run  # print edits and commands
```

Every command of a row runs. A command is `CAUGHT` only when cargo reported a failed
test that the command names (one of its filter arguments, or any test when it has
none). It is `INVALID` when no verdict can be drawn: a compile error, a cargo
infrastructure failure (no test result at all, as when a dependency download fails),
zero tests run, a crash, or a failure in tests the row does not name. A row is
`INVALID` if any command is, else `CAUGHT` if any command is, else `SURVIVED` (the
check lost its teeth). The exit status is 1 for `SURVIVED` or `INVALID`.

A cutover PR records, per row, the failing test name and the first failing
assertion. After a cutover moves the code a row edits, update that row's `old` and
`new` text in the same PR; `--check` finds the rows that no longer apply.

## CI

CI does not run these rows, because each one is a full cargo test run.
`tests/test_c13_3_mutations.py` covers the script's argument parsing, row selection,
the row-file schema and the verdict rules (on fixed cargo output and a simulated
cargo), and never starts cargo.
