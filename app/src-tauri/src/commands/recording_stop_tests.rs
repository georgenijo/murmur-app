//! Shell-level stop-family tests (Stage 2a-2).
//!
//! Each scenario drives `stop_native_recording_inner` through the Stage 0b
//! golden harness and asserts the exact trace from the stop request onward
//! (after the start reply when the scenario starts a recording). The
//! expected traces were captured on the pre-refactor base: this file passes
//! unchanged before and after the stop family moves behind `decide_*`.
use super::golden_harness::*;
use super::golden_tests::{cancelled, recording, started, stopped};
use super::*;
use std::sync::Mutex as StdMutex;
use std::task::Poll;

fn trace(setup: Setup, body: fn(&Arc<World>)) -> Vec<String> {
    run_sub_run(&SubRun {
        title: "stop",
        setup,
        body,
    })
}

/// The lines after the `reply start` line, or the whole trace when the
/// scenario starts no recording through the shell.
fn from_stop(lines: Vec<String>) -> Vec<String> {
    match lines.iter().position(|l| l.starts_with("reply start")) {
        Some(at) => lines[at + 1..].to_vec(),
        None => lines,
    }
}

fn assert_trace(name: &str, setup: Setup, body: fn(&Arc<World>), expected: &[&str]) {
    let got = from_stop(trace(setup, body));
    let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want, "stop trace for {name}\n{}", got.join("\n"));
}

/// R201.1 stale expected id: observe, supersede, no core write
#[test]
fn stop_stale() {
    assert_trace(
        "stale",
        Setup {
            status: DictationStatus::Recording,
            recording_id: 2,
        },
        |w| {
            w.install_context(2);
            w.accept_silently(1);
            stopped(w, "stop", Some(1));
        },
        &[
            "~ context 2 installed",
            "supervisor.take_interrupted 1 -> none",
            "~ terminal 1 Superseded/StaleOwner",
            "reply stop Ok {\"state\":\"idle\",\"type\":\"recording_superseded\"}",
            "= final status=Recording processing=false context=2 cancelled_through=0",
        ],
    );
}

