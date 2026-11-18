//! Shell reproductions of the behaviours the Stage 2b-1 property harness found
//! outside the C7.1 allowlist (decisions D-S2b-5, D-S2b-9). Each test drives today's
//! commands through the Stage 0a `DictationHost` seam and the Stage 0b harness
//! with the schedule of the harness seed that found the behaviour, and asserts
//! what the code does today. A test that reproduces makes the behaviour a
//! preserved defect (a PD row in C7.1); a fix deletes the row and its test in the
//! same PR (C7.0). Ordering is by causal barriers only: no clock, no sleep.
//!
//! D-S2b-9 (b): the fixtures drive the shell commands through the Stage 0b fake host.
//! The real supervisor cannot be reached from the shell without a production seam, so
//! every supervisor notification and port reply a fixture injects is one the real
//! supervisor produces in that state: its sender and state are cited at the injection
//! (`audio_lifecycle.rs` line numbers), the replies are the production strings, and
//! each test asserts the exact replies, statuses and ledger outcomes. The supervisor's
//! own side (which notifications can follow which) is modeled and checked against the
//! source by the property harness's mirror table.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{cancelled, recording, sub};
use super::pd_witness_tests::{position, repeat};
use super::*;

/// The reply of a stop whose 12 s receive expired (`STOP_RESPONSE_TIMEOUT`,
/// `audio_lifecycle.rs:3276-3282`): the receiver is dropped, so a later
/// `response.send` of the supervisor fails and `finish_attempt` notifies `Idle`
/// (`2358-2373`).
const STOP_RECEIVE_TIMEOUT: &str =
    "Microphone teardown is still blocked; the owned worker remains in recovery";

/// The reply of a cancel whose 2 s receive expired with the request still queued
/// (`audio_lifecycle.rs:3360-3363`).
const CANCEL_RECEIVE_TIMEOUT: &str = "Audio lifecycle supervisor did not acknowledge cancellation";

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

/// The terminal the ledger holds once the failure and the stop with no context have
/// run: the failure's `InitializationFailed` bridge arm claimed it first.
const TERMINAL_AFTER_MISSING_CONTEXT: Option<(DictationTerminalOutcome, DictationErrorCode)> =
    Some((
        DictationTerminalOutcome::RuntimeInterruption,
        DictationErrorCode::TerminationUnconfirmed,
    ));

/// The stop reply of an empty capture: an idle, empty transcription.
fn empty_transcription() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "type": "transcription", "state": "idle", "text": "" }))
}

// ----------------------------------------------------------------- PD-19
// Harness seed 11,125 (`FINDING_SEEDS`). The worker's exit is unconfirmed while a dictation records.
// The real supervisor handles `TerminationUnconfirmed` outside `Stopping` with
// `StartupDiagnostic(WorkerInvariantViolation)` (`audio_lifecycle.rs:2036-2046`, no
// standby deadline), then `begin_recovery(RuntimeFailure, Some(failure))`
// (`2056-2062`, `2630-2760`): `Recovering{RuntimeFailure}`, then
// `InitializationFailed{error, kind: TerminationUnconfirmed, recovery_reason:
// Some(RuntimeFailure)}` through `report_failure_once` (`2734-2758`), whose bridge
// arm clears the context. The helper then exits and `finish_attempt` retains the
// PCM: it stores the preserved capture and only then notifies `Interrupted`
// (`2405-2432`), so a user stop on another thread can land in between: it takes the
// capture (R201.5), and with no context writes Idle (R202.3, PD-3) while the owner
// is still `Recovering`; a start then reads cleanup pending and takes R101.12
// (`Idle -> Recovering`, old id). The `Interrupted` arm schedules the auto-stop
// (R215.2); it finds `Recovering` and no capture (R201.6) and restores nothing.
// Every later command is refused or answered as asserted below.

