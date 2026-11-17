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
