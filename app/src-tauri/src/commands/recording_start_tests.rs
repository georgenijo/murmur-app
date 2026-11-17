//! Shell-level start-family tests (Stage 2a-4a: sites 101 and 102).
//!
//! Each scenario drives `start_native_recording_inner` through the Stage 0b golden
//! harness. The scenarios that compare traces take both sides from the same
//! shell, so they hold before and after the start family moves behind
//! `decide_*`; the awaited-effect and history witnesses read the executor's
//! parked cursor and `core_history`, which only the refactored shell has.
use super::golden_harness::*;
use super::golden_tests::{cancelled, recording};
use super::*;
use std::task::Poll;

fn trace(setup: Setup, body: fn(&Arc<World>)) -> Vec<String> {
    run_sub_run(&SubRun {
        title: "start",
        setup,
        body,
    })
}

fn history_counts(world: &Arc<World>) -> (usize, usize) {
    let history = &world.state().app_state.core_history;
    (history.pipeline_count(), history.outstanding_ops())
}

fn resumed() -> Vec<crate::dictation_executor::executor::Witness> {
    start_ports::RESUMED.with(|slot| slot.borrow().clone())
}

fn last_two(
    witness: &[crate::dictation_executor::executor::Witness],
) -> [(&'static str, String); 2] {
    let tail = &witness[witness.len() - 2..];
    [
        (tail[0].code, format!("{:?}", tail[0].outcome)),
        (tail[1].code, format!("{:?}", tail[1].outcome)),
    ]
}

fn keep_a_mic_test(w: &Arc<World>) {
    w.state()
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
}

/// The kept-preview start awaits `CS` through the executor and resumes it with the
/// supervisor's real answer: `Completed` for an acknowledged start.
#[test]
fn the_awaited_capture_start_resumes_completed_when_the_supervisor_acknowledges() {
    run_sub_run(&SubRun {
        title: "resume success",
        setup: Setup::idle(),
        body: |w| {
            keep_a_mic_test(w);
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            start_ports::RESUMED.with(|slot| slot.borrow_mut().clear());
            // The kept-preview start joins a `spawn_blocking` task.
            let mut start = w.start();
            let reply = complete_joining(&mut start);
            w.reply("start", &reply);
            assert!(reply.is_ok(), "{reply:?}");
            assert_eq!(
                last_two(&resumed()),
                [
                    ("CS", "Awaiting".to_string()),
                    ("CS", "Resumed(Completed)".to_string())
                ]
            );
            assert_eq!(history_counts(w), (0, 0), "every start op is retired");
        },
    });
}

/// A supervisor refusal reaches `resume` as `Failed`, and site 102 then answers
/// from the same result.
#[test]
fn the_awaited_capture_start_resumes_failed_when_the_supervisor_refuses() {
    run_sub_run(&SubRun {
        title: "resume failure",
        setup: Setup::idle(),
        body: |w| {
            keep_a_mic_test(w);
            w.knobs().start_ack = Some(AudioStartError::AudioRecovering);
            start_ports::RESUMED.with(|slot| slot.borrow_mut().clear());
            let mut start = w.start();
            let reply = complete_joining(&mut start);
            w.reply("start", &reply);
            assert_eq!(
                reply.expect("the refusal is a typed reply"),
                serde_json::json!({ "type": "audio_recovering", "state": "idle" })
            );
            assert_eq!(
                last_two(&resumed()),
                [
                    ("CS", "Awaiting".to_string()),
                    ("CS", "Resumed(Failed)".to_string())
                ]
            );
            assert_eq!(history_counts(w), (0, 0), "every start op is retired");
        },
    });
}

/// `K:CaptureStart` is registered right after the decide and retired by its
/// consumer (site 102) or by `Drop`, on every path; `K:ResolveContext` follows the
/// accepted capture start. None outlives the command.
#[test]
fn every_start_history_entry_is_retired_when_the_command_returns() {
    run_sub_run(&SubRun {
        title: "accepted start",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            assert_eq!(history_counts(w), (0, 0));
        },
    });
    run_sub_run(&SubRun {
        title: "refused start",
        setup: Setup::idle(),
        body: |w| {
            w.knobs().start_ack = Some(AudioStartError::AlreadyStarting);
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            assert_eq!(history_counts(w), (0, 0));
        },
    });
    run_sub_run(&SubRun {
        title: "blocked start",
        setup: Setup {
            status: DictationStatus::Processing,
            recording_id: 4,
        },
        body: |w| {
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            assert_eq!(history_counts(w), (0, 0));
        },
    });
}

