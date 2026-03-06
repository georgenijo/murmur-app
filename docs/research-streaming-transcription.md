# Research: Streaming/Chunked Whisper Transcription

**Date:** 2026-03-04
**Status:** Research complete — not yet implemented

## Context

Murmur currently records audio fully, then runs whisper.cpp inference on the complete buffer after the user stops. For a 45-second recording with `base.en` on Metal GPU, inference takes 2-3 seconds. With `large-v3-turbo`, it could be 10-20+ seconds.

This document explores **processing audio incrementally during recording** so that when the user stops, most of the transcription is already done.

---

## 1. Feasibility Verdict

**Conditionally feasible. Confidence: High.**

whisper.cpp's "streaming" is not truly incremental — it re-runs the full encoder+decoder pipeline on each chunk. This is a fundamental Whisper architecture limitation, not a whisper.cpp gap. Despite this, the approach works well in practice: whisper.cpp's `stream` example, UFAL's `whisper_streaming`, and several production apps ship with it.

**Conditions:**

- **Model size matters.** `tiny.en` and `base.en` are comfortably real-time for 5s chunks on any Apple Silicon (~50-250ms inference). `small.en` is viable with careful buffering (~400-700ms). `large-v3-turbo` is marginal (~800-1500ms) and may not keep up with a 3s step interval on M1.
- **whisper-rs 0.15 exposes everything we need.** `set_no_context()`, `set_audio_ctx()`, `set_offset_ms()`, `set_single_segment()`, `set_initial_prompt()` — all available.
- **The `backend` mutex is the key architectural bottleneck.** Currently held for the entire duration of inference. Must be reworked for concurrent capture + inference.

---

## 2. How whisper.cpp Streaming Works

whisper.cpp's `stream` example (`examples/stream/stream.cpp`) implements real-time transcription via a **sliding window** over a live audio capture buffer. It does NOT use true incremental/stateful inference — it re-runs the full `whisper_full()` pipeline on each window.

### Three key timing parameters

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `step_ms` | 3000ms | How often new audio is processed |
| `length_ms` | 10000ms | Total audio window fed to encoder each iteration |
| `keep_ms` | 200ms | Audio overlap retained from previous chunk |

### How it works

1. Audio is continuously captured into a ring buffer.
2. Every `step_ms`, the system extracts the latest `length_ms` of audio.
3. From the previous iteration, `keep_ms` of trailing samples are prepended (overlap).
4. The combined window goes through the full whisper pipeline: PCM → mel → encoder → decoder → text.
5. Output text replaces (not appends to) the previous chunk's output.

### Two operating modes

- **Fixed-step mode** (`step_ms > 0`): Processes at regular intervals. Simpler but wastes computation on silence.
- **VAD mode** (`step_ms <= 0`): Only triggers inference when voice activity is detected. More efficient.

### Key limitation

Each `whisper_full()` call is independent. There is no way to feed new audio into an already-running encoder or extend a previous encoder output. The encoder always expects 30 seconds of input (padded with silence if shorter).

---

## 3. Recommended Approach

### Hybrid streaming with full-buffer final pass

Show interim results during recording for responsive UX, but re-transcribe the entire recording when the user stops. The final full-buffer transcription goes to clipboard. This sidesteps most chunked-Whisper quality pitfalls while giving real-time feedback.

### Chunking parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Audio window | 5-10s | Below 5s, quality degrades. 10s is whisper.cpp's default. |
| Step interval | 3s | whisper.cpp default. Balances responsiveness vs compute. |
| Overlap (keep_ms) | 200-500ms | Prevents word-splitting at boundaries. |
| `audio_ctx` | 768 | Halves encoder cost (~2x speedup). Floor before decoder instability. |
| `no_context` | false | Pass previous tokens as prompt for linguistic continuity. |
| `single_segment` | true | Simplifies output handling per chunk. |

### Deduplication: LocalAgreement-2

From UFAL's `whisper_streaming` research ([paper](https://arxiv.org/html/2307.14743)):

