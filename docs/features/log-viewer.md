# Structured Event System and Log Viewer

## Overview

All application logging goes through Rust's `tracing` crate, captured by a custom
`TauriEmitterLayer` that routes every event to three destinations: an in-memory
ring buffer, a persistent JSONL file, and real-time emission to all frontend
windows. The dedicated Diagnostics window keeps these structured Events beside a
typed live Performance view and bounded per-run history.

## Telemetry Architecture (`telemetry.rs`)

The old `logging.rs` module has been replaced entirely by `telemetry.rs`. The `commands/logging.rs` file still exists as a thin command layer that delegates to `telemetry.rs` functions.

### TauriEmitterLayer

A custom `tracing_subscriber::Layer` that intercepts every tracing event in the application:

1. **Field collection** — A `JsonVisitor` collects all tracing fields into `serde_json` values.
2. **AppEvent construction** — Each event becomes an `AppEvent` struct:

```typescript
type AppEvent = {
  timestamp: string;
  stream: StreamName;   // "pipeline" | "audio" | "keyboard" | "transform" | "system"
  level: LevelName;     // "trace" | "debug" | "info" | "warn" | "error"
  summary: string;      // the tracing message
  data: Record<string, unknown>; // structured fields
};
```

3. **Ring buffer** — Pushed to an in-memory `VecDeque` capped at 500 events (FIFO eviction when full).
4. **JSONL file** — Appended as a single JSON line to `events.jsonl` (release) or `events.dev.jsonl` (dev).
5. **Frontend emission** — Emitted as an `app-event` Tauri event to all windows.

### Three Outputs

| Output | Format | Capacity | Purpose |
|--------|--------|----------|---------|
| Ring buffer | `AppEvent` structs | 500 events | Fast in-memory access for `get_event_history` |
| JSONL file | One JSON object per line | 5MB before rotation | Persistent log, survives restarts |
| `app-event` emission | `AppEvent` payload | Real-time stream | Live updates in log viewer |

### JSONL Rotation

When the JSONL file exceeds 5MB, it is rotated — renamed to `.jsonl.1` — and a fresh file is started. On startup, the ring buffer is pre-populated with up to 500 events from the existing JSONL file.

### Privacy Stripping

In release builds, all string-valued fields from `pipeline` target events are stripped from the `data` object. Only numeric fields survive. For the `transform` stream in both debug and release builds, each string must match an explicit key-specific enum/bucket vocabulary; unknown keys or values are dropped. Numeric and boolean diagnostic fields are retained. The `summary` (message) field is not stripped and transform summaries are constant. This prevents transcription or transform content from being persisted in structured log data.

### Pretty-Printed Log File

A separate human-readable log file (`app.log` or `app.dev.log`) is maintained via `tracing_appender`. This is the file returned by `get_log_contents`.

### Tracing Streams

Tracing targets map to stream names used for filtering:

| Target | Stream | Typical Events |
|--------|--------|----------------|
| `pipeline` | pipeline | Transcription timing, VAD results, model loading |
| `audio` | audio | Device selection, sample rates, audio levels |
| `keyboard` | keyboard | Hotkey detection, rejection reasons/timing, mode changes, listener lifecycle |
| `transform` | transform | Correlated transform key, state, capture, audio, effects, and terminal outcomes |
| `system` | system | Startup, permissions, updates, resource usage |

### Frontend Logging

The `flog` utility (`lib/log.ts`) routes frontend log messages through the Rust tracing system via the `log_frontend` command. Messages appear in the log viewer alongside Rust-originated events.

```typescript
flog.info("overlay", "double-click detected");
flog.warn("settings", "device not found", { device: name });
flog.error("updater", "download failed", { error: msg });
```

Messages are formatted as `[tag] message` with optional JSON data. Calls are fire-and-forget (errors silenced).

## Log Viewer Window

The log viewer is a separate Tauri window (`label: "log-viewer"`, 800x600, min 600x400) opened via the `open_log_viewer` command. It hides on close rather than being destroyed.

### Events Tab

The primary view for browsing structured events.

**Stream filter chips** — Colored toggle buttons for each stream (`pipeline`, `audio`, `keyboard`, `transform`, `system`). Click to show/hide events from that stream. Default: `pipeline`, `audio`, `transform`, `system` active.

**Correlation filter** — Select `run_id`, `recording_id`, `file_run_id`, or
`transform_pass_id` and enter an exact value. Run detail opens Events with the
canonical correlation already selected. The filter matches structured fields
only; it never parses summary text.

**Level filter** — Toggle buttons for `info`, `warn`, `error`. All active by default.

**Event list** — Scrollable list with monospace font. Each row shows:
- Timestamp (time portion only)
- Stream chip (colored)
- Level label (uppercase, colored)
- Summary text

Rows with structured data are expandable — click to reveal a `<pre>` block with formatted JSON.

**Auto-scroll** — The list automatically scrolls to the bottom as new events arrive. If the user scrolls up (more than 40px from the bottom), auto-scroll disengages. Scrolling back near the bottom re-engages it.

**Copy filtered Events** — Copies all filtered events as text lines including
compact JSON structured data, so correlation IDs, outcomes, timings, and error
codes remain in pasted diagnostic evidence.

