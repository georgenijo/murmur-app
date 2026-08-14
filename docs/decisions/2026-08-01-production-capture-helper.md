# Production capture is owned by a killable helper

Date: 2026-08-01  
Status: active  
Issues: #405, #408, #409, #410, #411, #412, #426, #436, #542

## Decision

The Tauri application process never links CPAL or CoreAudio capture APIs. It
owns each signed `murmur-capture-worker` process group directly and communicates
over production protocol v6. Capture, bounded enumeration, and the passive
input-topology watcher use separate owned sessions. Every frame carries a
monotonic capture ID and a random
128-bit nonce. Control payloads are bounded JSON; audio is bounded binary mono
`f32` PCM with a strict sequence and sample rate.

The worker offers two independent macOS capture implementations: direct AUHAL
and CPAL/CoreAudio. macOS tries direct AUHAL first because CPAL's synchronous
stream builder can remain blocked on a healthy USB default input; CPAL remains
the exact-device fallback. A failed backend may fall back once, and only before
the first PCM buffer is retained. The primary helper process group must be
positively confirmed empty and Stop must still be absent before the fallback is
spawned. If termination cannot be confirmed, Murmur retains recovery ownership,
rejects competing starts, and never opens the alternate backend. The fallback
must reuse the exact raw device UID. After
retained audio, any gap, malformed frame, overflow, backend error, stall, or
process exit terminates capture and preserves the delivered prefix.

If both backends in a complete pre-PCM pass report `device_unavailable` for an
explicit pinned microphone, the supervisor may repeat that full two-backend
pass in the session's immutable memo-resolved order up to two more times. Each
500 ms inter-pass gap remains interruptible by Stop or command-channel
disconnect. Every attempt receives the same immutable raw device UID; no
attempt substitutes the default input. System-default capture, mixed
failure kinds, retained audio, and any other terminal condition do not enter
this re-resolution loop.

The worker reports content-free setup transitions around AUHAL device
resolution, AudioUnit creation, format configuration, callback installation,
stream start, and the transition to awaiting the first callback. CPAL reports
the comparable device-resolution, default-config, stream-build, stream-start,
and callback-wait transitions. Together with host-side helper launch, first-PCM,
and stop-to-exit timings, this localizes a blocked HAL operation without logging
device identity, raw backend errors, or microphone content.

Protocol v6 additionally requires a bounded `InputResolution` message before
each live backend open. It proves the backend, whether enumeration succeeded,
whether a pinned requested input was present when knowable, the input count
capped at 256 (with a separate cap flag), and whether a default input existed.
The host rejects inconsistent field combinations. Its strict telemetry
projection permits only bounded attempt/owner/mode coordinates and those
booleans/counts; it strips unknown fields and never admits a device ID, name,
raw error, path, or content. Contradictory evidence retains only the stable
event code in debug and release builds.

Initialization uses one decided 30-second active-time contract: 8 seconds for
AUHAL through first PCM, 2 seconds for confirmed AUHAL termination, 16 seconds
for CPAL through first PCM, 2 seconds for confirmed CPAL termination, and 2
seconds of protocol/scheduler reserve. Time spent while macOS reports a genuine
pending TCC prompt (`notDetermined`) is excluded from those active deadlines.
The prompt has its own 120-second wall-clock watchdog; denial and user Stop
remain immediate.

The real-time callback only converts to mono and writes into a preallocated
eight-second SPSC ring. A non-real-time writer drains the ring into protocol
frames. Audio content is never logged or included in telemetry.

When an unexpected failure leaves at least 500 ms of resampled PCM, Murmur
automatically transcribes that prefix through the normal clipboard-first
pipeline and marks the result as interrupted/partial. Shorter prefixes fail
without transcription. User cancellation remains a distinct discard path.

## Consequences

- A blocked HAL call can be terminated by killing the exact owned process group.
- The app retains ordering, accumulation, level calculation, deadlines, and
  transcription policy while the helper exclusively owns macOS audio objects.
- CI rejects future app-crate CPAL/CoreAudio dependencies and direct HAL source.
- Device failover cannot silently switch microphones after capture begins.
- Short-lived USB re-enumeration can recover within two extra same-device
  passes, while cancellation and the no-substitution boundary remain explicit.
- If all bounded passes fail, the terminal typed failure drives a five-second
  mic-off overlay status labelled “Selected microphone unavailable. Open
  Settings to choose another.” It never expands, focuses, or resizes the
  overlay; non-device failures retain the generic cue.
- This repairs the bounded-fallback contract and adds attribution evidence. It
  does not claim to remove the underlying Core Audio hang.
