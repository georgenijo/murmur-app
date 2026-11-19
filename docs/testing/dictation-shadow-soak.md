# Dictation shadow: native soak runbook

The dictation shadow (appendix C8, #1154) re-decides every captured dictation
decision on a separate thread with the extracted `DictationCore` and compares the
result with what the live shell did. It never executes an effect. This runbook
covers the native part of the cutover exit criteria **E1-E9**: build the soak
binary on the Mac mini, run the native checklist with the shadow armed, and read
the two files it writes. E7 and E8 are proven by tests (see the end), not by the
soak.

Nothing in this runbook is done by CI or by a worker thread: a person (or an
approved native-UI tool) builds, launches and exercises the app.

## Build

The feature needs unwinding panics, so the soak uses its own profile (it inherits
`release` with `panic = "unwind"`). A plain `--release --features dictation-shadow`
build does not compile on purpose.

```bash
python3 scripts/build_capture_helpers.py        # once per checkout (helpers are gitignored)
cd app/src-tauri
cargo build --profile shadow-soak --features dictation-shadow
```

The binary is `app/src-tauri/target/shadow-soak/ui`. For a signed, bundled build
that macOS permissions (microphone, accessibility) accept, use the same bundle
steps as any other native build in [Development](../DEVELOPMENT.md) with
`--profile shadow-soak --features dictation-shadow`, and confirm the running app is
this build before counting a pass.

A default build (no feature) contains no shadow code, so a pass on a default build
says nothing about the shadow.

## Enable

Two switches, both required:

1. the `dictation-shadow` Cargo feature (compiled in), and
2. `MURMUR_SHADOW=1` in the environment of the process **at start**. It is read
   once; changing it later does nothing.

```bash
MURMUR_SHADOW=1 open -n /path/to/the/soak/Murmur.app      # bundled build
# or, to see stderr: MURMUR_SHADOW=1 app/src-tauri/target/shadow-soak/ui
```

A soak launched without `MURMUR_SHADOW=1` writes no files. Run exactly one Murmur
process per soak; two processes share one log directory and overwrite each other's
summary.

## Where the files land

Both files are in the app's log directory (`telemetry::logs_dir`):

| Bundle identifier | Directory |
| --- | --- |
| `com.localdictation` (production bundle) | `~/Library/Application Support/local-dictation/logs/` |
| `com.localdictation.refactor-test` | `~/Library/Application Support/local-dictation-refactor-test/logs/` |
| `com.localdictation.bench` | `~/Library/Application Support/local-dictation-bench/logs/` |

* `dictation-shadow-summary.json`: counters, written every five minutes and once
  on a **clean exit** (quit from the menu or Cmd-Q; a `kill -9` or a crash loses up
  to five minutes and the final drain). Each write goes to a unique temporary file
  and is renamed, so reading it while the app runs is safe.
* `dictation-shadow-events.jsonl`: one numeric line per mismatch,
  `{"event_code","site_id","input_kind","diff_kind"}`, at most 50 lines, truncated
  at the start of each session. Copy it before relaunching.

Both are numeric only: no transcript, audio, clipboard or selection content and no
recording or operation ids. They are safe to attach to a PR. Nothing the shadow
does goes through `tracing`.

## Reading the summary

`jq` is enough. Quit the app cleanly first, then:

```bash
S="$HOME/Library/Application Support/local-dictation/logs/dictation-shadow-summary.json"
jq '{compared, matched, mismatched_root, followup, shadow_panic, witness_overflow,
     dropped, lost_open, abandoned_by_loss, abandoned_by_panic, unverifiable,
     skipped_incomplete, unconsumed, opened}' "$S"
```

| Criterion | What to read | Pass |
| --- | --- | --- |
| **E1** | `mismatched_root` over the whole checklist (and `dictation-shadow-events.jsonl` is empty) | `0`. `followup` counts cascades of an earlier root and is not a separate failure; the first root is. |
| **E2** | Complete the native checklist (plan section 7) with the shadow on, on the same build | at least 3 passes on at least 2 different days. Keep one summary per pass. |
| **E3** | `per_row["<site>.<row>"].compared` for every C3 row of the family, summed over the checklist runs and the golden replay | each row `>= 5`, or on the family's reviewed *not natively reachable* list (named golden or unit test). See the minimum inputs below. |
| **E4** | `dropped + lost_open + sum(lost_by_site)` for the family's `site_id`s (`lost_by_site` is keyed by site id) | `0`. A lost message is an unverified input. |
| **E5** | `(dropped + lost_open + sum(lost_by_site)) / (compared + dropped + lost_open + sum(lost_by_site))` | `<= 0.001` overall |
| **E6** | `shadow_panic` and `witness_overflow` | both `0` |
| **E9** | A run is *tainted* when the shadow gave up a flight it should have compared: `abandoned_by_loss + abandoned_by_panic + skipped_incomplete + late_message_discarded + late_header_discarded + unverifiable` is not `0`, or `unconsumed` is not `0` at a clean exit. Repeat a tainted run. | `0` in the criterion runs |

Notes for reading:

* The summary has **no counter named `skipped_tainted`**; E9 is read as above. (The
  comparator's cascade control marks a recording id tainted after a mismatch;
  that shows up as `followup`, which E1 already covers.)
* `abandoned` and `abandoned_pre_d` are flights that ended without completing (a
  start dropped mid-await, a stop abandoned before D). They are not errors.
* `opened` equals the sum of `matched`, `closed_mismatched`, `unverifiable`,
  `abandoned_by_loss`, `abandoned_by_panic`, `abandoned_pre_d` and
  `skipped_incomplete`, plus the recorders still open (`<= 64`). A sum that does not
  balance is itself a bug; report it.
* `per_row[...].ticker_skipped` counts `PostDT:SD` spawns that returned before the
  ticker gate (reason `1` no recording context, `2` model without live preview). It
  is recorded, never compared.
* `pd_witness["PD-n"]` counts witnessed preserved defects (E8). A native run that
  witnesses one is information, not a failure; a decision that *differs* from the
  core is what `mismatched_root` reports.

### Reading a mismatch line

`diff_kind` (C8.3): `1` effect set, `2` effect order, `3` placement, `4` reply
kind, `5` post-state (a mirror or M1/M2), `6` continuation or history operation,
`7` observation (an input the live path did not record), `8` the shadow's own
panic, `9` a conditional effect. `site_id` is the C0.3 site (`202n` is
2021, `204a` is 2041, `204b` is 2042); `input_kind` is the `Input` variant number.
With the site and the diff kind, find the row in the summary's `per_row` entries
with `mismatched > 0` (`"<site>.<row>"`), then reproduce it with a golden or a unit
test. **A `diff_kind` 5 divergence is a rollback trigger** for a cutover (C9).

### Minimum native inputs (so E3 is not vacuous)

* start family (R101-R104): at least 5 completed starts of each of toggle, hold,
  double-tap and mic-busy refusal;
* lifecycle family (R207-R216): at least 5 each of `Accepted`, `Ready`,
  `Recovering`, `CleanupFinished`, `Idle`;
* stop family (R201-R204b): at least 5 stops each of normal, too short and empty;
* cancel family (R205-R206): at least 5 cancels each in Starting, Recording and
  Processing.

Rows a native run cannot produce (`RecoveryStalled`, `InitializationFailed`
variants, interruptions of 8,000 samples or more, timeouts) are covered by a named
golden trace or unit test on the family's not-reachable list.

## E7 and E8 (tests, not the soak)

Run from `app/src-tauri` with the feature on:

```bash
# E7: the Stage 0b goldens through the real shell and the real comparator in
# synchronous mode, 100 consecutive runs each; 0 mismatches, no loss, every flight
# closed exactly once.
MURMUR_GOLDEN_REPEAT=100 cargo nextest run --features dictation-shadow \
  -E 'kind(lib) & test(/golden_tests::golden_/)' --config-file ../../.github/nextest-golden.toml

# E8: every preserved defect whose row is in a family is witnessed (> 0) through the
# shadow, by the goldens and the family's Stage 1 PD tests.
cargo test --lib --features dictation-shadow every_family_pd_is_witnessed -- --test-threads=1

# Site order (C8.7): headers leave D, and precede the blocking effects.
cargo test --lib --features dictation-shadow headers_leave_d -- --test-threads=1
```

Capture sites are **insertion only** in the live files (D-S2c-8, D-S2c-9): against `main`
the diff adds lines and removes none, and every added line is behind
`feature = "dictation-shadow"`. `python3 scripts/check_shadow_diff.py` (CI,
`tests/test_shadow_diff.py`) reads `git diff --numstat` and fails on any deleted line in
**any** changed production path under `app/src-tauri/src` and `crates/*/src` (test files and
the named shadow-only modules excluded; a rename counts as a delete plus an add) unless
`SHADOW_DIFF_ALLOW` names it (one test-only entry today, `main_thread_watch.rs`, with its
reason). Comments are stripped before attributes are matched, and a gate that holds only in
a test build, or a file-level `#![cfg(..)]` added to a file the base had, fails like an
always-false one. Files are classified from the base revision, so adding `#![cfg(test)]` to a
production file does not exempt its deletions, and a gate that is always false
(`cfg(any())`, `cfg(not(all()))`, ...) on a statement that exists in the base fails as
disabling it.

How E7 counts, so a missed site cannot hide:

* **The count is taken at the guard.** Every lock of the dictation mutex goes through
  `MutexExt::lock_or_recover`, which (feature on) calls
  `shadow::dictation_lock_taken` with the caller's location. Each acquisition made while a
  flight is active must resolve to a header, a named no-decision outcome
  (`capture::no_decision`), or an unwind; acquisitions that are not decision sections are
  in a named allowlist by site (`replay::NON_FAMILY`). That policy scan matches the
  **receiver** (`.dictation`), so every acquisition form (`lock`, `try_lock`,
  `try_lock_or_recover`, any future method) outside the counted `lock_or_recover` and a
  short named list of non-decision reads fails (the executor's own borrowed guard field
  `ctx.dictation` is exempt in `dictation_executor/executor.rs` only)
  `every_production_dictation_lock_goes_through_the_counting_guard`.
* **No ambient flight.** There is no thread-local current flight and no stash. The
  executor carries the flight (`Executor::attach_flight`); the flight owns its mirror,
  ticker and speculative-flow slots (`Flight::slots()`), and the port wrappers write into
  those slots. Two flights interleaved on one thread across an await, or dropped out of
  order, cannot see each other. The two thread-locals left name no flight: the keyboard
  write sequence (read only inside an active flight's bracket, so a disarmed shadow reads no
  state of its own under D) and a one-call mailbox for the ticker spawner's skip.
* **Nothing is inferred.** The speculative-flow observation is the boolean the port read
  returned; the comparator checks it against the independently witnessed branch. Every
  reply is witnessed where the production code returns it (`*_witnessed` wrapper commands;
  the detached auto-stop records its real result, and a source scan pins that until georgenijo/murmur-app#1171 drives it by behaviour). A finish a drain ran from the deferred queue (closed inside a no-reply scope;
  a command nested in it keeps its own reply) and a parent whose reply is another site's
  (R201.4 / R205.4, an explicit `Delegated` disposition; the comparison stays at R206)
  close as a **named no-reply close**, never with a kind filled in. The import's pipeline
  failure is an `Err` outcome for R302.2 compared with the actual return: an `Err` return
  closes as the named no-reply close, and **any other return, the decided success
  included**, is credited with a reply kind no decision names, so the comparator reports it
  (`diff_kind=4`).
* **The gate.** `Report::assert_clean` requires `open == 0`, `lost == 0`,
  `unverifiable == 0`, `closes == headers`, credit at close, and entries == headers +
  no-decision + unwound, unless the test that provoked an incomplete close asserts it by
  name (`replay::expect`). A suite whose every `Close` is dropped fails.
* The ticker spawner makes **one** lookup (`ticker_start_check`): the production spawner and
  the harness's spawners both go through it. Its outcome (a skip, or none) overwrites a
  one-call mailbox on every call while armed, and the port that called the spawner takes it
  **once**, so no earlier call's value is ever this one's and nothing is read twice under D.
  A test per skip reason asserts exactly one note, and a Core ML replay requires the
  registration through both ready entry points (the comparator accepts any
  `NoLivePreview` skip for a recording, so only that replay sees a spawner that stops
  registering Core ML).
* **One flag load per inserted block.** Every inserted block begins with exactly one read of
  the enable flag, which is the active check; everything else sits behind it. The probe
  counts those reads (`dictation_shadow::enabled()` in test builds) and the pin
  `the_shadow_makes_only_its_one_flag_load_and_its_one_take_as_pinned` fixes, per entry,
  `(state reads, flag loads)` disarmed and armed, so a second load in a block, a second
  mailbox take, or any state read while disarmed fails it. The argument of a shadow call is
  evaluated **behind** the check (`if enabled() { ticker_outcome(ticker_skip_of(&started)) }`;
  `Flight::history_of(|| ..)` builds a history op and asks the live path's guard for it only
  for an active flight). The state reads are counted by instrumented accessors
  (`ticker_skip_of`, `registered`, the guards' `take_retired`, and `keyboard_sequence::read`,
  which is the only way to read the keyboard write sequence, the bracket's opener and
  closer), so an argument evaluated before its check, or a sequence read before the hook's
  check, fails the disarmed pin.
* **Reads are pinned.** `the_shadow_reads_the_clock_and_the_live_fields_only_as_pinned`
  counts `core_now()` calls and live-field view projections per entry (disarmed and armed),
  and `a_disarmed_keyboard_write_reads_no_shadow_state` the disarmed shadow's reads of its
  own state, which must be none. An extra read under D allocates nothing, so the
  allocation pins cannot see it.

Each `#[tauri::command]` return is witnessed by a feature-only wrapper command
(`commands/recording/shadow_commands.rs`) that calls the original and records the reply it
returned; only the `generate_handler!` entries in `lib.rs` choose between the two.

The default-build allocation pins (`recording_pd_witness_tests`, `under_d`) count
allocations only while D is held, stabilise the timing input, and pin the first delivery
on a fresh thread (cold) and a later delivery (steady) separately; there is no
minimum-of-N.

`MURMUR_E7_COLLECT=1` turns the E7 assertion into a findings listing (one
`E7FINDING` line per divergence: sub-run, site, rows, input kind, diff kind) and
keeps going, to report every divergence at once. The gate never runs in that mode.

Replays are process-global: run them with `--test-threads=1` (or nextest, one
process per test), never beside other tests that drive the shell.

## Troubleshooting

* **No files appear**: the build lacks the feature, `MURMUR_SHADOW=1` was not in
  the launch environment, or the process never ran long enough for the first
  five-minute write and never exited cleanly.
* **`dropped_unavailable > 0`**: recorders were created before the comparison thread
  published its queue (the first moments after launch). Counted, harmless.
* **`dropped` or `lost_open` > 0**: the ring (256 messages) filled or the open-record
  cap (64) was passed. Reproduce with fewer concurrent flows; this fails E4/E5.
* **A shadow panic** (`shadow_panic`, `diff_kind 8`) never reaches the live path (it
  is caught on the comparison thread); it fails E6 and must be reported with the
  site and row.
* The shadow adds no user-visible behaviour. If the app behaves differently with
  `MURMUR_SHADOW=1`, that is a bug in the capture sites: report the steps first.
