//! Stage 1 contract tests (C4.9) that pin the executor contract (C4) on
//! today's shell behaviour. They need no Stage 2 executor: each drives the
//! existing commands through the Stage 0a/0b harness, or the existing ports
//! (terminal ledger, performance begin ledger, interrupted-capture store)
//! directly. The tests C4.9 lists that need the Stage 2 executor, the op ids or
//! the `core_history` are deferred; `recording_pd_witness_map.md` lists each
//! with its reason.
//!
//! Ordering is by causal barriers only: no clock, no sleep.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{cancelled, recording, started, stopped, sub, sub_from};
use super::pd_witness_tests::{position, repeat};
use super::*;
use crate::dictation_telemetry::DictationTelemetry;

fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
}

fn terminal(
    world: &Arc<World>,
    recording_id: u64,
) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
    world
        .state()
        .app_state
        .dictation_telemetry
        .terminal_for_test(recording_id)
}

fn terminal_lines(trace: &[String]) -> usize {
    trace
        .iter()
        .filter(|line| line.starts_with("~ terminal "))
        .count()
}

// ------------------------------------------- terminal ledger (C4.5 T1, T2)

/// `terminal_first_wins_under_concurrency`: contenders released together by a
/// barrier race for one accepted id; exactly one claim succeeds and the ledger
/// holds that contender's outcome. Which contender wins is not predicted.
#[test]
fn terminal_first_wins_under_concurrency() {
    use std::sync::Barrier;
    let outcomes = [
        (DictationTerminalOutcome::Success, DictationErrorCode::None),
        (
            DictationTerminalOutcome::NoSpeech,
            DictationErrorCode::VadNoSpeech,
        ),
        (
            DictationTerminalOutcome::TooShort,
            DictationErrorCode::BelowMinimumDuration,
        ),
        (
            DictationTerminalOutcome::UserCancelledProcessing,
            DictationErrorCode::CancelledProcessing,
        ),
        (
            DictationTerminalOutcome::StopFailure,
            DictationErrorCode::StopFinalizationFailed,
        ),
        (
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::TranscriptionFailed,
        ),
        (
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner,
        ),
        (
            DictationTerminalOutcome::RuntimeInterruption,
            DictationErrorCode::RuntimeFailure,
        ),
    ];
    for _ in 0..repeat_count() {
        let telemetry = Arc::new(DictationTelemetry::default());
        telemetry.accepted(1);
        let barrier = Arc::new(Barrier::new(outcomes.len()));
        let contenders: Vec<_> = outcomes
            .into_iter()
            .map(|(outcome, code)| {
                let (telemetry, barrier) = (Arc::clone(&telemetry), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    telemetry
                        .emit_terminal_with_output(1, outcome, code, 0)
                        .then_some((outcome, code))
                })
            })
            .collect();
        let winners: Vec<_> = contenders
            .into_iter()
            .filter_map(|contender| contender.join().expect("contender"))
            .collect();
        assert_eq!(winners.len(), 1, "exactly one claim wins: {winners:?}");
        assert_eq!(telemetry.terminal_for_test(1), Some(winners[0]));
    }
}

/// `terminal_dropped_before_accepted` (C4.5 T2, PD-10): a terminal for an id
/// the ledger has not accepted is dropped, and does not poison a later accept.
#[test]
fn terminal_dropped_before_accepted() {
    for _ in 0..repeat_count() {
        let telemetry = DictationTelemetry::default();
        assert!(!telemetry.emit_terminal(
            7,
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::RuntimeFailure
        ));
        assert_eq!(telemetry.terminal_for_test(7), None);
        telemetry.accepted(7);
        assert!(telemetry.emit_terminal(
            7,
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        ));
    }
    // Through the shell: a stale stop for an id nothing ever accepted replies
    // superseded and emits no terminal.
    repeat(&sub_from(
        "stale stop for an unaccepted id",
        DictationStatus::Idle,
        2,
        |world| {
            world.install_context(2);
            stopped(world, "stale stop(expected=1)", Some(1));
            assert_eq!(terminal_lines(&world.trace()), 0);
            assert_eq!(terminal(world, 1), None);
        },
    ));
}

// ---------------------------- start vs supervisor (C4.5 T2, T3, scenario 12)

/// The start's 2 s timeout fails after the supervisor accepted it. When the
/// supervisor's `Cancel{User}` publishes first, it wins the ledger.
fn timeout_loses_to_the_supervisor_cancel(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SupervisorUnavailable);
        knobs.before_ack = vec![AudioLifecycleEventScript::Recovering(
            AudioCancelReason::User,
        )];
    }
    started(world);
    let trace = world.trace();
    assert_eq!(terminal_lines(&trace), 1);
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::UserCancelledStarting,
            DictationErrorCode::CancelledStarting
        ))
    );
    assert_eq!(status(world), DictationStatus::Idle);
}

/// The start's timeout path claims first; the supervisor's later `Cancel{User}`
/// finds Idle, takes the stale rule and loses.
fn timeout_wins_over_a_late_supervisor_cancel(world: &Arc<World>) {
    world.knobs().start_ack = Some(AudioStartError::SupervisorUnavailable);
    started(world);
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    world.notify(
        1,
        AudioLifecycleEvent::Recovering {
            reason: AudioCancelReason::User,
        },
    );
    assert_eq!(terminal_lines(&world.trace()), 1);
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    assert_eq!(status(world), DictationStatus::Idle);
}

/// `start_timeout_races_supervisor_cancel` (C4.5 T2): both orders, one terminal.
#[test]
fn start_timeout_races_supervisor_cancel() {
    repeat(&sub(
        "supervisor cancel first",
        timeout_loses_to_the_supervisor_cancel,
    ));
    repeat(&sub(
        "start timeout first",
        timeout_wins_over_a_late_supervisor_cancel,
    ));
}

/// The supervisor reports the failure (`InitializationFailed`, then `Idle`)
/// before it sends the start's `Err` (T3): the supervisor's terminal wins and
/// the start's own `TERM` and the `Idle` event's both lose.
fn initialization_failed_precedes_the_start_err(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SpawnFailed(
            "scripted start failure".to_string(),
        ));
        knobs.before_ack = vec![
            AudioLifecycleEventScript::InitializationFailed,
            AudioLifecycleEventScript::Idle,
        ];
    }
    started(world);
    let trace = world.trace();
    let failed = position(&trace, "~ terminal 1 CaptureInitFailure/BackendError");
    let ack = trace
        .iter()
        .position(|line| line.starts_with("supervisor.start_ack 1 Err("))
        .expect("the start ack is an Err");
    let reply = trace
        .iter()
        .position(|line| line.starts_with("reply start Err"))
        .expect("the start replies Err");
    assert!(
        failed < ack && ack < reply,
        "terminal, then Err ack, then reply"
    );
    assert_eq!(
        terminal_lines(&trace),
        1,
        "only the first claim is recorded"
    );
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::BackendError
        ))
    );
}

/// `init_failed_terminal_precedes_start_err` (T3).
#[test]
fn init_failed_terminal_precedes_start_err() {
    repeat(&sub(
        "InitializationFailed before the start Err",
        initialization_failed_precedes_the_start_err,
    ));
}

// -------------------------------------------- stale continuations (rule 3)

/// A start whose id went stale while it was parked in its context resolution
/// takes the start's stale rule: a `Superseded` terminal, the begin row aborted,
/// no context installed, reply `audio_recovering` carrying the live status.
fn stale_start_continuation(world: &Arc<World>) {
    world.close_gate("smart_capture");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    world.poke_recording_id(2);
    world.note("the recording id advanced while the start was parked");
    world.open_gate("smart_capture");
    let reply = complete(&mut start);
    world.reply("start", &reply);
    assert_eq!(
        reply,
        Ok(serde_json::json!({ "type": "audio_recovering", "state": "starting" }))
    );
    let trace = world.trace();
    let passed = position(&trace, "gate smart_capture pass");
    assert!(trace[passed..].contains(&"performance.begin_abort 1".to_string()));
    assert!(!trace.contains(&"performance.spawn_begin 1".to_string()));
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        ))
    );
    assert!(world.state().app_state.active_context(1).is_none());
}

/// Every supervisor notification for a stale id takes the arm's stale rule: a
/// `Superseded/StaleOwner` terminal for that id and no change to the current
/// recording's status, id or keyboard gate.
fn stale_supervisor_events(world: &Arc<World>) {
    let events: [(u64, AudioLifecycleEvent); 6] = [
        (1, AudioLifecycleEvent::Accepted),
        (2, AudioLifecycleEvent::Ready),
        (
            3,
            AudioLifecycleEvent::Recovering {
                reason: AudioCancelReason::User,
            },
        ),
        (
            4,
            AudioLifecycleEvent::InitializationFailed {
                error: "scripted".to_string(),
                kind: audio::AudioFailureKind::BackendError,
                recovery_reason: None,
            },
        ),
        (
            5,
            AudioLifecycleEvent::Interrupted {
                reason: AudioCancelReason::RuntimeFailure,
                delivered_samples: 9_000,
                duration_ms: 562,
            },
        ),
        (6, AudioLifecycleEvent::Idle),
    ];
    keyboard::set_processing(true);
    for (recording_id, event) in events {
        world.accept_silently(recording_id);
        world.notify(recording_id, event);
        assert_eq!(
            terminal(world, recording_id),
            Some((
                DictationTerminalOutcome::Superseded,
                DictationErrorCode::StaleOwner
            )),
            "stale id {recording_id}"
        );
        assert_eq!(status(world), DictationStatus::Idle);
        assert_eq!(
            world.state().app_state.recording_id.load(Ordering::SeqCst),
            9
        );
        assert!(keyboard::is_processing(), "the current gate is untouched");
    }
}

