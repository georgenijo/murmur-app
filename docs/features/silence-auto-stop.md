# Stop on Silence

## Overview

A toggle-started recording runs until you stop it. That is the right default when you are steering the recording, and the wrong one when your hands are somewhere else — you finish a sentence, walk to the whiteboard, and the recorder is still running.

Stop on Silence ends a hands-free recording after a chosen run of quiet. It is off by default, and when it is on it is designed so that its worst failure mode is *doing nothing*.

## Where it applies

**Any recording that was not started by holding the trigger key.** That is:

- **Double-Tap** — every recording.
- **Both** — double-tap-started recordings auto-stop; hold-started ones end on release, so both gestures keep their natural meaning.
- **Hold Down** — recordings started from the main-window button, the overlay click, and locked mode. Those are toggle-started and otherwise only end on another click.

The one exclusion is a recording in flight while the trigger key is physically held: there the key release owns the stop, and ending a recording while you are still pressing the button that means "I'm still going" would be wrong.

How the hook knows: `useRecordingOrigin` tracks the origin from the keyboard events. `hold-down-start` marks the in-flight recording `'hold'`, and so does `hold-over-recording-start`: in Both mode, holding the trigger during a recording the hold did not start (Shift held for Cmd+Shift+4 during a double-tap recording) starts nothing but is still a physical hold. `hold-over-recording-end` (a combo key ended that hold, recording continues), `hold-down-stop` and `double-tap-toggle` reset to `'toggle'`, which is also the default — button, overlay, and locked-mode starts emit no keyboard event at all. (`hold-down-cancel` is handled defensively too, though the backend's deferred-hold design currently never emits it.) While the origin is `'hold'` the detector ignores samples entirely, so nothing is ever accumulated that could fire in the moments around the key release.

The origin is per-recording, not just per-event: it resets whenever status leaves `'recording'`. A `hold-down-stop` can go missing — Escape cancels a hold recording while suppressing the release's stop event, and a dead rdev thread mid-hold loses the release entirely — and without the status-tied reset a stranded `'hold'` would silently disable auto-stop for every later toggle-started recording. The reset fires only on the transition *out of* recording, so a transition between non-recording states can never wipe a freshly latched hold before its recording starts.

Stopping manually always still works. Auto-stop is an extra way for a recording to end, never the only one.

## How it decides

The detector consumes the same `audio-level` RMS samples the overlay waveform already listens to (emitted from the capture stream at ~60 fps). No extra audio path, no extra permission, nothing new on disk.

All of the logic is a pure fold in `app/src/lib/silenceAutoStop.ts`:

```text
reduceSilenceSample(state, { level, atMs }, silenceMsToStop) -> { state, stop }
```

Two rules keep it from cutting anyone off:

1. **It must hear speech first.** Silence does not accumulate until `MIN_SPEECH_MS` (400 ms) of above-threshold audio has been heard. Speech is *cumulative*, not continuous, so ordinary pauses while thinking still count toward arming. A recording started before you are ready never stops itself.
2. **The threshold only ever rises above an absolute floor.** It is `max(0.015, peak × 0.08)` for the loudest sample seen so far in that recording. On a quiet microphone the detector simply never arms, and the feature degrades to today's behavior instead of stopping early. On a loud one, steady room tone that clears the absolute floor still counts as silence relative to the speaker.

Timing is charged per sample from caller-supplied timestamps, clamped to `[0, 500 ms]` per sample. A backwards clock contributes nothing, and a stalled or suspended stream cannot bank two minutes of "silence" in a single late sample.

The stop is **latched**: it is reported exactly once per recording. Each recording starts from a fresh state, so a previous latch never leaks into the next one.

## Settings

`autoStopSilenceMs: number` — one of `0` (Off), `1500`, `2500`, `4000`. Persisted in `localStorage` with the rest of the settings.

Because this value can end a recording on its own, `loadSettings` accepts **only** those exact values. Anything unrecognised, non-numeric, negative, fractional, or absent (older settings blobs) coerces back to Off rather than to some arbitrary duration.

The control is locked while a recording is in flight, like the sibling Recording Trigger and Double-Tap Key controls — the detector reads the value live, so changing it mid-recording would retune the recording already running.

The setting is purely frontend — nothing is pushed to Rust, and the recording still stops through the same `stop_native_recording` path a tap would use.

## Diagnostics

When it fires, the hook logs a content-free line to the `recording` stream:

```text
silence auto-stop fired { silenceMs, speechMs }
```

Visible in the Performance workspace's Events tab like any other recording event.

## Smart Auto-Stop (Experimental)

`smartAutoStopEnabled: boolean`, off by default. It shortens the wait at the end of a turn; it never extends it.

At each pause of a recording Stop on Silence is watching, once the pause has lasted `SMART_TURN_CHECK_MS` (300 ms), the hook calls `check_smart_turn` — once per pause, never two checks at once. Rust peeks the last 8 s of the recording at 16 kHz (a copy taken through the audio supervisor; capture keeps running and the final stop still owns the buffer), computes Whisper log-mel features, and runs Pipecat's [Smart Turn v3.2](https://huggingface.co/pipecat-ai/smart-turn-v3) on a Tokio blocking worker with a single-threaded ONNX session. A probability at or over `SMART_TURN_THRESHOLD` (0.7) stops the recording through the same `onAutoStop` path and latches the detector, so the silence timer cannot fire a second stop.

The answer is honored only if the pause it was asked about is still running: speech that resumed, a newer pause, a status change, a hold that began, or a new recording all make it stale. Anything other than a confident score — a lower score, `notRecording`, `tooShort`, or `unavailable` (model not downloaded, a check already running, a load or inference failure) — leaves the recording to the silence timer, which stays the hard ceiling. With Stop on Silence Off the control is disabled and the hook never asks.

The 300 ms check point and 0.7 threshold are Hex's tuned values. Smart Turn is trained on human speech; synthetic `say` voices are well outside its distribution, so the `#[ignore]` real-model test (`cargo test --lib smart_turn::tests::real_model -- --ignored --nocapture`) asserts runtime parity with the Python reference and only reports content-free scores for synthetic mid-sentence pauses and turn ends.

**Model.** `smart-turn-v3.2-cpu.onnx`, 8,679,182 bytes, pinned to Hugging Face revision `f766f81d` and SHA-256 `2bb02631…967e4f`. The download is user-initiated from Settings → Recording, streamed with an exact size cap and hash, fsynced, and published atomically under `~/Library/Application Support/local-dictation/models/smart-turn/<sha256>/`. The full hash is re-verified on every load. Remove drops the loaded session first. The model is BSD-2-Clause (Copyright (c) 2024–2025, Daily); see `THIRD_PARTY_NOTICES.md`.

**Runtime.** No second ONNX Runtime. sherpa-onnx already links ONNX Runtime 1.24 statically for CPU Parakeet; `ort` is built with `alternative-backend`, which links nothing, and `smart_turn.rs` hands it that runtime's C API (`OrtGetApiBase` → API version 24) once per process. The feature extractor and session setup are adapted from Hex (MIT).

**Diagnostics.** Rust logs `smart turn check { recording_id, probability, inference_ms, loaded_model }` or `smart turn check unavailable { outcome }` to the `recording` stream; when it fires, the hook logs `smart auto-stop fired { probability, inferenceMs, silenceMs, speechMs }`. No audio or transcript is logged or stored.

## Files

| File | Role |
|------|------|
| `app/src/lib/silenceAutoStop.ts` | Thresholds, state, the pause counter, and the pure per-sample fold |
| `app/src/lib/smartTurn.ts` | Smart Auto-Stop check point, threshold, command wrappers, and pure check/verdict decisions |
| `app/src/components/settings/SmartAutoStopSettings.tsx` | Smart Auto-Stop toggle and model download/remove row |
| `app/src-tauri/src/smart_turn.rs` | Log-mel features, the shared ONNX Runtime hookup, single-flight scoring, model pins |
| `app/src-tauri/src/commands/smart_turn.rs` | Model status/download/remove and `check_smart_turn` |
| `app/src/lib/hooks/useSilenceAutoStop.ts` | Subscription, per-recording reset, the hold-origin gate, single call out |
| `app/src/lib/hooks/useRecordingOrigin.ts` | Tracks whether the in-flight recording is hold-started |
| `app/src/lib/settings.ts` | `autoStopSilenceMs`, its option list, and its validation |
| `app/src/components/settings/SettingsPanel.tsx` | The Recording page control |
| `app/src/App.tsx` | Arms the hook and points it at `handleStop` |

## Tests

- `app/src/lib/silenceAutoStop.test.ts` — arming, cumulative speech, the exact stop point, latching, silence reset on resumed speech, the disabled path, the sample-gap cap, backwards timestamps, non-finite levels, a microphone that never clears the floor, and room tone under a loud peak.
- `app/src/lib/hooks/useSilenceAutoStop.test.tsx` — fires once, never on a silent recording, ignores levels while disabled/off/not recording, resets per recording, never fires for a hold-started recording, re-arms after a hold ends, survives the Both-mode tap-then-cancel sequence, and unsubscribes on unmount.
- `app/src/lib/settings.test.ts` — the duration allow-list and its coercions; Smart Auto-Stop stays off unless explicitly `true`.
- `app/src/lib/smartTurn.test.ts` — one pause per silence run, the check point, once-per-pause, the threshold edge, and stale answers.
- `app/src/lib/hooks/useSilenceAutoStop.test.tsx` (Smart Auto-Stop) — early stop on a confident score, a low score leaving the ceiling in charge, re-asking after speech resumes, answers dropped after resumed speech or a new recording, fail-open on unavailable/rejected checks, and no checks while off, with Stop on Silence Off, or for a held recording.
- `app/src-tauri/src/smart_turn.rs` — log-mel parity with `WhisperFeatureExtractor`, non-finite input, window tail, load-once, fail-closed load/inference, busy instead of waiting; plus the ignored real-model test.