fn pd19_user_stop_then_start_before_interrupted(world: &Arc<World>) {
    recording(world);
    let failure = audio::AudioFailure::new(
        audio::AudioFailureKind::TerminationUnconfirmed,
        audio::AudioInitPhase::Runtime,
    );
    // `audio_lifecycle.rs:2038-2046`
    world.notify(
        1,
        AudioLifecycleEvent::StartupDiagnostic(
            audio::AudioStartupDiagnostic::WorkerInvariantViolation,
        ),
    );
    // `begin_recovery`: `Recovering{RuntimeFailure}` (`2630-2760`), then
    // `report_failure_once` (`2734-2758`).
    world.runtime_failure(1);
    world.notify(
        1,
        AudioLifecycleEvent::InitializationFailed {
            error: failure.to_string(),
            kind: failure.kind,
            recovery_reason: Some(AudioCancelReason::RuntimeFailure),
        },
    );
    // The helper has exited with 9,000 samples retained: `finish_attempt` stores
    // the capture (`2417-2432`) and has not notified `Interrupted` yet.
    audio_lifecycle::insert_interrupted_dictation_for_test(
        1,
        audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 9_000],
            reason: failure.kind.as_str().to_string(),
            duration_ms: 9_000 / 16,
        },
    );
    world.note("the capture is stored; Interrupted has not been notified yet");
    // The user stop takes the capture (R201.5), finds no context and writes Idle.
    assert_eq!(
        stop_reply(world, None),
        Err("Missing dictation context for recording 1".to_string())
    );
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(
        terminal(world, 1),
        TERMINAL_AFTER_MISSING_CONTEXT,
        "the terminal after the failure and the stop"
    );
    // A start takes D while the owner is still `Recovering`.
    assert_eq!(start_reply(world), audio_recovering());
    assert_eq!(status(world), DictationStatus::Recovering);
    // The Interrupted arm: 9,000 samples >= 8,000, so it schedules the auto-stop
    // (`2417-2432`).
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
    let mut cancel = world.cancel(None);
    let reply = complete(&mut cancel);
    world.reply_unit("cancel", &reply);
    assert_eq!(reply, Ok(()), "a cancel answers Ok and changes nothing");
    assert_eq!(status(world), DictationStatus::Recovering);
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
fn pd19_termination_unconfirmed_after_retained_pcm_leaves_recovering() {
    repeat(&sub(
        "PD-19: TerminationUnconfirmed after retained PCM; a user stop and a start land before Interrupted",
        pd19_user_stop_then_start_before_interrupted,
    ));
}

// PD-19, second entry (harness seed 23,985). The same defect with the context missing
// for another reason: the start future was dropped at its await after the supervisor's
// ack (PD-9, `smart_capture`), so no context was ever installed and the status stays
// `Starting`. The worker then fails at runtime with 8,000 samples retained
// (`Recovering{RuntimeFailure}`, `2630-2698`; the helper's exit stores the capture
// before `Interrupted`, `2405-2432`). The user's stop takes the capture (R201.5), finds
// no context and writes `Idle` while the owner is still `Recovering` (R202.3, PD-3);
// a start reads cleanup pending and takes R101.12; `Interrupted` schedules the auto-stop,
// which finds nothing to take. Nothing restores `Idle`.

fn pd19_a_dropped_start_then_a_runtime_failure_with_retained_pcm(world: &Arc<World>) {
    world.close_gate("smart_capture");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    drop(start);
    assert_eq!(status(world), DictationStatus::Starting);
    // `begin_recovery(RuntimeFailure)` (`2630-2698`).
    world.runtime_failure(1);
    assert_eq!(status(world), DictationStatus::Recovering);
    // The helper has exited with 8,000 samples retained: the capture is stored, and
    // `Interrupted` has not been notified yet (`2405-2432`).
    audio_lifecycle::insert_interrupted_dictation_for_test(
        1,
        audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 8_000],
            reason: "stream_invalidated".to_string(),
            duration_ms: 8_000 / 16,
        },
    );
    // The user stop takes the capture, finds no context and writes Idle.
    assert_eq!(
        stop_reply(world, None),
        Err("Missing dictation context for recording 1".to_string())
    );
    assert_eq!(status(world), DictationStatus::Idle);
    // A start takes D while the owner is still `Recovering`.
    assert_eq!(start_reply(world), audio_recovering());
    assert_eq!(status(world), DictationStatus::Recovering);
    world.notify(
        1,
        AudioLifecycleEvent::Interrupted {
            reason: AudioCancelReason::RuntimeFailure,
            delivered_samples: 8_000,
            duration_ms: 8_000 / 16,
        },
    );
    world.hold_owner_clear(1);
    assert_eq!(world.auto_stop_requests(), vec![1]);
    assert_eq!(
        stop_reply(world, Some(1)),
        audio_recovering(),
        "the auto-stop is refused: nothing to take"
    );
    world.release_owner_clear();
    nothing_restores_idle(world);
}

#[test]
fn pd19_a_dropped_start_and_a_retained_runtime_failure_leave_recovering() {
    repeat(&sub(
        "PD-19 (second entry): a dropped start, a runtime failure with retained PCM, a context-less user stop and a start",
        pd19_a_dropped_start_then_a_runtime_failure_with_retained_pcm,
    ));
}

