# Performance diagnostics data

Issue [#351](https://github.com/georgenijo/murmur-app/issues/351) defines the
versioned, local data layer used by the Diagnostics performance workspace.
Phase A covers dictation and imported-file runs. Selected-text transform and
sidecar correlation remain reserved until #332 supplies the canonical
`transform_pass_id` and one typed trace source.

## Storage and retention

The Rust-owned SQLite database lives at:

```text
~/Library/Application Support/com.localdictation/diagnostics/performance.sqlite3
```

It is separate from logs, transcription history, settings, personal knowledge,
and Performance Lab/evaluation reports. The store keeps:

- the newest 200 completed runs;
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
- kind (`dictation`, `fileTranscription`, or reserved
  `selectedTextTransform`);
- start/finish UTC timestamps and exactly one terminal outcome;
- the existing `recordingId`, a dedicated `fileRunId`, or the future canonical
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

The selected-text capture, instruction-ASR, sidecar-load, generation,
review-ready, apply, and undo stage names are reserved in V1 but Phase A does
not emit them.

## Resource scopes

| Field | Scope and unit |
| --- | --- |
| Host CPU | Whole-host utilization, normalized to 0–100 percent |
| Main-process CPU | Murmur process utilization; 100 percent equals one logical core and multithreaded work may exceed 100 |
| Main-process RSS | Physical resident memory in bytes |
| Rust heap | Bytes in Murmur's dedicated Rust malloc zone on macOS |
| FFI/native heap | Bytes in all other malloc zones; not an RSS component and not a complete GPU/unified-memory measurement |
| Sidecar CPU/RSS | Reserved as unavailable in Phase A; non-transform run summaries mark the scope not applicable |

The first host/process CPU observation needs a prior counter baseline and is
therefore unavailable rather than reported as zero. Rust/FFI heap breakdown is
unavailable on unsupported platforms. Accelerator identity is recorded, but
GPU or ANE utilization is not estimated.

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
The current resource cards use these samples and label host CPU, Murmur CPU,
Murmur RSS, Rust heap, and FFI/native heap explicitly. The Runs table and
waterfall remain #352, not this data-layer issue.
