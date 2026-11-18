# Structured Event System and Performance Workspace

## Overview

All application logging goes through Rust's `tracing` crate, captured by a custom
`TauriEmitterLayer` that routes every info-or-higher event to three destinations: an in-memory
ring buffer, a persistent JSONL file, and real-time frontend emission.
`events.jsonl` is the single record of those events. The human-readable
`app.log` repeats only warn and above (or info as well while debug logging is
on, plus the debug lines themselves), so each event is no longer stored twice.
`app.log` is a local diagnostic file, not a privacy boundary: its lines pass the
key-identity redaction, and debug lines also pass the structured-event sanitizer,
but a warn or error message is otherwise written as its author wrote it. The
embedded **Settings → Model → Advanced** workspace keeps these structured Events
beside a typed live Performance view and bounded per-run history.

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
  data: Record<string, unknown>; // structured fields; may include a stable event_code
};
```

3. **Ring buffer** — Pushed to an in-memory `VecDeque` capped at 500 events (FIFO eviction when full).
4. **JSONL file** — Queued to the writer thread, which appends one JSON line per event to `events.jsonl` (release) or `events.dev.jsonl` (dev).
5. **Frontend emission** — Emitted as an `app-event` Tauri event to all windows.

### Three Outputs

| Output | Format | Capacity | Purpose |
|--------|--------|----------|---------|
| Ring buffer | `AppEvent` structs | 500 events | Fast in-memory access for `get_event_history` |
| JSONL file | One JSON object per line | 5MB before rotation | Persistent log, survives restarts |
| `app-event` emission | `AppEvent` payload | Real-time stream | Live updates in the Performance workspace |

### Log levels and debug logging

The default level is `info`. Debug is opt-in and local:

- `MURMUR_LOG=debug` in the environment turns it on for that launch.
- `debug-logging.on`, an empty file in the log directory (`~/Library/Application Support/local-dictation/logs/` for production), turns it on at launch and while the app runs. Create or remove it and the level follows within about three seconds; no rebuild or restart is needed. `telemetry::set_debug_logging` writes it for a future Settings control. There is no Tauri command or Diagnostics toggle yet.

Debug events are admitted only from Murmur's own targets (`pipeline`, `audio`, `system`, `keyboard`, `meeting`, `transform`, `overlay`, `recording`, `hunt` and the `ui_lib` module paths); dependency crates stay at `info`. A debug or trace event is written only to the local `app.log`. It never reaches `events.jsonl`, the ring buffer, the Events tab, capture health or the `app-event` emission, and the log shipper also drops any `debug` or `trace` line it finds. Its fields pass through the release-strength structured-event sanitizer in every build, so `pipeline` strings and unknown fields are dropped; the message keeps its text with key identity redacted, as info lines in `app.log` always have.

### Off-thread JSONL writes

The `events.jsonl` write is not done on the thread that logged. `record_event` sanitizes the event, updates capture health and the ring buffer, emits it, and hands it to a bounded queue (4,096 events, `telemetry/writer.rs`). One `event-log-writer` thread drains the queue in order, joins up to 256KB of whole lines into a single `write` to the rotating file, and repeats. Ordering is the queue's FIFO order. Sending never blocks: a full queue drops the event, counts it, and the writer records the count as one `Log events dropped` line (`dropped_events`) ahead of the next batch. A batch the file rejects counts every event in it as lost (not one) and the count carries over to the next successful write. A flush reports failure if any event write was rejected since the previous flush, even when the loss note itself was then written successfully; the flush that reports it acknowledges it. Nothing is buffered beyond the queue, so a killed process loses at most the events still queued. The app flushes the queue on exit (`RunEvent::Exit` calls `telemetry::flush_on_exit`) and from a chained panic hook, which also flushes `app.log` in a release build because release builds abort on panic. Neither takes a lock on the logging thread.

### JSONL Rotation

The JSONL file is rotated while the app runs, not only at launch. `SizeRotatingFile` (`telemetry.rs`) tracks the file size and, before the first event written after the file reaches 5MB, shifts `events.jsonl.1` → `.2`, renames `events.jsonl` → `.1`, and starts a fresh file. Rotation happens between events, so rotated files always end on a line boundary. Two rotated generations are kept (`.1` newest, `.2` oldest; the oldest is overwritten by the next rotation). A file that is already over the cap at launch is rotated the same way. Failures are handled per step. If shifting or renaming fails, the current file is intact: the writer keeps appending to it (events are never dropped) and retries after another 1MB. If the rename succeeds but re-creating `events.jsonl` fails, the writer has no file and drops events rather than appending to `.1`, which the shipper treats as complete. It tries to create the file at most once per second, counts the dropped events without writing to stderr under the lock, and writes a single `Log events dropped` system line with the count when the file is writable again. Nothing is buffered. Clear logs unlinks each writer's file and rotated generations and reopens it on a fresh file under that writer's own short lock, so output after a clear is neither lost nor written to an unlinked handle. For `events.jsonl` the reset runs on the `event-log-writer` thread itself, behind every event already queued: events still queued or batched at that moment are discarded, so none of them can reappear in the fresh file afterwards (before the off-thread writer, `events.jsonl` was written synchronously on the logging thread, so this held trivially). The clear is all or nothing. If the writer has not started the reset within 2 seconds (its sink is stuck), the reset is cancelled before it runs, `clear_logs` returns an error saying nothing was cleared, and the Events buffer is left as it was; the cancelled reset is never executed late. Once the reset has started it is waited for to completion. Events dropped by a full queue while the reset runs are still counted and noted in the fresh file; losses from before it are discarded with the data they belonged to. The `app.log` queue (`tracing_appender`'s own worker) is not behind that barrier, exactly as before this change: a line already queued for `app.log` when Clear logs runs can still be written to the fresh file just after it. That race is pre-existing, is not made worse by the lower default volume, and is accepted rather than adding a second coordination mechanism; Clear logs also has no button in the current UI. the slower cleanup of other files and dated rolling files runs afterwards with no lock held and skips every file a live writer manages, so a rotation during cleanup is never deleted.

The log shipper depends on this layout; see [log-shipping.md](log-shipping.md). On startup, the ring buffer is pre-populated with the last 500 valid events across all retained generations (`.2`, `.1`, then current, in chronological order), so a restart right after a rotation — or a crash between the rename and the re-create — still restores history.

### Privacy Stripping

In release builds, arbitrary string-valued fields from `pipeline` target events are stripped from the `data` object. Numeric fields survive, and the dictation lifecycle's exact `event_code`, `outcome`, and `error_code` values survive only when they match their key-specific allowlists. For the `transform` stream in both debug and release builds, each string must likewise match an explicit key-specific enum/bucket vocabulary; unknown keys or values are dropped. Numeric and boolean diagnostic fields are retained. Release `summary` values are derived only from an allowlisted `event_code`, with `Structured event` as the fallback; meeting and frontend summaries are constant in every build. This prevents transcription, transform, or caller-provided frontend content from being persisted in structured log data.

The pipeline string exceptions are exact allowlisted `event_code` values and
the bounded terminal `outcome` / `error_code` vocabularies. Event codes are
fixed identifiers such as `pipeline.dictation_terminal`; arbitrary values are
removed. Transform event codes pass through the same exact-value allowlist.
The `performance.store_operation_failed` system event is treated as its own
all-build schema: only its code, bounded operation/error class, attempt count,
and recording ID survive. SQLite text, SQL, paths, and unknown fields are
dropped.

Key identity is content: a key pressed while a hotkey is held is usually the
next character the user is typing. Keyboard telemetry therefore never records
the identity of any key other than the configured hotkey, in any build. Detector
and rejection events carry `event_kind` (`key_press`, `key_release`, ...),
`is_target_key`, `target_key` (the configured hotkey's settings id such as
`shift_l` or `f8`, never a raw key), and a bounded `key_class`: `modifier`,
`function`, `navigation`, `editing`, `character`, `other`, or `none`. The raw
rdev `EventType`, `Key`, and typed `name` are never formatted into a field.
`telemetry.rs` enforces this independently of call sites, in every sink:
structured events, the pretty `app.log`, and JSONL written by an older build.
The `keyboard` stream keeps only an exact key-and-value string allowlist, and
its messages may not name any non-hotkey key (Escape cancellation logs
`keyboard.escape_cancel`, not the key). In every other stream, string fields
and summaries keep their text but any key identity is replaced with `[key]`:
a distinct rdev name such as `KeyI`, `Num4`, or `SemiColon`, a raw
`Unknown(<code>)`, or, inside parentheses such as `Some(End)` or
`KeyPress(Return)`, an ordinary-word key name. A name counts only as a whole
token delimited by whitespace, quotes, `(`, `)`, `,`, `=`, or `:`, so paths and
model names such as `/models/KeyA/model.bin` are untouched. `app.log` string
fields are quoted and escaped, as with the default formatter.
Persisted lines are re-sanitized before history restore and before upload, so
a backlog queued before an upgrade cannot ship key identity.

Frontend-originated codes are validated by Rust before entering the structured
event. This gives local and fleet presentation layers stable semantic keys
without permitting user content or arbitrary labels.

### Pretty-Printed Log File

A separate human-readable log file (`app.log` or `app.dev.log`) is maintained via `tracing_appender::non_blocking` over the same `SizeRotatingFile` writer. `tracing_appender` has no size-based rotation, so the writer caps the file at 10MB and keeps one rotated generation (`app.log.1`). The rotation runs on the non-blocking worker thread, so logging call sites never wait on it. An oversized file left by an older build is rotated at launch. `get_log_contents` reads only the current file. By default the file holds warn and error lines only; `events.jsonl` is the record of info events. With debug logging on it also holds info lines and sanitized debug lines, which makes it the file to read when hunting a bug. It is never read by the log shipper. Clear logs removes it with the rest (see the note on its queue above).

### Tracing Streams

Tracing targets map to stream names used for filtering:

| Target | Stream | Typical Events |
|--------|--------|----------------|
| `pipeline` | pipeline | Transcription timing, VAD results, model loading |
| `audio` | audio | Device selection, sample rates, audio levels |
| `keyboard` | keyboard | Hotkey detection, rejection reasons/timing, key class (never typed-key identity), mode changes, listener lifecycle |
| `transform` | transform | Correlated transform key, state, capture, audio, effects, and terminal outcomes |
| `system` | system | Startup, permissions, updates, resource usage |

`meeting` and `hunt` are also streams (`hunt` carries debug-only [`hunt!`](#hunting-logs) lines). The former `recording` target used by the Smart Turn check now logs as `pipeline`, and a straggler on `recording` is sanitized as `pipeline`, so string fields such as an error carrying a model path never reach `events.jsonl` or the shipper.

### Hunting logs

Temporary logs added while chasing a bug use `hunt!` instead of `info!`/`warn!`:

```rust
hunt!(issue = 1234, "queue drained at stop");
```

It takes the issue number and a string literal and nothing else: no fields, so there is no value that could carry private content, and any other argument is a compile error (the macro's `compile_fail` doctests show a field and a `message =` argument being rejected). The literal is passed to the event as a value, not a format string, so `{name}` in it is logged as written and never expanded from a variable in scope. It emits at debug on the `hunt` stream, so it is off by default, local-only and never shipped; the sanitizer still drops string fields from any `hunt` event as a second line of defense. Remove every `hunt!` for an issue when the issue closes. `python3 scripts/check_hunt_sites.py` (run by the `policy` CI job, covered by `tests/test_hunt_sites.py`) lists each site and fails if one has no issue number, passes anything besides the literal, or if code emits on the `hunt` target directly; it recognizes `hunt!(`, `hunt!{` and `hunt![`, ignores comments (including nested block comments and `//` inside strings), the macro definition and `#[cfg(test)]` module bodies, and does not call GitHub, so removal at close stays a review habit. An event that must stay on by default is not a hunting log: give it a stable `event_code` in `canonical_event_code` (`telemetry.rs`) and, if it carries any string field, an exact schema in the sanitizer.

### Dictation shadow mismatches

Temporary, for the dictation-core refactor (removal tracked in #1154). A build with the `dictation-shadow` Cargo feature, started with `MURMUR_SHADOW=1` (exactly `1`, read once at launch), runs `murmur-dictation-shadow`: a thread that re-decides each dictation input with the extracted core and compares the decision with what the live path did. It never executes an effect and never touches a live field, lock, port or window. The live side makes **one attempt** to store an owned message in a preallocated ring (`dictation_shadow/ring.rs`, a small bounded Vyukov queue of 256 messages, one consumer) and nothing else: no allocation and no free (a message owns no heap memory; Smart Auto's refusal is a typed code, not text), no lock, no channel, no wake-up, no loop, no backoff and no yield. A message the ring cannot take (it is full, another producer won the slot, or the consumer has advanced but not yet released it) is handed back and costs the producer two atomic increments and nothing more: a global `lost` count and one counter per registered site (`lost_by_site`, so E4 still gets per-site loss counts; the counters are indexed densely from the site registry, `SiteId::ALL`, so every registered site has its own; a loss with no registered site is `loss_unattributed`), the contention part also counted as `dropped_contended`. There is no loss log and no subject id. The consumer stages each batch, then reads `lost` again: if it is not the value the last batch ended on, the staged batch and every open, held or waiting record are given up (`abandoned_by_loss`, counted once per flight) and the sequence resynchronises on the next header. The header ticket records the `lost` count when it is taken (one atomic load, no lock or allocation), and after a reset the comparator admits only a flight whose header epoch is at least the count it acknowledged: a flight whose header was still in the ring, or hidden behind a reserved slot, while its own message was lost is given up when its header arrives, and what it sends afterwards is dropped (`late_message_discarded`) before it reaches any record or cache. The H1 registration cache takes only registrations that an admitted record validated. A panic the comparison catches (`shadow_panic`) is handled the same way (`abandoned_by_panic`), so nothing half applied is compared afterwards. Nothing is compared past a loss and nothing is guessed about what it touched. A producer bumps `lost` before it can publish a later message, so a batch holding a message that follows a loss always sees it. The ring keeps its head-of-line semantics: the consumer stops at a slot a producer reserved and has not published, never skips it, and the live path is unaffected. The comparison thread polls every 5 ms. `start()` only spawns that thread: the ring and the counters are allocated and published by the thread itself (a recorder created before that, in the first enabled startup, is an `Unavailable` one: it never blocks or allocates, and each message it would send is counted as `dropped_unavailable`), and the log directory lookup, creating and truncating the event file and every write happen inside it, so a slow disk never delays app setup, and a panic the thread catches skips the app panic hook's flush of the live event writer (a thread-local flag, compiled only with the feature). A default build contains none of the comparison code and its panic hook is unchanged.

A build with the feature refuses to compile under `panic = "abort"` (the shadow catches its own panics, which only works with unwinding), and the release profile aborts. Build a native soak binary with the `shadow-soak` profile instead: `cargo build --profile shadow-soak --features dictation-shadow` (`shadow-soak` inherits `release` and sets `panic = "unwind"`; `app/src-tauri/src/dictation_shadow_policy_tests.rs` pins both halves).

A difference is written by the comparison thread to `dictation-shadow-events.jsonl` in the log directory (truncated at each launch): one line of three numbers, `site_id`, `input_kind` (the C3 row's ordinal) and `diff_kind` (1 effect set, 2 effect order, 3 placement, 4 reply, 5 post-state, 6 continuation, 7 observation, 8 shadow panic, 9 conditional). It never goes through `tracing`, so it never reaches the in-app log buffer, `TauriEmitterLayer` or a window. The line is validated before it is written (a known site, an ordinal of at most 9999, a `diff_kind` of 1 to 9); a malformed record is dropped whole. `pipeline.dictation_shadow_mismatch` keeps its exact numeric schema in every build, and the sanitizer there also drops a record whole when any of the three numbers is missing or out of range, and both log paths (`app.log` debug lines and the structured event path) then reject the **event** itself: no empty record, summary, ring-buffer entry or window event is produced. At most one record per (site, input_kind, diff_kind) per 60 seconds and 50 per session are written; the suppressed ones are counted.

The counters are written to `dictation-shadow-summary.json` in the log directory every 5 minutes and on a clean exit: compared, matched, `mismatched_root`, `followup`, `skipped_incomplete`, `late_header_discarded`, `dropped`, `dropped_contended`, `dropped_unavailable`, `lost_open`, `abandoned_by_loss`, `abandoned_by_panic`, `late_message_discarded`, `unverifiable`, `unconsumed`, `reserved_unpublished`, `loss_unattributed`, `lost_by_site`, `abandoned`, `abandoned_pre_d`, `witness_overflow`, `shadow_panic`, `events_suppressed`, `per_row` (keyed `site.row`) and `pd_witness` (keyed `PD-n`). A preserved defect is counted only after the comparison witnessed the behaviour that defines it (PD-4 needs the winning stale terminal, the second ticker for the same recording and that same recording's later successful performance outcome, named by the completing input). Each C3 row's history is checked against what the decision and the input require: a decided continuation is registered in H1, a `K:Pipeline` row (R203.9, R203.10, R301.5) also registers the recording in H2, and a continuation input retires its full token (recording, kind, operation) in H1; a pipeline-owned finish (the pipeline's own guard finish, R204a.*, and the import's parse failure, R302.1) retires its owner in both tables: H1 with the `K:Pipeline` operation the registration drew and H2 for the finishing recording, never the newer current one; a missing or mismatching operation is `diff_kind 6`. The owner's full H1 token must come from the comparison's own registration witness: a finish for which none was witnessed (or whose registration was given up by a reset) is `unverifiable`, neither clean nor a mismatch. Only the comparison thread writes the file: on exit the hook asks it to stop, it makes a bounded number of drain attempts (each takes at most four rings' worth of messages, and the thread's stop flag is read between drains), writes the final summary (with `unconsumed`, the messages still claimed in the ring, and `reserved_unpublished`, the slots a producer reserved and never published) to a unique temporary file and renames it into place; a reserved slot never blocks exit. The file is numbers only. Capture sites are marked `shadow_site!(issue = 1154)` (an empty macro without the feature); `python3 scripts/check_shadow_sites.py` (the `policy` CI job, `tests/test_shadow_sites.py`) lists them and fails when one has no issue number. Remove every site and the module when the issue closes.

### Session stamp

Once per launch, right after telemetry starts, one `system.session_started` info event records `app_version`, a random per-launch `session_id` (16 hex characters) and `build` (`release` or `debug`). `recording_id` restarts on every launch, so a reader joins a recording to a version and launch by the nearest earlier session line instead of inferring from timestamps. Events themselves do not carry these fields.

### Frontend Logging

The `flog` utility (`lib/log.ts`) records a content-free frontend event through the `log_frontend` command. It takes an event code from the `FrontendEventCode` union (the frontend hydration milestone and the `updater.*` codes), plus an updater version and selection where relevant. Free-text messages are not accepted: the backend never logs frontend text, and drops any code it does not allowlist (`canonical_event_code` in `telemetry.rs`). Coded events appear in the Events tab with `source: "frontend"`.

```typescript
flog.info("updater.download_ready", { version });
flog.error("updater.check_failed");
```

Calls are fire-and-forget (errors silenced). Add a new frontend event by adding its code to `canonical_event_code` and to `FrontendEventCode`; the type keeps uncoded calls from compiling.

## Embedded workspace

The workspace is rendered inside the main window under **Settings → Model →
Advanced**. **⌘L**, the command-palette diagnostics action, and App's **View
Performance** button all navigate to the Model tab. The diagnostics tabs are
ordered **Events**, **Runs**, **Performance**, **Compare**, **Transform**. The
diagnostics shell owns its inner scroll regions so live Events and charts
remain usable without opening or managing a second native window.

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

Microphone startup health reads a separate versioned history containing at most
20 finalized dictation observations. Each record has only startup duration, a
fallback boolean, and an optional allowlisted backend enum. It survives app restarts and is
not evicted by the general event ring; the five-recording verdict remains
insufficient until five successful captures exist.

**Scoped resources** keep each measurement in its typed scope:

- Host CPU is whole-host utilization normalized to 0–100%.
- Murmur CPU is main-process utilization; 100% equals one logical core.
- Main-process RSS is physical resident memory.
- Rust heap and FFI/native heap are allocator-zone measurements, not additive
  RSS components or GPU memory.

CPU and memory charts use one keyboard-operable timeline cursor. Missing or
failed measurements create chart gaps. A measured zero remains zero;
`notApplicable` and `unavailable` remain distinct text states.

### Runs Tab

Runs reads at most the newest 200 `PerformanceRunV1` records. Dictation, file
transcription are current kinds. Retired run kinds remain readable as content-free diagnostics. Kind and
outcome filters cover success, no-speech, cancelled, timed-out, failed, and
interrupted terminal outcomes.

Each row shows timestamp, runtime identity, outcome, privacy-safe input shape,
total start-to-terminal latency, real-time factor or token throughput where
meaningful, and measured resource peaks. Selecting the row uses
`get_performance_run` and opens its detail view.

**Phase waterfall** displays the canonical stage order and duration contribution.
The V1 schema does not record absolute stage offsets, so the UI does not invent
them. Measured zero is a zero-width marker; unavailable and not-applicable rows
use explicit text. Legacy apply/undo records remain readable as correlated follow-ups.

**Resource summary** reports start, average, peak, and end for every typed host,
main-process, and sidecar range with its scope named.

**Clear Performance Data** requires confirmation and clears only the performance
database. Events, logs, transcription history, settings, knowledge, and
benchmark/evaluation reports are untouched. Loading, never-recorded, cleared,
filtered-empty, error, stale/partial, unsupported, and unavailable states are
presented separately.

### Transforms Tab

Transforms lists the newest content-free `TransformAttemptV1` records. Each
attempt carries the same `transform_pass_id` used by Events and
`PerformanceRunV1`, plus ordered selection, instruction, sidecar, review,
apply/undo, cancellation, and process-lifecycle phase evidence. The companion
performance run remains schema V1 and supplies the correlated resource peaks.

**Capture next transform** is an explicit one-shot privacy boundary. After a
warning and confirmation, an in-memory arm lasts for one attempt or ten minutes.
Exact selection, recognized instruction, and output are stored locally with
restrictive permissions, capped at three captures and seven days. Opening a
capture is the only command that returns its text to the Diagnostics webview.
The tab offers Review and Delete, but no copy/export integration. Ordinary
Events, Performance records, reports, and log export stay content-free.

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

## Commands

| Command | Description |
|---------|-------------|
| `get_log_contents` | Returns the last N lines from the pretty-printed log file |
| `clear_logs` | Removes all log files and clears the in-memory event ring buffer |
| `log_frontend` | Records an allowlisted, content-free frontend event through Rust tracing (INFO/WARN/ERROR) |
| `get_event_history` | Returns all events from the in-memory ring buffer (up to 500) |
| `clear_event_history` | Clears the in-memory event ring buffer |
| `list_performance_runs` | Returns at most 200 newest typed performance runs |
| `get_performance_run` | Returns one typed run by opaque `run_id` |
| `get_performance_resource_window` | Returns the bounded typed resource window |
| `get_capture_health_history` | Returns the bounded, content-free V1 capture-startup observation history |
| `clear_performance_diagnostics` | Clears local Performance runs, samples, and capture-startup observations |
| `list_transform_attempts` | Lists bounded, content-free transform attempts |
| `arm_next_transform_diagnostic_capture` | Arms one local exact-content capture for up to 10 minutes |
| `get_transform_diagnostic_capture_status` | Reads the in-memory one-shot arm state |
| `list_transform_diagnostic_captures` | Lists content-free capture summaries |
| `get_transform_diagnostic_capture` | Returns one exact local capture for explicit review |
| `delete_transform_diagnostic_capture` | Deletes one exact local capture |

## Log Files

All files stored in `~/Library/Application Support/local-dictation/logs/`:

| File | Format | Purpose |
|------|--------|---------|
| `app.log` / `app.dev.log` | Pretty-printed text | Human-readable log (read by `get_log_contents`): warn and above, or everything when debug logging is on. Rotated at 10MB at runtime |
| `app.log.1` / `app.dev.log.1` | Pretty-printed text | Previous rotated app log |
| `events.jsonl` / `events.dev.jsonl` | JSONL | Structured events, rotated at 5MB at runtime |
| `events.jsonl.1` / `events.dev.jsonl.1` | JSONL | Most recent rotated JSONL file |
| `events.jsonl.2` / `events.dev.jsonl.2` | JSONL | Older rotated JSONL file (kept so the shipper survives a second rotation) |

`clear_logs` removes all known log file variants, including legacy files (`frontend.log`, `transcription.log`, dated rolling files).

For read-only diagnostics across both release and dev logs, including MCP setup
and the single user-level registration model, see
[`tools/murmur-diag/README.md`](../../tools/murmur-diag/README.md).