/// R201.2 not recording
#[test]
fn stop_idle() {
    assert_trace(
        "idle",
        Setup::idle(),
        |w| stopped(w, "stop", None),
        &[
            "reply stop Ok {\"state\":\"idle\",\"type\":\"not_recording\"}",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R201.3 already processing
#[test]
fn stop_processing() {
    assert_trace(
        "processing",
        Setup {
            status: DictationStatus::Processing,
            recording_id: 3,
        },
        |w| stopped(w, "stop", None),
        &[
            "reply stop Ok {\"state\":\"processing\",\"type\":\"already_processing\"}",
            "= final status=Processing processing=false context=none cancelled_through=0",
        ],
    );
}

/// R203 empty audio
#[test]
fn stop_zero() {
    assert_trace(
        "zero",
        Setup::idle(),
        |w| {
            recording(w);
            w.knobs().stop_result = Ok(0);
            stopped(w, "stop", None);
        },
        &[
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recording -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "keyboard.set_processing true",
            "~ processing true",
            "emit recording-status-changed \"processing\"",
            "supervisor.stop 1 Other",
            "supervisor.set_phase Stopping",
            "supervisor.sealed_delivery 1 samples=0",
            "performance.production_sample_count 1 0",
            "~ processing false",
            "~ context 1 cleared",
            "~ terminal 1 NoSpeech/EmptyAudio",
            "emit recording-status-changed \"idle\"",
            "~ status Processing -> Idle",
            "~ perf-claim 1 {\"status\":\"noSpeech\"}",
            "reply stop Ok {\"state\":\"idle\",\"text\":\"\",\"type\":\"transcription\"}",
            "# end of run: the supervisor finishes the sealed stop",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "supervisor.notify 1 CleanupFinished",
            "emit recording-cleanup-finished {\"recordingId\":1}",
            "emit recording-status-changed \"idle\"",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R203 below minimum duration
#[test]
fn stop_short() {
    assert_trace(
        "short",
        Setup::idle(),
        |w| {
            recording(w);
            w.knobs().stop_result = Ok(100);
            stopped(w, "stop", None);
        },
        &[
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recording -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "keyboard.set_processing true",
            "~ processing true",
            "emit recording-status-changed \"processing\"",
            "supervisor.stop 1 Other",
            "supervisor.set_phase Stopping",
            "supervisor.sealed_delivery 1 samples=100",
            "performance.production_sample_count 1 100",
            "~ processing false",
            "~ context 1 cleared",
            "~ terminal 1 TooShort/BelowMinimumDuration",
            "emit recording-status-changed \"idle\"",
            "~ status Processing -> Idle",
            "~ perf-claim 1 {\"status\":\"noSpeech\"}",
            "reply stop Ok {\"state\":\"idle\",\"text\":\"\",\"type\":\"transcription\"}",
            "# end of run: the supervisor finishes the sealed stop",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "supervisor.notify 1 CleanupFinished",
            "emit recording-cleanup-finished {\"recordingId\":1}",
            "emit recording-status-changed \"idle\"",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R203 stop finalization failure
#[test]
fn stop_err() {
    assert_trace(
        "err",
        Setup::idle(),
        |w| { recording(w); w.knobs().stop_result = Err("boom"); stopped(w, "stop", None); },
        &[
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recording -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "keyboard.set_processing true",
            "~ processing true",
            "emit recording-status-changed \"processing\"",
            "supervisor.stop 1 Other",
            "supervisor.set_phase Stopping",
            "~ processing false",
            "~ context 1 cleared",
            "~ terminal 1 StopFailure/StopFinalizationFailed",
            "emit recording-status-changed \"idle\"",
            "~ status Processing -> Idle",
            "performance.complete 1 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"audioCaptureFailed\"}",
            "~ perf-claim 1 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"audioCaptureFailed\"}",
            "reply stop Err boom",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R203 pipeline success then guard finish, idle emit
#[test]
fn stop_long() {
    assert_trace(
        "long",
        Setup::idle(),
        |w| { recording(w); w.knobs().stop_result = Ok(10_000); stopped(w, "stop", None); },
        &[
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recording -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "keyboard.set_processing true",
            "~ processing true",
            "emit recording-status-changed \"processing\"",
            "supervisor.stop 1 Other",
            "supervisor.set_phase Stopping",
            "supervisor.sealed_delivery 1 samples=10000",
            "performance.production_sample_count 1 10000",
            "pipeline 1 samples=10000",
            "pipeline 1 returns",
            "~ status Processing -> Idle",
            "~ processing false",
            "~ context 1 cleared",
            "pipeline 1 idle_guard wrote_idle=true",
            "keyboard.set_processing false",
            "emit recording-status-changed \"idle\"",
            "~ terminal 1 Success/None",
            "~ perf-claim 1 {\"status\":\"success\"}",
            "emit transcription-complete {\"duration\":0,\"interrupted\":null,\"rawText\":\"scripted transcript\",\"recording\":{\"modelId\":\"base.en\",\"profileId\":null,\"recordingId\":1,\"stages\":[]},\"recordingId\":1,\"text\":\"scripted transcript\"}",
            "reply stop Ok {\"state\":\"idle\",\"text\":\"scripted transcript\",\"type\":\"transcription\"}",
            "# end of run: the supervisor finishes the sealed stop",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "supervisor.notify 1 CleanupFinished",
            "emit recording-cleanup-finished {\"recordingId\":1}",
            "emit recording-status-changed \"idle\"",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R201 stop during Starting
#[test]
fn stop_starting() {
    assert_trace(
        "starting",
        Setup::idle(),
        |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            started(w);
            stopped(w, "stop", None);
            w.release_cancel_finish();
        },
        &[
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
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
            "reply stop Ok {\"state\":\"recovering\",\"type\":\"recording_cancelled\"}",
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

/// R202 missing context
#[test]
fn stop_nocontext() {
    assert_trace(
        "nocontext",
        Setup { status: DictationStatus::Recording, recording_id: 1 },
        |w| { w.accept_silently(1); stopped(w, "stop", None); },
        &[
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recording -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "~ terminal 1 PipelineFailure/MissingContext",
            "keyboard.set_processing false",
            "emit recording-status-changed \"idle\"",
            "performance.complete 1 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"internalEarlyExit\"}",
            "~ status Processing -> Idle",
            "~ perf-claim 1 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"internalEarlyExit\"}",
            "reply stop Err Missing dictation context for recording 1",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R201 recovering with no interrupted capture
#[test]
fn stop_recovering_none() {
    assert_trace(
        "recovering_none",
        Setup::idle(),
        |w| {
            recording(w);
            w.runtime_failure(1);
            stopped(w, "stop", None);
        },
        &[
            "supervisor.set_phase Recovering",
            "supervisor.notify 1 Recovering(runtime_failure)",
            "~ status Recording -> Recovering",
            "emit recording-status-changed \"recovering\"",
            "emit audio-recovery-started {\"reason\":\"runtime_failure\",\"recordingId\":1}",
            "supervisor.take_interrupted 1 -> none",
            "reply stop Ok {\"state\":\"recovering\",\"type\":\"audio_recovering\"}",
            "= final status=Recovering processing=false context=1 cancelled_through=0",
        ],
    );
}

/// R201 recovering with interrupted capture
#[test]
fn stop_recovering_capture() {
    assert_trace(
        "recovering_capture",
        Setup::idle(),
        |w| { recording(w); w.runtime_failure(1); w.finish_interrupted(1, 9000); stopped(w, "stop", None); w.release_owner_clear(); },
        &[
            "supervisor.set_phase Recovering",
            "supervisor.notify 1 Recovering(runtime_failure)",
            "~ status Recording -> Recovering",
            "emit recording-status-changed \"recovering\"",
            "emit audio-recovery-started {\"reason\":\"runtime_failure\",\"recordingId\":1}",
            "supervisor.notify 1 Interrupted(runtime_failure, 9000 samples)",
            "emit recording-interrupted {\"actionCode\":\"wait_for_partial_transcription\",\"autoTranscribe\":true,\"deliveredSamples\":9000,\"durationMs\":562,\"reason\":\"runtime_failure\",\"recordingId\":1,\"statusCode\":\"microphone_interrupted\"}",
            "bridge.auto_stop_scheduled 1",
            "supervisor.hold_before_owner_clear 1",
            "supervisor.take_interrupted 1 -> 9000 samples",
            "claim_preview_hide 1",
            "ax.capture_delivery_target_snapshot",
            "~ status Recovering -> Processing",
            "keyboard.hotkey_release_take_for 1",
            "keyboard.stop_gesture_take_for 1",
            "keyboard.set_processing true",
            "~ processing true",
            "emit recording-status-changed \"processing\"",
            "performance.production_sample_count 1 9000",
            "pipeline 1 samples=9000",
            "pipeline 1 returns",
            "~ status Processing -> Idle",
            "~ processing false",
            "~ context 1 cleared",
            "pipeline 1 idle_guard wrote_idle=true",
            "keyboard.set_processing false",
            "emit recording-status-changed \"idle\"",
            "~ terminal 1 RuntimeInterruption/StreamInvalidated",
            "~ perf-claim 1 {\"status\":\"success\"}",
            "emit transcription-complete {\"duration\":0,\"interrupted\":{\"deliveredSamples\":9000,\"durationMs\":562,\"reason\":\"stream_invalidated\"},\"rawText\":\"scripted transcript\",\"recording\":{\"modelId\":\"base.en\",\"profileId\":null,\"recordingId\":1,\"stages\":[]},\"recordingId\":1,\"text\":\"scripted transcript\"}",
            "reply stop Ok {\"state\":\"idle\",\"text\":\"scripted transcript\",\"type\":\"transcription\"}",
            "supervisor.clear_owner 1",
            "supervisor.set_phase Idle",
            "= final status=Idle processing=false context=none cancelled_through=0",
        ],
    );
}

/// R204b.4: a newer recording started while the first one's pipeline was still
/// running. The stop's own idle section compares the recording id and stays
/// silent: no `idle` emit and no keyboard-gate clear for recording 1 after its
/// pipeline returns. (The pipeline guard's separate section, 204a, is fenced the
/// same way: it leaves recording 2's status alone.)
#[test]
fn stop_idle_emit_after_the_pipeline_is_fenced_by_the_recording_id() {
    let lines = from_stop(trace(Setup::idle(), |world| {
        recording(world);
        world.knobs().stop_result = Ok(16_000);
        world.close_gate("pipeline");
        let mut stop = world.stop(None);
        assert!(poll_once(&mut stop).is_pending());
        cancelled(world, "cancel");
        world.finish_sealed_stop(1);
        recording(world);
        world.open_gate("pipeline");
        let reply = complete(&mut stop);
        world.reply("stop", &reply);
    }));
    let at = lines
        .iter()
        .position(|line| line == "pipeline 1 returns")
        .expect("the pipeline returned");
    assert_eq!(
        lines[at..].to_vec(),
        [
            "pipeline 1 returns",
            // The pipeline's own guard (204a) finds a newer recording: it clears
            // nothing and writes nothing.
            "pipeline 1 idle_guard wrote_idle=false",
            // 204b: no `idle` emit and no keyboard-gate clear for recording 1.
            "reply stop Ok {\"state\":\"idle\",\"text\":\"\",\"type\":\"transcription\"}",
            "= final status=Recording processing=false context=2 cancelled_through=1",
        ]
    );
}

// ------------------------------------------------------- Stage 2a-2 review fixes

/// Run `body` on its own thread under a watchdog: a regression that asks for a
/// lock the thread already holds hangs forever, and a hung golden lock would hang
/// every later scenario, so the watchdog aborts the run with the reason instead.
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

thread_local! {
    /// What each nested entry returned, in order (on the scenario's own thread).
    static NESTED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// An inline listener re-enters the stop command on the thread of the port call.
fn nested_stop(world: &Arc<World>) {
    let mut stop = world.stop(None);
    let outcome = match poll_once(&mut stop) {
        Poll::Ready(reply) => format!("{reply:?}"),
        Poll::Pending => "pending".to_string(),
    };
    NESTED.with(|nested| nested.borrow_mut().push(outcome));
}

/// An inline listener re-enters the lifecycle bridge on the thread of the port call.
fn nested_bridge(world: &Arc<World>) {
    world.bridge_inline(1, AudioLifecycleEvent::CleanupFinished);
    NESTED.with(|nested| nested.borrow_mut().push("bridge returned".to_string()));
}

/// Arm `nested` on the `n`th status emit after now (the earlier ones do nothing).
fn arm_on_status_emit(world: &Arc<World>, n: usize, nested: fn(&Arc<World>)) {
    for index in 1..=n {
        let world_for_hook = Arc::clone(world);
        world.on_inline("emit:recording-status-changed", move || {
            if index == n {
                nested(&world_for_hook);
            }
        });
    }
}

/// The scenario's trace and what the nested entries returned.
fn run_nested(
    what: &'static str,
    setup: fn() -> Setup,
    body: fn(&Arc<World>),
) -> (Vec<String>, Vec<String>) {
    under_watchdog(what, move || {
        NESTED.with(|nested| nested.borrow_mut().clear());
        let lines = from_stop(trace(setup(), body));
        (lines, NESTED.with(|nested| nested.borrow().clone()))
    })
}

fn refused(outcome: &str) -> bool {
    outcome.contains("Err(")
        && outcome.contains("refused the stop")
        && outcome.contains("Reentrant")
}

/// The stop's `In`-phase emit (`EM(idle)` of an empty capture, under D): an
/// inline listener that stops again is refused before T and D, so it neither
/// deadlocks on D nor changes anything.
#[test]
fn a_stop_nested_in_an_in_phase_emit_is_refused_before_any_lock() {
    fn plain(w: &Arc<World>) {
        recording(w);
        w.knobs().stop_result = Ok(0);
        stopped(w, "stop", None);
    }
    fn nested(w: &Arc<World>) {
        recording(w);
        w.knobs().stop_result = Ok(0);
        // The first status emit is `processing` (D released); the second is the
        // `In` idle emit, with D held.
        arm_on_status_emit(w, 2, nested_stop);
        stopped(w, "stop", None);
    }
    let (baseline, _) = run_nested("baseline", Setup::idle, plain);
    let (lines, nested_results) = run_nested("a stop nested in an In emit", Setup::idle, nested);
    assert_eq!(nested_results.len(), 1, "the listener ran once");
    assert!(refused(&nested_results[0]), "{}", nested_results[0]);
    assert_eq!(lines, baseline, "the refused entry changed nothing");
}

/// The second D section after the pipeline (204b): its idle emit runs under D; an
/// inline listener that stops again, or enters the lifecycle bridge, is refused.
#[test]
fn a_stop_or_a_bridge_entry_nested_in_the_204b_idle_emit_is_refused_before_d() {
    fn plain(w: &Arc<World>) {
        recording(w);
        w.knobs().stop_result = Ok(16_000);
        stopped(w, "stop", None);
    }
    fn nested_stop_body(w: &Arc<World>) {
        recording(w);
        w.knobs().stop_result = Ok(16_000);
        // `processing`, then (the fake pipeline emits nothing) 204b's `idle`.
        arm_on_status_emit(w, 2, nested_stop);
        stopped(w, "stop", None);
    }
    fn nested_bridge_body(w: &Arc<World>) {
        recording(w);
        w.knobs().stop_result = Ok(16_000);
        arm_on_status_emit(w, 2, nested_bridge);
        stopped(w, "stop", None);
    }
    let (baseline, _) = run_nested("baseline", Setup::idle, plain);
    let (lines, results) = run_nested(
        "a stop nested in the 204b emit",
        Setup::idle,
        nested_stop_body,
    );
    assert_eq!(results.len(), 1);
    assert!(refused(&results[0]), "{}", results[0]);
    assert_eq!(lines, baseline, "the refused stop changed nothing");
    let (lines, results) = run_nested(
        "a bridge entry nested in the 204b emit",
        Setup::idle,
        nested_bridge_body,
    );
    assert_eq!(
        results,
        ["bridge returned"],
        "the bridge refused and returned"
    );
    assert_eq!(lines, baseline, "the refused bridge entry changed nothing");
}

/// The reverse: a stop polled inline during a lifecycle `In` emit
/// (`CleanupFinished` emits under D) would take the free T and then block on D.
#[test]
fn a_stop_nested_in_a_lifecycle_in_emit_is_refused_before_t_and_d() {
    fn recovering() -> Setup {
        Setup {
            status: DictationStatus::Recovering,
            recording_id: 1,
        }
    }
    fn plain(w: &Arc<World>) {
        w.accept_silently(1);
        w.notify(1, AudioLifecycleEvent::CleanupFinished);
    }
    fn nested(w: &Arc<World>) {
        w.accept_silently(1);
        w.on_inline("emit:recording-cleanup-finished", {
            let world = Arc::clone(w);
            move || nested_stop(&world)
        });
        w.notify(1, AudioLifecycleEvent::CleanupFinished);
    }
    let (baseline, _) = run_nested("baseline", recovering, plain);
    let (lines, results) = run_nested("a stop nested in a lifecycle In emit", recovering, nested);
    assert_eq!(results.len(), 1, "the listener ran once");
    assert!(refused(&results[0]), "{}", results[0]);
    assert_eq!(lines, baseline, "the refused stop changed nothing");
}

/// C3.5: a newer start lands between the pipeline guard's finish (204a) and the
/// stop's own idle section (204b). 204b compares the id only: it emits no `idle`
/// and clears no keyboard gate for the old recording, while the guard (204a)
/// already wrote `Idle` and cleared the old context.
#[test]
fn a_newer_start_between_the_guard_finish_and_the_idle_emit_silences_204b() {
    let lines = under_watchdog("barrier between 204a and 204b", || {
        from_stop(trace(Setup::idle(), |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            let starter = Arc::clone(w);
            let _barrier = crate::barrier_hook::install(move |point| {
                if point == "pipeline_idle_emit" {
                    starter.note("a newer start lands between 204a and 204b");
                    // The supervisor finishes recording 1's sealed stop, then a
                    // new recording starts.
                    starter.finish_sealed_stop(1);
                    recording(&starter);
                }
            });
            stopped(w, "stop", None);
        }))
    });
    let guard_at = lines
        .iter()
        .position(|line| line == "pipeline 1 idle_guard wrote_idle=true")
        .expect("204a wrote idle for recording 1");
    let note_at = lines
        .iter()
        .position(|line| line.contains("a newer start lands between 204a and 204b"))
        .expect("the barrier fired");
    let start_at = lines
        .iter()
        .position(|line| line.starts_with("reply start"))
        .expect("the newer start replied");
    assert!(
        guard_at < note_at && note_at < start_at,
        "{}",
        lines.join("\n")
    );
    let after_start = &lines[start_at..];
    assert!(
        !after_start
            .iter()
            .any(|line| line == "emit recording-status-changed \"idle\""
                || line == "keyboard.set_processing false"),
        "204b stays silent for recording 1: {}",
        after_start.join("\n")
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("= final status=Recording processing=false context=2 cancelled_through=0"),
        "{}",
        lines.join("\n")
    );
}

// ------------------------------------------------ the pipeline start and its history

fn history_counts(world: &Arc<World>) -> (usize, usize) {
    let history = &world.state().app_state.core_history;
    (history.pipeline_count(), history.outstanding_ops())
}

fn assert_history_empty(world: &Arc<World>, when: &str) {
    assert_eq!(
        history_counts(world),
        (0, 0),
        "{when}: the pipeline's H2 entry and `K:Pipeline` op are both gone"
    );
}

fn resumed() -> Vec<crate::dictation_executor::executor::Witness> {
    stop_ports::RESUMED.with(|slot| slot.borrow().clone())
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

/// The stop drives `PS` through the executor and resumes it with the pipeline's
/// real result: `Completed` for a pipeline that returned `Ok`.
#[test]
fn the_awaited_pipeline_start_resumes_completed_when_the_pipeline_succeeds() {
    run_sub_run(&SubRun {
        title: "resume success",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(10_000);
            stop_ports::RESUMED.with(|slot| slot.borrow_mut().clear());
            stopped(w, "stop", None);
            assert_eq!(
                last_two(&resumed()),
                [
                    ("PS", "Awaiting".to_string()),
                    ("PS", "Resumed(Completed)".to_string())
                ]
            );
            assert_history_empty(w, "after a successful pipeline");
        },
    });
}

/// A pipeline error reaches `resume` as `Failed`, and the stop still returns the
/// error through R204b.5.
#[test]
fn the_awaited_pipeline_start_resumes_failed_when_the_pipeline_errors() {
    run_sub_run(&SubRun {
        title: "resume failure",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            {
                let mut knobs = w.knobs();
                knobs.stop_result = Ok(10_000);
                knobs.pipeline_error = Some("scripted pipeline failure");
            }
            stop_ports::RESUMED.with(|slot| slot.borrow_mut().clear());
            let mut stop = w.stop(None);
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            assert_eq!(reply, Err("scripted pipeline failure".to_string()));
            assert_eq!(
                last_two(&resumed()),
                [
                    ("PS", "Awaiting".to_string()),
                    ("PS", "Resumed(Failed)".to_string())
                ]
            );
            assert_history_empty(w, "after a pipeline that returned early");
        },
    });
}

/// While the pipeline runs, its operation and registration are outstanding; its
/// own guard owns them and retires both when a cancelled pipeline exits.
#[test]
fn the_pipeline_guard_retires_both_history_entries_on_a_cancelled_exit() {
    run_sub_run(&SubRun {
        title: "cancelled pipeline",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            assert_eq!(history_counts(w), (1, 1), "in flight: H2 and `K:Pipeline`");
            cancelled(w, "cancel");
            w.open_gate("pipeline");
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            assert_history_empty(w, "after a cancelled pipeline");
        },
    });
}

/// A panic in the guard's `In` effects (the idle closure runs under D) cannot
/// leave either entry behind: both were retired after the decision, before the
/// effects ran.
#[test]
fn a_panic_in_the_guards_in_effects_leaves_no_history_entry() {
    run_sub_run(&SubRun {
        title: "panic in the guard",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            {
                let mut knobs = w.knobs();
                knobs.stop_result = Ok(10_000);
                knobs.pipeline_finish_panics = true;
            }
            let mut stop = w.stop(None);
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| complete(&mut stop)));
            assert!(outcome.is_err(), "the closure's panic propagated");
            assert!(
                w.trace()
                    .contains(&"pipeline 1 history before In: h2=0 h1=0".to_string()),
                "both entries were retired before the `In` effects ran:\n{}",
                w.trace().join("\n")
            );
            drop(stop);
            assert!(!crate::dictation_executor::executor::in_core());
            assert_history_empty(w, "after a panic in the guard's In effects");
            // The panicking recording left Processing behind (nothing wrote Idle):
            // put the scenario's end state in order for the golden runner.
            w.state().app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
        },
    });
}

/// A stale pipeline exit (a newer recording started before the guard finished)
/// also retires both entries.
#[test]
fn a_stale_pipeline_exit_retires_both_history_entries() {
    run_sub_run(&SubRun {
        title: "stale pipeline exit",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            cancelled(w, "cancel");
            w.finish_sealed_stop(1);
            recording(w);
            w.open_gate("pipeline");
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            let (pipelines, _) = history_counts(w);
            assert_eq!(pipelines, 0, "recording 1's H2 entry is gone");
            // Recording 2 is live and owns no pipeline op: the stop's was 1's.
            assert_eq!(
                w.state().app_state.core_history.outstanding_ops(),
                0,
                "recording 1's `K:Pipeline` is gone"
            );
        },
    });
}

// ----------------------------------- D-S2-3 / D-S2-4 and the disarm order (re-check)

/// A stop parked at the pipeline gate is dropped by an inline listener of a
/// lifecycle `In` emit, while the bridge holds D. Its pipeline guard (a
/// `PipelineExit` drop) reaches `IdleGuard::run_finish` inside the core span: it
/// does not ask for D (it would deadlock on the non-reentrant D); the finish is
/// queued and the bridge runs it after it releases D. Both history entries are
/// retired exactly once, by that deferred finish.
#[test]
fn dropping_a_parked_stop_from_an_inline_listener_defers_the_finish_until_d_is_released() {
    let lines = under_watchdog("a stop dropped from a listener under D", || {
        from_stop(trace(Setup::idle(), |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            assert_eq!(
                history_counts(w),
                (1, 1),
                "the parked stop's pipeline is registered"
            );
            let parked = Arc::new(StdMutex::new(Some(stop)));
            let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
            w.on_inline("emit:recording-cleanup-finished", move || {
                observer.note("listener: dropping the parked stop while D is held");
                drop(slot.lock().expect("slot").take());
                // The finish is queued, not run: D is held and not reentrant.
                assert_eq!(
                    history_counts(&observer),
                    (1, 1),
                    "nothing is retired while the bridge still holds D"
                );
                observer.note("listener: dropped");
            });
            w.finish_sealed_stop(1);
            assert!(parked.lock().expect("slot").is_none(), "the listener ran");
            assert_history_empty(w, "after the deferred finish");
            assert!(
                super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                "the queue was drained"
            );
            w.open_gate("pipeline");
        }))
    });
    let dropped_at = lines
        .iter()
        .position(|line| line.contains("listener: dropped"))
        .expect("the listener ran");
    assert!(
        lines[dropped_at..]
            .iter()
            .any(|line| line.contains("context 1 cleared")),
        "the deferred finish ran after the listener returned:\n{}",
        lines.join("\n")
    );
}

/// A stop parked at the pipeline gate and resumed (polled) inline by a listener
/// of a lifecycle `In` emit reaches its 204b section inside the core span: it
/// parks there instead of asking for D, and finishes when it is next polled
/// outside the span.
#[test]
fn a_stop_resumed_inline_under_d_waits_out_the_core_span_before_its_204b_section() {
    under_watchdog("a stop resumed from a listener under D", || {
        trace(Setup::idle(), |w| {
            recording(w);
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            let parked = Arc::new(StdMutex::new(Some(stop)));
            let (observer, slot) = (Arc::clone(w), Arc::clone(&parked));
            w.on_inline("emit:recording-cleanup-finished", move || {
                observer.open_gate("pipeline");
                let mut stop = slot.lock().expect("slot").take().expect("parked");
                // D is held by the bridge: the stop runs on to its 204b section
                // and must park there rather than take D.
                for _ in 0..5 {
                    assert!(poll_once(&mut stop).is_pending(), "the stop parked");
                }
                assert!(
                    observer.trace().contains(&"pipeline 1 returns".to_string()),
                    "the stop got past the pipeline"
                );
                *slot.lock().expect("slot") = Some(stop);
            });
            w.finish_sealed_stop(1);
            let mut stop = parked.lock().expect("slot").take().expect("parked");
            stop_ports::RESUMED.with(|slot| slot.borrow_mut().clear());
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            assert!(reply.is_ok());
            // The PS continuation was not lost: the stop resumed it with the
            // pipeline's result (a resume inside the span would be `Invalid`).
            assert_eq!(
                last_two(&resumed()),
                [
                    ("PS", "Awaiting".to_string()),
                    ("PS", "Resumed(Completed)".to_string())
                ]
            );
            assert_history_empty(w, "after the resumed stop");
        })
    });
}

/// C3.5: the capture guard hands over to the pipeline's own guard **before**
/// `PE(Vad)`. A panic in that effect therefore leaves the status `Processing`, as
/// the original shell did, and the stop guard does not run 204a.
#[test]
fn a_panic_in_pe_vad_leaves_processing_because_the_guard_was_disarmed_first() {
    run_sub_run(&SubRun {
        title: "panic in PE(Vad)",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(10_000);
            let _barrier = crate::barrier_hook::install(|point| {
                assert_ne!(point, "stop.perf_enter_vad", "PE(Vad) panics");
            });
            let mut stop = w.stop(None);
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| complete(&mut stop)));
            assert!(outcome.is_err(), "the effect's panic propagated");
            drop(stop);
            assert!(!crate::dictation_executor::executor::in_core());
            let status = w.state().app_state.dictation.lock_or_recover().status;
            assert_eq!(
                status,
                DictationStatus::Processing,
                "the guard did not run 204a"
            );
            assert!(
                w.state().app_state.active_context(1).is_some(),
                "the context was not cleared"
            );
            assert_history_empty(w, "after a panic in PE(Vad)");
            // Put the scenario's end state in order for the golden runner.
            w.state().app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
            crate::dictation_executor::FieldAdapter::clear_context(&w.state().app_state, 1);
        },
    });
}

// --------------------------- the deferred finish ends on the thread that queued it

/// Recording 1's stop is parked at the pipeline gate; recording 2 is live. The
/// listener of recording 2's own `processing` emit drops recording 1's parked
/// pipeline: the drop queues a finish on this thread (the span is set). Recording 2
/// then suspends at its pipeline await, and may finish on another worker, whose
/// drains cannot see this thread's queue. The finish must therefore be applied
/// before recording 2's stop awaits, on this thread: 204a for recording 1 (stale)
/// runs and its H1 and H2 are retired.
#[test]
fn a_finish_queued_in_one_stops_emit_is_applied_before_that_stop_awaits() {
    under_watchdog("a finish queued in a stop's emit", || {
        run_sub_run(&SubRun {
            title: "cross-thread drain",
            setup: Setup::idle(),
            body: |w| {
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                w.close_gate("pipeline");
                let mut first = w.stop(None);
                assert!(poll_once(&mut first).is_pending());
                cancelled(w, "cancel");
                w.finish_sealed_stop(1);
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                let parked = Arc::new(StdMutex::new(Some(first)));
                let slot = Arc::clone(&parked);
                w.on_inline("emit:recording-status-changed", move || {
                    drop(slot.lock().expect("slot").take());
                });
                let mut second = w.stop(None);
                assert!(
                    poll_once(&mut second).is_pending(),
                    "recording 2 awaits its pipeline"
                );
                assert!(
                    parked.lock().expect("slot").is_none(),
                    "the listener dropped recording 1"
                );
                assert!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the queued finish ran before the stop awaited"
                );
                assert_eq!(
                    history_counts(w),
                    (1, 1),
                    "recording 1's H1 and H2 are retired; only recording 2's pipeline is left"
                );
                // Recording 2 goes on elsewhere.
                w.open_gate("pipeline");
                let reply = std::thread::scope(|scope| {
                    scope
                        .spawn(|| complete(&mut second))
                        .join()
                        .expect("the second worker")
                });
                w.reply("stop", &reply);
                assert_history_empty(w, "after recording 2's stop");
            },
        })
    });
}

/// A listener that queues a finish inside the 204b emit and then panics: the
/// drain still runs, during the unwind, after D is released.
#[test]
fn a_panic_after_a_queued_finish_still_drains_it_during_the_unwind() {
    under_watchdog("a panic after a queued finish", || {
        run_sub_run(&SubRun {
            title: "panic after queue",
            setup: Setup::idle(),
            body: |w| {
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                w.close_gate("pipeline");
                let mut first = w.stop(None);
                assert!(poll_once(&mut first).is_pending());
                cancelled(w, "cancel");
                w.finish_sealed_stop(1);
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                let parked = Arc::new(StdMutex::new(Some(first)));
                // The first status emit is recording 2's `processing`; the second
                // is its 204b `idle`, under D.
                w.on_inline("emit:recording-status-changed", || {});
                let slot = Arc::clone(&parked);
                w.on_inline("emit:recording-status-changed", move || {
                    drop(slot.lock().expect("slot").take());
                    panic!("the listener panics after queueing a finish");
                });
                let mut second = w.stop(None);
                assert!(poll_once(&mut second).is_pending());
                w.open_gate("pipeline");
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    complete(&mut second)
                }));
                assert!(outcome.is_err(), "the listener's panic propagated");
                drop(second);
                assert!(parked.lock().expect("slot").is_none());
                assert!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the unwind drained the queue"
                );
                assert_history_empty(w, "after the unwind");
                assert!(!crate::dictation_executor::executor::in_core());
            },
        })
    });
}

/// The 204b section's own drain runs on an unwind too, with no enclosing stop
/// entry to do it: a listener of its `idle` emit drops a parked pipeline (queueing
/// a finish) and panics.
#[test]
fn the_204b_section_drains_a_queued_finish_when_its_listener_panics() {
    under_watchdog("204b listener panics after queueing", || {
        run_sub_run(&SubRun {
            title: "204b unwind",
            setup: Setup::idle(),
            body: |w| {
                recording(w);
                w.knobs().stop_result = Ok(16_000);
                w.close_gate("pipeline");
                let mut parked = w.stop(None);
                assert!(poll_once(&mut parked).is_pending());
                assert_eq!(history_counts(w), (1, 1));
                let slot = Arc::new(StdMutex::new(Some(parked)));
                let state = w.state();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::run_pipeline_idle_emit(
                        &state.app_state,
                        1,
                        murmur_dictation_core::inputs::PostKind::StopPost,
                        &|_| {},
                        &|| {
                            drop(slot.lock().expect("slot").take());
                            panic!("the listener panics after queueing a finish");
                        },
                    );
                }));
                assert!(outcome.is_err());
                assert!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the unwind drained the queue"
                );
                assert_history_empty(w, "after the unwind");
                w.open_gate("pipeline");
            },
        })
    });
}

// ------------------------------------------- the drain's own failure and lock scope

/// Recording 1's stop is parked at the pipeline gate and recording 2 is live
/// (`Recording`, id 2, its context installed).
fn park_stop_of_recording_1_then_start_recording_2(
    w: &Arc<World>,
) -> Fut<Result<serde_json::Value, String>> {
    recording(w);
    w.knobs().stop_result = Ok(16_000);
    w.close_gate("pipeline");
    let mut first = w.stop(None);
    assert!(poll_once(&mut first).is_pending());
    cancelled(w, "cancel");
    w.finish_sealed_stop(1);
    recording(w);
    w.knobs().stop_result = Ok(16_000);
    first
}

/// Two finishes are queued and the first panics while it runs during the unwind
/// of a 204b listener's panic. The first panic still propagates, and the second
/// finish is still applied: its status written and its H1 and H2 retired.
#[test]
fn a_panic_in_one_deferred_finish_does_not_strand_the_next() {
    under_watchdog("a second panic during the drain", || {
        run_sub_run(&SubRun {
            title: "second panic",
            setup: Setup::idle(),
            body: |w| {
                let first = park_stop_of_recording_1_then_start_recording_2(w);
                let mut second = w.stop(None);
                assert!(poll_once(&mut second).is_pending());
                assert_eq!(history_counts(w), (2, 2), "both pipelines are registered");
                // The first deferred finish panics (a failing status-transition
                // trace); the barrier hook is taken out while it runs, so only
                // the first finish panics.
                let _barrier = crate::barrier_hook::install(|point| {
                    assert_ne!(point, "guard_finish", "the first deferred finish panics");
                });
                let parked = Arc::new(StdMutex::new(Some((first, second))));
                let state = w.state();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::run_pipeline_idle_emit(
                        &state.app_state,
                        2,
                        murmur_dictation_core::inputs::PostKind::StopPost,
                        &|_| {},
                        &|| {
                            // Queues two finishes (recording 1's, then 2's), then
                            // the listener panics.
                            let (first, second) = parked.lock().expect("slot").take().expect("two");
                            drop(first);
                            drop(second);
                            panic!("the listener's own panic");
                        },
                    );
                }));
                let panic = outcome.expect_err("the listener's panic propagated");
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                assert!(
                    message.contains("the listener's own panic"),
                    "the original panic wins: {message}"
                );
                assert!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the later finish was not abandoned"
                );
                assert_history_empty(w, "both finishes' H1 and H2 are retired");
                assert_eq!(
                    w.state().app_state.dictation.lock_or_recover().status,
                    DictationStatus::Idle,
                    "recording 2's deferred 204a wrote Idle"
                );
                w.open_gate("pipeline");
            },
        })
    });
}

