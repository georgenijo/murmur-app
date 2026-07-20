# Transcription benchmark

The same corpus is available to users through **Settings > Performance**. This
CLI remains useful for developer automation and raw backend investigations.

This corpus compares Murmur backends with identical 16 kHz mono WAV input.
The runner reports model loading separately from first and warm inference and
uses ordered word-error rate (WER) against the adjacent reference transcript.

```bash
# First install the selected Core ML or Parakeet model through Murmur's
# model setup screen (the runner loads an existing cache; it does not download).
bench/make_audio.sh
cd app/src-tauri
cargo run --release --example transcription_bench -- --engine coreml --iterations 5
cargo run --release --example transcription_bench -- --engine parakeet --iterations 5

# Whisper stop-latency comparison on the longest fixture. The first command-line
# argument is a 16kHz mono WAV; the second is an installed Whisper model name.
cargo run --release --example streaming_bench -- ../../bench/audio/xlong.wav base.en
```

The streaming runner warms the model, times a full-buffer batch pass, then simulates the production 10-second window / 8-second step / 2-second overlap algorithm. During-recording chunk time is reported separately; `incremental_post_stop_ms` measures only the final tail that remains after stop. It also reports reference WER and incremental-vs-batch WER so latency improvements are never presented without an output comparison.

### Issue #129 incremental result

Measured on an Apple M4 Mac mini (24 GB) with `base.en` and the 28.50-second `xlong` fixture on 2026-07-18. Latencies are medians of five warm runs:

| Path | Post-stop inference | Reference WER | Output coverage |
| --- | ---: | ---: | --- |
| Full-buffer batch | 298.0 ms | 25.3% | Truncated after "for dictating" |
| Incremental, final tail | 95.4 ms | 13.3% | Complete through "throughout the day" |

The final-tail path was 3.12x faster after stop. Three bounded chunks used a median ~433 ms of inference spread across the 28.5-second recording; no queued work or second model context was created. Incremental-vs-batch WER was 12.7%, reported because chunking changes decoding context rather than promising byte-identical output.

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

## Vocabulary alias text evaluation

`vocabulary-aliases.json` is a backend-neutral deterministic transform corpus. It exercises the production Smart Correction and CLI composition path without loading an audio model:

```bash
cd app/src-tauri
cargo test transcript_transform::tests::vocabulary_alias_eval -- --exact
```

The corpus includes `Tori`/`Tory` -> `Tauri`, command casing, punctuation, ordinary-prose false positives, and idempotence. Backend independence is guaranteed by running after recognition; raw audio WER remains separately reported above.

## Full pipeline observation

A separate native dev-app smoke recorded 4.47 seconds and pasted the exact
56-character result into TextEdit 176 ms after key release: 20 ms VAD, 108 ms
Core ML inference, and 37 ms clipboard plus native paste. This is one observed
smoke, not a production median.
