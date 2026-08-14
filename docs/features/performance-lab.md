# Performance Lab

## Overview

The Performance Lab compares installed transcription configurations on the
current machine. It is available under **Settings > Performance** and runs
entirely on device.

The UI deliberately labels every run as a **directional local comparison**, not
a universal model ranking. The bundled corpus is clean synthetic English speech
and does not represent a user's voice, accent, microphone, room, or every
dictation workload.

Supported configurations are compatible model/backend pairs:

| Model | Backend | Accelerator |
| --- | --- | --- |
| Parakeet v3 | FluidAudio Core ML | Apple Neural Engine |
| Parakeet v2 | sherpa-onnx | CPU |
| Whisper models | whisper.cpp | Metal on macOS; platform fallback elsewhere |

Missing models can be downloaded from the lab before a run. Benchmarking does
not change the model selected for normal dictation.

The lab gets labels, backend/accelerator metadata, platform support, and install
state from the same model-runtime catalog used by onboarding and Settings. Its
benchmark runner also creates backends through the catalog factory, so adding a
model does not require a second backend-name classifier.

## Microphone startup diagnostic

**Test microphone startup** runs five bounded start/stop cycles against the
currently selected input, including a saved pinned input that is presently
missing. It exercises the signed production capture worker and its real
AUHAL/CPAL setup budgets. Each cycle stops only after the exact worker has exited,
joined, and returned the single-owner lifecycle to Idle; a later cycle can never
overlap a recovering predecessor. The action refuses while dictation, Transform,
Voice Query, meeting capture, file transcription, corpus recording, model
preparation, or another benchmark owns the relevant runtime.

Live progress is correlated by a frontend UUID and a Rust-monotonic benchmark
ID. The UI installs and confirms its event listener before enabling Run, ignores
events from older IDs, and sends the UUID on cancellation. Starting, capturing,
stopping, recovering, waiting-for-owner, and complete states stay visible until
the run command resolves after post-join Idle. After confirmed teardown,
cancellation produces a bounded partial report rather than discarding completed
cycles.

The result separates whole-cycle start-to-first-PCM from the production attempt
records that produced it. For each resolution pass and backend attempt it shows:

- AUHAL or CPAL, immutable attempt order, and whether that order came from the
  default policy or the session's existing first-PCM memo;
- successful attempt start-to-first-PCM, active elapsed time, and attempt budget;
- fallback use, stable failure kind/phase, and the last entered or completed
  native setup step.

Attempt start-to-first-PCM is wall time after the worker Start message. Active
elapsed time excludes a pending microphone-permission prompt, so it may be
smaller than the wall-clock startup value.

The dashboard reports whole-cycle median/p95/range plus independent AUHAL and
CPAL attempt counts, successes, failures, median, p95, and maximum over successful
attempts. It never forces fallback merely to collect a backend sample, so a
backend with no successful attempt displays no invented latency. Diagnostic
captures do not train or mutate the production backend memo.
With the default five-cycle run, nearest-rank p95 is the maximum successful
sample, so it is a bounded tail signal rather than a high-confidence percentile.

No PCM is transcribed, retained after readiness, written, copied, or emitted.
Reports contain only fixed enums, timings, cycle counters, app version, `macos`,
and whether the request used System Default or a pinned input—never the device
ID/name, raw Core Audio errors, paths, hostnames, audio, or transcript content.
The latest ten full reports persist in the local Performance Lab dashboard.
Cancelled/partial reports remain session-only unless the user explicitly chooses
Copy JSON or Save to file; only full runs may use the Lab's existing auto-save
folder. The dedicated typed save command revalidates schema/cross-fields and
writes a Rust-owned filename.

## Internal personal corpus recorder

The private **Murmur Bench** flavor includes a guided recorder for building a repeatable
corpus from the developer's own voice and microphone. This UI and its Rust IPC
surface are compiled only with the `internal-benchmark` feature; neither is
registered in a normal Murmur build. It provides 20 fixed prompts
covering short commands, ordinary prose, technical terms, numbers, natural
disfluencies, long passages, pauses, faster delivery, and quieter delivery.

This is a capture-only path. It uses the same signed microphone worker and mono
16 kHz resampling as production dictation, but it does not load a transcription
model, write transcript history, touch the clipboard, auto-paste, or run a text
transform. Recording is mutually exclusive with dictation, file transcription,
selected-text transforms, and Performance Lab runs.

Recordings are stored outside the repository at:

```text
~/Library/Application Support/Murmur Benchmark Corpus/v1
```

The `audio/` directory contains sequential, prompt-labelled WAV files. A
versioned `manifest.json` records the exact reference, selected take, SHA-256,
duration, input-level measurements, microphone label, and quality warnings.
Retakes are non-destructive: the newest take is selected while earlier WAVs stay
available. The app warns about recordings that are very short, very quiet, or
clipping. These files contain real user data, remain local, and must not be
committed to Git.