/// Recording 2's `processing` listener drops recording 1's parked pipeline (a
/// queued finish) and panics while T is held. The deferred 204a must run after T is
/// released, as it does on the success path.
#[test]
fn an_early_stop_unwind_drains_after_it_releases_t() {
    under_watchdog("an early unwind drains with T held", || {
        run_sub_run(&SubRun {
            title: "unwind drains after T",
            setup: Setup::idle(),
            body: |w| {
                let first = park_stop_of_recording_1_then_start_recording_2(w);
                let parked = Arc::new(StdMutex::new(Some(first)));
                let slot = Arc::clone(&parked);
                w.on_inline("emit:recording-status-changed", move || {
                    drop(slot.lock().expect("slot").take());
                    panic!("the listener panics while the stop holds T");
                });
                let t_free_during_finish = Arc::new(StdMutex::new(Vec::new()));
                let (observer, seen) = (Arc::clone(w), Arc::clone(&t_free_during_finish));
                let _barrier = crate::barrier_hook::install(move |point| {
                    if point == "guard_finish" {
                        let state = observer.state();
                        let free = state.app_state.recording_transition.try_lock().is_ok();
                        seen.lock().expect("seen").push(free);
                    }
                });
                let mut second = w.stop(None);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    poll_once(&mut second)
                }));
                assert!(outcome.is_err(), "the listener's panic propagated");
                drop(second);
                assert_eq!(
                    *t_free_during_finish.lock().expect("seen"),
                    [true],
                    "the deferred 204a ran once, with T released"
                );
                assert!(
                    super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
                    "the unwind drained the queue"
                );
                assert_history_empty(w, "after the unwind");
                w.open_gate("pipeline");
            },
        })
    });
}

