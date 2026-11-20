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

/// [`repeat`] for a sub-run that ends `abandoned` flights without a completed close (a
/// task that panicked mid-flight): each repetition names them to the E7 gate.
#[cfg_attr(not(feature = "dictation-shadow"), allow(unused_variables))]
fn repeat_abandoning(sub_run: &SubRun, abandoned: u64) {
    for _ in 0..repeat_count() {
        #[cfg(feature = "dictation-shadow")]
        crate::dictation_shadow::replay::expect(crate::dictation_shadow::replay::Expect {
            abandoned,
            ..Default::default()
        });
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
    repeat_abandoning(
        &sub("PD-13 stop task panic", pd13a_panic_in_the_stop_path),
        1,
    );
    repeat_abandoning(&sub("PD-13 pipeline panic", pd13b_panic_in_the_pipeline), 1);
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

/// The names of the functions of `source` that Cargo runs: a `#[test]` function without
/// `#[ignore]`, at any module depth. A function without the attribute, or an ignored one,
/// is not a witness.
fn running_tests(source: &str) -> std::collections::BTreeSet<String> {
    fn walk(items: &[syn::Item], names: &mut std::collections::BTreeSet<String>) {
        for item in items {
            match item {
                syn::Item::Fn(function) => {
                    let has =
                        |name: &str| function.attrs.iter().any(|attr| attr.path().is_ident(name));
                    if has("test") && !has("ignore") {
                        names.insert(function.sig.ident.to_string());
                    }
                }
                syn::Item::Mod(module) => {
                    if let Some((_, inner)) = &module.content {
                        walk(inner, names);
                    }
                }
                _ => {}
            }
        }
    }
    let file = syn::parse_file(source).expect("the findings tests parse");
    let mut names = std::collections::BTreeSet::new();
    walk(&file.items, &mut names);
    names
}

/// The names between backticks in `text` that are shell witnesses of a property-harness
/// finding (`pd<number>_...`).
fn finding_witness_names(text: &str) -> std::collections::BTreeSet<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|name| {
            name.strip_prefix("pd").is_some_and(|rest| {
                let digits = rest.chars().take_while(char::is_ascii_digit).count();
                digits > 0 && rest[digits..].starts_with('_')
            }) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .map(str::to_string)
        .collect()
}

/// The section of the map that lists the defects the property harness found.
fn harness_section(map: &str) -> &str {
    let start = map
        .find("## Defects found by the property harness")
        .expect("the map has the property-harness section");
    let rest = &map[start..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |at| at + 3);
    &rest[..end]
}

/// Every `pd*_` witness the map's property-harness section names exists in
/// `recording_harness_findings_tests.rs` as a `#[test]` function without `#[ignore]`, and PD-19..PD-24 each (PD-21 and PD-25 are fixed) name at least one: renaming
/// or deleting such a test fails here instead of passing silently.
#[test]
fn the_property_harness_section_names_real_shell_tests() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let map = std::fs::read_to_string(root.join("recording_pd_witness_map.md")).expect("map");
    let section = harness_section(&map);
    let names = finding_witness_names(section);
    let mut running = std::collections::BTreeSet::new();
    let source = std::fs::read_to_string(root.join("recording_harness_findings_tests.rs"))
        .expect("a findings test file");
    running.extend(running_tests(&source));
    assert!(!names.is_empty(), "the section names no shell witness");
    for name in &names {
        assert!(
            running.contains(name),
            "the map names `{name}`, which is not a `#[test]` function without `#[ignore]` in recording_harness_findings_tests.rs"
        );
    }
    // PD-21 is fixed (see the map's "Fixed" section): no entry in this section.
    for pd in (19..=24).filter(|pd| *pd != 21) {
        let line = section
            .lines()
            .find(|line| line.starts_with(&format!("- **PD-{pd}**")))
            .unwrap_or_else(|| panic!("PD-{pd} has no entry in the property-harness section"));
        assert!(
            !finding_witness_names(line).is_empty(),
            "PD-{pd} names no shell witness: {line}"
        );
    }
}

/// A function without `#[test]`, an ignored test and a test in a nested module: only
/// the tests Cargo runs count as witnesses.
#[test]
fn the_witness_check_ignores_functions_cargo_never_runs() {
    let running = running_tests(
        "fn pd1_plain() {}\n#[test]\nfn pd2_runs() {}\n#[test]\n#[ignore = \"slow\"]\nfn pd3_ignored() {}\nmod inner { #[test] fn pd4_nested() {} }",
    );
    assert_eq!(
        running.into_iter().collect::<Vec<_>>(),
        vec!["pd2_runs".to_string(), "pd4_nested".to_string()]
    );
}

/// Every `shell_test` of the core crate's `FINDING_SEEDS` names a real test of
/// `recording_harness_findings_tests.rs` that the map's property-harness section names too
/// (the crate cannot see the app's tests, so the app reads the crate's source).
#[test]
fn every_finding_seed_names_a_real_shell_test() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let seeds = std::fs::read_to_string(root.join("crates/dictation-core/src/property/tests.rs"))
        .expect("the property tests");
    let findings =
        std::fs::read_to_string(root.join("src/commands/recording_harness_findings_tests.rs"))
            .expect("the findings tests");
    let map = std::fs::read_to_string(root.join("src/commands/recording_pd_witness_map.md"))
        .expect("map");
    let mapped = finding_witness_names(harness_section(&map));
    let running = running_tests(&findings);
    let table = &seeds[seeds
        .find("const FINDING_SEEDS")
        .expect("FINDING_SEEDS exists")..];
    let table = &table[..table.find("\n];").expect("the table ends")];
    let mut named = 0;
    let mut rest = table;
    while let Some(at) = rest.find("shell_test:") {
        rest = &rest[at + "shell_test:".len()..];
        let entry = &rest[..rest.find("sig:").unwrap_or(rest.len())];
        if entry.trim_start().starts_with("None") {
            continue;
        }
        let name = entry
            .split('"')
            .nth(1)
            .unwrap_or_else(|| panic!("a shell_test names no string literal: {entry}"));
        named += 1;
        assert!(
            running.contains(name),
            "FINDING_SEEDS names `{name}`, which is not a `#[test]` function without `#[ignore]` in recording_harness_findings_tests.rs"
        );
        assert!(
            mapped.contains(name),
            "FINDING_SEEDS names `{name}`, which the map's property-harness section does not"
        );
    }
    assert!(
        named >= 10,
        "only {named} shell tests were read from FINDING_SEEDS"
    );
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
/// microphone test: the R101.15 flight ran `PR` and never reaches `CS`. As in PD-9
/// proper, the begin-ledger row it registered stays pending with nobody to resolve it,
/// so a completion requested afterwards is held. The spawned blocking task is parked
/// at its supervisor call by a barrier while the ledger is read, then released.
#[cfg(feature = "dictation-shadow")]
fn pd09c_start_dropped_at_the_awaited_capture_start(world: &Arc<World>) {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    world
        .state()
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
    let (reached_tx, reached) = channel::<()>();
    let (go, go_rx) = channel::<()>();
    world.on("supervisor.start", move || {
        let _ = reached_tx.send(());
        let _ = go_rx.recv_timeout(Duration::from_secs(30));
    });
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    drop(start);
    world.note("the start future is dropped while it waits for the capture start");
    reached
        .recv_timeout(Duration::from_secs(30))
        .expect("the blocking task reached the supervisor's capture start");
    // The orphaned ledger row: registered, never begun, and it holds the completion.
    let state = world.state();
    assert!(
        dictation_performance_begins().begin_is_pending(1),
        "PD-9: the dropped start's begin-ledger row stays pending"
    );
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
    go.send(()).expect("the blocking task is parked");
    // The task finishes on its own thread; wait for its last act (the supervisor's
    // acknowledgement is pushed after the barrier and after the cold start), bounded.
    // The entry line is pushed before the barrier, so it proves nothing about the end.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !world
        .trace()
        .iter()
        .any(|line| line.starts_with("supervisor.start_ack 1"))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the blocking task never ran"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// E8 (appendix C8.5): every preserved defect whose row is in a family is witnessed by
/// the shadow. The twelve goldens and the Stage 1 PD witness tests run through the
/// shadow in this process (every sub-run is replayed, `run_sub_run_observed`), and each
/// PD the allowlist credits is counted. PD-11, PD-14 and PD-17 are not row signatures
/// (properties of an effect's implementation, or latent) and are not counted. D-S2c-15 C4:
/// E8 enumerates every preserved defect, PD-1 to PD-25 less the fixed ones ([`FIXED_PDS`])
/// (`e8_pds`): each is driven by a
/// named shell fixture, the PD-19 to PD-24 fixtures are replayed through the shadow too
/// (each must close clean), and `assert_every_preserved_signature_is_accounted` fails on a
/// PD with no row, a row-signature PD the shadow never credited, or a fixture Cargo does
/// not run.
#[cfg(feature = "dictation-shadow")]
#[test]
fn every_family_pd_is_witnessed_by_the_shadow() {
    use crate::dictation_shadow::replay;
    replay::reset_totals();
    run_e8_fixtures();
    let totals = replay::totals();
    eprintln!("E8 pd_witness: {:?}", totals.pd_witness);
    eprintln!("E7 rows compared: {:?}", totals.per_row);
    // S6 (D-S2c-15): hand the replay's counts to `scripts/shadow_soak_verdict.py`. The file is
    // written when this test function *returns*, after the E7 divergence checks (a diverging
    // sub-run fails earlier) and after the E8 assertion below: a failed test leaves no file, and
    // a file from an older run cannot be mistaken for this one: the path must be new (an
    // existing file is refused, never overwritten), and the revision it names is the one this
    // test binary was *built* from (`build.rs`'s `MURMUR_GIT_SHA`), not whatever the source
    // checkout's HEAD is when the test runs. Numbers keyed by row id and PD number, and a
    // revision hash; no content.
    struct E7Dump(std::ffi::OsString);
    impl Drop for E7Dump {
        fn drop(&mut self) {
            if std::thread::panicking() {
                return;
            }
            let totals = crate::dictation_shadow::replay::totals();
            // Per row: `compared` (the header count, for information), `matched` (flights that
            // ran to their end and matched) and `delegated` (a delegated row's verified
            // no-reply closes). E3 reads `matched`, plus `delegated` for the rows of
            // `replay::DELEGATED_ROWS`, which is `Totals::e3_credit`; never `compared`.
            let credit = totals.e3_credit();
            let mut keys: std::collections::BTreeSet<(u16, u16)> =
                totals.per_row.keys().copied().collect();
            keys.extend(totals.row_matched.keys().copied());
            keys.extend(totals.row_delegated.keys().copied());
            let count = |map: &std::collections::BTreeMap<(u16, u16), u64>, key: &(u16, u16)| {
                map.get(key).copied().unwrap_or(0)
            };
            let per_row: serde_json::Map<String, serde_json::Value> = keys
                .iter()
                .map(|key| {
                    (
                        format!("{}.{}", key.0, key.1),
                        serde_json::json!({
                            "compared": count(&totals.per_row, key),
                            "matched": count(&totals.row_matched, key),
                            "delegated": count(&totals.row_delegated, key),
                            "e3_credit": count(&credit, key),
                        }),
                    )
                })
                .collect();
            let pd_witness: serde_json::Map<String, serde_json::Value> = totals
                .pd_witness
                .iter()
                .map(|(pd, count)| (format!("PD-{pd}"), serde_json::json!(count)))
                .collect();
            let repeat = std::env::var("MURMUR_GOLDEN_REPEAT")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(1);
            let git = |args: &[&str]| {
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .output()
                    .ok()
                    .filter(|output| output.status.success())
                    .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            };
            // The revision compiled into this binary. A checkout whose HEAD is not that revision
            // (a binary built earlier, run after a checkout) is not a clean run of it.
            let built = option_env!("MURMUR_GIT_SHA").unwrap_or("unknown");
            let head = git(&["rev-parse", "HEAD"]);
            let dump = serde_json::json!({
                "git_sha": built,
                "dirty": head.as_deref() != Some(built)
                    || git(&["status", "--porcelain", "--untracked-files=no"])
                        .is_none_or(|status| !status.is_empty()),
                "golden_repeat": repeat,
                "per_row": per_row,
                "pd_witness": pd_witness,
            });
            use std::io::Write as _;
            // `create_new`: a path that already exists is a reused one, and a failed rerun
            // would leave the earlier run's dump in place for the verdict to accept.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.0)
                .expect("MURMUR_E7_PER_ROW_OUT must be a new path: use a fresh file for each run");
            file.write_all(&serde_json::to_vec_pretty(&dump).expect("E7 dump JSON"))
                .expect("write MURMUR_E7_PER_ROW_OUT");
        }
    }
    let _e7_dump = std::env::var_os("MURMUR_E7_PER_ROW_OUT").map(E7Dump);
    let missing: Vec<u8> = [1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 15, 16, 18]
        .into_iter()
        .filter(|pd| totals.pd_witness.get(pd).copied().unwrap_or(0) == 0)
        .collect();
    assert!(
        missing.is_empty(),
        "preserved defects the shadow never witnessed: {missing:?} (witnessed: {:?})",
        totals.pd_witness
    );
    assert_every_preserved_signature_is_accounted(&totals);
}

/// What a preserved defect is, to E8 (D-S2c-15 C4).
#[cfg(feature = "dictation-shadow")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PdKind {
    /// A row signature of the shadow's allowlist (`pd::PD_DEFS`): the shadow credits it,
    /// and E8 requires the credit to be above zero.
    Row,
    /// A property of an effect's implementation, not a row (PD-11, PD-14): the named
    /// fixture is its own `#[test]`, which E8 does not replay.
    Property,
    /// Not a row signature either: a split between a recording's terminal and its
    /// performance outcome that the property harness found (PD-19 to PD-24). E8 replays
    /// the named fixtures through the shadow in this process, where each must close
    /// clean (E7), and the shadow credits nothing for them.
    CrossLedger,
    /// No production schedule (PD-17): there is no fixture to name.
    Latent,
}

#[cfg(feature = "dictation-shadow")]
type Fixture = (&'static str, &'static str, fn());

/// A fixture by the name of its function: `(name, the path of the function, the function)`.
/// The path is what the compiled test harness lists the test as (minus the crate).
#[cfg(feature = "dictation-shadow")]
macro_rules! fx {
    ($module:ident :: $name:ident) => {
        (
            stringify!($name),
            std::any::type_name_of_val(&$module::$name),
            $module::$name as fn(),
        )
    };
    ($name:ident) => {
        (
            stringify!($name),
            std::any::type_name_of_val(&$name),
            $name as fn(),
        )
    };
}

/// PD-9's signature in the allowlist is an R101.15 flight abandoned between `PR` and
/// `CS`: a start future dropped while it waits for a kept preview's capture start.
/// [`run_e8_fixtures`] replays it with PD-9's `#[test]` fixture (it is a helper, not a test).
#[cfg(feature = "dictation-shadow")]
fn pd09c_abandoned_at_the_awaited_capture_start() {
    repeat_abandoning(
        &sub(
            "PD-9 abandoned at the awaited capture start",
            pd09c_start_dropped_at_the_awaited_capture_start,
        ),
        1,
    );
}

/// The PDs a fix PR has fixed and removed (C7.0): no E8 row, no allowlist entry, no
/// signature. Their shell fixtures assert the fixed behaviour as ordinary tests. PD-21:
/// `a_timed_out_cancel_claims_nothing_before_a_short_stop` and
/// `a_timed_out_cancel_claims_nothing_before_the_late_recovery`
/// (`recording_harness_findings_tests.rs`); PD-25 (#1157): `recording_pd25_tests.rs`.
#[cfg(feature = "dictation-shadow")]
const FIXED_PDS: &[u8] = &[21, 25];

/// Every preserved defect, PD-1 to PD-25 less [`FIXED_PDS`], with the shell fixtures that
/// assert it. Seed 92,332 of #1157 is not a defect (the generator draws an uncaused
/// `Cancelled`) and is not here. A new PD, or a PD added to the shadow's allowlist, needs its row here:
/// [`assert_every_preserved_signature_is_accounted`] fails otherwise.
#[cfg(feature = "dictation-shadow")]
fn e8_pds() -> Vec<(u8, PdKind, Vec<Fixture>)> {
    use super::golden_tests as g;
    use super::harness_findings_tests as f;
    use PdKind::{CrossLedger, Latent, Property, Row};
    vec![
        (1, Row, vec![fx!(g::golden_s04_interruption_under_8000_samples)]),
        (2, Row, vec![fx!(g::golden_s11_idle_on_a_finished_recording)]),
        (
            3,
            Row,
            vec![
                fx!(g::golden_s12_start_failure_vs_initialization_failed),
                fx!(pd03_stop_missing_context_runs_unconditional_idle_effects),
            ],
        ),
        (4, Row, vec![fx!(g::golden_s10_ready_vs_context_rendezvous)]),
        (
            5,
            Row,
            vec![fx!(pd05_supervisor_events_for_the_current_id_claim_superseded)],
        ),
        (
            6,
            Row,
            vec![
                fx!(g::golden_s05_timeout_teardown),
                fx!(g::golden_s09_supervisor_idle_during_processing),
                fx!(pd06_pipeline_guard_overwrites_recovering),
            ],
        ),
        (
            7,
            Row,
            vec![
                fx!(g::golden_s09_supervisor_idle_during_processing),
                fx!(g::golden_s05_timeout_teardown),
            ],
        ),
        (
            8,
            Row,
            vec![fx!(pd08_flushing_start_leaves_its_context_installed)],
        ),
        (
            9,
            Row,
            vec![fx!(pd09_dropped_start_orphans_the_begin_ledger_row)],
        ),
        (
            10,
            Row,
            vec![fx!(pd10_terminal_before_accepted_is_dropped_and_ledger_evicts)],
        ),
        (
            11,
            Property,
            vec![fx!(pd11_terminal_claim_and_diagnostics_complete_are_not_atomic)],
        ),
        (
            12,
            Row,
            vec![
                fx!(g::golden_s01_stop_during_start),
                fx!(g::golden_s06_stale_stop),
            ],
        ),
        (
            13,
            Row,
            vec![fx!(pd13_task_panic_restores_idle_without_a_terminal)],
        ),
        (
            14,
            Property,
            vec![fx!(pd14_ticker_task_panic_never_runs_the_hide)],
        ),
        (
            15,
            Row,
            vec![
                fx!(g::golden_s03_interruption_of_8000_samples_or_more),
                fx!(g::golden_s04_interruption_under_8000_samples),
            ],
        ),
        (16, Row, vec![fx!(g::golden_s02_cancel_in_each_phase)]),
        (17, Latent, vec![]),
        (
            18,
            Row,
            vec![
                fx!(g::golden_s04_interruption_under_8000_samples),
                fx!(pd18bc_start_in_the_teardown_tail_leaves_recovering),
            ],
        ),
        (
            19,
            CrossLedger,
            vec![
                fx!(f::pd19_termination_unconfirmed_after_retained_pcm_leaves_recovering),
                fx!(f::pd19_a_dropped_start_and_a_retained_runtime_failure_leave_recovering),
                fx!(f::pd19_a_refused_auto_stop_leaves_the_capture_and_recovering),
            ],
        ),
        (
            20,
            CrossLedger,
            vec![fx!(f::pd20_cancel_terminal_and_a_short_stop_performance_outcome_disagree)],
        ),
        (
            22,
            CrossLedger,
            vec![
                fx!(f::pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance),
                fx!(f::pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance),
            ],
        ),
        (
            23,
            CrossLedger,
            vec![
                fx!(f::pd23_superseded_terminal_and_an_empty_stop_no_speech_disagree),
                fx!(f::pd23_superseded_terminal_and_a_refused_stop_failed_disagree),
                fx!(f::pd23_superseded_terminal_and_a_cancel_of_processing_cancelled_disagree),
            ],
        ),
        (
            24,
            CrossLedger,
            vec![
                fx!(f::pd24_start_timeout_terminal_and_the_users_cancel_arm_performance_outcome_disagree),
                fx!(f::pd24_start_timeout_terminal_and_an_environmental_recovery_arm_performance_outcome_disagree),
            ],
        ),
    ]
}

/// Run every golden, then every replayed fixture of [`e8_pds`] once (a fixture named for
/// two PDs runs once), so each closes through the shadow in this process.
#[cfg(feature = "dictation-shadow")]
fn run_e8_fixtures() {
    use super::golden_tests as g;
    let goldens: [Fixture; 12] = [
        fx!(g::golden_s01_stop_during_start),
        fx!(g::golden_s02_cancel_in_each_phase),
        fx!(g::golden_s03_interruption_of_8000_samples_or_more),
        fx!(g::golden_s04_interruption_under_8000_samples),
        fx!(g::golden_s05_timeout_teardown),
        fx!(g::golden_s06_stale_stop),
        fx!(g::golden_s07_meeting_conflict),
        fx!(g::golden_s08_preview_handoff),
        fx!(g::golden_s09_supervisor_idle_during_processing),
        fx!(g::golden_s10_ready_vs_context_rendezvous),
        fx!(g::golden_s11_idle_on_a_finished_recording),
        fx!(g::golden_s12_start_failure_vs_initialization_failed),
    ];
    let mut ran = std::collections::BTreeSet::new();
    let mut run = |(name, _, fixture): Fixture| {
        if ran.insert(name) {
            fixture();
        }
    };
    goldens.into_iter().for_each(&mut run);
    for (_, kind, fixtures) in e8_pds() {
        if kind != PdKind::Property {
            fixtures.into_iter().for_each(&mut run);
        }
    }
    run(fx!(pd09c_abandoned_at_the_awaited_capture_start));
}

/// The tests the compiled harness lists, and those of them it ignores, by their full path
/// without the crate (`commands::recording::golden_tests::golden_s01..`). Asked of this very
/// test binary (`--list`), so `cfg_attr(.., ignore)` and `cfg` are resolved as compiled.
#[cfg(feature = "dictation-shadow")]
fn compiled_harness() -> &'static (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    static LISTS: std::sync::OnceLock<(
        std::collections::BTreeSet<String>,
        std::collections::BTreeSet<String>,
    )> = std::sync::OnceLock::new();
    LISTS.get_or_init(|| {
        let list = |ignored_only: bool| {
            let mut command = std::process::Command::new(
                std::env::current_exe().expect("the test binary's own path"),
            );
            command.args(["--list", "--format", "terse"]);
            if ignored_only {
                command.arg("--ignored");
            }
            let output = command.output().expect("the test binary lists its tests");
            assert!(output.status.success(), "`--list` failed");
            String::from_utf8(output.stdout)
                .expect("a test list is UTF-8")
                .lines()
                .filter_map(|line| line.strip_suffix(": test"))
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<_>>()
        };
        (list(false), list(true))
    })
}

