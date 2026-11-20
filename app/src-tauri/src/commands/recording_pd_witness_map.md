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
| PD-4 | C8 ready/context rendezvous (R207.3): a repeat `Ready` still starts a second ticker (the `Superseded` terminal it also claimed is fixed with PD-23) | `s10-ready-vs-context.golden` (10c, known-bad) | golden |
| PD-5 | Stale-branch terminals for the current id (R207.2, R209.2) | `pd05_supervisor_events_for_the_current_id_claim_superseded` | Stage 1 test |
| PD-6 | `IdleGuard` writes Idle on an id match, whatever the status (R204a.2-.3, R204b.2, R203.x, R102.3) | `s05-timeout-teardown.golden`, `s09-supervisor-idle-during-processing.golden`, `pd06_pipeline_guard_rewrites_idle_while_the_owner_is_stopping` | golden (a, over Idle) + Stage 1 test (b, the pipeline's real guard writing Idle again after a user cancel of Processing, with the owner still `Stopping`; its `Recovering` variant, R204a.3, is covered by the unit test only: the PD-18 / PD-19 fix removed the `Recovering` that R101.12 used to publish) |
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

## E8 enumerates every preserved defect (D-S2c-15 C4)

`every_family_pd_is_witnessed_by_the_shadow` (in `recording_pd_witness_tests.rs`, `dictation-shadow` feature) holds one row per unfixed PD, PD-1 to PD-25 less the fixed ones (`FIXED_PDS`: PD-18, PD-19, PD-20, PD-21, PD-23, PD-24, PD-25) (`e8_pds`), each with the named shell fixtures that assert it: the table above for PD-1 to PD-17, and the section below for PD-22 (PD-20, PD-21 and PD-23 are fixed and have no row). It replays every fixture through the shadow in sync mode (each must close clean, E7) and fails on a PD with no row, on a PD the shadow's allowlist (`pd::PD_DEFS`) credits that E8 does not list as a row signature, on a row-signature PD the shadow never credited, on a credit for a PD that is not a row signature, and on a fixture that is not a `#[test]` Cargo runs. PD-11 and PD-14 are properties of an effect's implementation (their own tests, not replayed here), PD-17 is latent (no fixture), and PD-22 is a cross-ledger split with no row signature. **PD-18 and PD-19, PD-20 (see below), PD-21 (see "Fixed" below), PD-23, PD-24 and PD-25 are fixed**; **PD-23** (a repeat `Ready` for a live recording claims no terminal, see its bullet below, and it retires PD-4's terminal half under D-PD-1); PD-25 specifically (issue #1157): a stale scoped stop whose take finds nothing claims no terminal, so the original stop's R204b.6 keeps generation 1's terminal. It has no row, and its two fixtures in `recording_pd25_tests.rs` now assert the fixed behaviour as ordinary tests (the terminal and the performance outcome both come from the original stop). **PD-24 is fixed**: the supervisor's `Recovering` arm out of `Starting` completes performance only for a run that exists, so a start whose 2 s receiver timeout had already aborted the run leaves no `Cancelled` performance claim next to its `CaptureInitFailure` terminal. It has no row, and its two fixtures in `recording_harness_findings_tests.rs` now assert the fixed behaviour as ordinary tests. **PD-20 is fixed**: the cancel of a `Processing` recording (R205.5) completes performance as the second half of its terminal claim (`PerfComplete::follows_terminal`: one step, and only while the claim stands), so a stop's or the pipeline's completion inside the cancel's tail decides nothing, and a claim lost to the recording's own end completes nothing. It has no row, and its two fixtures in `recording_harness_findings_tests.rs` (`pd20_a_cancel_of_a_processing_recording_owns_both_ledgers_against_a_short_stop`, `pd20_a_cancel_after_the_stops_terminal_leaves_the_stops_performance_outcome`) assert the fixed behaviour as ordinary tests.

## Defects found by the property harness (D-S2b-5)

Shell characterization witnesses in `recording_harness_findings_tests.rs` (not in the table above:
`pd_witness_map_is_complete_and_names_real_tests` reads this section for them and requires every named
`pd*_` test to exist there, and `every_finding_seed_names_a_real_shell_test` requires every `shell_test`
of `FINDING_SEEDS` to exist and to be named here). The harness allowlists each exact signature and pins
its first seed (`FINDING_SEEDS` in the core crate's `property/tests.rs`); the PD text is in the errata's
"Defects found during the refactor".

- **PD-22** (seed 480,584, R211.2 / R203.2, re-pinned from 31,086 by the PD-20 fix, which makes that variant rarer; seed 154,052, R211.2 / R205.5): `pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance` and `pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance` (the stop's 12 s receive expired, so the worker exit's `Idle` is the only reachable one: a sealed stop notifies `CleanupFinished`, never `Idle`). The window is one cancel's tail; its events land in either order.
- **PD-23** (fixed, issue #1157; formerly seeds 450, R207.3 / R203.5; 1,291, R207.3 / R203.1; 169, R207.3 / R205.5; pending source-only `Completed` variant, seed 757 then, R207.3 / R204b.6): a repeat `Ready` for the live recording (R207.3) claims no terminal, so the stop, cancel or pipeline that ends the recording owns the terminal. It has no row, no allowlist signature and no E8 entry (`FIXED_PDS`). Its three fixtures keep their driving sequences and assert the fixed behaviour as ordinary tests in `recording_harness_findings_tests.rs`: `pd23_a_repeat_ready_claims_no_terminal_and_an_empty_stop_completes_no_speech`, `pd23_a_repeat_ready_claims_no_terminal_and_a_refused_stop_completes_failed` (in both the supervisor's `Recovering` arm reaches the current id at `Processing` first, R209.2, so the `Superseded` terminal that remains is PD-5's) and `pd23_a_repeat_ready_claims_no_terminal_and_a_cancel_of_processing_owns_both_ledgers` (the cancel's `UserCancelledProcessing` terminal and its `Cancelled` outcome agree). The `Completed` variant (a second ticker gated off) is covered by the same fix: it needs R207.3's claim, which no longer exists. PD-4 keeps its second ticker (`SD` still attempts it, golden 10c); its terminal half is gone with this fix.

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
- PD-6 (b) and PD-13: the fake pipeline's real guard now writes Idle again (`pd06_*`; it overwrote a `Recovering` before the PD-18 / PD-19 fix) and restores Idle on a pipeline panic (`pd13b_*`) with no terminal, so both go through the pipeline instead of `IdleGuard` called directly.
- PD-6 (b) schedule: no supervisor `Idle` is sent for the sealed attempt (production's `finish_attempt` does not send one after a delivered recording, and the merged harness ends the attempt on it). A user cancel of Processing takes the status to Idle while the sealed stop's `clear_owner` is still pending, which is the barrier before owner clearance.

Remaining:

2. **PD-11:** A is the caller paused between the function's two steps (the test takes only A's telemetry claim); B is the production `emit_dictation_terminal`, so making the function atomic fails the witness. The async persist is not observed; no port sits between the two steps to hold.
3. **PD-14:** the ticker task needs `AppHandle<Wry>`, which no port reaches. The loop was extracted into `run_partial_ticker_loop` (behaviour-neutral); the test pins the loop's structure, not the Wry task. A guard added at the call site in `spawn_dictation_partial_ticker` (outside the loop) would not be caught; a comment there says so.
4. **C2.2 `take_attempts_iff_recovering`:** the stop also takes the interrupted capture before D on the stale-expected-id path (PD-15), not only when status is `Recovering`.
5. **`cancel_window_preserved`:** the window between the status write and `MC` has no port; only the ordering at the first port after the tail is observable (the trace's `~ cancelled_through` and `~ context` lines now show it in the goldens).
6. **PD-10 eviction:** exercised through `emit_dictation_terminal` after 129 acceptances; id 1 is evicted, ids 2 and 129 are kept.

## Fixed

- **PD-18 and PD-19** (the permanent `Recovering`; fixed by making R101.12 a pure refusal). A start that finds the retiring capture's cleanup pending is answered `audio_recovering` and writes nothing: the status stays `Idle`, nothing is published, and the next start after the owner clears is accepted. The old witnesses are now fixed-behaviour tests with the same driving sequences: golden `s04-interruption-under-8000.golden` (4d, PD-18 (a)); `a_start_in_the_teardown_tail_is_refused_and_the_app_stays_idle` (PD-18 (b), (c) and the sealed-stop control); and, in `recording_harness_findings_tests.rs`, `a_start_after_the_stop_took_a_retained_capture_is_refused_and_the_app_stays_idle`, `a_start_after_a_dropped_start_and_a_retained_runtime_failure_is_refused_and_the_app_stays_idle` and `a_start_after_a_refused_auto_stop_is_refused_and_the_app_stays_idle` (PD-19's three entries). The property harness has no PD-18 / PD-19 signature; a run that ends in `Recovering` is a liveness failure, and `refused_in_teardown_gap` counts the schedules the fix refuses.
- **PD-21** (a failed or timed-out capture cancel, R206.2, claimed `PipelineFailure/RuntimeFailure` for a recording that goes on; fixed by making R206.2 claim no terminal). The cancel (or the stop that cancels a `Starting` recording) still answers the failed call's error, and the recording's own path owns the terminal and performance: a stop that took the capture claims its own (`NoSpeech/EmptyAudio` with performance `NoSpeech`), unless the supervisor's `Recovering{RuntimeFailure}` at `Processing` got there first (R209.2, PD-5: `Superseded/StaleOwner`, which seed 289,110 takes now), and the supervisor's late `Recovering{User}` claims `UserCancelledRecording/CancelledRecording` with performance `Cancelled`. The old witnesses are now fixed-behaviour tests with the same driving sequences, in `recording_harness_findings_tests.rs`: `a_timed_out_cancel_claims_nothing_before_a_short_stop` (old seed 289,110, R206.2 / R203.4) and `a_timed_out_cancel_claims_nothing_before_the_late_recovery` (old seed 3, R206.2 / R209.4); the cancel and stop command tests assert that no terminal and no site-206 claim is recorded. The property harness has no PD-21 signature; `FAILED_CANCEL_SEEDS` pins the two shapes and `a_failed_capture_cancel_claims_no_terminal` asserts R206.2 never claims.
- **PD-24** (the start call's 2 s receiver timeout, R102.3, claimed `CaptureInitFailure/RuntimeFailure` while the supervisor's `Recovering` arm, R209.3 / R209.5, completed performance `Cancelled` for the run the failed start had aborted; fixed by `PerfComplete::only_if_run`, which the arm sets out of `Starting`: the coordinator answers `NoRun` and claims nothing for an id with no run). The reply, the status (`Idle`) and the terminal are unchanged. Fixtures in `recording_harness_findings_tests.rs`: `pd24_start_timeout_leaves_one_terminal_and_no_performance_claim_in_the_users_cancel_arm` and `pd24_start_timeout_leaves_one_terminal_and_no_performance_claim_in_an_environmental_recovery_arm`; model seeds 3,651 and 24,199 (`FIXED_START_TIMEOUT_SEEDS`).
