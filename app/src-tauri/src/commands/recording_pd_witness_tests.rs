//! Stage 1 shell-level characterization tests for the preserved defects (PDs)
//! no Stage 0b golden witnesses (plan §6 Stage 1 exit, C7.0, C9.4).
//!
//! Each test drives today's commands through the Stage 0a `DictationHost`
//! seam and the Stage 0b harness (`recording_golden_harness.rs`), reproduces
//! one preserved defect from the C7.1 allowlist, and asserts what the code does
//! today, which is the defect. A test that starts failing without a behaviour
//! change on purpose means the defect moved or was fixed; a fix deletes the PD
//! row and its test in the same PR (C7.0).
//!
//! Ordering is by causal barriers only (hand-polled futures, gates, supervisor
//! hold points, hooks that run where production reaches a port). There is no
//! clock and no sleep. `MURMUR_GOLDEN_REPEAT=N` runs each body N times.
//!
//! The PD → test map lives in `recording_pd_witness_map.md` and is checked by
//! `pd_witness_map_is_complete_and_names_real_tests`.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{cancelled, recording, sub};
use super::*;

// -------------------------------------------------------------- helpers

pub(super) fn position(trace: &[String], line: &str) -> usize {
    trace
        .iter()
        .position(|entry| entry == line)
        .unwrap_or_else(|| panic!("`{line}` is missing from the trace:\n{}", trace.join("\n")))
}

pub(super) fn count(trace: &[String], line: &str) -> usize {
    trace.iter().filter(|entry| *entry == line).count()
}

/// Count of `line` strictly after the first `anchor` line.
fn count_after(trace: &[String], anchor: &str, line: &str) -> usize {
    count(&trace[position(trace, anchor) + 1..], line)
}

const IDLE_EMIT: &str = "emit recording-status-changed \"idle\"";
const SET_PROCESSING_FALSE: &str = "keyboard.set_processing false";

/// Run a sub-run once per `MURMUR_GOLDEN_REPEAT` repetition.
pub(super) fn repeat(sub_run: &SubRun) {
    for _ in 0..repeat_count() {
        run_sub_run(sub_run);
    }
}

fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
}

fn start_reply(world: &Arc<World>) -> Result<serde_json::Value, String> {
    let mut start = world.start();
    let reply = complete(&mut start);
    world.reply("start", &reply);
    reply
}

fn stop_reply(world: &Arc<World>, expected: Option<u64>) -> Result<serde_json::Value, String> {
    let mut stop = world.stop(expected);
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
    reply
}

fn audio_recovering(state: &str) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "type": "audio_recovering", "state": state }))
}

/// A dictation in `Processing` with its pipeline parked at the `pipeline`
/// gate: the stop command is the returned, pending future.
fn processing_with_the_pipeline_running(
    world: &Arc<World>,
) -> Fut<Result<serde_json::Value, String>> {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.close_gate("pipeline");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    world.note("the pipeline is running for id 1: status is Processing");
    assert_eq!(status(world), DictationStatus::Processing);
    stop
}

// ------------------------------------------------- PD-3 (S7 half)
// C7.1 PD-3, rows R202.3 / R202.4: the stop's missing-context branch writes the
// status conditionally but runs `set_processing(false)` and the `idle` emit
// unconditionally (recording.rs, the `None` arm after `active_context`).