// PD-19, third entry (harness seed 377,529): the refused auto-stop, with no ordinary
// control that takes the capture. A user stop moved the status to `Processing` and waits in the
// supervisor's capture stop (R201.8, R202.1). The worker fails at runtime with 8,000
// samples retained: `begin_recovery(RuntimeFailure)` publishes `Recovering` (`2630-2698`,
// the arm finds `Processing`, R209.2). The user cancels the processing recording (R205.5),
// which writes `Idle` while the owner is still `Recovering`. The parked capture stop
// returns the supervisor's recovery refusal (`audio_lifecycle.rs:1291-1295`, R203.2 at
// `Idle`). `finish_attempt` then stores the capture and notifies `Interrupted`
// (`2405-2432`): the arm schedules the auto-stop (R215.2), which runs at status `Idle` and
// is refused without taking anything (R201.3, `NotRecording`). A start reads cleanup
// pending and takes R101.12 (`Idle -> Recovering`, old id); the owner clears. The capture
// is still in the store, and the only stop that could take it is the user's stop in
// `Recovering`, which the frontend never sends (`useRecordingState.ts` `handleStop`: it
// presents "Microphone cleanup is still in progress"; `useRecordingState.test.tsx` "issues
// no stop or cancel IPC while the status is recovering"). Ordinary controls (start, cancel,
// the frontend's stop) leave the app stuck in `Recovering`. Only an unscoped
// `stop_native_recording` IPC, which no control the user has sends, takes the capture and
// restores `Idle` (asserted at the end; D-S2b-15 (d)). It stays a defect.

const REFUSED_STOP_IN_RECOVERY: &str = "Audio is still recovering from the previous attempt";

fn pd19_the_auto_stop_is_refused_at_idle_and_the_capture_stays(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Err(REFUSED_STOP_IN_RECOVERY);
    // The user's stop: Processing, parked in the capture stop with T released.
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    // The worker fails at runtime while the stop waits (`2630-2698`).
    world.runtime_failure(1);
    // The user's cancel of the processing recording writes `Idle` (R205.5).
    let mut cancel = world.cancel(None);
    let cancel_reply = complete(&mut cancel);
    world.reply_unit("cancel", &cancel_reply);
    assert_eq!(cancel_reply, Ok(()));
    assert_eq!(status(world), DictationStatus::Idle);
    // The capture stop returns the supervisor's refusal (`1291-1295`): R203.2.
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(reply, Err(REFUSED_STOP_IN_RECOVERY.to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
    // `finish_attempt`: the capture is stored, then `Interrupted` (`2417-2432`).
    audio_lifecycle::insert_interrupted_dictation_for_test(
        1,
        audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 8_000],
            reason: "stream_invalidated".to_string(),
            duration_ms: 8_000 / 16,
        },
    );
    world.notify(
        1,
        AudioLifecycleEvent::Interrupted {
            reason: AudioCancelReason::RuntimeFailure,
            delivered_samples: 8_000,
            duration_ms: 8_000 / 16,
        },
    );
    world.hold_owner_clear(1);
    assert_eq!(
        world.auto_stop_requests(),
        vec![1],
        "the arm scheduled the auto-stop"
    );
    // The auto-stop runs at `Idle`: refused, nothing taken.
    let refused = stop_reply(world, Some(1));
    assert_eq!(
        refused,
        Ok(serde_json::json!({ "type": "not_recording", "state": "idle" })),
        "the auto-stop is refused at Idle (R201.3)"
    );
    assert_eq!(status(world), DictationStatus::Idle);
    // A start takes D while the owner is still `Recovering` (R101.12).
    assert_eq!(start_reply(world), audio_recovering());
    assert_eq!(status(world), DictationStatus::Recovering);
    world.release_owner_clear();
    // Nothing restores `Idle`: a start and a cancel are refused or change nothing.
    assert_eq!(status(world), DictationStatus::Recovering);
    assert_eq!(start_reply(world), audio_recovering());
    let mut cancel = world.cancel(None);
    let reply = complete(&mut cancel);
    world.reply_unit("cancel", &reply);
    assert_eq!(reply, Ok(()), "a cancel answers Ok and changes nothing");
    assert_eq!(status(world), DictationStatus::Recovering);
    let trace = world.trace();
    let cleared = position(&trace, "supervisor.clear_owner 1");
    assert!(
        !trace[cleared..]
            .iter()
            .any(|line| line.starts_with("supervisor.notify 1 ")),
        "no notification follows the owner clear: {trace:#?}"
    );
    // The unscoped stop IPC is the one stop that recovers it (D-S2b-15 (c), (d)). The
    // registered `stop_native_recording` command (registered in `lib.rs`'s command list; `stop_native_recording` in `recording.rs`) has no
    // `Recovering` guard and no control the user has sends it in `Recovering`: it takes the
    // retained capture (R201.5), finds no context (R202.3) and writes `Idle`.
    assert_eq!(
        stop_reply(world, None),
        Err("Missing dictation context for recording 1".to_string()),
        "the unscoped stop takes the capture the auto-stop never took"
    );
    assert_eq!(status(world), DictationStatus::Idle);
    assert!(
        audio_lifecycle::take_interrupted_dictation(1).is_none(),
        "the unscoped stop took the capture"
    );
}

