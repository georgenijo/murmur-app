//! Shell-level cancel-family tests (Stage 2a-3).
//!
//! Each scenario drives `cancel_native_recording_inner` through the Stage 0b
//! golden harness and asserts the exact trace from the cancel request onward.
//! The expected traces were captured on the pre-refactor base (the cancel
//! command inline): this file passes unchanged before and after the cancel
//! family moves behind `decide_*`.
#![cfg(test)]
use super::golden_harness::*;
use super::golden_tests::{recording, started};
use super::*;
use std::sync::Mutex as StdMutex;
use std::task::Poll;

fn trace(setup: Setup, body: fn(&Arc<World>)) -> Vec<String> {
    run_sub_run(&SubRun {
        title: "cancel",
        setup,
        body,
    })
}

const MARK: &str = "# cancel requested";

/// The cancel command, announced by a marker line so the trace can start there.
fn cancel_command(world: &Arc<World>, expected: Option<u64>) {
    world.note("cancel requested");
    let mut cancel = world.cancel(expected);
    let reply = complete(&mut cancel);
    world.reply_unit("cancel", &reply);
}

fn from_cancel(lines: Vec<String>) -> Vec<String> {
    let at = lines
        .iter()
        .position(|line| line == MARK)
        .expect("the scenario issues a cancel");
    lines[at + 1..].to_vec()
}

fn assert_trace(name: &str, setup: Setup, body: fn(&Arc<World>), expected: &[&str]) {
    let got = from_cancel(trace(setup, body));
    let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "cancel trace for {name}\n{}", got.join("\n"));
}