/// Whether the compiled harness runs the test at `path` (a `type_name` path, with the
/// crate): it lists it and does not ignore it.
#[cfg(feature = "dictation-shadow")]
fn compiled_harness_runs(path: &str) -> bool {
    let Some((_, listed_as)) = path.split_once("::") else {
        return false;
    };
    let (all, ignored) = compiled_harness();
    all.contains(listed_as) && !ignored.contains(listed_as)
}

/// A test the feature build ignores through a `cfg_attr`, as the control for
/// [`e8_rejects_a_fixture_the_compiled_harness_skips`]: a source scan for `#[ignore]` sees no
/// `ignore` attribute on it.
#[cfg(feature = "dictation-shadow")]
#[test]
#[cfg_attr(feature = "dictation-shadow", ignore = "E8 probe: skipped by libtest")]
fn e8_probe_skipped_through_cfg_attr() {}

/// E8 does not accept a fixture libtest skips (a `cfg_attr(.., ignore)` the feature build
/// enables), nor a path that is no test, and does accept one it runs.
#[cfg(feature = "dictation-shadow")]
#[test]
fn e8_rejects_a_fixture_the_compiled_harness_skips() {
    let probe = std::any::type_name_of_val(&e8_probe_skipped_through_cfg_attr);
    assert!(
        !compiled_harness_runs(probe),
        "the harness skips the probe, so E8 must not take it as run: {probe}"
    );
    let running = std::any::type_name_of_val(&pd13_task_panic_restores_idle_without_a_terminal);
    assert!(compiled_harness_runs(running), "{running}");
    assert!(!compiled_harness_runs(
        "ui_lib::commands::recording::nothing"
    ));
    assert!(
        !compiled_harness().1.is_empty(),
        "the control saw no ignored test at all"
    );
}

