# Transcription benchmark

This corpus compares Murmur backends with identical 16 kHz mono WAV input.
The runner reports model loading separately from first and warm inference and
uses ordered word-error rate (WER) against the adjacent reference transcript.

```bash
bench/make_audio.sh
cd app/src-tauri
cargo run --release --example transcription_bench -- --engine coreml --iterations 5
cargo run --release --example transcription_bench -- --engine parakeet --iterations 5
```

## Apple M4 results

Measured on the same M4 MacBook and release binary on 2026-07-16. Each warm
number is the median of five sequential inferences. Model-load time is excluded
from inference and normally overlaps recording in Murmur.

| Fixture | Audio | Core ML warm | sherpa CPU warm | Speedup | Core ML WER | CPU WER |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| short | 2.38 s | 54.8 ms | 139.1 ms | 2.5x | 0.0% | 0.0% |
| medium | 7.27 s | 73.3 ms | 492.3 ms | 6.7x | 0.0% | 0.0% |
| long | 15.21 s | 109.5 ms | 832.7 ms | 7.6x | 9.5% | 4.8% |
| xlong | 28.50 s | 159.0 ms | 1576.2 ms | 9.9x | 12.0% | 4.8% |

Cached Core ML model loads after the first pass were 99-103 ms. One first load
immediately after a full release compilation took 14.6 seconds under heavy
machine load; a repeat after the build completed took 246 ms, followed by the
cached range above. First setup downloads and compiles approximately 470 MB and
is handled by Murmur's setup screen before recording becomes available.

The WER comparison is intentionally raw backend output. Core ML's errors in the
longest technical fixture include `Tauri` to `Tori`, `Parakeet` to `Para Key`,
and `sherpa-onnx` to `Sherpa Onx`; Murmur's post-model vocabulary correction may
repair some domain terms in normal app use, but the benchmark does not apply it.

## Full pipeline observation

A separate native dev-app smoke recorded 4.47 seconds and pasted the exact
56-character result into TextEdit 176 ms after key release: 20 ms VAD, 108 ms
Core ML inference, and 37 ms clipboard plus native paste. This is one observed
smoke, not a production median.