/// R205.2 idle: a no-op
#[test]
fn cancel_idle() {
    assert_trace(
        "idle",
        Setup::idle(),
        |w| cancel_command(w, None),
        &[
            "reply cancel Ok",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R205.1 over Idle: a stale expectation is a no-op
#[test]
fn cancel_idle_with_a_stale_expectation() {
    assert_trace(
        "idle-stale",
        Setup::idle(),
        |w| cancel_command(w, Some(5)),
        &[
            "reply cancel Ok",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R205.3 recovering: a no-op
#[test]
fn cancel_recovering() {
    assert_trace(
        "recovering",
        Setup {
            status: DictationStatus::Recovering,
            recording_id: 3,
        },
        |w| cancel_command(w, None),
        &[
            "reply cancel Ok",
            "= final status=Recovering processing=false context=none cancelled_through=0",
        ],
    );
}

/// R205.1 stale expected id: no terminal, nothing changes (PD-12)
#[test]
fn cancel_stale() {
    assert_trace(
        "stale",
        Setup {
            status: DictationStatus::Recording,
            recording_id: 2,
        },
        |w| {
            w.install_context(2);
            cancel_command(w, Some(1));
        },
        &[
            "reply cancel Ok",
            "= final status=Recording processing=false context=2 cancelled_through=0",
        ],
    );
}

/// R205.4 starting, capture cancel accepted
#[test]
fn cancel_starting() {
    assert_trace(
        "starting",
        Setup::idle(),
        |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            started(w);
            cancel_command(w, None);
            w.release_cancel_finish();
        },
        &[
            "supervisor.cancel 1 user",
            "supervisor.set_phase Recovering",
            "supervisor.notify 1 Recovering(user)",
            "~ status Starting -> Recovering",
            "~ context 1 cleared",
            "emit recording-status-changed \"recovering\"",
            "emit audio-recovery-started {\"reason\":\"user\",\"recordingId\":1}",
            "~ terminal 1 UserCancelledStarting/CancelledStarting",
            "~ perf-claim 1 {\"status\":\"cancelled\",\"stage\":\"captureFinalization\"}",
            "emit recording-cancelled {\"recordingId\":1}",
            "supervisor.hold_cancel_finish 1",
            "reply cancel Ok",
            "supervisor.release_cancel_finish 1",
            "supervisor.notify 1 Idle",
            "emit recording-status-changed \"idle\"",
            "~ status Recovering -> Idle",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R205.4 recording, capture cancel accepted
#[test]
fn cancel_recording() {
    assert_trace(
        "recording",
        Setup::idle(),
        |w| {
            recording(w);
            cancel_command(w, Some(1));
            w.release_cancel_finish();
        },
        &[
            "supervisor.cancel 1 user",
            "supervisor.set_phase Recovering",
            "supervisor.notify 1 Recovering(user)",
            "~ status Recording -> Recovering",
            "~ context 1 cleared",
            "emit recording-status-changed \"recovering\"",
            "emit audio-recovery-started {\"reason\":\"user\",\"recordingId\":1}",
            "~ terminal 1 UserCancelledRecording/CancelledRecording",
            "~ perf-claim 1 {\"status\":\"cancelled\",\"stage\":\"captureFinalization\"}",
            "emit recording-cancelled {\"recordingId\":1}",
            "supervisor.hold_cancel_finish 1",
            "reply cancel Ok",
            "supervisor.release_cancel_finish 1",
            "supervisor.notify 1 Idle",
            "emit recording-status-changed \"idle\"",
            "~ status Recovering -> Idle",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R205.4 + R206.2: the capture cancel fails
#[test]
fn cancel_recording_err() {
    assert_trace(
        "recording-err",
        Setup::idle(),
        |w| {
            recording(w);
            w.knobs().cancel_result = Err("boom");
            cancel_command(w, None);
        },
        &[
            "supervisor.cancel 1 user",
            "~ terminal 1 PipelineFailure/RuntimeFailure",
            "reply cancel Err boom",
            "= final status=Recording processing=false context=1 cancelled_through=0",
        ],
    );
}

/// R205.4 + R206.2 while Starting
#[test]
fn cancel_starting_err() {
    assert_trace(
        "starting-err",
        Setup::idle(),
        |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            started(w);
            w.knobs().cancel_result = Err("boom");
            cancel_command(w, None);
            // The failed cancel left the start in flight: the readiness it was
            // waiting for lands now.
            w.release_ready();
        },
        &[
            "supervisor.cancel 1 user",
            "~ terminal 1 PipelineFailure/RuntimeFailure",
            "reply cancel Err boom",
            "supervisor.release_ready 1",
            "supervisor.set_phase Recording",
            "supervisor.notify 1 Ready",
            "emit recording-status-changed \"recording\"",
            "~ status Starting -> Recording",
            "SD ticker 1 registered=true",
            "= final status=Recording processing=false context=1 cancelled_through=0",
        ],
    );
}

/// R205.5 processing, no expectation
#[test]
fn cancel_processing() {
    assert_trace(
        "processing",
        Setup {
            status: DictationStatus::Processing,
            recording_id: 3,
        },
        |w| {
            w.install_context(3);
            cancel_command(w, None);
        },
        &[
            "~ status Processing -> Idle",
            "~ context 3 cleared",
            "~ cancelled_through 3",
            "keyboard.set_processing false",
            "emit recording-status-changed \"idle\"",
            "emit recording-cancelled {\"recordingId\":3}",
            "performance.complete 3 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
            "~ perf-claim 3 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
            "reply cancel Ok",
            "= final status=Idle processing=false context=none cancelled_through=3",
        ],
    );
}

/// R205.5 processing with a matching expectation
#[test]
fn cancel_processing_with_the_matching_expectation() {
    assert_trace(
        "processing-expected",
        Setup {
            status: DictationStatus::Processing,
            recording_id: 3,
        },
        |w| {
            w.install_context(3);
            cancel_command(w, Some(3));
        },
        &[
            "~ status Processing -> Idle",
            "~ context 3 cleared",
            "~ cancelled_through 3",
            "keyboard.set_processing false",
            "emit recording-status-changed \"idle\"",
            "emit recording-cancelled {\"recordingId\":3}",
            "performance.complete 3 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
            "~ perf-claim 3 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
            "reply cancel Ok",
            "= final status=Idle processing=false context=none cancelled_through=3",
        ],
    );
}

// ------------------------------------------- site 206: the capture-cancel result

/// A hook that proves T is free while the capture cancel (`CC`) runs: the cancel
/// family releases T before `PT:CC` (R205.4 / R201.4), so a supervisor that
/// blocks for up to 2 s never holds up the next transition.
pub(super) fn t_is_free_in_cc(world: &Arc<World>) -> impl FnOnce() + Send + 'static {
    let world = Arc::clone(world);
    move || {
        let free = world
            .state()
            .app_state
            .recording_transition
            .try_lock()
            .is_ok();
        world.note(&format!("T free inside CC: {free}"));
    }
}

/// A hook for the supervisor's own terminal: it claims `UserCancelledStarting`
/// for recording 1 while the capture cancel is still in flight (before the
/// command gives up on the ack), so the ledger winner is the supervisor's. It
/// also proves T is free inside `CC`, as [`t_is_free_in_cc`] does, so the trace
/// fails if the command held T across the capture cancel.
pub(super) fn supervisor_claims_first(world: &Arc<World>) -> impl FnOnce() + Send + 'static {
    let world = Arc::clone(world);
    let probe = t_is_free_in_cc(&world);
    move || {
        let state = world.state();
        emit_dictation_terminal(
            state.inner(),
            1,
            DictationTerminalOutcome::UserCancelledStarting,
            DictationErrorCode::CancelledStarting,
        );
        probe();
    }
}

/// Forget the previous scenario's site 206 witness (the slot is thread-local).
pub(super) fn forget_206_witness() {
    cancel_ports::LAST_206_WITNESS.with(|slot| slot.borrow_mut().clear());
}

/// Site 206's witness as one trace line: `TERM Failed` is the real result of a
/// lost claim, `TERM Ran` a won one, and `none` a decision with no effect.
pub(super) fn note_206_witness(world: &Arc<World>) {
    let witness = cancel_ports::LAST_206_WITNESS.with(|slot| slot.borrow().clone());
    let text = if witness.is_empty() {
        "none".to_string()
    } else {
        witness
            .iter()
            .map(|entry| format!("{} {:?}", entry.code, entry.outcome))
            .collect::<Vec<_>>()
            .join(", ")
    };
    world.note(&format!("206 witness: {text}"));
}

/// R205.4 + R206.1: the supervisor answers `Ok(false)` (nothing to cancel), so no
/// `Recovering` follows; the command answers `Ok` and changes nothing.
#[test]
fn cancel_recording_not_accepted() {
    assert_trace(
        "recording-not-accepted",
        Setup::idle(),
        |w| {
            recording(w);
            w.knobs().cancel_result = Ok(false);
            w.on("supervisor.cancel", t_is_free_in_cc(w));
            cancel_command(w, None);
        },
        &[
            "supervisor.cancel 1 user",
            "# T free inside CC: true",
            "reply cancel Ok",
            "= final status=Recording processing=false context=1 cancelled_through=0",
        ],
    );
}

/// R205.4 + R206.1 while Starting
#[test]
fn cancel_starting_not_accepted() {
    assert_trace(
        "starting-not-accepted",
        Setup::idle(),
        |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            started(w);
            w.knobs().cancel_result = Ok(false);
            w.on("supervisor.cancel", t_is_free_in_cc(w));
            cancel_command(w, None);
            w.release_ready();
        },
        &[
            "supervisor.cancel 1 user",
            "# T free inside CC: true",
            "reply cancel Ok",
            "supervisor.release_ready 1",
            "supervisor.set_phase Recording",
            "supervisor.notify 1 Ready",
            "emit recording-status-changed \"recording\"",
            "~ status Starting -> Recording",
            "SD ticker 1 registered=true",
            "= final status=Recording processing=false context=1 cancelled_through=0",
        ],
    );
}

/// R206.2 over a start that the supervisor cancelled first: the supervisor holds
/// the ledger (`UserCancelledStarting`), the command's `PipelineFailure` claim
/// loses (no second terminal), and the error text is unchanged.
#[test]
fn cancel_starting_err_after_the_supervisor_claimed_first() {
    assert_trace(
        "starting-err-claimed",
        Setup::idle(),
        |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            started(w);
            w.knobs().cancel_result = Err("boom");
            w.on("supervisor.cancel", supervisor_claims_first(w));
            cancel_command(w, None);
            w.release_ready();
        },
        &[
            "supervisor.cancel 1 user",
            "~ terminal 1 UserCancelledStarting/CancelledStarting",
            "# T free inside CC: true",
            "reply cancel Err boom",
            "supervisor.release_ready 1",
            "supervisor.set_phase Recording",
            "supervisor.notify 1 Ready",
            "emit recording-status-changed \"recording\"",
            "~ status Starting -> Recording",
            "SD ticker 1 registered=true",
            "= final status=Recording processing=false context=1 cancelled_through=0",
        ],
    );
}

/// The same scenario with the witness: the lost claim is recorded `Failed`, a
/// won one `Ran`, a no-effect decision has an empty witness.
#[test]
fn cancel_records_the_real_206_claim_result() {
    let witnessed = |setup: Setup, body: fn(&Arc<World>)| {
        let lines = from_cancel(trace(setup, body));
        lines
            .into_iter()
            .find(|line| line.starts_with("# 206 witness"))
            .expect("the scenario notes the witness")
    };
    let lost = witnessed(Setup::idle(), |w| {
        w.knobs().cold_start = ColdStart::ReadyAfterContext;
        started(w);
        w.knobs().cancel_result = Err("boom");
        w.on("supervisor.cancel", supervisor_claims_first(w));
        forget_206_witness();
        cancel_command(w, None);
        note_206_witness(w);
        w.release_ready();
    });
    assert_eq!(lost, "# 206 witness: TERM Failed");
    let won = witnessed(Setup::idle(), |w| {
        recording(w);
        w.knobs().cancel_result = Err("boom");
        forget_206_witness();
        cancel_command(w, None);
        note_206_witness(w);
    });
    assert_eq!(won, "# 206 witness: TERM Ran");
    let none = witnessed(Setup::idle(), |w| {
        recording(w);
        w.knobs().cancel_result = Ok(false);
        forget_206_witness();
        cancel_command(w, None);
        note_206_witness(w);
    });
    assert_eq!(none, "# 206 witness: none");
}

// ------------------------------------------------------------ re-entrancy

/// What the nested cancel returned when it was polled once: `None` while it was
/// still pending (parked on T or D), `Some(result)` once it completed.
static NESTED: StdMutex<Option<Option<Result<(), String>>>> = StdMutex::new(None);

/// A hook (registered with `on_inline`) that enters the cancel command again, on the thread inside the
/// outer cancel's executor span.
fn nested_cancel(world: &Arc<World>) -> impl FnOnce() + Send + 'static {
    let world = Arc::clone(world);
    move || {
        let mut nested = world.cancel(None);
        let outcome = match poll_once(&mut nested) {
            Poll::Ready(result) => Some(result),
            Poll::Pending => None,
        };
        *NESTED.lock().unwrap() = Some(outcome);
    }
}

fn run_with_watchdog(setup: Setup, body: fn(&Arc<World>)) -> Vec<String> {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(|| trace(setup, body));
        let _ = done.send(outcome.map_err(|_| "panicked"));
    });
    match finished.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(Ok(lines)) => lines,
        Ok(Err(why)) => panic!("the scenario {why}"),
        Err(_) => panic!("deadlocked: the nested cancel asked for a lock before refusing"),
    }
}

/// C4.2 / decision D-S2-2: the cancel entry is the first statement, before T, D
/// or `core_history`, and `IN_CORE` stays set across production port calls, so a
/// callback that a port or an emit runs inline cannot re-enter the cancel. Two
/// inline points are exercised: the capture cancel of R205.4 (`PT`, T released)
/// and the keyboard clear of R205.5 (`PDT`, T held, so a nested entry that did
/// not refuse first would park on T). Each runs under a watchdog.
#[test]
fn a_cancel_nested_in_a_port_call_is_refused_before_any_lock() {
    type Scenario = (&'static str, Setup, fn(&Arc<World>));
    let scenarios: [Scenario; 2] = [
        ("capture cancel (PT:CC)", Setup::idle(), |w| {
            recording(w);
            w.on_inline("supervisor.cancel", nested_cancel(w));
            cancel_command(w, None);
            w.release_cancel_finish();
        }),
        (
            "keyboard clear (PDT:KB)",
            Setup {
                status: DictationStatus::Processing,
                recording_id: 3,
            },
            |w| {
                w.install_context(3);
                w.on_inline("keyboard.set_processing:false", nested_cancel(w));
                cancel_command(w, None);
            },
        ),
    ];
    for (name, setup, body) in scenarios {
        *NESTED.lock().unwrap() = None;
        let lines = from_cancel(run_with_watchdog(setup, body));
        let nested = NESTED
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| panic!("{name}: the hook never ran"));
        match nested {
            Some(Err(refusal)) => assert!(
                refusal.contains("refused the cancel") && refusal.contains("Reentrant"),
                "{name}: {refusal}"
            ),
            other => panic!("{name}: the nested cancel was not refused at once: {other:?}"),
        }
        // The nested entry did nothing: one capture cancel at most, and the outer
        // cancel's own effects, once.
        let count = |needle: &str| lines.iter().filter(|l| l.starts_with(needle)).count();
        assert!(
            count("supervisor.cancel") <= 1,
            "{name}\n{}",
            lines.join("\n")
        );
        assert!(count("~ terminal") <= 1, "{name}\n{}", lines.join("\n"));
        assert_eq!(count("reply cancel Ok"), 1, "{name}\n{}", lines.join("\n"));
    }
}

// ----------------------------------------------- D-S2-8: the cancel's drain guard

fn history_counts(world: &Arc<World>) -> (usize, usize) {
    let history = &world.state().app_state.core_history;
    (history.pipeline_count(), history.outstanding_ops())
}

fn run_under_watchdog<T: Send + 'static>(
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

/// R205.5's `idle` emit runs listeners inline, under the cancel's span with T held.
/// A listener drops a stop that is parked mid-pipeline: its idle guard's finish
/// (the 204a section) and the pipeline's history entries H1 and H2 are deferred, in
/// FIFO order. The cancel command drains them before it returns: nothing is applied
/// while it still holds T, the finish and both retirements are applied once, and
/// the queue is empty. Without the cancel's drain guard they would stay queued on
/// this thread (and the test-build safety net fails the command).
#[test]
fn a_cancel_drains_what_its_idle_listener_defers_before_it_returns() {
    let lines = run_under_watchdog("cancel with a listener that drops a stop", || {
        from_cancel(trace(Setup::idle(), |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            assert_eq!(history_counts(w), (1, 1), "the parked stop's pipeline");
            let counters = w.state().app_state.core_history.counters();
            let parked = Arc::new(StdMutex::new(Some(stop)));
            let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
            w.on_inline("emit:recording-status-changed", move || {
                observer.note("listener: dropping the parked stop under the cancel's span");
                drop(slot.lock().expect("slot").take());
                assert_eq!(
                    history_counts(&observer),
                    (1, 1),
                    "nothing retires while the cancel's span is open"
                );
                assert_eq!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().len()),
                    1,
                    "the dropped stop's finish is queued"
                );
                observer.note("listener: dropped");
            });
            cancel_command(w, None);
            assert!(parked.lock().expect("slot").is_none(), "the listener ran");
            assert_eq!(
                history_counts(w),
                (0, 0),
                "the cancel drained H1 and H2 before it returned"
            );
            assert_eq!(
                w.state().app_state.core_history.counters(),
                counters,
                "each entry retired exactly once"
            );
            assert!(
                super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                "the queue is empty when the cancel returns"
            );
            w.open_gate("pipeline");
        }))
    });
    let listener = lines
        .iter()
        .position(|line| line == "# listener: dropped")
        .unwrap_or_else(|| panic!("the listener ran:\n{}", lines.join("\n")));
    let reply = lines
        .iter()
        .position(|line| line == "reply cancel Ok")
        .unwrap_or_else(|| panic!("the cancel replied:\n{}", lines.join("\n")));
    assert!(
        listener < reply,
        "the drain ran inside the cancel, before its reply:\n{}",
        lines.join("\n")
    );
}

