# Meeting echo cancellation: offline evaluation

Status: evidence for [#594](https://github.com/georgenijo/murmur-app/issues/594).
It changes no runtime behaviour. The opt-in AEC3 path in
[Meeting Capture](meeting-capture.md) stays off by default.

## Decision

**No-go for enabling the shipped AEC3 configuration by default.** The
measured next experiment is to cut double-talk suppression and run at the ASR
rate.

- **Echo removal is not the problem.** On synthetic, linear echo paths AEC3
  removes 27–44 dB once it has converged. That holds for 0–400 ms bulk delay,
  up to 100 ppm clock drift, and RT60 up to 0.5 s, at about 14 ms of CPU per
  second of audio on an Apple M4, which is 1.4% of one core.
- **Near-end preservation fails.** When the user talks over playback, AEC3's
  suppressor turns the user down by 4–11 dB on average. In frames where echo
  is present, 35–86% of the user's speech ends up more than 10 dB below its
  true level. The issue forbids exactly this kind of muting of genuine
  near-end speech. When the user talks alone (no echo in the frame), only
  3–6% of frames are muted.
- **The 48 kHz path changes the user's waveform even with no echo.** With a
  silent reference, the shipped 48 kHz path scores 14.5 dB SI-SDR in the ASR
  band. The same configuration run at 16 kHz scores 26.6 dB. Neither meets
  the issue's 1 dB loss gate (the raw microphone scores 48.1 dB on that
  fixture). With AEC disabled, the processor is bit-transparent.

### Gates from #594 on these fixtures

| Gate | Result | Notes |
|---|---|---|
| Far-end ERLE ≥ 18 dB, moderate volume | passes on synthetic audio (36.6 dB) | Linear synthetic echo is optimistic. It does not replace room recordings. |
| Near-end single-talk SI-SDR loss ≤ 1 dB | **fails**: loss of 33.6 dB at 48 kHz and 21.5 dB at 16 kHz; at a −50 dBFS noise floor, 13.8 dB and 3.8 dB | A strict waveform metric. ASR impact still needs a WER test. |
| No muting of genuine near-end speech | **fails** in double-talk | 30–73% of near-end frames (35–86% of the frames with echo) |
| Double-talk WER regression ≤ 5% | not measured | Synthetic talkers are not speech, so there is no WER. It needs consented recordings. |
| Delay convergence | 0–400 ms works; 600 ms and above fails (≈1 dB) | The production timeline caps render lead at 250 ms, inside the working range. |
| Reference arriving after the echo | loses about 12 dB (24.6 vs 36.6) | Alignment must keep the reference ahead of the echo. |
| Clock drift | 20–100 ppm: no loss; 1000 ppm: worst second drops to 14 dB at 48 kHz (23 dB at 16 kHz) | |
| CPU | ≈14 ms per audio second (48 kHz and 16 kHz alike) | Single-thread, release, Apple M4. Resampling cost not included at 16 kHz. |

### Next experiment

1. Run AEC3 at 16 kHz in the worker. Meeting ASR already consumes 16 kHz.
   ERLE is the same as at 48 kHz, and near-end waveform fidelity is 12 dB
   better.
2. Stop the double-talk ducking. Use the crate's `experimental-aec3-config`
   API to try a less aggressive suppressor (dominant near-end detection,
   near-end tuning) or the linear-only AEC output. ASR tolerates residual echo
   better than deleted speech. That feature changes the native build, so it
   needs its own spike.
3. Only then run consented room recordings and the WER comparison on the
   trusted test Mac, per the matrix in #594.

## What was measured

The evaluation is a test-only module,
[`sidecars/capture/src/aec/eval.rs`](../../app/src-tauri/sidecars/capture/src/aec/eval.rs),
compiled only under `cfg(test)`. It opens no audio device, plays nothing, and
reads no private audio.

- **Fixtures.** The fixtures are generated deterministically from fixed seeds
  at 48 kHz. Two synthetic talkers (pitched glottal pulses through moving
  formants, fricatives, syllable and phrase timing) stand in for far-end and
  near-end speech. The echo path is: optional speaker overdrive (tanh), then
  clock drift (resampling), then bulk delay, then a sparse room impulse
  response (direct path, early reflections, exponential tail). A noise floor
  is added. In double-talk fixtures the near-end talker joins after 3 s of
  far-end-only audio.
- **Candidates.**
  - The shipped AEC3 configuration at 48 kHz, built with the worker's own
    constructor. A test checks that it is bit-identical to
    `MeetingMicrophonePath`.
  - The same configuration on 16 kHz streams.
  - A time-domain NLMS reference given the true delay.
- **Scoring.** Every candidate is scored in the 16 kHz ASR band (0–7 kHz) with
  its fixed latency removed. AEC3 adds 430 samples (9.0 ms) at 48 kHz and 128
  samples (8.0 ms) at 16 kHz.
  - ERLE is measured over echo-active 10 ms frames after 2 s of convergence.
  - Near-end SI-SDR and gain are measured against the clean near-end
    component.
  - A near-end frame counts as "muted" when the output is more than 10 dB
    below the near-end component in that frame.
