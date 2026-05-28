# Investigation: audio ducks when Murmur gains focus (#177)

**Issue:** #177 — "app audio quiets or ducks when Murmur focus changes"
**Resolved by:** #182 (fix + CI guard + QA harness), #181 (test-suite env fix)
**Superseded:** #180 (diagnostics-first approach — closed, did not find the cause)

## TL;DR

Bringing the Murmur window to the foreground quieted/stuttered audio playing in
other apps (e.g. a video in a browser). The cause was **`PermissionsBanner`
calling `navigator.mediaDevices.getUserMedia({ audio: true })` on every window
focus** to check microphone permission. In WKWebView, `getUserMedia` opens the
mic through macOS **voice-processing I/O (VPIO)**, which ducks all other system
audio while active. It fired on every focus — even after permission was granted,
because the component stays mounted and keeps a `focus` listener.

The fix reads microphone authorization status natively
(`AVCaptureDevice.authorizationStatus(for: .audio)`) without ever opening the
device. No mic session → no VPIO → no ducking.

## Symptom

- Audio playing elsewhere (movie/meeting) dropped in volume, frames lagged, then
  recovered — each time Murmur was surfaced; recovered when Murmur lost focus.
- Reproduced with the Anker USB mic and with wired HDMI output, so not specific
  to any device.
- Only Murmur triggered it; surfacing Finder/terminal/other apps did not.

## What made it hard

The first approach (#180) assumed an **audio-routing** cause and added ~900 lines
of instrumentation around device/route snapshots. This was the wrong layer and
actively misleading:

- Every observation was read through an audio-routing lens (instrument bias). A
  `coreaudiod`/`HFPCall` log line looked like a smoking gun and sent the
  investigation down a Bluetooth/HFP path.
- The route snapshot logged the **system-default** device, not the device Murmur
  actually used, and reported the device's **advertised** config, not the live
  stream — so it literally could not observe this bug.
- The route snapshot itself queried CoreAudio on every focus, adding a
  `coreaudiod` CPU burst that was self-inflicted, not the duck.

## How it was actually found

The breakthrough was building an **objective signal** instead of guessing from
CPU/log proxies: a CoreAudio process-tap RMS meter (macOS 14.4+) that taps the
global system **output** and prints its level ~10×/sec. The movie's own audio
level became a number.

With that, the duck was measured directly and correlated to window focus:

| Murmur event | other-audio RMS |
|---|---|
| not focused | loud (≈ −30 dBFS) |
| focus / surface | ducks (≈ −60 to −120 dBFS) |
| blur / leave | recovers (≈ −30 dBFS) |

Recoveries landed on blur, not on scene changes — so it was caused by focus, not
content. The meter then let the rest of the investigation run on numbers instead
of ears.

### Hypotheses tested and ruled out

Each was disproven against the meter and/or by toggling the suspect:

- **Bluetooth / HFP route switch** — ducked on wired HDMI output too; no HFP in
  the unified log.
- **Recording / capture pipeline** — output stayed 48 kHz through record cycles;
  duck happened with no recording active.
- **Overlay window / GPU compositor contention** — overlay fully hidden, Brave's
  GPU helpers never spiked, still ducked.
- **The #180 audio-route instrumentation** — disabled the focus snapshot; the
  `coreaudiod` burst vanished but the duck remained.
- **Anything on the #180 branch** — stock `main` (zero instrumentation) ducked
  identically. The bug was pre-existing and in untouched code.

That pointed at something the **app itself** does on focus, leading to the
`getUserMedia` call in `PermissionsBanner`. Stubbing that call out flattened the
RMS trace completely (no duck), confirming the cause; the native fix reproduced
the flat trace.

## The fix (#182)

- New Tauri command `check_microphone_permission` reads
  `AVCaptureDevice.authorizationStatus(for: .audio)` via `objc2`. It queries TCC
  state and never instantiates a capture/voice-processing unit, so it cannot duck
  other audio. `build.rs` links the AVFoundation framework for the class lookup.
- `PermissionsBanner` uses the command instead of `getUserMedia`; the focus
  re-check stays (now a cheap status read).

## Tests added

- **CI guard** — `app/src/lib/no-mic-probe.test.ts` fails the build if
  `getUserMedia(` is reintroduced anywhere in the frontend. The invariant: the
  WebView must never open the microphone; recording goes through the native cpal
  pipeline.
- **Manual QA harness** — `scripts/qa/`:
  - `rms-meter.swift` — the output-level tap.
  - `audio-duck-check.sh` — scripts a surface/leave cycle on a target app,
    compares foreground vs baseline output level, exits PASS/FAIL (>15 dB drop =
    duck). Needs macOS + audio playing, so it's a local check, not CI.
  - Verified PASS on the fix; would report FAIL on the prior behaviour.

## Lessons

- **Build an objective signal early.** Hours went into CPU/log proxies that
  didn't track the symptom. A direct measurement of the thing the user perceives
  (output level) settled in minutes what proxies couldn't.
- **Beware instrument bias.** Instrumentation built around one hypothesis makes
  every reading look like confirmation of that hypothesis.
- **Reproduce on a clean baseline.** Confirming the bug on stock `main` ruled out
  the entire diagnostics branch in one test and redirected the search to
  pre-existing code.
- **A WebView is an app with audio capability.** `getUserMedia` is not free —
  on macOS it activates voice-processing I/O that ducks other audio. Permission
  *status* should be queried natively, never by opening the device.