/// The counted drain guard follows the thread that runs the synchronous section
/// (D-S2-13). A cancel parks on T on worker A and resumes on worker B; the idle
/// listener queues a stop's finish there. B has a guard alive (it registered after
/// the post-T fence), the finish and both retirements drain on B with T acquirable,
/// and worker A is left with no stale count that could hide a missing guard.
#[test]
fn a_cancel_that_resumes_on_another_worker_registers_its_drain_there() {
    run_under_watchdog("a cancel resumed on another worker", || {
        trace(Setup::idle(), |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            assert_eq!(history_counts(w), (1, 1), "the parked stop's pipeline");
            let parked = Arc::new(StdMutex::new(Some(stop)));
            let slot = Arc::clone(&parked);
            w.on_inline("emit:recording-status-changed", move || {
                drop(slot.lock().expect("slot").take());
            });
            let alive_before = super::drains_alive_on_this_thread();
            let state = w.state();
            let holder = state.app_state.recording_transition.try_lock().expect("T");
            let mut cancel = w.cancel(None);
            assert!(poll_once(&mut cancel).is_pending(), "parked on T");
            assert_eq!(
                super::drains_alive_on_this_thread(),
                alive_before,
                "a cancel parked on T holds no drain registration on this thread"
            );
            drop(holder);
            super::take_t_free_at_drain();
            let (reply, t_free) = std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        let reply = complete(&mut cancel);
                        (reply, super::take_t_free_at_drain())
                    })
                    .join()
                    .expect("the second worker (no `no drain guard alive` panic)")
            });
            assert_eq!(reply, Ok(()));
            assert!(parked.lock().expect("slot").is_none(), "the listener ran");
            assert_eq!(t_free, [true], "the finish drained, with T acquirable");
            assert_eq!(history_counts(w), (0, 0), "H1 and H2 retired once");
            assert!(
                super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                "nothing is left queued on this thread"
            );
            assert_eq!(
                super::drains_alive_on_this_thread(),
                alive_before,
                "no stale count is left on the first worker"
            );
            w.open_gate("pipeline");
        })
    });
}