#[test]
fn pd19_a_refused_auto_stop_leaves_the_capture_and_recovering() {
    repeat(&sub(
        "PD-19 (third entry): the auto-stop runs at Idle and is refused; the capture stays in the store, no ordinary control takes it; only the unscoped stop IPC does",
        pd19_the_auto_stop_is_refused_at_idle_and_the_capture_stays,
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

// ------------------------------------------------------------------ PD-20
// Harness seed 305 (R205.5 / R203.5). A stop is in its capture stop with T released
// (status Processing). A cancel takes T and runs R205.5: its `PostDT` `TERM` wins
// the terminal as `UserCancelledProcessing`. Inside that tail, before the cancel's
// `PT:PC`, the stop's capture returns zero samples (a sealed delivery of nothing,
// `audio_lifecycle.rs:2358-2362`) and its R203.5 empty-audio rows finish performance
// as `NoSpeech`. The cancel's `PT:PC(Cancelled)` then loses the performance claim.
// (R203.5 is the zero-sample row at status Idle; R203.7 is the short row, 1..4,799
// samples, at Idle. The seed's split is R203.5, so the stop returns none.)
//
// Where the empty capture comes from (D-S2b-11 (g)): the recording is started by a
// **preview handoff**. A Settings microphone test is kept, the confirmed dictation start
// hands the running worker to dictation in place (`hand_preview_to`, `audio_lifecycle.rs:
// 1719-1830`): it begins fresh PCM retention, so nothing the preview saw is retained
// (`begin_handoff_retention`, 1732), and publishes `Accepted` and `Ready` (1802, 1807).
// A stop that lands before the worker captured anything new has nothing to deliver. The
// fixture drives that start through the host's `start_dictation_recording_after_preview`
// port (the trace names it) and the stop's empty capture follows.

fn pd20_stop_empty_path_between_cancel_tail_and_perf_complete(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyBeforeContext;
    world
        .state()
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a Settings microphone test is kept");
    let mut start = world.start();
    let reply = complete_joining(&mut start);
    world.reply("start", &reply);
    assert!(reply.is_ok(), "{reply:?}");
    assert!(
        world
            .trace()
            .iter()
            .any(|line| line.starts_with("supervisor.start_after_preview 1 ")),
        "the start was the preview handoff"
    );
    assert_eq!(status(world), DictationStatus::Recording);
    world.knobs().stop_result = Ok(0);
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    let (finish, reply) = stop.finish_into();
    world.on("keyboard.set_processing:false", finish);
    let mut cancel = world.cancel(None);
    let cancel_reply = complete(&mut cancel);
    world.reply_unit("cancel", &cancel_reply);
    assert_eq!(cancel_reply, Ok(()));
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the hook finished the stop");
    world.reply("stop", &reply);
    assert_eq!(reply, empty_transcription());
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
fn pd20_cancel_terminal_and_a_short_stop_performance_outcome_disagree() {
    repeat(&sub(
        "PD-20: a stop's empty-audio path completes performance inside a cancel's tail (R205.5, R203.5)",
        pd20_stop_empty_path_between_cancel_tail_and_perf_complete,
    ));
}

// ------------------------------------------------------------------ PD-21
// Harness seed 244,103 (R206.2 / R203.4). The cancel of a `Recording` dictation
// (R205.4) releases T for its capture cancel; before its `Cancel` reaches the
// supervisor's queue a stop lands (R201.8), takes T, and its `Stop` is enqueued
// first. The supervisor is busy, so the cancel's 2 s receive expires with the request
// still queued (`audio_lifecycle.rs:3353-3363`): the client gets
// [`CANCEL_RECEIVE_TIMEOUT`] and R206.2 claims the terminal as `PipelineFailure`.
// The supervisor then handles the queue in order: the `Stop` first (`Recording ->
// Stopping`, `1229-1280`), whose reply is the sealed capture at the worker's exit; the
// late `Cancel` finds `Stopping` and answers `Ok(false)` with no notification
// (`1301-1330`, `should_abandon` is false). The stop's capture is empty at status
// Processing (R203.4): its terminal loses and it completes performance as `NoSpeech`.

fn pd21_timed_out_cancel_then_a_stop_that_wins_capture(world: &Arc<World>) {
    recording(world);
    {
        let mut knobs = world.knobs();
        knobs.stop_result = Ok(0);
        knobs.cancel_result = Err(CANCEL_RECEIVE_TIMEOUT);
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
    assert_eq!(cancel_reply, Err(CANCEL_RECEIVE_TIMEOUT.to_string()));
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
    // The late `Cancel` answers `Ok(false)` and publishes nothing: the only `Recovering`
    // the supervisor publishes below is the worker's own runtime failure.
    // Seed 244,103's order after the cancel's receive expired: the worker failed at
    // runtime with 12,000 samples retained (`2630-2698`, `Recovering{RuntimeFailure}`,
    // R209.2 at `Processing`), `finish_attempt` stored the capture and notified
    // `Interrupted` (`2405-2432`; the arm schedules the auto-stop, R215.2), the
    // auto-stop is refused at `Processing` (R201.2) and the owner clears (`2460`). Only
    // then does the parked stop's capture stop return, and its reply is empty (R203.4).
    world.runtime_failure(1);
    world.finish_interrupted(1, 12_000);
    assert_eq!(
        world.auto_stop_requests(),
        vec![1],
        "the Interrupted arm scheduled the auto-stop"
    );
    let refused = stop_reply(world, Some(1));
    assert_eq!(
        refused,
        Ok(serde_json::json!({ "type": "already_processing", "state": "processing" })),
        "the auto-stop is refused at Processing (R201.2)"
    );
    world.release_owner_clear();
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(reply, empty_transcription());
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
    let trace = world.trace();
    assert!(
        !trace
            .iter()
            .any(|line| line.starts_with("supervisor.notify 1 Recovering") && line.contains("User")),
        "the late cancel published no Recovering{{User}}: {trace:#?}"
    );
    let cleared = position(&trace, "supervisor.clear_owner 1");
    let stopped = trace
        .iter()
        .position(|line| line.starts_with("supervisor.stop 1 "))
        .expect("the stop reached the capture stop");
    assert!(
        cleared > stopped,
        "the owner cleared while the stop was parked"
    );
}

#[test]
fn pd21_timed_out_cancel_then_a_short_stop_disagree() {
    repeat(&sub(
        "PD-21: a capture cancel times out while a stop that won the capture is parked (R206.2, R203.4)",
        pd21_timed_out_cancel_then_a_stop_that_wins_capture,
    ));
}

// Harness seed 207 (R206.2 / R209.4). The queued `Cancel{User}` is handled after the
// client's 2 s receive expired: the attempt is still `Recording`, so `begin_recovery`
// publishes `Recovering{User}` (`audio_lifecycle.rs:1301-1330`) and the reply goes to a
// dropped receiver. R206.2 had already claimed the terminal as `PipelineFailure`; the
// late `Recovering{User}` arm (R209.4) completes performance as `Cancelled`.

fn pd21_timed_out_cancel_then_the_supervisor_recovers(world: &Arc<World>) {
    recording(world);
    world.knobs().cancel_result = Err(CANCEL_RECEIVE_TIMEOUT);
    let mut cancel = world.cancel(None);
    let reply = complete(&mut cancel);
    world.reply_unit("cancel", &reply);
    assert_eq!(reply, Err(CANCEL_RECEIVE_TIMEOUT.to_string()));
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    assert_eq!(perf_claim(1), None, "no performance outcome yet");
    // `begin_recovery(User)` publishes `Recovering{User}` (`1313-1329`, `2630-2700`).
    world.notify(
        1,
        AudioLifecycleEvent::Recovering {
            reason: AudioCancelReason::User,
        },
    );
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::PipelineFailure,
            DictationErrorCode::RuntimeFailure
        )),
        "the supervisor's terminal lost the claim"
    );
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd21_timed_out_cancel_and_the_late_recovery_disagree() {
    repeat(&sub(
        "PD-21: a capture cancel times out, then the supervisor recovers (R206.2, R209.4)",
        pd21_timed_out_cancel_then_the_supervisor_recovers,
    ));
}

// ------------------------------------------------------------------ PD-22
// Harness seeds 5,084 (R211.2 / R203.2) and 22,704 (R211.2 / R205.5). A stop is in its
// capture stop with T released (status Processing) and its 12 s receive expires
// (`audio_lifecycle.rs:3276-3282`): the client's reply is [`STOP_RECEIVE_TIMEOUT`]
// and the receiver is dropped. The stop's continuation has not run when the worker
// exits: `finish_attempt` finds the `Stopping` attempt's `response.send` failing and
// notifies `Idle` (`2358-2373`, no sealed delivery). A cancel at `Processing` runs
// R205.5; at its tail's `CX-` (D released, status already Idle, the id still current,
// the cancel's own `TERM` not yet run) that `Idle` is handled: R211.2 claims the
// terminal as `Superseded/StaleOwner` (PD-2). The injection's sender and state are the
// ones cited: only an expired stop receive reaches it.

fn idle_at_cx_minus(world: &Arc<World>, then: impl FnOnce() + 'static) {
    let supervisor = Arc::clone(world);
    let mut landing = Some(then);
    let barrier = crate::barrier_hook::install(move |point| {
        if point != "adapter.clear_context" {
            return;
        }
        if let Some(then) = landing.take() {
            // `audio_lifecycle.rs:2358-2373`
            supervisor.notify(1, AudioLifecycleEvent::Idle);
            then();
        }
    });
    cancelled(world, "cancel");
    drop(barrier);
}

fn the_stop_receive_expired(world: &Arc<World>) -> ParkedStop {
    recording(world);
    world.knobs().stop_result = Err(STOP_RECEIVE_TIMEOUT);
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    stop
}

const SUPERSEDED: (DictationTerminalOutcome, DictationErrorCode) = (
    DictationTerminalOutcome::Superseded,
    DictationErrorCode::StaleOwner,
);

/// The cancel runs to its end first: its `PT:PC(Cancelled)` wins performance before
/// the stop's continuation is released. The terminal says superseded, the
/// performance record says cancelled.
fn pd22_cancel_wins_performance_before_the_stops_failure_tail(world: &Arc<World>) {
    let stop = the_stop_receive_expired(world);
    idle_at_cx_minus(world, || {});
    assert_eq!(
        terminal(world, 1),
        Some(SUPERSEDED),
        "the supervisor's Idle claimed the terminal before the cancel's TERM"
    );
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
    // Now the stop's failure tail: its terminal and its performance completion both lose.
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(reply, Err(STOP_RECEIVE_TIMEOUT.to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance() {
    repeat(&sub(
        "PD-22: an expired stop's Idle before a cancel's terminal claim; the cancel completes performance (R211.2, R205.5)",
        pd22_cancel_wins_performance_before_the_stops_failure_tail,
    ));
}

/// The stop's continuation is released inside the cancel's tail: R203.2 completes
/// performance as `Failed` before the cancel's `PT:PC`.
fn pd22_stops_failure_tail_wins_performance(world: &Arc<World>) {
    let stop = the_stop_receive_expired(world);
    let (finish, reply) = stop.finish_into();
    idle_at_cx_minus(world, finish);
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the barrier finished the stop");
    world.reply("stop", &reply);
    assert_eq!(reply, Err(STOP_RECEIVE_TIMEOUT.to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Failed { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance() {
    repeat(&sub(
        "PD-22: an expired stop's Idle before a cancel's terminal claim; the stop's failure tail completes performance (R211.2, R203.2)",
        pd22_stops_failure_tail_wins_performance,
    ));
}

// ------------------------------------------------------------------ PD-24
// Harness seeds 6,047 (R102.3 / R209.3, the user's cancel) and 18,279 (R102.3 / R209.5,
// an environmental recovery). The start call's 2 s receiver expired while the supervisor
// was still handling the start (`await_start_response`, `audio_lifecycle.rs:3065-3079`):
// the caller is revoked, `Cancel{User}` is enqueued, and the call returns
// `SupervisorUnavailable`, so the start thread runs R102 (a failure result). The
// supervisor's `Recovering` arm is in flight for the same recording, started by
//
// * R209.3: the queued `Cancel{User}` (`handle_message`, `1301-1337`:
//   `begin_recovery(User)` sets the phase and notifies `Recovering{User}`);
// * R209.5: an environment change while the start waits
//   (`cancel_starting_for_environment_change`, `3322-3330`: `Cancel{owner: None,
//   starting_only: true}`, handled by the same arm; here `SystemWake`).
//
// The bridge's arm takes D (`Starting -> Recovering`) and runs its tail outside D. The
// start thread's R102.3 takes D inside that tail, after `audio-recovery-started` and
// before the arm's `PC`/`TERM`: it writes `Idle` and claims the terminal as
// `CaptureInitFailure`; the arm's `PC(Cancelled)` then completes performance and its
// `TERM` loses. The mismatch is at the coordinator's claim: the failed start never
// begins a performance run, so no row is behind it (`performance_metrics/repository.rs:
// 421`, `request_completion`, `recording.rs:3114`). C4.4 and T2 name the race; C7.1 lists no
// row for it. Each variant is its own signature, bound to the arm's cause by its row.

fn pd24_start_timeout_claims_inside_the_recovering_arms_tail(
    world: &Arc<World>,
    reason: AudioCancelReason,
) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SupervisorUnavailable);
        knobs.accept_before_failing = false;
    }
    let (entered, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    world.on("supervisor.start", move || {
        let _ = entered.send(());
        let _ = release_rx.recv();
    });
    let thread_world = Arc::clone(world);
    let start = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a start thread runtime");
        let _enter = runtime.enter();
        let mut start = thread_world.start();
        let reply = complete_joining(&mut start);
        thread_world.reply("start", &reply);
        reply
    });
    entered_rx
        .recv()
        .expect("the start reached the supervisor's capture start");
    world.note("the start waits in the supervisor call; its 2 s receive is the one that expires");
    // `handle_start` accepted it and has not answered (`1481-1588`).
    world.accepted_start(1);
    let reply: Arc<std::sync::Mutex<Option<StopReply>>> = Arc::new(std::sync::Mutex::new(None));
    let filed = Arc::clone(&reply);
    world.on("emit:audio-recovery-started", move || {
        // The receiver expired: the start thread returns `SupervisorUnavailable` and
        // runs R102 now, inside the arm's tail.
        let _ = release.send(());
        let result = start.join().expect("the start thread panicked");
        *filed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
    });
    // The supervisor handles the cancel that abandons the starting attempt
    // (`begin_recovery(reason)`, `1301-1337`).
    world.recovering(1, reason);
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the hook finished the start");
    assert_eq!(
        reply,
        Err("Audio lifecycle supervisor is unavailable".to_string())
    );
    assert_eq!(status(world), DictationStatus::Idle);
    // The terminal says the capture failed to start; the performance record says the
    // recording was cancelled.
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::RuntimeFailure
        ))
    );
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd24_start_timeout_terminal_and_the_users_cancel_arm_performance_outcome_disagree() {
    repeat(&sub(
        "PD-24: the start's 2 s timeout claims CaptureInitFailure inside the supervisor's Recovering{User} arm (R102.3, R209.3)",
        |world| {
            pd24_start_timeout_claims_inside_the_recovering_arms_tail(
                world,
                AudioCancelReason::User,
            )
        },
    ));
}

#[test]
fn pd24_start_timeout_terminal_and_an_environmental_recovery_arm_performance_outcome_disagree() {
    repeat(&sub(
        "PD-24: the start's 2 s timeout claims CaptureInitFailure inside the supervisor's Recovering{SystemWake} arm (R102.3, R209.5)",
        |world| {
            pd24_start_timeout_claims_inside_the_recovering_arms_tail(
                world,
                AudioCancelReason::SystemWake,
            )
        },
    ));
}

// ------------------------------------------------------------------ PD-23
// Harness seeds 447 (R207.3 / R203.5), 171 (R207.3 / R203.1) and 163 (R207.3 / R205.5).
// C8 (`s10c`, PD-4): the fake supervisor sets the phase to `Recording` and holds before
// notifying `Ready`, the start's continuation publishes `Recording`, and the held `Ready`
// is handled for a recording that is already `Recording`: R207.3 claims
// `Superseded/StaleOwner` for the live recording. PD-4 is the case where the recording
// then completes `Success` after a second ticker spawned; PD-23 is the same claim when
// the recording's own path ends another way. The recording's own path wins no terminal
// and completes performance with its own outcome, bound to its row by the signature:
//
// * R203.5: an empty stop, `NoSpeech`;
// * R203.1: a stop whose capture stop is refused, `Failed`;
// * R205.5: a cancel of a recording whose stop is in its capture stop, `Cancelled`.
//
// The fourth variant (R204b.6, `Completed`, the ticker's gate refusing the second ticker)
// is PENDING: no fake-host schedule makes the gate refuse after R207.3's claim, so it
// is source-only and stays allowlisted (D-S2b-13 (e)).

fn pd23_c8_forced(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::HoldBeforeReady;
    start_reply(world).expect("the start returned");
    assert_eq!(status(world), DictationStatus::Recording);
    world.release_ready();
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
}

/// Seed 447's cold-start rendezvous (R207.3 / R203.5). The cold start's `Ready` is held
/// until the start's continuation published `Recording`, and R207.3 claims `Superseded`
/// for the live recording (`pd23_c8_forced`). The user cancels the `Recording` dictation
/// (R205.4): the capture cancel is accepted, `Recovering{User}` is published and the
/// attempt's own `Idle` is held, and before the cancel's capture cancel returns a stop
/// lands (R201.8), takes T and parks in the supervisor's capture stop at `Processing`.
/// A second cancel of the `Processing` dictation runs R205.5. At its tail's `CX-` (D
/// released, status already `Idle`) the attempt's `finish_attempt` publishes `Idle` and
/// clears the owner (`audio_lifecycle.rs:2457-2467`), and only then does the parked
/// capture stop return its empty reply (`2358-2362`): the stop's R203.5 empty-audio rows
/// complete performance as `NoSpeech` before the cancel's `PT:PC(Cancelled)`, which then
/// loses. The terminal stays `Superseded`.
fn pd23_an_empty_stop_completes_no_speech(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.stop_result = Ok(0);
        knobs.cancel_result = Ok(true);
    }
    pd23_c8_forced(world);
    // The first cancel (R205.4): a stop lands while its capture cancel is in the supervisor.
    let landing = Arc::clone(world);
    let parked: Arc<std::sync::Mutex<Option<ParkedStop>>> = Arc::new(std::sync::Mutex::new(None));
    let filed = Arc::clone(&parked);
    world.on("supervisor.cancel", move || {
        let stop = ParkedStop::start(&landing);
        *filed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(stop);
    });
    let mut first = world.cancel(None);
    let first_reply = complete(&mut first);
    world.reply_unit("cancel", &first_reply);
    assert_eq!(first_reply, Ok(()));
    assert_eq!(status(world), DictationStatus::Processing);
    let stop = parked
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the hook parked the stop");
    // The second cancel (R205.5). At its `CX-` the attempt's own `Idle` and the owner
    // clear run, then the parked stop's empty reply.
    let (finish, reply) = stop.finish_into();
    let supervisor = Arc::clone(world);
    let mut landing = Some(finish);
    let barrier = crate::barrier_hook::install(move |point| {
        if point != "adapter.clear_context" {
            return;
        }
        if let Some(finish) = landing.take() {
            supervisor.release_cancel_finish();
            finish();
        }
    });
    let mut second = world.cancel(None);
    let second_reply = complete(&mut second);
    drop(barrier);
    world.reply_unit("cancel", &second_reply);
    assert_eq!(second_reply, Ok(()));
    let reply = reply
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .expect("the barrier finished the stop");
    world.reply("stop", &reply);
    assert_eq!(reply, empty_transcription());
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
    assert_eq!(
        perf_claim(1),
        Some(RunOutcomeV1::NoSpeech),
        "NoSpeech won performance before the cancel tail completed"
    );
    let trace = world.trace();
    let cleared = position(&trace, "supervisor.clear_owner 1");
    let stopped = trace
        .iter()
        .position(|line| line.starts_with("supervisor.stop 1 "))
        .expect("the stop reached the capture stop");
    assert!(
        cleared > stopped,
        "the owner cleared while the stop was parked"
    );
}

#[test]
fn pd23_superseded_terminal_and_an_empty_stop_no_speech_disagree() {
    repeat(&sub(
        "PD-23: R207.3's Superseded for a live recording, then a stop that gets its empty reply after the owner cleared completes NoSpeech inside a cancel's tail (R207.3, R203.5)",
        pd23_an_empty_stop_completes_no_speech,
    ));
}

/// The stop's capture stop is refused by the supervisor's recovery (`1291-1295`: the worker
/// failed at runtime, the attempt is `Recovering`): R203.1 completes performance as
/// `Failed`.
fn pd23_a_refused_stop_completes_failed(world: &Arc<World>) {
    world.knobs().stop_result = Err(REFUSED_STOP_IN_RECOVERY);
    pd23_c8_forced(world);
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    // The worker failed at runtime while the stop waited (`2630-2698`): the supervisor's
    // attempt is `Recovering`, so the stop's capture stop is refused.
    world.runtime_failure(1);
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(reply, Err(REFUSED_STOP_IN_RECOVERY.to_string()));
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Failed { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd23_superseded_terminal_and_a_refused_stop_failed_disagree() {
    repeat(&sub(
        "PD-23: R207.3's Superseded for a live recording, then a stop refused by the recovering supervisor completes Failed (R207.3, R203.1)",
        pd23_a_refused_stop_completes_failed,
    ));
}

/// A stop is in its capture stop (status `Processing`, T released) and the user cancels:
/// R205.5 loses the terminal claim (`Superseded` is held) and its `PC(Cancelled)` wins
/// performance; the stop's own completion then loses.
fn pd23_a_cancel_of_a_processing_recording_completes_cancelled(world: &Arc<World>) {
    world.knobs().stop_result = Ok(0);
    pd23_c8_forced(world);
    let stop = ParkedStop::start(world);
    assert_eq!(status(world), DictationStatus::Processing);
    let mut cancel = world.cancel(None);
    let cancel_reply = complete(&mut cancel);
    world.reply_unit("cancel", &cancel_reply);
    assert_eq!(cancel_reply, Ok(()));
    assert_eq!(status(world), DictationStatus::Idle);
    let reply = stop.finish();
    world.reply("stop", &reply);
    assert_eq!(reply, empty_transcription());
    assert_eq!(status(world), DictationStatus::Idle);
    assert_eq!(terminal(world, 1), Some(SUPERSEDED));
    assert!(
        matches!(perf_claim(1), Some(RunOutcomeV1::Cancelled { .. })),
        "{:?}",
        perf_claim(1)
    );
}

#[test]
fn pd23_superseded_terminal_and_a_cancel_of_processing_cancelled_disagree() {
    repeat(&sub(
        "PD-23: R207.3's Superseded for a live recording, then a cancel of the processing recording completes Cancelled (R207.3, R205.5)",
        pd23_a_cancel_of_a_processing_recording_completes_cancelled,
    ));
}