/// R202.3: the context is gone while status is still Processing.
fn pd03_s7a_missing_context_in_processing(world: &Arc<World>) {
    recording(world);
    let clearer = Arc::clone(world);
    world.on("keyboard.stop_gesture_take_for", move || {
        assert!(clearer.state().app_state.clear_active_context(1));
    });
    let reply = stop_reply(world, None);
    assert_eq!(
        reply,
        Err("Missing dictation context for recording 1".to_string())
    );
    let trace = world.trace();
    let after = "keyboard.stop_gesture_take_for 1";
    assert_eq!(count_after(&trace, after, SET_PROCESSING_FALSE), 1);
    assert_eq!(count_after(&trace, after, IDLE_EMIT), 1);
    assert!(trace.contains(&"~ terminal 1 PipelineFailure/MissingContext".to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
}

/// R202.4: a supervisor `Idle` (PD-7) already restored Idle and emitted
/// `idle`; the stop's missing-context branch emits `idle` and clears the
/// keyboard gate again. Two `idle` emits for one recording, and the stop's own
/// terminal loses to the supervisor's.
fn pd03_s7b_missing_context_after_a_supervisor_idle(world: &Arc<World>) {
    recording(world);
    let supervisor = Arc::clone(world);
    world.on("keyboard.stop_gesture_take_for", move || {
        supervisor.notify(1, AudioLifecycleEvent::Idle)
    });
    let reply = stop_reply(world, None);
    assert_eq!(
        reply,
        Err("Missing dictation context for recording 1".to_string())
    );
    let trace = world.trace();
    assert_eq!(
        count_after(&trace, "supervisor.notify 1 Idle", IDLE_EMIT),
        2
    );
    assert_eq!(
        count_after(&trace, "supervisor.notify 1 Idle", SET_PROCESSING_FALSE),
        1
    );
    assert_eq!(
        world
            .state()
            .app_state
            .dictation_telemetry
            .terminal_for_test(1),
        Some((
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    assert_eq!(status(world), DictationStatus::Idle);
}

#[test]
fn pd03_stop_missing_context_runs_unconditional_idle_effects() {
    repeat(&sub(
        "PD-3 S7 R202.3",
        pd03_s7a_missing_context_in_processing,
    ));
    repeat(&sub(
        "PD-3 S7 R202.4",
        pd03_s7b_missing_context_after_a_supervisor_idle,
    ));
}

// ----------------------------------------------------------------- PD-5
// C7.1 PD-5, rows R207.2 / R209.2: a supervisor event for the current id in a
// status the arm does not expect claims `Superseded/StaleOwner` for a live
// recording, and the stop's later terminal loses.

fn pd05_finish(world: &Arc<World>, mut stop: Fut<Result<serde_json::Value, String>>) {
    assert_eq!(
        world
            .state()
            .app_state
            .dictation_telemetry
            .terminal_for_test(1),
        Some((
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        ))
    );
    assert_eq!(status(world), DictationStatus::Processing);
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
    assert_eq!(
        reply,
        Ok(serde_json::json!({
            "state": "idle", "text": "scripted transcript", "type": "transcription"
        }))
    );
    // The recording succeeded, but the ledger kept the stale-owner terminal.
    let trace = world.trace();
    assert_eq!(
        trace
            .iter()
            .filter(|line| line.starts_with("~ terminal 1 "))
            .count(),
        1
    );
    assert_eq!(
        world
            .state()
            .app_state
            .dictation_telemetry
            .terminal_for_test(1),
        Some((
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        ))
    );
}

/// R209.2: `Recovering{DeviceChanged}` while a stop's pipeline is running.
fn pd05a_recovering_event_while_processing(world: &Arc<World>) {
    let stop = processing_with_the_pipeline_running(world);
    world.notify(
        1,
        AudioLifecycleEvent::Recovering {
            reason: AudioCancelReason::DeviceChanged,
        },
    );
    pd05_finish(world, stop);
}

/// R207.2: a `Ready` for the current id while a stop's pipeline is running.
fn pd05b_ready_event_while_processing(world: &Arc<World>) {
    let stop = processing_with_the_pipeline_running(world);
    world.notify(1, AudioLifecycleEvent::Ready);
    pd05_finish(world, stop);
}

#[test]
fn pd05_supervisor_events_for_the_current_id_claim_superseded() {
    repeat(&sub("PD-5 R209.2", pd05a_recovering_event_while_processing));
    repeat(&sub("PD-5 R207.2", pd05b_ready_event_while_processing));
}

// ----------------------------------------------------------- PD-6 (b)
// C7.1 PD-6 (b), rows R204a.3 / R102.3: a pipeline guard whose id is still
// current overwrites status `Recovering` (published by R101.12 while cleanup
// was pending) with Idle before `CleanupFinished`. The golden scenarios 5 and
// 9 witness (a), the re-write over Idle. The fake pipeline holds the real
// `IdleGuard`, so the guard itself does the overwrite.
//
// The schedule uses only events production can send. A supervisor `Idle` for
// the sealed attempt is not one (`finish_attempt` sends `Idle` only when no
// recording was delivered, and the merged harness ends the attempt on it), so
// the status reaches Idle by a user cancel of Processing instead (golden 2e):
// the cancel resets the UI and leaves the owner `Stopping`, which is the
// barrier. The sealed stop's `clear_owner` has not run when the start looks at
// the owner (`is_dictation_cleanup_pending`), so the start takes R101.12.

fn pd06b_pipeline_guard_overwrites_recovering(world: &Arc<World>) {
    let mut stop = processing_with_the_pipeline_running(world);
    cancelled(world, "cancel");
    assert_eq!(status(world), DictationStatus::Idle);
    let trace = world.trace();
    assert_eq!(
        count(&trace, "supervisor.notify 1 Idle"),
        0,
        "no supervisor Idle: production cannot send one for a sealed stop"
    );
    assert_eq!(
        count(&trace, "supervisor.clear_owner 1"),
        0,
        "the sealed stop's owner clear has not run"
    );
    world.note("the sealed stop's owner is still Stopping: a start takes R101.12");
    assert_eq!(start_reply(world), audio_recovering("recovering"));
    assert_eq!(status(world), DictationStatus::Recovering);
    assert_eq!(
        count(
            &world.trace(),
            "supervisor.is_dictation_cleanup_pending 1 -> true"
        ),
        1,
        "the start took R101.12 because cleanup was pending"
    );
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
    let trace = world.trace();
    assert_eq!(
        count(&trace, "supervisor.notify 1 CleanupFinished"),
        0,
        "the guard's overwrite precedes the sealed stop's CleanupFinished"
    );
    let returned = position(&trace, "pipeline 1 returns");
    let overwritten = returned
        + trace[returned..]
            .iter()
            .position(|line| line == "~ status Recovering -> Idle")
            .expect("PD-6: the pipeline guard wrote Idle over Recovering");
    let guard = position(&trace, "pipeline 1 idle_guard wrote_idle=true");
    assert!(overwritten < guard, "the overwrite is the guard's finish");
    assert_eq!(status(world), DictationStatus::Idle);
}

#[test]
fn pd06_pipeline_guard_overwrites_recovering() {
    repeat(&sub(
        "PD-6 (b) R204a.3",
        pd06b_pipeline_guard_overwrites_recovering,
    ));
}

// ----------------------------------------------------------------- PD-8
// C7.1 PD-8, row R103.2: a start whose supervisor cancel landed first
// (status Recovering) still installs its context in the flush branch, and
// nothing clears it until the next `CX-`.

fn pd08_flush_leaves_the_context_installed(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    world.close_gate("smart_capture");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    world.note("the start is parked in its context resolution; no context is installed");
    world.notify(
        1,
        AudioLifecycleEvent::Recovering {
            reason: AudioCancelReason::User,
        },
    );
    assert_eq!(status(world), DictationStatus::Recovering);
    assert!(world.state().app_state.active_context(1).is_none());
    world.open_gate("smart_capture");
    let reply = complete(&mut start);
    world.reply("start", &reply);
    assert_eq!(reply, audio_recovering("recovering"));
    assert!(
        world.state().app_state.active_context(1).is_some(),
        "PD-8: the flushing start left its context installed"
    );
    world.release_ready();
    assert!(world.state().app_state.active_context(1).is_some());
    assert!(world.state().app_state.clear_active_context(1));
    assert!(world.state().app_state.active_context(1).is_none());
}

#[test]
fn pd08_flushing_start_leaves_its_context_installed() {
    repeat(&sub("PD-8 R103.2", pd08_flush_leaves_the_context_installed));
}

// ----------------------------------------------------------------- PD-9
// C7.1 PD-9 (C4.6): a start future dropped between the begin-ledger
// registration and `spawn_begin` leaves a registered row nobody resolves, so a
// completion requested afterwards is held forever.

fn pd09_start_dropped_after_register(world: &Arc<World>) {
    world.close_gate("smart_capture");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    world.note("the start is parked after begin_register and the supervisor ack");
    assert!(dictation_performance_begins().begin_is_pending(1));
    drop(start);
    world.note("the start future is dropped; nothing aborts or begins the row");
    let state = world.state();
    assert!(state.app_state.recording_transition.try_lock().is_ok());
    assert_eq!(status(world), DictationStatus::Starting);
    assert!(dictation_performance_begins().begin_is_pending(1));
    complete_dictation_performance(
        &state.performance,
        1,
        RunOutcomeV1::Cancelled {
            stage: PerformanceStageV1::CaptureFinalization,
        },
        Vec::new(),
        None,
        None,
    );
    let coordinator = dictation_performance_begins();
    assert!(
        !coordinator.completion_is_claimed(1),
        "PD-9: the completion is held behind a begin that never runs"
    );
    assert!(coordinator.begin_is_pending(1));
}

#[test]
fn pd09_dropped_start_orphans_the_begin_ledger_row() {
    repeat(&sub("PD-9", pd09_start_dropped_after_register));
}

// ---------------------------------------------------------------- PD-10
// C7.1 PD-10 (C4.5 T2): `claim_terminal` is false for an id never accepted,
// and the ledger forgets ids after 128.

fn pd10a_start_refused_before_the_supervisor_accepted(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SpawnFailed(
            "scripted start failure".to_string(),
        ));
        knobs.accept_before_failing = false;
    }
    let reply = start_reply(world);
    assert!(reply.is_err());
    let state = world.state();
    assert_eq!(
        state.app_state.dictation_telemetry.terminal_for_test(1),
        None
    );
    assert!(!world
        .trace()
        .iter()
        .any(|line| line.starts_with("~ terminal 1 ")));
    // The start's own `TERM` was dropped, and any later one is too.
    assert!(!emit_dictation_terminal(
        &state,
        1,
        DictationTerminalOutcome::CaptureInitFailure,
        DictationErrorCode::RuntimeFailure,
    ));
}

fn pd10b_ledger_forgets_ids_after_128(world: &Arc<World>) {
    for recording_id in 1..=129 {
        world.accept_silently(recording_id);
    }
    let state = world.state();
    let terminal = |recording_id| {
        emit_dictation_terminal(
            &state,
            recording_id,
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner,
        )
    };
    assert!(!terminal(1), "id 1 was evicted by the 129th acceptance");
    assert_eq!(
        state.app_state.dictation_telemetry.terminal_for_test(1),
        None
    );
    assert!(terminal(2));
    assert!(terminal(129));
}

#[test]
fn pd10_terminal_before_accepted_is_dropped_and_ledger_evicts() {
    repeat(&sub(
        "PD-10 start refused before accepted",
        pd10a_start_refused_before_the_supervisor_accepted,
    ));
    repeat(&sub(
        "PD-10 eviction after 128",
        pd10b_ledger_forgets_ids_after_128,
    ));
}

// ---------------------------------------------------------------- PD-11
// C7.1 PD-11 (C4.5 T5): `emit_dictation_terminal_with_capture` claims the
// telemetry ledger and then calls `dictation_diagnostics.complete`, two steps
// with nothing between them. A is the caller paused between the two steps: the
// test takes only A's telemetry claim, directly. B is the production
// `emit_dictation_terminal`, so a fix that makes the function atomic (gate
// `complete` on the claim) changes B's behaviour and fails this witness.
// (Its asynchronous persist runs on the blocking pool and is not observed.)

fn pd11_claim_and_complete_are_two_steps(world: &Arc<World>) {
    let state = world.state();
    let directory = tempfile::tempdir().expect("diagnostics directory");
    state
        .dictation_diagnostics
        .initialize(directory.path().join("diagnostics"))
        .expect("diagnostics store");
    state.dictation_diagnostics.arm_next().expect("arm");
    assert!(state.dictation_diagnostics.claim(1));
    world.accept_silently(1);
    let telemetry = &state.app_state.dictation_telemetry;
    // A claims the terminal ledger first.
    assert!(telemetry.emit_terminal_with_output(
        1,
        DictationTerminalOutcome::Success,
        DictationErrorCode::None,
        0
    ));
    // A is paused here, between the function's two steps. B is the production
    // function itself: it loses the telemetry claim and still reaches
    // `complete`, which consumes the one-shot capture.
    world.note(
        "A holds the telemetry claim and has not called complete; B runs the production emitter",
    );
    assert!(
        !emit_dictation_terminal(
            &state,
            1,
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::TranscriptionFailed,
        ),
        "B lost the telemetry claim"
    );
    // A resumes: its `complete` finds nothing left, so A's outcome is never
    // written to diagnostics although A owns the terminal.
    assert!(
        state
            .dictation_diagnostics
            .complete(
                1,
                DictationCaptureCompletion::Terminal {
                    outcome: DictationTerminalOutcome::Success,
                    error_code: DictationErrorCode::None,
                },
            )
            .is_none(),
        "PD-11: B, which lost the claim, used up the capture"
    );
    // The ledger keeps A's outcome; diagnostics took B's.
    assert_eq!(
        telemetry.terminal_for_test(1),
        Some((DictationTerminalOutcome::Success, DictationErrorCode::None))
    );
}

#[test]
fn pd11_terminal_claim_and_diagnostics_complete_are_not_atomic() {
    repeat(&sub("PD-11", pd11_claim_and_complete_are_two_steps));
}

// ---------------------------------------------------------------- PD-13
// C7.1 PD-13, row R204a.6: a panic in the stop task (or the pipeline's
// `IdleGuard`) restores Idle on unwind and emits no terminal.

/// A panic in the stop command after its `IdleGuard` is armed: the unwind
/// writes Idle and clears the keyboard gate, emits no `idle`, and leaves the
/// ledger entry unclaimed.
fn pd13a_panic_in_the_stop_path(world: &Arc<World>) {
    recording(world);
    world.on("supervisor.stop", || panic!("scripted stop-task panic"));
    let mut stop = world.stop(None);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = poll_once(&mut stop);
    }));
    assert!(
        unwound.is_err(),
        "the scripted panic unwinds out of the stop"
    );
    drop(stop);
    world.note("the stop task panicked; its guard dropped during the unwind");
    let state = world.state();
    assert_eq!(status(world), DictationStatus::Idle);
    assert!(!keyboard::is_processing());
    let trace = world.trace();
    let processing = position(&trace, "emit recording-status-changed \"processing\"");
    assert!(
        !trace[processing..].contains(&IDLE_EMIT.to_string()),
        "PD-13: the unwind emits no idle, so the UI stays on processing"
    );
    assert!(!trace.iter().any(|line| line.starts_with("~ terminal 1 ")));
    assert_eq!(
        state.app_state.dictation_telemetry.terminal_for_test(1),
        None
    );
    // The attempt was accepted and is still unclaimed: nothing emitted for it.
    assert!(emit_dictation_terminal(
        &state,
        1,
        DictationTerminalOutcome::PipelineFailure,
        DictationErrorCode::RuntimeFailure,
    ));
}