thread_local! {
    /// What each nested entry returned (on the scenario's own thread).
    static NESTED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// An inline listener re-enters the start command on the thread of the port call.
fn nested_start(world: &Arc<World>) {
    let mut start = world.start();
    let outcome = match poll_once(&mut start) {
        Poll::Ready(reply) => format!("{reply:?}"),
        Poll::Pending => "pending".to_string(),
    };
    NESTED.with(|nested| nested.borrow_mut().push(outcome));
}

fn under_watchdog<T: Send + 'static>(
    what: &'static str,
    body: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        let _ = done.send(outcome);
    });
    match finished.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(Ok(value)) => value,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => {
            eprintln!("DEADLOCK: {what}");
            std::process::abort();
        }
    }
}

fn run_nested(what: &'static str, body: fn(&Arc<World>)) -> (Vec<String>, Vec<String>) {
    under_watchdog(what, move || {
        NESTED.with(|nested| nested.borrow_mut().clear());
        let lines = trace(Setup::idle(), body);
        (lines, NESTED.with(|nested| nested.borrow().clone()))
    })
}

fn refused(outcome: &str) -> bool {
    outcome.contains("Err(")
        && outcome.contains("refused the start")
        && outcome.contains("Reentrant")
}

/// C4.2 / decision D-S2-2: the start entry is the first statement, before the
/// claims, T, D or any managed-state lookup, and `IN_CORE` stays set across
/// production port calls. An inline listener of the `PDT` emits (T held, D
/// released) that starts again would park on T; it is refused instead, and the
/// refused entry changes nothing.
#[test]
fn a_start_nested_in_a_port_emit_is_refused_before_any_lock() {
    fn plain(w: &Arc<World>) {
        recording(w);
    }
    fn nested_in_generation_started(w: &Arc<World>) {
        w.on_inline("emit:dictation-generation-started", {
            let world = Arc::clone(w);
            move || nested_start(&world)
        });
        recording(w);
    }
    fn nested_in_starting(w: &Arc<World>) {
        w.on_inline("emit:recording-status-changed", {
            let world = Arc::clone(w);
            move || nested_start(&world)
        });
        recording(w);
    }
    let (baseline, _) = run_nested("baseline", plain);
    for (what, body) in [
        (
            "a start nested in the generation-started emit",
            nested_in_generation_started as fn(&Arc<World>),
        ),
        ("a start nested in the starting emit", nested_in_starting),
    ] {
        let (lines, results) = run_nested(what, body);
        assert_eq!(results.len(), 1, "{what}: the listener ran once");
        assert!(refused(&results[0]), "{what}: {}", results[0]);
        assert_eq!(lines, baseline, "{what}: the refused start changed nothing");
    }
}

// ------------------------------------------------- Smart Auto routing notification

fn script_smart_auto(w: &Arc<World>) {
    w.knobs().smart_auto = Some(Ok(crate::microphone_auto::SmartAutoSelection {
        device_id: "mic-7".to_string(),
        reason: crate::microphone_auto::SmartAutoReason::CurrentVerified,
    }));
}

fn smart_auto_started(w: &Arc<World>) {
    script_smart_auto(w);
    w.knobs().cold_start = ColdStart::ReadyBeforeContext;
    let mut start = w.start_smart_auto(None);
    let reply = complete(&mut start);
    w.reply("start", &reply);
    assert!(reply.is_ok(), "{reply:?}");
}

/// Decision D-S2-2: the Smart Auto resolver runs under T and D and emits
/// `smart-auto-microphone-changed` inline on success. A listener of that
/// notification that starts again would park on T; it is refused before any
/// lock instead, and the refused entry changes nothing.
#[test]
fn a_start_nested_in_the_smart_auto_routing_notification_is_refused_before_any_lock() {
    let (baseline, _) = under_watchdog("smart auto baseline", || {
        NESTED.with(|nested| nested.borrow_mut().clear());
        (trace(Setup::idle(), smart_auto_started), ())
    });
    let (lines, results) = under_watchdog("a start nested in the routing notification", || {
        NESTED.with(|nested| nested.borrow_mut().clear());
        let lines = trace(Setup::idle(), |w| {
            w.on_inline("smart_auto.routing_changed", {
                let world = Arc::clone(w);
                move || nested_start(&world)
            });
            smart_auto_started(w);
        });
        (lines, NESTED.with(|nested| nested.borrow().clone()))
    });
    assert_eq!(results.len(), 1, "the listener ran once");
    assert!(refused(&results[0]), "{}", results[0]);
    assert_eq!(lines, baseline, "the refused start changed nothing");
}