/// `continuation_stale_uses_site_rule`, on today's shell: each continuation
/// (the start's context resolution, every supervisor arm) applies its own stale
/// rule. (The stop's stale rule is golden scenario 6.)
#[test]
fn continuation_stale_uses_site_rule() {
    repeat(&sub("stale start continuation", stale_start_continuation));
    repeat(&sub_from(
        "stale supervisor events",
        DictationStatus::Idle,
        9,
        stale_supervisor_events,
    ));
}

// ------------------------------------------------ M3: cancelled_through

/// `cancel_window_preserved` (M3, C4.7), on today's shell: `cancelled_id` is a
/// high-water mark; a capture-phase cancel never sets it; a Processing cancel
/// sets it after `ST->Idle` and `CX-` and before the first port after the
/// tail; a later recording is not cancelled by it.
#[test]
fn cancel_window_preserved() {
    for _ in 0..repeat_count() {
        let app_state = AppState::default();
        assert!(!app_state.is_cancelled(1));
        app_state.cancel_recording(5);
        assert!((1..=5).all(|id| app_state.is_cancelled(id)));
        assert!(!app_state.is_cancelled(6));
        app_state.cancel_recording(3);
        assert!(app_state.is_cancelled(5), "the mark is never lowered");
    }
    repeat(&sub("cancel while Starting", |world| {
        world.knobs().cold_start = ColdStart::ReadyAfterContext;
        started(world);
        cancelled(world, "cancel");
        world.release_cancel_finish();
        assert!(!world.state().app_state.is_cancelled(1));
    }));
    repeat(&sub("cancel while Recording", |world| {
        recording(world);
        cancelled(world, "cancel");
        world.release_cancel_finish();
        assert!(!world.state().app_state.is_cancelled(1));
    }));
    repeat(&sub("cancel while Processing", |world| {
        world.knobs().stop_result = Ok(16_000);
        recording(world);
        world.close_gate("pipeline");
        let mut stop = world.stop(None);
        assert!(poll_once(&mut stop).is_pending());
        let observer = Arc::clone(world);
        world.on("keyboard.set_processing:false", move || {
            // The first port after the tail's ordered statements: ST->Idle,
            // CX- and MC have all happened, and the terminal is claimed.
            let state = observer.state();
            assert_eq!(
                state.app_state.dictation.lock_or_recover().status,
                DictationStatus::Idle
            );
            assert!(state.app_state.active_context(1).is_none());
            assert!(state.app_state.is_cancelled(1));
            assert_eq!(
                state.app_state.dictation_telemetry.terminal_for_test(1),
                Some((
                    DictationTerminalOutcome::UserCancelledProcessing,
                    DictationErrorCode::CancelledProcessing
                ))
            );
        });
        cancelled(world, "cancel");
        world.open_gate("pipeline");
        let reply = complete(&mut stop);
        world.reply("stop", &reply);
        world.finish_sealed_stop(1);
        recording(world);
        assert!(world.state().app_state.is_cancelled(1));
        assert!(!world.state().app_state.is_cancelled(2));
    }));
}

// -------------------------------------------------------------- C2.2 take

/// `take_attempts_iff_recovering` (C2.2), the stop half, on today's shell: the
/// destructive `take_interrupted_dictation` is attempted when status is
/// `Recovering`, and also before D on the stale-expected-id path (PD-15, a
/// discrepancy against C2.2's "iff Recovering"); never from Starting,
/// Recording, Processing or Idle. The start half (smart-auto resolution) is
/// deferred: `resolve_smart_auto` is not a port.
fn takes(world: &Arc<World>) -> usize {
    world
        .trace()
        .iter()
        .filter(|line| line.starts_with("supervisor.take_interrupted "))
        .count()
}

#[test]
fn take_attempts_iff_recovering() {
    repeat(&sub("stop while Starting", |world| {
        world.knobs().cold_start = ColdStart::ReadyAfterContext;
        started(world);
        stopped(world, "stop", None);
        world.release_cancel_finish();
        assert_eq!(takes(world), 0);
    }));
    repeat(&sub("stop while Recording", |world| {
        recording(world);
        stopped(world, "stop", None);
        assert_eq!(takes(world), 0);
    }));
    repeat(&sub("stop while Processing", |world| {
        world.knobs().stop_result = Ok(16_000);
        recording(world);
        world.close_gate("pipeline");
        let mut first = world.stop(None);
        assert!(poll_once(&mut first).is_pending());
        stopped(world, "second stop", None);
        assert_eq!(takes(world), 0);
        world.open_gate("pipeline");
        let _ = complete(&mut first);
    }));
    repeat(&sub("stop while Idle", |world| {
        stopped(world, "stop", None);
        assert_eq!(takes(world), 0);
    }));
    repeat(&sub_from(
        "stop while Recovering",
        DictationStatus::Recovering,
        1,
        |world| {
            stopped(world, "stop", None);
            assert_eq!(takes(world), 1);
            assert!(world
                .trace()
                .contains(&"supervisor.take_interrupted 1 -> none".to_string()));
        },
    ));
    repeat(&sub_from(
        "stop with a stale expected id",
        DictationStatus::Idle,
        2,
        |world| {
            stopped(world, "stale stop(expected=1)", Some(1));
            assert_eq!(takes(world), 1);
        },
    ));
}

/// `take_attempts_iff_recovering` (C4.9), the start half (decision D-S1-1): the
/// start's own destructive-or-costly observation, the Smart Auto resolve (a host
/// port since Stage 2a-4), is attempted exactly when the decision asks for it:
/// status `Idle`, no blocker, no retiring capture, no pre-D refusal. Never from
/// `Starting`, `Recording`, `Recovering` or `Processing`, never when the app is
/// disabled, never with a fixed microphone, and never for a request that did not
/// ask for Smart Auto.
fn smart_auto_start(world: &Arc<World>, device_name: Option<String>) -> usize {
    let mut start = world.start_smart_auto(device_name);
    let reply = complete(&mut start);
    world.reply("start", &reply);
    world.smart_auto_resolves()
}

#[test]
fn take_attempts_iff_recovering_start_half() {
    for status in [
        DictationStatus::Starting,
        DictationStatus::Recording,
        DictationStatus::Recovering,
        DictationStatus::Processing,
    ] {
        // One sub-run per status: a body is a plain `fn`, so the status is its
        // setup, and every one of them must never resolve.
        let sub_run = match status {
            DictationStatus::Starting => sub_from("start while Starting", status, 1, |world| {
                assert_eq!(smart_auto_start(world, None), 0);
            }),
            DictationStatus::Recording => sub_from("start while Recording", status, 1, |world| {
                assert_eq!(smart_auto_start(world, None), 0);
            }),
            DictationStatus::Recovering => sub_from("start while Recovering", status, 1, |world| {
                assert_eq!(smart_auto_start(world, None), 0);
            }),
            DictationStatus::Processing => sub_from("start while Processing", status, 1, |world| {
                assert_eq!(smart_auto_start(world, None), 0);
            }),
            DictationStatus::Idle => unreachable!("Idle is the resolving case"),
        };
        repeat(&sub_run);
    }
    repeat(&sub("start while Idle", |world| {
        world.knobs().smart_auto = Some(Err("no verified microphone".to_string()));
        assert_eq!(smart_auto_start(world, None), 1);
        assert!(world
            .trace()
            .iter()
            .any(|line| line == "reply start Err no verified microphone"));
        assert_eq!(status(world), DictationStatus::Idle);
    }));
    repeat(&sub("start with the app disabled", |world| {
        world.knobs().app_disabled = true;
        assert_eq!(smart_auto_start(world, None), 0);
    }));
    repeat(&sub("start with a fixed microphone", |world| {
        assert_eq!(smart_auto_start(world, Some("fixed-mic".to_string())), 0);
        assert!(world
            .trace()
            .iter()
            .any(|line| line.starts_with("reply start Err Smart Auto cannot be combined")));
    }));
    repeat(&sub("start without Smart Auto", |world| {
        started(world);
        assert_eq!(world.smart_auto_resolves(), 0);
    }));
}

// ----------------------------------------------- C4.6 perf begin ledger

fn completion() -> DictationPerformanceCompletion {
    DictationPerformanceCompletion {
        delivery_timing: None,
        outcome: RunOutcomeV1::Success,
        stages: Vec::new(),
        input: None,
        runtimes: None,
    }
}

#[derive(Clone, Copy, Debug)]
enum Column {
    Absent,
    /// Beginning with no completion, holding a latest stage.
    BeginningNoCompletion,
    /// Beginning with a completion held (and a latest stage).
    BeginningCompletionHeld,
    Ready,
    Completing,
}

const COLUMNS: [Column; 5] = [
    Column::Absent,
    Column::BeginningNoCompletion,
    Column::BeginningCompletionHeld,
    Column::Ready,
    Column::Completing,
];

fn established(column: Column) -> DictationPerformanceBeginCoordinator {
    let coordinator = DictationPerformanceBeginCoordinator::default();
    let stage = PerformanceStageV1::CaptureFinalization;
    match column {
        Column::Absent => {}
        Column::BeginningNoCompletion => {
            coordinator.register(1);
            assert!(coordinator.defer_stage(1, stage));
        }
        Column::BeginningCompletionHeld => {
            coordinator.register(1);
            assert!(coordinator.defer_stage(1, stage));
            assert_eq!(
                action_name(coordinator.request_completion(1, completion())),
                "Deferred"
            );
        }
        Column::Ready => {
            coordinator.register(1);
            assert!(coordinator.resolve_begin(1, true).is_some());
        }
        Column::Completing => {
            assert_eq!(
                action_name(coordinator.request_completion(1, completion())),
                "Persist"
            );
        }
    }
    coordinator
}