/// A cancel that waits on T and is woken by a listener that polls it inline, inside
/// another entry's span, runs on to its D section there only after the span ended:
/// it parks (holding T, self-waking) instead of asking for D inside the span, and
/// finishes when it is next polled outside it. A watchdog fails a deadlock.
#[test]
fn a_cancel_waiting_on_t_and_polled_inside_a_span_parks_without_deadlock() {
    use crate::dictation_executor::executor::CoreEntry;
    run_under_watchdog("a cancel polled inside a span after waiting on T", || {
        trace(
            Setup {
                status: DictationStatus::Processing,
                recording_id: 3,
            },
            |w| {
                w.install_context(3);
                let state = w.state();
                // Someone else holds T, so the cancel parks on it.
                let holder = state.app_state.recording_transition.try_lock().expect("T");
                let mut cancel = w.cancel(None);
                assert!(poll_once(&mut cancel).is_pending(), "parked on T");
                drop(holder);
                // Polled inside a span: T is acquired, then the fence parks it.
                let span = CoreEntry::enter().expect("outside any span");
                assert!(
                    poll_once(&mut cancel).is_pending(),
                    "inside a span the cancel parks before its D section"
                );
                assert!(
                    state.app_state.dictation.try_lock().is_ok(),
                    "the parked cancel never asked for D inside the span"
                );
                assert_eq!(
                    state.app_state.dictation.lock_or_recover().status,
                    DictationStatus::Processing,
                    "nothing ran inside the span"
                );
                drop(span);
                let reply = complete(&mut cancel);
                assert_eq!(reply, Ok(()), "outside the span the cancel finishes");
                assert_eq!(
                    state.app_state.dictation.lock_or_recover().status,
                    DictationStatus::Idle
                );
                assert!(state.app_state.recording_transition.try_lock().is_ok());
            },
        )
    });
}

