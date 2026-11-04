# Performance diagnostics data

Issue [#351](https://github.com/georgenijo/murmur-app/issues/351) defines the
versioned, local data layer used by the Diagnostics performance workspace.
Dictation, imported-file, selected-text transform, and Voice Query runs share this contract.
Transform metrics reuse #332's canonical `transform_pass_id`; the existing
content-free transform trace remains the only structured trace source.

## Storage and retention

The Rust-owned SQLite database lives at:

```text
~/Library/Application Support/com.localdictation/diagnostics/performance.sqlite3
```

It is separate from logs, transcription history, settings, personal knowledge,
and Performance Lab/evaluation reports. The store keeps:

- the newest 200 completed runs;
- at most eight apply/undo follow-up attempts per completed transform run;
- active content-free lifecycle rows so early exits and restart interruption can
  close a run exactly once;
- the newest 600 one-second resource samples (a ten-minute window).

Completion and pruning share one transaction. On startup, a stale active row is
closed as `interrupted` with the stable `interruptedByRestart` code. Clearing
performance diagnostics removes only these runs and resource samples. It also
advances a clear epoch so an operation that began before Clear cannot reinsert
its old diagnostics when it eventually finishes.

The database currently uses SQLite `user_version = 5`; the v2 migration adds
Voice Query support and the v3 migration adds the dictation `releaseToStop`
and `releaseToPaste` stages, neither changing tables. Run and resource JSON records
remain backward-compatible at `schemaVersion: 1`. V4 adds `homophoneContext`
using the same additive-stage migration: a transaction updates `user_version`
without rewriting old JSON or changing tables. Historical runs omit this stage;
new readers preserve that absence. V5 adds optional production physical-gesture
capture timing in a separate 4 → 5 metadata migration. The v3 → v5 path applies
v4 then v5 in one transaction, preserving existing JSON bytes, including v4
homophoneContext rows. Older builds refuse the newer database. A
database created by a newer Murmur build is preserved and treated as unavailable rather than rewritten. Unknown
record versions are not decoded as V1.

SQLite failures use a bounded taxonomy: `busyLocked`, `storageFull`,
`readOnly`, `io`, `corruptIntegrity`, `schemaMigration`, `invalidRecord`, or
`unavailable`. Only `busyLocked` is retried, for at most three total attempts
with short bounded backoff. The retry happens in diagnostics persistence after
the capture transition; it never delays microphone ownership or prevents
ordinary dictation. A run that still cannot begin is explicitly skipped.

Initialization validates integrity and supported-version records after opening
the database and before the store is used. On automatic startup, only proven
physical corruption quarantines the database and its SQLite sidecars inside the
diagnostics directory before creating a fresh store once. An undecodable
supported-version record stays unavailable until the user explicitly confirms
**Reinitialize Store**. Unsupported/newer schemas and other persistent failures
also leave the store unavailable with a bounded local action; the health banner
distinguishes that state from a single skipped run. Explicit recovery reopens
and rechecks the store, and never quarantines without both confirmed caller
intent and fresh corruption or invalid-record evidence. It does not delete
quarantined evidence, and a healthy database is never quarantined.

## Run contract

`PerformanceRunV1` contains:

- an opaque random `runId`;
- kind (`dictation`, `fileTranscription`, `selectedTextTransform`, or `voiceQuery`);
- start/finish UTC timestamps and exactly one terminal outcome;
- the existing `recordingId`, a dedicated `fileRunId`, the canonical
  `transformPassId`, or the monotonic `queryPassId`;
- catalog-backed model, backend, accelerator, and warm/cold state;
- typed stage measurements;
- content-free audio duration or bounded size/token fields;
- scoped resource summaries.

Voice Query records may additionally carry `queryProcess` with only an integer
exit code (or unavailable) and a boolean `stderrPresent`. Old V1 rows omit the
field. No stderr bytes or provider error detail is admitted.

Every measurement is one of:

- `measured { value }` — including a legitimate measured zero;
- `notApplicable` — the stage or scope does not apply to this run;
- `unavailable { reason }` — measurement was expected but unsupported, failed,
  had no samples, or awaits an explicit dependency.

The contract never uses numeric zero as a missing-data sentinel.

### Dictation stages

- capture finalization/resampling;
- VAD;
- model preparation queue/lock wait and model load;
- a missing preparation sample remains not applicable; it is not replaced by
  the independently measured final-pass wait;
- final inference queue/lock wait, recorded separately as `inferenceQueue` so
  waiting behind a live preview is preserved even when preparation was instant;
  preparation can overlap capture, so these rows are not an additive timeline;
  historical v1 records without this new stage remain readable and omit the
  unrecorded measurement rather than displaying a measured zero;
- inference/decode;
- aggregate deterministic transcript transformation and the existing
  content-free cleanup, Voice Commands, Smart Correction, homophone context,
  Smart Formatting, IDE context, and CLI stage outcomes;
- `homophoneContext` measures the optional tier's complete wall time, with
  `completed` or `fallback` outcomes (including timeout, unavailable model,
  rejected answer, and adaptive cooldown). A cooldown fallback records its
  actual short duration, including a legitimate measured 0 ms after rounding;
  disabled/non-live runs use `notApplicable`. An unavailable timing uses the
  existing `unavailable { reason }` contract, never a synthetic zero. This row
  is included in `transcriptTransform`, not additional time to add to it;
- optional file output;
- clipboard/paste;
- total post-stop processing;
- `releaseToStop`: native hotkey release → `stop_native_recording_for` entry,
  the rdev → `hold-down-stop` → webview → IPC hop that `totalProcessing` never
  saw;
- `releaseToPaste`: native hotkey release → end of delivery, measured at the
  same instant `totalProcessing` ends, recorded only for successful runs whose
  delivery was confirmed (clipboard-only or posted auto-paste; a failed
  clipboard write, timeout, or stopped delivery has none).

The rdev listener stamps a monotonic instant when a trigger-key release (not a
chord press that ends the hold) publishes `hold-down-stop`, keyed by the live
recording ID. The first release of a recording wins. The stop claims the stamp
when it accepts the transition, only for its own recording and only within 10
seconds, so a release that did not drive the stop is never attributed to a
run. Toggle, overlay, and auto-stop
stops, and historical records, have no release stages; they are omitted, never
shown as zero. The stamp is one instant and one generation ID, with no content.
The Runs workspace shows nearest-rank p50/p90 of `totalProcessing` and
`releaseToPaste` across retained successful dictations.

### File stages

- decode/downmix/resample;
- VAD;
- final inference queue (`inferenceQueue`), model load and inference/decode;
  historical file runs retain their original `modelQueue` measurement;
- the authoritative verbatim transcript-transform entry point;
- file return;
- total command processing.

The new build reads older diagnostic records. Older builds do not recognize
the added stage in new records; downgrading does not provide forward-compatible
diagnostic history. This does not change dictation or meeting history formats.

### Selected-text transform stages

- selected-text capture;
- instruction audio capture and cleanup-only ASR;
- sidecar spawn/model-load and generation as separate timings;
- review-ready completion or a stable terminal failure, cancellation, or
  timeout.

The run starts only after the canonical pass wins the transform pipeline claim.
It closes at review-ready or the first terminal outcome. Apply and Undo happen
after that completion, so their measured duration and completed/failed outcome
are appended as bounded correlated follow-up records rather than changing the
run's single terminal outcome. Retry keeps #332's same pass ID and does not
create a competing trace identity.

### Voice Query stages

- instruction capture is presented as **Capture**;
- instruction ASR is presented as **Transcription**;
- sidecar spawn/load is reused as **Provider spawn** for the directly spawned
  CLI process;
- generation is the time to **First answer**;
- total processing is presented as **Total**.

The stored stage IDs remain the existing V1 enum, so old readers and rows do
not require a JSON schema bump. A terminal provider/configuration/query-path
failure maps to the stable `queryFailed` run error. Audio and transcription
failures continue to use their existing stable errors.

## Resource scopes

| Field | Scope and unit |
| --- | --- |
| Host CPU | Whole-host utilization, normalized to 0–100 percent |
| Main-process CPU | Murmur process utilization; 100 percent equals one logical core and multithreaded work may exceed 100 |
| Main-process RSS | Physical resident memory in bytes |
| Rust heap | Bytes in Murmur's dedicated Rust malloc zone on macOS |
| FFI/native heap | Bytes in all other malloc zones; not an RSS component and not a complete GPU/unified-memory measurement |
| Sidecar CPU/RSS | Signed local-LLM helper process only; sampled by its atomic resident PID, including model handshake |

The first host/process CPU observation needs a prior counter baseline and is
therefore unavailable rather than reported as zero. Rust/FFI heap breakdown is
unavailable on unsupported platforms. Accelerator identity is recorded, but
GPU or ANE utilization is not estimated.

Only one transform request can own the helper at a time. Resource samples in
the transform run's wall-clock interval are therefore attributable to that
pass; an idle/nonresident helper yields `unavailable { reason: noSamples }`.
Non-transform run summaries mark the sidecar scope `notApplicable`. A vanished
PID or failed process read is `sampleFailed`, and unsupported platforms report
`unsupportedPlatform`.

## Privacy

Persistent diagnostics never contain transcript, query, answer, or instruction text,
selected/proposed/replaced text, clipboard contents, paths or filenames, bundle
IDs, window titles, selected context, project/profile names, raw stderr, or
free-form native error messages. Voice Query records retain stderr presence
only. Errors are stable enums. Text-related sizes are bounded buckets.
Database contents are never uploaded. When a dictation diagnostics row still
cannot begin after its bounded retry, one content-free structured event reports
only `operation`, the safe error class, attempt count, and `recording_id`. The
telemetry layer independently rejects every other key and unknown string in
both debug and release builds; SQL, database paths, transcript, and audio can
never enter this event.

## Commands and events

| API | Purpose |
| --- | --- |
| `list_performance_runs` | Read newest supported V1 runs, bounded to 200 |
| `get_performance_run` | Read one V1 run by opaque ID |
| `get_performance_resource_window` | Read the persistent ten-minute sample window |
| `get_performance_store_health` | Read available/unavailable state and bounded recovery evidence |
| `recover_performance_store` | Explicitly retry initialization without restarting Murmur; destructive reinitialization requires confirmed caller intent |
| `clear_performance_diagnostics` | Clear only the diagnostics database |
| `show_diagnostics_window` | Show the persistent pop-out on an exact allowlisted tab |
| `performance-run-completed` | Live typed completion event |
| `performance-resource-sample` | Live typed one-second sample event |

The TypeScript guards reject unsupported schemas before UI code consumes them.
The Diagnostics Performance tab uses these samples for synchronized, explicitly
scoped host, main-process, and sidecar cards and charts. The Runs tab reads the
bounded records directly, then uses `get_performance_run` for detail. Its phase
waterfall preserves canonical stage order and availability but does not infer
absolute offsets that V1 does not record. Correlated Events navigation matches
the structured canonical correlation field rather than parsing event summaries.

Every Diagnostics tab has a **Pop out** action. It opens the persistent
Diagnostics window on the currently selected tab, allowing the main window to
remain navigable while events, resource samples, and UI latency samples update.
Closing the pop-out hides it so opening it again avoids another WebView cold
start.

The production regression watch counts exhausted
`performance.store_operation_failed` events by install, app version,
operation, and safe error class. It consumes only the already privacy-stripped
JSONL stream and collapses unrecognized labels to `unknown` before producing a
report or dashboard alert.

## Capture startup health

The Performance tab also derives a read-only, on-device microphone startup
signal from a dedicated rolling history of the 20 newest successful dictation
captures. Rust correlates `audio.fallback_started` with `audio.capture_ready`
only for the same owner within the bounded capture contract, then persists only
the finalized startup duration, whether fallback occurred, and an optional
stable backend enum. Persistence runs off the capture-ready path. The frontend
polls this small typed history instead of the general 500-event telemetry ring.
Idle heartbeats and system chatter therefore cannot evict capture health
evidence. On first upgrade, Murmur reconstructs safe finalized observations
once from the current and rotated structured event logs.

The signal considers the five newest observations. It reports degraded health
when all five captures required fallback or their median `startup_ms` is at
least two seconds.

This judgment does not change backend order, retry budgets, or any capture-path
behavior. The persistent record accepts only the stable `auhal` and `cpal`
backend enums; it contains no timestamps, capture-owner IDs, device labels,
device UIDs, transcript content, or free-form errors. Fewer than five successful
captures is reported as insufficient data rather than healthy or degraded.
Clearing Performance diagnostics also clears these retained observations.

## Cold model loads

The Performance tab's Live health section also shows how many of the recent
successful dictations in the bounded run history paid a cold model load, and
the median measured `modelLoad` stage across those runs. A run counts as cold
when recording-start preparation loaded the model instead of finding it
resident (`warmState: coldLoaded`); runs whose warm state is `unknown` are left
out. The line is derived in the frontend from the same content-free run records
as the Runs tab and changes no runtime behavior. See
[models.md](models.md#idle-release) for the setting that controls idle release.

## UI navigation latency

The Diagnostics **Latency** tab records content-free frontend transitions
separately from the Rust-owned pipeline run store. A transition begins in the
interaction handler before its React state update. It records:

- time to the destination's React layout-effect commit;
- time to the first `requestAnimationFrame`, the primary JS-visible response
  metric after the compositor-only History/Settings swap;
- the observed interval between the first and second animation frames, used to
  expose missed frames and display scheduling differences;
- time to a second `requestAnimationFrame`, used as the stable painted-frame
  proxy retained for regression continuity;
- the source and destination view IDs plus pointer, keyboard, or programmatic
  trigger;
- the app version, Git revision (including a dirty-worktree marker), and
  development/release build mode. `MURMUR_BUILD_ID` can override the revision
  label for named before/after profiling runs.

Murmur also emits User Timing marks and measures for each completed transition,
so the same route is visible in a Web Inspector performance recording.

The newest 500 V1 samples are retained in local WebView storage under
`murmur-ui-latency-v1`. Samples contain no transcript text, settings values,
paths, app identities, search text, or free-form errors. The Latency workspace
groups exact route edges and reports count, median commit, median/P95 first
frame, median frame count, and paint-proxy summaries. Build filtering and JSON
copy support local before/after comparisons. Clear removes only UI latency
samples.

The inline Advanced Diagnostics workspace is mounted only while its disclosure
is open. Its event and performance subscriptions are disabled while inactive,
the full bounded event buffer remains available for filtering and copying, and
only the newest 100 matching rows are rendered. This keeps the hidden Settings
surface quiet without changing the persistent pop-out window's live behavior.

## Production app-version comparisons

Issue [#430](https://github.com/georgenijo/murmur-app/issues/430) adds **Compare
app versions** to Diagnostics → Runs. This compares the bounded local production
dictation records, separately from imported Performance Lab and evaluation
reports in Reports. It does not query the log receiver or upload diagnostics.

Comparisons use only the newest 200 completed runs across all run kinds.
Older baseline rows are deleted as new dictation, file, transform, or Voice
Query runs complete. Version selectors and counts describe this retained
window, not a version's complete history. Compare soon after a release;
a baseline is not preserved as a durable snapshot. There is no separate
comparison archive or longer retention period.

New native dictation runs carry an optional `production` companion with its own
`schemaVersion: 1`. Older run JSON remains readable; a missing companion means
compatibility is unavailable and cannot produce a latency verdict. Unknown
companion versions, unknown fields, invalid identifiers, or invalid measurements
fail closed. The companion lives in the same active/completed run JSON, so the
existing 200-run retention, restart interruption, Clear epoch, and recovery
rules apply. Clear also drops pending in-memory observations; delayed updates
only update an existing active row and cannot recreate a cleared observation.

The capture worker sends bounded typed observations through the supervisor's
exact `AudioOwner::Dictation(recording_id)` checks. These preserve helper
resolve/signature/spawn times, stream-open time, first-callback wait,
start-to-first-retained-PCM, total readiness, confirmed stop-to-helper-exit,
fallback attempt/result, and stable failure evidence. Readiness and stop/error
checkpoints persist off the capture path. Completion records whether the actual
retained sample array was empty; a missing measurement never becomes zero.
The `audio.stop_tail_applied` log records the accepted stop trigger and applied
`release_tail_ms` (0 for explicit toggle stops with standby disabled; 50 for
all standby stops, held-key releases and conservative overlay/auto-stop/unknown
sources). The supervisor owns that tail;
helper stop-to-exit timing starts when the worker receives Stop and excludes it.
On the cold teardown path, a checked successful native stop and validated
complete drain permit an early handoff: `audio.sealed_recording_delivered` marks
the handoff before exit confirmation. Standby retains its 50 ms margin and
does not use Stop/Stopped or the sealed-drain handoff. The unchanged timed
standby snapshot is not a delivery fence for PCM delayed beyond 50 ms. `captureFinalization`
then measures through sealed-sample delivery and resampling; exit confirmation and thread join may
continue concurrently with inference while capture ownership stays busy.
The helper timing totals include observed attempts; stream-open and first-PCM
measurements describe the successful/current attempt. A failed attempt remains
visible even if fallback later succeeds. Stale-owner events and unconfirmed
termination mark worker-invariant evidence when their run is still retained in
the bounded pending set. A confirmed normal helper stop supplies healthy
termination evidence; unobserved invariants remain unavailable. This is not an
OS-wide count of arbitrary processes.

Compatibility matches local installation identity, numeric OS version,
architecture, release/development mode, microphone selection and cached device
class, model/backend/accelerator/warm state, capture backend/order, supported
runtime configuration, recording-duration bucket, and output-size bucket. The
installation identity is a random UUID retained in the diagnostics directory;
it contains no hostname, hardware serial, or device UID. Do not copy a whole
installation's diagnostics directory to another machine and treat it as local
comparison evidence. Device class is obtained only from the existing valid
inventory cache; a stale/unknown cache cannot establish compatibility. The
helper's undifferentiated external-device class combines USB, Bluetooth and
other transports, so it cannot establish compatibility either. Older companion
records with that external label are also excluded.

The configuration fingerprint contains only supported content-free settings
and capture backend order. It never hashes transcript, prompt, vocabulary,
learned correction, project, or profile text. Runs with custom context/rules,
IDE context, or external file-output configuration that cannot be compared on
that basis have an unavailable configuration identity. The UI states that these
runs cannot establish a compatible latency cohort. Missing/unknown warm state
and historical metadata also exclude a run. Separate compatible cohorts remain
separate; the UI never combines different microphone/configuration/size buckets
into a single release-regression verdict.

For each metric the UI uses successful runs with a measured value, reports
separate sample counts, and shows median, absolute delta and percentage delta.
Use **Swap versions** to reverse the baseline and candidate.
Small samples are preliminary. Nearest-rank p95 requires at least 20 measured
values per side for that metric; fewer samples cannot generate a p95 verdict.
Central policy requires both more than 20 ms and more than 15% for a median
warning, or both more than 30 ms and more than 20% for a p95 warning. A zero
baseline has no meaningful percentage delta and cannot satisfy a percentage
threshold by inference.

Within the retained window, version-wide success/failure counts, capture-error observations, fallback,
zero-sample success, and stale/unconfirmed worker evidence remain visible even
without enough compatible latency samples. They are labeled observations, not
proof that incompatible cohorts establish a release-caused latency regression.
An anomaly is labeled new only when a nonempty baseline provides known negative
evidence for every retained run. Missing baseline observations leave the label
at **Observed in candidate**. An absent capture-failure value remains unknown,
so it cannot by itself establish that no capture failure occurred.

For an explicitly requested comparison whose retained baseline predates this
companion, use the manual read-only procedure in `prompts/PROMPT_RELEASE.md`.
The in-app comparison becomes authoritative only when both selected cohorts
carry the required evidence. Release requests do not imply a performance study.

## Physical gesture startup timing

New dictation production companions optionally include `capture.gesture`:

- `pressToAcceptedMs`: first trigger press received by rdev to the command's
  accepted Idle → Starting transition (including detector arming and IPC);
- `doubleTapGapMs`: first release to toggle emission after the second release,
  including the second tap's hold; not the narrower up-to-down gap;
- `pressToFirstPcmMs` and `acceptedToFirstPcmMs`: first validated, retained,
  nonempty PCM at the host worker, including any fallback;
- `pressToReadyMs` and `acceptedToReadyMs`: native supervisor first-buffer
  readiness publication, including supervisor dispatch delay. This is capture
  readiness, not later context/model/UI readiness.

Each duration is a `MeasurementV1`: measured zero is valid; tokenless
non-keyboard starts have `notApplicable`; keyboard measurements that cannot be
correlated and a startup failing/cancelling before PCM or readiness have
`unavailable { reason: noSamples }`. Hold gestures have no double-tap gap.
Older records omit `gesture` entirely. There are no helper-preparation outcomes:
no speculative helper/capture work occurs during a tap or hold arming.

Correlation uses an opaque decimal-text token emitted with the keyboard event,
passed through the webview's keyboard start request, and claimed at native
command entry before every rejection or await. Tokenless toolbar/overlay starts
cannot claim a queued keyboard stamp. Rejected requests discard their own
claimed stamp, including busy file transcription. Accepted timestamps are
passed directly into the audio supervisor for that exact recording owner.
Eight bounded pending stamps and a two-second validity window handle interleaved
requests without assigning timing by the next recording ID. No key, token,
transcript, audio, device UID, or content-derived hash is persisted.

`audio.gesture_start_timing` logs `GestureAccepted`, `GesturePcm`, and
`GestureReady` with the recording ID and content-free durations. A first tap in Double Tap mode or an unpromoted Both-mode press
produces no capture/helper-start event. Existing `readyMs` retains its
supervisor-acceptance origin and must not be relabeled as physical press latency.

SQLite `user_version = 5` reserves the optional gesture companion separately
from #919's v4 `homophoneContext` stage. The transactional 4 → 5 metadata step
preserves tables and historical V1 JSON; v3 → v5 applies both additive steps.
Run and companion `schemaVersion` remain 1. Rust and TypeScript accept absent
historical `gesture` fields and validate all durations/statuses when present.

## Launch milestones

The structured event log and Diagnostics event ring contain five content-free
launch milestones, with monotonic `launch_elapsed_ms` measured from executable
`main` entry (or `run` entry on mobile). The process-start event is emitted once
telemetry is initialized, with elapsed zero; its log timestamp is the later
emission time. Use the elapsed field for launch comparisons.

| Event code | Boundary |
|------------|----------|
| `system.launch_process_start` | Clock origin at executable entry; backfilled after telemetry setup |
| `system.launch_native_setup_done` | Rust setup, including stores and tray, returned successfully |
| `system.launch_listener_armed` | Shared keyboard thread initialized and entering rdev; detectors still require frontend configuration/subscription |
| `system.launch_model_ready` | First ready ASR runtime, including frontend/recording fallback |
| `system.launch_frontend_hydrated` | Main settings, history/stats, and theme caches settled, immediately before React root rendering |

The listener event marks entry into the blocking rdev listener, not a verified
macOS event-tap receipt. `keyboard.listener_failed` reports a later start failure;
actual shortcut interaction establishes working input. A disabled app, incomplete
onboarding, unavailable Accessibility, missing settings, or a model without the
catalog startup-warm policy may omit early listener/model milestones. Failures
use the existing frontend path. There is no new SQLite run kind or retained
content; the telemetry boundary permits only the fixed code and elapsed integer.