fn action_name(action: DictationPerformanceCompletionAction) -> &'static str {
    match action {
        DictationPerformanceCompletionAction::Persist(_) => "Persist",
        DictationPerformanceCompletionAction::Deferred => "Deferred",
        DictationPerformanceCompletionAction::AlreadyCompleting => "AlreadyCompleting",
    }
}

fn snapshot(coordinator: &DictationPerformanceBeginCoordinator) -> String {
    match coordinator.entries.lock_or_recover().get(&1) {
        None => "absent".to_string(),
        Some(DictationPerformanceBeginState::Beginning {
            completion,
            latest_stage,
        }) => format!(
            "beginning completion={} stage={}",
            completion.is_some(),
            latest_stage.is_some()
        ),
        Some(DictationPerformanceBeginState::Ready) => "ready".to_string(),
        Some(DictationPerformanceBeginState::Completing(_)) => "completing".to_string(),
    }
}

fn resolution_name(resolution: Option<DictationPerformanceBeginResolution>) -> String {
    match resolution {
        None => "None".to_string(),
        Some(resolution) => format!(
            "Some(completion={} stage={})",
            resolution.completion.is_some(),
            resolution.latest_stage.is_some()
        ),
    }
}

/// `perf_ledger_table` (C4.6, every cell): each event applied to each state,
/// with the value it returns and the state it leaves.
#[test]
fn perf_ledger_table() {
    type Event = fn(&DictationPerformanceBeginCoordinator) -> String;
    type Row = (&'static str, Event, [(&'static str, &'static str); 5]);
    let register: Event = |c| {
        c.register(1);
        "-".to_string()
    };
    let abort: Event = |c| {
        c.abort(1);
        "-".to_string()
    };
    let request_completion: Event =
        |c| action_name(c.request_completion(1, completion())).to_string();
    let resolve_ok: Event = |c| resolution_name(c.resolve_begin(1, true));
    let resolve_fail: Event = |c| resolution_name(c.resolve_begin(1, false));
    let finish: Event = |c| {
        c.finish_completion(1);
        "-".to_string()
    };
    let defer_stage: Event = |c| {
        c.defer_stage(1, PerformanceStageV1::InferenceDecode)
            .to_string()
    };

    const FRESH: &str = "beginning completion=false stage=false";
    // (event, [(returned, state) for Absent, Beginning, Beginning+held, Ready, Completing])
    let table: [Row; 7] = [
        (
            "register (overwrites, drops a held stage or completion)",
            register,
            [
                ("-", FRESH),
                ("-", FRESH),
                ("-", FRESH),
                ("-", FRESH),
                ("-", FRESH),
            ],
        ),
        (
            "abort",
            abort,
            [
                ("-", "absent"),
                ("-", "absent"),
                ("-", "absent"),
                ("-", "absent"),
                ("-", "absent"),
            ],
        ),
        (
            "request_completion",
            request_completion,
            [
                ("Persist", "completing"),
                ("Deferred", "beginning completion=true stage=true"),
                ("AlreadyCompleting", "beginning completion=true stage=true"),
                ("Persist", "completing"),
                ("AlreadyCompleting", "completing"),
            ],
        ),
        (
            "resolve_begin(ok)",
            resolve_ok,
            [
                ("None", "absent"),
                ("Some(completion=false stage=true)", "ready"),
                ("Some(completion=true stage=true)", "completing"),
                ("None", "ready"),
                ("None", "completing"),
            ],
        ),
        (
            "resolve_begin(fail)",
            resolve_fail,
            [
                ("None", "absent"),
                ("None", "absent"),
                ("None", "absent"),
                ("None", "ready"),
                ("None", "completing"),
            ],
        ),
        (
            "finish_completion",
            finish,
            [
                ("-", "absent"),
                ("-", "beginning completion=false stage=true"),
                ("-", "beginning completion=true stage=true"),
                ("-", "ready"),
                ("-", "absent"),
            ],
        ),
        (
            "defer_stage",
            defer_stage,
            [
                ("false", "absent"),
                ("true", "beginning completion=false stage=true"),
                ("true", "beginning completion=true stage=true"),
                ("false", "ready"),
                ("true", "completing"),
            ],
        ),
    ];
    for _ in 0..repeat_count() {
        for (name, event, expected) in &table {
            for (column, (returned, state)) in COLUMNS.into_iter().zip(expected) {
                let coordinator = established(column);
                let got = event(&coordinator);
                assert_eq!(
                    (got.as_str(), snapshot(&coordinator).as_str()),
                    (*returned, *state),
                    "{name} on {column:?}"
                );
            }
        }
    }
}

/// A held completion is flushed by `resolve_begin(ok)` (the early-terminal
/// rule of C4.6), and a failed begin loses it.
#[test]
fn perf_ledger_early_terminal_is_flushed_or_lost() {
    for _ in 0..repeat_count() {
        let coordinator = established(Column::BeginningCompletionHeld);
        let resolution = coordinator
            .resolve_begin(1, true)
            .expect("the begin resolved");
        assert!(resolution.completion.is_some());
        assert!(coordinator.completion_is_claimed(1));
        let coordinator = established(Column::BeginningCompletionHeld);
        assert!(coordinator.resolve_begin(1, false).is_none());
        assert!(!coordinator.begin_is_pending(1), "the completion is lost");
    }
}

// ------------------------------- Stage 2a PR 0: core_history (C1.4.1, C4.3)
//
// These pin the operation history before any decide function uses it. The
// per-row halves ("every consuming row yields identical effects for
// `Outstanding` and `Unknown`") are asserted by each family PR's row tests; here
// the shell protocol that makes that possible is pinned with a stand-in site
// rule that, like every real stale rule, ignores the op status.

mod core_history_contract {
    use crate::core_history::{
        assert_not_held, is_held_on_this_thread, ContinuationGuard, CoreHistoryLock, Retired,
    };
    use murmur_dictation_core::decision::{ContKind, Continuation, ContinuationSpec, OpStatus};
    use murmur_dictation_core::ids::{OperationId, RecordingId};
    use murmur_dictation_core::state::DictationCore;
    use std::cell::Cell;

    const ALL_KINDS: [ContKind; 8] = [
        ContKind::CaptureStart,
        ContKind::ResolveContext,
        ContKind::ReadyPublish,
        ContKind::CaptureCancel,
        ContKind::StopContext,
        ContKind::CaptureStop,
        ContKind::Pipeline,
        ContKind::AutoStop,
    ];

    fn spec(recording: u64, kind: ContKind) -> ContinuationSpec {
        ContinuationSpec {
            recording_id: RecordingId(recording),
            kind,
        }
    }

    /// A stand-in for a site's stale rule: the effects it returns do not depend
    /// on the op status (C1.4.1 rule 3).
    fn site_rule(_status: OpStatus) -> Vec<&'static str> {
        vec!["TERM", "PC"]
    }

    /// The production half of `core_history.rs` (before its test module).
    fn production_source() -> &'static str {
        let source = include_str!("../core_history.rs");
        source
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .expect("core_history.rs has a production half")
    }

    fn forged_tokens(legit: Continuation) -> [(&'static str, Continuation); 4] {
        let other_kind = if legit.kind == ContKind::CaptureStop {
            ContKind::CaptureStart
        } else {
            ContKind::CaptureStop
        };
        [
            (
                "wrong kind",
                Continuation {
                    kind: other_kind,
                    ..legit
                },
            ),
            (
                "wrong recording",
                Continuation {
                    recording_id: RecordingId(legit.recording_id.0 + 1),
                    ..legit
                },
            ),
            (
                "forged op id",
                Continuation {
                    op: OperationId(u64::MAX),
                    ..legit
                },
            ),
            (
                "retired op",
                Continuation {
                    op: OperationId(u64::MAX - 1),
                    ..legit
                },
            ),
        ]
    }

    /// `continuation_unknown_forged_kind_takes_site_rule`: a retired, forged,
    /// wrong-kind or wrong-recording token still runs the site's rule with the
    /// same effects as an outstanding one, is counted, and retires nothing else.
    #[test]
    fn continuation_unknown_forged_kind_takes_site_rule() {
        let history = CoreHistoryLock::default();
        let legit = history.reg(spec(4, ContKind::CaptureStart));
        let outstanding_effects =
            history.consume(history.reg(spec(4, ContKind::CaptureStart)), site_rule);
        for (what, token) in forged_tokens(legit) {
            let ran = Cell::new(0);
            let counted_before = history.counters().unknown_op;
            let effects = history.consume(token, |status| {
                ran.set(ran.get() + 1);
                assert_eq!(status, OpStatus::Unknown, "{what}");
                site_rule(status)
            });
            assert_eq!(ran.get(), 1, "{what}: the site's rule runs exactly once");
            assert_eq!(effects, outstanding_effects, "{what}: same effects");
            assert!(history.counters().unknown_op > counted_before, "{what}");
            assert!(
                history.is_outstanding(legit),
                "{what} must not retire another op"
            );
        }
        // An already-retired token behaves the same way.
        let retired = history.reg(spec(5, ContKind::Pipeline));
        assert_eq!(history.ret_if_matches(retired), Retired::Removed);
        let effects = history.consume(retired, |status| {
            assert_eq!(status, OpStatus::Unknown);
            site_rule(status)
        });
        assert_eq!(effects, outstanding_effects);
    }

    /// `unknown_op_runs_site_stale_rule`: for every continuation kind the shell
    /// protocol runs the site's decide exactly once whatever the status, with no
    /// history lock held, and retires the op afterwards. (The per-row half lives
    /// in each family PR's row tests.)
    #[test]
    fn unknown_op_runs_site_stale_rule() {
        let history = CoreHistoryLock::default();
        for kind in ALL_KINDS {
            let outstanding = history.reg(spec(7, kind));
            let unknown = Continuation {
                op: OperationId(u64::MAX),
                ..outstanding
            };
            let mut results = Vec::new();
            for token in [outstanding, unknown] {
                let ran = Cell::new(0);
                let effects = history.consume(token, |status| {
                    ran.set(ran.get() + 1);
                    assert!(
                        !is_held_on_this_thread(),
                        "{kind:?}: decide runs with no core_history lock held"
                    );
                    (status, site_rule(status))
                });
                assert_eq!(ran.get(), 1, "{kind:?}: the rule is never skipped");
                results.push(effects);
            }
            assert_eq!(results[0].0, OpStatus::Outstanding, "{kind:?}");
            assert_eq!(results[1].0, OpStatus::Unknown, "{kind:?}");
            assert_eq!(results[0].1, results[1].1, "{kind:?}: identical effects");
            assert!(!history.is_outstanding(outstanding), "{kind:?}: consumed");
        }
        assert_eq!(history.outstanding_ops(), 0);
    }

    /// `wrong_kind_token_does_not_retire_legit_op`: right op id and recording id,
    /// wrong kind. The site's rule runs as for `Outstanding`, `unknown_op`
    /// increments, the entry stays, and the legitimate completion that follows
    /// consumes it normally.
    #[test]
    fn wrong_kind_token_does_not_retire_legit_op() {
        let history = CoreHistoryLock::default();
        let legit = history.reg(spec(9, ContKind::CaptureStart));
        let wrong = Continuation {
            kind: ContKind::CaptureStop,
            ..legit
        };
        let before = history.counters().unknown_op;
        let effects = history.consume(wrong, |status| {
            assert_eq!(status, OpStatus::Unknown);
            site_rule(status)
        });
        assert_eq!(effects, site_rule(OpStatus::Outstanding));
        // `validate` counted the observation and `ret_if_matches` the mismatch.
        assert_eq!(history.counters().unknown_op, before + 2);
        assert!(history.is_outstanding(legit), "the entry stays");
        assert_eq!(history.ret_if_matches(wrong), Retired::Mismatch);
        assert_eq!(history.outstanding_ops(), 1);
        let effects = history.consume(legit, |status| {
            assert_eq!(status, OpStatus::Outstanding);
            site_rule(status)
        });
        assert_eq!(effects, site_rule(OpStatus::Outstanding));
        assert_eq!(history.outstanding_ops(), 0, "the legitimate op completed");
    }

    /// `ret_if_matches_is_one_lock`: op id, recording id and kind are compared
    /// inside one `core_history` critical section (a lock-count assertion plus
    /// a grep of the source).
    #[test]
    fn ret_if_matches_is_one_lock() {
        let history = CoreHistoryLock::default();
        let token = history.reg(spec(2, ContKind::CaptureStop));
        let wrong_kind = Continuation {
            kind: ContKind::CaptureStart,
            ..token
        };
        let wrong_recording = Continuation {
            recording_id: RecordingId(3),
            ..token
        };
        for (probe, expected) in [
            (wrong_kind, Retired::Mismatch),
            (wrong_recording, Retired::Mismatch),
            (token, Retired::Removed),
            (token, Retired::Absent),
        ] {
            let before = history.acquisitions();
            assert_eq!(history.ret_if_matches(probe), expected);
            assert_eq!(history.acquisitions() - before, 1, "{expected:?}: one lock");
        }
        let source = production_source();
        let start = source
            .find("pub(crate) fn ret_if_matches(&self")
            .expect("the locked entry point");
        let body = &source[start..];
        let body = &body[..body.find("\n    }\n").expect("end of fn")];
        assert_eq!(body.matches("critical(").count(), 1, "{body}");
    }

    /// `outstanding_op_never_evicted`: advancing `current_id` past 100 outstanding
    /// ops evicts none; each leaves only by consumption or its owner's drop. The
    /// thresholds are diagnostic: they count and never change a decision.
    #[test]
    fn outstanding_op_never_evicted() {
        let history = CoreHistoryLock::default();
        history.allow_over_threshold_for_test();
        let tokens: Vec<Continuation> = (1..=100)
            .map(|recording| history.reg(spec(recording, ContKind::AutoStop)))
            .collect();
        for current in 1..=120 {
            let core = DictationCore {
                current_id: RecordingId(current),
                ..DictationCore::default()
            };
            let _ = history.snapshot_for_occupancy(&core);
            assert_eq!(history.outstanding_ops(), 100, "current_id = {current}");
        }
        assert_eq!(history.high_water().ops_total, 100);
        assert!(history.counters().over_threshold > 0);
        for token in &tokens[..50] {
            history.consume(*token, site_rule);
        }
        assert_eq!(history.outstanding_ops(), 50);
        assert!(tokens[50..]
            .iter()
            .all(|token| history.is_outstanding(*token)));
        let guards: Vec<ContinuationGuard> = tokens[50..]
            .iter()
            .map(|token| {
                ContinuationGuard::register(&history, spec(token.recording_id.0, token.kind))
            })
            .collect();
        assert_eq!(history.outstanding_ops(), 100);
        drop(guards);
        assert_eq!(
            history.outstanding_ops(),
            50,
            "owner drop retires its own ops only"
        );
    }

    /// `owner_drop_retires_op`: a dropped future, a panic unwind and a spawned
    /// task that never runs each retire the op through the owner's `Drop`; a
    /// consumed guard leaves nothing to retire.
    #[test]
    fn owner_drop_retires_op() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};

        // (1) A future dropped mid-flight.
        let history = CoreHistoryLock::default();
        let guard = ContinuationGuard::register(&history, spec(1, ContKind::CaptureStart));
        let mut flight = Box::pin(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(flight.as_mut().poll(&mut cx), Poll::Pending));
        assert_eq!(history.outstanding_ops(), 1, "outstanding while in flight");
        drop(flight);
        assert_eq!(history.outstanding_ops(), 0, "dropped future");

        // (2) A panic unwind.
        let guard = ContinuationGuard::register(&history, spec(2, ContKind::CaptureStop));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = guard;
            panic!("effect panicked");
        }));
        assert!(unwound.is_err());
        assert_eq!(history.outstanding_ops(), 0, "panic unwind");

        // (3) A spawned task that never runs (`RS`): the runtime is dropped first.
        let guard = ContinuationGuard::register(&history, spec(3, ContKind::AutoStop));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let task = runtime.spawn(async move {
            let _guard = guard;
        });
        assert_eq!(history.outstanding_ops(), 1, "the task never ran");
        drop(task);
        drop(runtime);
        assert_eq!(history.outstanding_ops(), 0, "spawned task never run");

        // (4) A consumed guard retires once, at consumption; its drop is silent.
        let mut guard = ContinuationGuard::register(&history, spec(4, ContKind::Pipeline));
        let duplicates = history.counters().duplicate_op;
        guard.consume(site_rule);
        assert_eq!(history.outstanding_ops(), 0);
        drop(guard);
        assert_eq!(history.counters().duplicate_op, duplicates);

        // (5) A panic inside the consuming decide: the guard is not disarmed
        // until consumption and retirement returned, so the owner's `Drop`
        // still retires the op (no eviction means it would leak forever).
        // (5a) the owner outlives the panic.
        let mut guard = ContinuationGuard::register(&history, spec(5, ContKind::CaptureStop));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard.consume(|_| -> () { panic!("decide panicked") })
        }));
        assert!(unwound.is_err());
        assert_eq!(
            history.outstanding_ops(),
            1,
            "not retired by the panicking consume"
        );
        drop(guard);
        assert_eq!(
            history.outstanding_ops(),
            0,
            "panic in decide, owner dropped"
        );
        // (5b) the owner is unwound with the panic.
        let mut guard = ContinuationGuard::register(&history, spec(6, ContKind::ResolveContext));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            guard.consume(|_| -> () { panic!("decide panicked") })
        }));
        assert!(unwound.is_err());
        assert_eq!(
            history.outstanding_ops(),
            0,
            "panic in decide, owner unwound"
        );
        // The successful consumption above still retires exactly once.
        assert_eq!(history.counters().duplicate_op, duplicates);
    }

    /// `history_lock_is_leaf`: the lock is only ever taken inside one short
    /// critical section that runs nothing of the caller's; it is taken after D
    /// and never held across another lock, an await or an effect.
    #[test]
    fn history_lock_is_leaf() {
        // Grep: one lock site, no async fn, no other lock, no guard escaping.
        let code: String = production_source()
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let source = code.as_str();
        assert_eq!(
            source.matches(".lock_or_recover()").count() + source.matches(".lock()").count(),
            1,
            "only `critical` locks the mutex"
        );
        assert!(!source.contains("async fn"), "no method awaits");
        assert!(!source.contains(".await"));
        for forbidden in ["MutexGuard", "recording_transition", "dictation.lock"] {
            assert!(
                !source.contains(forbidden),
                "core_history.rs must not mention `{forbidden}`"
            );
        }
        // The order D -> core_history works on one thread.
        let history = CoreHistoryLock::default();
        let dictation = std::sync::Mutex::new(());
        {
            let _d = dictation.lock().unwrap();
            let token = history.reg(spec(1, ContKind::CaptureStart));
            assert_eq!(history.validate(token), OpStatus::Outstanding);
            assert_eq!(history.ret_if_matches(token), Retired::Removed);
            assert!(
                !is_held_on_this_thread(),
                "released before the next statement"
            );
        }
        // Lock-order assertion: code that must not run under the leaf lock
        // refuses to, and a nested history entry is refused rather than deadlocking.
        assert_not_held();
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            history.hold_for_test(assert_not_held);
        }));
        assert!(refused.is_err(), "an effect under core_history is refused");
        let nested = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            history.hold_for_test(|| history.outstanding_ops());
        }));
        assert!(nested.is_err(), "the lock is not re-entrant");
        assert!(
            !is_held_on_this_thread(),
            "the flag is cleared by the unwind"
        );
        assert_eq!(
            history.outstanding_ops(),
            0,
            "the lock is usable afterwards"
        );
    }
}

