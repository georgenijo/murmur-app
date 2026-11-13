# Mute speakers while dictating

## Overview

Dictating over a video or a song means the microphone hears the playback, and
the transcript can pick up words from it. With **Settings → Recording → Mute
speakers while dictating** on, Murmur mutes the Mac's speakers while a
dictation records and unmutes them when it ends.

The setting is off by default (`muteSpeakersWhileDictating: false`) because it
silences everything on that output, including a call.

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
| Live dictation (hold, double-tap, both, button, overlay) | Muted from 150 ms after the start until Processing or Idle |
| Stop | Unmuted at the Recording → Processing transition |
| Cancel, failed start, error, empty or too-short recording | Unmuted when the status returns to Idle |
| Capture interrupted (Recovering) | Unmuted; muted again if recording resumes |
| Fn / Globe key-down capture that is discarded | Never muted: it never leaves Idle |
| Meetings | Never muted: meetings block dictation and do not use dictation status |
| Microphone test in Settings | Never muted: it does not use dictation status |
| Imported-file transcription | Never muted: it goes Idle → Processing with no start |
| Clean quit (`RunEvent::Exit`) | Unmuted before exit (bounded 750 ms wait) |
| Crash, force quit, or kill mid-dictation | Unmuted at the next launch |

The setting is read once, at the Idle → Starting transition, like other
recording configuration. Changing it during a dictation applies to the next one.

## Which outputs are muted

Murmur skips the mute when the microphone cannot hear the output:

- **Headphones on the built-in jack**: the output's data source is
  `'hdpn'`.
- **Bluetooth and Bluetooth LE outputs** (AirPods, headsets). On a Mac these
  are nearly always worn. Bluetooth speakers are a known limit: they are
  skipped too.

Any other output is muted, including USB, HDMI, DisplayPort, AirPlay,
Thunderbolt, aggregate, virtual, and outputs whose transport cannot be read.
When unsure, Murmur mutes. The user asked for playback to stay out of
transcripts. A needless mute is undone at stop, but a wrong skip would quietly
defeat the setting. USB headsets are muted for the same reason.

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
- If the device is gone at stop (unplugged), the record is kept. The restore is
  retried at the next dictation start and the next launch.

## Crash safety

Before muting, Murmur writes `output-quieting.json` beside `settings.json` in
its app data directory. The file is fsynced, written atomically, and has mode
`0600`. It holds only a schema version, the device UID, and the muted element
numbers. It has no audio, transcript, clipboard, selection, or app identity.
The file is removed once the speakers are restored. If it cannot be written,
Murmur does not mute.

At launch, a background thread reads the file. It unmutes the device if that
device is still muted, and then removes the file.

## Sound cues

The start cue is a 70 ms tone that plays when the dictation is accepted.
Murmur waits 150 ms after the start before muting, so the cue stays audible.
A stop within those 150 ms cancels the mute entirely. The unmute runs
at the Recording → Processing transition, the same moment the stop cue is
scheduled, so the stop and success cues play at the restored level. The 50 ms
release tail ends before the unmuted output reaches the microphone.

## Latency

The dictation status seam (`transition_dictation_status`) only sends a message
to a dedicated `murmur-output-quieting` thread. Start and stop never wait on
Core Audio. Each decision logs its own Core Audio time in `hal_us`.

## Telemetry

One content-free event per decision, `audio.output_quieting`:

| Field | Values |
|-------|--------|
| `recording_id` | Dictation generation (0 for launch recovery) |
| `outcome` | `engaged`, `restored`, `restored_after_exit`, `user_changed`, `skipped_no_output`, `skipped_headphones`, `skipped_bluetooth`, `skipped_no_mute_control`, `skipped_already_muted`, `skipped_record_unavailable`, `mute_failed`, `device_missing`, `restore_failed`, `unreadable` |
| `transport` | `built_in`, `bluetooth`, `usb`, `hdmi`, `displayport`, `airplay`, `aggregate`, `virtual`, `thunderbolt`, `other`, `unknown`, `none` |
| `hal_us` | Core Audio time for the decision |

Device names and UIDs are never logged.

## Code

- `app/src-tauri/src/output_quieting.rs`: state machine, record store, Core
  Audio HAL, worker, and tests.
- `app/src-tauri/src/commands/recording.rs`: `transition_dictation_status`
  calls `output_quieting::observe_transition`, and `configure_dictation`
  accepts `muteSpeakersWhileDictating`.
- `app/src/components/settings/SettingsPanel.tsx`: the Recording → Input row
  and its search entry.
