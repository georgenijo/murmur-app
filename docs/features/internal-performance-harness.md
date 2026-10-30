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
VAD threshold, execution path, transform profile, percentile method, model run
order, and shared-initialization order:

```bash
python3 scripts/murmur_bench.py compare baseline.json candidate.json \
  --output comparison.json
```

The default gate reports a regression when latency grows by more than 10% and
more than 25 ms (RTF uses the relative limit), normalized recognition or
delivered WER grows by more than one percentage point, or process RSS delta
grows by more than 128 MB. Exit code 2 means the gate failed; `--no-fail` keeps
exploratory runs informational.

## Accuracy corpus and speech-model A/B

Issue #827 adds accuracy metrics for punctuation, capitalization, and named
terms on top of the latency/WER harness, plus a second private corpus built
from the user's own corrections. Everything below runs locally. Corpora and raw
reports contain reference and recognized text and stay in Application Support;
the printed table and the `.accuracy.json` summary hold only counts, rates, and
latencies.

### Building a corpus

`scripts/murmur_accuracy.py export` is an explicit opt-in copy. It needs two
local inputs:

1. **Save audio** and **Save transcript** enabled while dictating, so each
   recording leaves `murmur-NNNN.wav` plus the delivered `murmur-NNNN.txt` in
   the output folder (default `~/Documents/Murmur`).
2. A History JSON export (History → Export → JSON) after correcting entries
   with Correct and Teach (**Save correction only** is enough).

```bash
python3 scripts/murmur_accuracy.py export \
  --history ~/Downloads/murmur-history.json --dry-run   # counts only
python3 scripts/murmur_accuracy.py export \
  --history ~/Downloads/murmur-history.json
```

History keeps no audio, so the exporter pairs a recording with an entry when
the entry finished within `--max-skew-seconds` (default 20) of the WAV's
modification time and the durations agree within max(1 s, 15%). A recording
with more than one candidate, or an entry claimed by two recordings, is skipped
rather than guessed. Only pairs whose history text differs from the delivered
`.txt` are exported. The corrected history text becomes the reference. The
words the user replaced or inserted, compared case-sensitively so `github` →
`GitHub` counts, become that clip's `terms`, the homophone/name targets.

The corpus is written to `~/Library/Application Support/Murmur Accuracy
Corpus/v1` (mode 0700). The exporter refuses a non-empty destination and any
path inside the Git checkout. Its `manifest.json` reuses the guided corpus
fields (`promptId`, `label`, `reference`, `fileName`, `sha256`, `selected`)
with `corpusId: "accuracy-v1"`, `source: "corrected-history-export"`, and
`containsRealUserData: true`, plus an optional `terms` list per recording. It
has no fixed prompt count. The Rust loader (`accuracy_corpus.rs`, compiled only
with `internal-benchmark`) checks provenance, IDs, single-component WAV names,
size, and every SHA-256 before it decodes audio.

`synth --out <dir>` builds a five-clip `synthetic-say` corpus with
`containsRealUserData: false` from fixed prompts. Use it for smoke tests only.

### Metrics

`score` and `ab` compute, per model, over both the raw recognition and the
delivered (transformed) text:

| Metric | Definition |
|---|---|
| WER | The report's raw, normalized, and delivered normalized WER (same normalization as Murmur Bench). |
| Punctuation F1 | Micro F1 over `.` `,` `?` `-`. The mark after each word is compared across a word-level Levenshtein alignment. Per-mark F1 and TP/FP/FN counts are in the summary. |
| Sentence-start capitalization | Share of reference sentence starts (first word and words after `.` `?` `!` plus whitespace) whose aligned hypothesis word is capitalized. |
| Term hit rate | Each reference occurrence of a term is expected once. A hit needs the same word sequence in the hypothesis, case-insensitive, capped at the reference count. The exact-case rate also requires matching spelling and case. |

Terms come from the corpus manifest. `--terms <file>` (one per line) adds
global terms, for example for the guided personal corpus, which has no
per-clip terms.

### Running the A/B

```bash
python3 scripts/murmur_accuracy.py ab --preset standard
python3 scripts/murmur_accuracy.py ab --corpus personal \
  --corpus-dir ~/Library/Application\ Support/Murmur\ Benchmark\ Corpus/v1 \
  --terms ~/terms.txt
python3 scripts/murmur_accuracy.py score <report.json> --corpus-dir <corpus>
```

`ab` drives `murmur_bench.py run --corpus accuracy|personal` once with every
runnable arm. It writes the raw report to `~/Library/Application Support/Murmur
Bench/accuracy-reports` and then prints the table. The default arms are:

| Arm | Model | Status |
|---|---|---|
| `coreml-v3-auto` | `parakeet-tdt-0.6b-v3-coreml` | Runnable (production default) |
| `coreml-v3-en` | — | Not runnable. The pinned `fluidaudio-rs` `transcribe_samples` API takes no language hint, and the Core ML backend ignores `language`, so this arm would match `auto` byte for byte. |
| `sherpa-v2-en` | `parakeet-tdt-0.6b-v2-fp16` | Runnable (sherpa-onnx CPU, English) |
| `unified-en` | — | Not runnable. Hex's `parakeet-unified-en-0.6b` GGUF needs a GGUF Parakeet runtime that Murmur does not have. |

