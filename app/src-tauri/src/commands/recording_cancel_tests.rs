//! Shell-level cancel-family tests (Stage 2a-3).
//!
//! Each scenario drives `cancel_native_recording_inner` through the Stage 0b
//! golden harness and asserts the exact trace from the cancel request onward.
//! The expected traces were captured on the pre-refactor base (the cancel
//! command inline): this file passes unchanged before and after the cancel
//! family moves behind `decide_*`.
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
