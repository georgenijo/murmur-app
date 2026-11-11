# Main-thread diagnostics

Content-free instrumentation of the macOS main thread, owned by
`app/src-tauri/src/main_thread_watch.rs`. Each paste waits for the main thread;
these events show what occupied it during that wait. The events carry only static
labels, recording ids, monotonic timestamps, and durations. The exact schema is
enforced in `telemetry.rs` in every build.

## What is recorded

- **Jobs.** Every `run_on_main_thread` closure is wrapped in
  `main_thread_watch::job("<label>", …)`. Each record has its queue wait and its
  run time.
- **Commands.** Every Tauri command that the invoke handler runs on the main
  thread, labelled with its registered command name.
- **Callbacks and sections.** Notification blocks on the main queue, window and run
  events, and tray events. Nested native calls such as `window.hide`,
  `window.show`, `window.set_size`, `overlay.set_position`, and
  `telemetry.event`. Only spans of 100 µs or longer are recorded.
- **Run loop.** Three `CFRunLoopObserver`s on the main run loop record each
  *busy span*: time when the thread is not waiting. They also split it into
  phases: `runloop.wake`, `runloop.sources`, `runloop.observers` (Core Animation
  and AppKit display), `runloop.event_loop_drain` (Tao's drain of queued jobs,
  window messages from other threads, and script evaluation), and
  `runloop.outside` (AppKit event dispatch). This catches stalls the app did
  not queue.

The main thread only pushes fixed-size records into a preallocated ring. A
background thread does every log write, every 200 ms.

## Events

| Event code | When | Key fields |
|---|---|---|
| `system.main_thread_watch_started` | once at launch | `epoch_unix_ms` (the zero for every `*_us` timestamp), `observers_installed` |
| `system.main_thread_span` | any span ≥ 10 ms, or a job queued ≥ 20 ms (at most 30 per 10 s) | `kind`, `label`, `start_us`, `start_unix_ms`, `duration_us`, `queue_us`, `longest_phase`, `longest_phase_us`, `segments` |
| `pipeline.main_thread_paste_wait` | every dictation paste, about 1.2 s after it | `recording_id`, `inject_queue_us`, `inject_job_us`, `queued_us`, `started_us`, `slow`, `runloop_busy_us`, `runloop_idle_us`, `labelled_us`, `unlabelled_busy_us`, `job_us`, `command_us`, `callback_us`, `section_us`, `event_loop_drain_us`, `longest_label`, `longest_kind`, `longest_us` |
| `system.main_thread_paste_span` | every span that overlapped a paste wait ≥ 20 ms (at most 48) | the span fields plus `recording_id` and `overlap_us` |

`kind` is one of `job`, `command`, `callback`, `section`, `runloop_busy`, or
`runloop_segment`. The paste-wait window runs from dispatch (`queued_us`) to the
start of the paste job (`started_us`).

How to read the summary fields:

- **High `runloop_idle_us`.** The thread was waiting while the paste was queued.
- **High `unlabelled_busy_us`.** The thread was busy with work the app did not
  label. Use the `runloop_segment` phase labels to see which part of the loop.

## Querying

```bash
LOG=~/Library/Application\ Support/local-dictation/logs/events.jsonl   # events.dev.jsonl for tauri:dev
jq -c 'select(.data.event_code=="pipeline.main_thread_paste_wait" and .data.slow)|.data' "$LOG"
jq -c 'select(.data.event_code=="system.main_thread_paste_span" and .data.recording_id==42)|.data' "$LOG"
```
