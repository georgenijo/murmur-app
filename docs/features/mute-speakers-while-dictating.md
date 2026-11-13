# Mute speakers while dictating

## Overview

Dictating over a video or a song means the microphone hears the playback, and
the transcript can pick up words from it. With **Settings → Recording → Mute
speakers while dictating** on, Murmur mutes the Mac's speakers while a
dictation records and unmutes them when it ends.

The setting is off by default (`muteSpeakersWhileDictating: false`) because it
silences everything on that output, including a call. USB and USB-C headsets
and dongles are muted too (see [Which outputs are muted](#which-outputs-are-muted)).

## What it does and does not do

- It changes one thing: the **mute** property of the default output device.
  The volume level is never written, so unmuting returns the exact prior level.
- It never pauses, resumes, or sends media commands. The earlier
  browser-media design (#835, PR #842) was dropped because its resume could
  start media the user had paused. Unmuting cannot start playback.
- It never opens an audio unit, stream, or IOProc. Every operation is a Core
  Audio HAL property read or write, so it cannot reintroduce the
  voice-processing ducking from #177, and it does not use macOS ducking.

## When it applies

| Situation | Behavior |
|-----------|----------|
| Live dictation (hold, double-tap, both, button, overlay) | Muted once the start cue has played out (see [Sound cues](#sound-cues)), until the dictation leaves live capture |
| Stop | Unmuted at the Recording → Processing transition |
| Cancel, failed start, error, empty or too-short recording | Unmuted when the status returns to Idle |
| Capture interrupted (Recovering) | Unmuted. A Recovering dictation delivers what it has or ends; it never returns to Recording |
| Fn / Globe key-down capture that is discarded | Never muted: it never leaves Idle |
| Meetings | Never muted: meetings block dictation and do not use dictation status |
| Microphone test in Settings | Never muted: it does not use dictation status |
| Imported-file transcription | Never muted: it goes Idle → Processing with no start |
| Clean quit (`RunEvent::Exit`) | Unmuted before exit (bounded 750 ms wait) |
| Crash, force quit, or kill mid-dictation | Unmuted at the next launch, if that is within 10 minutes of the mute |

The setting is read once, at the Idle → Starting transition, like other
recording configuration. Changing it during a dictation applies to the next one.
Each dictation makes one mute decision. A skipped or failed mute is not
retried later in the same dictation.

## Which outputs are muted

Murmur skips the mute when the microphone cannot hear the output:

- **Headphones on the built-in jack**: the output's data source is `'hdpn'`,
  or it is the built-in device with UID `BuiltInHeadphoneOutputDevice`. On Apple
  Silicon the jack is its own device. On the Mac mini used for testing, the
  Speakers device lists only the `'ispk'` source, and the headphone device only
  appears when something is plugged in, so neither check has been observed on
  real headphones. Anything plugged into the 3.5 mm jack counts, including
  powered speakers on a Mac mini, Mac Studio, or iMac: they are skipped, and
  their playback can still reach the microphone.
- **Bluetooth and Bluetooth LE outputs** (AirPods, headsets). On a Mac these
  are nearly always worn. Bluetooth speakers are a known limit: they are
  skipped too.

Any other output is muted. That includes USB and USB-C headsets and dongles,
HDMI, DisplayPort, AirPlay, Thunderbolt, aggregate and virtual devices, and
outputs whose transport cannot be read. When unsure, Murmur mutes. The user
asked for playback to stay out of transcripts. A needless mute is undone at
stop, but a wrong skip would quietly defeat the setting. The Settings
description says the setting quiets calls and is skipped only for Bluetooth and
the headphone jack.

Murmur also does nothing when:

- the device has no settable mute, on either the main element or both stereo
  channels. Many HDMI and DisplayPort outputs have no software volume. The
  dictation is unaffected.
- the output is already muted. Nothing is recorded and nothing is unmuted
  later.

## Restore rules

- Murmur restores only the device it muted. It finds that device again by its
  Core Audio UID, so a new default output chosen mid-dictation is never touched.
- If the user unmuted during the dictation, the restore leaves the device alone.
  Volume keys unmute, so pressing volume up or down counts.
- Only the channels Murmur muted are recorded. A channel the user had already
  muted stays muted.
- A restore that cannot finish is retried on the worker. This covers a device
  that is gone (unplugged, undocked), a mute that cannot be read (a device that
  Core Audio is removing or has not finished adding), and an unmute that fails.
  The retries back off at 0.5, 1, 2, 4, 8 and 15 seconds, then every 30
  seconds. They also run at the next dictation start, and they stop when the
  record expires. A retry never touches a device that a later dictation has
  muted: the record waits until that dictation's own restore, and is dropped
  if the device is already unmuted by then.

## Crash safety and expiry

After a successful mute, Murmur writes `output-quieting.json` beside
`settings.json` in its app data directory. The record holds a schema version,
the device's Core Audio UID, the muted element numbers, and the time of the mute.
It has no audio, transcript, clipboard, selection, or app identity. A USB
device's UID can include its product name and serial number, so the file is
local only and has mode `0600`, and the UID is never logged. The write goes to a
temporary file, is flushed with `F_FULLFSYNC`, is renamed into place, and then
the directory is flushed. The file is removed once the speakers are restored.

At launch, a background thread reads the file:

- A record younger than **10 minutes** is restored if its device is still
  muted.
- An older record, or one dated in the future, is dropped without touching the
  device (`expired`).
- A record from an earlier version, which has no mute time, is dropped.

Records waiting for a retry expire the same way, but their 10 minutes count
from the end of the dictation, when the restore first failed. A long dictation
therefore gets the full retry window.

Why 10 minutes: an unexpected unmute is worse than a Mac left muted. A muted Mac
shows the muted speaker in the menu bar, and one key press fixes it. An unmute
can blast audio in a library or a meeting. Ten minutes is enough to relaunch
after a crash, and short enough that a later deliberate mute is unlikely to be
undone.

Remaining limits:

- If the user unmutes and then mutes again on purpose within 10 minutes of a
  crash, the next launch in that window unmutes it.
- After a crash at least 10 minutes into a long dictation, the Mac stays muted.
- The record is written just after the mute. If the app dies between the two
  (a few milliseconds), the Mac stays muted.

All three leave the Mac muted rather than unmuting unexpectedly, except the
first, which needs a deliberate re-mute inside the window.

If the record cannot be written, Murmur unmutes at once and logs
`skipped_record_unavailable`.

## Sound cues

The start cue is a 70 ms tone played by the main window when the dictation is
accepted. With the setting on, the main window reports
(`report_start_cue_finished`) once the cue has played out. That is when the last
tone has ended on the audio clock, plus the context's output latency (at most
200 ms) and 30 ms. If no cue plays (cues off, volume 0, during a meeting), the
report is sent at once. The mute follows the report. If the report never
arrives, the mute happens anyway 500 ms after the start (`trigger="cap"`). Nothing waits on
the report: it only releases a pending mute on the worker. A stop before the
mute cancels it.

The unmute runs at the Recording → Processing transition, when the stop cue is
scheduled. So the stop and success cues play at the restored level.

## The release tail

Processing is set when the stop is accepted. The capture then keeps a 50 ms
release tail (`DICTATION_RELEASE_TAIL`), and the unmute lands a few milliseconds
after Processing. Playback can therefore reach the microphone during the last
part of that tail, after the output's latency. That is at most about 50 ms at
the very end of the recording, too short to form a word. The stop cue has always
played into the same tail.

Unmuting after the tail would close that gap, but it would silence the stop cue.
The capture hands off about 75–80 ms after Processing on the test Mac, longer
than the 70 ms cue. Keeping the cue would mean rescheduling it behind a new
event. The current design keeps the cue.

## Latency

The dictation status seam (`transition_dictation_status`) only sends a message
to a dedicated `murmur-output-quieting` thread. `report_start_cue_finished` does
the same. Start and stop never wait on Core Audio or the record file. Each
decision logs its own time in `hal_us`.

## Telemetry

One content-free event per decision, `audio.output_quieting`:

| Field | Values |
|-------|--------|
| `recording_id` | Dictation generation. Retries that run at a dictation start carry that dictation's id. Launch recovery and timed background retries use 0 |
| `outcome` | `engaged`, `restored`, `restored_after_exit`, `user_changed`, `expired`, `skipped_no_output`, `skipped_headphones`, `skipped_bluetooth`, `skipped_no_mute_control`, `skipped_already_muted`, `skipped_record_unavailable`, `mute_failed`, `device_missing`, `restore_failed`, `unreadable` |
| `transport` | `built_in`, `bluetooth`, `usb`, `hdmi`, `displayport`, `airplay`, `aggregate`, `virtual`, `thunderbolt`, `other`, `unknown`, `none` |
| `trigger` | Engage decisions: `cue_finished` or `cap`; otherwise `none` |
| `since_start_ms` | Engage decisions: start to mute decision |
| `hal_us` | Core Audio and record-store time for the decision |

A device that stays unplugged is logged on the first miss and when it expires,
not on every quiet retry. Device names and UIDs are never logged.

## Code

- `app/src-tauri/src/output_quieting.rs`: state machine, record store, Core
  Audio HAL, worker, and tests.
- `app/src-tauri/src/commands/recording.rs`: `transition_dictation_status`
  calls `output_quieting::observe_transition`, `configure_dictation` accepts
  `muteSpeakersWhileDictating`, and `report_start_cue_finished` forwards the
  cue report.
- `app/src/lib/soundCues.ts` and `app/src/lib/hooks/useSoundCues.ts`: cue
  play-out promise and the report.
- `app/src/components/settings/SettingsPanel.tsx`: the Recording → Input row
  and its search entry.
