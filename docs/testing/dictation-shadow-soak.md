# Dictation shadow: native soak runbook

The dictation shadow (appendix C8, #1154) re-decides every captured dictation
decision on a separate thread with the extracted `DictationCore` and compares the
result with what the live shell did. It never executes an effect. This runbook
covers the native part of the cutover exit criteria **E1-E9** (D-S2c-14 and
D-S2c-15): build the soak bundle on the Mac mini, run the native checklist with the
shadow armed, and judge the summaries with `scripts/shadow_soak_verdict.py`. E7 and
E8 are proven by tests (see the end), not by the soak.

Nothing in this runbook is done by CI or by a worker thread: a person (or an
approved native-UI tool) builds, launches and exercises the app. Every command runs
from the repository root unless it says otherwise. Native passes take the lock
`mkdir /tmp/murmur-native-verify.lock` (remove it afterwards), because two native
runs on one Mac skew each other's timing.

Only the bundle identifier `com.localdictation.refactor-test` is used. It logs to its
own directory, has the log shipper off, and never touches production Murmur data.

## Build

The feature needs unwinding panics, so the soak uses its own profile (`shadow-soak`
inherits `release` with `panic = "unwind"`). A plain `--release --features
dictation-shadow` build does not compile on purpose, and a default build (no
feature) contains no shadow code, so a pass on it says nothing about the shadow.

```bash
python3 scripts/build_capture_helpers.py          # once per checkout (helpers are gitignored)
export MURMUR_DEV_SIGNING_IDENTITY="<a Developer ID or Apple Development identity>"
python3 scripts/build_dev_bundle.py --require-identity \
  --profile shadow-soak --features dictation-shadow \
  --config app/src-tauri/tauri.refactor-test.conf.json
```

The script prints the bundle path as `DEV_APP=...`. It is derived from the profile:
`app/src-tauri/target/shadow-soak/bundle/macos/Murmur Refactor Test.app` (under
`$CARGO_TARGET_DIR` when that is set). `--profile` cannot be combined with `--debug`.
Signing needs the Mac's desktop session (see [Development](../DEVELOPMENT.md)).

Record the commit (`git rev-parse HEAD`) and keep one signed bundle for every pass:
**E2 needs all three passes on one build.** Confirm the running binary carries the
shadow before counting a pass:

```bash
BIN="$DEV_APP/Contents/MacOS/ui"
grep -ac dictation_shadow "$BIN"    # > 0
grep -ac shadow-summary "$BIN"      # > 0
```

## Enable, and prove the session is fresh

Two switches, both required: the `dictation-shadow` Cargo feature (compiled in) and
`MURMUR_SHADOW=1` in the environment of the process **at start**. It is read once.

Settings are also read at launch. A pass may therefore be several launches, each
quit with Cmd-Q, with each session's summary kept and all of them given to the
verdict script (it sums `per_row.matched` over N summaries).

Before every launch, move the old files aside so that a stale file can never be read
as this session's:

```bash
L="$HOME/Library/Application Support/local-dictation-refactor-test/logs"
SOAK="$HOME/shadow-soak/$(date +%Y%m%d)"; mkdir -p "$SOAK"
for f in dictation-shadow-summary.json dictation-shadow-events.jsonl; do
  [ -e "$L/$f" ] && mv "$L/$f" "$SOAK/before-$(date +%H%M%S)-$f"
done
pgrep -fl "Murmur Refactor Test" || echo "none running"     # must print "none running"
open -n --env MURMUR_SHADOW=1 --env MURMUR_LOG=debug --env MURMUR_LOG_SHIPPER=off "$DEV_APP"
sleep 5; pgrep -f "Murmur Refactor Test.app/Contents/MacOS/ui" | wc -l     # exactly 1
ls -l "$L/dictation-shadow-events.jsonl"     # exists, 0 bytes: the shadow is armed
```

Run exactly one Murmur process per session (two share one log directory and overwrite
each other's summary). Quit with Cmd-Q or the menu so the final summary is written;
then copy it:

```bash
cp "$L/dictation-shadow-summary.json" "$SOAK/session-A-summary.json"
cp "$L/dictation-shadow-events.jsonl"  "$SOAK/session-A-events.jsonl"
jq '.session' "$SOAK/session-A-summary.json"
```

The `session` block is the freshness proof: `started_unix_ms` is after this launch, `pid` is
the process you launched, `git_sha` is the commit you built, `armed` is `true` and
`build_profile` is `shadow-soak`. A summary without it comes from a build before the
S1-S4 evidence and cannot count (the verdict script fails closed). A kill or a crash
loses up to five minutes and the final drain, so the session does not count.

A **non-shadow panic fails the pass**: if `app.log` in the same directory has a
`panic` line that is not the shadow's own (the shadow's panics are counted as
`shadow_panic` and fail E6), report it and repeat the session.

## Where the files land

| Bundle identifier | Directory |
| --- | --- |
| `com.localdictation.refactor-test` (**use this one**) | `~/Library/Application Support/local-dictation-refactor-test/logs/` |
| `com.localdictation` (production) | `~/Library/Application Support/local-dictation/logs/` |
| `com.localdictation.bench` | `~/Library/Application Support/local-dictation-bench/logs/` |

* `dictation-shadow-summary.json`: counters, written every five minutes and once
  on a **clean exit**. Each write goes to a unique temporary file and is renamed, so
  reading it while the app runs is safe.
* `dictation-shadow-events.jsonl`: one numeric line per mismatch,
  `{"event_code","site_id","input_kind","diff_kind"}`, at most 50 lines, truncated
  at the start of each session. Copy it before relaunching.

Both are numeric only: no transcript, audio, clipboard or selection content and no
recording or operation ids. They are safe to attach to a PR. Nothing the shadow does
goes through `tracing`. Spoken test input is a fixed content-free phrase; never log it.

## Test input and fixtures

* **No microphone on the Mac mini.** The default input is BlackHole 2ch. Feed speech with
  `say -a "BlackHole 2ch" "<fixed test phrase>"`.
* **The hotkey** is read by `rdev`, which ignores `osascript` key codes. Post real
  CGEvents: `kCGEventFlagsChanged`, keycode 56 (Left Shift), flags `maskShift | 0x2` on
  press and `0` on release, from `CGEventSource(stateID: .hidSystemState)` to
  `.cghidEventTap` (the `0x2` device bit is what makes a hold start).
* **Paste target.** Auto-paste needs another app frontmost with a text field. Open the
  checked-in fixture page in Safari (`open -a Safari docs/testing/shadow-soak-paste.html`);
  it is one textarea. TextEdit and Notes cannot be a smart-capture source. Never
  `osascript` into Safari or TextEdit (it raises Automation prompts).
* **A 500 ms auto-paste delay keeps Processing open long enough to cancel in.** In
  Settings turn auto-paste on with a 500 ms delay, relaunch (settings are read at launch),
  hold-to-dictate with Safari frontmost, and press Escape within the delay: that is the
  *Cancel while Processing* input.
* The microphone and Accessibility prompts for a new bundle are answered once by the
  person running the pass; quit and relaunch after granting.

## The native checklist

Run these with the shadow armed, across as many sessions as the settings need. Counts
are minimums per **family** (`docs/testing/shadow-rows/<family>.json` lists each family's
rows); the verdict script, not this list, decides E3.

| Family | Native inputs (>= 5 each) |
| --- | --- |
| start (R101-R104) | toggle (Home Start/Stop button), hold-to-talk, double-tap, mic-busy refusal (start a meeting on BlackHole only, then hold: `busy_meeting`) |
| lifecycle (R207-R216) | `Accepted`, `Ready`, `Recovering`, `CleanupFinished`, `Idle` come with the inputs above; `Default input switch` below drives R209.5 |
| stop (R201-R204b) | normal stops, too-short stops (hold 320-500 ms), and empty stops where they can be produced |
| cancel (R205-R206) | Escape in Starting (<= 300 ms), in Recording, and in Processing (500 ms auto-paste delay) |

### File transcription busy

Drives **R101.5** (`busy_transcribing_file`). The refusal exists only while a file
transcription is running, so the file must keep it busy for the whole step:

1. Make a long, content-free audio file (about 15 minutes):
   `say -o "$SOAK/long.aiff" "$(yes 'one two three four five six seven eight' | head -n 1500)"`
2. In Murmur, start a file transcription on it (drop the file onto the window or use the
   file transcription control) and confirm it is in progress.
3. While it runs, hold-to-dictate six times, a second apart. Each must be refused; the
   pass needs five or more.
4. Let it finish or cancel it, then continue with the checklist. If the file finishes
   before you finish the holds, the holds become ordinary recordings (that is what
   pass 1 saw): use a longer file and repeat.

### Default input switch

Replaces unplugging a microphone (D-S2c-13). It is user-level, needs no `sudo` and is
reversible. Drives **R209.5**.

1. Audio MIDI Setup (`open -a "Audio MIDI Setup"`) → **+** → *Create Aggregate Device*;
   tick BlackHole 2ch only, name it `Soak Aggregate`. It now exists as a second input.
2. Start a recording (hold-to-talk or toggle) with BlackHole 2ch as the default input.
3. While it records, switch the default input: `SwitchAudioSource -t input -s "Soak Aggregate"`
   (or System Settings → Sound → Input). Do it five or more times, switching back to
   BlackHole 2ch (`SwitchAudioSource -t input -s "BlackHole 2ch"`) before each next
   recording.
4. Afterwards switch the default input back to BlackHole 2ch and remove the aggregate
   in Audio MIDI Setup (select it, **-**). Check with `SwitchAudioSource -t input -c`.

### Sleep and wake: waived

Never sleep the machine for a soak (D-S2c-13). Interruption and runtime failure are
covered by the goldens and unit tests named in each manifest's `not_natively_reachable`
list: `golden_s03_interruption_of_8000_samples_or_more` and
`golden_s04_interruption_under_8000_samples` (interruption, R215/R216),
`golden_s05_timeout_teardown` (timeout), `golden_s12_start_failure_vs_initialization_failed`
(initialization failure) and the core-crate row tests that name each row. No native
substitute is run.

### Rows no native step reaches

The rows a native run cannot produce (`RecoveryStalled`, `InitializationFailed`
variants, interruptions of 8,000 samples or more, timeouts, an empty stop with n == 0,
already-starting/recording/recovering refusals, the X/H race rows) are on the family
manifest's `not_natively_reachable` list. Each entry names the golden or unit test that
drives it; the verdict script accepts those rows without native hits and a CI test
checks that every named test exists.

### PD-19 and the `Recovering` state

If the app is left in `Recovering` (the lifecycle state after an interruption or a
cleanup that has not finished) when a session ends, the exit leaves
`outstanding_at_exit > 0` and the session is **tainted**. Do not let it count. Quit
cleanly with Cmd-Q (never kill), keep that summary in `$SOAK` (a tainted summary is
evidence, not noise), relaunch with a fresh set of files, and restart the pass from its
first step. PD-19 is witnessed through `pd_witness["PD-19"]`; seeing it is information,
not a failure, but a decision that differs from the core is `mismatched_root`.

## Reading the result

```bash
python3 scripts/shadow_soak_verdict.py --e7 "$SOAK/e7-per-row.json" \
  "$SOAK/session-A-summary.json" "$SOAK/session-B-summary.json" ...
python3 scripts/shadow_soak_verdict.py --family start --e7 "$SOAK/e7-per-row.json" "$SOAK"/*-summary.json
```

It prints `family  E#  PASS|FAIL  values` for every family, exits 0 only if every line
passes, 1 on a FAIL and 2 on unreadable input. Summaries from earlier passes (the
moved-aside `before-*` files) are not passed to it. The criteria it applies:

| Criterion | What is read | Pass |
| --- | --- | --- |
| **E1** | global `mismatched_root` and the family's rows with `mismatched > 0` (and the events file is empty) | `0`. `followup` is a cascade of an earlier root, not a separate failure. |
| **E2** | one `session` block per summary: distinct `(pid, started_unix_ms)`, local-day of `started_unix_ms`, one `git_sha` (40 hex) and one `build_profile`, `armed` true | at least 3 passes on at least 2 different days, one build |
| **E3** | `per_row["<site>.<row>"].matched` for every C3 row of the family, summed over the summaries and the E7 dump | each row `>= 5`, or waived in the manifest with a named test, or driven by a runbook step above (those need 5 hits too) |
| **E4** | `dropped + lost_open + loss_unattributed + sum(lost_by_site[family sites])` | `0` |
| **E5** | the same sum over `compared` plus the sum, overall | `<= 0.001` |
| **E6** | `shadow_panic` and `witness_overflow` | both `0` |
| **E7** | the E7 dump: `golden_repeat >= 100` and every row it lists | present, at least 100 |
| **E8** | the manifest's `pds` for the family all `> 0` in the dump's `pd_witness` | all witnessed |
| **E9** | taint counters: `abandoned_by_loss + abandoned_by_panic + skipped_incomplete + late_message_discarded + late_header_discarded + unverifiable + unconsumed + outstanding_at_exit + core_divergences`, a non-empty `incomplete_closes`, and a conservation imbalance (`entries != headers + no_decision + unwound` for the family's sites; `conservation.non_family` is informational) | all `0`, balanced. Repeat a tainted run. |

`E3` reads `matched`, not `compared`: a row that compared but never matched is a
mismatch, not a hit.

The D-S2c-14 gate has two parts, and **both** must be true before a family's cutover is
considered:

1. **Baseline soak:** the three-pass soak with the whole table above, on one build.
2. **Per-family light gate:** for the family being cut over, the goldens x100 (E7), E8 for
   that family's preserved defects, `core_divergences == 0`, the switched-entry shell
   fixtures, and one native pass in which E1, E4, E6 and E9 are all `0`.

The manifests record, per family, which rows were classified natively reachable from the
exploratory soak pass 1 (`basis`); that classification is for a reviewer to sign off.

Notes for reading:

* `abandoned` and `abandoned_pre_d` are flights that ended without completing (a start
  dropped mid-await, a stop abandoned before D). They are not errors.
* `opened` equals the sum of `matched`, `closed_mismatched`, `unverifiable`,
  `abandoned_by_loss`, `abandoned_by_panic`, `abandoned_pre_d` and `skipped_incomplete`,
  plus the recorders still open (`<= 64`). A sum that does not balance is a bug; report it.
* `per_row[...].ticker_skipped` counts `PostDT:SD` spawns that returned before the ticker
  gate (reason `1` no recording context, `2` model without live preview). It is recorded,
  never compared.
* There is no counter named `skipped_tainted`; E9 is read as above.

### Reading a mismatch line

`diff_kind` (C8.3): `1` effect set, `2` effect order, `3` placement, `4` reply
kind, `5` post-state (a mirror or M1/M2), `6` continuation or history operation,
`7` observation (an input the live path did not record), `8` the shadow's own
panic, `9` a conditional effect. `site_id` is the C0.3 site (`202n` is
2021, `204a` is 2041, `204b` is 2042); `input_kind` is the `Input` variant number.
With the site and the diff kind, find the row in the summary's `per_row` entries
with `mismatched > 0` (`"<site>.<row>"`), then reproduce it with a golden or a unit
test. **A `diff_kind` 5 divergence is a rollback trigger** for a cutover (C9).

## E7 and E8 (tests, not the soak)

Run from `app/src-tauri` with the feature on, and `CARGO_BUILD_JOBS=4`. Do not pass
`--config-file`: the kill budgets are in `app/src-tauri/.config/nextest.toml`, which
nextest finds on its own. CI runs the same selections (the `rust-macos` shadow steps).

```bash
cd app/src-tauri
# E7: the Stage 0b goldens through the real shell and the real comparator in
# synchronous mode, 100 consecutive runs each; 0 mismatches, no loss, every flight
# closed exactly once.
MURMUR_GOLDEN_REPEAT=100 cargo nextest run --lib --features dictation-shadow \
  -E 'kind(lib) & test(/golden_tests::golden_/)'

# E8 and the per-row dump the verdict script reads (E3 adds these hits; E7 reads
# golden_repeat; E8 reads pd_witness). The dump is numeric.
MURMUR_GOLDEN_REPEAT=100 MURMUR_E7_PER_ROW_OUT="$SOAK/e7-per-row.json" \
  cargo test --lib --features dictation-shadow every_family_pd_is_witnessed -- --test-threads=1

# Site order (C8.7): headers leave D, and precede the blocking effects.
cargo test --lib --features dictation-shadow headers_leave_d -- --test-threads=1

# The rest of the shadow tests, as CI runs them.
cargo nextest run --lib --features dictation-shadow \
  -E 'test(/dictation_shadow::/) | test(/dictation_shadow_policy_tests::/)'
```

CI also runs clippy with the feature (`cargo clippy --all-targets --features
dictation-shadow -- -D warnings`), and pins both steps in
`scripts/validate_workflow_policy.py`, so a change that drops either fails the policy job.

### What `check_shadow_diff.py` guards

`python3 scripts/check_shadow_diff.py --base origin/main` (CI, `tests/test_shadow_diff.py`
and `tests/test_shadow_diff_scoped.py`) is token based, so comments, whitespace and a
spelling that hides a gate in a string do not change what it sees. It reads the diff in two
tiers (D-S2c-15 C2):

* **Always, for any PR that changes a guarded file:** the twin table must match the code.
  No new `cfg(not(feature = "dictation-shadow"))` twin appears outside `SHADOW_TWINS`;
  every `closed`, `handler` and `frozen` entry is checked as before; a `frozen_fn` entry
  pins the digest of the reviewed original function and its observed copy
  (`set_active_context`, `clear_active_context`, `cancel_recording`,
  `register_partial_ticker_locked`, `note_ticker_started`, `HideOwnership::ticker_started`
  and the `&self` `consume` of `core_history.rs`), so editing an original without
  re-pinning fails and says which side changed.
* **Only for a shadow change** (a PR that adds or removes a line under a dictation-shadow
  gate, or one naming `dictation_shadow` or `shadow_site`): insertion only, no deleted
  line in a guarded production path, every inserted token behind the feature, a test, a
  named twin or the `shadow_site!` macro (or one of the named allowances: `SHADOW_DIFF_ALLOW`
  for `main_thread_watch.rs`, `SHADOW_UNGATED_ALLOW` for the `ordinal_inline` helper), and the
  base's tokens surviving in order. A non-shadow PR may delete or rewrite live code freely.

The D-S2c-12 rules for the twins are unchanged: a `closed` twin is its original plus at most
one insertion (a trailing `.attach_flight(<ident>)` or one extra trailing `<ident>`
argument); a `frozen` twin pins the digest of the reviewed original and twin streams; the
attribute stack on a twin is part of the comparison; files are classified from the base
revision, so adding `#![cfg(test)]` to a production file does not exempt its deletions. The
seven shapes Sol found at `a4bc9d8d` (`tests/fixtures/shadow_twins/`) stay as fixtures that
must fail.

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
  the detached auto-stop's task is rebound once before the original await in `run_auto_stop`: disarmed it is the same task, armed it is wrapped so its result is witnessed at its return and passed through; the original await and diagnostic run in both builds, and `a_wrong_auto_stop_return_is_a_reply_mismatch` drives that by behaviour: a wrong return is `diff_kind=4`). A finish a drain ran from the deferred queue (closed inside a no-reply scope;
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
  check, fails the disarmed pin. The status a consume reported (`seen_status`) is kept in
  `SeenStatus`, whose store and read both take the site's flight and touch the cell only
  when it is active (a counted state access), so a disarmed start neither writes nor reads
  it. The detached auto-stop's reply witness is a gated rebinding (`witness_detached`) in
  front of the original await: disarmed it returns the very task it was given.
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
* **The summary has no `session` block, or `per_row` has no `matched`**: the build predates
  the S1-S4 evidence. Rebuild with the command above; the verdict script will not count it.
* **A stale summary**: the files were not moved aside before launch. Discard the session and
  repeat it from the freshness step.