1. Maintain `confirmed_text` (emitted, won't change) and `unconfirmed_text` (latest hypothesis, may change).
2. On each new chunk, compute longest common prefix (LCP) between previous `unconfirmed_text` and new output.
3. LCP becomes newly confirmed text — append to `confirmed_text`.
4. Remainder becomes new `unconfirmed_text`.
5. For overlap dedup: match n-grams (n=1..5) within 1s timestamp window against suffix of `confirmed_text`.

This naturally filters hallucinations — if chunk N hallucinates something chunk N+1 doesn't reproduce, it never reaches the user. ~2 chunk latency tradeoff (~6s before text is confirmed).

### VAD-gated chunking

Use Silero VAD on small windows (~500ms) during recording to detect speech pauses. Trigger chunk inference on pause boundaries rather than fixed intervals, producing more natural utterance-aligned chunks. Fall back to fixed-interval if no pause detected within max window (10s).

---

## 4. API Surface

### whisper-rs `FullParams` setters we'd use

```
set_no_context(false)          // carry previous tokens as prompt
set_audio_ctx(768)             // halve encoder cost
set_single_segment(true)       // one segment per chunk
set_initial_prompt(&str)       // custom vocabulary + previous confirmed text
set_suppress_blank(true)       // already set
```

### whisper-rs `WhisperState` methods

```
full(params, &[f32])           // main inference call (same as today, per chunk)
```

### whisper-rs `WhisperContext` methods

```
create_state()                 // create additional states for concurrent inference (optional)
```

The existing `state.full(params, samples)` call is reused per-chunk. No new low-level API needed — the stream approach is "call `full()` repeatedly on overlapping windows."

All parameters are exposed by whisper-rs 0.15 (our current version).

---

## 5. Current Pipeline Analysis

### Flow: user presses record → text appears

1. **Start**: `start_native_recording` sets status to `Recording`, calls `audio::start_recording()` which spawns a dedicated audio thread. A cpal callback on a real-time thread appends mono `f32` samples to `Arc<Mutex<Vec<f32>>>`.
2. **During recording**: Samples accumulate in the shared Vec. No inference occurs.
3. **Stop**: `stop_native_recording` sets `active = false`, sends `AudioCommand::Stop`, joins the audio thread. `std::mem::take()` moves the Vec out (zero-copy).
4. **Pipeline**: VAD filters silence (`spawn_blocking`, clones samples). Whisper inference runs via `state.full(params, samples)`. Text injected to clipboard via `run_on_main_thread`.
5. **Done**: `transcription-complete` event fires. `IdleGuard` resets status to Idle.

### Threading model

| Thread | Role |
|--------|------|
| Tokio async runtime | Runs Tauri commands |
| Audio thread (`std::thread::spawn`) | Owns cpal stream, blocks on stop command |
| cpal callback thread (CoreAudio) | Real-time audio callback, appends to buffer |
| `spawn_blocking` thread | VAD inference (`WhisperVadContext` is `!Send`) |
| Main thread | Text injection (`inject_text` via `run_on_main_thread`) |

### Mutex map

| Mutex | Held During | Contention Risk |
|-------|-------------|-----------------|
| `app_state.dictation` | Status transitions, reading settings (microseconds) | Low |
| `app_state.backend` | **Entire whisper model load + inference (seconds)** | **HIGH** |
| `RECORDING_STATE` | Start/stop recording operations | Low |
| Inner buffer `Arc<Mutex<Vec<f32>>>` | cpal callback appends (frequent, short); stop_recording takes (once) | Moderate |

No deadlock risk — code never holds `dictation` and `backend` simultaneously.

### Buffer architecture

- `Arc<Mutex<Vec<f32>>>` — contiguous, grows via `Vec::extend()`, not a ring buffer.
- Fresh allocation per recording session.
- Ownership transferred out via `std::mem::take()` at stop time.
- VAD clones the full buffer (`samples.to_vec()`). Whisper takes `&[f32]` reference.

---

## 6. Architecture Delta

### New components

| Component | Purpose |
|-----------|---------|
| **ChunkScheduler** | Background task during recording. Snapshots audio, runs VAD, submits chunks for inference, emits partial results. |
| **StreamReconciler** | LocalAgreement-2 implementation. Holds `confirmed_words` and `prev_unconfirmed_words`. Returns newly confirmed + current unconfirmed text per chunk. |
| **Concurrent audio buffer** | Replace `Arc<Mutex<Vec<f32>>>` with structure supporting non-destructive reads while cpal continues writing. |

### File-by-file changes

| File | Change |
|------|--------|
| `audio.rs` | Add `snapshot_since(offset) -> Vec<f32>` for non-destructive reads during recording. Consider lock-free ring buffer or `crossbeam` channel to reduce cpal callback contention. |
| `commands/recording.rs` | `start_native_recording` spawns ChunkScheduler. `stop_native_recording` signals scheduler to flush, waits for completion, runs full-buffer final pass. New `transcription-partial` event emission. |
| `transcriber/whisper.rs` | New `transcribe_chunk()` method with streaming-specific params (`no_context=false`, `audio_ctx=768`, `initial_prompt` set to previous confirmed text). Existing `transcribe()` unchanged for final pass. |
| `transcriber/mod.rs` | Extend `TranscriptionBackend` trait with `transcribe_chunk()`. |
| `vad.rs` | Add incremental VAD: run on ~500ms windows to detect pause boundaries. Cache `WhisperVadContext` for session lifetime (not recreated each time). Must live on dedicated thread (`!Send`). |
| `state.rs` | Add chunk scheduler handle to `DictationState`. |
| `lib.rs` | Register new event types. Minimal. |
| `useRecordingState.ts` | Listen for `transcription-partial` events. Maintain `confirmedText` + `unconfirmedText` state. |
| `App.tsx` / new component | Live preview area above history: confirmed text (normal color) + unconfirmed text (`text-stone-400`). Disappears on completion. |
| `OverlayWidget.tsx` | Optional: brief green pulse on waveform when a chunk confirms. No text in overlay (too small). |

### What stays the same

- **Clipboard**: Write once after final pass. `transcription-complete` fires once.
- **Auto-paste**: Unchanged, fires after final text.
- **History entries**: Created from final text only.
- **Overlay states**: `idle` / `recording` / `processing` — frontend doesn't see streaming sub-states.
- **Short recordings** (< 1 chunk): Fall through to batch mode, identical to current behavior.

---

## 7. UX Design

### Partial results display

**Main window only, not overlay.** The Dynamic Island overlay is physically constrained (~37px height). It should continue showing status indicators only.

A transient "live preview" section appears above the history list during recording:

- **Confirmed text**: Normal font weight, normal color (`text-stone-900` / `text-stone-100`).
- **Unconfirmed text**: Lighter color (`text-stone-400` / `text-stone-500`). Not italic — color alone communicates uncertainty.
- **Transition**: When unconfirmed → confirmed, color transitions over ~150ms ("solidifying" effect).
- **Live indicator**: Pulsing dot at end of unconfirmed text. Disappears on completion.

### Clipboard and paste strategy

**Do NOT update clipboard during streaming.** User might Cmd+V mid-stream and paste incomplete text. Auto-paste with mid-stream writes would fire multiple paste events. Clipboard write happens exactly once, after the final full-buffer pass.

### Stop-recording edge cases

| Scenario | Handling |
|----------|----------|
| Final chunk ≥ 0.3s | Process normally (existing `MIN_RECORDING_SAMPLES` threshold) |
| Final chunk < 0.3s | Discard (likely silence or keystroke artifact) |
| Final chunk 0.3-1.0s | VAD first; if no speech, discard |
| On stop | Force-confirm all remaining unconfirmed text (no next chunk to agree with) |

### State machine

Frontend sees the same three statuses plus a new event:

| Event | Payload | Effect |
|-------|---------|--------|
| `recording-status-changed` | `"recording"` | Show waveform, start timer |
| `transcription-partial` | `{ confirmed, unconfirmed, chunk_index }` | Update live preview |
| `recording-status-changed` | `"processing"` | Spinner in overlay, "finalizing..." in preview |
| `transcription-complete` | `{ text, duration }` | Remove preview, add to history, write clipboard |

---

## 8. Prior Art

| Project | Approach | Chunk Size | Overlap | Dedup |
|---------|----------|------------|---------|-------|
| whisper.cpp `stream` | Sliding window, fixed step | 10s window, 3s step | 200ms keep | Prompt token context |
| UFAL whisper_streaming | LocalAgreement-2, full buffer re-process | 0.5-2s min chunk, buffer up to 30s | Full buffer | 2-iteration prefix agreement |
| WhisperLive (Collabora) | VAD-gated, multiple backends | VAD-determined | VAD handles | Hallucination filtering |
| Buzz | Fixed time chunks | 10s default | None | None |
| HuggingFace Transformers | Chunked pipeline with stride | 30s default | `chunk_length/6` (~5s) | Longest common sequence |

---

## 9. Latency Benchmarks (Apple Silicon)

### Estimated inference for 5s audio chunks with Metal

| Model | M1 | M2 | Streaming viable? |
|-------|-----|-----|-------------------|
| tiny.en | ~50-100ms | ~40-80ms | Yes, easily |
| base.en | ~120-250ms | ~100-200ms | Yes, comfortable |
| small.en | ~400-700ms | ~350-600ms | Yes, with buffering |
| large-v3-turbo | ~800-1500ms | ~700-1200ms | Marginal; tight for 3s step |

### Full transcription (10s audio, M4, Metal)

| Model | Time | Real-time factor |
|-------|------|-----------------|
| tiny | 0.37s | 27x faster |
| base | 0.54s | 18x faster |
| small | 1.44s | 7x faster |

`audio_ctx=768` approximately halves encoder cost (~2x speedup per chunk).

---

## 10. Risks and Open Questions

### Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Quality degradation vs full-buffer | Medium | Hybrid approach: streaming for preview, full-buffer for final clipboard text |
| Hallucination at chunk boundaries | Medium | LocalAgreement-2 filters hallucinations; VAD pre-filters silence |
| `backend` mutex contention | High | Restructure to release between chunks, or use `create_state()` for separate streaming state |
| cpal callback blocking | Medium | Lock-free buffer or `try_lock` with staging buffer |
| VAD `!Send` constraint | Low | Dedicated thread for VAD during session |
| Model-dependent feasibility | Medium | Auto-fallback to batch mode when inference exceeds step interval |

### Open questions

1. **Should the final pass re-transcribe the full buffer or concatenate confirmed chunks?** Full re-transcription gives higher quality but adds latency at stop time.
2. **Model switching for streaming?** If user selects `large-v3-turbo`, auto-downgrade to `base.en` for chunks, use `large-v3-turbo` for final pass only?
3. **Memory budget.** Full recording buffer for final pass = O(n) growth. 5 min at 16kHz ≈ 9.6MB. Acceptable but worth monitoring (see #125).
4. **Opt-in toggle?** A settings toggle ("Show live preview during recording") lets users who don't want it stick with batch mode.
5. **`audio_ctx=768` with short chunks.** Needs prototyping — 768 frames = ~15s context, so 5-10s chunks should be fine (remainder zero-padded), but needs validation.

---

## 11. Estimated Complexity

**Large.** Touches audio buffering, transcription backend, VAD, state management, recording pipeline, and frontend.

| Phase | Scope | Files |
|-------|-------|-------|
| Phase 1: Streaming infrastructure | ChunkScheduler, concurrent buffer, `transcribe_chunk()`, reconciler | ~3-5 |
| Phase 2: Frontend live preview | New component, `transcription-partial` listener, styling | ~2-3 |
| Phase 3: Hybrid final pass | Full-buffer re-transcription on stop, model-size gating, fallback | ~1-2 |
| Phase 4: Polish | VAD-gated chunking, overlap tuning, hallucination guards, settings toggle | ~2-3 |

**Recommendation:** Prototype Phase 1 in isolation first — get a ChunkScheduler producing partial results on a branch — before committing to the full feature. The key unknowns (mutex restructuring, `audio_ctx` behavior, dedup quality) need hands-on validation.

---

## Sources

- [whisper.cpp stream example](https://github.com/ggml-org/whisper.cpp/blob/master/examples/stream/stream.cpp)
- [whisper.cpp audio_ctx discussion #297](https://github.com/ggml-org/whisper.cpp/discussions/297)
- [whisper.cpp faster streaming #137](https://github.com/ggml-org/whisper.cpp/issues/137)
- [whisper.cpp benchmarks #89](https://github.com/ggml-org/whisper.cpp/issues/89)
- [whisper-rs 0.15 docs](https://docs.rs/whisper-rs/0.15.1/whisper_rs/index.html)
- [UFAL whisper_streaming](https://github.com/ufal/whisper_streaming)
- [Turning Whisper into Real-Time Transcription (paper)](https://arxiv.org/html/2307.14743)
- [UFAL SimulStreaming](https://github.com/ufal/SimulStreaming)
- [Collabora WhisperLive](https://github.com/collabora/WhisperLive)
- [WhisperFlow paper (2024)](https://arxiv.org/abs/2412.11272)
- [Apple Silicon whisper benchmarks (Voicci)](https://www.voicci.com/blog/apple-silicon-whisper-performance.html)
- [mac-whisper-speedtest](https://github.com/anvanvan/mac-whisper-speedtest)
- [M4 whisper benchmarks](https://dev.to/theinsyeds/whisper-speech-recognition-on-mac-m4-performance-analysis-and-benchmarks-2dlp)
