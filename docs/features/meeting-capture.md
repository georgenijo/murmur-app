# Meeting Capture (Phase 1)

Meeting Capture is an explicit, local-only long-form recording mode for macOS
14.2 and newer. It records the selected microphone and Mac playback as separate
streams, transcribes both incrementally with the selected local model, and
stores an ordered transcript in SQLite. After capture, an explicit action can
derive a local summary, decisions, action items, and open questions.

## Derived artifact foundation

Meeting summaries use the portable `murmur.meeting-artifact.v1` contract. Every
summary, decision, action item, and open question carries one or more source
segment IDs. Action owners and due dates are nullable; absent, informal, or
unsupported values remain `null` and are displayed as Unknown rather than
being guessed.

Long transcripts are divided in capture order with both character and
segment-count bounds. Partial artifacts merge with a fan-in of eight and repeat
hierarchically until one bounded artifact remains. Duplicate claims are folded
deterministically. Validation rejects unknown schema fields, oversized arrays
or text, malformed values, and source IDs that do not belong to the meeting.

Artifacts export as Markdown, plain text, or self-identifying JSON. All three
retain source-segment provenance; Markdown and text render unknown action
owners/dates explicitly. The foundation is local and pure: it does not upload
transcripts or log meeting content.

## Local summary execution

Completed and interrupted meetings with finalized transcript text expose a
**Summarize** action. The coordinator chunks the immutable ordered transcript,
runs one request at a time through the signed local-LLM sidecar, validates exact
JSON and source provenance after every request, merges bounded results
deterministically, and atomically upserts one artifact for the session. Retry
replaces the artifact only after the new result fully succeeds.

The UI shows chunk progress, elapsed runtime, cancellation, retry, and the final
summary with source segment IDs. Cancellation targets the exact generation and
keeps any prior artifact. Summary ownership blocks competing capture and model
work until the sidecar exits. Runtime and peak helper RSS remain content-free
status evidence.

## User contract

- Start or stop from **History → Meetings**, or with the command palette.
- **Me** is microphone audio. **Them** is system playback. No diarization or
  speaker inference occurs. If the user's voice is played through speakers, it
  can also appear as Them; phase 1 does not perform echo cancellation.
- The overlay shows a persistent two-dot meeting state from accepted start
  through capture stop. A spinner remains while final durable chunks transcribe.
- Dictation, file transcription, benchmarks, corpus capture, selected-text
  transform, meeting capture, and interrupted-meeting recovery refuse to start
  over one another instead of contending for Core Audio or the model.
- Audio retention is off by default. A chunk WAV is deleted only after its
  transcript commits. Users may retain audio and set age/session-count caps.

## Capture boundary

`murmur-capture-worker --production-v6` owns both native streams:

1. A private, unmuted stereo `CATap` captures global system output and the
   realtime callback downmixes it to mono without allocation.
2. A private aggregate device binds the tap to an IOProc callback.
3. The existing AUHAL microphone path captures the selected input.
4. Each callback writes only to its own preallocated eight-second SPSC ring.
5. The worker drains those rings into channel-tagged, capture-scoped PCM frames.

The protocol carries `channel`, per-channel `sequence` and `sample_offset`, and
a best-effort worker monotonic timestamp. The host rejects gaps, duplicates,
rate changes, wrong capture identity, wrong nonce, unknown channels, and non-v7
frames. It never mixes the streams.

Protocol v7 also carries bounded `InputResolution` evidence before the live
microphone backend opens: backend, enumeration outcome, knowable pinned-input
presence, a count capped at 256, and default-input availability. It contains no
device ID, display name, raw error, path, or audio content.

The tap exists only inside an explicit permission probe or meeting session.
Status reads return a cached `unknown`/`granted`/`denied`/`unsupported` value and
never create a tap. Stop waits two seconds for the worker teardown receipt,
then uses the exact managed process group and confirms termination. Closing the
parent pipe also makes an orphaned worker leave its loop and drop the IOProc,
aggregate device, and tap in reverse creation order.

## Incremental pipeline

The host has three bounded stages:

```text
two 8-second callback rings
  → protocol reader / 128-frame PCM queue
  → per-channel streaming resampler + VAD chunker
  → fsynced 16 kHz mono spool WAV + pending SQLite row
  → serialized model-runtime worker
  → final segment transaction + optional WAV deletion
```