/// A panic inside the pipeline, after the real `IdleGuard` the fake pipeline
/// builds first: the guard's drop writes Idle and clears the gate, and no
/// terminal is emitted for the id (R204a.6).
fn pd13b_panic_in_the_pipeline(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.on("pipeline", || panic!("scripted pipeline panic"));
    let mut stop = world.stop(None);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = poll_once(&mut stop);
    }));
    assert!(
        unwound.is_err(),
        "the scripted panic unwinds out of the stop"
    );
    drop(stop);
    world.note("the pipeline panicked; its IdleGuard dropped during the unwind");
    let state = world.state();
    assert_eq!(status(world), DictationStatus::Idle);
    assert!(!keyboard::is_processing());
    let trace = world.trace();
    let processing = position(&trace, "emit recording-status-changed \"processing\"");
    assert!(
        !trace[processing..].contains(&IDLE_EMIT.to_string()),
        "PD-13: the unwind emits no idle"
    );
    assert!(!trace.iter().any(|line| line.starts_with("~ terminal 1 ")));
    assert!(emit_dictation_terminal(
        &state,
        1,
        DictationTerminalOutcome::PipelineFailure,
        DictationErrorCode::RuntimeFailure,
    ));
}

#[test]
fn pd13_task_panic_restores_idle_without_a_terminal() {
    repeat(&sub("PD-13 stop task panic", pd13a_panic_in_the_stop_path));
    repeat(&sub("PD-13 pipeline panic", pd13b_panic_in_the_pipeline));
}

