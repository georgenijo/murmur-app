# Performance Lab

## Overview

The Performance Lab compares installed transcription configurations on the
current machine. It is available under **Settings > Performance** and runs
entirely on device.

Supported configurations are compatible model/backend pairs:

| Model | Backend | Accelerator |
| --- | --- | --- |
| Parakeet v3 | FluidAudio Core ML | Apple Neural Engine |
| Parakeet v2 | sherpa-onnx | CPU |
| Whisper models | whisper.cpp | Metal on macOS; platform fallback elsewhere |

Missing models can be downloaded from the lab before a run. Benchmarking does
not change the model selected for normal dictation.

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
| Standard | All four clips | 5 |
| Thorough | All four clips | 10 |

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

## Concurrency

The benchmark uses isolated backend instances. Live recording and file
transcription are blocked while a benchmark owns the benchmark coordinator, and
a benchmark cannot start while either transcription path is active. Cancellation
is checked between inference calls; an inference already inside a native backend
finishes before cancellation returns.
