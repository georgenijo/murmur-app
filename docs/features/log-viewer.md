# Structured Event System and Log Viewer

## Overview

All application logging goes through Rust's `tracing` crate, captured by a custom `TauriEmitterLayer` that routes every event to three destinations: an in-memory ring buffer, a persistent JSONL file, and real-time emission to all frontend windows. The log viewer is a dedicated Tauri window (not a modal) that displays these structured events with filtering, search, and transcription performance metrics.

## Telemetry Architecture (`telemetry.rs`)

The old `logging.rs` module has been replaced entirely by `telemetry.rs`. The `commands/logging.rs` file still exists as a thin command layer that delegates to `telemetry.rs` functions.

### TauriEmitterLayer

A custom `tracing_subscriber::Layer` that intercepts every tracing event in the application:

1. **Field collection** — A `JsonVisitor` collects all tracing fields into `serde_json` values.
2. **AppEvent construction** — Each event becomes an `AppEvent` struct:

```typescript
type AppEvent = {
  timestamp: string;
  stream: StreamName;   // "pipeline" | "audio" | "keyboard" | "system"
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

In release builds, all string-valued fields from `pipeline` target events are stripped from the `data` object. Only numeric fields survive. The `summary` (message) field is not stripped. This prevents transcription text from being persisted in structured log data.

### Pretty-Printed Log File

A separate human-readable log file (`app.log` or `app.dev.log`) is maintained via `tracing_appender`. This is the file returned by `get_log_contents`.

### Tracing Streams

Tracing targets map to stream names used for filtering:

| Target | Stream | Typical Events |
|--------|--------|----------------|
| `pipeline` | pipeline | Transcription timing, VAD results, model loading |
| `audio` | audio | Device selection, sample rates, audio levels |
| `keyboard` | keyboard | Hotkey detection, mode changes, listener lifecycle |
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

**Stream filter chips** — Colored toggle buttons for each stream (`pipeline`, `audio`, `keyboard`, `system`). Click to show/hide events from that stream. Default: `pipeline`, `audio`, `system` active.

**Level filter** — Toggle buttons for `info`, `warn`, `error`. All active by default.

**Event list** — Scrollable list with monospace font. Each row shows:
- Timestamp (time portion only)
- Stream chip (colored)
- Level label (uppercase, colored)
- Summary text

Rows with structured data are expandable — click to reveal a `<pre>` block with formatted JSON.

**Auto-scroll** — The list automatically scrolls to the bottom as new events arrive. If the user scrolls up (more than 40px from the bottom), auto-scroll disengages. Scrolling back near the bottom re-engages it.

**Copy All** — Copies all filtered events as text lines: `{timestamp} [{stream}] {LEVEL} {summary}`.

**Clear** — Clears all events (calls `clear_event_history` on the backend and clears the local buffer).

### Metrics Tab

Visualizes transcription performance data extracted from pipeline events where `summary === 'transcription complete'`.

**Four timing series:**

| Series | Color | Description |
|--------|-------|-------------|
| Total | stone-600 | End-to-end pipeline time |
| Inference | amber-500 | Backend transcription time |
| VAD | stone-400 | Voice activity detection time |
| Paste | slate-500 | Text injection time |

**Stat cards** — One per visible series showing the latest value, average, and a trend indicator (up arrow red, down arrow green, dash for flat). Trend threshold: 10% deviation from the average.

**Line charts** — Two SVG polyline charts:
- Upper chart (150px): Total + Inference timing
- Lower chart (120px): VAD + Paste timing

Y-axis auto-scales with "nice" round numbers and three tick marks. X-axis shows transcription index (1-based). Each data point has a dot marker.

The metrics view shows the last 20 transcriptions. The series legend is toggleable — click a series label to show/hide it (at least one must remain visible).

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

## Log Files

All files stored in `~/Library/Application Support/local-dictation/`:

| File | Format | Purpose |
|------|--------|---------|
| `app.log` / `app.dev.log` | Pretty-printed text | Human-readable log (read by `get_log_contents`) |
| `events.jsonl` / `events.dev.jsonl` | JSONL | Structured events, rotated at 5MB |
| `events.jsonl.1` | JSONL | Previous rotated JSONL file |

`clear_logs` removes all known log file variants, including legacy files (`frontend.log`, `transcription.log`, dated rolling files).
