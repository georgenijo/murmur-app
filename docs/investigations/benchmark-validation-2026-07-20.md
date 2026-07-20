# Benchmark validation — what the Performance Lab now proves and can discern

**Date:** 2026-07-20 · **Branch:** `bench/integration` (issues #269, #270, #271, #272, #273, #274 + headless runner)
**Data:** three fresh runs on the production MacBook via the headless runner — 2× Standard (all 7 installed models, 7 fixtures, back-to-back, identical config) + 1× Thorough (9 fixtures incl. 64s `xxlong`). Raw reports: `/tmp/bench-std-run{1,2}.json`, `/tmp/bench-thorough.json` on that machine.

## What changed

| Issue | Fix | Proof in this data |
|---|---|---|
| #269 | `single_segment` now duration-conditional (≤12s); long batch decodes are multi-segment | `xxlong` (213 ref words): every model reaches the final clause; tiny.en delivers 212/213 words. The old code truncated 28s audio at ~24s. |
| #270 | WER normalized (digits↔words, units, compounds) before scoring; raw kept alongside | Standard corpus: small.en raw 14.5% → normalized 7.7%; the raw metric was hiding that small.en is the most accurate installed model. |
| #271 | Benchmark scores **delivered** text (whisper dev-vocab prompt + production transform pipeline) as a third tier | Parakeet v3 jargon fixture: 22.2% normalized → **7.4% delivered** — smart correction repairs camelCase identifiers. v3 ignores prompts, so post-ASR correction is its only accuracy lever, now measured. |
| #272 | Recommendations rank on realtime factor with a 10% tie band + memory tie-break | Both Standard runs pick identical recommendations (previously `fastest` flipped between identical runs). |
| #273 | 5 stress fixtures (jargon, numbers, disfluent, xxlong, fast) | Corpus no longer saturates: normalized WER spread across models is 7.7–11.4% (was: four models tied at 0.0%). |
| #274 | Untimed shared-init warm-up; `sharedInitMs` reported once | Cold run: sharedInit = 14,195ms while tiny.en's own load = 51ms (was: 12.5s misattributed to tiny.en). Warm runs: sharedInit ≈ 2.1s. |
| — | Headless runner (`tests/headless_benchmark.rs`) | These three reports were produced by it — first step toward #267's "one command runs the eval suite". |

## Headline results (Standard preset, both runs agree)

| model | RTF | raw WER | norm WER | delivered norm WER | mem Δ MB | load ms |
|---|---|---|---|---|---|---|
| parakeet v3 (Core ML) | **0.0079** | 16.9 | 11.4 | 9.8 | **65** | 105 |
| parakeet v2 (fp16) | 0.047 | 14.1 | 9.8 | 9.8 | 2324 | 1313 |
| tiny.en | 0.0092 | 17.3 | 10.6 | 10.6 | 156 | 51 |
| base.en | 0.0148 | 16.1 | 10.2 | 10.2 | 170 | 70 |
| small.en | 0.041 | 14.5 | **7.7** | **7.7** | 544 | 181 |
| medium.en | 0.107 | 12.5 | 8.1 | 8.1 | 1664 | 523 |
| large-v3-turbo | 0.112 | 15.7 | 9.3 | 8.5 | 1604 | 598 |

Recommendations (identical across both runs): fastest = **parakeet v3**, mostAccurate = **small.en**, balanced = **small.en**.

## What the benchmark has proved

1. **The truncation bug was real and is fixed** — silent tail loss on >24s batch/file transcriptions affected production paths (batch fallback, file transcription), not just the benchmark. Fixed, model-proven, regression-tested.
2. **Run-to-run recommendation stability** — two identical runs now produce identical picks. The remaining jitter (parakeet v2 RTF 0.047 vs 0.061 between runs, ~29%) is real machine noise the tie-band absorbs.
3. **Raw WER was ranking models wrong.** ~Half of the reported error mass was formatting/ITN, distributed unevenly across models (it punished v3's digit style and small.en's compounds hardest).
4. **The correction layer measurably works, and matters most for the default backend.** Delivered−normalized deltas: v3 −1.6pts corpus-wide, −14.8pts on jargon; whisper models mostly unchanged (the prompt already helps them). This quantifies #271's claim that post-ASR correction is the only lever for prompt-deaf backends.
5. **Cold-start cost is ~12–14s of shared Metal/ANE init**, not any model's property. Now measured separately — it *is* the true first-launch experience.

## What the benchmark can now discern (and couldn't before)

- **Recognition vs formatting vs correction** — three scored tiers per fixture/model separate "the model misheard" from "the style differs" from "the pipeline repaired it".
- **Model ranking with headroom** — jargon (3.7–22.2% spread) and numbers fixtures discriminate top models that were previously indistinguishable at 0.0%.
- **Long-form completeness** — xxlong catches any recurrence of tail truncation.
- **Speed per audio-second** (RTF) comparable across corpus mixes, with honest tie treatment.
- **Preset-dependence is visible, not hidden**: Thorough's different mix flips mostAccurate to medium.en and balanced to v3 — recommendations are corpus-relative, and the per-fixture table now shows why.

## Known limits and follow-ups (honest caveats)

1. **`numbers` fixture: uniform 40–46% WER across all seven models.** Error mass this uniform across wildly different architectures is a reference/normalizer artifact, not recognition: the normalizer doesn't yet fold spoken decimals/IPs/versions ("two point seventeen point two" vs "2.17.2", "one twenty seven dot zero…" vs "127.0.0.1"). The fixture currently measures the ITN gap, which is interesting but should be scored as such, not as WER. → extend `normalized_words` with decimal/dot folding, or split an explicit ITN metric.
2. **`disfluent` and `fast` are saturated (0% everywhere)** — macOS `say` is unnaturally clean. Real human recordings needed (George: `bench/make_audio.sh` documents the format; replace the WAVs, keep the .txt references).
3. **Whisper model-name lists are duplicated** in `whisper_initial_prompt`, `WHISPER_SIZE_ORDER`, and the catalog — a new whisper model needs three edits; should derive from the catalog's `backend` field.
4. **Delivered tier uses the built-in dev dictionary as active** (a strict out-of-the-box default has `code_vocab_enabled=false`, which would make the tier a no-op). Deliberate, documented in code. The product question — ship correction terms on by default — is #271's point 3, still open.
5. `memoryDeltaMb` remains a sequential-process RSS delta (allocator retention skews later baselines) — now documented on the field and in the UI tooltip, not yet re-architected.
6. Benchmark measures whisper's **batch** path, which is also the authoritative production path after #279 removed incremental transcription. The benchmark-vs-streaming gap described by #275 is therefore obsolete.