// ------------------ Stage 2a PR 0: field adapter and PersistedCore (C1.4.2)
//
// Pre-cutover: the persisted core is written only by the adapter's `ST` and
// `ID+` commits, M3 and M4 only ever live, and every entry observes them from
// the live slots exactly as `DictationCore::project` does.

mod field_adapter_contract {
    use crate::dictation_admission::core_status;
    use crate::dictation_executor::FieldAdapter;
    use crate::state::{AppState, DictationState, DictationStatus};
    use crate::MutexExt;
    use murmur_dictation_core::ids::RecordingId;
    use murmur_dictation_core::state::{DictationCore, LiveObs, PersistedCore};
    use murmur_dictation_core::status::Status;
    use std::sync::Arc;

    fn snapshot() -> Arc<crate::dictation_context::DictationContextSnapshot> {
        let settings = DictationState::default();
        Arc::new(crate::dictation_context::resolve(
            crate::dictation_context::ResolverInputs {
                bundle_id: None,
                global: &settings,
                prompt: None,
                correction_matcher: None,
                ide_context_index: None,
                vocabulary_version: 0,
            },
        ))
    }

    fn persisted(app: &AppState) -> PersistedCore {
        app.dictation.lock_or_recover().core
    }

    /// `persisted_core_has_only_m1_m2`: `PersistedCore` has exactly `status` and
    /// `current_id`; running `MC` and `CX±` through the adapter leaves
    /// `DictationState.core` unchanged while the live M3/M4 move; `ST` and `ID+`
    /// move the persisted copy together with the live field.
    #[test]
    fn persisted_core_has_only_m1_m2() {
        // Shape: two fields, nothing else (the fields are private to the crate).
        assert_eq!(
            std::mem::size_of::<PersistedCore>(),
            std::mem::size_of::<(Status, RecordingId)>()
        );
        assert_eq!(
            format!("{:?}", PersistedCore::default()),
            "PersistedCore { status: Idle, current_id: RecordingId(0) }"
        );

        let app = AppState::default();
        let before = persisted(&app);
        FieldAdapter::cancel_through(&app, 7);
        FieldAdapter::set_context(&app, 7, snapshot());
        assert!(app.active_context(7).is_some(), "live M4 written");
        assert_eq!(
            app.cancelled_id.load(std::sync::atomic::Ordering::SeqCst),
            7,
            "live M3 written"
        );
        assert_eq!(persisted(&app), before, "MC and CX+ never touch the core");
        assert!(FieldAdapter::clear_context(&app, 7));
        assert!(!FieldAdapter::clear_context(&app, 7), "stale-safe");
        assert_eq!(persisted(&app), before, "CX- never touches the core");

        // `ST` and `ID+` commit M1 and M2 with their live fields.
        let mut dictation = app.dictation.lock_or_recover();
        let id = FieldAdapter::alloc_id(&app, &mut dictation);
        assert_eq!(dictation.core.current_id(), RecordingId(id));
        assert_eq!(dictation.core.status(), Status::Idle, "ID+ commits M2 only");
        let from = FieldAdapter::set_status(id, &mut dictation, DictationStatus::Starting);
        assert_eq!(from, DictationStatus::Idle);
        assert_eq!(dictation.status, DictationStatus::Starting);
        assert_eq!(dictation.core.status(), Status::Starting);
        assert_eq!(
            dictation.core.current_id(),
            RecordingId(app.recording_id.load(std::sync::atomic::Ordering::SeqCst))
        );
    }