- **Not measured.** The AEC3 delay statistic (`get_stats().delay_ms`) read a
  constant 16 ms whatever the true delay, so it is not reported. The spike
  does not cover real rooms, nonlinear speaker distortion beyond tanh
  clipping, Bluetooth routes, ASR word error rate, or signed-worker RSS.

### Reproduce

```bash
cd app/src-tauri
# Always-on checks (about 4 s):
cargo nextest run -p murmur-capture-helper --bin murmur-capture-helper aec::eval
# Full matrix, printed as Markdown tables (about 30 s in release):
cargo test --release -p murmur-capture-helper --bin murmur-capture-helper \
  aec::eval::feasibility_report -- --ignored --nocapture
# Optionally also write each fixture and output as 16 kHz WAVs to a local dir:
MURMUR_AEC_EVAL_WAV_DIR=/tmp/murmur-aec-eval cargo test --release \
  -p murmur-capture-helper --bin murmur-capture-helper \
  aec::eval::feasibility_report -- --ignored --nocapture
```

## Results (release, Apple M4)

Columns:

- **ERLE** is in steady state.
- **s1–s4** is ERLE in each of the first four seconds.
- **worst 1 s** is the lowest one-second ERLE after the first two seconds.
- **Muted** is the share of near-end frames more than 10 dB below the
  near-end component: all frames, then frames with echo, then frames without.
- **CPU** is milliseconds per audio second.

### AEC3 @ 48 kHz (shipped config)

| scenario | ERLE dB | ERLE s1/s2/s3/s4 | worst 1 s ERLE | near SI-SDR raw → out dB | near gain dB | muted: all / with echo / alone | CPU ms / audio s |
|---|---|---|---|---|---|---|---|
| far, moderate (ERL 10 dB) | 36.6 | 26/21/27/44 | 27.5 | – | – | – / – / – | 14.00 |
| far, low (ERL 20 dB) | 30.5 | 20/17/22/35 | 21.7 | – | – | – / – / – | 13.94 |
| far, loud (ERL 0 dB) | 42.7 | 32/25/34/54 | 33.7 | – | – | – / – / – | 14.38 |
| far, loud + speaker clipping | 34.4 | 25/22/30/53 | 29.6 | – | – | – / – / – | 14.16 |
| delay 0 ms | 29.7 | 20/16/21/30 | 20.7 | – | – | – / – / – | 13.74 |
| delay 10 ms | 37.2 | 25/23/28/46 | 28.0 | – | – | – / – / – | 15.47 |
| delay 100 ms | 36.3 | 25/22/27/43 | 27.1 | – | – | – / – / – | 14.56 |
| delay 200 ms | 37.5 | 26/23/29/43 | 28.7 | – | – | – / – / – | 13.93 |
| delay 400 ms | 44.3 | 42/40/38/42 | 38.1 | – | – | – / – / – | 14.08 |
| delay 600 ms | 1.1 | 44/7/1/1 | 0.5 | – | – | – / – / – | 13.95 |
| delay 800 ms | 0.9 | 44/5/1/1 | 0.5 | – | – | – / – / – | 13.98 |
| reference 20 ms late | 24.6 | 19/15/17/25 | 16.8 | – | – | – / – / – | 13.70 |
| room RT60 0.1 s | 36.9 | 27/25/28/44 | 27.9 | – | – | – / – / – | 13.92 |
| room RT60 0.5 s | 29.8 | 20/15/24/43 | 23.6 | – | – | – / – / – | 13.93 |
| room RT60 0.8 s | 24.5 | 13/10/19/33 | 15.8 | – | – | – / – / – | 15.61 |
| drift 20 ppm, 30 s | 40.9 | 25/22/27/44 | 27.0 | – | – | – / – / – | 14.05 |
| drift 100 ppm, 30 s | 40.0 | 25/22/27/44 | 26.9 | – | – | – / – / – | 13.82 |
| drift 1000 ppm, 30 s | 31.8 | 21/17/23/34 | 14.3 | – | – | – / – / – | 13.78 |
| noise floor −50 dBFS | 27.0 | 23/19/25/25 | 23.1 | – | – | – / – / – | 14.30 |
| near only, render silent | – | – | – | 48.1 → 14.5 | −0.6 | 0% / – / 0% | 9.58 |
| near only, noise −50 dBFS | – | – | – | 28.1 → 14.3 | −0.6 | 0% / – / 0% | 9.17 |
| double-talk, moderate | – | – | – | 4.0 → 2.7 | −4.5 | 30% / 35% / 5% | 13.85 |
| double-talk, loud echo (ERL 0) | – | – | – | −4.8 → −3.1 | −11.0 | 73% / 86% / 3% | 13.71 |
| double-talk, quiet near (-36 dBFS) | – | – | – | −4.6 → −3.2 | −11.1 | 71% / 85% / 6% | 14.24 |

