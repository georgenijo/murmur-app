# murmur-dictation-core

The pure dictation decision vocabulary and admission policy. Stage 1a of the
recording refactor: the crate holds the types and `admit()`. Stage 1b
(`ui_lib`'s `dictation_admission` module) routes every admission predicate
through `admit()` / `admit_after()` at its existing call site; Stage 2a adds the
`decide_*` functions.

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
| `inputs` | One input struct per decide function (C2.3 table, C2.4 sketches), the `Input` enum (every one of the 24 sites is reachable, incl. 216 via `SupervisorEvent::InterruptedShortIdle`) and `StartInput::pre_verdict`. | C2.3, C2.4, C3 |
| `replies` | `Reply`, `ReplyType` (the literal `type` strings), `ErrText`, `ProcessAudioRefusal`. No serde: the shell maps replies to JSON. | C2.1, C3 "Reply" columns |
| `rows` | `C3_ROWS`: all 140 C3 row ids with their owning site (`site_of_row`, `rows_of_site`); the row-level half of the Stage 1 "C3 complete" exit. | C3.1-C3.7 |
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

## Stage 1 exit artifacts

### C3 completeness, per site and per row

"C3 complete for every site" (plan §6 Stage 1) is checked by two crate tests that
meet at the site id:

- `inputs::tests::every_decision_site_has_an_input`: each of the 24 `SiteId`s is
  reached by exactly one `Input` variant (`inputs::every_input()` builds one of each).
- `rows::tests`: `C3_ROWS` holds the 140 row ids of the C3 tables (C3.1-C3.7) in
  appendix order; ids are unique, each id's `R<site>.<n>` prefix is its site's
  code (202 is split: R202.1-.2 are `StopContextNoLock`, R202.3-.5 `StopContext`),
  every site owns at least one row, and the per-site counts equal the counts in
  the appendix tables (`ROWS_PER_SITE`).

Rows per site: 101: 15, 102: 6, 103: 7, 104: 7, 201: 8, 202: 3, 202n: 2, 203: 21,
204a: 10, 204b: 10, 205: 5, 206: 2, 207: 5, 208: 2, 209: 7, 210: 4, 211: 6, 212: 1,
213: 2, 214: 2, 215: 3, 216: 5, 301: 5, 302: 2 (total 140). What the tests do **not**
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