// ---------------------------------------------------------------- PD-14
// C7.1 PD-14: `hide_on_ticker_exit` runs after the ticker loop, so a panic in
// the loop never hides the preview pill. The ticker task needs a concrete
// `AppHandle<Wry>`, which no port reaches, so this witnesses the loop through
// the behaviour-neutral `run_partial_ticker_loop` seam (Stage 1, called out in
// the PR). Time is tokio's paused clock: nothing waits on a real timer.

fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("paused-time runtime")
}

#[test]
fn pd14_ticker_task_panic_never_runs_the_hide() {
    use std::cell::Cell;
    for _ in 0..repeat_count() {
        // Control: a loop that ends by itself hides exactly once.
        let (ticks, exits) = (Cell::new(0), Cell::new(0));
        let polls = Cell::new(0);
        paused_runtime().block_on(run_partial_ticker_loop(
            std::time::Duration::from_millis(700),
            || {
                polls.set(polls.get() + 1);
                polls.get() < 3
            },
            || ticks.set(ticks.get() + 1),
            || exits.set(exits.get() + 1),
        ));
        assert_eq!((ticks.get(), exits.get()), (2, 1));

        // PD-14: a panic inside the loop skips the hide.
        let exits = Cell::new(0);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            paused_runtime().block_on(run_partial_ticker_loop(
                std::time::Duration::from_millis(700),
                || true,
                || panic!("scripted ticker panic"),
                || exits.set(exits.get() + 1),
            ));
        }));
        assert!(unwound.is_err());
        assert_eq!(exits.get(), 0, "PD-14: no hide after a ticker panic");
    }
}

