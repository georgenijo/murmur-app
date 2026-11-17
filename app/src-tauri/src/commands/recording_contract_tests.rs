//! Stage 1 contract tests (C4.9) that pin the executor contract (C4) on
//! today's shell behaviour. They need no Stage 2 executor: each drives the
//! existing commands through the Stage 0a/0b harness, or the existing ports
//! (terminal ledger, performance begin ledger, interrupted-capture store)
//! directly. The tests C4.9 lists that need the Stage 2 executor, the op ids or
//! the `core_history` are deferred; `recording_pd_witness_map.md` lists each
//! with its reason.
//!
//! Ordering is by causal barriers only: no clock, no sleep.

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
    world
        .state()
        .app_state
        .recording_id
        .store(2, Ordering::SeqCst);
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
