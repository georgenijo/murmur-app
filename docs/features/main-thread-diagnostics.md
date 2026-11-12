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

- **Stack samples.** When one busy stretch passes 50 ms, the reporter reads the
  main thread's stack and logs symbol names only. See [Stack samples](#stack-samples).
- **Applied markers.** `set_size`, `set_position`, and `set_ignore_cursor_events`
  are queued main-queue blocks, so their wrappers time only the enqueue. A marker
  queued behind each one records when the block actually came up. See
  [Where window work really runs](#where-window-work-really-runs).
- **Rates.** Per-second counts of `audio-level` emits and IPC invokes.
- **Stage gaps.** Every dictation logs the time between its named stop-path
  stages, so `totalProcessing` is fully accounted for.

The main thread only pushes fixed-size records into a preallocated ring. A
background thread does every log write. It drains the ring about every 200 ms,
and it sleeps by what the main thread is doing: parked for the drain interval
while the main thread waits, and woken by the run-loop observer when a busy
stretch begins (see [Stack samples](#stack-samples)). An idle app costs the
reporter about five wakeups a second.

## Events

| Event code | When | Key fields |
|---|---|---|
| `system.main_thread_watch_started` | once at launch | `epoch_unix_ms` (the zero for every `*_us` timestamp), `observers_installed` |
| `system.main_thread_span` | any span ≥ 10 ms, or a job queued ≥ 20 ms (at most 30 per 10 s) | `kind`, `label`, `start_us`, `start_unix_ms`, `duration_us`, `queue_us`, `longest_phase`, `longest_phase_us`, `segments` |
| `pipeline.main_thread_paste_wait` | every dictation paste, about 1.2 s after it | `recording_id`, `inject_queue_us`, `inject_job_us`, `queued_us`, `started_us`, `slow`, `runloop_busy_us`, `runloop_idle_us`, `labelled_us`, `unlabelled_busy_us`, `job_us`, `command_us`, `callback_us`, `section_us`, `event_loop_drain_us`, `longest_label`, `longest_kind`, `longest_us` |
| `system.main_thread_paste_span` | every span that overlapped a paste wait ≥ 20 ms (at most 48) | the span fields plus `recording_id` and `overlap_us` |
| `system.main_thread_stack` | a busy stretch reaches 50 ms (sample 1) and 250 ms (sample 2); at most 8 per minute | `sample`, `busy_us`, `start_us`, `start_unix_ms`, `frame_count`, `frames`, `held_us`, `sampling_stopped`, `suppressed` |
| `system.main_thread_rates` | each second that carried `audio-level` emits, plus an idle second with ≥ 10 invokes (at most once per 10 s) | `window_us`, `start_unix_ms`, `audio_level_emits`, `ipc_invokes`, `ipc_main_invokes`, `ipc_main_us` |
| `pipeline.dictation_stage_gaps` | once per dictation, when its stop path ends | `recording_id`, `total_us`, `unattributed_us`, and one `*_us` field per stage (below) |

`kind` is one of `job`, `command`, `callback`, `section`, `runloop_busy`, or
`runloop_segment`. `preview_hide` on the paste-wait event says where that recording's
live-preview hide started relative to the paste:

- `before_paste`: before the paste was queued.
- `during_wait`: while the paste waited.
- `after_paste`: after the paste job started.
- `none`: no hide ran, for example because the card was never used.

`pipeline.dictation_preview_presentation` carries the hide's `trigger`:

- `after_paste`: queued behind delivery.
- `no_paste`: the stop ended without delivery.
- `recording_ended`: cancel or interruption while recording. The paste-wait window runs from dispatch (`queued_us`) to the
start of the paste job (`started_us`).

How to read the summary fields:

- **High `runloop_idle_us`.** The thread was waiting while the paste was queued.
- **High `unlabelled_busy_us`.** The thread was busy with work the app did not
  label. Use the `runloop_segment` phase labels to see which part of the loop.

## Stack samples

A busy stretch is one run-loop wake with no wait in between. When the stretch
passes 50 ms the reporter captures the main thread's stack once, and a stretch
still busy at 250 ms gets one more sample. Samples are rate-limited to 8 per
minute; the rest are counted in `suppressed` on the next event.

The reporter does not poll while the app is idle. It parks for the drain
interval and sets `REPORTER_PARKED`. The run-loop observer, at the start of a
busy stretch, stores the stretch's start and then reads that flag; if the
reporter is parked it clears the flag and calls `Thread::unpark`, an atomic swap
plus a semaphore signal, with no lock or allocation on the main thread. The
reporter sets the flag and then re-reads the stretch start before it parks, so
one side always sees the other and a stretch is never slept through. Once woken,
the reporter sleeps until the stretch's 50 ms mark (and while a stretch is being
watched it wakes at least every 50 ms), so a stall is sampled at about 50 ms, not
at the next drain tick. It parks again when the main thread waits.

`frames` is `;`-joined `image!symbol` items, innermost first, at most 24 frames
and 2,400 characters. The image is a file name, never a path, with no spaces.
Symbols are the raw `dladdr` names, so Rust frames are mangled (`rustfilt` or
`c++filt` reads them). A frame `dladdr` cannot name is `?!?`. A symbol has a
space only inside Objective-C `[...]`. There are no arguments, strings,
registers, or memory contents, and the telemetry sanitizer drops the whole field
unless every item has that shape.

How the capture stays safe (macOS arm64 only; other targets log no stacks):

- It never uses a signal. A signal handler on the main thread can interrupt
  `malloc` or a lock holder and deadlock the app.
- It suspends the main thread with `thread_suspend`, reads the register state,
  walks the frame-pointer chain, and calls `thread_resume`. Between suspend and
  resume it allocates nothing, takes no lock, logs nothing, and does no symbol
  lookup (`dladdr` takes dyld's lock). Symbolication runs after the resume.
- The walk only reads memory between the stack pointer and the top of the main
  thread's stack, captured at install, so a corrupt frame pointer cannot reach
  unmapped memory. Frames are bounded, and the code before resume cannot panic.
- The reporter runs at user-interactive QoS so it is not left descheduled while
  it holds the main thread.
- Before it samples, the reporter calls `thread_get_state` and `thread_resume`
  once on its own thread (`stack::warm_up`). A binary built for the arm64
  default deployment target (macOS 11, which dev and test builds use) binds
  imports lazily, and the first call through a stub takes dyld's lock; that must
  not happen for the first time while the main thread is suspended. Warming from
  the reporter's own thread works for every build configuration, including the
  bundle's macOS 14 minimum, where the linker decides how imports bind.
- `held_us` is how long the main thread stayed suspended (normally a few
  microseconds). If one capture holds it longer than 2 ms, the event reports
  `sampling_stopped: true` and sampling stays off until the next launch.
- A stack is dropped if the stretch ended while it was being read.
- `MURMUR_DISABLE_MAIN_THREAD_STACKS=1` (or `true`) turns sampling off at
  launch; other values, including `0`, leave it on. The
  `system.main_thread_watch_started` event reports `stack_sampling`.

The sample is a snapshot of one instant. Read two samples of one stretch together,
and treat a single frame list as a hint, not a measurement.

## Where window work really runs

tao applies `set_size`, `set_position`, and `set_ignore_cursor_events` as
`dispatch_async` blocks on the main queue. From the main thread, Tauri calls tao
at once, and tao returns after the enqueue. So the `section(...)` wrappers
(`window.set_size`, `overlay.set_position`, ...) measure only the enqueue.

Each of those calls goes through `queued_section`, which also queues a marker on
the main queue right behind the setter's block. When the marker runs, the setter's
block has finished. The marker is a `job` span named `applied.<label>` with zero
duration, and its `queue_us` is the time from enqueue to run. That is an upper
bound for the block itself: any other queued work in front of it counts too. Markers
that waited under 1 ms are not recorded, and one that waited 20 ms or more is
logged like a slow job. The marker is queued only when the setter returned `Ok`.

Limits:

- A wrapper only times a call made on the main thread. A setter called from
  another thread goes through the event-loop proxy and is not labelled.
- `plugin:window|outer_position` and `plugin:window|cursor_position` cannot be
  labelled. Tauri sends every `plugin:*` command straight to the plugin and never
  through the app's invoke handler, which is where the `command` spans and the IPC
  counters are taken. Their main-thread time shows up in
  `runloop.event_loop_drain` and in the stack samples. The `ipc_*` rates count
  app commands only.

Other labels added for these freezes: `preview.follow_overlay`,
`overlay.primary_monitor`, `overlay.detect_notch`, `overlay.dock_prefs`,
`overlay_material.apply`, `overlay_material.install` (first install of the
material views), `overlay_material.attach`, and `overlay_material.refit`.

## Rates

The audio capture thread and the invoke handler add to lock-free counters. The
reporter swaps them to zero once per second.

- `audio_level_emits`: `emit("audio-level")` calls in the window. Each call is
  delivered to every webview.
- `ipc_invokes`: app commands that reached the invoke handler, on any thread.
- `ipc_main_invokes` and `ipc_main_us`: the subset handled on the main thread
  and the time they took.

## Stage gaps

`pipeline.dictation_stage_gaps` reports microseconds along the stop path. The
segments are disjoint and in order. `unattributed_us` is `total_us` minus all of
them.

| Field | From → to |
|---|---|
| `capture_finalization_us` | stop accepted → capture handed off (teardown, resample) |
| `handoff_us` | capture handed off → pipeline entered |
| `pipeline_prologue_us` | pipeline entered → VAD stage start |
| `vad_us` | VAD stage, including the `spawn_blocking` hop |
| `vad_blocking_queue_us` | inside `vad_us`: the blocking pool starting the closure |
| `pre_backend_us` | VAD end → backend request |
| `backend_acquire_us` | backend request → decode start (lease, lock wait, model load) |
| `backend_model_queue_us`, `backend_model_load_us` | inside `backend_acquire_us`, millisecond resolution |
| `backend_decode_us` | decode |
| `backend_release_us` | decode end → backend request returned |
| `transform_setup_us` | backend return → transform start |
| `transform_us` | the ordered transform pipeline |
| `post_transform_us` | transform end → delivery start (history, file output) |
| `delivery_us` | clipboard write and paste, with bookkeeping |
| `pipeline_return_us` | pipeline body done → `run_transcription_pipeline` returned to the stop command |
| `idle_status_emit_us` | the idle-status emit and dictation lock |
| `tail_us` | after the idle block → `totalProcessing` ends |

## Querying

```bash
LOG=~/Library/Application\ Support/local-dictation/logs/events.jsonl   # events.dev.jsonl for tauri:dev
jq -c 'select(.data.event_code=="pipeline.main_thread_paste_wait" and .data.slow)|.data' "$LOG"
jq -c 'select(.data.event_code=="system.main_thread_paste_span" and .data.recording_id==42)|.data' "$LOG"
# What was on the main thread during a busy stretch, and how long the sample held it
jq -c 'select(.data.event_code=="system.main_thread_stack")|.data|{sample,busy_us,held_us,frames}' "$LOG"
# Per-second audio-level and invoke counts
jq -c 'select(.data.event_code=="system.main_thread_rates")|.data' "$LOG"
# Window work that waited in the main queue
jq -c 'select(.data.event_code=="system.main_thread_span" and (.data.label|startswith("applied.")))|.data' "$LOG"
# Unexplained stop-path time per dictation
jq -c 'select(.data.event_code=="pipeline.dictation_stage_gaps")|.data|{recording_id,total_us,unattributed_us}' "$LOG"
```
