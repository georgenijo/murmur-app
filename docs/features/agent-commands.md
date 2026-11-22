# Agent commands

## Overview

Agent commands let you speak to your coding agents instead of typing into an
app: hold a second key, say "launch a thread on Volta to fix the map fade" or
"what's running?", and release. Murmur transcribes the speech locally as it
does for dictation, then hands the text to the local
T3 Code server, which runs a dispatcher agent that does the
work and replies in a macOS notification.

The feature is off by default (`agentCommandEnabled: false`). Turn it on in
**Settings → Recording → Agent commands**.

## What leaves Murmur

Dictation stays on the Mac. An agent command does not: when the setting is on,
what you say while holding the agent command key is sent to your local T3 Code
server and the model it runs, instead of being typed. Nothing is sent while the
setting is off, and the dictation key never sends anything.

- The text goes to the `t3` command-line client on this Mac. Murmur holds no
  T3 or model credentials.
- Murmur logs only that a hold started or ended and whether a command was
  handed off, at debug level, which is off by default. The transcript and the
  agent's reply are never logged.
- An agent command is not added to History or to the usage statistics, is not
  written to the output folder, is not put on the clipboard, and is not
  mirrored to NotchPill.
- A diagnostic capture armed for the next dictation records an agent command's
  outcome but not its text.

## The key

| Key | Identifier |
|-----|------------|
| Right Option (default) | `alt_r` |
| Right Shift | `shift_r` |
| Left Control | `ctrl_l` |

The key is hold-to-talk and independent of the dictation trigger and its
recording mode. The three keys are not offered for dictation, Paste Last, the
command palette, or Settings, so the binding cannot collide with another one;
`set_agent_command_shortcut` refuses any other identifier.

- Holding the key for 250 ms starts the recording. Releasing it ends the
  recording and sends the command.
- A key that is part of a shortcut starts nothing: pressing it while another
  modifier is down, or pressing any other key or mouse button inside the first
  250 ms, is ignored.
- Pressing a non-modifier key (other than Space) within one second of the start
  discards the recording, so a slow shortcut such as Option+E sends nothing.
- Escape, disabling Murmur, or turning the setting off discards the recording.
- The key never starts over a recording that is already running, and the
  dictation key's release never ends an agent command.
- The key is bound only while the setting is on, Murmur is enabled, and
  Accessibility is granted.

## How a command is handled

1. The key's hold emits `agent-command-start`; `useAgentCommandKey` forwards it
   to `useRecordingState`, which calls `start_native_recording` with
   `origin: "agent_command"`. The start treats that origin as a hold.
2. The start freezes the agent purpose and the dispatch settings (project, provider,
   model) into the recording's immutable context under the dictation lock, at
   Idle → Starting. A later settings change applies to the next recording.
   Smart Destination is not captured for an agent command.
3. The recording runs the normal pipeline: voice activity detection,
   transcription, and the ordered transcript transforms. The overlay shows
   recording and processing as usual.
4. Where a dictation would be delivered, the pipeline rechecks that this
   recording still owns the Processing state and, under the same lock hold,
   calls `agent_command::spawn_dispatch` with the final text. A cancel either
   lands before the hand-off or after it.
5. The dispatcher (`agent_command.rs`) runs in the background and shows its
   replies through the notification plugin. A failure shows a short notification
   without the spoken text. An absolute dispatch project path that is not a
   folder fails up front, because `t3` would otherwise fall back to the home
   project.

`stop_native_recording` and the `transcription-complete` event carry an empty
`text` for an agent command, and the event carries `purpose: "agent_command"`,
so nothing presents the command as dictated or pasted text.

If a recording starts with the agent origin while the setting is off, it delivers
nothing: it is neither pasted nor dispatched.

## Settings

See [the settings reference](../reference/settings.md#agent-command-settings).
The dispatch project, provider, and model are stored settings without a control
in Settings. An empty dispatch project means `~/Documents/code/agent-dispatch`.

## Requirements

- macOS Accessibility permission, as for every global key.
- T3 Code installed and its server running on this Mac, with the dispatch
  project present.
- Notifications allowed for Murmur, to see the replies.
