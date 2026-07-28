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

How the hook knows: `useRecordingOrigin` tracks the origin from the keyboard events. `hold-down-start` marks the in-flight recording `'hold'`; `hold-down-stop`, `hold-down-cancel`, and `double-tap-toggle` reset to `'toggle'`, which is also the default — button, overlay, and locked-mode starts emit no keyboard event at all. While the origin is `'hold'` the detector ignores samples entirely, so nothing is ever accumulated that could fire in the moments around the key release, and a cancelled speculative hold in Both mode cannot leave the next toggle-started recording disarmed.

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

Visible in the log viewer's Events tab like any other recording event.

## Files

| File | Role |
|------|------|
| `app/src/lib/silenceAutoStop.ts` | Thresholds, state, and the pure per-sample fold |
| `app/src/lib/hooks/useSilenceAutoStop.ts` | Subscription, per-recording reset, the hold-origin gate, single call out |
| `app/src/lib/hooks/useRecordingOrigin.ts` | Tracks whether the in-flight recording is hold-started |
| `app/src/lib/settings.ts` | `autoStopSilenceMs`, its option list, and its validation |
| `app/src/components/settings/SettingsPanel.tsx` | The Recording page control |
| `app/src/App.tsx` | Arms the hook and points it at `handleStop` |

## Tests

- `app/src/lib/silenceAutoStop.test.ts` — arming, cumulative speech, the exact stop point, latching, silence reset on resumed speech, the disabled path, the sample-gap cap, backwards timestamps, non-finite levels, a microphone that never clears the floor, and room tone under a loud peak.
- `app/src/lib/hooks/useSilenceAutoStop.test.tsx` — fires once, never on a silent recording, ignores levels while disabled/off/not recording, resets per recording, never fires for a hold-started recording, re-arms after a hold ends, survives the Both-mode tap-then-cancel sequence, and unsubscribes on unmount.
- `app/src/lib/settings.test.ts` — the duration allow-list and its coercions.