/// E8's accounting (D-S2c-15 C4), over what [`run_e8_fixtures`] replayed:
/// * PD-1 to PD-25, less [`FIXED_PDS`], each have one row in [`e8_pds`], and a latent PD names no fixture;
/// * the PDs of the shadow's allowlist are exactly the `Row` ones, and each was credited;
/// * the shadow credited nothing else;
/// * every named fixture is, by its full path, a test the **compiled** harness runs: listed by
///   libtest in this very build and not ignored in it (so an `#[ignore]` behind a `cfg_attr`
///   that this build enables is seen).
#[cfg(feature = "dictation-shadow")]
fn assert_every_preserved_signature_is_accounted(totals: &crate::dictation_shadow::replay::Totals) {
    let table = e8_pds();
    let numbers: Vec<u8> = table.iter().map(|(pd, _, _)| *pd).collect();
    assert_eq!(
        numbers,
        (1..=25)
            .filter(|pd| !FIXED_PDS.contains(pd))
            .collect::<Vec<u8>>(),
        "one E8 row per unfixed PD"
    );
    let allowlisted: std::collections::BTreeSet<u8> = crate::dictation_shadow::pd::PD_DEFS
        .iter()
        .map(|def| def.pd)
        .collect();
    let rows: std::collections::BTreeSet<u8> = table
        .iter()
        .filter(|(_, kind, _)| *kind == PdKind::Row)
        .map(|(pd, _, _)| *pd)
        .collect();
    assert_eq!(
        allowlisted, rows,
        "the shadow's allowlist and E8's row-signature PDs differ"
    );
    for (pd, kind, fixtures) in &table {
        assert_eq!(
            fixtures.is_empty(),
            *kind == PdKind::Latent,
            "PD-{pd}: only a latent PD names no fixture"
        );
        for (name, path, _) in fixtures {
            assert!(
                compiled_harness_runs(path),
                "PD-{pd}: `{name}` ({path}) is not a test the compiled harness runs: it is \
                 missing, ignored in this build, or not a `#[test]`"
            );
        }
        let credited = totals.pd_witness.get(pd).copied().unwrap_or(0);
        match kind {
            PdKind::Row => assert!(
                credited > 0,
                "PD-{pd} is a row signature and the shadow never witnessed it"
            ),
            _ => assert_eq!(
                credited, 0,
                "PD-{pd} is not a row signature, yet the shadow credited it {credited} times"
            ),
        }
    }
    assert!(
        totals.pd_witness.keys().all(|pd| rows.contains(pd)),
        "the shadow credited a PD E8 does not list as a row signature: {:?}",
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

// ------------------------------------------ what the armed shadow adds under D (D-S2c-7 b, c)

#[cfg(feature = "dictation-shadow")]
pub(super) mod under_d {
    use super::*;
    use crate::dictation_shadow::alloc_probe::count_allocations_when;
    use std::cell::Cell;

    thread_local! {
        static D: Cell<*const std::sync::Mutex<crate::state::DictationState>> =
            const { Cell::new(std::ptr::null()) };
        static ALLOCATIONS_UNDER_D: Cell<u64> = const { Cell::new(0) };
        static WARNED_UNDER_D: Cell<Vec<bool>> = const { Cell::new(Vec::new()) };
        /// `(core_now calls, view projections)` made by the measured entries.
        static READS_UNDER_D: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
        /// Reads of the shadow's own per-thread state made by the measured entries.
        static SHADOW_READS_UNDER_D: Cell<u64> = const { Cell::new(0) };
        /// Reads of the shadow's enable flag made by the measured entries.
        static FLAG_LOADS_UNDER_D: Cell<u64> = const { Cell::new(0) };
    }

    /// Run `f` and add the clock reads and view projections it made to [`READS_UNDER_D`].
    fn count_reads<R>(f: impl FnOnce() -> R) -> R {
        let before = crate::dictation_executor::probe::counts();
        let shadow_before = crate::dictation_executor::probe::shadow_reads();
        let loads_before = crate::dictation_executor::probe::flag_loads();
        let result = f();
        FLAG_LOADS_UNDER_D.with(|total| {
            total.set(total.get() + crate::dictation_executor::probe::flag_loads() - loads_before)
        });
        let after = crate::dictation_executor::probe::counts();
        SHADOW_READS_UNDER_D.with(|total| {
            total
                .set(total.get() + crate::dictation_executor::probe::shadow_reads() - shadow_before)
        });
        READS_UNDER_D.with(|total| {
            let (now, views) = total.get();
            total.set((now + after.0 - before.0, views + after.1 - before.1));
        });
        result
    }

    /// The dictation lock is held right now (by this thread: the shell is synchronous).
    /// Performs no allocation, which the allocator calls it from requires.
    fn d_is_held() -> bool {
        D.try_with(|pointer| {
            let pointer = pointer.get();
            // SAFETY: the pointer is set by `measure` from the world's own state, which
            // outlives the measured call, and is cleared right after it.
            !pointer.is_null()
                && matches!(
                    unsafe { &*pointer }.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                )
        })
        .unwrap_or(false)
    }

    /// Run `f` and count the allocations this thread made while `dictation` was held.
    pub(in crate::commands) fn count_under_d<R>(
        dictation: &std::sync::Mutex<crate::state::DictationState>,
        f: impl FnOnce() -> R,
    ) -> (R, u64) {
        D.with(|pointer| pointer.set(dictation));
        let (result, allocations) = count_allocations_when(d_is_held, f);
        D.with(|pointer| pointer.set(std::ptr::null()));
        (result, allocations)
    }

    /// Drive an entry to completion and count the allocations made while D was held.
    fn measure<T>(world: &Arc<World>, mut entry: Fut<T>) {
        let state = world.state();
        let (_, allocations) = count_under_d(&state.app_state.dictation, || {
            count_reads(|| complete(&mut entry))
        });
        ALLOCATIONS_UNDER_D.with(|total| total.set(total.get() + allocations));
    }

    /// What each refusal allocates while D is held in the default build (counted on a
    /// feature build with the shadow disarmed, where every shadow read is gated off).
    /// An intended change to a refusal's own D section re-pins its line.
    const DEFAULT_BUILD_ALLOCATIONS_UNDER_D: [(&str, u64); 10] = [
        ("start refused before D (Smart Auto with a fixed device)", 0),
        ("start refused under D (a meeting is active)", 6),
        ("start refused under D (already recording)", 21),
        ("stop refused (stale, R201.1)", 0),
        ("stop refused (processing, R201.2)", 5),
        ("stop refused (idle, R201.3)", 5),
        ("stop refused (recovering, nothing preserved, R201.6)", 11),
        ("cancel refused (stale, R205.1)", 0),
        ("cancel no-op (idle, R205.2)", 0),
        ("cancel no-op (recovering, R205.3)", 0),
    ];

    struct Scenario {
        title: &'static str,
        setup: Setup,
        body: fn(&Arc<World>),
    }

    fn at(status: DictationStatus, recording_id: u64) -> Setup {
        Setup {
            status,
            recording_id,
        }
    }

    /// One refusal branch per entry of every family that refuses under D or before it:
    /// start (pre-D Smart Auto + fixed device, meeting, already recording), stop
    /// (R201.1 stale, .2 processing, .3 idle, .6 nothing preserved), cancel (R205.1
    /// stale, R205.2 idle, R205.3 recovering).
    fn scenarios() -> Vec<Scenario> {
        vec![
            Scenario {
                title: "start refused before D (Smart Auto with a fixed device)",
                setup: Setup::idle(),
                body: |w| measure(w, w.start_smart_auto(Some("fixed".to_string()))),
            },
            Scenario {
                title: "start refused under D (a meeting is active)",
                setup: Setup::idle(),
                body: |w| {
                    w.state()
                        .app_state
                        .meeting_active
                        .store(true, Ordering::SeqCst);
                    measure(w, w.start());
                },
            },
            Scenario {
                title: "start refused under D (already recording)",
                setup: at(DictationStatus::Recording, 1),
                body: |w| measure(w, w.start()),
            },
            Scenario {
                title: "stop refused (stale, R201.1)",
                setup: at(DictationStatus::Recording, 2),
                body: |w| measure(w, w.stop(Some(1))),
            },
            Scenario {
                title: "stop refused (processing, R201.2)",
                setup: at(DictationStatus::Processing, 1),
                body: |w| measure(w, w.stop(None)),
            },
            Scenario {
                title: "stop refused (idle, R201.3)",
                setup: Setup::idle(),
                body: |w| measure(w, w.stop(None)),
            },
            Scenario {
                title: "stop refused (recovering, nothing preserved, R201.6)",
                setup: at(DictationStatus::Recovering, 1),
                body: |w| measure(w, w.stop(None)),
            },
            Scenario {
                title: "cancel refused (stale, R205.1)",
                setup: at(DictationStatus::Recording, 2),
                body: |w| measure(w, w.cancel(Some(1))),
            },
            Scenario {
                title: "cancel no-op (idle, R205.2)",
                setup: Setup::idle(),
                body: |w| measure(w, w.cancel(None)),
            },
            Scenario {
                title: "cancel no-op (recovering, R205.3)",
                setup: at(DictationStatus::Recovering, 1),
                body: |w| measure(w, w.cancel(None)),
            },
        ]
    }

    fn run_measured(scenario: &Scenario, disarmed: bool) -> u64 {
        ALLOCATIONS_UNDER_D.with(|total| total.set(0));
        READS_UNDER_D.with(|total| total.set((0, 0)));
        SHADOW_READS_UNDER_D.with(|total| total.set(0));
        FLAG_LOADS_UNDER_D.with(|total| total.set(0));
        // A disarmed run reaches the site hooks with no replay to compare against: its
        // "violations" are not findings, and must not reach the next armed run's check.
        crate::dictation_shadow::replay::reset_site_totals();
        REPLAY_DISARMED.store(disarmed, Ordering::SeqCst);
        struct Rearm;
        impl Drop for Rearm {
            fn drop(&mut self) {
                REPLAY_DISARMED.store(false, Ordering::SeqCst);
            }
        }
        let _rearm = Rearm;
        run_sub_run(&SubRun {
            title: scenario.title,
            setup: Setup {
                status: scenario.setup.status,
                recording_id: scenario.setup.recording_id,
            },
            body: scenario.body,
        });
        crate::dictation_shadow::replay::reset_site_totals();
        ALLOCATIONS_UNDER_D.with(Cell::get)
    }

    /// D-S2c-7 b: through the shell entry point (not the `Flight` API), each refusal
    /// branch allocates under D exactly what it allocates with the shadow compiled in and
    /// not armed. The armed run builds no executor, clones no effect vector and sends
    /// nothing that allocates while D is held.
    #[test]
    fn the_armed_shadow_adds_no_allocation_under_d_at_any_refusal() {
        let mut counted = 0;
        for scenario in scenarios() {
            let baseline = run_measured(&scenario, true);
            let armed = run_measured(&scenario, false);
            assert_eq!(
                armed, baseline,
                "{}: armed {armed} allocations under D, disarmed {baseline}",
                scenario.title
            );
            // The disarmed run is the default build's allocation: every shadow read is
            // gated on an armed flight, so a feature-only extra under D (equal armed and
            // disarmed, so the comparison above cannot see it) changes this number.
            let (_, pinned) = DEFAULT_BUILD_ALLOCATIONS_UNDER_D
                .iter()
                .find(|(title, _)| *title == scenario.title)
                .unwrap_or_else(|| panic!("{}: no pinned default-build count", scenario.title));
            assert_eq!(
                baseline, *pinned,
                "{}: {baseline} allocations under D with the shadow compiled in and not armed, \
                 the default build makes {pinned} (D-S2c-7 c)",
                scenario.title
            );
            counted += baseline;
        }
        assert!(
            counted > 0,
            "the probe counted no allocation under D at all: it is not measuring"
        );
    }

    /// What the lifecycle bridge allocates while D is held in the default build, per
    /// notification, as `(title, first delivery on a thread, a later one)` (counted on a
    /// feature build with the shadow disarmed). The bridge is the one live path the entry
    /// scenarios above do not reach, and the shadow's capture sits inside it: a disarmed
    /// shadow must not allocate there (D-S2c-8 b), and a cost that only the first delivery
    /// pays must not hide behind a later one (D-S2c-9 f).
    const DEFAULT_BUILD_BRIDGE_ALLOCATIONS_UNDER_D: [(&str, u64, u64); 6] = [
        ("bridge Ready while starting (R207)", 2, 2),
        ("bridge StillConnecting while starting (R208)", 20, 19),
        ("bridge Recovering while recording (R209)", 9, 9),
        ("bridge CleanupFinished while recovering (R210)", 30, 30),
        ("bridge Idle while recording (R211)", 26, 26),
        ("bridge Idle while idle (short idle, R215.3/R216)", 6, 5),
    ];

    /// The blocking threads the pool is warmed to hold idle before a measured delivery.
    const WARM_BLOCKING_THREADS: usize = 8;

    /// Make the app runtime's blocking pool hold [`WARM_BLOCKING_THREADS`] idle threads, so a
    /// measured delivery never makes the pool create one under D. The R211 arm persists
    /// the recording's performance completion with `spawn_blocking`
    /// (`spawn_dictation_performance_completion`, a call both builds make while D is
    /// held), and Tokio creates a worker thread there whenever none is idle: seven
    /// allocations that land in the calling thread's count. Whether one is idle depends on
    /// whether the previous delivery's persist has returned to the pool, which is timing,
    /// not behaviour; the pins measure the bridge, not the pool. The tasks rendezvous on a
    /// barrier, so every one of them holds its own thread at once; the pool then keeps them
    /// idle for its keep-alive, far longer than a run. First-delivery and steady-state
    /// numbers stay separate measurements of the same code.
    fn warm_the_blocking_pool() {
        let barrier = Arc::new(std::sync::Barrier::new(WARM_BLOCKING_THREADS + 1));
        let tasks: Vec<_> = (0..WARM_BLOCKING_THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                tauri::async_runtime::spawn_blocking(move || {
                    barrier.wait();
                })
            })
            .collect();
        barrier.wait();
        tauri::async_runtime::block_on(async {
            for task in tasks {
                task.await.expect("a warming task finished");
            }
        });
    }

    fn bridge_measure(world: &Arc<World>, event: AudioLifecycleEvent) {
        let state = world.state();
        let (_, allocations) = count_under_d(&state.app_state.dictation, || {
            count_reads(|| world.notify(1, event))
        });
        ALLOCATIONS_UNDER_D.with(|total| total.set(total.get() + allocations));
    }

    /// One scenario per bridge arm that takes a decision section: Ready (R207),
    /// StillConnecting (R208), Recovering (R209), CleanupFinished (R210), Idle (R211)
    /// and the short idle (R215.3/R216), plus the same notifications arriving idle.
    fn bridge_scenarios() -> Vec<Scenario> {
        vec![
            Scenario {
                title: "bridge Ready while starting (R207)",
                setup: at(DictationStatus::Starting, 1),
                body: |w| bridge_measure(w, AudioLifecycleEvent::Ready),
            },
            Scenario {
                title: "bridge StillConnecting while starting (R208)",
                setup: at(DictationStatus::Starting, 1),
                body: |w| bridge_measure(w, AudioLifecycleEvent::StillConnecting),
            },
            Scenario {
                title: "bridge Recovering while recording (R209)",
                setup: at(DictationStatus::Recording, 1),
                body: |w| {
                    bridge_measure(
                        w,
                        AudioLifecycleEvent::Recovering {
                            reason: crate::audio_lifecycle::AudioCancelReason::DeviceChanged,
                        },
                    )
                },
            },
            Scenario {
                title: "bridge CleanupFinished while recovering (R210)",
                setup: at(DictationStatus::Recovering, 1),
                body: |w| bridge_measure(w, AudioLifecycleEvent::CleanupFinished),
            },
            Scenario {
                title: "bridge Idle while recording (R211)",
                setup: at(DictationStatus::Recording, 1),
                body: |w| bridge_measure(w, AudioLifecycleEvent::Idle),
            },
            Scenario {
                title: "bridge Idle while idle (short idle, R215.3/R216)",
                setup: Setup::idle(),
                body: |w| bridge_measure(w, AudioLifecycleEvent::Idle),
            },
        ]
    }

    /// D-S2c-8 b: the lifecycle bridge, like every refusal above, is inert while the
    /// shadow is compiled in and not armed: no shadow read, no per-notification
    /// allocation under D, and the armed run adds none either.
    #[test]
    fn the_lifecycle_bridge_allocates_the_same_armed_disarmed_and_in_the_default_build() {
        // A delivery's first run on a thread pays that thread's one-time costs (a
        // thread-local's first use); a later one does not. Both are pinned, separately:
        // taking the least of several runs would hide a regression only the first delivery
        // makes. The run happens on a fresh thread under a subscriber that formats nothing,
        // so a logged duration's digits cannot move the count (D-S2c-9 f). One throwaway
        // delivery first warms the process-wide state (callsite registration) so the
        // numbers do not depend on which test ran before.
        let pair = |scenario: &Scenario, disarmed: bool| -> (u64, u64) {
            warm_the_blocking_pool();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let quiet = tracing_subscriber::Registry::default();
                        tracing::subscriber::with_default(quiet, || {
                            (
                                run_measured(scenario, disarmed),
                                run_measured(scenario, disarmed),
                            )
                        })
                    })
                    .join()
                    .expect("the measuring thread panicked")
            })
        };
        let mut counted = 0;
        for scenario in bridge_scenarios() {
            let _ = pair(&scenario, true);
            let baseline = pair(&scenario, true);
            let armed = pair(&scenario, false);
            assert_eq!(
                armed, baseline,
                "{}: armed (first, later) {armed:?} allocations under D, disarmed {baseline:?}",
                scenario.title
            );
            let (_, cold, steady) = DEFAULT_BUILD_BRIDGE_ALLOCATIONS_UNDER_D
                .iter()
                .find(|(title, _, _)| *title == scenario.title)
                .unwrap_or_else(|| panic!("{}: no pinned default-build count", scenario.title));
            assert_eq!(
                baseline,
                (*cold, *steady),
                "{}: (first, later) {baseline:?} allocations under D with the shadow compiled \
                 in and not armed, the default build makes ({cold}, {steady}) (D-S2c-9 f)",
                scenario.title
            );
            counted += baseline.0 + baseline.1;
        }
        assert!(
            counted > 0,
            "the probe counted no allocation under D at all"
        );
    }

    /// The clock reads and view projections each entry makes with the shadow compiled in
    /// and not armed (the default build's own): `(core_now calls, view projections)`, then
    /// the same armed. The disarmed shadow reads nothing for any entry; the armed one
    /// reads exactly what its header needs. An extra read added under D allocates nothing,
    /// so the allocation pins cannot see it; one of these does (D-S2c-10 R4).
    type Reads = (u64, u64);
    const READS: [(&str, Reads, Reads); 17] = [
        (
            "start refused before D (Smart Auto with a fixed device)",
            (1, 0),
            (1, 1),
        ),
        (
            "start refused under D (a meeting is active)",
            (1, 1),
            (1, 1),
        ),
        ("start refused under D (already recording)", (1, 1), (1, 1)),
        ("stop refused (stale, R201.1)", (1, 0), (1, 1)),
        ("stop refused (processing, R201.2)", (1, 1), (1, 1)),
        ("stop refused (idle, R201.3)", (1, 1), (1, 1)),
        (
            "stop refused (recovering, nothing preserved, R201.6)",
            (1, 1),
            (1, 1),
        ),
        ("cancel refused (stale, R205.1)", (1, 1), (1, 1)),
        ("cancel no-op (idle, R205.2)", (1, 1), (1, 1)),
        ("cancel no-op (recovering, R205.3)", (1, 1), (1, 1)),
        ("bridge Ready while starting (R207)", (1, 1), (1, 1)),
        (
            "bridge StillConnecting while starting (R208)",
            (1, 1),
            (1, 1),
        ),
        ("bridge Recovering while recording (R209)", (1, 1), (1, 1)),
        (
            "bridge CleanupFinished while recovering (R210)",
            (1, 1),
            (1, 1),
        ),
        ("bridge Idle while recording (R211)", (1, 1), (1, 1)),
        (
            "bridge Idle while idle (short idle, R215.3/R216)",
            (1, 1),
            (1, 1),
        ),
        (
            "start accepted (R104.2, the ticker spawner under D)",
            (5, 8),
            (5, 9),
        ),
    ];

    /// The entries whose reads are pinned: every refusal, every bridge arm, and the accepted
    /// start whose R104.2 `SD` asks the ticker spawner (the ticker mailbox's take).
    fn read_scenarios() -> Vec<Scenario> {
        let mut all = scenarios();
        all.extend(bridge_scenarios());
        all.push(Scenario {
            title: "start accepted (R104.2, the ticker spawner under D)",
            setup: Setup::idle(),
            body: |w| count_reads(|| recording(w)),
        });
        all
    }

    fn reads_of(scenario: &Scenario, disarmed: bool) -> (u64, u64) {
        run_measured(scenario, disarmed);
        READS_UNDER_D.with(Cell::get)
    }

    #[test]
    fn the_shadow_reads_the_clock_and_the_live_fields_only_as_pinned() {
        let mut counted = (0, 0);
        for scenario in read_scenarios() {
            let disarmed = reads_of(&scenario, true);
            assert_eq!(
                SHADOW_READS_UNDER_D.with(Cell::get),
                0,
                "{}: the disarmed shadow read its own thread-local state",
                scenario.title
            );
            let armed = reads_of(&scenario, false);
            if std::env::var_os("MURMUR_PRINT_PINS").is_some() {
                eprintln!("PIN (\"{}\", {disarmed:?}, {armed:?}),", scenario.title);
                continue;
            }
            let (_, pinned_disarmed, pinned_armed) = READS
                .iter()
                .find(|(title, _, _)| *title == scenario.title)
                .unwrap_or_else(|| panic!("{}: no pinned reads", scenario.title));
            assert_eq!(
                disarmed, *pinned_disarmed,
                "{}: (core_now, views) {disarmed:?} with the shadow compiled in and not armed, \
                 pinned {pinned_disarmed:?} (D-S2c-10 R4)",
                scenario.title
            );
            assert_eq!(
                armed, *pinned_armed,
                "{}: (core_now, views) {armed:?} with the shadow armed, pinned {pinned_armed:?}",
                scenario.title
            );
            counted = (counted.0 + disarmed.0, counted.1 + armed.1);
        }
        assert!(
            counted.0 > 0 || std::env::var_os("MURMUR_PRINT_PINS").is_some(),
            "the probe counted no clock read at all: it is not measuring"
        );
    }

    /// What the shadow reads of **its own** state and of its enable flag per entry, as
    /// `(state reads, flag loads)` disarmed, then armed (D-S2c-10 amendment 1 i, ii). Each
    /// inserted block makes exactly one flag load, as its first operation, and the ticker
    /// mailbox is taken once per spawner call: a second load, a second take or a read made
    /// while disarmed changes a number here. Disarmed, no entry reads any state.
    type Gates = (u64, u64);
    const GATES: [(&str, Gates, Gates); 17] = [
        (
            "start refused before D (Smart Auto with a fixed device)",
            (0, 6),
            (1, 5),
        ),
        (
            "start refused under D (a meeting is active)",
            (0, 6),
            (1, 5),
        ),
        ("start refused under D (already recording)", (0, 6), (1, 5)),
        ("stop refused (stale, R201.1)", (0, 6), (1, 5)),
        ("stop refused (processing, R201.2)", (0, 3), (0, 2)),
        ("stop refused (idle, R201.3)", (0, 3), (0, 2)),
        (
            "stop refused (recovering, nothing preserved, R201.6)",
            (0, 3),
            (0, 2),
        ),
        ("cancel refused (stale, R205.1)", (0, 3), (0, 2)),
        ("cancel no-op (idle, R205.2)", (0, 3), (0, 2)),
        ("cancel no-op (recovering, R205.3)", (0, 3), (0, 2)),
        ("bridge Ready while starting (R207)", (0, 7), (3, 6)),
        (
            "bridge StillConnecting while starting (R208)",
            (0, 5),
            (1, 4),
        ),
        ("bridge Recovering while recording (R209)", (0, 18), (1, 17)),
        (
            "bridge CleanupFinished while recovering (R210)",
            (0, 9),
            (1, 8),
        ),
        ("bridge Idle while recording (R211)", (0, 16), (3, 15)),
        (
            "bridge Idle while idle (short idle, R215.3/R216)",
            (0, 5),
            (1, 4),
        ),
        (
            "start accepted (R104.2, the ticker spawner under D)",
            (0, 75),
            (21, 68),
        ),
    ];

    #[test]
    fn the_shadow_makes_only_its_one_flag_load_and_its_one_take_as_pinned() {
        for scenario in read_scenarios() {
            crate::dictation_executor::probe::record_flag_sites(true);
            run_measured(&scenario, true);
            let sites = crate::dictation_executor::probe::take_flag_sites();
            crate::dictation_executor::probe::record_flag_sites(false);
            let disarmed = (
                SHADOW_READS_UNDER_D.with(Cell::get),
                FLAG_LOADS_UNDER_D.with(Cell::get),
            );
            if std::env::var_os("MURMUR_PRINT_PINS").is_some() {
                eprintln!("SITES {} disarmed: {sites:?}", scenario.title);
            }
            run_measured(&scenario, false);
            let armed = (
                SHADOW_READS_UNDER_D.with(Cell::get),
                FLAG_LOADS_UNDER_D.with(Cell::get),
            );
            if std::env::var_os("MURMUR_PRINT_PINS").is_some() {
                eprintln!("GATE (\"{}\", {disarmed:?}, {armed:?}),", scenario.title);
                continue;
            }
            let (_, pinned_disarmed, pinned_armed) = GATES
                .iter()
                .find(|(title, _, _)| *title == scenario.title)
                .unwrap_or_else(|| panic!("{}: no pinned gates", scenario.title));
            assert_eq!(
                disarmed, *pinned_disarmed,
                "{}: (state reads, flag loads) {disarmed:?} not armed, pinned {pinned_disarmed:?}",
                scenario.title
            );
            assert_eq!(
                armed, *pinned_armed,
                "{}: (state reads, flag loads) {armed:?} armed, pinned {pinned_armed:?}",
                scenario.title
            );
        }
    }

    /// The keyboard effect reads the shadow's per-thread write sequence only for a flight
    /// that is armed: a disarmed shadow reads none of its own state under D, even where a
    /// keyboard write happens (D-S2c-10 R4). R211 (an idle notice while recording) writes
    /// the keyboard under D; its armed run must read the sequence, or the scenario does not
    /// reach the keyboard effect at all.
    #[test]
    fn a_disarmed_keyboard_write_reads_no_shadow_state() {
        let scenario = bridge_scenarios()
            .into_iter()
            .find(|scenario| scenario.title == "bridge Idle while recording (R211)")
            .expect("the R211 scenario");
        run_measured(&scenario, false);
        assert!(
            SHADOW_READS_UNDER_D.with(Cell::get) > 0,
            "the armed R211 never read the keyboard write sequence: the scenario does not \
             reach the keyboard effect"
        );
        run_measured(&scenario, true);
        assert_eq!(
            SHADOW_READS_UNDER_D.with(Cell::get),
            0,
            "the disarmed shadow read its own state at a keyboard write under D"
        );
    }

    struct WarnUnderD;

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for WarnUnderD {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            // The refusal's own warning: it is emitted from inside the D section in the
            // default build, so it must be here too, armed or not (D-S2c-7 c).
            let text = format!("{event:?}");
            if text.contains("blocked") || text.contains("not recording") {
                let held = d_is_held();
                WARNED_UNDER_D.with(|seen| {
                    let mut all = seen.take();
                    all.push(held);
                    seen.set(all);
                });
            }
        }
    }

    fn warning_release_points(scenario: &Scenario, disarmed: bool) -> Vec<bool> {
        use tracing_subscriber::layer::SubscriberExt;
        WARNED_UNDER_D.with(|seen| seen.set(Vec::new()));
        REPLAY_DISARMED.store(disarmed, Ordering::SeqCst);
        struct Rearm;
        impl Drop for Rearm {
            fn drop(&mut self) {
                REPLAY_DISARMED.store(false, Ordering::SeqCst);
            }
        }
        let _rearm = Rearm;
        let subscriber = tracing_subscriber::Registry::default().with(WarnUnderD);
        tracing::subscriber::with_default(subscriber, || {
            // The measure helper points the probe at D for the run.
            run_sub_run(&SubRun {
                title: scenario.title,
                setup: Setup {
                    status: scenario.setup.status,
                    recording_id: scenario.setup.recording_id,
                },
                body: scenario.body,
            });
        });
        WARNED_UNDER_D.with(|seen| seen.take())
    }

    /// D-S2c-7 c: with the feature compiled in, a refusal that warns does so at the same
    /// lock boundary armed or not: inside the D section, as the default build does.
    #[test]
    fn a_refusal_keeps_its_d_release_point_armed_or_not() {
        let mut seen_any = false;
        for scenario in scenarios() {
            let baseline = warning_release_points(&scenario, true);
            let armed = warning_release_points(&scenario, false);
            assert_eq!(
                armed, baseline,
                "{}: the refusal's warning moved across the D release",
                scenario.title
            );
            seen_any |= !baseline.is_empty();
        }
        assert!(seen_any, "no refusal warned: the probe saw nothing");
    }
}