// --------------------------------------------------------------- PD-18
// C7.1 PD-18 (b) and (c): a start that takes R101.12 in the gap between the
// attempt's last notification and `clear_owner` (audio_lifecycle.rs:2460)
// leaves status `Recovering` with nothing to restore Idle. Golden 4d witnesses
// (a). The plan allows (b) and (c) to be property witnesses at 2b; the fake
// supervisor can hold the gap open, so they are also witnessed here.

/// After the owner is cleared: nothing restores Idle, and no other actor can.
fn pd18_nothing_restores_idle(world: &Arc<World>, held_at: &str) {
    assert_eq!(status(world), DictationStatus::Recovering);
    assert_eq!(
        world.state().app_state.recording_id.load(Ordering::SeqCst),
        1
    );
    assert_eq!(stop_reply(world, None), audio_recovering("recovering"));
    cancelled(world, "cancel");
    assert_eq!(start_reply(world), audio_recovering("recovering"));
    assert_eq!(status(world), DictationStatus::Recovering);
    let trace = world.trace();
    let held = position(&trace, held_at);
    assert!(
        !trace[held..]
            .iter()
            .any(|line| line.starts_with("supervisor.notify 1 ")),
        "no notification follows the gap: {trace:#?}"
    );
}

/// (b) Idle-arm tail: `Idle` is notified while the public phase is `Stopping`
/// (an undelivered, timed-out stop); the supervisor then reaches its gap.
fn pd18b_start_in_the_idle_arm_tail(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Err("stop timed out");
    let idle = Arc::clone(world);
    world.on("supervisor.stop", move || {
        idle.notify(1, AudioLifecycleEvent::Idle)
    });
    let reply = stop_reply(world, None);
    assert_eq!(reply, Err("stop timed out".to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
    world.hold_owner_clear(1);
    assert_eq!(start_reply(world), audio_recovering("recovering"));
    assert_eq!(status(world), DictationStatus::Recovering);
    world.release_owner_clear();
    pd18_nothing_restores_idle(world, "supervisor.hold_before_owner_clear 1");
}

/// (c) Unsealed delivered stop: the stop reply is delivered without a sealed
/// `CleanupFinished`, the short-audio path writes Idle, and the supervisor has
/// not cleared the owner.
fn pd18c_start_after_an_unsealed_delivered_stop(world: &Arc<World>) {
    recording(world);
    {
        let mut knobs = world.knobs();
        knobs.stop_result = Ok(0);
        knobs.stop_unsealed = true;
    }
    assert_eq!(
        stop_reply(world, None),
        Ok(serde_json::json!({ "state": "idle", "text": "", "type": "transcription" }))
    );
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(start_reply(world), audio_recovering("recovering"));
    assert_eq!(status(world), DictationStatus::Recovering);
    world.release_owner_clear();
    pd18_nothing_restores_idle(world, "supervisor.hold_before_owner_clear 1");
}

/// Control: a sealed stop is not an instance. A start taken while the sealed
/// stop's owner is still `Stopping` also reads cleanup pending, but the
/// supervisor's `CleanupFinished` after `clear_owner` restores Idle.
fn pd18c_control_sealed_stop_restores_idle(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(0);
    assert!(stop_reply(world, None).is_ok());
    assert_eq!(start_reply(world), audio_recovering("recovering"));
    assert_eq!(status(world), DictationStatus::Recovering);
    world.note("the supervisor finishes the sealed stop: clear_owner, then CleanupFinished");
    world.finish_sealed_stop(1);
    assert_eq!(status(world), DictationStatus::Idle);
}

#[test]
fn pd18bc_start_in_the_teardown_tail_leaves_recovering() {
    repeat(&sub(
        "PD-18 control: sealed stop",
        pd18c_control_sealed_stop_restores_idle,
    ));
    repeat(&sub(
        "PD-18 (b) Idle-arm tail",
        pd18b_start_in_the_idle_arm_tail,
    ));
    repeat(&sub(
        "PD-18 (c) unsealed delivered stop",
        pd18c_start_after_an_unsealed_delivered_stop,
    ));
}

// ----------------------------------------------------------------- map

/// `recording_pd_witness_map.md` lists every PD, names a witness for each, and
/// every named test or golden exists.
#[test]
fn pd_witness_map_is_complete_and_names_real_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let map = std::fs::read_to_string(root.join("recording_pd_witness_map.md")).expect("map");
    let sources: String = [
        "recording_pd_witness_tests.rs",
        "recording_contract_tests.rs",
        "recording_golden_tests.rs",
    ]
    .iter()
    .map(|file| std::fs::read_to_string(root.join(file)).expect("test source"))
    .collect();
    let rows: Vec<&str> = map
        .lines()
        .filter(|line| line.starts_with("| PD-"))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for row in &rows {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let id = cells[1].to_string();
        seen.insert(id.clone());
        let witness = cells[3];
        assert!(
            witness == "latent" || witness.contains('`'),
            "{id}: no witness named"
        );
        for name in witness.split('`').skip(1).step_by(2) {
            let exists = if let Some(file) = name.strip_suffix(".golden") {
                root.join("recording_golden")
                    .join(format!("{file}.golden"))
                    .exists()
            } else {
                sources.contains(&format!("fn {name}("))
            };
            assert!(exists, "{id}: witness `{name}` does not exist");
        }
    }
    for number in 1..=17 {
        assert!(
            seen.contains(&format!("PD-{number}")),
            "PD-{number} missing"
        );
    }
    for instance in ["a", "b", "c"] {
        assert!(
            seen.contains(&format!("PD-18 ({instance})")),
            "PD-18 ({instance}) missing"
        );
    }
}

/// Every C4.9 contract test (appendix C4.9, including the v5.1 additions).
const C4_9_TESTS: [&str; 26] = [
    "executor_phase_order",
    "executor_never_locks",
    "reentrancy_refused",
    "continuation_unknown_forged_kind_takes_site_rule",
    "continuation_stale_uses_site_rule",
    "terminal_first_wins_under_concurrency",
    "terminal_dropped_before_accepted",
    "start_timeout_races_supervisor_cancel",
    "init_failed_terminal_precedes_start_err",
    "perf_ledger_table",
    "cancel_window_preserved",
    "panic_partial_application",
    "take_attempts_iff_recovering",
    "panic_before_st_leaves_core_equal_live",
    "reentrancy_refused_before_lock",
    "awaited_effect_is_last_in_phase",
    "awaited_effect_resumes_as_continuation",
    "history_lock_is_leaf",
    "unknown_op_runs_site_stale_rule",
    "outstanding_op_never_evicted",
    "owner_drop_retires_op",
    "emit_result_gates_pres",
    "wrong_kind_token_does_not_retire_legit_op",
    "ret_if_matches_is_one_lock",
    "persisted_core_has_only_m1_m2",
    "m3_m4_observed_before_and_after_cutover",
];

/// Decision D-S1-1: every C4.9 test is implemented here (a test function of
/// that name exists in `recording_contract_tests.rs`) or is listed in the map's
/// owner table with the Stage 2 PR that cannot merge without it and the reason
/// it cannot exist yet. `take_attempts_iff_recovering` was split (stop half, then
/// start half in Stage 2a-4) and is now implemented in full.
///
/// The gate is also **enforced from the code side** (decision D-S1-5 b): each
/// OWNED test has marker symbols of its subject, `(path under src/, text)`. If
/// a marker exists while the row is still OWNED, the subject has landed and the
/// owner PR is shipping without its test, so the test fails. Moving a row out of
/// OWNED means implementing the tests in `recording_contract_tests.rs`. A new
/// OWNED name must register its markers here.
const OWNED_SUBJECT_MARKERS: &[(&str, &[(&str, &str)])] = &[
    // No test is owed any more: the start family's smart-auto port landed in
    // Stage 2a-4 together with `take_attempts_iff_recovering` (start half).
];

#[test]
fn c4_9_every_contract_test_is_implemented_or_owned() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let map = std::fs::read_to_string(root.join("recording_pd_witness_map.md")).expect("map");
    let contract =
        std::fs::read_to_string(root.join("recording_contract_tests.rs")).expect("contract tests");
    let mut owned = std::collections::BTreeMap::new();
    for row in map.lines().filter(|line| line.starts_with("| OWNED ")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let (names, owner, reason) = (cells[2], cells[3], cells[4]);
        assert!(
            owner.starts_with("2a") && !reason.is_empty(),
            "an owned test needs a 2a owner PR and a reason: {row}"
        );
        for name in names.split('`').skip(1).step_by(2) {
            assert!(
                C4_9_TESTS.contains(&name),
                "`{name}` is not a C4.9 test name"
            );
            assert!(
                owned.insert(name.to_string(), owner.to_string()).is_none(),
                "`{name}` is owned twice"
            );
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for name in owned.keys() {
        let markers = OWNED_SUBJECT_MARKERS
            .iter()
            .find(|(owned_name, _)| owned_name == name)
            .unwrap_or_else(|| panic!("OWNED `{name}` has no entry in OWNED_SUBJECT_MARKERS"))
            .1;
        for (path, symbol) in markers {
            let landed = std::fs::read_to_string(src.join(path))
                .map(|text| text.contains(symbol))
                .unwrap_or(false);
            assert!(
                !landed,
                "`{symbol}` exists in src/{path}, so the subject of OWNED `{name}` has landed: \
                 implement the test in recording_contract_tests.rs and drop its OWNED row"
            );
        }
    }
    for name in C4_9_TESTS {
        let implemented = contract.contains(&format!("fn {name}("));
        let owner = owned.get(name);
        assert!(
            implemented || owner.is_some(),
            "C4.9 `{name}` is neither implemented nor owned by a Stage 2 PR"
        );
        if implemented && owner.is_some() {
            assert_eq!(
                name, "take_attempts_iff_recovering",
                "`{name}` is implemented, so its owner row is stale"
            );
        }
    }
}

/// A start future dropped while it is parked on the awaited capture start of a kept
/// microphone test: the R101.15 flight ran `PR` and never reaches `CS`.
#[cfg(feature = "dictation-shadow")]
fn pd09c_start_dropped_at_the_awaited_capture_start(world: &Arc<World>) {
    world
        .state()
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    drop(start);
    world.note("the start future is dropped while it waits for the capture start");
    // The blocking task the future spawned still runs; wait for it by what it does.
    while !world
        .trace()
        .iter()
        .any(|line| line.starts_with("supervisor.start_after_preview 1"))
    {
        std::thread::yield_now();
    }
}

/// E8 (appendix C8.5): every preserved defect whose row is in a family is witnessed by
/// the shadow. The twelve goldens and the Stage 1 PD witness tests run through the
/// shadow in this process (every sub-run is replayed, `run_sub_run_observed`), and each
/// PD the allowlist credits is counted. PD-11, PD-14 and PD-17 are not row signatures
/// (properties of an effect's implementation, or latent) and are not counted.
#[cfg(feature = "dictation-shadow")]
#[test]
fn every_family_pd_is_witnessed_by_the_shadow() {
    use super::golden_tests as g;
    use crate::dictation_shadow::replay;
    replay::reset_totals();
    g::golden_s01_stop_during_start();
    g::golden_s02_cancel_in_each_phase();
    g::golden_s03_interruption_of_8000_samples_or_more();
    g::golden_s04_interruption_under_8000_samples();
    g::golden_s05_timeout_teardown();
    g::golden_s06_stale_stop();
    g::golden_s07_meeting_conflict();
    g::golden_s08_preview_handoff();
    g::golden_s09_supervisor_idle_during_processing();
    g::golden_s10_ready_vs_context_rendezvous();
    g::golden_s11_idle_on_a_finished_recording();
    g::golden_s12_start_failure_vs_initialization_failed();
    pd03_stop_missing_context_runs_unconditional_idle_effects();
    pd05_supervisor_events_for_the_current_id_claim_superseded();
    pd06_pipeline_guard_overwrites_recovering();
    pd08_flushing_start_leaves_its_context_installed();
    pd09_dropped_start_orphans_the_begin_ledger_row();
    pd10_terminal_before_accepted_is_dropped_and_ledger_evicts();
    pd13_task_panic_restores_idle_without_a_terminal();
    pd18bc_start_in_the_teardown_tail_leaves_recovering();
    // PD-9's signature in the allowlist is an R101.15 flight abandoned between `PR` and
    // `CS`: a start future dropped while it waits for a kept preview's capture start.
    repeat(&sub(
        "PD-9 abandoned at the awaited capture start",
        pd09c_start_dropped_at_the_awaited_capture_start,
    ));
    let totals = replay::totals();
    eprintln!("E8 pd_witness: {:?}", totals.pd_witness);
    eprintln!("E7 rows compared: {:?}", totals.per_row);
    let missing: Vec<u8> = [1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 16, 18]
        .into_iter()
        .filter(|pd| totals.pd_witness.get(pd).copied().unwrap_or(0) == 0)
        .collect();
    assert!(
        missing.is_empty(),
        "preserved defects the shadow never witnessed: {missing:?} (witnessed: {:?})",
        totals.pd_witness
    );
}

/// C8.7 at the live sites, over every golden: each header is submitted with D free
/// (`shadow.submit`, a `try_lock` on D), and each blocking effect that the goldens
/// reach (`PE(Vad)` of R203.9, the cold `supervisor.start`) runs only after its header
/// was compared. The probe is installed by `run_sub_run_observed`, which fails the
/// sub-run on a violation; here the evidence that the probe saw the points at all
/// (a probe that never fires proves nothing).
#[cfg(feature = "dictation-shadow")]
#[test]
fn headers_leave_d_and_precede_blocking_effects_at_every_live_site() {
    use super::golden_tests as g;
    use crate::dictation_shadow::replay;
    replay::reset_site_totals();
    g::golden_s01_stop_during_start();
    g::golden_s02_cancel_in_each_phase();
    g::golden_s03_interruption_of_8000_samples_or_more();
    g::golden_s04_interruption_under_8000_samples();
    g::golden_s05_timeout_teardown();
    g::golden_s06_stale_stop();
    g::golden_s07_meeting_conflict();
    g::golden_s08_preview_handoff();
    g::golden_s09_supervisor_idle_during_processing();
    g::golden_s10_ready_vs_context_rendezvous();
    g::golden_s11_idle_on_a_finished_recording();
    g::golden_s12_start_failure_vs_initialization_failed();
    let site = replay::site_totals();
    eprintln!("SITE PROBE: {site:?}");
    assert!(site.violations.is_empty(), "{:?}", site.violations);
    assert!(site.submits > 0, "no header submission was probed");
    for point in ["stop.perf_enter_vad", "supervisor.start"] {
        assert!(
            site.blocking_points.get(point).copied().unwrap_or(0) > 0,
            "the goldens never reached `{point}`: {:?}",
            site.blocking_points
        );
    }
}
