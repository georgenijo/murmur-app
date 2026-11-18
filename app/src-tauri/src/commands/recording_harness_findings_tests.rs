//! Shell reproductions of the behaviours the Stage 2b-1 property harness found
//! outside the C7.1 allowlist (decision D-S2b-5). Each test drives today's
//! commands through the Stage 0a `DictationHost` seam and the Stage 0b harness
//! with the schedule of the harness seed that found the behaviour, and asserts
//! what the code does today. A test that reproduces makes the behaviour a
//! preserved defect (a PD row in C7.1); a fix deletes the row and its test in the
//! same PR (C7.0). Ordering is by causal barriers only: no clock, no sleep.

use super::golden_harness::*;
use super::golden_tests::{cancelled, recording, sub};
use super::pd_witness_tests::{position, repeat};
use super::*;

fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
}

fn stop_reply(world: &Arc<World>, expected: Option<u64>) -> Result<serde_json::Value, String> {
    let mut stop = world.stop(expected);
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
    reply
}

fn start_reply(world: &Arc<World>) -> Result<serde_json::Value, String> {
    let mut start = world.start();
    let reply = complete(&mut start);
    world.reply("start", &reply);
    reply
}

fn audio_recovering() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "type": "audio_recovering", "state": "recovering" }))
}

// ------------------------------------------------------------------ F-4
// Harness seed 633. An `Interrupted` end of 8,000 samples or more, after
// `Recovering{RuntimeFailure}` and `InitializationFailed(recovery_reason =
// RuntimeFailure)` (which clears the context). The supervisor stores the
// preserved capture; a user stop lands before the `Interrupted` notification, takes
// the capture (R201.7) and, with no context, writes Idle (R202.3, PD-3) while the
// owner is still `Recovering`; a start then reads cleanup pending and takes R101.12
// (`Idle -> Recovering`, old id). The `Interrupted` arm schedules the auto-stop
// (R215.2); it finds `Recovering` and no capture (R201.6) and restores nothing.

fn f4_user_stop_then_start_before_interrupted(world: &Arc<World>) {
    recording(world);
    world.runtime_failure(1);
    world.notify(
        1,
        AudioLifecycleEvent::InitializationFailed {
            error: "scripted initialization failure".to_string(),
            kind: audio::AudioFailureKind::BackendError,
            recovery_reason: Some(AudioCancelReason::RuntimeFailure),
        },
    );
    audio_lifecycle::insert_interrupted_dictation_for_test(
        1,
        audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 9_000],
            reason: "stream_invalidated".to_string(),
            duration_ms: 9_000 / 16,
        },
    );
    world.note("the capture is stored; Interrupted has not been notified yet");
    // The user stop takes the capture, finds no context and writes Idle.
    let reply = stop_reply(world, None);
    assert!(
        matches!(&reply, Err(error) if error.contains("Missing dictation context")),
        "{reply:?}"
    );
    assert_eq!(status(world), DictationStatus::Idle);
    // A start takes D while the owner is still `Recovering`.
    assert_eq!(start_reply(world), audio_recovering());
    assert_eq!(status(world), DictationStatus::Recovering);
    // The Interrupted arm: 9,000 samples >= 8,000, so it schedules the auto-stop.
    world.notify(
        1,
        AudioLifecycleEvent::Interrupted {
            reason: AudioCancelReason::RuntimeFailure,
            delivered_samples: 9_000,
            duration_ms: 9_000 / 16,
        },
    );
    world.hold_owner_clear(1);
    let requests = world.auto_stop_requests();
    assert_eq!(requests, vec![1], "the arm scheduled the auto-stop");
    assert_eq!(
        stop_reply(world, Some(1)),
        audio_recovering(),
        "the auto-stop is refused: nothing to take"
    );
    world.release_owner_clear();
    nothing_restores_idle(world);
}

/// After the owner is cleared nothing can restore Idle: every command is refused.
fn nothing_restores_idle(world: &Arc<World>) {
    assert_eq!(status(world), DictationStatus::Recovering);
    assert_eq!(
        world.state().app_state.recording_id.load(Ordering::SeqCst),
        1
    );
    assert_eq!(stop_reply(world, None), audio_recovering());
    cancelled(world, "cancel");
    assert_eq!(start_reply(world), audio_recovering());
    assert_eq!(status(world), DictationStatus::Recovering);
    let trace = world.trace();
    let cleared = position(&trace, "supervisor.clear_owner 1");
    assert!(
        !trace[cleared..]
            .iter()
            .any(|line| line.starts_with("supervisor.notify 1 ")),
        "no notification follows the owner clear: {trace:#?}"
    );
}