/// The production ports' ticker observation (D-S2c-9 f): the synchronous preflight the
/// ports take before the spawn and the spawner's own early return agree, and each skip
/// reason reaches the comparator as exactly one note. A disagreement notes nothing, so it
/// would fail the replay instead of being guessed.
#[cfg(feature = "dictation-shadow")]
mod ticker_skip_notes {
    use super::super::golden_tests::sub_from;
    use super::*;
    use crate::dictation_shadow::replay;
    use crate::dictation_shadow::types::TickerSkipReason;

    fn ready(world: &Arc<World>) {
        world.notify(1, AudioLifecycleEvent::Ready);
    }

    fn with_model(world: &Arc<World>, model: &str) {
        world.install_context(1);
        let state = world.state();
        let mut context = state
            .app_state
            .active_context(1)
            .expect("the fixture installed a context");
        Arc::make_mut(&mut context).transcription.model_name = model.to_string();
        state.app_state.set_active_context(1, context);
    }

    fn no_context(world: &Arc<World>) {
        ready(world);
    }
    fn no_live_preview(world: &Arc<World>) {
        with_model(world, "small.en");
        ready(world);
    }
    fn a_live_preview_model(world: &Arc<World>) {
        with_model(world, "base.en");
        ready(world);
    }

    /// A Core ML recording, reaching the ticker spawner through the lifecycle bridge's
    /// `Ready` (the cold start that finishes after the context is installed).
    fn a_coreml_model_through_the_bridge(world: &Arc<World>) {
        with_model(world, crate::transcriber::COREML_MODEL_NAME);
        ready(world);
        registered(world);
    }