**Clear Events** — Clears the event ring buffer only. It does not clear
Performance runs or resource samples.

### Performance Tab

Performance replaces the former event-derived Metrics view. It hydrates from the
typed persistent resource window, subscribes to typed live samples, and never
reconstructs timings from human-readable events.

**Live health** names the current local pipeline state, configured model,
backend, and accelerator identity. Accelerator identity is not utilization.
`Accelerator utilization` is explicitly unavailable because Murmur has no
production whole-device GPU or ANE percentage.

**Scoped resources** keep each measurement in its typed scope:

- Host CPU is whole-host utilization normalized to 0–100%.
- Murmur CPU is main-process utilization; 100% equals one logical core.
- Main-process RSS is physical resident memory.
- Rust heap and FFI/native heap are allocator-zone measurements, not additive
  RSS components or GPU memory.
- Sidecar CPU and RSS refer only to the local LLM helper process.

CPU and memory charts use one keyboard-operable timeline cursor. Missing or
failed measurements create chart gaps. A measured zero remains zero;
`notApplicable` and `unavailable` remain distinct text states.

### Runs Tab

Runs reads at most the newest 200 `PerformanceRunV1` records. Dictation, file
transcription, and selected-text transform are first-class kinds. Kind and
outcome filters cover success, no-speech, cancelled, timed-out, failed, and
interrupted terminal outcomes.

Each row shows timestamp, runtime identity, outcome, privacy-safe input shape,
total start-to-terminal latency, real-time factor or token throughput where
meaningful, and measured resource peaks. Selecting the row uses
`get_performance_run` and opens its detail view.

**Phase waterfall** displays the canonical stage order and duration contribution.
The V1 schema does not record absolute stage offsets, so the UI does not invent
them. Measured zero is a zero-width marker; unavailable and not-applicable rows
use explicit text. Transform apply/undo records are shown separately as
correlated follow-ups.

**Resource summary** reports start, average, peak, and end for every typed host,
main-process, and sidecar range with its scope named.

**Clear Performance Data** requires confirmation and clears only the performance
database. Events, logs, transcription history, settings, knowledge, and
benchmark/evaluation reports are untouched. Loading, never-recorded, cleared,
filtered-empty, error, stale/partial, unsupported, and unavailable states are
presented separately.

### Reports Tab

Reports imports local Performance Lab benchmark or `murmur-eval` JSON through
an explicit file picker and compares two normalized reports. The picker rejects
files larger than 8 MiB before reading; parser/schema/collection failures use
fixed content-free messages. Imported paths, filenames, raw JSON, transcript
fields, and evaluation stage text are neither retained nor logged.

Saved Performance Lab history appears as local choices. Imported reports remain
in memory for the Diagnostics session only, with at most 20 session imports.
Clear imports removes only those in-memory choices and does not alter source
files, local benchmark history, Events, or Performance data.

Compatibility blockers are shown before deltas and recommendation eligibility.
Blockers suppress metrics entirely; machine and app-version warnings permit
deltas but disable recommendations. Missing metrics remain unavailable,
measured zero remains zero, and percentage delta from a zero baseline is
explicitly unavailable.

### Event Store (`useEventStore`)

The frontend event buffer is managed by the `useEventStore` hook:

- **Hydration** — On mount, fetches existing events via `get_event_history`.
- **Live streaming** — Listens for `app-event` Tauri events and appends to the buffer.
- **Batched rendering** — Uses `requestAnimationFrame` to coalesce rapid event bursts into a single React state update.
- **Capacity** — 500 events maximum (`MAX_EVENTS`).
- **Filtering** — `getByStream(stream)` and `getByLevel(level)` methods.

## Commands

| Command | Description |
|---------|-------------|
| `open_log_viewer` | Shows and focuses the log-viewer window |
| `get_log_contents` | Returns the last N lines from the pretty-printed log file |
| `clear_logs` | Removes all log files and clears the in-memory event ring buffer |
| `log_frontend` | Routes a frontend message through Rust tracing (INFO/WARN/ERROR) |
| `get_event_history` | Returns all events from the in-memory ring buffer (up to 500) |
| `clear_event_history` | Clears the in-memory event ring buffer |
| `list_performance_runs` | Returns at most 200 newest typed performance runs |
| `get_performance_run` | Returns one typed run by opaque `run_id` |
| `get_performance_resource_window` | Returns the bounded typed resource window |
| `clear_performance_diagnostics` | Clears only local Performance runs and samples |

## Log Files

All files stored in `~/Library/Application Support/local-dictation/logs/`:

| File | Format | Purpose |
|------|--------|---------|
| `app.log` / `app.dev.log` | Pretty-printed text | Human-readable log (read by `get_log_contents`) |
| `events.jsonl` / `events.dev.jsonl` | JSONL | Structured events, rotated at 5MB |
| `events.jsonl.1` / `events.dev.jsonl.1` | JSONL | Previous rotated JSONL file |

`clear_logs` removes all known log file variants, including legacy files (`frontend.log`, `transcription.log`, dated rolling files).

For read-only diagnostics across both release and dev logs, including MCP setup
and the single user-level registration model, see
[`tools/murmur-diag/README.md`](../../tools/murmur-diag/README.md).
