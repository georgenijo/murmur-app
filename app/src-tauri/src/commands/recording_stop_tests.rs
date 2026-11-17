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