    /// The harness's spawner (which makes the production check) ran the registration.
    fn registered(world: &Arc<World>) {
        assert!(
            world
                .trace()
                .contains(&"SD ticker 1 registered=true".to_string()),
            "the Core ML recording's ticker was not registered: {:?}",
            world.trace()
        );
    }

    /// A Core ML recording, reaching it through the start command's own ready entry (R104.2:
    /// the supervisor is already `Recording` when the context is installed).
    fn a_coreml_model_through_the_start_command(world: &Arc<World>) {
        world
            .state()
            .app_state
            .dictation
            .lock_or_recover()
            .model_name = crate::transcriber::COREML_MODEL_NAME.to_string();
        recording(world);
        registered(world);
    }

    fn skipped_by(title: &'static str, body: fn(&Arc<World>)) -> ([u64; 2], u64) {
        replay::reset_totals();
        repeat(&sub_from(title, DictationStatus::Starting, 1, body));
        let totals = replay::totals();
        (totals.ticker_skipped, totals.replays)
    }

    /// The unsupported-model outcome keeps the recorded context's `Arc` for the spawner's
    /// `unsupported_model` emit, as the original spawner's own binding does (round 9): the
    /// lookup's clone is not dropped when the check returns `Err`.
    #[test]
    fn an_unsupported_model_outcome_holds_the_snapshot_through_the_emit() {
        let app_state = AppState::default();
        let destination = select_recording_destination(
            &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
        );
        let mut context = resolve_recording_context(&app_state, destination);
        Arc::make_mut(&mut context).transcription.model_name = "small.en".to_string();
        app_state.set_active_context(1, context);
        match ticker_start_check(&app_state, 1) {
            Err((TickerSkipReason::NoLivePreview, Some(held))) => assert_eq!(
                Arc::strong_count(&held),
                2,
                "the app state's reference and the outcome's own"
            ),
            other => panic!(
                "expected the unsupported outcome with its snapshot: {:?}",
                other.err().map(|(reason, held)| (reason, held.is_some()))
            ),
        }
        // Nothing recorded: no snapshot to hold.
        let empty = AppState::default();
        assert!(matches!(
            ticker_start_check(&empty, 1),
            Err((TickerSkipReason::NoContext, None))
        ));
        // A supported model is the lookup's own pair.
        let mut context = app_state.active_context(1).unwrap();
        Arc::make_mut(&mut context).transcription.model_name = "base.en".to_string();
        app_state.set_active_context(1, context);
        assert!(matches!(ticker_start_check(&app_state, 1), Ok((_, true))));
        // The lookups above only wrote the skip mailbox (when armed): leave it empty.
        let _ = crate::dictation_executor::shadow::take_ticker_skip();
    }

