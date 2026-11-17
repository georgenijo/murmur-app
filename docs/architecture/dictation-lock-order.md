# Dictation lock order

Scope: the locks that serialize and guard the dictation lifecycle in
`app/src-tauri/src`, the order they are taken in, and every place a wait
happens while one of them is held. This note records what the code does at
`main` @ `466bdb37`. It does not propose behaviour changes. Where a wait is
unbounded or a hold is long, the note says so and leaves the behaviour alone.

Line numbers are for that revision. `recording.rs` means `commands/recording.rs`;
`mp` means `commands/microphone_preview.rs`; `al` means `audio_lifecycle.rs`.

## Lock inventory

| Name | Kind | Defined | Role |
|---|---|---|---|
| **T** | `tokio::sync::Mutex<()>` (`AppState.recording_transition`) | `state.rs:368` | Serializes start, stop, cancel, preview start, meeting start, benchmarks and file transcription against each other. Held across `.await` points. |
| **D** | `std::sync::Mutex<DictationState>` (`AppState.dictation`) | `state.rs:364` | Dictation status and recording id. It is also the delivery owner: there is no separate delivery lock, so delivery jobs take D (`with_current_delivery_owner*`, `recording.rs:1616-1651`). |
| `active_context` | std mutex | `state.rs:405` | Per-recording context snapshot. `clear_active_context` (`state.rs:481`) takes it and, inside it, `vad_stream` (`state.rs:380`) and `stream_reuse` (`state.rs:378`). |
| `last_transcription_at`, `model_idle_policy` | std mutexes | `state.rs:370-371` | Idle-unload bookkeeping. |
| backend mutex | `ModelRuntimeManager.inner` | `model_runtime.rs:860` (in `unload`) | Held by any decode, model load or unload. |
| `dictation_telemetry`, diagnostics store, performance `entries` | std mutexes | `dictation_telemetry.rs`, `recording.rs` | Terminal-claim and performance bookkeeping. Leaves. |
| supervisor `PublicState.owner`, `hal_boundary`, `INTERRUPTED_CAPTURES` | std mutexes | `al:675` (`hal_boundary`), `al:2706` (`INTERRUPTED_CAPTURES`) | Supervisor-side state. Leaves, except that `hal_boundary` is held across `sink.notify` on the supervisor thread (`al:1455`). |
| keyboard detectors, `HOLD_PROMOTION` | std mutexes | `keyboard.rs:1574`, `:1781-1782` | Taken after D when both are needed (see below). |
| `PREPARATION` | std mutex | `audio_prepare.rs:261` | Capture-preparation slot. Leaf. |
| **`core_history`** (planned) | std mutex | not yet in the code | See [the planned leaf lock](#planned-leaf-lock-core_history). |

Preview hide ownership is atomics, not a lock (`commands/dictation_preview.rs`).

## Order: T → D → leaves

Wherever T and D are both taken, T is taken first. Wherever D and a leaf are
both taken, D is taken first.

```
T  →  D  →  active_context → vad_stream / stream_reuse
         →  last_transcription_at, model_idle_policy
         →  dictation_telemetry → diagnostics, performance entries
         →  PublicState.owner, INTERRUPTED_CAPTURES, PREPARATION
         →  keyboard detectors, HOLD_PROMOTION
         →  backend mutex   (blocking only in the idle-unload path; see waits)
         →  Rust listeners of emitted events (inline, see waits)
```

T → D, observed at: start (`recording.rs:5473-5479`, D at `:5505`), stop
(`:5898` → `:5920`), cancel (`:6383` → `:6385`), legacy `process_audio`
(`:3682` → `:3691`), preview start (`mp:187` → `:202`), benchmark (`commands/benchmark.rs:48` → `:62`),
microphone startup benchmark (`commands/microphone_startup_benchmark.rs:1043-1048`),
corpus (`commands/corpus.rs:534-539`), and the Fn key-down admission
(`keyboard.rs:1009`, D read inside the admission predicate).
File transcription is T, then the `file_transcribing` flag, then T released,
then D (`recording.rs:6516`, `:6532`, `:6537`, `:6544`).

There is no D → T inversion. Model removal uses `try_lock` for both T and D
(`commands/models.rs:411`, `:416`), so it cannot wait on either.

The reverse direction toward the backend mutex is `try_lock` only:
`dictation_partial_is_current_if_uncontended` (`recording.rs:115-126`) takes D
with `try_lock_or_recover` because idle unload holds D while it blocks on the
backend. The blocking variant `dictation_partial_is_current`
(`recording.rs:106-111`) is only for callers that do not hold the backend.

`keyboard.rs:1640-1641` documents the same discipline for `HOLD_PROMOTION`: the
promotion lock is never held while D is acquired (`dictation_in_flight`, `:1613`).

Two orders are sequential, not nested, and differ per site, so a refactor must
keep each site's order: `IdleGuard::finish` calls `clear_active_context` and
then takes D (`recording.rs:1063`, `:1067`); the bridge's Idle arm calls
`clear_active_context` while holding D (`:5272`).

## The comment at `state.rs:365-367` was wrong

The comment on `recording_transition` said the lock "is never held while waiting
for Core Audio or a channel". That is false for start. The start command holds T
across a synchronous wait on a supervisor channel:

- Direct path: `audio_lifecycle::start_dictation_recording` at
  `recording.rs:5652`, blocking up to `SUPERVISOR_RESPONSE_TIMEOUT` = 2 s
  (`al:21`, chosen at `al:2978-2982`, awaited by `await_start_response`,
  `al:3051`). It runs on the async worker.
- Kept-preview path: when a Settings microphone test was kept running for
  hand-off, the same call is wrapped in
  `spawn_blocking(..).await` (`recording.rs:5636-5650`), up to
  `START_AFTER_PREVIEW_TIMEOUT` = 15 s + 2 s = 17 s (`al:3092-3093`).

T is released only at `recording.rs:5849` (or `:5774` on the flush path). Stop
and cancel do not do this: stop drops T at `:6053` before
`stop_dictation_recording` (`:6067`, up to `STOP_RESPONSE_TIMEOUT` = 12 s,
`al:22`), and cancel drops T at `:6407` before `cancel_dictation_capture`
(`:6408`, up to 2 s, `al:3347`). The stop-while-Starting branch also drops T
(`:5991`) before `cancel_dictation_capture` (`:5996`). The comment in `state.rs`
now says this. The behaviour is unchanged.

## Waits under T or D

"Held" lists what is held during the wait. Bounds are those in the code; where
the code has none, the row says so.

### Under T (D not held)

| Site | Wait | Held | Bound |
|---|---|---|---|
| `recording.rs:5652` | Supervisor acknowledgement of the start | T | 2 s |
| `recording.rs:5636-5650` | Same, via `spawn_blocking`, with a kept preview | T | 17 s |
| `recording.rs:5697-5698` | `smart_destination::capture(..).await` | T | Bound lives in `smart_destination`; not re-read for this note. D is free here. |
| `mp:195` | `retire_standby`, which sends `RetireStandby` and `recv_timeout`s (`al:3383-3392`) | T | 2 s |
| `mp:269-283` | `spawn_blocking(start_preview_recording)` in the preview's spawned task | T | 2 s (`al` `send_start`, `wait_until_ready = false`) |
| `mp:154-168` | `transition_after_key_down_resolves` polls an unconfirmed Fn key-down | T released between polls | `KEY_DOWN_RESOLUTION_WAIT` = 2.5 s (`mp:12`) |

`transition_after_preview_cleanup` (`mp:420-443`) takes T, and when a preview
must be stopped it drops T before `stop_exact_preview(..).await`, so that stop
does not run under T.

### Under D

| Site | Wait | Held | Bound |
|---|---|---|---|
| `mp:227-231` (D block `:202-234`) | `preview_audio_blocker` calls `retire_standby` when a key-down capture is unconfirmed (`mp:142-143`). The supervisor thread may be waiting for D in a bridge arm at the same time. | D + T | 2 s, then times out. A real circular wait until the timeout. |
| `resource_monitor.rs:200-215` | `model_runtime.unload` takes the backend mutex with a blocking lock (`model_runtime.rs:860`) | D | **None.** See below. |
| `recording.rs:5593`, `:5952`, `:5974`, `:3711` | `frontmost::capture_delivery_target_snapshot` (NSWorkspace plus AX queries, `frontmost.rs:757-821`; the AX query timeout is 25 ms per call, `frontmost.rs:1407`) | D (+ T except `:3711`, where T is released later) | 25 ms per AX call; the call count is not bounded here. |
| `recording.rs:5546` | `audio_inventory::resolve_smart_auto`: commits routing and emits under D | D + T | No blocking I/O; destructive observation. |
| `recording.rs:3286-3297` | `smart_destination::prepare` in a main-thread job | D (main thread) | 1 s preparation authority (`:3261`) |
| `recording.rs:3393-3408` | `injector::inject_with_destination_authorized` (clipboard write, paste, optional AX verification) in a main-thread job | D (main thread) | 2 s authority (`:3370`) |
| `delivery_recovery.rs:244-256` | `smart_destination::prepare_focused` in a main-thread job | D (main thread) | 1 s authority on the awaiting side (`:259`); no timeout on the hold itself |
| `delivery_recovery.rs:298-319` | `injector::inject_with_destination_authorized` plus `smart_destination::abort`, in a main-thread job | D (main thread) | 2 s on the awaiting side (`:323`); none on the hold |
| every `emit` under D | Rust listeners run inline unless another thread is mid-emit, then later on that thread (tauri 2.11.2 `emit_filter`). `meeting_suggestions::busy_changed` takes its own mutex. | D | Short. |

Emits under D include `dictation-delivery-outcome` (`recording.rs:3591-3606`),
`smart-destination-selected` (`:1550-1582`), the status emits in the bridge arms
(`:4928`, `:4983`, `:5228`, `:5236`, `:5287` take D), and `delivery-retry-feedback`
(`delivery_recovery.rs:178`).

### Idle unload: unbounded wait on the backend while holding D

`check_idle_timeout` (`resource_monitor.rs:165`) runs on the 1 s heartbeat task
every 60th tick (`:289`). It takes D at `:200` and calls `unload` at `:215`,
which blocks on the backend mutex with no timeout and no `try_lock`. Its gates
(`benchmark.is_running()`, `meeting_inference_active`, status Idle, idle age)
do not exclude every backend holder at status Idle:

1. `transcribe_file` holds the backend across the whole decode while status
   stays Idle and `file_transcribing` is not consulted. `last_transcription_at`
   is touched only after the decode.
2. A cancelled pipeline's decode that is still draining.
3. `spawn_idle_model_preparation` holding the backend for a model load. The
   shared-backend-change guard does not set `benchmark.is_running()`.
4. A model load that outlives a very short recording.

While it waits, everything that needs D blocks: start, stop and cancel (after
T), every supervisor bridge arm (so the supervisor thread), delivery, the
partial ticker (`dictation_partial_is_current`), the VAD stream poll, and the
heartbeat task itself. The wait is bounded in practice only by the longest such
holder, typically a long file decode. It is not time-bounded in code. Plan §9
item 9 originally called this bounded; that was wrong.

### Main-thread D holds in `delivery_recovery.rs`

Paste Last (`retry_last_delivery`, `delivery_recovery.rs:154`; callers
`spawn_retry` `:389`) takes no T and has no lifecycle effect: no status write,
no terminal, no `set_processing`, no id allocation. Its three D uses read
`status` and `recording_id` and refuse unless status is Idle:

- `:172-182`, the `retry_generation` block, on an async task. On refusal it
  emits `delivery-retry-feedback` at `:178` and returns with D still held
  until the block ends.
- `:244-256`, a main-thread job (`delivery_recovery.prepare`): D is held across
  `smart_destination::prepare_focused` (`:248`).
- `:298-319`, a main-thread job (`delivery_recovery.retry_inject`): D is held
  across `injector::inject_with_destination_authorized` (`:309`) and
  `smart_destination::abort` (`:317`).

While either main-thread job runs, every D taker blocks: start, stop, cancel
(T → D), the supervisor bridge, the partial ticker and the VAD poll. A start
that bumps `recording_id` between `:181` and `:245`/`:299` makes the job
refuse, so a retry cannot overwrite a start.

The rule that follows: **nothing may hold D while waiting for the main thread**,
because main-thread jobs take D (`recording.rs:3286`, `:3393`,
`delivery_recovery.rs:244`, `:298`, `commands/dictation_preview.rs:323` through
`can_show`). No D holder does that today: the async tasks hold no D across
their `.await`s and the D guards live only inside the main-thread closures.

## Placement classes

Every effect at every call site has one of four placements. The refactor's
executor runs them in this order, at the site's own boundaries:

pre-verdict → `PreD` → acquire D → decide → `InD` → release D → `PostD_T` →
release T → `PostT`

| Class | Meaning |
|---|---|
| `PreD` | Before D is taken. T may or may not be held, stated per site. |
| `InD` | While D is held, in the order the code runs. |
| `PostD_T` | After D is released, with T still held. |
| `PostT` | After T is released, or at a site that never takes T (supervisor arms, `IdleGuard`, the stop command's post-pipeline blocks). |

Examples verified in this checkout:

- Start's accepted transition is `InD` (D `recording.rs:5505` to `:5605`):
  `next_recording_id` (`:5563`), `capture_claimed`, the Starting status write,
  and the AX snapshot (`:5593`). The audio start (`:5652`) is `PostD_T`: D is
  released, T is held.
- Stop-during-Starting stamps (`hotkey_release::take_for` `:5981`,
  `stop_gesture::take_for` `:5982`) are `PostD_T`: D released at `:5978`, T
  released at `:5991`.
- Missing-context stop (`:6020-6038`) holds D to the arm's return, and T is
  released at `:6026` **inside** D (T → D → release T → release D). So
  `complete_dictation_performance` (`:6027`) is `InD` there, with T already
  gone. This is the one site off the monotone chain.
- The stop's two D sections after the pipeline are separate: `IdleGuard::finish`
  (`:1067-1076`) and the idle emit (`:6209-6213`). T was released at `:6053`,
  so a newer start can be admitted between them.
- Supervisor bridge arms never hold T. `PostD_T` is empty there and `PostT`
  means D is not held.

Moving an effect to another class is a behaviour change.

## Planned leaf lock: `core_history`

The refactor adds exactly one lock, `AppState.core_history`, a `std::sync::Mutex`
holding the operation history: pending operations (H1) and running pipelines
(H2). Operation ids come from a process-global `NEXT_OP` atomic that lives
outside the lock. It does not exist yet; this section is the rule it must
follow when it lands in the first Stage 2a PR.

1. **Leaf.** It is taken only after D if D is held (D → `core_history`, never
   the reverse). It is never held while taking another lock, while awaiting, or
   while running an effect. T is never taken while it is held.
2. **One short critical section per access:** `validate`, `reg`,
   `ret_if_matches`, `snapshot_for_occupancy`. Each is released before the next
   statement.
3. **D-less sites add no D.** Entries that take no D today (`Accepted`,
   `InitializationFailed`, the context-present stop continuation, and the like)
   touch only `core_history`.
4. **Decide functions never mutate it.** The shell reads a validated token
   status before `decide` and applies `reg` / `ret_if_matches` right after.
   No decision branches into a no-effect outcome on an operation's status.
5. **Retirement** is by `ret_if_matches(token)`, which compares operation id,
   recording id and kind under one lock and removes the entry only on a full
   match. Nothing is evicted and a `recording_id` advance retires nothing.
6. **Caps are diagnostic** high-water counters; they never refuse or evict.
7. The executor never takes D or T, and re-entrancy is refused before any lock
   is attempted.

The new lock changes none of the waits above.

## Summary of preserved hazards

- T held across a supervisor acknowledgement, up to 2 s, or 17 s with a kept
  preview.
- D held with T across `retire_standby` (2 s) while the supervisor may be blocked on D.
- D held across a blocking, untimed backend-mutex acquisition in idle unload.
- D held on the main thread across `prepare_focused` and the retry paste
  (`delivery_recovery.rs`), and across the delivery prepare and paste.
- Rust listeners of emits run under D.