Silero VAD examines 500 ms windows independently per channel, retains 250 ms
of pre-roll and 500 ms of trailing silence, and forces a boundary at roughly 15
seconds (at most one analysis window later). The segmenter warms VAD and signals
readiness before the host tells the worker to begin capture; a short bounded
queue retry absorbs scheduler jitter without hiding sustained backpressure.
VAD Off produces fixed-size chunks. Only one chunk's samples enter inference at
a time; the wake queue has one token and pending work is fetched one SQLite row
at a time. The one-hour accelerated test asserts buffers remain bounded.

The persisted session freezes model, language, punctuation, and audio-retention
policy at start. `PreparationReason::Meeting` uses the same serialized
`ModelRuntimeManager` as dictation, and the idle monitor cannot unload it while
live or recovery inference owns the meeting flag.

## Durability

The store lives under the app data directory in `meetings/`:

- `meetings.sqlite3` uses WAL, `synchronous=FULL`, foreign keys, and schema v2.
- `meeting_sessions` stores start/end, status, selected model/language, frozen
  punctuation/audio policy, and a stable content-free failure code.
- `meeting_segments` stores speaker, per-channel sequence, relative timing,
  pending/final/failed status, text, and an optional relative spool path.
- `meeting_artifacts` stores one validated schema-v1 derived result plus its
  content-free runtime and peak helper RSS; deleting the session cascades it.
- FTS5 indexes finalized segment text for bounded session search.
- Checked SQLite backups are retained before migrations; corrupt databases are
  quarantined and restored from the newest valid backup when possible.

Publication order is deliberate: write and fsync a sibling temporary WAV,
rename and sync its directory, insert a pending row, transcribe, commit final
text and FTS, then delete the WAV in transcript-only mode. Startup marks active
sessions interrupted, sweeps only unowned audio, and drains pending rows. A
crash preserves every published chunk and loses only the currently open VAD
chunk on each channel.

## Privacy and permissions

Both app and worker carry `NSAudioCaptureUsageDescription`. macOS 14.2 is a
feature gate, not a new app-wide minimum. Onboarding includes a skippable,
explicit System Audio step; denial produces an actionable banner and link to
Screen & System Audio Recording settings.

Authorization and capture health are separate facts, and the probe reports both:

- **Permission** comes from the tap attempt, which is authoritative. Core Audio
  refuses an unauthorized tap at creation with `kAudioDevicePermissionsError`,
  so a tap that creates its aggregate device and starts its IO proc proves
  access. The host also queries `CGPreflightScreenCaptureAccess` first and calls
  `CGRequestScreenCaptureAccess` only when the app is not yet determined, so an
  already-authorized user is never re-prompted.
- **Capture readiness** (`captureReady`) means the tap started. **Audio flow**
  (`audioFlowing`) means a callback delivered samples inside a bounded 500 ms
  observation window. A granted tap on a silent Mac is healthy — `granted`,
  `captureReady: true`, `audioFlowing: false` — and is never reported as a
  permission failure.
- **`needsRelaunch`** covers the one contradiction: macOS reports the app as
  authorized but Core Audio still refuses the tap, meaning the grant has not
  reached this process image. The UI asks the user to quit and reopen Murmur
  rather than retrying silently.

Every probe writes a content-free terminal result to the `meeting` stream:
`meeting.permission_probe_started`, `meeting.permission_probe_finished` (TCC
state, permission, capture readiness, audio flow, relaunch), and
`meeting.permission_probe_failed`.

Meeting traces contain only allowlisted lifecycle phase, channel, generation,
and stable error code. The sanitizer removes every other string in all builds.
The fleet log shipper additionally drops the entire `meeting` stream and
malformed JSONL, so neither transcript content nor session lifecycle leaves the
Mac.

## Validation

- `cargo test meeting -- --test-threads=1` covers the store, recovery,
  rendering, privacy sanitizer, and accelerated bounded-memory chunking.
- Protocol/helper tests cover v7 framing, channel identity, overflow, two
  independent rings under Loom, macOS gating, and typed permission failures.
- The native gate exercises signed-app permission attribution, start → first
  PCM on both channels → stop, and confirmed tap destruction.
- The implementing PR requires Murmur Bench `standard` against its immutable
  pushed SHA; replay benchmarks do not replace the native tap smoke.

## Non-goals

No system-channel diarization, calendar integration, auto-start, cloud sync,
translation, or caption overlay is part of phase 1. Action items are derived
text only; Murmur does not execute, send, or sync them.
