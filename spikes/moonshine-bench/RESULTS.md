# Moonshine v2 vs whisper.cpp Benchmark Results

**Hardware:** Apple M4, 16 GB unified memory, macOS
**Date:** 2026-02-23
**Methodology:** 3 runs averaged per configuration. Audio generated via macOS `say` TTS.

## Results

| Model | Clip | First Token (ms) | Total (ms) | Peak RSS (MB) | Output |
|-------|------|-------------------|------------|---------------|--------|
| whisper base.en | 3s | 106 | 106 | 234 | Hello world testing microphone. |
| whisper base.en | 10s | 178 | 178 | 245 | The quick brown fox jumps over the lazy dog. This is a longer sentence to tes... |
| whisper base.en | 30s | 322 | 322 | 260 | Local dictation is a privacy-first voice-to-text application built with Tori ... |
| whisper large-v3-turbo | 3s | 1375 | 1375 | 550 | Hello World Testing Microphone. |
| whisper large-v3-turbo | 10s | 1416 | 1416 | 562 | The quick brown fox jumps over the lazy dog. This is a longer sentence to tes... |
| whisper large-v3-turbo | 30s | 1841 | 1841 | 582 | Local Dictation is a privacy-first voice-to-text application built with Tori ... |
| moonshine tiny | 3s | 16 | 16 | 423 | Hello world testing microphone |
| moonshine tiny | 10s | 90 | 90 | 442 | The quick brown fox jumps over the lazy dog. This is a longer sentence to tes... |
| moonshine tiny | 30s | 481 | 481 | 629 | Local dictation is a privacy first voice-to-text application built with Tory ... |
| moonshine base | 3s | 37 | 37 | 862 | Hello world testing microphone |
| moonshine base | 10s | 194 | 194 | 897 | The quick brown fox jumps over the lazy dog. This is a longer sentence to tes... |
| moonshine base | 30s | 903 | 903 | 1095 | Local dictation is a privacy-first voice-to-text application built with Tori ... |

## Speed Summary (Total Inference Time)

| Model | 3s clip | 10s clip | 30s clip |
|-------|---------|----------|----------|
| **moonshine tiny** | **16 ms** | **90 ms** | 481 ms |
| moonshine base | 37 ms | 194 ms | 903 ms |
| whisper base.en (Metal) | 106 ms | 178 ms | **322 ms** |
| whisper large-v3-turbo (Metal) | 1375 ms | 1416 ms | 1841 ms |

## Model Load Memory (RSS delta on load)

| Model | Model Size on Disk | RSS Delta on Load |
|-------|-------------------|-------------------|
| whisper base.en | 147 MB | +153 MB |
| whisper large-v3-turbo | 1624 MB | +427 MB |
| moonshine tiny (int8) | ~124 MB | +332 MB |
| moonshine base (int8) | ~286 MB | +585 MB |

*Note: Peak RSS values in the main table are cumulative process-level measurements (models run sequentially in a single process). The RSS deltas above are more meaningful for comparing per-model memory footprint.*

## Key Findings

1. **Moonshine tiny is extremely fast for short clips.** At 16ms for 3s audio, it's 6.6x faster than whisper base.en (106ms) and 86x faster than whisper large-v3-turbo (1375ms). For dictation (typically short utterances), this is a massive win.

2. **Whisper base.en + Metal scales better for long audio.** At 30s, whisper base.en (322ms) slightly edges out moonshine tiny (481ms). Whisper benefits from Metal GPU acceleration while moonshine runs CPU-only through sherpa-onnx.

3. **Whisper large-v3-turbo is the slowest by far.** Despite its accuracy advantage, it takes 1.3-1.8 seconds per transcription. For real-time dictation, this latency is noticeable.

4. **Moonshine accuracy is competitive.** Both moonshine tiny and base produce comparable transcriptions on TTS audio. Moonshine tends to omit punctuation (e.g., no trailing period), while whisper large-v3-turbo capitalizes words differently. Real voice recordings would be needed to assess accuracy under noise, accents, etc.

5. **Memory footprint is reasonable for all models.** Moonshine tiny uses +332 MB on load (vs +153 MB for whisper base.en). This is acceptable for a desktop app.

6. **sherpa-rs builds cleanly on macOS.** The `download-binaries` feature provides a prebuilt universal binary — no cmake or manual compilation needed. Integration is straightforward.

## Limitations

- **Synthetic audio only.** TTS-generated WAV files are clean and unnaturally clear. Real voice recordings with background noise, varying accents, and natural speech patterns would give more representative accuracy results.
- **No CoreML acceleration for Moonshine.** sherpa-onnx supports a `coreml` provider, but the default is `cpu` due to reported instability. Testing CoreML could improve Moonshine's performance on longer clips.
- **Both models are offline/batch.** Neither whisper-rs nor sherpa-rs Moonshine supports true streaming (word-by-word output). First-token latency equals total inference time. Sherpa-onnx does support streaming with transducer models, but not with Moonshine specifically.
- **RSS measured sequentially.** Process RSS is cumulative; later models' peak RSS includes memory from earlier models that wasn't reclaimed. The RSS delta on model load is the more accurate per-model metric.

## Recommendation

**Moonshine tiny is a strong candidate for the default dictation model**, especially for short utterances (the primary use case). It delivers sub-20ms latency for typical dictation clips with acceptable accuracy.

For users who need higher accuracy, **whisper base.en with Metal** remains a good option at ~100-300ms latency, and **moonshine base** provides a middle ground.

The current default (whisper large-v3-turbo) should be reconsidered — its 1.3-1.8s latency is significantly worse than all alternatives, and its accuracy advantage may not justify the speed penalty for dictation use cases.
