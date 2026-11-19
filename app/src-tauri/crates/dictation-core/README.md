# murmur-dictation-core

The pure dictation decision vocabulary and admission policy. Stage 1a of the
recording refactor: the crate holds the types and `admit()`. Stage 1b
(`ui_lib`'s `dictation_admission` module) routes every admission predicate
through `admit()` / `admit_after()` at its existing call site; Stage 2a adds the
`decide_*` functions, one input family per PR (lifecycle events first, `lifecycle`).

Hard rules, enforced by the crate shape:

- No dependencies at all (no tauri, no app crates, no serde, no async runtime).
- `#![forbid(unsafe_code)]`; no I/O, no locks, no clocks. Time enters as
  `ids::Now` values; every fact enters as a plain field of an input or of
  `Occupancy`.
- Transcript, audio and clipboard content never appear in a type here. Payloads
  are opaque `ids::Handle`s, lengths or counts.

Run the tests from `app/src-tauri`: `cargo test -p murmur-dictation-core`.
CI runs them with the rest of the workspace libraries (`cargo nextest run -E
'kind(lib)'` over `--workspace`).

## Module map

| Module | Holds | Appendix C |
|---|---|---|
| `ids` | `RecordingId`, `OperationId`, `PreviewId`, `Now`, `Handle`. The `NEXT_OP` counter and the id allocation stay in the shell; only the newtypes live here. | C2.1, C1.4.1 |
| `status` | `Status` (M1), the `ErrCode` / `TerminalOutcome` mirrors of `dictation_telemetry.rs`, `AudioCancelReason`, `StopTrigger`. | C1.2, C2.1, C3 |
| `state` | The C1 state vocabulary: `PersistedCore` (**only** M1 `status` and M2 `current_id`; the fields are private, read through `status()` / `current_id()` and written only by their effect commits `set_status` / `alloc_id`, decision D-S1-4), `LiveObs` and `DictationCore` (the immutable decision view: M3 `cancelled_through` and M4 `context_for` are observations, never stored), `LockFreeFacts` (M2, M3 for D-less entries), and the pure C3.8 queries 401/402/403/406/407. H1/H2 stay in the shell's `core_history`. Query 404 (ticker registration) reads atomics and stays in the shell. | C1.2, C1.4, C2.3, C3.8 |
| `effects` | `Placement` (`Pre`/`In`/`PostDT`/`PostT`), `Placed`, the `Effect` enum (one variant per legend code), `Cond`, and `validate_effects` (placements non-decreasing, `ST` only `In`, `EmitOk` pairs an earlier emit). `HIST` is deliberately not an effect. | C0.1, C0.2 |
| `decision` | `Decision`, `Verdict`, `Refusal`, `StaleKind`, `InvalidKind`, `ObsKind` (`NeedObservation`), `ContKind`, `ContinuationSpec` / `Continuation`, `OpStatus`, `RowId`, `SiteId` with the shadow `site_id` encoding (202n = 2021, 204a = 2041, 204b = 2042), and `Decision::validate`. | C2.1, C2.2, C0.3, C1.4.1 |
| `lifecycle` | Stage 2a-1: the ten supervisor-lifecycle decide functions (sites 207-216, rows R207.1-R216.5) and their small input structs. Pure: `&DictationCore` (or `LockFreeFacts` for the `*_nolock` entries) in, a `Decision` with ordered, placed effects out. None reads M3 or M4. | C2.3, C3.6 |
| `stop` | Stage 2a-2: the stop-family decide functions, one per D critical section: `decide_stop_request` (201, with the pre-D stale path `decide_stop_request_stale`), `decide_stop_context` / `decide_stop_context_nolock` (202 / 202n), `decide_stop_capture_result` / `decide_stop_capture_result_nolock` (203), `decide_guard_finish` (204a, shared with the pipeline guard, the legacy parse failure and `IdleGuard::drop`), `decide_pipeline_idle_emit` and its D-less tail `decide_pipeline_idle_emit_tail` (204b), plus the input-pure `Pre` effect lists the shell runs before D (`stop_context_pre_effects`, `stop_capture_pre_effects`, `guard_finish_pre_effects`). Rows R201.1-R204b.6. Pure and mutation-free; none reads M3 or M4. | C2.3, C3.5 |
| `cancel` | Stage 2a-3: the cancel-family decide functions: `decide_cancel_request` (205, rows R205.1-R205.5) and `decide_cancel_capture_result_nolock` (206, rows R206.1-R206.2, D-less, shared by the cancel command's R205.4 and the stop command's R201.4 through `CancelOrigin`). Pure and mutation-free; neither reads M3 or M4, and no row branches on an op status. | C2.3, C3.5 |
| `start` | Stage 2a-4a/4b: the start decide functions: `decide_start_destination` (R103.0, the smart-destination emit's own D section: `In:SDE` iff Smart destination is enabled and the capture is still current; query 402's earlier read stays the shell's), `decide_start_context` (103, R103.1-R103.6), `decide_ready_publish_nolock` / `decide_ready_publish` (104, R104.1-R104.6; the installed-context read is `NeedObservation(ContextPresence)`, so `ReadyPublishInput.context_present` is an `Option<bool>`) and `decide_start_flight_end` (R104.7: `PM`, `PB`, `REL`, the `recording_starting` reply), plus the 4a functions: `decide_start_request_pre` (R101.1-R101.2, no D), `decide_start_request` (101, rows R101.3-R101.15) and `decide_start_capture_result_nolock` / `decide_start_capture_result` (102, rows R102.1-R102.6, with `start_capture_failure_pre_effects`). Pure and mutation-free; neither reads M3 or M4. The blockers and the status step run on the occupancy the shell read lazily; the retiring-capture read and the Smart Auto resolve are `NeedObservation(CleanupPending)` / `NeedObservation(SmartAutoSelection)` (D-S1-2), so `StartInput.cleanup_pending` is an `Option<bool>`. | C2.3, C3.1-C3.4 |
| `process_audio` | Stage 2a-5: the legacy `process_audio` decide functions: `decide_process_audio_start_pre` (R301.1, the meeting refusal, no D), `decide_process_audio_start` (301, rows R301.1-R301.5, and R301.6 = `decide_process_audio_destination`, the import's own Smart Destination recheck, exactly `decide_start_destination` under its own row id so shadow capture never credits site 103; the benchmark read is `NeedObservation(BenchmarkRunning)`, so `ProcessAudioStartInput.benchmark_running` is an `Option<bool>`) and `decide_process_audio_finish` (302: R302.1 is exactly the 204a `LegacyParse` rows and R302.2 exactly the 204b `LegacyPost` rows, by delegation to `decide_guard_finish` / `decide_pipeline_idle_emit`, so the returned row ids are the R204a / R204b ones). No `TERM` is ever decided here. Pure and mutation-free; neither reads M3 or M4. | C2.3, C3.7, C11.2 |
| `inputs` | One input struct per decide function (C2.3 table, C2.4 sketches), the `Input` enum (every one of the 24 sites is reachable, incl. 216 via `SupervisorEvent::InterruptedShortIdle`; three sub-section decisions that the site table folds into a parent site, R103.0 `StartDestination`, R104.7 `StartFlightEnd` and R301.6 `ProcessAudioDestination`, have their own variants and report their parent's `site()`), `Input::recording_id()` (what the shadow taints when a record is lost) and `StartInput::pre_verdict`. No input owns heap memory (a test scans the production text for `String`, `Vec`, `Box`, `Arc` and maps), because the shadow captures an input by value on the live thread. | C2.3, C2.4, C3 |
| `handle` | Stage 2c-1: `DictationCore::handle(input, now) -> Decision`, the one entry that routes an `Input` to the `decide_*` that owns it (picking the D or D-less entry of a split site from the input's own facts, and handing a `*_nolock` entry the view's `LockFreeFacts`). It adds no behaviour and mirrors the shell's inputs exactly: the supervisor `Ready` carries `Option<bool>` (the staged `NeedObservation` round trip, both calls), and the legacy import's two finishes have one representation: `GuardFinish(LegacyParse)` and `PipelineIdleEmit(LegacyPost)` route through `decide_process_audio_finish` (R302.1 / R302.2 with their replies) and report site 302 (D-S2c-1 h; parity tests against the live entry). The shadow comparison (C8.3, `ui_lib::dictation_shadow`) and the cutover PRs (C9.7) call it; `DictationCore::project` builds the view it runs on. A decision's row always belongs to the input's `site()` (test). | C1.2, plan §6 2c |
| `replies` | `Reply`, `ReplyType` (the literal `type` strings), `ErrText`, `ProcessAudioRefusal`. No serde: the shell maps replies to JSON. | C2.1, C3 "Reply" columns |
| `rows` | `C3_ROWS`: all 141 C3 row ids with their owning site (`site_of_row`, `rows_of_site`); the row-level half of the Stage 1 "C3 complete" exit. | C3.1-C3.7 |
| `admission` | `Request` (17 per-phase variants), `Occupancy`, `Admission`, `RefusalCode` (today's literals), `admit()`, and `admit_traced()` / `check_order()` which expose the evaluated checks. | C6.1, C6.1a, C6.2, C6.3, C6.4 |

## Admission

`admit(request, &occupancy)` evaluates one ordered check list per request
(C6.2) and **returns at the first check that fires; later checks are not
evaluated**. `admit_traced` returns the names of the checks it evaluated, which
is how the tests prove the short-circuit. The check lists, the `Occupancy`
fields they read and the refusal literals are the single source in
`src/admission.rs`; `check_order(request)` returns the order.

The C6.3 matrix is pinned cell by cell in `src/admission/tests.rs`
(`c6_3_matrix_every_cell`): R refuse, A allow, K takeover (a `Takeover`, or an
`Admit` whose `PreviewDisposition` is not `NotPresent`), W `WaitThen`, S and `-`
both "`admit` allows" (the supervisor decides, or the code makes no check).

### C6.4 per-resource claim protocols

`admit` centralizes the **predicate**, never the **atomicity**. Nothing in this
crate claims, locks or releases a resource. Each resource keeps the claim, the
lock it is taken under, the recheck points and the release it has today; the
shell calls the per-phase variants in the order the code does. Admission is
not made atomic across resources, and the crate does not claim it is.

| Resource | Claim | Under which lock | Recheck | Release |
|---|---|---|---|---|
| Dictation | status `Idle→Starting` + id bump (R101.15) | D inside T | after audio start (R102 id guard), after smart-destination capture (`q_capture_current`), at context install (R103), at ready publish (R104) | R203 / R204a / R204b / R205 / R211 / R216 / R210 (status back to Idle); the supervisor releases microphone ownership in `finish_attempt` after it notifies Idle (`audio_lifecycle.rs:2460-2461`) |
| File transcription | `file_transcribing.swap(true)`, after the pre-admission preview stop | T, then T released, then the D recheck | D: benchmark, corpus, status | `FileTranscribeGuard::drop` |
| Meeting | store `meeting_active` / `meeting_inference_active`, **then** re-read `relaunch_committed` (Dekker) | T for the whole `start_meeting`; D not held across the claim | relaunch only; dictation status is read before the claim and **not** re-read (T keeps a dictation start out) | on start failure; thread exit clears `meeting_active` only if `meeting_generation` still matches |
| Preview | `start_generation` before any wait, then `preview.claim(vad)`, then `capture_claimed(Preview)` | T for the claim; D only for the preceding reads | `is_connecting(id)` after re-taking T in the spawned task | `Idle` / `Recovering` / `InitializationFailed` bridge, `stop_exact_preview`, `clear_if`, `fail_and_clear` |
| Mic startup benchmark | `benchmark.try_start()` under D, then `microphone_startup_benchmark.claim(run_id)` under T only | D then T | the second claim can fail, then `coordinator.finish()` | `coordinator.finish()` at the end of the run |
| Benchmark | `coordinator.try_start()` | D inside T | none | `BenchmarkRunGuard` drop |
| Corpus | `corpus.claim(request)` | D + T | none | `corpus.clear_if(id)` on failure; bridge `Idle` |
| Shared backend change (model preparation and model removal) | `try_start_shared_backend_change()` | the coordinator's own mutex only; model removal calls it while holding `try_lock`ed T and D | none | `SharedBackendChangeGuard::drop` / `ChangeGuard::drop` |
| Relaunch | `set_committed(true)`, then the `gate().open()` re-read, then relaunch or `set_committed(false)` | D inside the gate | the re-read **is** the recheck | `ReleaseUnlessRelaunched::drop` |
| Speculative capture | `speculative::claim(token)` before T, single-use, 1.5 s TTL | the `BINDING` mutex | adopted at the supervisor | `SpeculativeClaim::drop` discards |

Microphone exclusion (standby adoption, speculative adoption, kept-preview
handoff, `AlreadyStarting` / `AlreadyRecording` / `AudioRecovering`) is enforced
by the supervisor (`audio_lifecycle.rs:1425-1433`, `1052-1075`), not by `admit`.
`DictationStart` therefore never reads `audio_active`, `standby` or
`unconfirmed_key_down`.

Any change to a claim protocol (for example making `Meeting` re-read dictation
status after its claim, or giving `Meeting` a preview check) is a separate,
separately-approved fix.

### Phase sequences

A request that claims or waits is split into phases, and a recheck or retry
must call the **next** phase, never the one before the claim or the wait:

| Sequence | Phases | Rule |
|---|---|---|
| Preview start | `PreviewStart` (before `preview.claim()`) → `PreviewStartRecheck` (`microphone_preview.rs:270`, `is_connecting(id)` only) | After the claim `preview_active` is true, so re-running `PreviewStart` would refuse the preview's own generation. The recheck repeats no admission check and claims nothing. |
| Standby / key-down retirement (`PreviewStart`, `MicStartupBenchmarkPreClaim`) | `WaitThen(RetireStandby)` / `WaitThen(RetireKeyDownCapture)` → the shell retires → `admit_after(request, retry, &fresh)` | **A retry resumes, it never restarts.** `admit_after` evaluates only the checks **after** the wait, against fresh facts; earlier checks are not re-run or re-read (production already passed them: Settings closing during a retire revokes the start, but production goes straight to `audio_active`, `microphone_preview.rs:137-146`). So a wait is returned at most once, and a retire that timed out falls through to the `audio_active` refusal. Standby resumes at the check after `standby` (`start_revoked` for preview, `meeting_blocks_asr` for the mic-startup benchmark); a key-down retire resumes at `audio_active`. |
| File transcription | `PreClaim` → claim → `PostClaim` | `PostClaim` does not read `file_transcribing` (its own claim). The `file_transcribing` fallback of `PreClaim` is acceptable **only as a pre-claim view**: production wiring must preserve the atomic swap and its returned prior value (`recording.rs:6599-6605`); it cannot substitute a flag load for that result. |
| Meeting | `PreClaim` → claim → `PostClaim` | `PostClaim` reads only the re-read `relaunch_committed`. |
| Mic startup benchmark | `PreClaim` → `try_start()` → `PostClaim` | `PostClaim` reads only `second_claim`. |
| Model preparation | `ModelPreparation` (twice around the backend wait, `recording.rs:1153`, `:1168`) | The recording id the worker was spawned for must equal the current id, evaluated first; a stale worker is never eligible. The idle warm-up has no generation guard (`recording.rs:1240`, `:1248`), and its read under its own `SharedBackendChange` claim does not see that claim. |
| Relaunch gate | gate reads activity with the relaunch's own commitment excluded | `RelaunchGate` ignores `relaunch_committed`. |
| Single-phase claims (`DictationStart`, `Benchmark`, `Corpus`, `SharedBackendChange`, `ModelRemoval`) | one `admit`, then the claim | After the claim a second `admit` conflicts with the claim itself; the shell must not recheck through `admit` (C6.4's own recheck points apply). |

`tests.rs` spans every row of this table (`preview_recheck_spans_the_claim`,
`key_down_retirement_resumes_at_the_audio_check` (revocation changes during the
retire), `preview_standby_retirement_resumes_after_the_standby_check`,
`mic_startup_benchmark_standby_retirement_resumes_after_the_standby_check`,
`resume_evaluates_exactly_the_checks_after_the_wait`,
`no_request_can_wait_more_than_once_per_kind`, `model_preparation_*`,
`post_claim_phases_do_not_read_their_own_claim`,
`single_phase_claims_are_terminal_and_must_not_be_readmitted`).

## Deviations from the appendix, and where the code wins

The appendix is normative except where it contradicts the code at the base
commit; then the code's behaviour is implemented and listed here (and in the
Stage 1a PR).

**Contradictions (the code wins)**

1. **Retiring capture, prepare / key-down / relaunch.** C6.3 prints
   `cleanup_pending`; `dictation_refusal` (`recording.rs:5495-5515`) checks
   `audio_active` first and a retiring capture is always `audio_active`
   (`is_dictation_cleanup_pending` requires phase `Stopping|Recovering`,
   `audio_lifecycle.rs:3485-3492`, and `is_active` is `phase != Idle`,
   `:337-339`). The refusal is `microphone_busy`. `cleanup_pending` is returned
   only when `audio_active` is false, which no real retiring capture produces.
2. **Relaunch committed, prepare / key-down.** C6.3 prints `busy_restarting`;
   `dictation_refusal` maps every blocker, including a committed relaunch
   (`recording.rs:5496-5503`), to `dictation_busy`.
3. **Mic-startup check, `FileTranscription` / `PreviewStart` / `Corpus`.** C6.3
   prints `–`; each reads `benchmark.is_running()` (`recording.rs:6614`,
   `microphone_preview.rs:212`, `corpus.rs:549`), which a running mic-startup
   check holds (`coordinator.try_start()`, `microphone_startup_benchmark.rs:1073`),
   so they refuse.
4. **Unconfirmed key-down, `MicStartupBenchmark`.** C6.3 prints `–`; the start
   reads `audio_lifecycle::is_audio_active()`
   (`microphone_startup_benchmark.rs:1066`), which is true for an unconfirmed
   key-down capture, so it refuses.
5. **Line numbers.** The appendix's `recording.rs` admission lines are about 60
   lines earlier than the code at the Stage 0b base (`dictation_start_blocker`
   is at `recording.rs:5407`, not 5347); the logic matches.

Cells 3 and 4 are encoded as `CODE_WINS` in the matrix test, and a test fails
if one stops differing from the printed matrix.

**Additive fields and variants** (facts a C6.2 check reads that the C6.1 sketch
omits)

- `Occupancy`: `standby` (the matrix has a standby column), `fixed_and_smart_auto`,
  `smart_auto` (`SmartAutoObservation`), `key_down` (`KeyDownShellFacts`: the
  listener / gesture / permission checks that sit between `processing` and
  `disabled` in `fn_capture.rs` `admission_refusal`), `preview_start_revoked`,
  `transition_lock_busy`, `dictation_lock_busy` (`ModelRemoval`'s two
  `try_lock`s), `model_matches`, `model_prep_variant`, `recording_id` /
  `expected_recording_id` (the model-preparation generation guard),
  and the `admit_after` resume API (a retry resumes after the wait instead of
  re-running earlier checks).
- `Request::PreviewStartRecheck`: a 17th request. C6.1 lists the recheck as part
  of `PreviewStart`; the code runs it after the claim, in another task.
- `Admission::NeedObservation`: C6.2 #1 routes the smart-auto resolve "through
  `NeedObservation`"; `admit` returns it before any later check.
- `PreviewStart` puts the `standby` wait first, because the code retires Standby
  before every read (`retire_standby`, ahead of `refuse_revoked_start`).
- `DictationStart` checks `app_disabled` and Smart-Auto-with-a-fixed-device
  first (C6.2 #1 lists them as pre-D checks); `StartInput::pre_verdict` is the
  same pair.
- `KeyDownCapture` includes the `fn_capture.rs` shell pre-checks in
  `admission_refusal` order, so the whole key-down gate is one `admit` call.
- `ModelPreparation` covers both eligibility variants of C6.2 #12 through
  `model_prep_variant`; the preparation tasks return silently today, so
  `ModelPreparationNotEligible` carries a diagnostic tag, not a user message.
- `PreviewStart`'s `smart_auto && device_id` check (after the audio blocker in
  `microphone_preview.rs`) is a pure input check, not in C6.2 #7, and is not
  an `admit` check.

Preconditions the shell must keep (not enforced by the types): a running
mic-startup check implies `benchmark_running`; a retiring, standby or
unconfirmed key-down capture implies `audio_active`.

## Per-stage authority matrix (C1.4.3)

Who is authoritative **for decisions** and who writes the field, per stage. The
matrix is updated by every Stage 2a PR (one row of changes per PR below the
table). "F" is the family being cut over (lifecycle, stop, cancel, start, in that
order). **M3 and M4 are never persisted in the core**: in every column the live
field is authoritative, `MC` and `CX±` write only it, and every entry fills them
as immutable observations from the live slot.

| Item | 0a / 0b / Stage 1 | 2a, every family PR (decide inline) | 2c cutover of F | cleanup |
|---|---|---|---|---|
| M1 `status` | live; sole writer `transition_dictation_status` | live authoritative; `project()` reads live; writes through the adapter `set_status` (`ST` for migrated entries, the legacy call for unmigrated ones) | F's entries read the persisted core; the adapter still writes live | `project()` deleted; the adapter is the sole writer |
| M2 `current_id` | live `recording_id`; writers `next_recording_id` (start, `process_audio`) | live authoritative; `alloc_id` through the adapter (`ID+` / the legacy call) | as M1 | as M1 |
| M3 `cancelled_through` | live `cancelled_id`; writer in `cancel_native_recording` | **live only**: adapter `cancel_through` (`MC`); `project()` observes it | unchanged: observed from live at every entry | adapter writes live |
| M4 `context_for` | live `active_context`; 1 set site, 7 clear sites | **live only**: adapter `set_context` / `clear_context` (`CX±`), each site keeping its placement; observed at entry | unchanged | adapter writes live |
| H1 `pending_ops` | n/a | `core_history` from the first 2a PR; written only by `reg` / `ret_if_matches` / owner `Drop` | unchanged | unchanged |
| H2 `pipelines` | n/a | as H1 | unchanged | unchanged |
| legacy `process_audio` (301 / 302) | legacy inline | **decide inline** (2a-5): the single writer of M1 `status` and M2 `current_id` is the executor's `ST` / `ID+` through the adapter, at R301.5's `In` placement under the D hold the section already takes; H1 `K:Pipeline` and H2 `pipelines[rid]` are registered by the shell right after the decision and retired at the C3.9 boundary on every exit; the parse failure (R302.1) and the post-pipeline tail (R302.2) are decided by site 302 | switches with the stop family | rows deleted only if the command is removed |
| `IdleGuard` (204a) | inline | writes through the adapter; `decide_guard_finish` inline in the stop-family PR | all three origins switch with the stop family | the guard only calls the executor |
| queries `q_*` | inline predicates | each reads `project()` (live) | F's M1/M2 from the persisted core, M3/M4 from the live slots | `project()` deleted |
| shadow | none | **2c-1: infrastructure only** (`ui_lib::dictation_shadow`, Cargo feature `dictation-shadow`, `MURMUR_SHADOW=1`): its `ShadowCore` holds M1/M2 only and re-syncs them from each header; no live writer and no capture site yet (2c-2); needs `panic = "unwind"` (`shadow-soak` profile) | compares F's decisions; then the C8.6 divergence assertion | deleted one release later |

### Matrix updates by PR

| PR | Rows changed |
|---|---|
| **2a-0** (core_history, field adapter, executor skeleton) | M1/M2: every production writer already goes through `FieldAdapter` (`set_status`, `alloc_id`), which also commits `DictationState.core` (`PersistedCore`, M1/M2 only; created by this PR and written only by the adapter). M3/M4: `cancel_through` / `set_context` / `clear_context` write the live field only (the core copy does not exist); `observe_live` is the entry read. H1/H2: `core_history` and `NEXT_OP` exist; no production caller yet. `IdleGuard`, `process_audio` and every start/stop/cancel/supervisor write: unchanged inline code calling the adapter (zero behaviour change). The executor `run` exists but no production path calls it. SP5 ratchet starts: 0 direct M-field writes outside the adapter, 15 `transition_dictation_status` call sites and 24 `emit_dictation_terminal*` call sites, only down. `PersistedCore::new` has one caller (the seeding adapter). |
| **2a-1** (lifecycle events: sites 207-216) | **Authoritative writer, per input moved.** For every input of the supervisor-thread lifecycle events the single writer of M1 `status` is the executor's `ST` effect through `FieldAdapter::set_status` (R207.5, R209.3-.7, R210.2-.3, R211.3-.6, R216.1-.5), run at the decision's `In` placement under the D hold the arm already takes. M2 `current_id` is **not written** by any lifecycle row (only `ID+` of the start family writes it). M3 and M4 stay live-only: `CX-` (R209.3-.5/.7, R211.3-.6, R213.2, R216.1-.5) writes only the live slot through `FieldAdapter::clear_context`; the lifecycle decide functions read neither (`Ready` asks for the context through `NeedObservation(ContextPresence)` after its stale check, D-S1-2). H1: `R215.2` returns the `AutoStop` continuation spec and the shell registers it in `core_history` right after the decide, before any effect runs (C4.3); the `ContinuationGuard` waits in the ports and `RS` moves it into the spawned stop, which consumes it at stop admission (T held, `ret_if_matches`; the rest of the stop family is unchanged). A task that never runs, is cancelled while it waits on T, or panics drops the guard and `Drop` retires the op. D-less sites 212 and 213 run their mutex-taking leading effect (`ACC`, `PCF`) **before** the current id is read, as today (`accepted_prefix`, `init_failed_prefix`, `Decision::after_prefix`), and the bridge refuses a nested entry (`CoreEntry`, C4.2) as its first statement, before the managed-state lookup and any lock. H2: untouched. Unchanged and still inline: `start_native_recording`, `stop_native_recording_for`, `cancel_*`, `IdleGuard`, `process_audio` and the start-side ready publish (site 104), all calling the adapter. `complete_dictation_capture_cleanup` is now test-only (the pre-extraction oracle the supervisor's tests drive); the production cleanup arm is `decide_sup_cleanup_finished`. SP5 ratchet lowered: `transition_dictation_status` call sites 15 -> 10, `emit_dictation_terminal*` 24 -> 14 (one new caller: the lifecycle `Ports::terminal`). |
| **2a-2** (stop family: sites 201, 202, 202n, 203, 204a, 204b) | **Authoritative writer, per input moved.** The single writer of M1 `status` for the stop command, the pipeline guard, the legacy parse failure and `IdleGuard::drop` is the executor's `ST` effect through `FieldAdapter::set_status` at the decision's `In` placement, under the D hold each section already takes: R201.7/.8 `→Processing`, R202.3-.5 `→Idle` (only for the guard's own id, PD-3), R203.1-.8 and R204a.1-.6 `→Idle` (the id compared, the status never read: PD-6, PD-13). M2 is not written by any stop row. M3 and M4 stay live-only: `CX-` (R202 `Pre`, R203.1-.8 and R204a `Pre`, before D) writes only the live slot through `FieldAdapter::clear_context`; no stop decide function reads M3 or M4 (the view is `project_lifecycle_view`, M4 absent). H1: R201.4 `K:CaptureCancel`, R201.7/.8 `K:StopContext`, R202.1/.2 `K:CaptureStop`, R203.9/.10 `K:Pipeline` are registered in `core_history` right after the decide, before any effect, and each consuming site (202, 203) consumes through `validate`, decide, `ret_if_matches`; `op_status` never branches a row; a flight dropped mid-way retires its op through the owner's `Drop`. The `AutoStop` op of 2a-1 is still consumed at stop admission (T held), before the stale check. H2: R203.9 registers the pipeline (`reg_pipeline`) together with its `K:Pipeline` op; both are owned by the pipeline's own `IdleGuard` (`PipelineOwner`), which validates the op before deciding 204a and retires H1 and H2 after the decision and before the `In` effects (C3.9, C1.4.1), so a finish, a cancelled exit, a stale exit, a panic in the `In` effects or a dropped future leaves neither entry. `CP` (R202.1/.2/.5, `PT`, T released) runs through the blocking port, an interrupted capture's samples bypassing the supervisor; the executor parks across the pipeline await holding `Awaiting(PS)` and resumes with the pipeline's actual `Completed` / `Failed`. The legacy `process_audio` shares only the post-pipeline idle tail (204b) and its guard's parse failure (204a); its own sites 301/302 (C3.7, including H1/H2 registration and retirement on parse failure) are deferred per D-S2-1 to a later PR "2a-5 legacy process_audio" and the matrix row for `process_audio` stays "legacy inline". Re-entrancy (D-S2-2): the stop family's entry checks `CoreEntry` as its first statement, before any managed-state lookup or lock, as the lifecycle bridge does; the core span stays set across production port calls, so an inline listener that an `In` emit (or the 204b emit) invokes can neither enter the bridge nor stop again (it would deadlock on D). `Ports::emit_under_d` runs the guard's `on_idle` as `EM(idle)` with D's contents and every emit witness reports the real emit result. Unchanged and still inline: `cancel_*`, `start_native_recording`, `process_audio`'s own admission and the cancel `Err` of R206 after R201.4's `CC`. SP5 ratchet lowered: `transition_dictation_status` call sites 10 -> 6, `emit_dictation_terminal*` 14 -> 9 (two new callers: the stop `Ports::terminal` and the `TERM` hook of the tail; the R206 failure of R201.4's `CC` stays inline, with the start and cancel sites). One retirement rule (D-S2-6, D-S2-4): every RAII owner dropped inside a core span (`PipelineOwner`, a bare `ContinuationGuard`, the guards) does not retire on the spot; its H1/H2 retirement is one item in the deferred FIFO (kinds: H1-only, paired H1/H2, full finish, each carrying its generation and idle emit as owned data), behind any finish queued earlier, and runs on the drain after D is released. Outside a span every owner retires at once. A deferred explicit `finish` carries its owned idle emit and makes it at the drain (D-S2-7); the drain's finish runs its own D-protected generation check. Every entry that can open a core span declares a drain guard before T/D, and a test-build check asserts at the outermost entry's exit that no deferred work is queued without one (D-S2-8). The stop's perf stamps keep their base boundaries (D-S2-12): `pipeline_returned_at` right after the pipeline await, the idle-status measurement ends when the 204b D section ends, before any deferred drain. The pipeline's own D sections after its awaits do not get drain guards (D-S2-5: production polls dictation futures only from Tokio tasks and IPC handlers, and `until_outside_core` after T is defence in depth). |
| **2a-3** (cancel family: sites 205, 206) | **Authoritative writer, per input moved.** The single writer of M1 `status` for the cancel command is the executor's `ST` effect through `FieldAdapter::set_status` at R205.5's `In` placement, under the D hold the section already takes (`Processing→Idle` only; R205.4 writes no status: `Recovering{User}` comes from the supervisor, R209). M2 is not written by any cancel row. M3 `cancelled_through`: the executor's `MC` through `FieldAdapter::cancel_through` (R205.5 `PDT`, after `CX-`), live only, never read by a cancel decide (the view is `project_lifecycle_view`, M3 as an atomic read the family ignores). M4: `CX-` (R205.5 `PDT`) writes only the live slot through `FieldAdapter::clear_context`. H1: R205.4 returns `K:CaptureCancel`, registered in `core_history` right after the decide and before any effect; site 206 (the cancel command's capture-cancel result and, now, the stop command's R201.4 result) consumes it through `validate`, decide, `ret_if_matches`, and a flight dropped mid-way retires it through the owner's `Drop`; `op_status` never branches a row. H2: untouched (R205.5 leaves `pipelines[rid]` in place: it is draining until R204a, C3.9). The cancel entry checks re-entrancy (`CoreEntry`) as its first statement, before T, D or `core_history` and before any managed-state lookup; the cancel ports run with `IN_CORE` set (no `outside_core`, decision D-S2-2), and the golden harness's fake supervisor delivers its inline events on a joined thread, as an independent input. Unchanged and still inline: `start_native_recording`, `process_audio` (own PR, D-S2-1). SP5 ratchet lowered: `transition_dictation_status` call sites 6 -> 5, `emit_dictation_terminal*` 9 -> 7 (the cancel command's two inline emits and the stop's R206 inline emit are gone; one new caller: the cancel `Ports::terminal`). |
| **2a-4a** (start request: sites 101, 102) | **Authoritative writer, per input moved.** The single writer of M1 `status` and M2 `current_id` for the start command is the executor's `ST` / `ID+` effect through `FieldAdapter::set_status` / `alloc_id`, at the decision's `In` placement under the D hold the section already takes: R101.12 `Idle→Recovering` (old id), R101.15 `ID+` then `Idle→Starting`, R102.2-.5 `→Idle` (only for the current id, PD-3, PD-6). M3 and M4 stay live-only: `CX-` (R102.2-.6 `Pre`, before D) writes only the live slot through `FieldAdapter::clear_context`; no start decide function reads M3 or M4 (the view is `project_lifecycle_view`). H1: R101.15's `K:CaptureStart` is registered in `core_history` right after the decide, before any effect, and site 102 consumes it through `validate`, decide, `ret_if_matches`; R102.1 returns `K:ResolveContext`, registered by the shell and retired by its owner's `Drop` until site 103 moves (2a-4b). `op_status` never branches a row; a flight dropped mid-way retires its op through the owner's `Drop`. H2: untouched. The kept-preview `CS` is the awaited effect: the cursor is parked across the `spawn_blocking` await with no port or executor held and resumed (after the wait outside any core span) with the supervisor's real result. `resolve_smart_auto` is a `DictationHost` port, read only when the decision asks for it. It runs with the core span set (`observe_in_core`, its lock placement unchanged): its success emits `smart-auto-microphone-changed` inline under T and D, so a Rust listener that re-enters the start is refused (D-S2-2) and an `IdleGuard` it drops queues its finish for the drain (D-S2-4). The start entry checks re-entrancy (`CoreEntry`) as its first statement, before the claims, T, D or any managed-state lookup (D-S2-2); the start ports keep `IN_CORE` set; the drain of re-entrantly queued finishes (D-S2-4) runs in the synchronous stretch that holds D, never across an await. D-S1-5(c): the start-site fallbacks (`reply None => admitted`, `_ => {}`, the `audio_recovering` `map_or_else` and the two `debug_assert`s) are exhaustive matches with `unreachable!`. Unchanged and still inline: the smart-destination capture, the context install (103), the readiness publish (104) and the model preparation (2a-4b), `process_audio` (D-S2-1). SP5 ratchet lowered: `transition_dictation_status` call sites 5 -> 2; `emit_dictation_terminal*` 7 -> 7 (the start's failure `TERM` moves into the start `Ports::terminal`; the superseded `TERM` of site 103 stays inline until 2a-4b). |
| **2a-4b** (start context and ready publish: sites 103, 104, query 402) | **Authoritative writer, per input moved.** M1 `status`: the single writer of the readiness publish (R104.2 `Starting→Recording`) is the executor's `ST` effect through `FieldAdapter::set_status` at `In`, under the D hold the publish already takes (`TL`, `EM(recording)` and `SD` follow at `In`; `SD` runs through the new `Ports::detached_under_d`, so the partial ticker registers under that same hold). M2: not written. M4 `active_context`: `CX+` (R103.1, R103.2 `In`) writes only the live slot through `FieldAdapter::set_context`; no decide function reads M3 or M4 except R103.0's `q_capture_current`, which reads M3 as an atomic observation (the view). H1: R102.1's `K:ResolveContext` is consumed at site 103 through `validate`, decide, `ret_if_matches`; R103.1 returns `K:ReadyPublish`, registered by the shell before any effect and consumed by the publish's own decision (R104.1 D-less, R104.2-.6 under D); every flight end retires its op, a dropped flight retires it through the owner's `Drop`; `op_status` never branches a row. H2: untouched. Staged reads (D-S1-2): the supervisor phase read stays before D (R104.1 takes no D); the installed context is read only after the id and status guards passed. The smart-destination capture await is followed by `until_outside_core()` before the next D sections (D-S2-3). The smart-destination emit's D section is owned by the shell (it was the host helper's hidden lock): R103.0 rechecks generation ownership under it and `In:SDE` runs under that hold, with the real emit result as its witness and the ownership gate suppressing the effect before dispatch (it is then absent from the decision); a drain guard declared before D covers return and unwind (D-S2-4). The ready publish takes the start's own `&State` through the host seam, so the audio-not-ready path takes no managed-state lookup. Appendix deviations (code wins): R104.7 is its own pure function so the flight end does not cross the host seam; R103.0' has no row id of its own in the registry (`R103.0`, verdict `Stale`). SP5 ratchet lowered: `transition_dictation_status` call sites 2 -> 1; `emit_dictation_terminal*` 7 -> 6 (the superseded `TERM` of site 103 moved into the start `Ports::terminal`). |
| **2a-5** (legacy `process_audio`: sites 301, 302) | **Authoritative writer, per input moved.** The single writer of M1 `status` and M2 `current_id` for the import is the executor's `ST` / `ID+` effect through `FieldAdapter::set_status` / `alloc_id` at R301.5's `In` placement (`ID+`, `ST→Processing`, `AX(s)`), under the D hold the section already takes; `KB(true)`, `EV(GenStarted)` and `EM(processing)` run `PT`, after D and T are released. M2 is written only by `ID+`. M3 and M4 stay live-only and no 30x decide reads them. H1/H2: R301.5 returns `K:Pipeline`; the shell registers it with `pipelines[rid]` (`PipelineOwner::register`) right after the decide and before any effect (C4.3). The owner is held until the pipeline's own guard takes it over (`IdleGuard::take_owner` into `PipelineStopContext.pipeline_op`) and retires both at its finish (204a `PipelineExit`, C3.9); on every earlier exit it retires them itself: a parse failure (R302.1: `validate`, decide, retire H1 and H2 before the guard's `In` effects), a stale or cancelled capture return, a panic (`Drop`); an owner dropped inside a core span (an emit listener drops the owner itself) does not retire on the spot: its H2 and H1 retirement is one history-only item in the deferred FIFO, behind any finish queued earlier, and runs on the drain after D is released (D-S2-4). A refusal (R301.1-.4) registers nothing and burns no id. Site 302 is the 204a `LegacyParse` and 204b `LegacyPost` rows decided by `decide_process_audio_finish`, called inline at the guard's finish section and at the shared post-pipeline tail. Entry (D-S2-2/3): `refuse_reentrant` is the first statement of the command's body, before the transition await, T, D or `core_history`; T is taken through `transition_outside_core`, which parks on every poll made inside a core span (an import that parked on T and is polled by a listener holding D takes neither T nor D) and waits the span out again after T, as the stop family does; the drain of re-entrantly queued finishes (D-S2-4) covers the synchronous 301 section and ends before the destination-capture await, which is followed by `until_outside_core()` before the next D read. The Smart Destination emit follows the start's pattern (D-S2-11): after a short D read of query 402 (`q_capture_current`) the shell owns the emit's D section, rechecks generation ownership there (`decide_process_audio_destination`, R301.6: the start's R103.0 decision under the import's own row id) and runs `In:SDE` under that hold, with the emit's real result as its witness and the ownership gate suppressing the effect before dispatch; the executor's span keeps `IN_CORE` set around the emit and a drain guard declared before D runs a finish an inline listener queued on the return and on an unwind, after D is released (D-S2-4). The host helper that used to take its own D section and gate is gone. The command's own host seam (`ImportedAudioHost`, `process_audio_ports.rs`) passes the existing `&State` through; no port does a managed-state lookup. Appendix deviations (code wins): R301.6 (`decide_process_audio_destination`) is an import destination row the appendix does not list: the import's Smart Destination recheck needs the start's R103.0 decision under a row id of its own so shadow capture never credits site 103, and the row registry holds 141 rows (D-S2-14); R301.1 is `decide_process_audio_start_pre` (no D); `benchmark.is_running()` takes the coordinator mutex, so it is read through `NeedObservation(BenchmarkRunning)` after the file check (D-S1-2); R302.1/R302.2 delegate to the 204a / 204b decisions but return site 302's own row ids (`R302.1`, `R302.2`: a decision's row must belong to the site that produced it, and the tests compare effects, verdict, transition and continuation with the delegates row by row); R302.2's reply is the success reply, and a pipeline that returned `Err` has no row: after the 204b section ran, the shell returns the pipeline's error text as a separate shell tail (`pipeline_result?`), outside the shadow contract; `ProcessAudioFinishInput.closure_emit` carries a deferred finish's missing idle closure; the success reply has no `state` key and the shell keeps today's JSON. Unchanged and still inline: the context resolve and the performance begin. The shared C0.3a queries now back three more gates with their lock scopes, short-circuit order and the ticker claim unchanged: the import's capture check (query 402), the partial-ticker registration (query 401, then the claim) and the delivery ownership gate (query 403); the old predicates survive only as test oracles with parity tests. Shared helpers touched, with no change for the stop family: `IdleGuard::finish_section` and `run_pipeline_idle_emit` pick site 302's function for the legacy origin / kind and the 204a / 204b function otherwise; `IdleGuard::for_legacy_parse` now takes the import's owner. SP5 ratchet lowered: `transition_dictation_status` call sites 1 -> 0 (the helper is test-only); `emit_dictation_terminal*` 6 -> 6 (the import never emits a terminal). |
| **2c-1** (shadow infrastructure, feature `dictation-shadow`) | **No writer changes.** `DictationCore::handle` (crate `handle`) is pure and reads no live field. The shadow thread owns a `ShadowCore` (M1 `status`, M2 `current_id` only), re-synced from every header's `pre`; M3 and M4 are replayed from the header as observations and compared only as mirror witnesses (`diff_kind` 5). Op ids are replayed from the live shell, never drawn; H1/H2 are not copied. The live side only enqueues: one attempt to store an owned, heap-free message in a preallocated in-crate bounded ring (256; the Vyukov queue of `dictation_shadow/ring.rs`, one consumer, no loop, backoff or yield), with no allocation, free, lock, channel or wake-up (a counting-allocator test counts both allocations and frees on accept, on rejection and on the drop of a rejected header; Smart Auto's refusal is the typed `SmartAutoFailure`, and its reply text stays shell-owned), and a rejected message costs two atomic counters (a global `lost` and one per site), nothing else. The consumer stages each batch and reads `lost` again: if it moved, the batch and every open, held or waiting record are given up (`abandoned_by_loss`), and a later header whose ticket predates the loss is not admitted, its place in the sequence consumed as an explicit skip (Sol round 5; D-S2c-4 d); no loss is attributed to a subject (D-S2c-3, replacing D-S2c-1 c's attribution). Every opened recorder is closed exactly once through one `close(recorder, kind)` (`opened` equals the close counters plus the recorders still open; a second close is a counted no-op, D-S2c-4 b; the closed set never expires and an eviction is a close, D-S2c-5 a, b), and a panic reset closes the flight being processed as well (c). An `SD` (`In` or `PostDT`) is witnessed by exactly one ticker gate or one numeric `TickerSkipped { target, reason }` (the spawner returned early, D-S2c-6, D-S2c-7; for an `In:SD` it follows the header and the expectation stays open until the close; its target is checked like a gate's, the reason is recorded per row, not compared). `SPEC`'s value is an `Observed` mark that a post-D conditional requires (D-S2c-7 amendment 2: missing is `diff_kind 7`, with no header fallback; the header's value serves only header-witnessed conditionals); `Pre`/`In` keyboard and cancelled-through writes are header mirrors. A pipeline-owned finish is clean only with the owner's full H1 token from the shadow's own registration witness, a registration validated as the only one for its recording (a later conflicting one invalidates it), compared with the operation it retired at its close; otherwise `unverifiable` (D-S2c-4 a, D-S2c-5 d). Mismatches go to `dictation-shadow-events.jsonl` from the comparison thread, never through `tracing` (b); the comparison thread alone writes the summary (f); `PD` counters need the defining behaviour witnessed (d). The feature refuses to compile under `panic = "abort"`; soak builds use the `shadow-soak` profile (e). Nothing in the shell sends a message until the capture sites of 2c-2 land. |
| **2c lifecycle cutover** (supervisor-lifecycle D entries: sites 207, 208, 209, 210, 211, 216) | **The lifecycle row of the matrix moves from "adapter" to "core".** The six D entries of `handle_audio_lifecycle_events` now build their decision view with `FieldAdapter::lifecycle_view`: M1 `status` and M2 `current_id` come from the **persisted core** (`DictationState.core`) instead of `DictationCore::project` over the live fields, and M3 stays the same atomic observation (M4 stays unread by every lifecycle decide). **The C8.6 assertion** runs in that one function, on every lifecycle entry, under the D hold the entry already takes: the persisted M1/M2 are compared with the live `status` and `recording_id`; a difference is **counted** (`adapter::core_divergences`, the shadow summary's `core_divergences`, the soak's `diff_kind 5` gate) and then **re-synced** through `seed_core` before the decision runs, never ignored. Writers are unchanged: `ST`/`ID+` still write live and the persisted copy through the adapter; the D-less entries (212-215) keep their lock-free reads. `project()` and the legacy inline code stay compiled; the lifecycle entries no longer reach `project()` (`no_lifecycle_entry_projects`), and the stop, cancel and start families still do. Nothing is deleted, no flag or migration is added, and `git revert` of the merge commit restores the `project()` entries. The shadow's role for the family is executor fidelity plus `pd_witness`: its header `pre` is now the persisted M1/M2, and CP1 compares them, with the decided commits, against the live post-state. The production ticker and VAD spawners (and the window helpers they reach) became generic over the runtime so the ready-handoff runtime check runs the real spawners (D-S2c-16 amendment 1 c). **Queries 401/404 are not part of this cutover:** the ticker-registration query still projects, because it is shared with the start family's R104.2 and moves with the **start** cutover. |
| **2c stop cutover** (stop D entries: sites 201, 202 without a context, 203, 204a, 204b, and the legacy import's 301 with its destination row 301.6) | **The stop row of the matrix moves from "adapter" to "core"; the legacy `process_audio` (301/302) and `IdleGuard` rows switch with it (C1.4.3).** Each of those D entries builds its decision view with `FieldAdapter::stop_view`: M1 `status` and M2 `current_id` come from the **persisted core** instead of `DictationCore::project` over the live fields, M3 is the same atomic observation and M4 stays `None` (no stop decide reads it). `stop_view` is the stop family's own thin wrapper over lifecycle's shared `persisted_view` (D-S2c-18), with the stop probe: **the C8.6 assertion** compares the persisted M1/M2 with live under the D hold the entry already takes, **counts** a difference (`core_divergences`, the shadow summary's field, the soak's `diff_kind 5` gate) and **re-syncs** it through `seed_core` before the decision runs. Writers are unchanged. Sites 302.1 and 302.2 are the same `finish_section` (204a, all four `IdleGuard` origins) and `run_pipeline_idle_emit` (204b) sections, so the import's finish and tail switch with them. D-less reads stay lock-free: 202.1/202.2 (context present), the stale stop R201.1, and site 206. **Queries 401, 402, 403 and 404 are not part of this cutover:** they are shared with the start family (the ticker registration, the destination capture check, the delivery ownership gate) and move with the **start** cutover, so the import's capture check (402) still projects once. `project()` and the legacy inline code stay compiled; the stop entries no longer reach `project()` (`no_stop_entry_projects`), and the cancel and start families still do. Nothing is deleted, no flag or migration is added, and `git revert` of the merge commit restores the `project()` entries. The shadow's role is unchanged: `dictation_shadow` held no stop-specific legacy comparison (its one `project()` call re-seeds the shadow's own core and is family-agnostic), so nothing was removed; `pd_witness` continues, and the header `pre` of a stop flight is now the persisted M1/M2. PD-25 stays **preserved**: both allocators (the import R301.5 and a start R101.15) are pinned through the switched entries, with the terminal winner (R201.1 `Superseded/StaleOwner`) and the performance winner (R204b.6 `Success`) asserted; its fix is a later PR. |
| **2c cancel cutover** (cancel D entry: site 205; site 206 is D-less) | **The cancel row of the matrix moves from "adapter" to "core".** The cancel command's one D entry, site 205 (`decide_cancel_request`, R205.1-.5, decided under the D hold the section already takes with T held), builds its decision view with `FieldAdapter::cancel_view`: M1 `status` and M2 `current_id` come from the **persisted core** instead of `DictationCore::project` over the live fields, M3 is the same atomic observation and M4 stays `None` (no cancel decide reads it). `cancel_view` is the cancel family's own wrapper over the shared `persisted_view` (D-S2c-18), with the cancel probe: **the C8.6 assertion** compares the persisted M1/M2 with live under the D hold the entry already takes, **counts** a difference (`core_divergences`, the shadow summary's field, the soak's `diff_kind 5` gate) and **re-syncs** it through `seed_core` before the decision runs. Writers are unchanged. Site 206 (`decide_cancel_capture_result_nolock`, the capture-cancel result shared by the cancel command's R205.4 and the stop command's R201.4) is D-less and reads no live field, so it has no entry and keeps its lock-free reads; its stop caller is the stop family's own switched entry 201. R205.5's transition-lock boundary is unchanged and re-run through the switched entry (T held from `ST` through `EV`, released only before `PT:PC`, so a newer start's `starting` is never overwritten by the old cancel's trailing `idle`). R205.4 and R201.4 keep their delegated reply (`replay::DELEGATED_ROWS` `[(201, 4), (205, 4)]`, E3 credit `matched + delegated`, unchanged): site 206 consumes the command's own `K:CaptureCancel` and claims the terminal for the cancelled generation only. `project()` and the legacy inline code stay compiled; the cancel entry no longer reaches `project()` (`no_cancel_entry_projects`), and the start family still does (queries 401/404 and the start entries move with the **start** cutover). Nothing is deleted, no flag or migration is added, and `git revert` of the merge commit restores the `project()` entry. The shadow's role is unchanged: `dictation_shadow` holds no cancel-specific legacy comparison (its one `project()` call re-seeds the shadow's own core and is family-agnostic), so nothing was removed; `pd_witness` continues, and the header `pre` of a cancel flight is now the persisted M1/M2. |
| **2c start cutover** (start D entries: sites 101, 102, 103.0, 103, 104, the capture check 402, and queries 401, 403, 404) | **The start row of the matrix moves from "adapter" to "core"; with it the last `project()` entries and queries go, so no production entry or query reaches `DictationCore::project` any more.** The start command's D entries (101 `decide_start_request`, 102 the capture-failure section, 103.0 `decide_start_destination`, 103 the context install, 104 `decide_ready_publish`) build their decision view with `FieldAdapter::start_view`: M1 `status` and M2 `current_id` come from the **persisted core** (`DictationState.core`) instead of `DictationCore::project` over the live fields, M3 is the same atomic observation and M4 stays `None` (no start decide reads it; the context presence is a `NeedObservation`). `start_view` is `lifecycle_view`'s body (`persisted_view`) with the start probe: **the C8.6 assertion** compares the persisted M1/M2 with live under the D hold the entry already takes, **counts** a difference (`core_divergences`, the shadow summary's field, the soak's `diff_kind 5` gate) and **re-syncs** it through `seed_core` before the decision runs. **Query 402** (`q_capture_current`, the short D read after the destination capture, in the start and in the legacy import's `process_audio_inner`) takes the same `start_view`, so it re-syncs like an entry. **Queries 401/404** (`q_is_recording_current`, the ticker registration `register_partial_ticker_locked` and its shadow-observed twin) and **403** (`q_owns_delivery`, the delivery ownership fence) hold only `&DictationState` (the shared `Ports` trait hands R104.2's `SD` that reference, and the fence is held across a paste), so they use `FieldAdapter::start_query_view`: the persisted M1/M2 and the same M3 atomic, a difference **counted** but not re-synced (the next entry re-syncs it; R104.2's `SD` follows the ready entry's own `start_view`). Writers are unchanged. D-less reads stay lock-free: R104.1 (the audio is not ready), R101.1/R101.2 and the other refusals before D, R104.7. `project()` and the legacy inline code stay compiled; no entry or query reaches `project()` (`no_start_entry_or_query_projects`), nothing is deleted, no flag or migration is added, and `git revert` of the merge commit restores the `project()` entries and queries. The shadow's role is unchanged: `dictation_shadow` holds no start-specific legacy comparison (its one `project()` call re-seeds the shadow's own core and is family-agnostic), so nothing was removed; `pd_witness` continues, and the header `pre` of a start flight is now the persisted M1/M2. D-S2c-17 (the last shadow IPC wrapper): option (i), the start handler pair (`shadow_commands.rs`) stays in place until the #1154 cleanup. |

## Stage 1 exit artifacts

### C3 completeness, per site and per row

"C3 complete for every site" (plan §6 Stage 1) is checked by two crate tests that
meet at the site id:

- `inputs::tests::every_decision_site_has_an_input`: each of the 24 `SiteId`s is
  reached by exactly one `Input` variant (`inputs::every_input()` builds one of each).
- `rows::tests`: `C3_ROWS` holds the 141 row ids of the C3 tables (C3.1-C3.7, plus R301.6) in
  appendix order; ids are unique, each id's `R<site>.<n>` prefix is its site's
  code (202 is split: R202.1-.2 are `StopContextNoLock`, R202.3-.5 `StopContext`),
  every site owns at least one row, and the per-site counts equal the counts in
  the appendix tables (`ROWS_PER_SITE`).

Rows per site: 101: 15, 102: 6, 103: 7, 104: 7, 201: 8, 202: 3, 202n: 2, 203: 21,
204a: 10, 204b: 10, 205: 5, 206: 2, 207: 5, 208: 2, 209: 7, 210: 4, 211: 6, 212: 1,
213: 2, 214: 2, 215: 3, 216: 5, 301: 6, 302: 2 (total 141). What the tests do **not**
prove is each row's behaviour; the per-row unit tests are due at the Stage 2a PR
that extracts the row (C7.0). If the appendix gains or loses a row, `rows.rs` and
`ROWS_PER_SITE` change with it and the tests fail until they do.

### Stage 1b placement re-check (A vs B2)

Stage 1b (#1141) routes the admission *predicates* through `admit()`; it moves no
B2-placed effect. Re-checked against the diff, site by site:

| Site (B2) | Effect statements next to the changed predicate | Same place, same order? |
|---|---|---|
| Start, pre-D (R101.1-.2) | `PT:REL` is the early `return Ok(app_disabled)` / `Err(..)` | yes: still returned from the same statement position, with its `tracing::info!` first |
| Start, cleanup arm (R101.12) | `ST` (`transition_dictation_status(Recovering)`) then `EM` (`recording-status-changed`) then the reply | yes: the supervisor read, then ST, EM, reply, all under the same D hold |
| Start, Smart Auto (R101.14) | none (refusal returns before the id bump) | yes: resolve still after the cleanup check, before `next_recording_id` |
| Stop/cancel/lifecycle sites | not touched | n/a |
| `transcribe_file` | claim swap under T, `drop(T)`, then the `file-transcription-status-changed` emit, then the D recheck | yes: the swap runs inside the pre-claim call before T drops; emit position unchanged |
| Meeting claim | stores, `relaunch_committed` re-read, undo stores | yes (Dekker order) |
| Mic-startup benchmark | `try_start()` under D, second claim under T, `coordinator.finish()` on failure | yes |
| Benchmark, corpus, model removal, model preparation | claims/guards only (`try_start`, `corpus.claim`, `try_start_shared_backend_change`, `ChangeGuard`) | yes: claim stays where it was |

No `ST`, `EM`, `TERM`, `KB`, `PERF` or `REL` effect other than the start's own
`PT:REL` returns and the cleanup arm's `ST`/`EM` sits inside the changed regions,
and those keep their placement (`Pre`, `In`).

## Rules for Stage 2a (decision D-S1-2): staged fact reads

Stage 1b reads admission facts **lazily, in today's order**, through the shell's
`Staged` (`dictation_admission.rs`): one fact group at a time, `admit` (or
`admit_after`) evaluated after each group, the first non-`Admit` ends the call, and
a fact today's code reads only after an earlier check passed is never read before
that check. Reads that are not free (the supervisor's `is_audio_active`, which
builds the global supervisor on first use; the Stage 0a supervisor ports; a mutex
taken under D) are exactly the ones that must stay lazy.

Stage 2a decide functions keep that rule:

1. **No decide function builds a fully-populated `Occupancy` up front.** It must
   not force an eager read of a fact today's code reads only after an earlier check
   passed (for example `is_dictation_cleanup_pending`).
2. Where a decision needs such a fact, either the decide function returns
   `NeedObservation` for it (the shell reads it in the same D hold and calls the
   function again), or the shell calls the staged admit in place and passes the
   verdict in.
3. A partial `Occupancy` is valid input: unread facts are `Default` (quiet), which
   no check treats as a refusal, so evaluating after each group is equivalent to
   the old lazy chain.
4. Every family PR's row tests include the op-status parity half of
   `unknown_op_runs_site_stale_rule`: each continuation-consuming row yields
   identical effects for `Outstanding` and `Unknown` (Stage 2a PR 0 pins the shell
   protocol, `CoreHistoryLock::consume`, with a stand-in site rule).
5. The executor (`dictation_executor::executor`, Stage 2a PR 0) is called from the
   shell at each site's own boundaries: `Executor::run(phase)` per placement, the
   shell awaiting any `Awaiting` effect and re-entering through `resume` and the
   next decide. A family PR lowers `STATUS_WRITE_SITES_BASELINE` /
   `TERMINAL_EMIT_SITES_BASELINE` in `dictation_executor/ratchet_tests.rs` as it
   moves its sites behind `ST` / `TERM`. The terminal emits that remain are also
   pinned per enclosing function (`TERMINAL_EMIT_ALLOWLIST`), so swapping a `TERM`
   call for an inline emit in another function fails even at an equal count.

## Rules for Stage 2a (decisions D-S2-5 to D-S2-8): core spans, polling and the deferred queue

These rules cover what the shell does when a listener (an inline `emit` handler, a
port callback) runs inside a core span, that is, with `IN_CORE` set and D or T held.

1. **Polling rule (D-S2-5).** Production polls dictation futures (the start, stop,
   cancel and import commands, the lifecycle bridge) only from Tokio tasks and IPC
   handlers, never from inside a listener. A listener may drop a pipeline owner or
   a continuation guard (which queues work, rule 2), never a dictation future
   (D-S2-16: drops follow the same rule as polls). Entries still protect themselves:
   an entry reached inside a span is refused before any lock (D-S2-2/3), and a
   future that is resumed inside a span after an await waits with
   `until_outside_core().await` before it asks for T or D. That wait is defence in
   depth for the tests and for a future producer of such a poll, not a mechanism
   production relies on. The pipeline's own D sections after its awaits get **no**
   guards of their own: they are reached only by the rule above.
2. **One retirement rule (D-S2-6).** Every RAII owner dropped inside a core span
   (`PipelineOwner`, `ContinuationGuard`, the start's guards, `IdleGuard`) routes its
   history retirement through the one deferred FIFO (`DEFERRED_FINISHES`); outside a
   span it retires at once. The FIFO holds three kinds of entry (H1-only, paired
   H1/H2, a full finish), each carrying its generation and idle emit as owned data.
3. **A deferred `finish` carries its idle emit (D-S2-7).** Production passes an owned
   emit (`IdleGuard::finish_owned`, for the import `ImportedAudioHost::idle_emitter`);
   the drain runs it after D is released, behind a D-protected generation check of its
   own.
4. **Every entry that can open a span declares a drain guard before the first effect
   (D-S2-8).** `DrainDeferredFinishes` is declared before T and D (a guard is never held
   across an await that has queued work behind it: the stop uses one per synchronous
   stretch around its pipeline await, and the start flight follows rule 5), so it
   drops after both, on a return and on an unwind. Test builds assert
   at the outermost span's exit that no deferred work is queued without a guard
   alive, and at the guard's drop that nothing is left queued. The audit table of
   every entry and its guard is in the Stage 2a-5 PR description.
5. **A drain never runs while the T of the entry or flight that queued the work is
   held (D-S2-13), on a return or an unwind.** Declare the drain binding before T so
   the drop order is T, then D, then the drain; where an explicit early T release
   exists, keep it. The counted test guard is created after the awaits that
   precede the synchronous stretch (T, then the span fence): its live count is
   thread-local, and a future that waited on T may resume on another worker. A
   start flight holds T across awaits, so its sections do not drain: each
   synchronous stretch (`FlightStretch`) hands what its spans queued up to the
   flight's own `FlightDrain`, declared before T, which keeps the work in an owned
   list carried by the future across awaits (nothing stays on a thread-local queue
   while the flight is suspended, and no other entry on that thread can run it) and
   drains after T is released. Deferred entries own what they need (an owner, an
   idle emit, a guard) and never hold a borrow of, or a pointer to, an `AppState`
   (D-S2-16 a): the drain supplies the state, and an entry is only run by a drain
   of its own state (a retirement needs no state and runs under any drain).
   Work is ordered **per owner** (D-S2-15): a flight takes and drains only the work
   it queued itself, in queue order, and an entry's drain takes only its own. Two
   orders between different owners are therefore both permitted: F then R (a
   suspended start's carried finish F drains when the flight ends, then an
   independent entry's retirement R) and R then F (R drains while the flight is
   suspended, F after). They are equivalent because R is a history-only retirement
   of an unrelated owner's H1/H2 entries (no status, id or persisted-core write)
   while F decides from the live view and its own owner, so the two commute: the
   persisted core, live status and generation, H1/H2, the counters and F's single
   idle emit are identical in both orders (a test compares the typed end states).
   Order **within** one owner is never exchanged.

## Rules for Stage 2b (decision D-S2-5): polling dictation futures

Stage 2b's generator drives the same entries and must keep rule 1 above:

1. **The generator never polls a dictation future inline inside a core span.** A
   listener, an emit handler or a port callback that runs with `IN_CORE` set must never
   poll one (nor drop one, below); it may drop a pipeline owner or a continuation
   guard, whose retirement is deferred. The `until_outside_core` fences are
   defence in depth, not the contract.
   The same holds for drops (D-S2-16): production drops a dictation future only on a
   runtime worker, between polls (a task completing or `JoinHandle::abort`), never
   inline in a core span, and there is one `AppState` for the process. A start flight
   dropped inside a span is out of contract: a debug assertion in test and debug
   builds, and in release its carried work goes, as owned entries under its own
   tag, to the ordinary thread queue.

2. **The harness polls a future once and does not follow `until_outside_core`'s
   self-wake.** A future parked on `until_outside_core().await` inside a span wakes
   itself and returns `Pending`; the golden harness (`poll_once`) polls it once, with
   no waker that re-polls, and polls again only after the span ended. A driver that
   re-polled on the self-wake would spin inside the span.

## The property harness (Stage 2b-1, appendix C7.5)

`src/property/` is a `#[cfg(test)]` module of this crate (CI runs `cargo nextest
run -E 'kind(lib)'`, which does not run integration tests). It composes the **real**
`decide_*` functions of every site with an interpreter of their placed effects over a
model world (M1-M4, the telemetry ledger, the performance and diagnostics sinks,
`core_history` H1/H2, the supervisor's owner and phase). It has no dependency: a
splitmix64 generator turns a seed into the whole run.

**What the harness is evidence for.** Every flight and every supervisor arm decides
through `DictationCore::handle` (`entry.rs`, #1160), so the 10,000 seeds exercise the D /
D-less entry choice (`202`/`202n`, `204b`, the `*_nolock` entries) and every
`NeedObservation` round trip, the harness answering each observation from the model world
where the shell reads it. The flight keeps its old direct `decide_*` call as a cross-check:
a `handle` decision that differs from it fails the seed (`handle: ...` violation), and the
flight acts on the `handle` decision, so a routing defect also shows downstream. The
cutover evidence is that, the goldens, the replays, the harness-findings shell tests
(`recording_harness_findings_tests.rs`) and the shadow comparison E1-E9.

* **Generator** (`gen.rs`, `supervisor.rs`): commands (start, stop, cancel; ids
  `current`, `current-1`, `current+1`, unscoped) and, per start, one word of the C7.3
  `Attempt` grammar. A start has a caller (the main window, the overlay or the IPC) and a
  stop has one; the main window's start reply is delivered to it as a step of its own that
  writes the reply's status unless it is a stale `starting` (`useRecordingState.ts:465-486`,
  #1162), so a late reply can put `Recording` back over the window's local `Processing`. The
  reply is tied to the one start that produced it: the main window's `beginRecording` keeps
  `startOperationRef` until the reply is processed, so a second main-window start gesture
  meanwhile is coalesced and issues no IPC (`:456-459`); the reply carries its recording id,
  and the window tracks the newest generation it saw start and end (`:174-178`, `:244`), so
  a delayed `starting` reply of an ended generation is ignored (`:474-476`). An `Err` reply is
  a failed invoke that `lib/dictation.ts` turns into an error response with no `state`: it
  writes no status and ends the start operation. A start also carries its microphone selection
  (system default, a fixed device, Smart Auto resolved or failed, Smart Auto with a fixed
  device), so the gate's Smart Auto observation round (R101.13, R101.14, R101.2) is exercised;
  and a seed in four has a legacy `process_audio` import (sites 301 and 302: the meeting, file
  and benchmark refusals with the benchmark read through `NeedObservation`, the status
  refusal, the accepted import, its Smart Destination recheck, the parse failure and the
  pipeline tail), which is decided through `handle` like every other site. What the microphone
  and the imports draw comes from a second RNG stream (`extra`), so it moves none of the
  scenario's or the scheduler's other draws. Every supervisor word ends with an explicit `OwnerClear` step
  (followed by `CleanupFinished` for a sealed stop only); `cleanup_pending` is derived
  from the model (owner set and phase `Stopping`/`Recovering`), never generated.
* **Supervisor mirror** (D-S2b-9 (a), `mirror_table.rs`): `MIRROR` is a table in which
  every modeled supervisor or client step (and every helper the model mirrors:
  `is_dictation_cleanup_pending`, `is_dictation_recording`, `take_interrupted_dictation`,
  `PublicState::owns` and the rest of the public-state accessors) cites the
  `audio_lifecycle.rs` function it mirrors (no line range: the function is hash-pinned, so
  the test finds each cited token sequence **exactly once** inside it and prints the lines it matched; a citation that matches zero or several times fails, so a guard cannot be satisfied by the same tokens in another match arm), with a
  fragment of source that must appear there, the **guard expressions** of the cited code (`guards`) and, where one
  exists, a test of the real supervisor that runs the behaviour (`real_test`: for the sealed
  reply `a_sealed_stop_reply_is_delivered_at_the_seal_before_worker_exit`; for cancel
  `a_cancel_from_recording_stops_capture_and_answers_true` and
  `a_cancel_from_recovering_has_nothing_to_abandon`; for the queries
  `a_cleanup_is_pending_only_for_the_owner_while_stopping_or_recovering` and
  `a_taken_interrupted_capture_is_gone_for_the_next_stop`). This crate checks that every
  message, worker event, timer and supervisor step of the model has a row
  (`every_modeled_step_has_a_mirror_row`). **The pins and the citation checks live in the
  app crate** (D-S2b-15), in `src/audio_lifecycle_mirror_tests.rs`, which includes
  `mirror_table.rs` by path (so the table the harness runs is the table checked) and
  parses the source with `syn`: every production function, method, constant and static of
  `audio_lifecycle.rs` has its **own pin** (`LIFECYCLE_PINS`: the FNV-1a hash of the item's
  normalized token stream), so any edit, addition or removal fails
  `every_production_item_of_audio_lifecycle_is_pinned` and names the item until its pin is
  updated. The **whole production file is pinned too** (`LIFECYCLE_FILE_PIN`,
  D-S2b-17 (a)): the normalized tokens of every item except `#[cfg(test)]` modules and
  items whose own attribute is exactly `#[cfg(test)]`, so a serde attribute, a
  `macro_rules!` body, a methodless impl, a module's cfg or a `use` fails too; the per-item
  pins stay to name what changed. The normalization is the shell ratchet's (`src/token_pin_tests.rs`, shared:
  `proc_macro2` tokens, token kinds, punctuation spacing and commas kept, only whitespace and
  comments stripped), so a comment-only edit passes and a one-token edit fails. The
  citations are matched as token sequences, exactly once, within the cited, hash-pinned function
  (`span-locations` gives the matched lines for the diagnostics), so a comment inside a cited
  expression does not hide it and a line shift breaks nothing. A re-pin commit says the model step
  was re-checked against the change (`cargo test --lib print_current_pins -- --ignored
  --nocapture` in the app crate prints the tables). Every function a cited function calls is
  cited by a row or listed in `NOT_MODELED` with the reason it is outside the model.
  The core crate has no hand-written lexer and no dependency. A command (start, stop and cancel alike, each with its own
  receiver deadline) has five events: the client's enqueue (never blocked by a busy
  supervisor), the supervisor's acceptance, the reply at its production point (a stop's
  at the seal or the worker's exit, a cancel's when handled), the receiver's timeout and
  the client's continuation. An immediate refusal (a stop with no attempt or another
  owner, a cancel with nothing to abandon, a start refused because its caller was
  revoked or an attempt exists) is a step of its own after the acceptance, so the
  receiver can expire between the two and the reply is then dropped. A start whose
  receiver timed out revokes its caller and enqueues `Cancel{User}` (`await_start_response`).
  A notification fires only in a state where the cited code sends it.
* **Supervisor fidelity** (D-S2b-7 (b), `supervisor.rs`, `sup_task.rs`): the model may
  not invent or exclude a schedule. The worker's own end (a recovery, a crash, a failed
  open) is a pending end that a stop or a cancel command can still win until the
  supervisor takes it; the start's answer, a cancel's answer (after `begin_recovery`
  returned) and the attempt's results are steps of their own; `finish_attempt` is one
  uninterrupted group; `RecoveryStalled` is notified only while stopping or recovering; a
  cancel client's timeout is its own outcome (`CancelMode::Timeout`: `Err` while the
  supervisor still handles the cancel).
* **Scheduler** (`run.rs`, `flights.rs`, `sup_task.rs`): one thread; T-holder flights,
  D sections, the supervisor thread, each notification's bridge arm, the pipeline, the
  partial ticker (which takes and drops the partial-decode lease) and a task per
  duplicate or late token delivery (`redeliver.rs`) are separate actors. A D section is
  atomic; `Pre`, `PostDT` and `PostT` effects are one step each, so another actor's D
  section can land between any two. A blocked step changes nothing (a test compares the
  whole world). The README rules above hold: a start can only be dropped between steps,
  at its smart-capture await.
* **Checks** (`checks.rs`, `c3.rs`): SP1-SP4, SP6-SP10 after every step.
  * *C3 effect fixture* (`c3_fixture.txt`, `c3.rs`, D-S2b-9 (d)): a hand transcription
    of appendix-c.md C3.1-C3.7, one line per row of `C3_ROWS` (a snapshot of observed
    effects is not a fixture), reachable by the harness or not. Every decision, including
    each stale or valid input the model never reaches, must match one allowed outcome of
    its row with exactly those effects, each the stated number of times, so a row that
    grows an extra write, claims another outcome or emits a duplicate terminal fails. The
    typed failure travels through the model (`AudioFailureKind` to its `ErrCode`), and a
    row whose terminal code is the input's (R203.4-.8, R204b.5-.6, R213.2) is written
    `@in`: the code must equal the exact code the harness gave that decision, with no
    wildcard, so a row that rewrites it (`TerminationUnconfirmed` to `HostUnavailable`)
    fails. The
    continuation-ID bound is checked at every registration. SP2 for terminals: under D a
    stale input claims only what the row lists for a stale input. SP5 stays structural
    (the #1152 ratchet) and file admission stays a unit test.
  * *SP4(b) signatures*: a terminal and a performance outcome that disagree are judged
    at the step where the second one arrives. The split leaves the failure set only when
    one signature matches in full, with every element bound to the same window: the PD,
    the terminal rows with outcome and code, the statuses the claim saw, the performance
    outcome class (or exact rows) and a causal precondition read from what each winner
    recorded (`Won`: the cancel's tail open, the order of the winners, a later claim
    lost). PD-4 is exactly its C7.1 definition (R207.3 on a live `Recording`, a second
    ticker, `Success`); the same claim with another outcome is PD-23, whose signature binds
    each outcome to the stop, cancel or pipeline rows that complete it (a class is not
    enough), and whose `Success` variant needs the ticker's gate to have refused the second
    ticker (pending at the shell: source-only). PD-24 has one signature per cause, R209.3
    (the start timeout's own `Cancel{User}`) and R209.5 (an environmental recovery), and
    is a mismatch at the performance **coordinator's claim**: the failed start never began a
    run, so no row is behind it. PD-22 needs the winning `Idle` claim and the losing named
    claim in the same cancel tail and is judged when that tail closes, because the window's
    events land in either order. There is no stale-input skip (D-S2b-13 (d)): SP4(b)
    compares the outcomes a generation actually has, and the model follows the performance
    run's lifecycle (`request_completion`): a completion is the run's outcome once the run
    began, waits while the begin is in flight, is dropped by an abort, and is a bare claim
    of the coordinator with no row when no run was registered (an aborted start, seed
    8096); only a signature marked `no_run` (PD-24) credits such a claim. No allowlist entry
    is widened to make a seed pass: a split no signature covers is a finding to reproduce
    at the shell (PD-23, PD-24) or a model fix. No PD exempts a whole recording.
  * *SP10*: the partial-decode lease changes only by its holder's own acquire and drop,
    and is released by the end of the run.
  * *C7.3 liveness* is evaluated for every run from per-generation evidence, never by
    filtering a run: a stuck `Recovering` needs the stranded recording's own PD-18
    instance or PD-19 story. PD-19 has three entries: the user's stop took the preserved
    capture (R201.5), found no context and wrote `Idle` while the owner recovered, a start
    took R101.12 in the gap and the auto-stop was refused (`TerminationUnconfirmed`, or a
    dropped start); or the auto-stop ran at status `Idle`, took nothing, and the capture is
    still in the store. There is no exemption for a preserved capture nobody took: the stop
    that could take it is the user's in `Recovering`, which the frontend suppresses
    (`useRecordingState.ts` `handleStop`; `useRecordingState.test.tsx` "issues no stop or
    cancel IPC while the status is recovering"), so the generator binds every user stop to
    its caller (D-S2b-18): a window caller (the main window with the hold and toggle hotkeys
    and the silence auto-stop, or the overlay) sends the stop IPC only when *its own
    window* last received `Recording` (`useRecordingState.ts` `handleStop`,
    `useRecordingControls.ts` click and double-click; on `Starting` a window cancels
    instead, on `Recovering` it only presents a message). Observation and delivery are
    separate (D-S2b-17 (b)): the status event is queued when the shell *emits* it
    (`EmitStatus`, not at the `SetStatus` write, so two rows that write in one order can
    emit in the other and a window can receive `Idle -> Recovering`), and each window has
    its own delivery cursor and its own delivery steps. A stop sent while a window still
    shows `Recording` after the shell entered `Recovering` is a valid schedule, even while
    the other window has already received `Recovering` (a stop already in flight stays a
    flight). A delivery changes no shell state, so it takes no model time and carries no
    actor weight. At a boundary where a command is due and a delivery is ready, the two
    compete: the run's RNG picks which goes first (one draw, only when both are ready), so
    production's both orders are explored (#1159: a stop can reach a window that has just
    received `Recording`, or run before it does). Every generated
    user stop is unscoped (`recording_id = None`), as production sends it; a stale id
    reaches the stop family only through the `Interrupted` auto-stop and the explicit
    decision tests (D-S2b-17 (c)). The registered `stop_native_recording` IPC has no window
    gate and
    the supervisor's `Interrupted` auto-stop bypasses the frontend: both keep every
    schedule (D-S2b-15 (c)); a run in which the unscoped IPC takes the retained capture ends
    in `Idle`, and one in which nothing does is PD-19's third entry (ordinary controls leave
    the app stuck; only the unscoped IPC recovers it, D-S2b-15 (d)). Only
    the recording whose attempt awaits a stop or a cancel may stay `Recording` (L4), and
    only a dropped start's own recording may stay `Starting` or `Recording`.
  There is no "known findings" set: a behaviour outside the allowlist fails
  `seeds_hold_the_properties`.
* **Layer** (D-S2b-4, D-S2b-7 (c)): the harness runs the real `decide_*` functions over
  a model world, not the shell. Where a model witness and a production-reachable shell
  fixture disagree, the fixture wins and the model is fixed. PD-19..PD-24 were found
  this way (`src/commands/recording_harness_findings_tests.rs`, pinned seeds in
  `FINDING_SEEDS`, each beside its shell test). The shell fixtures drive the Stage 0b
  fake host, because the real supervisor has no test seam outside `audio_lifecycle.rs`
  (D-S2b-9 (b)): each injected notification or reply cites its production sender and
  state, uses the production strings, and the test asserts the exact replies, statuses
  and ledger outcomes.
* **Seeds**: 10,000 in CI (about a second and a half, budget 20 s), fixed whatever the
  environment says: `0..9,991` and nine fixed seeds (`FIXED_SEEDS`: 14,033, 81,584, 492,192,
  2,071,065, 289,110, 31,086, 154,052, 24,199, 73,499) that take the PD-18 (unsealed stop),
  PD-19 (dropped start, `TerminationUnconfirmed`, refused auto-stop), PD-21 (short stop),
  PD-22 (both rows), PD-24 (an environmental recovery) and the late main-window start reply
  with its second stop (#1162) entries no seed of the plain range takes, so the converse
  check (every PD and every signature row is witnessed) has no pinned exception. The same
  check requires the gated seeds to decide every row of the Smart Auto round (R101.2, R101.14)
  and of the legacy import (R301.1-R301.6, R302.1, R302.2), to answer the R101.13 and R301.3
  observation rounds, and to have a start reply put `Recording` over the `Processing` of a stop
  in flight, so a `handle` that stops asking for one of those, or a model that stops writing
  that reply, fails the gate. The deeper
  sweep is a separate ignored test, `cargo test -p murmur-dictation-core --lib --release --
  --ignored the_deep_sweep --nocapture` (300,000 seeds; `DEEP_SWEEP_SEEDS=3000000` sets
  another size). `TRACE_SEED=<n> cargo test -p
  murmur-dictation-core --lib property::tests::trace_one -- --nocapture` prints one run.
  `DUMP_SP4B=1` prints the context of every split no signature matches. A core row
  change that is meant to differ from C3 edits `c3_fixture.txt` by hand, after the
  appendix.

What this layer does not cover: SP5 (structural), the shell's own locks and executor
(the golden harness and the shell tests drive those through the 0a seams), and
`FieldAdapter`. SP1 is checked on the model ledger (first claim wins by construction).
A duplicate or late token delivery runs the site's decision in the later world (SP7:
the same row for a retired token as for an outstanding one) and may retire nothing; its
own effects do not run, because in the shell a token has one consumer, the flight that
registered it.