Any other runtime model ID can be passed as an arm (`--arms coreml-v3-auto,base.en`).
`ab` names a default-model candidate only for a real corpus with at least 20
clips. It ranks by lowest delivered normalized WER, then higher term hit rate,
then higher delivered punctuation F1, then lower warm median. Synthetic runs
print `none`. A candidate is evidence for a decision; it never changes the
shipped default by itself.

For a requested accuracy comparison (for example vocabulary biasing #832 or
the homophone pass #833), run `ab` for the baseline and the candidate on the
same corpus, arms, and trusted Mac. Compare `termHitRate`,
`termExactCaseHitRate`, delivered `punctuationF1`, and delivered normalized
WER, and share only the content-free `.accuracy.json` numbers.

## Fleet Mac runner

The trusted Mac Mini is the stable performance machine because comparisons are
meaningful only on the same hardware, OS, power state, model set, and corpus.
The remote helper accepts explicit paths so it cannot infer or delete a broad
home/workspace target. It fetches refs in a clean source repository, creates
detached temporary worktrees under the supplied cache root, shares one Cargo
release cache per immutable ref, copies the four exact gitignored helper build
prerequisites from the clean source checkout, runs both commits, compares their
reports, then removes only those temporary worktrees. It never modifies the
source helpers.

Fleet comparisons use the `conditioned-timed-path-v4` cache policy. Each
immutable ref is compiled once into its own Cargo target cache, and that exact
benchmark binary is reused for every pass. Immediately before each measured
ref, the helper first runs a discarded Quick pass of the
same commit and exact selected model set. This absorbs release compilation and
conditions every selected model's filesystem, model-weight, Core ML, and Metal
caches. It then reads that report's declared `sharedInitOrder` and runs two
identical discarded Quick passes over those targets. The fixed repeated target
workload is required for the timed path: in a clean seven-model Thorough
control, one full-set pass still left CPU Parakeet warm median, p95, and RTF
20–23% slower for the second ref, while the two target passes removed every
configured product regression. The host settles again only after all three
conditioning passes complete.

`sharedInitMs` is a setup diagnostic, not a product-latency metric. The Rust
benchmark deliberately performs and times this shared backend initialization
before it starts any model-load or inference timer. On the trusted Mac the
value can still vary from approximately 1.5 seconds to 15 seconds across
otherwise controlled processes; repeated external warm-up processes do not
predict or stabilize it. Two clean seven-model Thorough comparisons showed
that this setup-only swing can exceed 13 seconds while every configured
per-model latency, RTF, WER, and memory gate remains within threshold. Fleet
therefore records and reports a material `sharedInitMs` skew informationally,
while gating the timed product path that begins after the in-process warm-up.

The helper requires three consecutive host samples at or below 20% normalized
CPU five seconds apart both before conditioning and before starting the
measured preset. Active external build processes reset this idle window. The
host must remain on AC power with Low Power Mode off and no macOS
thermal/performance warning. Another Fleet comparison using the same cache root
causes a hard failure. The helper also samples the process tree throughout all
conditioning passes and measurement; any `cargo`, `rustc`, or executable whose
name ends in `headless_benchmark` outside the runner's own subtree invalidates
the run. The conditioning reports are temporary and are deleted without being
published.

The measured report metadata records the cache policy, conditioning durations
and `sharedInitMs` values, exact target order, conditioning commit, runner
order, host snapshots before conditioning, before measurement, and after
measurement, plus the idle-settling parameters. Comparison refuses conditioned
reports when the conditioning policy or target evidence differs, the
conditioning commit differs from the measured commit, either worktree was
dirty, host-settling policies differ, post-run host state is invalid, or model
order differs. A material measured `sharedInitMs` difference is retained in the
comparison receipt as an informational setup skew. No product regression
threshold is changed or bypassed.

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
feature establishes the baseline for subsequent commits. Each ref incurs the
discarded full-set Quick conditioning stage and two target-only Quick passes.
Use Quick (5 clips × 1) for a fast candidate smoke run, Standard (20 × 1) for
requested routine comparisons, and Thorough (20 × 3) for a requested deeper
comparison. Alternate
`--candidate-first` between repeat comparisons when investigating small deltas
to expose residual order/thermal bias.

## Requested comparisons

Run benchmarks only when George explicitly requests them. Follow the optional
performance policy in [the agent guide](../../CLAUDE.md). Routine implementation,
testing, PRs, and releases carry no benchmark gate or reporting requirement.

For a requested comparison, agree on or choose the relevant baseline, immutable
candidate commit, models, and preset. Fetch the candidate on the trusted Mac and
verify that it resolves before starting. Record the tested refs, preset, models,
thresholds, aggregate deltas, and pass/fail. A result describes only the tested
revision; do not present it as evidence for a later revision.

Use the normal failing exit status to interpret the result. If a requested
comparison fails, one `--candidate-first` repeat can help expose order or thermal
bias. Report mixed results as inconclusive. Benchmark results become a merge or
release condition only when George explicitly requests that condition.

Keep raw reports and personal transcript content on the trusted Mac. Shared
results contain only content-free provenance and aggregate metrics. Saved-WAV
replays do not verify live capture, device switching, or clipboard/paste delivery.

## CI policy

Do not upload raw personal reports as CI artifacts: they contain real reference
and recognized text. A future self-hosted GitHub runner may invoke the same
remote helper, but it should retain raw reports locally on the Mac Mini and
publish only a content-free pass/fail summary. Until runner credentials are
installed, Fleet is the control plane and the Mac Mini is the execution and
report-retention host.