/// A pipeline owner an inline listener of the routing notification drops while the
/// start holds T and D does not ask for D (it would deadlock on the non-reentrant
/// lock): its finish is queued and the start drains it after releasing D, so the
/// owner's H1 and H2 entries are retired exactly once.
#[test]
fn dropping_a_pipeline_owner_from_the_smart_auto_routing_notification_defers_its_finish() {
    under_watchdog("a pipeline dropped from the routing notification", || {
        run_sub_run(&SubRun {
            title: "pipeline finish from the routing notification",
            setup: Setup::idle(),
            body: |w| {
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                w.close_gate("pipeline");
                let mut stop = w.stop(None);
                assert!(poll_once(&mut stop).is_pending());
                cancelled(w, "cancel");
                w.finish_sealed_stop(1);
                assert_eq!(history_counts(w), (1, 1), "the parked stop's pipeline");
                let parked = Arc::new(std::sync::Mutex::new(Some(stop)));
                let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
                w.on_inline("smart_auto.routing_changed", move || {
                    drop(slot.lock().expect("slot").take());
                    assert_eq!(
                        history_counts(&observer),
                        (1, 1),
                        "nothing is retired while the start holds D and T"
                    );
                });
                take_t_free_at_drain();
                smart_auto_started(w);
                assert!(parked.lock().expect("slot").is_none(), "the listener ran");
                assert!(
                    DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the queue was drained"
                );
                assert_eq!(history_counts(w), (0, 0), "H1 and H2 are retired");
                assert_drained_with_t_free("the routing notification's finish");
                w.open_gate("pipeline");
            },
        });
    });
}

// ------------------------------------------- D-S2-8 / D-S2-10(b): the start's drain guards

type ParkedStop = Arc<std::sync::Mutex<Option<Fut<Result<serde_json::Value, String>>>>>;

/// Recording 1's stop parked mid-pipeline (its H1 and H2 registered) after the
/// recording was cancelled and its capture finished: the owner a listener drops.
fn park_a_stop_of_recording_1(w: &Arc<World>) -> ParkedStop {
    recording(w);
    w.knobs().stop_result = Ok(16_000);
    w.close_gate("pipeline");
    let mut stop = w.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    cancelled(w, "cancel");
    w.finish_sealed_stop(1);
    assert_eq!(history_counts(w), (1, 1), "the parked stop's pipeline");
    Arc::new(std::sync::Mutex::new(Some(stop)))
}

fn queued_finishes() -> usize {
    DEFERRED_FINISHES.with(|queue| queue.borrow().len())
}

/// D-S2-13: whether the flight's T was acquirable when the drain ran each entry
/// (`take_t_free_at_drain`, a test-build log kept by the drain itself).
fn assert_drained_with_t_free(what: &str) {
    let seen = take_t_free_at_drain();
    assert!(!seen.is_empty(), "{what}: the drain ran an entry");
    assert!(
        seen.iter().all(|free| *free),
        "{what}: a deferred drain ran while the flight's T was held: {seen:?}"
    );
}

/// R101.1 (the pre-D refusal): its `PT:REL` runs in a core span. A listener that
/// drops a parked stop there queues its finish; the start drains it before it
/// returns, so H1 and H2 are retired once and the queue is empty.
#[test]
fn the_pre_d_refusal_drains_what_its_release_defers() {
    under_watchdog("a stop dropped during the pre-D refusal's release", || {
        run_sub_run(&SubRun {
            title: "pre-D refusal drain",
            setup: Setup::idle(),
            body: |w| {
                let parked = park_a_stop_of_recording_1(w);
                w.knobs().app_disabled = true;
                let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
                take_t_free_at_drain();
                let _hook = crate::barrier_hook::install(move |point| {
                    if point == "start.release_claims" {
                        drop(slot.lock().expect("slot").take());
                        assert_eq!(
                            history_counts(&observer),
                            (1, 1),
                            "nothing retires in the span"
                        );
                        assert_eq!(queued_finishes(), 1, "the finish is queued");
                    }
                });
                let mut start = w.start();
                let reply = complete(&mut start);
                assert!(
                    matches!(&reply, Ok(value) if value["type"] == "app_disabled"),
                    "{reply:?}"
                );
                assert!(parked.lock().expect("slot").is_none(), "the listener ran");
                assert_eq!(
                    queued_finishes(),
                    0,
                    "the queue is empty when the start returns"
                );
                assert_eq!(history_counts(w), (0, 0), "H1 and H2 are retired once");
                assert_drained_with_t_free("the pre-D refusal");
                w.open_gate("pipeline");
            },
        });
    });
}

