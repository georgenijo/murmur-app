# murmur-dictation-core

The pure dictation decision vocabulary and admission policy. Stage 1a of the
recording refactor: the crate holds the types and `admit()`; nothing in
`ui_lib` calls it yet (Stage 1b replaces the admission predicates at their
existing call sites; Stage 2a adds the `decide_*` functions).

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
| `effects` | `Placement` (`Pre`/`In`/`PostDT`/`PostT`), `Placed`, the `Effect` enum (one variant per legend code), `Cond`, and `validate_effects` (placements non-decreasing, `ST` only `In`, `EmitOk` pairs an earlier emit). `HIST` is deliberately not an effect. | C0.1, C0.2 |
| `decision` | `Decision`, `Verdict`, `Refusal`, `StaleKind`, `InvalidKind`, `ObsKind` (`NeedObservation`), `ContKind`, `ContinuationSpec` / `Continuation`, `OpStatus`, `RowId`, `SiteId` with the shadow `site_id` encoding (202n = 2021, 204a = 2041, 204b = 2042), and `Decision::validate`. | C2.1, C2.2, C0.3, C1.4.1 |
| `inputs` | One input struct per decide function (C2.3 table, C2.4 sketches), the `Input` enum (one variant per site) and `StartInput::pre_verdict`. | C2.3, C2.4, C3 |
| `replies` | `Reply`, `ReplyType` (the literal `type` strings), `ErrText`, `ProcessAudioRefusal`. No serde: the shell maps replies to JSON. | C2.1, C3 "Reply" columns |
| `admission` | `Request` (16 per-phase variants), `Occupancy`, `Admission`, `RefusalCode` (today's literals), `admit()`, and `admit_traced()` / `check_order()` which expose the evaluated checks. | C6.1, C6.1a, C6.2, C6.3, C6.4 |

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
  `try_lock`s), `model_matches` and `model_prep_variant`.
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
