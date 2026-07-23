# Performance diagnostics data

Issue [#351](https://github.com/georgenijo/murmur-app/issues/351) defines the
versioned, local data layer used by the Diagnostics performance workspace.
Dictation, imported-file, and selected-text transform runs share this contract.
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

The database currently uses SQLite `user_version = 1`; run and resource JSON
records also carry `schemaVersion: 1`. A database created by a newer Murmur
build is preserved and treated as unavailable rather than rewritten. Unknown
record versions are not decoded as V1.

## Run contract

`PerformanceRunV1` contains:

- an opaque random `runId`;
- kind (`dictation`, `fileTranscription`, or `selectedTextTransform`);
- start/finish UTC timestamps and exactly one terminal outcome;
- the existing `recordingId`, a dedicated `fileRunId`, or the canonical
  `transformPassId`;
- catalog-backed model, backend, accelerator, and warm/cold state;
- typed stage measurements;
- content-free audio duration or bounded size/token fields;
- scoped resource summaries.

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

Persistent diagnostics never contain transcript or instruction text,
selected/proposed/replaced text, clipboard contents, paths or filenames, bundle
IDs, window titles, project/profile names, raw stderr, or free-form native error
messages. Errors are stable enums. Text-related sizes are bounded buckets.
There is no network upload or remote telemetry.

## Commands and events

| API | Purpose |
| --- | --- |
| `list_performance_runs` | Read newest supported V1 runs, bounded to 200 |
| `get_performance_run` | Read one V1 run by opaque ID |
| `get_performance_resource_window` | Read the persistent ten-minute sample window |
| `clear_performance_diagnostics` | Clear only the diagnostics database |
| `performance-run-completed` | Live typed completion event |
| `performance-resource-sample` | Live typed one-second sample event |

The TypeScript guards reject unsupported schemas before UI code consumes them.
The Diagnostics Performance tab uses these samples for synchronized, explicitly
scoped host, main-process, and sidecar cards and charts. The Runs tab reads the
bounded records directly, then uses `get_performance_run` for detail. Its phase
waterfall preserves canonical stage order and availability but does not infer
absolute offsets that V1 does not record. Correlated Events navigation matches
the structured canonical correlation field rather than parsing event summaries.