### AEC3 @ 16 kHz (same config)

| scenario | ERLE dB | ERLE s1/s2/s3/s4 | worst 1 s ERLE | near SI-SDR raw → out dB | near gain dB | muted: all / with echo / alone | CPU ms / audio s |
|---|---|---|---|---|---|---|---|
| far, moderate (ERL 10 dB) | 36.5 | 26/21/27/44 | 27.4 | – | – | – / – / – | 13.53 |
| far, low (ERL 20 dB) | 30.4 | 20/17/22/35 | 21.6 | – | – | – / – / – | 13.33 |
| far, loud (ERL 0 dB) | 42.8 | 32/29/33/54 | 33.5 | – | – | – / – / – | 14.40 |
| far, loud + speaker clipping | 34.8 | 27/25/27/44 | 27.3 | – | – | – / – / – | 14.11 |
| delay 0 ms | 28.6 | 19/17/20/30 | 20.2 | – | – | – / – / – | 13.60 |
| delay 10 ms | 37.1 | 25/23/28/46 | 27.9 | – | – | – / – / – | 13.83 |
| delay 100 ms | 36.5 | 25/23/27/43 | 27.3 | – | – | – / – / – | 13.77 |
| delay 200 ms | 37.4 | 26/25/29/43 | 28.5 | – | – | – / – / – | 13.48 |
| delay 400 ms | 46.1 | 42/39/42/42 | 41.7 | – | – | – / – / – | 13.57 |
| delay 600 ms | 0.9 | 44/4/1/1 | 0.0 | – | – | – / – / – | 13.59 |
| delay 800 ms | 0.5 | 44/5/1/0 | 0.0 | – | – | – / – / – | 13.53 |
| reference 20 ms late | 25.3 | 19/16/18/24 | 18.0 | – | – | – / – / – | 13.41 |
| room RT60 0.1 s | 36.9 | 27/25/28/44 | 27.9 | – | – | – / – / – | 13.46 |
| room RT60 0.5 s | 28.8 | 21/18/24/43 | 24.1 | – | – | – / – / – | 14.56 |
| room RT60 0.8 s | 21.3 | 17/13/20/42 | 12.5 | – | – | – / – / – | 13.44 |
| drift 20 ppm, 30 s | 40.9 | 25/23/27/44 | 27.0 | – | – | – / – / – | 13.92 |
| drift 100 ppm, 30 s | 40.4 | 25/22/27/43 | 27.0 | – | – | – / – / – | 14.70 |
| drift 1000 ppm, 30 s | 31.5 | 23/18/23/30 | 22.9 | – | – | – / – / – | 13.53 |
| noise floor −50 dBFS | 27.0 | 23/20/25/25 | 23.1 | – | – | – / – / – | 13.54 |
| near only, render silent | – | – | – | 48.1 → 26.6 | −0.0 | 0% / – / 0% | 8.77 |
| near only, noise −50 dBFS | – | – | – | 28.1 → 24.3 | −0.0 | 0% / – / 0% | 8.68 |
| double-talk, moderate | – | – | – | 4.0 → 3.5 | −3.7 | 30% / 35% / 4% | 13.82 |
| double-talk, loud echo (ERL 0) | – | – | – | −4.8 → −2.7 | −10.4 | 72% / 85% / 3% | 13.40 |
| double-talk, quiet near (-36 dBFS) | – | – | – | −4.6 → −2.7 | −10.3 | 70% / 83% / 6% | 14.20 |

### NLMS @ 16 kHz, oracle delay, 2048 taps

| scenario | ERLE dB | ERLE s1/s2/s3/s4 | worst 1 s ERLE | near SI-SDR raw → out dB | near gain dB | muted: all / with echo / alone | CPU ms / audio s |
|---|---|---|---|---|---|---|---|
| far, moderate (ERL 10 dB) | 23.8 | 16/14/26/19 | 19.0 | – | – | – / – / – | 29.18 |
| far, loud + speaker clipping | 12.5 | 13/11/14/11 | 10.1 | – | – | – / – / – | 32.30 |
| room RT60 0.5 s | 8.9 | 14/12/22/5 | 4.5 | – | – | – / – / – | 29.77 |
| drift 100 ppm, 30 s | 12.9 | 15/10/18/11 | 4.0 | – | – | – / – / – | 30.04 |
| drift 1000 ppm, 30 s | 10.2 | 11/8/15/10 | 4.8 | – | – | – / – / – | 31.77 |
| double-talk, moderate | – | – | – | 4.0 → −11.5 | −7.3 | 2% / 3% / 0% | 29.95 |

Even when given the true delay, a plain linear filter removes much less echo.
With no double-talk control it diverges and corrupts the near-end. That backs
keeping AEC3 and changing its suppressor over writing a custom canceller.