    #[test]
    fn each_skip_reason_is_noted_exactly_once_per_replay() {
        for (reason, title, body) in [
            (
                TickerSkipReason::NoContext,
                "ticker skip: no recorded context",
                no_context as fn(&Arc<World>),
            ),
            (
                TickerSkipReason::NoLivePreview,
                "ticker skip: a model without a live preview",
                no_live_preview as fn(&Arc<World>),
            ),
        ] {
            let (skipped, replays) = skipped_by(title, body);
            assert!(replays >= 1, "{title}: nothing was replayed");
            let mut expected = [0; 2];
            expected[reason.index()] = replays;
            assert_eq!(
                skipped, expected,
                "{title}: exactly one {reason:?} note per replay, no other"
            );
        }
        let (skipped, replays) = skipped_by("ticker: a live preview model", a_live_preview_model);
        assert!(replays >= 1);
        assert_eq!(skipped, [0, 0], "no skip when the spawner registers");
    }

    /// A Core ML recording registers its ticker through **both** ready entry points
    /// (D-S2c-10 amendment 1 iii). The soak comparator accepts a `NoLivePreview` skip for any
    /// recording, so only a replay of the production check against a Core ML context can
    /// see a spawner that stops registering one: `if !whisper` (and a check that refuses
    /// Core ML) leaves a skip note where a registration is required.
    #[test]
    fn a_coreml_recording_registers_its_ticker_through_both_ready_entry_points() {
        // The start command's cold start reaches `Ready` before its context exists, so the
        // bridge's own spawner call is the one `NoContext` skip of that run; R104.2 then
        // registers.
        for (title, status, id, body, expected) in [
            (
                "ticker: a Core ML model through the bridge's Ready",
                DictationStatus::Starting,
                1,
                a_coreml_model_through_the_bridge as fn(&Arc<World>),
                [0, 0],
            ),
            (
                "ticker: a Core ML model through the start command (R104.2)",
                DictationStatus::Idle,
                0,
                a_coreml_model_through_the_start_command as fn(&Arc<World>),
                [1, 0],
            ),
        ] {
            replay::reset_totals();
            repeat(&sub_from(title, status, id, body));
            let totals = replay::totals();
            assert!(totals.replays >= 1, "{title}: nothing was replayed");
            let mut per_replay = expected;
            for count in &mut per_replay {
                *count *= totals.replays;
            }
            assert_eq!(
                totals.ticker_skipped, per_replay,
                "{title}: a Core ML recording must register its ticker, not skip it as \
                 `NoLivePreview`"
            );
        }
    }
}