// ------------------------------------------------ the success terminal's witness

fn term_witness() -> crate::dictation_executor::executor::Witness {
    stop_ports::TAIL.with(|slot| {
        slot.borrow()
            .iter()
            .copied()
            .find(|witness| witness.code == "TERM")
            .expect("the tail ran a TERM")
    })
}

/// The tail's `TERM` hook returns the ledger's answer, so the witness records a
/// claim another sink won (the supervisor's own terminal landed first) as a failed
/// effect. The command's reply is the same either way.
#[test]
fn the_success_terminal_witness_records_a_claim_the_supervisor_won() {
    use crate::dictation_executor::executor::WitnessOutcome;
    run_sub_run(&SubRun {
        title: "terminal claimed",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(10_000);
            stop_ports::TAIL.with(|slot| slot.borrow_mut().clear());
            stopped(w, "stop", None);
            assert_eq!(
                term_witness().outcome,
                WitnessOutcome::Ran,
                "the stop's claim won"
            );
        },
    });
    run_sub_run(&SubRun {
        title: "terminal lost",
        setup: Setup::idle(),
        body: |w| {
            recording(w);
            w.knobs().stop_result = Ok(10_000);
            w.close_gate("pipeline");
            let mut stop = w.stop(None);
            assert!(poll_once(&mut stop).is_pending());
            // The supervisor's terminal for recording 1 lands first.
            assert!(
                w.state()
                    .app_state
                    .dictation_telemetry
                    .emit_terminal_with_output(
                        1,
                        DictationTerminalOutcome::Superseded,
                        DictationErrorCode::StaleOwner,
                        0
                    ),
                "the supervisor claimed the terminal"
            );
            stop_ports::TAIL.with(|slot| slot.borrow_mut().clear());
            w.open_gate("pipeline");
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            assert_eq!(
                reply.expect("the reply is unchanged by a lost claim")["type"],
                "transcription"
            );
            assert_eq!(
                term_witness().outcome,
                WitnessOutcome::Failed,
                "the witness records the lost claim"
            );
        },
    });
}

