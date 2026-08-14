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

The database currently uses SQLite `user_version = 2`; the v2 migration adds
Voice Query support without changing tables. Run and resource JSON records
remain backward-compatible at `schemaVersion: 1`. A database created by a newer Murmur
build is preserved and treated as unavailable rather than rewritten. Unknown
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
- model queue/lock wait and model load;
- inference/decode;
- aggregate deterministic transcript transformation and the existing
  content-free cleanup, Voice Commands, Smart Correction, Smart Formatting,
  IDE context, and CLI stage outcomes;
- optional file output;
- clipboard/paste;
- total post-stop processing.

### File stages

- decode/downmix/resample;
- VAD;
- model queue/load and inference/decode;
- the authoritative verbatim transcript-transform entry point;
- file return;
- total command processing.

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
