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

// ---------------------------------------------------------------- site 104

/// R104.5 / #216 mutation witness (appendix C13.3): the start reads
/// `is_dictation_recording(rid) == true` and, before it takes D, a runtime failure
/// for the current id lands (`Recovering{RuntimeFailure}` writes `Recovering` and
/// preserves the installed context). The publish then runs with a stale audio
/// read and must publish nothing: no `recording` emit, no status write, no `SD`.
///
/// Dropping R104.5's `status == Starting` guard makes it publish `Recording` over
/// `Recovering`: this test fails (the mutation check in the PR).
#[test]
fn a_late_ready_publish_after_a_runtime_failure_publishes_nothing() {
    let lines = run_sub_run(&SubRun {
        title: "late publish",
        setup: Setup::idle(),
        body: |w| {
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            w.on("supervisor.is_dictation_recording", {
                let world = Arc::clone(w);
                move || world.runtime_failure(1)
            });
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            assert_eq!(
                reply.expect("the start flight ends with its own reply"),
                serde_json::json!({ "type": "recording_starting", "state": "starting", "recordingId": 1 })
            );
            assert_eq!(
                w.side_task_spawns(),
                0,
                "no side task starts for a publish that did not happen"
            );
            let state = w.state();
            assert_eq!(
                state.app_state.dictation.lock_or_recover().status,
                DictationStatus::Recovering,
                "Recovering is not overwritten with Recording"
            );
        },
    });
    assert!(
        lines
            .iter()
            .any(|l| l == "publish_recording_ready 1 -> false"),
        "{lines:#?}"
    );
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("recording-status-changed \"recording\"")),
        "no `recording` emit: {lines:#?}"
    );
    assert!(
        !lines.iter().any(|l| l.contains("-> Recording")),
        "no status write to Recording: {lines:#?}"
    );
}

/// The same publish with nothing in the window is R104.2: `Starting→Recording`,
/// the `recording` emit and exactly one `SD`.
#[test]
fn an_undisturbed_ready_publish_writes_recording_emits_and_starts_the_side_tasks() {
    let lines = run_sub_run(&SubRun {
        title: "publish",
        setup: Setup::idle(),
        body: |w| {
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            assert!(reply.is_ok(), "{reply:?}");
            assert_eq!(w.side_task_spawns(), 1);
        },
    });
    assert!(
        lines
            .iter()
            .any(|l| l == "publish_recording_ready 1 -> true"),
        "{lines:#?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("-> Recording")),
        "{lines:#?}"
    );
}

// ---------------------------------------------------------------- site 103

fn enable_smart_destination(w: &Arc<World>) {
    let state = w.state();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.smart_destination_enabled = true;
    dictation.auto_paste = true;
    dictation.save_audio = false;
    dictation.save_transcript = false;
}

/// R103.0: a current capture with Smart destination enabled selects the
/// destination (`PDT:SDE`); a capture cancelled while the smart-destination
/// capture was in flight (R103.0') does not, and its late result is discarded.
#[test]
fn the_smart_destination_is_selected_only_for_a_current_capture() {
    let current = run_sub_run(&SubRun {
        title: "current capture",
        setup: Setup::idle(),
        body: |w| {
            enable_smart_destination(w);
            recording(w);
        },
    });
    assert!(
        current.iter().any(|l| l == "emit_smart_destination 1"),
        "{current:#?}"
    );
    let stale = run_sub_run(&SubRun {
        title: "cancelled during the capture",
        setup: Setup::idle(),
        body: |w| {
            enable_smart_destination(w);
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            w.close_gate("smart_capture");
            let mut start = w.start();
            assert!(poll_once(&mut start).is_pending());
            w.state().app_state.cancel_recording(1);
            w.open_gate("smart_capture");
            let reply = complete(&mut start);
            w.reply("start", &reply);
        },
    });
    assert!(
        stale
            .iter()
            .any(|l| l == "smart_destination.capture enabled=true"),
        "{stale:#?}"
    );
    assert!(
        !stale
            .iter()
            .any(|l| l.starts_with("emit_smart_destination")),
        "a stale capture selects no destination: {stale:#?}"
    );

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
                smart_auto_started(w);
                assert!(parked.lock().expect("slot").is_none(), "the listener ran");
                assert!(
                    DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the queue was drained"
                );
                assert_eq!(history_counts(w), (0, 0), "H1 and H2 are retired");
                w.open_gate("pipeline");
            },
        });
    });
}
