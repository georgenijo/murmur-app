# Pause Browser Media While Dictating

Optional, **off by default** (`pauseBrowserMedia`). Settings → Recording →
**Pause Browser Media While Dictating**. Issue #835; the approach is adapted
from Hex (MIT, see `THIRD_PARTY_NOTICES.md`).

## User contract

- When a dictation starts and a supported browser is audibly playing, Murmur
  pauses it. When the recording ends (to transcription or cancel), Murmur plays
  it again.
- Murmur sends play **only** when it proved its own pause silenced the
  browser. It never resumes media you paused yourself and never starts media
  that was not playing.
- Murmur sends the explicit MediaRemote pause and play commands, never the
  play/pause toggle.
- A browser using the microphone (probably a call) is left alone.
- The setting is read once when a recording starts. Turning it off mid-recording
  still lets Murmur resume what it already paused.

## How it decides

macOS no longer tells other apps what is playing, so Murmur listens to the
browsers' own output level through a private, unmuted, mono Core Audio process
tap that covers only browser processes.

```text
Dictation Starting → Recording (setting on, System Audio Recording granted)
├── A browser uses the microphone, or Murmur cannot tell → leave it alone
├── No browser process has its output open → leave it alone (no tap)
├── Listen 3 × 100 ms; any window at or below -60 dBFS, or no level → leave it alone
├── Send the system pause (MediaRemote 1)
└── Poll 5 × 100 ms
    ├── 2 windows in a row below -90 dBFS → Murmur paused it (proof kept)
    └── Otherwise (another tab, a sound that ended, pause reached another app) → no play later

While the recording lasts, the same tap keeps listening every 500 ms
├── The browser sounds again (you played it) → give up the proof: no play later,
│   even if you pause it again before the dictation ends
├── The tap stops delivering for good (worker or tap lost) → give up the proof
└── Silent, or its output closed after a long pause → keep the proof

Recording → Processing or Idle, and Murmur holds the proof
├── A newer recording already started → it inherits the pause; no play yet
├── The resume worker could not create its tap → no play
├── Listen 300 ms to every browser process
│   ├── Above -60 dBFS (you started it again) → no play
│   ├── No level, a browser runs and every one closed its output → still paused: play
│   └── No level otherwise (browser quit, tap failed) → no play
└── Send the system play (MediaRemote 0), then poll up to 20 × 100 ms
    ├── A window above -60 dBFS → resumed
    ├── No browser output open after 10 windows → send pause once
    └── Silent throughout (tab closed or reloaded) → send pause once
```

Work runs in order on one background thread, so dictation start and stop never
wait for it, and a resume always follows its own pause. A pause takes about
0.3–1.1 s; a very short dictation can end before the pause runs, in which case
nothing is paused.

## Process boundary

The app process never opens an audio device. Each pause or resume spawns the
signed, sandboxed capture worker in `--browser-level <playing|all>` mode. It
owns the tap and answers bounded line requests (`peak <ms>`, `closed`) with a
peak in dBFS or `none`. The IO callback keeps only an atomic peak and callback
count; no audio sample is copied, stored, or written out. Closing stdin tears
the tap down in reverse creation order. While Murmur holds a pause, the pause
worker stays open for the whole recording and is closed before the resume; the host hard-kills the exact process
group if the worker does not exit within 500 ms. The host owns the decision and
sends the MediaRemote command (the worker's sandbox does not reach it).

Safari plays through `com.apple.WebKit.GPU`, a process shared with every WebKit
client. The host passes only the GPU processes whose display name starts with
"Safari"; Mail and other WebKit apps are never tapped.

## Permission

The tap needs macOS **System Audio Recording** consent (TCC
`kTCCServiceAudioCapture`, shown under Screen & System Audio Recording). Murmur
requests it only when you turn the setting on. Before every pause it checks
consent without prompting and skips the pause unless consent is granted, so a
system dialog can never open while you dictate. If consent is denied the row
shows **Open Settings**. No Automation (Apple Events) permission is used.

## Supported players and limits

- Supported: Safari, Google Chrome, Chromium, Microsoft Edge, Brave, Arc,
  Vivaldi, Opera (and their beta/canary variants). Firefox is heard too, on a
  best-effort basis.
- Not covered: native players (Music, Spotify, VLC, IINA, Podcasts), Electron
  apps, and WebKit views inside other apps. They are never tapped. Their media
  keeps playing.
- The pause and play go to whatever macOS treats as **Now Playing**. If another
  app owns Now Playing while a browser plays, the pause can reach that app
  instead. The browser then keeps sounding, Murmur does not confirm the pause,
  and it sends no play, so the other app may stay paused. Likewise, if Now
  Playing changes to another app while the browser is paused, the play can
  reach that app. Murmur then hears the browser stay silent and sends pause
  again within 1–2 s. Since macOS 15.4, apps cannot read Now Playing, so Murmur
  cannot check the target first.
- Two tabs sounding at once: one pause silences only one, so the pause is not
  confirmed and nothing is resumed.
- Quitting Murmur during a recording leaves a paused browser paused.
- Requires macOS 14.2 or later on Apple Silicon (the process-tap and worker
  platform). Elsewhere the setting reports "Not available on this Mac".

## Diagnostics

Content-free `audio` events: `audio.browser_media_pause` (`outcome`:
`paused`, `no_permission`, `in_call`, `silent`, `unheard`, `refused`,
`unconfirmed`; `permission_granted`; `elapsed_ms`; `recording_id`) and
`audio.browser_media_resume` (`outcome`: `resumed`, `already_playing`,
`unheard`, `refused`, `lost_after_play`, `repaused`), and
`audio.browser_media_released` (`outcome`: `restarted`, `lost`) when a pause is
given up during dictation. No process, tab, page,
title, or audio is logged.

## Validation

- `cargo test browser_media -- --test-threads=1`: the pause/resume sequence on
  fakes (confirmed pause resumes, two sounding tabs never get play, a browser the
  user restarted gets no play, a call is never paused, a play nobody takes is
  paused again, closed-output long pauses), the during-dictation watch
  (restarted or lost releases the proof), transition mapping, and strict
  worker reply parsing.
- `cargo test -p murmur-capture-helper browser_level`: argument and request
  parsing, browser bundle matching, Safari attribution, fail-closed call check.
- Native smoke: a signed build with a local HTML page playing a generated tone
  in a browser; media pauses during dictation and resumes after, and nothing
  changes with the setting off.