/// D-S2-13: a start that suspends at its awaited `CS` with work its spans queued
/// takes that work off the thread-local queue (it travels with the future); it is
/// applied only after the flight released T, on the thread that finishes the start.
#[test]
fn a_flight_that_suspends_with_queued_work_carries_it_and_drains_after_t() {
    under_watchdog("a flight suspended with queued work", || {
        run_sub_run(&SubRun {
            title: "carried work",
            setup: Setup::idle(),
            body: |w| {
                let parked = park_a_stop_of_recording_1(w);
                keep_a_mic_test(w);
                w.knobs().cold_start = ColdStart::ReadyBeforeContext;
                let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
                w.on_inline("emit:recording-status-changed", move || {
                    if slot.lock().expect("slot").take().is_some() {
                        assert_eq!(queued_finishes(), 1, "the dropped stop's finish is queued");
                        observer.note("listener: dropped the parked stop");
                    }
                });
                take_t_free_at_drain();
                let mut start = w.start();
                assert!(
                    poll_once(&mut start).is_pending(),
                    "the kept-preview start awaits its capture start"
                );
                assert!(parked.lock().expect("slot").is_none(), "the listener ran");
                assert_eq!(
                    queued_finishes(),
                    0,
                    "the suspended flight took its queued work off this thread"
                );
                assert!(
                    take_t_free_at_drain().is_empty(),
                    "nothing drained while parked"
                );
                assert_eq!(
                    history_counts(w).0,
                    1,
                    "the dropped stop's pipeline is not retired yet"
                );
                let reply = complete_joining(&mut start);
                assert!(reply.is_ok(), "{reply:?}");
                assert_drained_with_t_free("the carried finish");
                assert_eq!(queued_finishes(), 0);
                assert_eq!(history_counts(w), (0, 0), "H1 and H2 are retired once");
                w.open_gate("pipeline");
            },
        });
    });
}

/// R102's failure prefix (`Pre:PA, Pre:CX-`) runs before D, in a core span: a
/// listener of the abort port that drops a parked stop queues its finish, and the
/// start drains it after releasing D (a finish that ran under D would deadlock).
#[test]
fn the_capture_failure_prefix_drains_what_the_abort_port_defers() {
    under_watchdog("a stop dropped during the failure prefix", || {
        run_sub_run(&SubRun {
            title: "failure prefix drain",
            setup: Setup::idle(),
            body: |w| {
                let parked = park_a_stop_of_recording_1(w);
                w.knobs().start_ack = Some(AudioStartError::AlreadyStarting);
                let slot = Arc::clone(&parked);
                take_t_free_at_drain();
                let _hook = crate::barrier_hook::install(move |point| {
                    if point == "start.perf_abort" {
                        drop(slot.lock().expect("slot").take());
                        assert_eq!(queued_finishes(), 1, "the finish is queued");
                    }
                });
                let mut start = w.start();
                let reply = complete(&mut start);
                w.reply("start", &reply);
                assert!(parked.lock().expect("slot").is_none(), "the listener ran");
                assert_eq!(queued_finishes(), 0);
                assert_eq!(history_counts(w), (0, 0), "H1 and H2 are retired once");
                assert_drained_with_t_free("the failure prefix");
                w.open_gate("pipeline");
            },
        });
    });
}

/// The same prefix, when the abort port's callback queues a finish and then
/// panics: the guard, declared before the first effect, drains during the unwind,
/// so nothing stays queued on the thread and no history entry is stranded.
#[test]
fn a_panic_in_the_abort_port_still_drains_the_failure_prefix() {
    under_watchdog("an unwind from the abort port", || {
        run_sub_run(&SubRun {
            title: "failure prefix unwind",
            setup: Setup::idle(),
            body: |w| {
                let parked = park_a_stop_of_recording_1(w);
                w.knobs().start_ack = Some(AudioStartError::AlreadyStarting);
                let slot = Arc::clone(&parked);
                take_t_free_at_drain();
                let _hook = crate::barrier_hook::install(move |point| {
                    if point == "start.perf_abort" {
                        drop(slot.lock().expect("slot").take());
                        panic!("the abort port's callback panics after queueing a finish");
                    }
                });
                let mut start = w.start();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    poll_once(&mut start)
                }));
                assert!(outcome.is_err(), "the panic unwinds out of the start");
                assert_eq!(queued_finishes(), 0, "the unwind drained the queue");
                assert_eq!(history_counts(w), (0, 0), "no history entry is stranded");
                assert_drained_with_t_free("the unwind from the abort port");
                w.open_gate("pipeline");
            },
        });
    });
}
