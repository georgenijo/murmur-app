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
