# Microphone Input Test

Settings → Dictation includes a capture-only microphone preview beside the
persisted input selector. It answers three questions before a dictation starts:
whether the selected device opens, whether speech reaches Murmur, and whether
the input is too quiet or clipping.

## User contract

- Opening Dictation Settings automatically opens the currently selected stable
  device ID (or the live system default), then shows RMS level, peak level, and
  a stabilized `No signal` / `Too quiet` / `Signal detected` / `Clipping`
  classification.
- Changing devices during a test first stops the exact preview generation and
  waits for its worker to exit and join. Murmur persists the new selection,
  then opens it only after teardown is confirmed. A teardown timeout preserves
  the selection but does not risk a second Core Audio owner.
- Monitoring yields automatically when dictation starts and resumes when the
  recording/transcription cycle returns to Idle. It also stops when the user
  leaves Dictation Settings, closes or hides Settings, or the Mac sleeps.
  Startup, permission, missing-device, runtime-unplug, and slow-teardown
  failures stay actionable in the row.
- An explicit device ID is never replaced by System Default. If the saved ID
  disappears, the existing missing-device warning remains and monitoring stays
  idle until the user chooses an available input.

## Capture ownership

`AudioOwner::Preview(preview_id)` is a first-class owner of the production
capture supervisor. Preview IDs are monotonic and every lifecycle event is
generation-checked. Preview is lower priority than real pipeline work:
dictation, Transform, Voice Query, corpus capture, and benchmark startup stop
the exact Preview owner, wait for confirmed worker teardown, and claim their
work under the same short `recording_transition` lock. Preview startup still
refuses while any real pipeline owns audio.

Stop acknowledgement is not teardown authority. The preview remains claimed
through Starting, Recording, Stopping, and Recovering, and clears only after the
worker has exited, its thread has joined, and the supervisor has published the
exact generation's `Idle` event. Optional cleanup resolves the current preview
ID before cancellation and never sends an ownerless supervisor cancel.

## Signal and privacy contract

Preview PCM follows the signed capture-worker protocol and the AUHAL → CPAL
fallback path, but the parent never appends it to the shared transcription
buffer. It is not transcribed, saved, copied, added to history, or emitted in
telemetry. A successful first callback does count as real per-device readiness
and intentionally trains the app-lifetime backend-selection memo.

The worker-side parent aggregates every finite sample between display-rate
updates. RMS and peak are clamped to 0–1 and sent only to the main window as
`microphone-preview-level`; preview never emits the global `audio-level` event
consumed by dictation and the overlay.

Classifications share the corpus recorder's quality thresholds:

- no signal: RMS < 0.001 and peak < 0.005
- too quiet: RMS < 0.01 or peak < 0.05
- signal detected: above the quiet thresholds
- clipping: peak sample ≥ 0.99

Non-clipping changes must remain stable for 250 ms. Clipping is immediate and
held for 500 ms so the label and accessibility text do not chatter. React owns
only low-frequency lifecycle state; requestAnimationFrame writes level, peak,
and meter ARIA values directly from refs.

## Implementation map

- Rust state/classification: `app/src-tauri/src/microphone_preview.rs`
- Rust commands/lifecycle bridge: `app/src-tauri/src/commands/microphone_preview.rs`
- Capture routing: `app/src-tauri/src/audio.rs`, `audio_lifecycle.rs`
- UI: `app/src/components/settings/MicrophoneInputTest.tsx`
- Invoke/types: `app/src/lib/microphonePreview.ts`
