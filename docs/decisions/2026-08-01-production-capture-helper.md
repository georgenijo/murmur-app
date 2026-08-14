# Production capture is owned by a killable helper

Date: 2026-08-01  
Status: active  
Issues: #405, #408, #409, #410, #411, #412, #426, #436

## Decision

The Tauri application process never links CPAL or CoreAudio capture APIs. It
owns each signed `murmur-capture-worker` process group directly and communicates
over production protocol v5. Capture, bounded enumeration, and the passive
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

The worker reports content-free setup transitions around AUHAL device
resolution, AudioUnit creation, format configuration, callback installation,
stream start, and the transition to awaiting the first callback. CPAL reports
the comparable device-resolution, default-config, stream-build, stream-start,
and callback-wait transitions. Together with host-side helper launch, first-PCM,
and stop-to-exit timings, this localizes a blocked HAL operation without logging
device identity, raw backend errors, or microphone content.

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
- This repairs the bounded-fallback contract and adds attribution evidence. It
  does not claim to remove the underlying Core Audio hang.
