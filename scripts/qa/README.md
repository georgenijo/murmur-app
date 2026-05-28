# Manual QA harness

Behavioral checks that need a real machine + audio and so can't run in CI. Run
these locally before shipping audio/window/focus changes.

## audio-ducking check (issue #177)

`audio-duck-check.sh` detects whether foregrounding an app **ducks other system
audio** — the regression where surfacing Murmur quieted/stuttered a video playing
in another app. Root cause was a WebView `getUserMedia` mic probe firing on focus
(voice-processing I/O ducks all other audio). The CI guard `app/src/lib/no-mic-probe.test.ts`
prevents that specific cause from returning; this harness measures the *behavior*.

### How it works
`rms-meter.swift` taps the global system output (CoreAudio process tap, macOS 14.4+,
unmuted — doesn't change what you hear) and logs output RMS ~10x/sec. The script
scripts a surface/leave cycle on the target app and compares the mean output level
while the app is foreground vs not. A drop > 15 dB while foreground = FAIL.

### Run it
1. Start the Murmur dev app (`cd app && npm run tauri dev`).
2. Play audio through the default output (a video/music in another app) — keep it
   playing for the whole run.
3. `scripts/qa/audio-duck-check.sh`
   - first run prompts once for terminal "audio recording" permission — approve it.
   - target another app instead: `audio-duck-check.sh "Safari"` or `--pid <pid>`.

Exit 0 = PASS (no duck), 1 = FAIL (duck detected), 2 = setup error.

### Reading raw output
`rms-meter` alone prints a level timeline you can eyeball:
```
swiftc -O rms-meter.swift -o rms-meter -framework CoreAudio -framework AVFoundation -framework Foundation
./rms-meter            # Ctrl-C to stop
```
A duck shows as the dB dropping (e.g. -35 → -90) when the app comes to the front
and recovering when it loses focus.
