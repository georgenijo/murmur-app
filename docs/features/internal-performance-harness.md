# Internal performance harness

## Purpose and distribution boundary

Murmur Bench is a private engineering flavor for answering one repeatable
question: did a candidate build change recognition latency, accuracy, delivery,
or memory on the same Apple Silicon hardware and the same recorded speech?

It is not a consumer feature. A normal build does not enable the Cargo feature,
does not register the corpus recorder IPC commands, cannot deserialize the
`personal` corpus source, and does not show the recorder or personal replay UI.
The internal flavor is built explicitly with both controls:

```bash
cd app
npm run tauri:bench:build
```

That produces **Murmur Bench** with bundle ID `com.localdictation.bench`. It has
its own webview data, app data, and `local-dictation-bench/logs` directory.
Launch-at-login, the updater plugin, updater UI/menu activity, and central log
shipping are disabled. It can coexist with production Murmur and Murmur
Refactor Test.

## Private corpus

The guided recorder creates 20 fixed prompts under:

```text
~/Library/Application Support/Murmur Benchmark Corpus/v1
```

The corpus contains real voice data and reference transcripts. It is ignored by
Git, never uploaded by Murmur, and should be copied only to a trusted benchmark
Mac. Before every replay, Murmur requires exactly 20 unique selected prompts,
safe single-component WAV names, bounded file sizes, the expected manifest
provenance, and a matching SHA-256 for every file. An alternate absolute root
may be supplied through `MURMUR_BENCH_CORPUS_DIR` only in an internal build.

## Local headless runner

Run a release-mode personal benchmark without launching the app:

```bash
python3 scripts/murmur_bench.py run \
  --corpus personal \
  --preset standard \
  --models base.en \
  --machine-label macbook
```

Model IDs must match the runtime catalog. With no `--models`, every installed
model is selected. Reports default to:

```text
~/Library/Application Support/Murmur Bench/reports
```

Each report stays compatible with the Performance Lab JSON schema and may
contain reference and recognized transcript text. A sibling `.meta.json` adds
the Git commit, branch, dirty state, machine label, timing, corpus tier, and
model selection without modifying the portable report.

Compare two runs made on the same OS, hardware, corpus, preset, iteration count,
VAD threshold, and execution path:

```bash
python3 scripts/murmur_bench.py compare baseline.json candidate.json \
  --output comparison.json
```

The default gate reports a regression when latency grows by more than 10% and
more than 25 ms (RTF uses the relative limit), normalized recognition or
delivered WER grows by more than one percentage point, or process RSS delta
grows by more than 128 MB. Exit code 2 means the gate failed; `--no-fail` keeps
exploratory runs informational.

## Fleet Mac runner

The trusted Mac Mini is the stable performance machine because comparisons are
meaningful only on the same hardware, OS, power state, model set, and corpus.
The remote helper accepts explicit paths so it cannot infer or delete a broad
home/workspace target. It fetches refs in a clean source repository, creates
detached temporary worktrees under the supplied cache root, shares one Cargo
release cache, copies the four exact gitignored helper build prerequisites from
the clean source checkout, runs both commits, compares their reports, then
removes only those temporary worktrees. It never modifies the source helpers.

Example on the Mac Mini:

```bash
python3 ~/Library/Application\ Support/Murmur\ Bench/tools/murmur_bench_remote.py \
  --repo /Users/george-mac-mini/Documents/code/murmur-app \
  --baseline origin/main \
  --candidate origin/codex/fluidvoice-trial \
  --corpus-dir '/Users/george-mac-mini/Library/Application Support/Murmur Benchmark Corpus/v1' \
  --cache-root '/Users/george-mac-mini/Library/Caches/Murmur Bench' \
  --report-root '/Users/george-mac-mini/Library/Application Support/Murmur Bench/reports' \
  --preset standard \
  --machine-label mac-mini
```

From any machine with Fleet configured, the repository wrapper supplies those
trusted Mac Mini paths:

```bash
python3 scripts/murmur_bench_fleet.py \
  --baseline origin/main \
  --candidate origin/my-branch \
  --preset standard \
  --models base.en
```

Both refs must contain the internal harness, so the first completed run of this
feature establishes the baseline for subsequent commits. Use Quick (5 clips ×
1) for a fast candidate smoke run, Standard (20 × 1) for routine comparisons,
and Thorough (20 × 3) before a release. Alternate `--candidate-first` between
repeat comparisons when investigating small deltas to reduce order/thermal
bias.

## Required PR and release gates

For a PR that can change recognition latency, accuracy, delivered-text output,
or memory, resolve the immutable PR-head SHA after pushing the exact candidate
branch, then run the Fleet wrapper against that SHA:

```bash
PR_HEAD_SHA="$(gh pr view --json headRefOid --jq .headRefOid)"
python3 scripts/murmur_bench_fleet.py \
  --baseline origin/main \
  --candidate "$PR_HEAD_SHA" \
  --preset quick
```

Fetch `origin` on the trusted benchmark Mac first and verify that it resolves
`PR_HEAD_SHA`; do not substitute a moving branch name. The same SHA must appear
in the validation receipt.

The gate applies to VAD, transcription backends, model runtime, transcript
transforms, benchmarked execution paths, and performance-sensitive Rust
dependencies. Use Standard for shared cross-model or pipeline changes. A PR
outside that surface records `Murmur Bench: N/A — <reason>` in its validation
receipt instead of running an irrelevant benchmark.

Record the tested candidate commit SHA in the receipt. Any later push, rebase,
merge from main, or conflict resolution invalidates the result and requires a
rerun against the new immutable candidate SHA before merge.

Before every release, compare the previous release tag with the exact
`origin/main` release candidate using Standard. Use Thorough when any commit
since the tag touches the benchmark-sensitive surface:

```bash
python3 scripts/murmur_bench_fleet.py \
  --baseline v<previous-version> \
  --candidate origin/main \
  --preset standard
```

Do not use `--no-fail` for either gate. On a failed comparison, repeat once
with `--candidate-first`. A repeated regression blocks merge or release. Mixed
results are inconclusive rather than a pass and require investigation or the
user's explicit acceptance of the measured risk. Keep raw reports and personal
transcript content on the trusted Mac. GitHub receipts may contain only
content-free provenance and results: exact refs, candidate SHA where applicable,
preset, model names, thresholds, aggregate deltas, and pass/fail.

This replay gate does not exercise live Core Audio startup, device switching,
first PCM, or real clipboard/paste delivery. Native smoke tests and the
post-release production-latency check remain mandatory where applicable.

## CI policy

Do not upload raw personal reports as CI artifacts: they contain real reference
and recognized text. A future self-hosted GitHub runner may invoke the same
remote helper, but it should retain raw reports locally on the Mac Mini and
publish only a content-free pass/fail summary. Until runner credentials are
installed, Fleet is the control plane and the Mac Mini is the execution and
report-retention host.