#[test]
fn f4_interrupted_over_8000_with_a_refused_auto_stop_leaves_recovering() {
    repeat(&sub(
        "F-4 / PD-19: user stop, then start, before Interrupted >= 8,000",
        f4_user_stop_then_start_before_interrupted,
    ));
}

// ------------------------------------------------------------ stop on a thread

type StopReply = Result<serde_json::Value, String>;
type ReplySlot = Arc<std::sync::Mutex<Option<StopReply>>>;

/// A stop running on its own thread, parked at the supervisor's capture stop
/// with T released (`PT:CP`, R202.1): the interleaving of a user stop and another
/// entry that the single-threaded harness cannot poll. The park is a channel the
/// scenario opens; no clock.
struct ParkedStop {
    thread: std::thread::JoinHandle<StopReply>,
    release: std::sync::mpsc::Sender<()>,
    reply: ReplySlot,
}

impl ParkedStop {
    fn start(world: &Arc<World>) -> Self {
        let (entered, entered_rx) = std::sync::mpsc::channel::<()>();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        world.on("supervisor.stop", move || {
            let _ = entered.send(());
            let _ = release_rx.recv();
        });
        let thread_world = Arc::clone(world);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a stop thread runtime");
            let _enter = runtime.enter();
            let mut stop = thread_world.stop(None);
            complete_joining(&mut stop)
        });
        entered_rx
            .recv()
            .expect("the stop reached the supervisor's capture stop");
        world.note("the stop is in the capture stop, T released");
        Self {
            thread,
            release,
            reply: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Let the capture stop return and wait for the stop to finish: what the
    /// stop does after the supervisor's reply runs to its end before this returns.
    fn finish(self) -> StopReply {
        let _ = self.release.send(());
        self.thread.join().expect("the stop thread panicked")
    }

    /// A hook body that finishes the stop and files its reply in `reply`.
    fn finish_into(self) -> (impl FnOnce() + Send + 'static, ReplySlot) {
        let slot = Arc::clone(&self.reply);
        let filed = Arc::clone(&slot);
        (
            move || {
                let reply = self.finish();
                *filed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reply);
            },
            slot,
        )
    }
}

fn terminal(world: &Arc<World>, id: u64) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
    world
        .state()
        .app_state
        .dictation_telemetry
        .terminal_for_test(id)
}

fn perf_claim(id: u64) -> Option<RunOutcomeV1> {
    dictation_performance_begins().claimed_outcome(id)
}

// ------------------------------------------------------------------ F-1
// Harness seed 470. A stop is in its capture stop with T released (status
// Processing). A cancel takes T and runs R205.5: its `PostDT` `TERM` wins the
// terminal as `UserCancelledProcessing`. Inside that tail, before the cancel's
// `PT:PC`, the stop's capture returns fewer than 4,800 samples and its R203
// short-audio rows finish performance as `NoSpeech`. The cancel's `PT:PC(Cancelled)`
// then loses the performance claim.

fn f1_stop_short_path_between_cancel_tail_and_perf_complete(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(1_000);
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    let (finish, reply) = stop.finish_into();
    world.on("keyboard.set_processing:false", finish);
    cancelled(world, "cancel");
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the hook finished the stop");
    world.reply("stop", &reply);
    assert_eq!(status(world), DictationStatus::Idle);
    // The terminal says the user cancelled; the performance record says no speech.
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::UserCancelledProcessing,
            DictationErrorCode::CancelledProcessing
        ))
    );
    assert_eq!(perf_claim(1), Some(RunOutcomeV1::NoSpeech));
}

#[test]
fn f1_cancel_terminal_and_a_short_stop_performance_outcome_disagree() {
    repeat(&sub(
        "F-1 / PD-20: a short stop completes performance inside a cancel's tail",
        f1_stop_short_path_between_cancel_tail_and_perf_complete,
    ));
}

