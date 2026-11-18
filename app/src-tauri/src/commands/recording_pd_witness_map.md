# Preserved-defect witness map (Stage 1 exit, C7.0 / C9.4)

Every PD in the C7.1 allowlist maps to a Stage 0b golden scenario or to a
Stage 1 shell-level characterization test. `pd_witness_map_is_complete_and_names_real_tests`
(in `recording_pd_witness_tests.rs`) parses the table below: each `PD-n` row must name a
witness that exists (a `*.golden` file under `recording_golden/`, or a test function in
`recording_golden_tests.rs`, `recording_pd_witness_tests.rs` or `recording_contract_tests.rs`).

Tests drive today's commands through the 0a `DictationHost` seam and the 0b harness
(`recording_golden_harness.rs`), by causal barriers only (hand-polled futures, gates,
supervisor hold points, hooks), with no clock and no sleep. They assert what the code does
today, which is the defect. A fix PR deletes the PD row and its witness together (C7.0).

| PD | Defect (C7.1 row) | Witness | Kind |
|---|---|---|---|
| PD-1 | S12 unguarded Interrupted → Idle (R215.3, R216.1-.5) | `s04-interruption-under-8000.golden` (4b) | golden |
| PD-2 | `Idle` on a finished current id (R211.2) | `s11-idle-on-finished-recording.golden` | golden |
| PD-3 | Unconditional idle emits (S3 R102.2-.6; S7 R202.3-.4) | `s12-start-failure-vs-initialization-failed.golden`, `pd03_stop_missing_context_runs_unconditional_idle_effects` | golden (S3) + Stage 1 test (S7) |
| PD-4 | C8 ready/context rendezvous (R207.3) | `s10-ready-vs-context.golden` (10c, known-bad) | golden |
| PD-5 | Stale-branch terminals for the current id (R207.2, R209.2) | `pd05_supervisor_events_for_the_current_id_claim_superseded` | Stage 1 test |
| PD-6 | `IdleGuard` writes Idle on an id match, whatever the status (R204a.2-.3, R204b.2, R203.x, R102.3) | `s05-timeout-teardown.golden`, `s09-supervisor-idle-during-processing.golden`, `pd06_pipeline_guard_overwrites_recovering` | golden (a, over Idle) + Stage 1 test (b, the pipeline's real guard over Recovering; the status reaches Idle by a user cancel of Processing, with the owner still `Stopping`) |
| PD-7 | Supervisor `Idle` during Processing (R211.3) | `s09-supervisor-idle-during-processing.golden`, `s05-timeout-teardown.golden` | golden |
| PD-8 | Context left installed after `Flush{install:true}` (R103.2) | `pd08_flushing_start_leaves_its_context_installed` | Stage 1 test |
| PD-9 | Perf begin ledger orphan (C4.6, R101.15) | `pd09_dropped_start_orphans_the_begin_ledger_row` | Stage 1 test |
| PD-10 | Terminal before `accepted()` dropped; eviction after 128 (`TERM`) | `pd10_terminal_before_accepted_is_dropped_and_ledger_evicts`, `terminal_dropped_before_accepted` | Stage 1 test |
| PD-11 | Claim and `complete` not atomic (`TERM`) | `pd11_terminal_claim_and_diagnostics_complete_are_not_atomic` | Stage 1 test |
| PD-12 | Stop emits a terminal for a stale expected id; cancel does not (R201.1 vs R205.1) | `s01-stop-during-start.golden` (1c), `s06-stale-stop.golden` | golden |
| PD-13 | Pipeline / stop task panic emits no terminal (R204a.6) | `pd13_task_panic_restores_idle_without_a_terminal` | Stage 1 test (stop path and the pipeline's real guard) |
| PD-14 | Preview ticker task panic: no hide (`PH` mechanism A) | `pd14_ticker_task_panic_never_runs_the_hide` | Stage 1 test (through the `run_partial_ticker_loop` seam: it covers the loop only; an unwind guard added in `spawn_dictation_partial_ticker`, outside the loop, would not be caught) |
| PD-15 | Destructive `take_interrupted_dictation` (R201.6-.7, R215.1, R216.1) | `s03-interruption-8000-or-more.golden`, `s04-interruption-under-8000.golden` | golden |
| PD-16 | S9 cancel post-D clears with no recheck (R205.5) | `s02-cancel-in-each-phase.golden` (2d: the sealed stop's CleanupFinished lands in the cancel's tail and idle is published twice) | golden |
| PD-17 | Keyboard `set_processing` is global with no id (`KB`) | latent | none: no production schedule (D3, C10) |
| PD-18 (a) | Start in the S12 tail leaves `Recovering` | `s04-interruption-under-8000.golden` (4d) | golden |
| PD-18 (b) | Start in the Idle-arm tail leaves `Recovering` | `pd18bc_start_in_the_teardown_tail_leaves_recovering` | Stage 1 test (also a 2b property witness) |
| PD-18 (c) | Start after an unsealed delivered stop leaves `Recovering` | `pd18bc_start_in_the_teardown_tail_leaves_recovering` (with a sealed-stop control that restores Idle) | Stage 1 test (also a 2b property witness) |

## Defects found by the property harness (D-S2b-5)

Shell characterization witnesses in `recording_harness_findings_tests.rs` (not in the table above: the
completeness test reads only the Stage 0b sources). The harness allowlists each exact signature and pins
its first seed (`FINDING_SEEDS` in the core crate's `property/tests.rs`); the PD text is in the errata's
"Defects found during the refactor".

- **PD-19** (three entries; seed 97,854: the worker's exit is unconfirmed, `TerminationUnconfirmed`; seed 14,262: another runtime failure whose recording never had a context, a dropped start; seed 2,798,896: the auto-stop ran at status `Idle`, was refused and took nothing): `pd19_termination_unconfirmed_after_retained_pcm_leaves_recovering`, `pd19_a_dropped_start_and_a_retained_runtime_failure_leave_recovering` and `pd19_a_refused_auto_stop_leaves_the_capture_and_recovering`. In the first two the user's stop took the capture; in the third nothing did and it stays in the store. A preserved capture nobody took is not an exemption. Ordinary controls (a start, a cancel, the frontend's stop, which `useRecordingState.ts` `handleStop` suppresses once `Recovering` has been delivered to the window; `useRecordingState.test.tsx` "issues no stop or cancel IPC while the status is recovering") leave the app stuck. Only the unscoped `stop_native_recording` IPC, which no control the user has sends, takes the capture, writes `Idle` and answers `Missing dictation context for recording 1` (the third fixture asserts it). It stays a defect (D-S2b-15 (d)).
- **PD-20** (seed 305, R205.5 / R203.5): `pd20_cancel_terminal_and_a_short_stop_performance_outcome_disagree`. Its setup is the preview handoff's: a handoff resets the retained PCM before it publishes `Ready` (`audio_lifecycle.rs`, ~1719-1820), so the stop's capture is empty (R203.5).
- **PD-21** (seed 10,843, R206.2 / R203.4; seed 243, R206.2 / R209.4): `pd21_timed_out_cancel_then_a_short_stop_disagree` and `pd21_timed_out_cancel_and_the_late_recovery_disagree` (the cancel's receive expires with the request still queued).
- **PD-22** (seed 93,074, R211.2 / R203.2; seed 382, R211.2 / R205.5): `pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance` and `pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance` (the stop's 12 s receive expired, so the worker exit's `Idle` is the only reachable one: a sealed stop notifies `CleanupFinished`, never `Idle`). The window is one cancel's tail; its events land in either order.
- **PD-23** (seeds 484, R207.3 / R203.5; 66, R207.3 / R203.1; 93, R207.3 / R205.5): `pd23_superseded_terminal_and_an_empty_stop_no_speech_disagree` (the recording starts through the preview handoff, whose reset PCM makes the stop's delivery empty), `pd23_superseded_terminal_and_a_refused_stop_failed_disagree` (the stop's capture stop is refused by the recovering supervisor, `Audio is still recovering from the previous attempt`) and `pd23_superseded_terminal_and_a_cancel_of_processing_cancelled_disagree` (a stop parked in its capture stop at `Processing`, the cancel's R205.5 completes `Cancelled`). PD-4's claim (R207.3 `Superseded` for a live `Recording`, golden 10c) when the recording then ends `NoSpeech`, `Failed` or `Cancelled`, each outcome bound to its rows. **Pending, source-only** (seed 244, R207.3 / R204b.6, `Completed`): the same claim with the ticker's gate **refusing** the second ticker; the fake host has no running ticker, so the gate never refuses and no shell fixture reaches it. It stays allowlisted; its shell test is not counted as passing.
- **PD-24** (two signatures, one per cause; seeds 8,161, R102.3 / R209.3: the user's `Cancel{User}`; 2,813, R102.3 / R209.5: an environmental recovery, `SystemWake`): `pd24_start_timeout_terminal_and_the_users_cancel_arm_performance_outcome_disagree` and `pd24_start_timeout_terminal_and_an_environmental_recovery_arm_performance_outcome_disagree`. The start call's 2 s receiver expired (`await_start_response`): the caller is revoked, `Cancel{User}` is queued, and the start's failure path (R102.3) claims `CaptureInitFailure` inside the supervisor's `Recovering` arm's tail, whose `PC(Cancelled)` then completes performance. The mismatch is at the performance **coordinator's claim**: the failed start never began a performance run, so no row is behind it. The fixture runs the start on its own thread, parked at the supervisor's capture start, and releases it from the arm's `audio-recovery-started` emit (a causal barrier, no clock).

Each sequence is production-reachable (D-S2b-7 (c)). The fixtures drive the Stage 0b fake host, so every supervisor notification and reply they inject cites the production sender and state (`audio_lifecycle.rs` lines, D-S2b-9 (b)); they assert the exact replies, statuses and ledger outcomes. The other
combinations of the SP4(b) signature table are the same two-ledger race through other rows; the model
witnesses them (`every_preserved_defect_is_witnessed` fails on a signature that never occurs) and no
shell test pins them.

## C4.9 contract tests

Implemented now (`recording_contract_tests.rs`), against today's shell:

| C4.9 test | Test function |
|---|---|
| `terminal_first_wins_under_concurrency` | `terminal_first_wins_under_concurrency` |
| `terminal_dropped_before_accepted` | `terminal_dropped_before_accepted` |
| `start_timeout_races_supervisor_cancel` | `start_timeout_races_supervisor_cancel` |
| `init_failed_terminal_precedes_start_err` (T3) | `init_failed_terminal_precedes_start_err` |
| `perf_ledger_table` (C4.6, every cell) | `perf_ledger_table`, `perf_ledger_early_terminal_is_flushed_or_lost` |
| `cancel_window_preserved` (M3) | `cancel_window_preserved` (ordering observed at the first port after the tail; the window itself has no port) |
| `take_attempts_iff_recovering` (C2.2), stop half | `take_attempts_iff_recovering` |
| `take_attempts_iff_recovering` (C2.2), start half (Stage 2a-4: `resolve_smart_auto` is a host port) | `take_attempts_iff_recovering_start_half` |
| `continuation_stale_uses_site_rule` | `continuation_stale_uses_site_rule` (start continuation and every supervisor arm; the stop's stale rule is golden 6) |

Implemented by Stage 2a PR 0 (`core_history` / `NEXT_OP`), in `recording_contract_tests.rs::core_history_contract`:

| C4.9 test | Test function |
|---|---|
| `continuation_unknown_forged_kind_takes_site_rule` | `continuation_unknown_forged_kind_takes_site_rule` (the shell protocol `CoreHistoryLock::consume` with a stand-in site rule that ignores the op status) |
| `unknown_op_runs_site_stale_rule` | `unknown_op_runs_site_stale_rule` (protocol half, every `ContKind`; the per-row half, "every consuming row yields identical effects for `Outstanding` and `Unknown`", is part of each family PR's row tests, C7.0) |
| `wrong_kind_token_does_not_retire_legit_op` | `wrong_kind_token_does_not_retire_legit_op` |
| `ret_if_matches_is_one_lock` | `ret_if_matches_is_one_lock` (lock-count assertion plus a grep of the source) |
| `outstanding_op_never_evicted` | `outstanding_op_never_evicted` |
| `owner_drop_retires_op` | `owner_drop_retires_op` (dropped future, panic unwind, spawned task that never runs, consumed guard) |
| `history_lock_is_leaf` | `history_lock_is_leaf` (grep plus the `D -> core_history` order and the held-flag assertion) |

Implemented by Stage 2a PR 0 (field adapter, `PersistedCore` commit), in `recording_contract_tests.rs::field_adapter_contract`:

| C4.9 test | Test function |
|---|---|
| `persisted_core_has_only_m1_m2` | `persisted_core_has_only_m1_m2` (shape, `MC`/`CX±` leave the core unchanged, `ST`/`ID+` move it with live) |
| `m3_m4_observed_before_and_after_cutover` | `m3_m4_observed_before_and_after_cutover` (the `project()` view and the persisted-core view hold the same live observations; the cut-over entry itself arrives with Stage 2c) |

The SP5 ratchet and the `PersistedCore::new` caller test are in `dictation_executor/ratchet_tests.rs`.

Implemented by Stage 2a PR 0 (the executor skeleton, driven by fake ports), in `recording_contract_tests.rs::executor_contract`:

| C4.9 test | Test function |
|---|---|
| `executor_phase_order` | `executor_phase_order` |
| `awaited_effect_is_last_in_phase` | `awaited_effect_is_last_in_phase` |
| `awaited_effect_resumes_as_continuation` | `awaited_effect_resumes_as_continuation` |
| `executor_never_locks` | `executor_never_locks` (a port probes D, T and `core_history` at every effect, plus a grep of `executor.rs`) |
| `reentrancy_refused` | `reentrancy_refused` (a listener under an `In` emit re-enters) |
| `reentrancy_refused_before_lock` | `reentrancy_refused_before_lock` (the shell holds D on the same thread; `core_history` acquisitions do not move; the check is the first statement) |
| `panic_partial_application` | `panic_partial_application` |
| `panic_before_st_leaves_core_equal_live` | `panic_before_st_leaves_core_equal_live` |
| `emit_result_gates_pres` | `emit_result_gates_pres` (the executor introduces `PRES` as an effect with its `Cond::EmitOk` gate, so this PR owns it) |

Every other C4.9 test needs something Stage 2a creates (the executor, op ids, `core_history`,
`PersistedCore`, the `PRES` effect, a `resolve_smart_auto` port). Decision D-S1-1 re-allocates each to
the first Stage 2 PR that creates what it tests; that PR cannot merge without the test (a named gate).
`c4_9_every_contract_test_is_implemented_or_owned` (in `recording_pd_witness_tests.rs`) checks that each
of the 26 C4.9 names is implemented above or has an `OWNED` row below with a `2a` owner PR and a reason.
It also fails when a marker symbol of an OWNED test's subject (`OWNED_SUBJECT_MARKERS`, for example
`struct CoreHistory` or `fn run(`) exists in `src/` while the row is still OWNED (decision D-S1-5 b), so
a PR that creates the subject cannot leave its test owed.

| Gate | C4.9 test | Owner PR | Why it cannot exist yet |
|---|---|---|---|

## Discrepancies against C7.1 / C4.9 (recorded as the code does them)

Resolved by the council's Stage 0b harness fixes (the fake pipeline holds the real `IdleGuard`; a sealed stop keeps the owner `Stopping` until `finish_sealed_stop`; 2d records the PD-16 duplicate `idle`):

- PD-16: the duplicate `idle` is witnessed by golden 2d with a realistic event, the sealed stop's `CleanupFinished`. (C7.1 says "a supervisor event"; a supervisor `Idle` still cannot duplicate it, because the bridge's Idle arm rechecks status under D, but `CleanupFinished` can.) The `Interrupted`-based test was dropped.
- PD-6 (b) and PD-13: the fake pipeline's real guard now overwrites `Recovering` (`pd06_*`) and restores Idle on a pipeline panic (`pd13b_*`) with no terminal, so both go through the pipeline instead of `IdleGuard` called directly.
- PD-6 (b) schedule: no supervisor `Idle` is sent for the sealed attempt (production's `finish_attempt` does not send one after a delivered recording, and the merged harness ends the attempt on it). A user cancel of Processing takes the status to Idle while the sealed stop's `clear_owner` is still pending, which is the barrier before owner clearance.

Remaining:

1. **PD-18 (b) and (c):** C7.1 says (c) is property-harness only; the held-gap fake witnesses both at shell level. The fake does not run `finish_attempt`; the hold is the barrier, as in 4d. (c) needs an *unsealed* delivered stop (`stop_unsealed`); a sealed stop, now the harness default, is not an instance, and `pd18c_control_sealed_stop_restores_idle` shows `CleanupFinished` restoring Idle.
2. **PD-11:** A is the caller paused between the function's two steps (the test takes only A's telemetry claim); B is the production `emit_dictation_terminal`, so making the function atomic fails the witness. The async persist is not observed; no port sits between the two steps to hold.
3. **PD-14:** the ticker task needs `AppHandle<Wry>`, which no port reaches. The loop was extracted into `run_partial_ticker_loop` (behaviour-neutral); the test pins the loop's structure, not the Wry task. A guard added at the call site in `spawn_dictation_partial_ticker` (outside the loop) would not be caught; a comment there says so.
4. **C2.2 `take_attempts_iff_recovering`:** the stop also takes the interrupted capture before D on the stale-expected-id path (PD-15), not only when status is `Recovering`.
5. **`cancel_window_preserved`:** the window between the status write and `MC` has no port; only the ordering at the first port after the tail is observable (the trace's `~ cancelled_through` and `~ context` lines now show it in the goldens).
6. **PD-10 eviction:** exercised through `emit_dictation_terminal` after 129 acceptances; id 1 is evicted, ids 2 and 129 are kept.
