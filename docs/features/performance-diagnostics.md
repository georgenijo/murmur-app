# Performance diagnostics data

Issue [#351](https://github.com/georgenijo/murmur-app/issues/351) defines the
versioned, local data layer used by the Diagnostics performance workspace.
Dictation and imported-file runs share this contract.
Older records for retired flows remain readable without producing new records.

## Storage and retention

The Rust-owned SQLite database lives at:

```text
~/Library/Application Support/com.localdictation/diagnostics/performance.sqlite3
```

It is separate from logs, transcription history, settings, and Performance
Lab/evaluation reports. The store keeps:

- the newest 200 completed runs;
- active content-free lifecycle rows so early exits and restart interruption can
  close a run exactly once;
- the newest 600 one-second resource samples (a ten-minute window).

Completion and pruning share one transaction. On startup, a stale active row is
closed as `interrupted` with the stable `interruptedByRestart` code. Clearing
performance diagnostics removes only these runs and resource samples. It also
advances a clear epoch so an operation that began before Clear cannot reinsert
its old diagnostics when it eventually finishes.

The database currently uses SQLite `user_version = 9`; the v2 migration adds
Voice Query support and the v3 migration adds the dictation `releaseToStop`
and `releaseToPaste` stages, neither changing tables. Run and resource JSON records
remain backward-compatible at `schemaVersion: 1`. V4 adds `homophoneContext`
using the same additive-stage migration: a transaction updates `user_version`
without rewriting old JSON or changing tables. Historical runs omit this stage;
new readers preserve that absence. V5 adds optional production physical-gesture
capture timing in a separate 4 → 5 metadata migration. The v3 → v5 path applies
v4 then v5 in one transaction, preserving existing JSON bytes, including v4
homophoneContext rows. V6 adds the optional stream-open split and
`helperPreparation` capture fields in a 5 → 6 metadata step with the same
byte-preserving guarantees. Older builds refuse the newer database. A
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
- kind (`dictation`, `fileTranscription` for new runs);
- start/finish UTC timestamps and exactly one terminal outcome;
- the existing `recordingId` or a dedicated `fileRunId`;
- catalog-backed model, backend, accelerator, and warm/cold state;
- typed stage measurements;
- content-free audio duration or bounded size/token fields;
- scoped resource summaries.

Retired Voice Query records may carry `queryProcess` with only an integer
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
  content-free cleanup, Smart Correction, homophone context,
  Smart Formatting, IDE context, and CLI stage outcomes;
- `homophoneContext` measures the optional tier's complete wall time, with
  `completed` or `fallback` outcomes (including timeout, unavailable model,
  rejected answer, and adaptive cooldown). A cooldown fallback records its
  actual short duration, including a legitimate measured 0 ms after rounding;
  #1081 makes eligible live passes promptly unavailable without generation or
  session preparation; their stage outcome remains `fallback`. The existing
  content-free `homophone_context.pass` event records `outcome=unavailable`,
  distinct from `not_triggered` and `backoff`, with `flagged`, `engine_us` and
  `budget_ms`. No fields or enums change and no schema migration is needed.
  Disabled/non-live runs use `notApplicable`. An unavailable timing uses the
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
run. Toggle stops use the separate stop-press stages described below. Overlay,
auto-stop, and historical records have no hold release stages; they are omitted, never
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

## Resource scopes

| Field | Scope and unit |
| --- | --- |
| Host CPU | Whole-host utilization, normalized to 0–100 percent |
| Main-process CPU | Murmur process utilization; 100 percent equals one logical core and multithreaded work may exceed 100 |
| Main-process RSS | Physical resident memory in bytes |
| Rust heap | Bytes in Murmur's dedicated Rust malloc zone on macOS |
| FFI/native heap | Bytes in all other malloc zones; not an RSS component and not a complete GPU/unified-memory measurement |

The first host/process CPU observation needs a prior counter baseline and is
therefore unavailable rather than reported as zero. Rust/FFI heap breakdown is
unavailable on unsupported platforms. Accelerator identity is recorded, but
GPU or ANE utilization is not estimated.

Legacy helper-resource fields remain in the versioned record format for decoding
older reports. New samples leave them unavailable; current charts show the host
and main process only.

## Privacy

Persistent diagnostics never contain transcript, query, answer, or instruction text,
selected/proposed/replaced text, clipboard contents, paths or filenames, bundle
IDs, window titles, selected context, project/profile names, raw stderr, or
free-form native error messages. Retired Voice Query records retain stderr presence
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
| `list_performance_runs` | Read newest supported V1 runs, bounded to 200 (blocking pool, never the main thread) |
| `get_performance_run` | Read one V1 run by opaque ID |
| `get_performance_resource_window` | Read the persistent ten-minute sample window |
| `get_performance_store_health` | Read available/unavailable state and bounded recovery evidence |
| `recover_performance_store` | Explicitly retry initialization without restarting Murmur; destructive reinitialization requires confirmed caller intent |
| `clear_performance_diagnostics` | Clear only the diagnostics database |
| `show_diagnostics_window` | Show the persistent pop-out on an exact allowlisted tab. The pop-out is built on first open with that tab and destroyed on close; reopening a live pop-out emits the tab request |
| `performance-run-completed` | Live typed completion event |
| `performance-resource-sample` | Live typed one-second sample event |

The TypeScript guards reject unsupported schemas before UI code consumes them.
The Diagnostics Performance tab uses these samples for synchronized, explicitly
scoped host and main-process cards and charts. The Runs tab reads the
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
Older baseline rows are deleted as new dictation, file, or Voice
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
only update an existing row and cannot recreate a cleared observation.

The capture worker sends bounded typed observations through the supervisor's
exact `AudioOwner::Dictation(recording_id)` checks. These preserve helper
resolve/signature/spawn times, stream-open time, first-callback wait,
start-to-first-retained-PCM, total readiness, confirmed stop-to-helper-exit,
fallback attempt/result, and stable failure evidence. Readiness and stop/error
checkpoints persist off the capture path. Completion records whether the actual
retained sample array was empty; a missing measurement never becomes zero.
The `audio.stop_tail_applied` log records the accepted stop trigger, the applied
`release_tail_ms`, whether the tail was anchored to the key-release instant, and
the `tail_remaining_ms` at acceptance (0 for explicit toggle stops with standby disabled; 50 for
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
the bounded pending set and is in flight or bound to its own cold Stop.
A confirmed normal helper stop supplies healthy
termination evidence; unobserved invariants remain unavailable. This is not an
OS-wide count of arbitrary processes.

Delivery still completes and persists the performance run immediately, without
waiting for helper teardown. Alongside at most eight unfinished observations,
the accumulator retains up to 200 completed dictations, each bound to its exact
recording ID and completed run UUID. The supervisor separately binds the exact
logical recording owner when that recording's own cold stop actually sends
`Stop`, after its release tail. Only a `WorkerStopped` belonging to this bound
stop is forwarded to diagnostics. Standby handoff does not acquire the binding;
later standby disable, expiry, or recovery Stops cannot supply exit timing or
healthy termination evidence for the already delivered dictation. Resuming a
worker clears the binding before accepting the next recording owner, so that
owner's later cold stop can amend only its own run. Unconfirmed standby
retirement also cannot supply an invariant observation for the delivered run.
The supervisor sends an in-memory `RecordingStopIssued` observation at the same
accepted cold Stop. The diagnostics accumulator can acquire this binding only
before run completion. Both typed capture observations and
`InitializationFailed` → `production_capture_failure` reject completed runs
without it. The async persistence continuation rechecks the same eligibility
after acquiring the operation lock, so an in-flight checkpoint that races
standby completion cannot write later. Standby failures still reach ordinary
recovery/presentation handling; they cannot change the completed dictation's
`failureKind` or invariant evidence through either diagnostics route. This
binding is ephemeral and adds no persisted JSON field or schema version.
A late recording-stop `WorkerStopped` or worker-invariant observation schedules
an off-path SQLite transaction that rereads that UUID and
checks its dictation recording ID before updating only capture teardown
evidence. A confirmed exit fills `stopToWorkerExitMs` and sets
`workerInvariantViolation` to false unless a true violation was already
observed. Unconfirmed recording teardown forwards an invariant observation even
after sealed delivery; a subsequent confirmed exit cannot erase it. The transaction
preserves the original outcome, timestamps, stage durations, sample evidence,
resources, and retention order. It only updates existing rows, so Clear, pruning,
and store recovery cannot resurrect old diagnostics. Reopening or refreshing
Runs reads the amended evidence; the original completion event describes the
evidence available at delivery.

Atomic amendment keeps the completed result durable even if teardown never
finishes or Murmur quits before an exit event/persistence task runs. There is no
deferred completion to flush or timeout to wait for: `stopToWorkerExitMs: null`
and `workerInvariantViolation: null` retain the existing unknown/missing
semantics until evidence is observed and persisted. A quit can lose an
as-yet-unwritten late observation, but cannot lose the already completed run.
Eviction or restart leaves the last durable evidence intact. No synthetic zero,
healthy termination inference, new JSON field, schema migration, or SQLite
`user_version` change is introduced. Standby snapshots still have no
per-recording helper exit measurement; ordinary exit-before-delivery runs
persist their exit evidence at completion as before.

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
project, or profile text. Runs with custom context/rules,
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
Older records omit `gesture` entirely.

A cancelled run whose `pressToAcceptedMs` is close to 200 ms is usually a
Both-mode hold promotion (`MAX_HOLD_DURATION_MS` is 200), not a single tap. A
`doubleTapGapMs` of `notApplicable` distinguishes a hold from a double tap.

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
`GestureReady` with the recording ID and content-free durations. Existing
`readyMs` retains its supervisor-acceptance origin and must not be relabeled as
physical press latency.

### Clean taps and silent first-tap helper preparation

A tap is clean only when no other key (modifiers included), mouse button, or
scroll-wheel event occurs during either press. Any of those cancels the tap.
Pointer movement alone does not.

In Double Tap or Both mode, a first clean release of the double-tap key may
silently prepare an idle capture helper. Preparation verifies its signature,
spawns it, completes hello, then sends a distinct `PrepareInput` command for
bounded, nonopening HAL input-UID/default lookup. Both host and helper require
an existing microphone grant; neither requests permission. No Audio Unit is
created, configured, initialized, or started, no Start is sent, and no PCM or
Murmur UI exists before confirmation. The contract is indicator- and prompt-free;
final native interaction on the rebuilt signed helper must verify that contract.
Preparation runs only while audio is idle, with no dictation starting/recording
and no live standby. Admission and the preparation-slot claim hold the same
`recording_transition` as capture claims. Every launch still verifies the signature.

The confirmed start must match gesture token, listener, input topology, and
settings generation. Device/Smart Auto settings writes revoke preparation even
without a topology notification. At Start the helper rechecks the current default,
exact selected UID, and input capability before reusing its passive resolution;
a mismatch uses ordinary post-confirmation resolution, never a different default
for a pinned input. CPAL keeps its existing confirmed-start resolution path.

Second-tap expiry, chords, mode/key/settings/topology changes, and another
feature's capture claim invalidate preparation immediately, before asynchronous
context capture or native setup. Expiry is identity-scoped so a delayed timer
cannot kill a newer gesture. The 1.5 s deadline starts at the request, including
spawn, hello, and passive resolution. In-flight pipe reads poll cancellation;
creation rechecks cancellation after signature verification and before spawn.
One single-flight preparation worker owns any child through confirmed teardown,
including failed hello, invalidation during spawn, or delayed termination. No new
preparation is launched while that worker is cleaning up; cleanup never hands a
child to a fallibly spawned detached thread.

Each dictation's first capture attempt records `capture.helperPreparation`:
`adopted`, `notPrepared`, `stillPreparing`, `identityMismatch`, or `expired`.
An adopted attempt sends no `HelperReady`, because helper resolve, signature,
and spawn were paid before acceptance. Its `helperResolveMs`,
`helperSignatureMs`, and `helperSpawnMs` remain null. An expired or abandoned
preparation has no run record. Those outcomes appear only as content-free
`audio.helper_prepare_*` event codes (`audio.helper_prepared`,
`audio.helper_prepare_adopted`, `audio.helper_prepare_expired`,
`audio.helper_prepare_abandoned`, `audio.helper_prepare_skipped`,
`audio.helper_prepare_failed`, and
`audio.helper_prepare_termination_unconfirmed`). The comparison's
**Prepared helper adopted** observation counts hits. Runs without an outcome
are counted as unavailable, and the observation never raises a warning.

### Stream-open split

`streamOpenMs` stays the host-observed stream-build phase. New records also
split it using host-observed Entered → Completed helper setup brackets:

- `deviceResolutionMs`: `DeviceResolution`;
- `audioUnitNewMs`: AUHAL component lookup/instance creation (`AudioUnitNew`);
- `audioUnitInitializeMs`: the separate AUHAL `AudioUnitInitialize` call;
  both AUHAL-only fields are absent for CPAL;
- `streamConfigureMs`: sum of the configuration steps between creation and
  start (`EnableInputIo`, `DisableOutputIo`, `SetCurrentDevice`,
  `FormatConfiguration`, `CallbackInstallation`, or CPAL `DefaultConfig` and
  `StreamBuild`);
- `streamStartMs`: `StreamStart` (`AudioOutputUnitStart` or cpal `play`).

Like `streamOpenMs`, these describe the latest capture attempt. A new attempt
clears them, so a fallback never mixes steps from two backends. The helper
duration fields still sum across attempts. Older records omit these five fields
and `helperPreparation`. A missing value is unavailable, never zero.

All AUHAL brackets run after confirmed Start. Creation uses the pinned safe
owning `coreaudio-rs 0.14.2` uninitialized constructor; its separate `initialize`
call immediately follows, preserving the old new → initialize → IO/device/format
configuration → callback → start order. The legacy HAL inventory helpers stay
on their existing dependency. No raw instance is transmuted into the older
wrapper's private layout, and an initialization failure still disposes its owner.

The local signed stopped-AUHAL experiment for #933 did **not** justify shipping
speculative configuration or a reorder: setting CurrentDevice with undetermined
TCC prompted even without Start. In 20 granted trials configure-before-initialize
had new-to-first-callback p50 33.58 ms versus 34.59 ms for the old order (~1 ms);
initialization was ~3.9 ms, and pre-initialize/hold delayed the later start.
RunningSomewhere stayed zero and callbacks absent during holds, but visual
orange-indicator proof was incomplete. These scratch observations are not a
production latency gain or an indicator-free proof. Therefore only passive lookup
is prepared; configuration, initialization, and capture remain post-confirmation.

SQLite `user_version = 5` reserves the optional gesture companion separately
from #919's v4 `homophoneContext` stage. The transactional 4 → 5 metadata step
preserves tables and historical V1 JSON; v3 → v5 applies both additive steps.
Run and companion `schemaVersion` remain 1. Rust and TypeScript accept absent
historical `gesture` fields and validate all durations/statuses when present.
The 5 → 6 step does the same for the stream-open split and `helperPreparation`.

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

## Early-final shadow measurement (#1081 phase 3)

SQLite `user_version = 7` adds `stopPressToRelease`, `stopReleaseToCommand`, and
an optional dictation `shadow` companion (`schemaVersion: 1`). The metadata-only
6 → 7 migration preserves historical JSON bytes. Run JSON remains V1; old
records may omit the companion and stages. The metadata-only 7 → 8 migration
adds the bounded repeat-skip enum without
rewriting historical JSON. The metadata-only 8 → 9 migration adds
`notMeasuredInApp`, preserving historical comparisons and skipped states. Older
builds refuse this database.

Double-tap/Both-mode single-tap stops stamp native press and release instants.
`stopPressToRelease` measures the gesture, and `stopReleaseToCommand` measures
release through the webview to native stop entry. The pending press belongs to
the detector and is cleared by reset, chords, excessive holds, or reconfiguration.
Only an emitted stop publishes it; the accepted stop consumes it for the same
generation within ten seconds. Only a resolved double-tap toggle stop attributes
that stamp; an overlay, auto-stop, other stop, or matching hold release discards
it without recording double-tap stages or press evidence. Hold-to-talk retains
its existing release stages.
The press reads a nonblocking retained-PCM sample count, without copying audio;
contention records an unavailable (`null`) cursor. The count represents PCM
already received at the host, not samples still buffered inside the helper.

The companion records only numbers, booleans, and fixed enums:

- `pressSample`, `pauseSample`: 16 kHz prefix endpoints, or null;
- `trailingSilenceBeforePressMs`, `trailingSilenceBeforeStopMs`: silence after
  the latest preceding padded full-clip VAD segment (zero within a segment);
  before the first segment, measure from clip start, since complete VAD ranges
  prove that prefix is silent. Missing VAD evidence remains null;
- `pressInputMatches`, `pauseInputMatches`: byte-exact equality of separately
  filtered prefixes against the final filtered PCM, or null when unavailable;
- `previewHeldBackendAtPress`: whether display-only preview actually held the
  shared backend at press, not whether a preview tick was merely scheduled;
- `determinism`: `disabled`, `pending`, `compared`, `backendUnavailable`,
  `superseded`, `decodeFailed`, `unsupportedBackend`, `notAttempted`,
  `skippedForegroundLatencyRisk`, or `notMeasuredInApp`; all new live records
  use `notMeasuredInApp`, and older comparison statuses stay readable;
- `decodeMatches`: the in-memory hash comparison result only when `compared`.

The pause endpoint is the first 300 ms point in the latest qualifying gap
between padded speech segments, including trailing silence. Prefix VAD uses the
same recording-start threshold and unchanged final PCM recipe. Full-clip bounds
are recomputed in the worker, and used only if that PCM is byte-identical to the
already-delivered final input; final VAD collects no shadow bounds. VAD-disabled,
failed/missing VAD, absent clean press, and absent qualifying pause remain
unavailable rather than becoming false or zero. This phase targets Core ML;
Whisper and imported-file transcription are unchanged.

All prefix passes run after confirmed delivery, off the pipeline, after the
completed run UUID is durable. The pipeline returns its already-owned filtered
PCM; after its Idle guard has dropped, the caller captures `delivered_at` and
`totalProcessing`, then moves both original and filtered buffers into the shadow
worker. There is no shadow PCM copy, text capture, or hashing. A boundary assertion
and a move/ordering regression test keep shadow setup after the Idle observation
and delivery timestamp. One process-wide shadow permit bounds retained audio/work.
Runs without an admitted job still record `notMeasuredInApp`; completion starts
with unavailable prefix evidence and a later atomic UPDATE fills it. Refresh
Runs to read that amendment. Clear/pruning cannot recreate a row, and a reused recording ID cannot
receive an old UUID's result. A quit before the amendment can leave the original
unavailable prefix evidence. No shadow result changes delivered text or supplies a
future decode.

The Runs detail view renders the numeric/boolean shadow evidence and repeat
status. False is shown as **No**, a missing measurement as **Unavailable**, and
an absent historical companion as **Shadow evidence not recorded for this run**.
Not measured in app, skipped, disabled, pending, not-attempted, unsupported, and
older completed comparisons have separate labels; skipped work never appears
as a failed or false comparison.

Live determinism is **not measured in app**, regardless of environment or settings.
The old `MURMUR_SHADOW_DETERMINISM` switch has no code path. Shadow work never
acquires, waits for, holds, loads, or calls the transcription backend. The shared
final decoder is non-preemptible, so even an idle admission could delay a later
foreground request and is prohibited. The worker has only VAD capabilities.
Concurrency tests hold shadow VAD work open and verify admission for dictation,
file transcription, meetings, preview, model preparation, and other backend
users, including an already-held backend. `decodeMatches` is always null on new
live records. The moved PCM is dropped with its job.

Determinism evidence is instead an ignored, developer-only Rust model test,
compiled only into the macOS Apple Silicon test harness, never the app:

```bash
cd app/src-tauri
cargo test synthetic_final_decode_is_deterministic -- --ignored --nocapture --test-threads=1
```

It requires the installed Parakeet Core ML and Silero models and macOS `say`.
It generates 32 synthetic clips in a temporary directory: 16 short clips and 16
clips whose VAD-filtered duration exceeds 15 seconds. It uses the app's final VAD
recipe and `decode_final_transcript`, including its existing retry behavior. Each
clip is decoded twice, with no warm-up before the first pair after model load.
The test then unloads and reloads the model and repeats all pairs. It compares
SHA-256 hashes in memory, requires nonempty successful decodes, and emits only
counts/booleans; neither text nor hashes are printed or persisted. It accepts no
user audio or external corpus path and removes generated audio on exit. This is
evidence for those synthetic inputs, not a guarantee for every possible input.

## Delivery attribution

New dictation runs optionally carry `deliveryTiming` in the active/completed
V1 envelope, with its own `schemaVersion: 1`. Historical JSON is unchanged and
omits it. Durations use monotonic **microseconds**, wrapped in the existing
measurement status contract; unavailable is never a numeric zero. The existing
`clipboardPaste` stage and `inject timing` log retain their original meanings.
`clipboardPaste` measures the whole delivery stage, not visible insertion.

The companion separates accepted/actual route (normal, Smart current field,
Smart handoff, or clipboard only), configured delay, permission, and final
injection/refusal/error outcome. `acceptedRoute` describes the recording-start
snapshot. `actualRoute` describes the preparation path: `ready` is
`smartCurrent`, while `activationRequested` is `smartHandoff`, including a
recording that started in the current field but switched away while speaking.
Normal/clipboard fallbacks use the post-preparation snapshot. Without a
confirmed Smart preparation path, the actual route is unavailable rather than
inferred from the start flag. Historical rows retain their recorded values;
there is no backfill or change to the schemaVersion 1 shape.

Disabled auto-paste makes no accessibility-permission query and has no Rust
permission cache. Its `accessibility` measurement remains unavailable: filling
it would add an `AXIsProcessTrusted` call on the delivery path or substitute
potentially stale UI state. Existing permission checks and their ordering are
unchanged.

Whole-phase fields distinguish preparation
queue, delivery-owner lock wait and work; activation settling; injection queue,
owner wait and work; cleanup; and receiver/outcome publication. `toPostUs` ends
immediately before actual key-down posting and is unavailable if no post was
observed before the record closed. A posted shortcut still is not insertion
acknowledgment. Error, timeout and early-return paths close the record too.

Native child measurements cover clipboard write, numeric pasteboard generation
queries (including generation capture), focused-field query/transport/AX status,
all three ordered target verifications with independent lookup retry counts,
event construction, final live-focus proof/status, key-down, key hold and key-up.
`nativePost` and `retryPost` separate event attempts; retry focus and target
checks have their own fields. Each focus query's `axStatus` records the last
attempted AX operation, including `AXSubrole` after a successful `AXRole` query;
`timedOut` retains any timeout observed throughout that query. This applies to
both the initial and retry query and does not change their role classification
or fallback policy. The existing record shape and nested `schemaVersion: 1`
are unchanged; historical rows remain readable.

`nativePasteRetryCount` is incremented when the
retry branch is entered, before its unchanged 100 ms wait, including a retry
that subsequently refuses. It is distinct from native identity lookup retries.

Smart child timers measure preparation validation (including its guarded
activation work), insertion validation, post-settle, structural readback and
restoration. Preparation, confirmation and rollback use bounded outcomes;
timeout, cancellation and expiry remain explicit. Child timers are included in
their parent phase; do not sum them again into the stage total. Clipboard write
includes capturing its generation, so that first generation query also overlaps
the pasteboard-query aggregate. A field not reached retains a status instead of
inventing a zero. At timeout, open timers retain elapsed-so-far through sealing;
metadata not yet returned remains unavailable. A finished final-focus proof
retains its atomically frozen duration even if sealing happens before key-up
publishes it. Freezing adds no accumulator lock between proof and posting.
Posting progress is registered before that proof and published with atomic
stores: sealing during key-down, the hold, or key-up preserves `toPostUs` and
the elapsed time of the active key-event leaf. Completed leaves keep their
measured durations. A partial key pair leaves the post outcome unavailable;
only completion of key-up establishes `posted`, still without insertion
acknowledgment. No schema fields or delivery checks change.
Those partial durations do not claim that native IPC completed.

Normal delivery verification and Smart preparation's process-only samples no longer query diagnostic-only `AXFocusedWindow` during
verification. `windowDiagnostic: unavailableOffPath` and an unavailable duration
make that absence explicit; its window relation is `unknown`. The fresh
process-instance samples, all three normal verifier calls, secure-field policy, final
live-focus proof and pasteboard equality remain authoritative. Smart's exact
window/field checks are separate and unchanged.

Each recording owns a distinct collector. Completion seals it before persistence
into that recording's correlation, including when diagnostics initialization is
still pending. Late main-thread callbacks cannot amend a sealed collector or a
newer run; Clear/pruning cannot be undone by an update. Only numbers, booleans,
measurement statuses and bounded enums enter this companion. No application or
window strings, content, paths or native error text are accepted. This change
does not establish a new paste p95 or a visible-insertion latency claim.