// ------------------------------------------------------------------ F-2
// Harness seed 42. A cancel of a `Recording` dictation (R205.4) is in its capture
// cancel (`PT:CC`, T released). A stop lands (R201.8) and parks in its own capture
// stop (T released). The capture cancel fails, and site 206 (R206.2) claims the
// terminal as `PipelineFailure` for a recording that is in fact being stopped.
// The stop's capture then returns fewer than 4,800 samples and its short-audio
// rows complete performance as `NoSpeech`: the terminal and the performance
// outcome disagree.

fn f2_stop_lands_while_the_capture_cancel_is_in_flight(world: &Arc<World>) {
    recording(world);
    {
        let mut knobs = world.knobs();
        knobs.stop_result = Ok(1_000);
        knobs.cancel_result = Err("boom");
    }
    let landing = Arc::clone(world);
    let parked: Arc<std::sync::Mutex<Option<ParkedStop>>> = Arc::new(std::sync::Mutex::new(None));
    let filed = Arc::clone(&parked);
    world.on("supervisor.cancel", move || {
        let stop = ParkedStop::start(&landing);
        *filed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(stop);
    });
    let mut cancel = world.cancel(None);
    let cancel_reply = complete(&mut cancel);
    world.reply_unit("cancel", &cancel_reply);
    assert_eq!(cancel_reply, Err("boom".to_string()));
    assert_eq!(status(world), DictationStatus::Processing);
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    let stop = parked
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the hook parked the stop");
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::RuntimeFailure
        )),
        "the stop's terminal lost the claim"
    );
    assert_eq!(perf_claim(1), Some(RunOutcomeV1::NoSpeech));
}

#[test]
fn f2_failed_capture_cancel_terminal_and_a_short_stop_performance_outcome_disagree() {
    repeat(&sub(
        "F-2 / PD-21: a stop parks in its capture stop while a capture cancel fails",
        f2_stop_lands_while_the_capture_cancel_is_in_flight,
    ));
}

// ------------------------------------------------------------------ F-3
// Harness seed 331. A cancel at `Processing` runs R205.5 and, in its tail at `CX-`
// (D released, status already Idle, the id still current, the cancel's own `TERM`
// not yet run), the supervisor's `Idle` for that recording arrives: R211.2 claims
// the terminal as `Superseded` (PD-2). The cancel's `TERM` loses, and its
// `PT:PC(Cancelled)` completes performance: the terminal says superseded, the
// performance record says cancelled.

fn idle_at_cx_minus(world: &Arc<World>, then: impl FnOnce() + 'static) {
    let supervisor = Arc::clone(world);
    let mut landing = Some(then);
    let barrier = crate::barrier_hook::install(move |point| {
        if point != "adapter.clear_context" {
            return;
        }
        if let Some(then) = landing.take() {
            supervisor.notify(1, AudioLifecycleEvent::Idle);
            then();
        }
    });
    cancelled(world, "cancel");
    drop(barrier);
}

fn f3_supervisor_idle_before_the_cancel_terminal(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.close_gate("pipeline");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    world.note("the stop is in the pipeline; status is Processing");
    idle_at_cx_minus(world, || {});
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(
        terminal(world, 1).map(|(outcome, _)| outcome),
        Some(DictationTerminalOutcome::Superseded)
    );
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn f3_superseded_terminal_and_a_cancels_performance_outcome_disagree() {
    repeat(&sub(
        "F-3 / PD-22: a supervisor Idle before a cancel's terminal claim (R211.2, R205.5)",
        f3_supervisor_idle_before_the_cancel_terminal,
    ));
}

// The same race with a stop in its capture stop (T released) whose capture then
// fails: R203.2 completes performance as `Failed` before the cancel's `PT:PC`
// (first seed 4265, R211.2 / R203.2).

fn f3_supervisor_idle_with_a_failing_stop(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Err("capture failed");
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    let (finish, reply) = stop.finish_into();
    idle_at_cx_minus(world, finish);
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the barrier finished the stop");
    world.reply("stop", &reply);
    assert!(reply.is_err(), "{reply:?}");
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(
        terminal(world, 1).map(|(outcome, _)| outcome),
        Some(DictationTerminalOutcome::Superseded)
    );
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Failed { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn f3_superseded_terminal_and_a_failed_stop_performance_outcome_disagree() {
    repeat(&sub(
        "F-3 / PD-22: a supervisor Idle before a cancel's terminal claim, a failing stop (R211.2, R203.2)",
        f3_supervisor_idle_with_a_failing_stop,
    ));
}