// ------------------------------------- panic reporting never strands or aborts the drain

/// A subscriber that panics on the first `pipeline.dictation_state_changed`
/// event (the finish's status trace) and on every recovery event the drain
/// reports for a panicked finish.
#[derive(Clone, Default)]
struct PanickingSubscriber {
    status_panics_left: Arc<std::sync::atomic::AtomicUsize>,
    recovery_events: Arc<StdMutex<Vec<String>>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for PanickingSubscriber {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        use std::sync::atomic::Ordering;
        let text = format!("{event:?}");
        if text.contains("pipeline.dictation_deferred_finish_panicked") {
            self.recovery_events.lock().expect("events").push(text);
            panic!("the subscriber panics on the recovery event");
        }
        if text.contains("pipeline.dictation_state_changed")
            && self
                .status_panics_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
        {
            panic!("the subscriber panics on the status trace");
        }
    }
}

/// Queues two finishes (recording 2's, which writes the status and so traces it,
/// then recording 1's stale one) in a 204b listener, under a subscriber that
/// panics on the first status trace and on the drain's recovery report. The
/// listener then returns, or panics itself when `listener_panics`. Returns the
/// recovery events the subscriber saw and the panic message that escaped.
fn drain_two_finishes_under_a_panicking_subscriber(
    w: &Arc<World>,
    listener_panics: bool,
) -> (Vec<String>, String) {
    use tracing_subscriber::layer::SubscriberExt;
    let first = park_stop_of_recording_1_then_start_recording_2(w);
    let mut second = w.stop(None);
    assert!(poll_once(&mut second).is_pending());
    assert_eq!(history_counts(w), (2, 2), "both pipelines are registered");
    let parked = Arc::new(StdMutex::new(Some((first, second))));
    let state = w.state();
    let layer = PanickingSubscriber::default();
    layer
        .status_panics_left
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let subscriber = tracing_subscriber::Registry::default().with(layer.clone());
    let outcome = tracing::subscriber::with_default(subscriber, || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::run_pipeline_idle_emit(
                &state.app_state,
                2,
                murmur_dictation_core::inputs::PostKind::StopPost,
                &|_| {},
                &|| {
                    let (first, second) = parked.lock().expect("slot").take().expect("two");
                    drop(second);
                    drop(first);
                    if listener_panics {
                        panic!("the listener's own panic");
                    }
                    true
                },
            );
        }))
    });
    let panic = outcome.expect_err("a panic escaped");
    let message = panic
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    // The payload may be leaked or dropped; a panic must not come out of it.
    drop(panic);
    assert!(
        super::DEFERRED_FINISHES.with(|queue| queue.borrow().is_empty()),
        "the second finish was not stranded"
    );
    assert_history_empty(w, "both finishes' H1 and H2 are retired");
    assert!(!crate::dictation_executor::executor::in_core());
    w.open_gate("pipeline");
    let events = layer.recovery_events.lock().expect("events").clone();
    (events, message)
}