// ------------------------------------------- pre-cutover: the tail's order and fence

/// The effects a cancel's tail ran, each with what the test saw just before it.
type TailSeen = Arc<StdMutex<Vec<(&'static str, String)>>>;

/// What a test can see of the cancel's tail at one barrier, before the effect named
/// there runs.
fn tail_state(world: &Arc<World>, id: u64) -> String {
    let state = world.state();
    let status = state
        .app_state
        .dictation
        .try_lock_or_recover()
        .map_or_else(|| "D-held".to_string(), |d| format!("{:?}", d.status));
    let trace = world.trace();
    let emitted = |needle: &str| trace.iter().any(|line| line.starts_with(needle));
    format!(
        "status={status} context={} cancelled={} terminal={} kb_processing={} idle_emitted={} cancelled_emitted={} transition={}",
        if state.app_state.active_context(id).is_some() {
            "present"
        } else {
            "cleared"
        },
        state.app_state.is_cancelled(id),
        state.app_state.dictation_telemetry.terminal_for_test(id).is_some(),
        keyboard::is_processing(),
        emitted("emit recording-status-changed \"idle\""),
        emitted("emit recording-cancelled"),
        // The cancel's own thread holds T at the barriers from `ST` through `EV`.
        if state.app_state.recording_transition.try_lock().is_err() {
            "held"
        } else {
            "free"
        },
    )
}

/// R205.5's exact order (appendix C: `In:ST→Idle`; `PDT:CX-, MC, TERM, KB, EM, EV`;
/// `PT:PC`). A barrier before each effect observes what the earlier effects did and
/// what the later ones have not yet done, so each statement of the tail is pinned
/// to its place, not only the first port after it (`cancel_window_preserved`).
#[test]
fn the_processing_cancel_tail_runs_in_r205_5_order() {
    let seen: TailSeen = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    TAIL_SINK.with(|slot| *slot.borrow_mut() = Some(sink));
    run_sub_run(&SubRun {
        title: "cancel tail order",
        setup: Setup {
            status: DictationStatus::Processing,
            recording_id: 3,
        },
        body: |w| {
            w.install_context(3);
            w.accept_silently(3);
            keyboard::set_processing(true);
            let sink = TAIL_SINK
                .with(|slot| slot.borrow().clone())
                .expect("the test installed its sink");
            let observer = Arc::clone(w);
            let _barrier = crate::barrier_hook::install(move |point| {
                // The executor's legend codes only (`ST`, `CX-`, ...).
                if point
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '+' || c == '-')
                {
                    let state = tail_state(&observer, 3);
                    sink.lock().expect("sink").push((point, state));
                }
            });
            cancel_command(w, None);
        },
    });
    TAIL_SINK.with(|slot| *slot.borrow_mut() = None);
    let seen = seen.lock().expect("sink").clone();
    let codes: Vec<&str> = seen.iter().map(|(code, _)| *code).collect();
    assert_eq!(
        codes,
        ["ST", "CX-", "MC", "TERM", "KB", "EM", "EV", "PC"],
        "R205.5's effect order: {seen:#?}"
    );
    let state = |index: usize| seen[index].1.as_str();
    // R205.5's transition-lock boundary: T is held from `ST` through `EV`, so a newer
    // start cannot publish `starting` before this cancel's trailing `idle`, and it is
    // released only before `PT:PC`.
    for (index, (code, state)) in seen.iter().enumerate() {
        let expected = if index < 7 {
            "transition=held"
        } else {
            "transition=free"
        };
        assert!(
            state.ends_with(expected),
            "T must be {expected} at {code} (R205.5 releases it before PT:PC): {state}"
        );
    }
    // Each effect has not run at its own barrier and every earlier one has.
    assert!(
        state(0).starts_with(
            "status=D-held context=present cancelled=false terminal=false kb_processing=true"
        ),
        "{}",
        state(0)
    );
    assert!(
        state(1).starts_with("status=Idle context=present cancelled=false terminal=false"),
        "ST ran, CX- has not: {}",
        state(1)
    );
    assert!(
        state(2).starts_with("status=Idle context=cleared cancelled=false terminal=false"),
        "CX- ran, MC has not: {}",
        state(2)
    );
    assert!(
        state(3).starts_with("status=Idle context=cleared cancelled=true terminal=false"),
        "MC ran, TERM has not: {}",
        state(3)
    );
    assert!(
        state(4).contains("terminal=true kb_processing=true idle_emitted=false"),
        "TERM ran, KB has not: {}",
        state(4)
    );
    assert!(
        state(5).contains("kb_processing=false idle_emitted=false"),
        "KB ran, EM has not: {}",
        state(5)
    );
    assert!(
        state(6).contains("idle_emitted=true cancelled_emitted=false"),
        "EM ran, EV has not: {}",
        state(6)
    );
    assert!(
        state(7).contains("idle_emitted=true cancelled_emitted=true"),
        "EV ran, PC has not: {}",
        state(7)
    );
}