The internal benchmark runner verifies all 20 selected WAV files against their
manifest SHA-256 values before decoding them, then replays the fixed clips
through the same VAD, model, transcript-transform, and scoring path on every
run. Quick uses 5 clips once, Standard uses all 20 once, and Thorough uses all
20 three times. See [Internal performance harness](internal-performance-harness.md)
for Fleet automation and comparison gates.

The separately identified **Murmur Refactor Test** bundle writes ordinary and
structured logs under `local-dictation-refactor-test/logs`, never the production
`local-dictation/logs` directory. Its central log shipper is disabled, keeping
trial sessions locally attributable and preventing them from advancing or
uploading the production shipper queue.

## Accuracy

Each bundled 16 kHz mono WAV fixture has an adjacent reference transcript.
Murmur compares each measured transcript with the reference and reports raw and
normalized word error rate (WER):

```text
(substitutions + deletions + insertions) / reference words
```

Normalized WER ignores formatting and number, unit, or compound-word spelling
differences so accuracy ranking reflects recognition. Raw WER remains visible in
parentheses. Delivered WER also scores the text after Murmur's production
transform pipeline, showing the result that would reach the clipboard. The
report keeps the reference, median-error measured output, error count, and
reference word count for every clip. This makes the accuracy result inspectable
without letting a single outlier iteration decide the ranking. Free-form speech
without a known transcript can measure latency but cannot produce an honest
accuracy score.

## Workloads

| Preset | Corpus | Measured runs per clip |
| --- | --- | ---: |
| Quick | Short and medium | 3 |
| Standard | Four original clips plus jargon, numbers, and disfluent stress fixtures (7 clips) | 5 |
| Thorough | Standard plus extra-extra-long and fast fixtures (9 clips) | 10 |

The bundled clips first pass through the same Silero VAD speech filter used by
normal dictation at a fixed threshold, keeping runs comparable even when the
user changes dictation sensitivity. VAD time is excluded from inference
measurements, and the reported audio duration reflects speech retained by VAD.
One untimed inference then warms each clip before measured iterations begin.
Models run sequentially and are released between configurations to avoid
contention.

## Results

The report separates:

- Cached model load time
- First inference time
- Warm median and p95 inference
- Duration-weighted corpus speed from each clip's median latency
- Raw, normalized, and delivered WER across the corpus
- Process memory increase observed at benchmark checkpoints
- Catalog download size, kept separate from observed process memory

New reports use report schema version 3 and record the environment (OS/version,
architecture, hardware model/chip, and RAM when available), corpus fixture IDs
and source, reference-word count, fixed VAD threshold, full-buffer final-after-stop
execution path, default delivery transform profile, nearest-rank percentile
method, model run order, and shared-initialization order. The metadata excludes
hostname, serial number, paths, window titles, and other user content. Reports
saved before this additive metadata remain readable and are identified in the UI
as legacy saved reports.

Recommendations remain explainable: **Fastest** has the strict lowest
duration-weighted realtime factor, and **Accurate** has the lowest normalized
recognition WER. **Balanced** first keeps models within two percentage points of
the best normalized recognition WER, treats realtime factors within an inclusive
10% of the fastest eligible model as equivalent, and prefers the lowest observed
memory increase within that speed band. Exact remaining ties use model name for
deterministic results.

The dashboard plots median/p95 latency and word accuracy separately, followed by
the complete metric table and transcript-level details. The latest ten reports
stay in local storage and can be selected from the saved-run menu or copied as
JSON. Benchmark audio and transcripts are bundled with Murmur; no audio or
result is uploaded.

P95 is nearest-rank over only 3, 5, or 10 measured warm samples per clip, so it
is a coarse tail-latency signal. Cold model load excludes the one-time shared
backend priming shown separately. Memory is a sequential process-RSS delta and
can be affected by allocator retention from an earlier model; it is neither the
catalog download size nor an isolated peak-memory measurement.

## Concurrency

The model benchmark uses isolated backend instances. In the internal build, live
recording, personal-corpus recording, and file transcription are blocked while a benchmark owns the
benchmark coordinator, and a benchmark cannot start while any recording or
transcription path is active. Cancellation is checked between inference calls;
an inference already inside a native backend finishes before cancellation
returns.

The microphone diagnostic instead owns the same audio lifecycle as production
capture under `MicrophoneBenchmark(benchmarkRunId)`. It never borrows Preview,
never bypasses the transition lock, and never considers stop acknowledgement to
be teardown completion. Its immutable backend plan is snapshotted from
production state once at run start and reused for every cycle; successful
diagnostic PCM is explicitly excluded from memo training.

These isolated benchmark instances do not replace the selected dictation model
or publish shared-runtime lifecycle changes. There is no automatic fallback if
a selected benchmark model cannot load; that model receives an explicit error
result and the run proceeds to the next user-selected entry.