    /// `m3_m4_observed_before_and_after_cutover`: at every step the view built
    /// the legacy way (`project()` from the live M1/M2) and the view built the
    /// cut-over way (the persisted core) hold the same M3/M4 observations, read
    /// from the live slots, and the persisted core does not change when they do.
    /// (The lifecycle family's entries are cut over: they build the persisted-core
    /// view through `FieldAdapter::lifecycle_view`, covered by
    /// `lifecycle_cutover_tests`; the persisted-core constructor it uses is exercised
    /// here.)
    #[test]
    fn m3_m4_observed_before_and_after_cutover() {
        let app = AppState::default();
        let views = |app: &AppState| {
            let dictation = app.dictation.lock_or_recover();
            let obs = FieldAdapter::observe_live(app);
            let legacy = DictationCore::project(
                core_status(dictation.status),
                RecordingId(app.recording_id.load(std::sync::atomic::Ordering::SeqCst)),
                obs,
            );
            (legacy, DictationCore::new(dictation.core, obs), obs)
        };
        let (legacy, cut_over, obs) = views(&app);
        assert_eq!(obs, LiveObs::default());
        assert_eq!(legacy, cut_over);

        let id = {
            let mut dictation = app.dictation.lock_or_recover();
            let id = FieldAdapter::alloc_id(&app, &mut dictation);
            FieldAdapter::set_status(id, &mut dictation, DictationStatus::Recording);
            id
        };
        let core_before = persisted(&app);

        FieldAdapter::set_context(&app, id, snapshot());
        let (legacy, cut_over, obs) = views(&app);
        assert_eq!(obs.context_for, Some(RecordingId(id)), "M4 observed live");
        assert_eq!(legacy, cut_over);

        FieldAdapter::cancel_through(&app, id);
        let (legacy, cut_over, obs) = views(&app);
        assert_eq!(obs.cancelled_through, RecordingId(id), "M3 observed live");
        assert!(cut_over.is_cancelled(RecordingId(id)));
        assert_eq!(legacy, cut_over);

        // A stale clear leaves the observation; the owning clear removes it.
        assert!(!FieldAdapter::clear_context(&app, id + 1));
        assert_eq!(views(&app).2.context_for, Some(RecordingId(id)));
        assert!(FieldAdapter::clear_context(&app, id));
        let (legacy, cut_over, obs) = views(&app);
        assert_eq!(obs.context_for, None);
        assert_eq!(legacy, cut_over);
        assert_eq!(persisted(&app), core_before, "M3/M4 never move the core");
    }

    /// `seed_core` is the way a core is made equal to live without an effect.
    #[test]
    fn seed_core_copies_live_m1_m2() {
        let app = AppState::default();
        app.recording_id
            .store(41, std::sync::atomic::Ordering::SeqCst);
        let mut dictation = app.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
        FieldAdapter::seed_core(&app, &mut dictation);
        assert_eq!(dictation.core.status(), Status::Processing);
        assert_eq!(dictation.core.current_id(), RecordingId(41));
    }

    /// The persisted core is never serialized: settings round trips and the
    /// frontend payloads are unchanged.
    #[test]
    fn persisted_core_is_not_serialized() {
        let value = serde_json::to_value(DictationState::default()).expect("serialize");
        assert!(value.get("core").is_none(), "{value}");
        let round_trip: DictationState = serde_json::from_value(value).expect("deserialize");
        assert_eq!(round_trip.core, PersistedCore::default());
    }
}

// ------------------------- Stage 2a PR 0: the executor skeleton (C4.1, C4.2)
//
// Driven with fake ports and a real `AppState` (so the field effects commit
// through the real adapter). No production path calls the executor yet.

mod executor_contract {
    use crate::core_history::ContinuationGuard;
    use crate::dictation_admission::core_status;
    use crate::dictation_context::DictationContextSnapshot;
    use crate::dictation_executor::executor::{
        decide_guarded, in_core, AwaitedEffect, AwaitedOutcome, CoreEntry, Executor, ExecutorError,
        PhaseCtx, PhaseReport, PortFailure, PortResult, Ports, WitnessOutcome,
    };
    use crate::state::{AppState, DictationStatus};
    use crate::MutexExt;
    use murmur_dictation_core::decision::{
        ContKind, ContinuationSpec, Decision, InvalidKind, OpStatus, RowId, Verdict,
    };
    use murmur_dictation_core::effects::{
        AxMode, Cond, Effect, EventKind, Placed, Placement, PresentationKind, Terminal,
    };
    use murmur_dictation_core::ids::RecordingId;
    use murmur_dictation_core::status::{ErrCode, Status, TerminalOutcome};
    use std::cell::{Cell, RefCell};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::rc::Rc;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use Placement::{In, PostDT, PostT, Pre};

    /// Recording lock state seen from inside a port call.
    #[derive(Clone, Copy, Debug)]
    struct Probe {
        code: &'static str,
        d_free: bool,
        t_free: bool,
        history_held: bool,
    }

    /// An inline `Rust` listener that runs under an emit (C4.2).
    type EmitListener = Box<dyn Fn(&FakePorts)>;

    struct FakePorts {
        app: AppState,
        log: RefCell<Vec<&'static str>>,
        probes: RefCell<Vec<Probe>>,
        fail_emits: Cell<bool>,
        speculative: Cell<bool>,
        panic_at: Cell<Option<&'static str>>,
        on_emit: RefCell<Option<EmitListener>>,
    }

    impl FakePorts {
        fn new() -> Self {
            Self {
                app: AppState::default(),
                log: RefCell::new(Vec::new()),
                probes: RefCell::new(Vec::new()),
                fail_emits: Cell::new(false),
                speculative: Cell::new(false),
                panic_at: Cell::new(None),
                on_emit: RefCell::new(None),
            }
        }