thread_local! {
    static TAIL_SINK: std::cell::RefCell<Option<TailSeen>> = const { std::cell::RefCell::new(None) };
}

/// A processing cancel keeps the recording transition through its trailing `idle`
/// emit. If it released T first, a newer start could publish `starting` and the old
/// cancel's `idle` would then overwrite it (#1160, cancel). The cancel is parked
/// just before that emit, and a new start is polled from an independent task: it
/// stays blocked until the old emit has finished.
#[test]
fn a_new_start_stays_blocked_until_the_cancels_trailing_idle_is_emitted() {
    let lines = run_sub_run(&SubRun {
        title: "late idle",
        setup: Setup {
            status: DictationStatus::Processing,
            recording_id: 3,
        },
        body: |w| {
            w.install_context(3);
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            let mut parked = w.cancel_parked_before("EM");
            parked.wait_parked();
            w.note("the cancel is parked just before its trailing idle emit");
            let mut start = w.start();
            assert!(
                poll_once(&mut start).is_pending(),
                "the new start waits on the transition the cancel still holds"
            );
            assert!(
                !w.trace().iter().any(|line| line.contains("\"starting\"")),
                "no `starting` was published while the cancel was parked"
            );
            w.note("the cancel resumes");
            let reply = parked.release_and_join();
            w.reply_unit("cancel", &reply);
            let started = complete(&mut start);
            w.reply("start", &started);
        },
    });
    let position = |needle: &str| {
        lines
            .iter()
            .position(|line| line.starts_with(needle))
            .unwrap_or_else(|| panic!("no `{needle}` in {lines:#?}"))
    };
    assert!(
        position("emit recording-status-changed \"idle\"")
            < position("emit recording-status-changed \"starting\""),
        "the stale idle is emitted before the new starting, never after it: {lines:#?}"
    );
    assert!(
        position("reply cancel Ok") < position("emit recording-status-changed \"starting\""),
        "{lines:#?}"
    );
}
