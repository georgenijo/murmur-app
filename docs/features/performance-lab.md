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
Murmur normalizes both texts into words and calculates word error rate (WER):

```text
(substitutions + deletions + insertions) / reference words
```

The report keeps the reference, model output, error count, and reference word
count for every clip. This makes the accuracy result inspectable. Free-form
speech without a known transcript can measure latency but cannot produce an
honest accuracy score.

## Workloads

| Preset | Corpus | Measured runs per clip |
| --- | --- | ---: |
| Quick | Short and medium | 3 |
| Standard | All four clips | 5 |
| Thorough | All four clips | 10 |

The bundled clips first pass through the same Silero VAD speech filter used by
normal dictation. VAD time is excluded from inference measurements, and the
reported audio duration reflects speech retained by VAD. One untimed inference
then warms each clip before measured iterations begin. Models run sequentially
and are released between configurations to avoid contention.

## Results

The report separates:

- Cached model load time
- First inference time
- Warm median and p95 inference
- Speed relative to the audio duration
- Weighted WER across the corpus
- Process memory increase observed at benchmark checkpoints

Recommendations remain explainable: **Fastest** has the lowest warm median,
**Accurate** has the lowest WER, and **Balanced** is the fastest model within two
percentage points of the best WER.

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