        fn hit(&self, effect: &Effect) {
            let code = effect.code();
            self.probes.borrow_mut().push(Probe {
                code,
                d_free: self.app.dictation.try_lock().is_ok(),
                t_free: self.app.recording_transition.try_lock().is_ok(),
                history_held: crate::core_history::is_held_on_this_thread(),
            });
            self.log.borrow_mut().push(code);
            if self.panic_at.get() == Some(code) {
                panic!("injected panic in {code}");
            }
        }

        fn log(&self) -> Vec<&'static str> {
            self.log.borrow().clone()
        }
    }

    impl Ports for FakePorts {
        fn app_state(&self) -> &AppState {
            &self.app
        }
        fn speculative_flowing(&self) -> bool {
            self.speculative.get()
        }
        fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
            None
        }
        fn emit(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            if let Some(listener) = self.on_emit.borrow().as_ref() {
                listener(self);
            }
            if self.fail_emits.get() {
                Err(PortFailure)
            } else {
                Ok(())
            }
        }
        fn trace(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn terminal(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn perf(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn keyboard(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn claims(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn blocking(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
        fn detached(&self, effect: &Effect) -> PortResult {
            self.hit(effect);
            Ok(())
        }
    }

    fn at(placement: Placement, effect: Effect) -> Placed {
        Placed::new(placement, effect)
    }

    fn decision(row: &'static str, effects: Vec<Placed>) -> Decision {
        let transition = effects.iter().find_map(|placed| match placed.effect {
            Effect::SetStatus(to) => Some((Status::Idle, to)),
            _ => None,
        });
        Decision {
            verdict: Verdict::Applied,
            transition,
            effects,
            continuation: None,
            reply: None,
            row: RowId(row),
        }
    }

    /// An R101.15-shaped start decision (C3.1): `ID+ GA PCC TRQ ST AX` under D,
    /// then the post-D effects ending in `CS`, then `KB` after T.
    fn start_decision() -> Decision {
        let rid = RecordingId(1);
        decision(
            "R101.15",
            vec![
                at(In, Effect::AllocId),
                at(In, Effect::GestureAccepted),
                at(In, Effect::PrepareCaptureClaimed),
                at(In, Effect::TraceRequested),
                at(In, Effect::SetStatus(Status::Starting)),
                at(In, Effect::AxSnapshot(AxMode::Accept)),
                at(PostDT, Effect::SpeculativeConsume),
                at(PostDT, Effect::EmitEvent(EventKind::GenStarted)),
                at(
                    PostDT,
                    Effect::EmitStatus {
                        status: Status::Starting,
                        cond: Cond::UnlessSpeculativeFlowing,
                    },
                ),
                at(PostDT, Effect::PerfRegister),
                at(PostDT, Effect::CaptureStart(rid)),
                at(PostT, Effect::KeyboardProcessing(false)),
            ],
        )
    }

    /// A decision with effects in every placement.
    fn four_phase_decision() -> Decision {
        decision(
            "R999.1",
            vec![
                at(Pre, Effect::ClearContext(RecordingId(1))),
                at(
                    Pre,
                    Effect::TakeInterrupted {
                        id: RecordingId(1),
                        mode: murmur_dictation_core::effects::TakeMode::Dispose,
                    },
                ),
                at(In, Effect::SetStatus(Status::Idle)),
                at(In, Effect::KeyboardProcessing(false)),
                at(
                    PostDT,
                    Effect::Terminal(Terminal::new(
                        TerminalOutcome::Superseded,
                        ErrCode::StaleOwner,
                    )),
                ),
                at(PostDT, Effect::PerfAbort),
                at(PostT, Effect::KeyboardProcessing(true)),
            ],
        )
    }

    /// Run `phase` with the shell holding D exactly for `In`, as the family PRs
    /// will: the executor only ever sees the guard's contents.
    fn run_phase(ports: &FakePorts, exec: &mut Executor<'_>, phase: Placement) -> PhaseReport {
        if phase == In {
            let mut guard = ports.app.dictation.lock_or_recover();
            let mut ctx = PhaseCtx {
                dictation: Some(&mut guard),
                recording_id: 1,
            };
            exec.run(phase, &mut ctx)
        } else {
            let mut ctx = PhaseCtx {
                dictation: None,
                recording_id: 1,
            };
            exec.run(phase, &mut ctx)
        }
    }

    fn executor_source() -> String {
        let source = include_str!("../dictation_executor/executor.rs");
        source
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .expect("production half")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Live and persisted M1/M2 agree (C1.4.2, X6).
    fn assert_core_equals_live(ports: &FakePorts, context: &str) {
        let dictation = ports.app.dictation.lock_or_recover();
        assert_eq!(
            dictation.core.status(),
            core_status(dictation.status),
            "{context}: M1"
        );
        assert_eq!(
            dictation.core.current_id(),
            RecordingId(ports.app.recording_id.load(Ordering::SeqCst)),
            "{context}: M2"
        );
    }

    /// `executor_phase_order`: the shell runs `Pre`, `In`, `PostDT`, `PostT`; each
    /// `run` executes exactly its placement, in returned order; an out-of-order or
    /// skipped phase runs nothing.
    #[test]
    fn executor_phase_order() {
        let ports = FakePorts::new();
        let mut exec = Executor::new(&ports, four_phase_decision(), false).expect("valid");
        // Skipping `Pre` and `In` is refused and runs nothing.
        assert_eq!(
            run_phase(&ports, &mut exec, PostDT),
            PhaseReport::Invalid(InvalidKind::Impossible(RowId("R999.1")))
        );
        assert!(ports.log().is_empty());

        let mut seen = 0;
        for (phase, expected) in [
            (Pre, vec!["TI"]),
            (In, vec!["KB"]),
            (PostDT, vec!["TERM", "PA"]),
            (PostT, vec!["KB"]),
        ] {
            assert_eq!(run_phase(&ports, &mut exec, phase), PhaseReport::Done);
            let log = ports.log();
            assert_eq!(&log[seen..], &expected[..], "{phase:?}");
            seen = log.len();
        }
        let codes: Vec<&str> = exec.witness().iter().map(|w| w.code).collect();
        assert_eq!(
            codes,
            ["CX-", "TI", "ST", "KB", "TERM", "PA", "KB"],
            "returned order"
        );
        let indexes: Vec<usize> = exec.witness().iter().map(|w| w.index).collect();
        assert_eq!(indexes, [0, 1, 2, 3, 4, 5, 6]);
        assert!(exec.is_finished());
        // The `ST` committed through the adapter.
        assert_eq!(
            ports.app.dictation.lock_or_recover().status,
            DictationStatus::Idle
        );

        // A phase earlier than the last one run is refused and runs nothing.
        assert_eq!(
            run_phase(&ports, &mut exec, In),
            PhaseReport::Invalid(InvalidKind::Impossible(RowId("R999.1")))
        );
        assert_eq!(ports.log().len(), seen);

        // `ST` outside `In` has no guard to write through: refused, not run.
        let ports = FakePorts::new();
        let mut exec = Executor::new(&ports, four_phase_decision(), false).expect("valid");
        assert_eq!(run_phase(&ports, &mut exec, Pre), PhaseReport::Done);
        let mut no_guard = PhaseCtx {
            dictation: None,
            recording_id: 1,
        };
        assert!(matches!(
            exec.run(In, &mut no_guard),
            PhaseReport::Invalid(InvalidKind::Impossible(_))
        ));
    }

    /// `awaited_effect_is_last_in_phase`: an Awaited effect (`CS` on the
    /// kept-preview path, `PS`) must be the last of its placement; `run` returns
    /// `Awaiting` instead of running it and runs nothing after it.
    #[test]
    fn awaited_effect_is_last_in_phase() {
        let ports = FakePorts::new();
        // Rejected when the decision is handed over.
        for (what, effects, kept) in [
            (
                "CS kept-preview then another PDT effect",
                vec![
                    at(PostDT, Effect::CaptureStart(RecordingId(1))),
                    at(PostDT, Effect::PerfAbort),
                ],
                true,
            ),
            (
                "PS then another PDT effect",
                vec![
                    at(PostDT, Effect::PipelineSpawn(RecordingId(1))),
                    at(PostDT, Effect::PerfAbort),
                ],
                false,
            ),
        ] {
            let result = Executor::new(&ports, decision("R000.1", effects), kept);
            assert_eq!(
                result.err(),
                Some(ExecutorError::AwaitedNotLast { index: 0 }),
                "{what}"
            );
        }
        // Allowed: the direct `CS` is Blocking and may be followed; an Awaited
        // effect may be followed by a *later* placement.
        assert!(Executor::new(
            &ports,
            decision(
                "R000.2",
                vec![
                    at(PostDT, Effect::CaptureStart(RecordingId(1))),
                    at(PostDT, Effect::PerfAbort),
                ],
            ),
            false,
        )
        .is_ok());
        assert!(Executor::new(
            &ports,
            decision(
                "R000.3",
                vec![
                    at(PostDT, Effect::PipelineSpawn(RecordingId(1))),
                    at(PostT, Effect::KeyboardProcessing(false)),
                ],
            ),
            false,
        )
        .is_ok());
        // A structurally invalid decision is refused too (C2.1).
        let mut bad = start_decision();
        bad.transition = None;
        assert!(matches!(
            Executor::new(&ports, bad, false).err(),
            Some(ExecutorError::Decision(_))
        ));

        // `run` executes everything before the awaited effect, returns
        // `Awaiting`, and runs nothing after it, not even in later calls.
        let mut exec = Executor::new(&ports, start_decision(), true).expect("valid");
        assert_eq!(run_phase(&ports, &mut exec, In), PhaseReport::Done);
        let report = run_phase(&ports, &mut exec, PostDT);
        assert_eq!(
            report,
            PhaseReport::Awaiting(AwaitedEffect {
                index: 10,
                effect: Effect::CaptureStart(RecordingId(1)),
            })
        );
        assert_eq!(
            ports.log(),
            ["GA", "PCC", "TRQ", "AX", "SPEC", "EV", "EM", "PR"],
            "everything before `CS` ran, `CS` and `KB` did not"
        );
        // While awaiting, nothing else runs.
        assert!(matches!(
            run_phase(&ports, &mut exec, PostT),
            PhaseReport::Invalid(InvalidKind::Impossible(_))
        ));
        assert_eq!(ports.log().len(), 8);
    }

    /// `awaited_effect_resumes_as_continuation`: the shell awaits the effect,
    /// `IN_CORE` is not held across the await, `resume` records the outcome, and
    /// the continuation re-enters as an ordinary input (validate, decide, retire)
    /// of the same flight before the later phases run.
    #[test]
    fn awaited_effect_resumes_as_continuation() {
        let ports = FakePorts::new();
        let history = &ports.app.core_history;
        let mut owner = ContinuationGuard::register(
            history,
            ContinuationSpec {
                recording_id: RecordingId(1),
                kind: ContKind::CaptureStart,
            },
        );
        let mut exec = Executor::new(&ports, start_decision(), true).expect("valid");
        assert_eq!(run_phase(&ports, &mut exec, In), PhaseReport::Done);
        let PhaseReport::Awaiting(awaited) = run_phase(&ports, &mut exec, PostDT) else {
            panic!("`CS` on the kept-preview path is awaited");
        };
        assert_eq!(awaited.effect, Effect::CaptureStart(RecordingId(1)));
        assert!(!in_core(), "IN_CORE is never held across an await");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // The shell awaits the port's future here, holding no D and no
            // `core_history`; the thread-local flag is clear.
            tokio::task::yield_now().await;
            assert!(!in_core());
            assert!(
                CoreEntry::enter().is_ok(),
                "another entry may run meanwhile"
            );
            assert!(!crate::core_history::is_held_on_this_thread());
        });
        assert!(!in_core(), "the probe entry released the flag");

        assert_eq!(exec.resume(AwaitedOutcome::Completed), PhaseReport::Done);
        let last = *exec.witness().last().expect("witness");
        assert_eq!(
            (last.index, last.code, last.outcome),
            (10, "CS", WitnessOutcome::Resumed(AwaitedOutcome::Completed))
        );
        assert!(!ports.log().contains(&"CS"), "the executor never ran it");

        // The continuation input is built in straight-line code and decided as
        // an ordinary input: the op is still outstanding until it consumes it.
        let effects = owner.consume(|status| {
            assert_eq!(status, OpStatus::Outstanding);
            decide_guarded(|| decision("R102.1", Vec::new()))
        });
        assert_eq!(effects.verdict, Verdict::Applied);
        assert_eq!(history.outstanding_ops(), 0, "consumed exactly once");

        // The rest of the flight continues after the resume.
        assert_eq!(run_phase(&ports, &mut exec, PostT), PhaseReport::Done);
        assert_eq!(ports.log().last(), Some(&"KB"));
        assert!(exec.is_finished());

        // A resume with nothing awaited is refused; a failed await is recorded.
        assert!(matches!(
            exec.resume(AwaitedOutcome::Completed),
            PhaseReport::Invalid(InvalidKind::Impossible(_))
        ));
        let mut exec = Executor::new(&ports, start_decision(), true).expect("valid");
        run_phase(&ports, &mut exec, In);
        run_phase(&ports, &mut exec, PostDT);
        assert_eq!(exec.resume(AwaitedOutcome::Failed), PhaseReport::Done);
        assert_eq!(
            exec.witness().last().map(|w| w.outcome),
            Some(WitnessOutcome::Resumed(AwaitedOutcome::Failed))
        );
    }

    /// `executor_never_locks`: D and T are never acquired by the executor (a
    /// fake port probes both at every effect), `core_history` is never touched,
    /// and the executor never calls a decide function.
    #[test]
    fn executor_never_locks() {
        let ports = FakePorts::new();
        let mut exec = Executor::new(&ports, four_phase_decision(), false).expect("valid");
        let acquisitions = ports.app.core_history.acquisitions();
        for phase in [Pre, In, PostDT, PostT] {
            assert_eq!(run_phase(&ports, &mut exec, phase), PhaseReport::Done);
        }
        let mut exec = Executor::new(&ports, start_decision(), false).expect("valid");
        for phase in [In, PostDT, PostT] {
            assert_eq!(run_phase(&ports, &mut exec, phase), PhaseReport::Done);
        }
        assert_eq!(
            ports.app.core_history.acquisitions(),
            acquisitions,
            "the executor never takes core_history"
        );
        let probes = ports.probes.borrow();
        assert!(!probes.is_empty());
        let mut in_phase_probes = 0;
        for probe in probes.iter() {
            assert!(probe.t_free, "{}: T is never taken", probe.code);
            assert!(
                !probe.history_held,
                "{}: no effect under core_history",
                probe.code
            );
        }
        // `KB` under D (the shell holds it, so a probe sees it busy) and outside D
        // (free): D is held exactly by the shell and by nobody else.
        for probe in probes.iter() {
            if probe.code == "GA" || probe.code == "PCC" {
                assert!(!probe.d_free, "{}: runs under the shell's D", probe.code);
                in_phase_probes += 1;
            }
            if matches!(
                probe.code,
                "SPEC" | "EV" | "EM" | "PR" | "CS" | "TERM" | "PA"
            ) {
                assert!(probe.d_free, "{}: runs with D released", probe.code);
            }
        }
        assert!(in_phase_probes >= 2);
        drop(probes);

        // Structure: no lock of any kind and no decide call in the executor.
        let source = executor_source();
        for forbidden in [
            ".lock(",
            "lock_or_recover",
            "try_lock",
            "recording_transition",
            "dictation.lock",
            "app_state().dictation",
            "core_history.",
            ".reg(",
            "ret_if_matches",
        ] {
            assert!(
                !source.contains(forbidden),
                "executor.rs must not contain `{forbidden}`"
            );
        }
        let decide_call = regex::Regex::new(r"\bdecide_\w+\s*\(").expect("regex");
        for found in decide_call.find_iter(&source) {
            assert!(
                found.as_str().starts_with("decide_guarded"),
                "the executor must not call a decide function: {}",
                found.as_str()
            );
        }
    }

    fn nested_decision() -> Decision {
        decision("R888.1", vec![at(PostDT, Effect::PerfAbort)])
    }

    /// `reentrancy_refused`: a listener that runs inline under an `In` emit tries
    /// to re-enter the core. A nested decide, entry and `run` each return
    /// `Invalid(Reentrant)` with no effects; the outer flight is unaffected and
    /// the flag clears afterwards.
    #[test]
    fn reentrancy_refused() {
        let ports = FakePorts::new();
        let nested_runs = Rc::new(Cell::new(0));
        let nested_counter = Rc::clone(&nested_runs);
        *ports.on_emit.borrow_mut() = Some(Box::new(move |p: &FakePorts| {
            let nested = decide_guarded(|| panic!("a nested decide must never run"));
            assert_eq!(nested.verdict, Verdict::Invalid(InvalidKind::Reentrant));
            assert!(nested.effects.is_empty());
            assert!(nested.transition.is_none() && nested.continuation.is_none());
            assert_eq!(CoreEntry::enter().err(), Some(InvalidKind::Reentrant));
            let mut inner = Executor::new(p, nested_decision(), false).expect("valid");
            let mut ctx = PhaseCtx {
                dictation: None,
                recording_id: 1,
            };
            assert_eq!(
                inner.run(PostDT, &mut ctx),
                PhaseReport::Invalid(InvalidKind::Reentrant)
            );
            assert_eq!(
                inner.resume(AwaitedOutcome::Completed),
                PhaseReport::Invalid(InvalidKind::Reentrant)
            );
            assert!(inner.witness().is_empty(), "no effects ran");
            nested_counter.set(nested_counter.get() + 1);
            assert!(!p.log().contains(&"PA"), "the nested effect never ran");
        }));
        // The nested attempt happens during an `In` emit, with the shell holding D.
        let outer = decision(
            "R777.1",
            vec![
                at(
                    In,
                    Effect::EmitStatus {
                        status: Status::Idle,
                        cond: Cond::Always,
                    },
                ),
                at(PostDT, Effect::PerfAbort),
            ],
        );
        let mut exec = Executor::new(&ports, outer, false).expect("valid");
        assert_eq!(run_phase(&ports, &mut exec, In), PhaseReport::Done);
        assert_eq!(nested_runs.get(), 1, "the listener ran inline");
        assert_eq!(run_phase(&ports, &mut exec, PostDT), PhaseReport::Done);
        assert_eq!(ports.log(), ["EM", "PA"]);
        assert!(!in_core(), "the flag clears when the outer span ends");
        // A later, ordinary decide works.
        let fresh = decide_guarded(nested_decision);
        assert_eq!(fresh.verdict, Verdict::Applied);
    }

    /// `reentrancy_refused_before_lock`: the nested entry attempts no D, T or
    /// `core_history` lock. The shell holds D on this very thread (a `std`
    /// mutex: a second `lock` would deadlock), the nested entries return, and the
    /// `core_history` acquisition count does not move. The check is the first
    /// statement of every entry.
    #[test]
    fn reentrancy_refused_before_lock() {
        let ports = FakePorts::new();
        let before_nested = Rc::new(Cell::new(0u64));
        let after_nested = Rc::new(Cell::new(0u64));
        let held_on_entry = Rc::new(Cell::new(false));
        let (before_probe, after_probe, held_probe) = (
            Rc::clone(&before_nested),
            Rc::clone(&after_nested),
            Rc::clone(&held_on_entry),
        );
        *ports.on_emit.borrow_mut() = Some(Box::new(move |p: &FakePorts| {
            before_probe.set(p.app.core_history.acquisitions());
            held_probe.set(p.app.dictation.try_lock().is_err());
            let _ = decide_guarded(nested_decision);
            let _ = CoreEntry::enter().err();
            let mut inner = Executor::new(p, nested_decision(), false).expect("valid");
            let mut ctx = PhaseCtx {
                dictation: None,
                recording_id: 1,
            };
            let _ = inner.run(PostDT, &mut ctx);
            let _ = inner.resume(AwaitedOutcome::Failed);
            after_probe.set(p.app.core_history.acquisitions());
        }));
        let outer = decision(
            "R777.2",
            vec![at(
                In,
                Effect::EmitStatus {
                    status: Status::Idle,
                    cond: Cond::Always,
                },
            )],
        );
        let mut exec = Executor::new(&ports, outer, false).expect("valid");
        // Hold T as well: a nested attempt at either lock cannot have succeeded.
        let transition = ports
            .app
            .recording_transition
            .try_lock()
            .expect("T is free");
        assert_eq!(run_phase(&ports, &mut exec, In), PhaseReport::Done);
        drop(transition);
        assert!(
            held_on_entry.get(),
            "the shell held D during the nested entry"
        );
        assert_eq!(
            before_nested.get(),
            after_nested.get(),
            "the nested entries took no core_history lock"
        );

        // Structure: `enter()` is the first statement of every entry point.
        let source = executor_source();
        for entry in [
            "pub(crate) fn run(",
            "pub(crate) fn resume(",
            "pub(crate) fn decide_guarded(",
        ] {
            let start = source.find(entry).unwrap_or_else(|| panic!("{entry}"));
            let body = &source[start..];
            let first_statement = body[body.find('{').expect("body") + 1..]
                .trim_start()
                .lines()
                .next()
                .expect("first line");
            let head: String = body.lines().skip(1).take(3).collect::<Vec<_>>().join(" ");
            assert!(
                first_statement.contains("CoreEntry::enter()")
                    || head.contains("CoreEntry::enter()"),
                "{entry}: the re-entrancy check must come first, found `{first_statement}`"
            );
            let enter_at = body.find("CoreEntry::enter()").expect("enter");
            for lock_like in ["assert_not_held", "ports.", "self.ports", "app_state"] {
                if let Some(pos) = body.find(lock_like) {
                    assert!(
                        enter_at < pos,
                        "{entry}: `{lock_like}` precedes the entry check"
                    );
                }
            }
        }
    }

    /// Drive `decision` through `In` and `PostDT` with a panic injected at `code`;
    /// returns the executor's witness codes.
    fn drive_with_panic(
        ports: &FakePorts,
        code: &'static str,
        kept_preview: bool,
    ) -> Vec<&'static str> {
        ports.panic_at.set(Some(code));
        let mut exec = Executor::new(ports, start_decision(), kept_preview).expect("valid");
        let before = ports.app.dictation.lock_or_recover().core;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            run_phase(ports, &mut exec, In);
            run_phase(ports, &mut exec, PostDT);
            run_phase(ports, &mut exec, PostT);
        }));
        let payload = outcome.expect_err("the injected panic unwinds through the executor");
        let message = payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_default();
        assert!(message.contains(code), "{message}");
        // The executor did not catch it, and left no flag or lock behind.
        assert!(!in_core(), "{code}: IN_CORE cleared by the unwind");
        assert!(!crate::core_history::is_held_on_this_thread());
        // Released by the unwind (a `std` mutex is poisoned by it, and
        // `lock_or_recover` recovers, as in production).
        assert!(
            ports.app.dictation.try_lock_or_recover().is_some(),
            "{code}: D released by the unwind"
        );
        let _ = before;
        exec.witness().iter().map(|w| w.code).collect()
    }

    /// `panic_partial_application` (X5): a panic in any effect unwinds through
    /// the executor; effects before it ran and committed their own field, effects
    /// after it did not run.
    #[test]
    fn panic_partial_application() {
        let full = [
            "ID+", "GA", "PCC", "TRQ", "ST", "AX", "SPEC", "EV", "EM", "PR", "CS", "KB",
        ];
        for code in [
            "GA", "PCC", "TRQ", "AX", "SPEC", "EV", "EM", "PR", "CS", "KB",
        ] {
            let ports = FakePorts::new();
            let witnessed = drive_with_panic(&ports, code, false);
            let panicked_at = full.iter().position(|c| *c == code).expect(code);
            assert_eq!(
                witnessed,
                &full[..panicked_at],
                "{code}: effects before it applied, it and the rest did not"
            );
            assert_eq!(
                ports.log().last(),
                Some(&code),
                "{code}: the panicking port was the last to run"
            );
            assert_core_equals_live(&ports, code);
            // Each field shows exactly "effects before the panic applied".
            let st_ran = panicked_at > full.iter().position(|c| *c == "ST").unwrap();
            let dictation = ports.app.dictation.lock_or_recover();
            assert_eq!(
                dictation.status,
                if st_ran {
                    DictationStatus::Starting
                } else {
                    DictationStatus::Idle
                },
                "{code}"
            );
            assert_eq!(
                ports.app.recording_id.load(Ordering::SeqCst),
                1,
                "{code}: ID+ ran first"
            );
        }
        // The test above is the proof that the executor does not `catch_unwind`
        // (it would have returned a report instead of unwinding).
    }

    /// `panic_before_st_leaves_core_equal_live` (X6): a panic in `GA`, `PCC` or
    /// `TRQ` before `ST` of an R101.15-shaped list, and in the `EM` after it,
    /// leaves the persisted core equal to live for M1 and M2, in the same partial
    /// state as today. Nothing commits at decide time.
    #[test]
    fn panic_before_st_leaves_core_equal_live() {
        // The decision exists but nothing has committed: decide mutates nothing.
        let ports = FakePorts::new();
        let _decision = decide_guarded(start_decision);
        {
            let dictation = ports.app.dictation.lock_or_recover();
            assert_eq!(dictation.core, Default::default());
            assert_eq!(ports.app.recording_id.load(Ordering::SeqCst), 0);
        }
        for code in ["GA", "PCC", "TRQ"] {
            let ports = FakePorts::new();
            drive_with_panic(&ports, code, false);
            assert_core_equals_live(&ports, code);
            let dictation = ports.app.dictation.lock_or_recover();
            // `ID+` (the first effect) committed M2 in both places; `ST` did not run.
            assert_eq!(dictation.core.current_id(), RecordingId(1), "{code}");
            assert_eq!(ports.app.recording_id.load(Ordering::SeqCst), 1, "{code}");
            assert_eq!(dictation.core.status(), Status::Idle, "{code}");
            assert_eq!(dictation.status, DictationStatus::Idle, "{code}");
        }
        // In the `EM` after `ST`: both show `Starting`.
        let ports = FakePorts::new();
        drive_with_panic(&ports, "EM", false);
        assert_core_equals_live(&ports, "EM");
        let dictation = ports.app.dictation.lock_or_recover();
        assert_eq!(dictation.core.status(), Status::Starting);
        assert_eq!(dictation.status, DictationStatus::Starting);
        assert_eq!(dictation.core.current_id(), RecordingId(1));
    }

    /// `emit_result_gates_pres`: a failing emit suppresses the paired `PRES`
    /// (`P-emit-ok`) and not the effects after it; a successful emit lets it run.
    #[test]
    fn emit_result_gates_pres() {
        let list = |paired: u8| {
            decision(
                "R999.9",
                vec![
                    at(
                        PostDT,
                        Effect::EmitStatus {
                            status: Status::Recovering,
                            cond: Cond::Always,
                        },
                    ),
                    at(
                        PostDT,
                        Effect::Presentation {
                            kind: PresentationKind::CleanupInProgress,
                            cond: Cond::EmitOk(paired),
                        },
                    ),
                    at(PostDT, Effect::KeyboardProcessing(false)),
                ],
            )
        };
        for (fails, expected, pres) in [
            (true, vec!["EM", "KB"], WitnessOutcome::Skipped),
            (false, vec!["EM", "PRES", "KB"], WitnessOutcome::Ran),
        ] {
            let ports = FakePorts::new();
            ports.fail_emits.set(fails);
            let mut exec = Executor::new(&ports, list(0), false).expect("valid");
            assert_eq!(run_phase(&ports, &mut exec, PostDT), PhaseReport::Done);
            assert_eq!(ports.log(), expected, "emit failing: {fails}");
            let outcomes: Vec<WitnessOutcome> = exec.witness().iter().map(|w| w.outcome).collect();
            assert_eq!(
                outcomes[0],
                if fails {
                    WitnessOutcome::Failed
                } else {
                    WitnessOutcome::Ran
                }
            );
            assert_eq!(outcomes[1], pres);
            assert_eq!(
                *outcomes.last().expect("kb"),
                WitnessOutcome::Ran,
                "later effects run"
            );
        }
        // The shell predicate `P-spec-flowing` skips `EM(starting)` the same way.
        let ports = FakePorts::new();
        ports.speculative.set(true);
        let mut exec = Executor::new(&ports, start_decision(), false).expect("valid");
        run_phase(&ports, &mut exec, In);
        run_phase(&ports, &mut exec, PostDT);
        assert!(
            !ports.log().contains(&"EM"),
            "EM(starting) skipped while flowing"
        );
        assert!(ports.log().contains(&"PR"));
    }
}