/// A normal drain: the first finish panics on its status trace and the subscriber
/// panics again on the recovery report. The second finish still runs, and the
/// first panic is re-raised once everything has run.
#[test]
fn a_panicking_subscriber_cannot_strand_the_second_finish_in_a_normal_drain() {
    under_watchdog("panicking subscriber, normal drain", || {
        run_sub_run(&SubRun {
            title: "panicking subscriber, normal drain",
            setup: Setup::idle(),
            body: |w| {
                let (events, message) = drain_two_finishes_under_a_panicking_subscriber(w, false);
                assert!(
                    message.contains("the subscriber panics on the status trace"),
                    "the first finish's panic is the one re-raised: {message}"
                );
                assert_eq!(events.len(), 1, "one panicked finish, reported once");
            },
        })
    });
}

/// An unwinding drain: the listener's own panic is unwinding when the drain runs,
/// the first finish panics, and the subscriber panics on the recovery report.
/// Nothing aborts, the second finish still runs, and the listener's panic wins.
#[test]
fn a_panicking_subscriber_cannot_strand_or_abort_an_unwinding_drain() {
    under_watchdog("panicking subscriber, unwinding drain", || {
        run_sub_run(&SubRun {
            title: "panicking subscriber, unwinding drain",
            setup: Setup::idle(),
            body: |w| {
                let (events, message) = drain_two_finishes_under_a_panicking_subscriber(w, true);
                assert!(
                    message.contains("the listener's own panic"),
                    "the original panic wins: {message}"
                );
                assert_eq!(events.len(), 1, "one panicked finish, reported once");
            },
        })
    });
}

/// The report carries the registered code, the recording id and the origin name,
/// and nothing from the panic payload.
#[test]
fn the_panicked_finish_report_is_registered_and_content_free() {
    under_watchdog("panic report content", || {
        run_sub_run(&SubRun {
            title: "panic report content",
            setup: Setup::idle(),
            body: |w| {
                let (events, _message) = drain_two_finishes_under_a_panicking_subscriber(w, false);
                let text = events.first().expect("a report");
                assert!(
                    text.contains("pipeline.dictation_deferred_finish_panicked"),
                    "{text}"
                );
                assert!(text.contains("recording_id"), "{text}");
                assert!(text.contains("origin"), "{text}");
                assert!(!text.contains("the subscriber panics"), "{text}");
                assert_eq!(
                    crate::telemetry::canonical_event_code(
                        "pipeline.dictation_deferred_finish_panicked"
                    ),
                    Some("pipeline.dictation_deferred_finish_panicked")
                );
            },
        })
    });
}
