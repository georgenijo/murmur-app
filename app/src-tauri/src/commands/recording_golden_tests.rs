//! Stage 0b golden characterization scenarios (plan §6 Stage 0b, C9.4).
//!
//! Twelve scenarios, each a checked-in golden trace under
//! `recording_golden/`. A scenario records what the code does today, including
//! the preserved defects (C7.1) it witnesses; it is not a statement that the
//! behaviour is right. A trace that changes without a behaviour change on
//! purpose is a regression, and a PD that stops appearing is a behaviour
//! change (C7.0).
//!
//! Run them once with `cargo test golden_`. For the exit gates (C9.3):
//!
//! * G1/G4: `MURMUR_GOLDEN_REPEAT=100 cargo test golden_ -- --test-threads=1`
//!   runs every scenario 100 times in one process and requires each run to
//!   equal its golden.
//! * Re-record a golden after an intended change with `MURMUR_GOLDEN_UPDATE=1`.
//!
//! Scenario bodies are driven by the barriers described in
//! `recording_golden_harness.rs`; none of them waits on a clock (G3 is
//! enforced by `golden_scenarios_never_wait_on_a_clock`).

use super::golden_harness::*;
use super::*;
use std::sync::Mutex as StdMutex;

type StopSlot = Arc<StdMutex<Option<Fut<Result<serde_json::Value, String>>>>>;

fn started(world: &Arc<World>) {
    let mut start = world.start();
    let reply = complete(&mut start);
    world.reply("start", &reply);
}

fn stopped(world: &Arc<World>, label: &str, expected: Option<u64>) {
    let mut stop = world.stop(expected);
    let reply = complete(&mut stop);
    world.reply(label, &reply);
}

fn cancelled(world: &Arc<World>, label: &str) {
    let mut cancel = world.cancel(None);
    let reply = complete(&mut cancel);
    world.reply_unit(label, &reply);
}

/// A dictation that is `Recording` with its context installed.
fn recording(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyBeforeContext;
    started(world);
}

// ------------------------------------------------- 1: stop during start
// Witnesses PD-12 (a stale-id stop emits a terminal for the stale id).

/// The stop queues behind a start that is parked after its supervisor ack.
/// "Queued" is observed, not timed: the stop future is pending on the
/// recording transition the start holds.
fn s01a_stop_queued_behind_start(world: &Arc<World>) {
    world.close_gate("smart_capture");
    let mut start = world.start();
    assert!(poll_once(&mut start).is_pending());
    world.note("start is parked after the supervisor ack and holds the recording transition");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    world.note("stop is parked on the transition: none of it has run");
    world.open_gate("smart_capture");
    let reply = complete(&mut start);
    world.reply("start", &reply);
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
}

/// A stop that finds the start still `Starting` cancels the exact owner.
fn s01b_stop_finds_starting(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    started(world);
    stopped(world, "stop", None);
    world.release_cancel_finish();
}

/// A stop expecting an older, accepted, unterminated id (PD-12): the current
/// generation is untouched and the stale id still receives a terminal.
fn s01c_stop_with_a_stale_expected_id(world: &Arc<World>) {
    world.accept_silently(1);
    world.accept_silently(2);
    world.install_context(2);
    stopped(world, "stop(expected=1)", Some(1));
}

// ----------------------------------------------- 2: cancel in each phase
// Witnesses PD-16 (cancel's post-lock clears run with no recheck).

fn s02a_cancel_starting(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    started(world);
    cancelled(world, "cancel");
    world.release_cancel_finish();
}

fn s02b_cancel_recording(world: &Arc<World>) {
    recording(world);
    cancelled(world, "cancel");
    world.release_cancel_finish();
}

/// Cancel while the pipeline still holds the delivery path: the stop is parked
/// in the pipeline, the cancel resets the UI, and the pipeline returns
/// afterwards.
fn s02c_cancel_processing_with_the_pipeline_running(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.knobs().pipeline = PipelineScript::Cancelled;
    world.close_gate("pipeline");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    world.note("the pipeline is running: status is Processing and the transition is free");
    cancelled(world, "cancel");
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
}

/// PD-16: a supervisor `Idle` for the same id lands between the cancel's
/// status write and its own idle emit (the hook runs at `set_processing`).
fn s02d_cancel_processing_with_a_supervisor_event_in_the_tail(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.knobs().pipeline = PipelineScript::Cancelled;
    world.close_gate("pipeline");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    let tail = Arc::clone(world);
    world.on("keyboard.set_processing:false", move || {
        tail.notify(1, AudioLifecycleEvent::Idle)
    });
    cancelled(world, "cancel");
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
}

// ------------------------------------- 3: interruption of 8,000+ samples
// Witnesses PD-15 (a path destructively takes the interrupted capture).

fn interrupted_recording(world: &Arc<World>, samples: usize) {
    recording(world);
    world.runtime_failure(1);
    world.finish_interrupted(1, samples);
}

/// `Interrupted` is accepted and the bridge schedules the auto-stop.
fn s03a_interrupted_is_accepted_and_auto_stops(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    for recording_id in world.auto_stop_requests() {
        stopped(world, "auto-stop", Some(recording_id));
    }
    world.release_owner_clear();
}

/// A user stop takes the capture first; the auto-stop then finds Processing.
fn s03b_user_stop_wins_the_race(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    world.close_gate("pipeline");
    let mut user = world.stop(None);
    assert!(poll_once(&mut user).is_pending());
    world.note("user stop took the capture and is in the pipeline");
    for recording_id in world.auto_stop_requests() {
        stopped(world, "auto-stop", Some(recording_id));
    }
    world.open_gate("pipeline");
    let reply = complete(&mut user);
    world.reply("user stop", &reply);
    world.release_owner_clear();
}

/// The auto-stop takes the capture first; the user stop then finds Processing.
fn s03c_auto_stop_wins_the_race(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    world.close_gate("pipeline");
    let requests = world.auto_stop_requests();
    let mut auto = world.stop(Some(requests[0]));
    assert!(poll_once(&mut auto).is_pending());
    world.note("auto-stop took the capture and is in the pipeline");
    stopped(world, "user stop", None);
    world.open_gate("pipeline");
    let reply = complete(&mut auto);
    world.reply("auto-stop", &reply);
    world.release_owner_clear();
}

// -------------------------------------- 4: interruption under 8,000 samples
// Witnesses PD-1 (4b), PD-15 and PD-18 (a) (4d is the golden witness).

/// (a) The plain path: the bridge restores Idle itself.
fn s04a_plain_short_interruption(world: &Arc<World>) {
    interrupted_recording(world, 6_000);
    world.release_owner_clear();
}

/// (b) PD-1: a user stop runs after the bridge read "current" and before its
/// unguarded Idle write. The hook runs at the bridge's `recording-interrupted`
/// emit, which is that window.
fn s04b_user_stop_between_the_check_and_the_idle_write(world: &Arc<World>) {
    recording(world);
    world.runtime_failure(1);
    world.close_gate("pipeline");
    let slot: StopSlot = Arc::new(StdMutex::new(None));
    let (between, parked) = (Arc::clone(world), Arc::clone(&slot));
    world.on("emit:recording-interrupted", move || {
        let mut stop = between.stop(None);
        assert!(poll_once(&mut stop).is_pending());
        *parked.lock().expect("slot") = Some(stop);
    });
    world.finish_interrupted(1, 6_000);
    world.note("the bridge has returned; the user stop is still in the pipeline");
    world.open_gate("pipeline");
    let mut stop = slot.lock().expect("slot").take().expect("a parked stop");
    let reply = complete(&mut stop);
    world.reply("user stop", &reply);
    world.release_owner_clear();
}

/// (c) Observed at the supervisor: after `Interrupted` no `Idle` follows, so
/// the bridge's own write is the only thing that restores Idle.
fn s04c_no_idle_follows_an_interrupted_end(world: &Arc<World>) {
    interrupted_recording(world, 6_000);
    world.note("Interrupted was the last notification; the supervisor now clears its owner");
    world.release_owner_clear();
    let notified: Vec<String> = world
        .trace()
        .into_iter()
        .filter(|line| line.starts_with("supervisor.notify 1 Idle"))
        .collect();
    assert!(
        notified.is_empty(),
        "an Idle followed Interrupted: {notified:?}"
    );
}

/// (d) PD-18 (a): a start takes the lock after the bridge restored Idle and
/// before the supervisor clears its owner. It reads cleanup pending, writes
/// `Idle -> Recovering` and nothing is left to restore Idle.
fn s04d_start_before_the_owner_is_cleared(world: &Arc<World>) {
    interrupted_recording(world, 6_000);
    world.note("the bridge restored Idle; the supervisor is held before clearing its owner");
    let mut start = world.start();
    let reply = complete(&mut start);
    world.reply("start", &reply);
    world.release_owner_clear();
    world.note("owner cleared; the supervisor has nothing more to notify");
    let mut again = world.start();
    let reply = complete(&mut again);
    world.reply("start again", &reply);
}

// --------------------------------------------- 5: timeout teardown (#384)
// Witnesses PD-6 (and PD-7: the supervisor's Idle lands during Processing).

fn s05_stop_times_out_and_idle_follows(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Err("stop timed out");
    let idle = Arc::clone(world);
    world.on("supervisor.stop", move || {
        idle.notify(1, AudioLifecycleEvent::Idle)
    });
    stopped(world, "stop", None);
}

// ---------------------------------------------------------- 6: stale stop
// Witnesses PD-12.

/// Id 1 completes and has a terminal; a newer start takes id 2; a stop that
/// still expects id 1 replies superseded and its terminal loses the ledger.
fn s06a_stale_stop_after_a_newer_start(world: &Arc<World>) {
    recording(world);
    stopped(world, "stop", None);
    recording(world);
    stopped(world, "stale stop(expected=1)", Some(1));
}

/// The stale id has no terminal yet: the stale stop's terminal is claimed.
fn s06b_stale_stop_for_an_unterminated_id(world: &Arc<World>) {
    world.accept_silently(1);
    world.accept_silently(2);
    world.install_context(2);
    stopped(world, "stale stop(expected=1)", Some(1));
}

// ------------------------------------------------------ 7: meeting conflict
// Admission only (SP9): no preserved defect.

fn s07a_dictation_start_while_a_meeting_is_active(world: &Arc<World>) {
    world
        .state()
        .app_state
        .meeting_active
        .store(true, Ordering::SeqCst);
    started(world);
}

fn s07b_meeting_start_while_dictation_is_starting(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    started(world);
    world.probe_meeting_conflict();
    world.release_ready();
}

// ------------------------------------------------------ 8: preview handoff
// No preserved defect.

fn s08_kept_preview_handed_to_the_dictation_start(world: &Arc<World>) {
    world
        .state()
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
    world.note("a Settings microphone test is kept; the start hands it to the supervisor");
    let mut start = world.start();
    let reply = complete_joining(&mut start);
    world.reply("start", &reply);
}

// --------------------------------- 9: supervisor Idle during Processing
// Witnesses PD-7 and PD-6.

fn s09_supervisor_idle_while_the_pipeline_runs(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Ok(16_000);
    world.close_gate("pipeline");
    let mut stop = world.stop(None);
    assert!(poll_once(&mut stop).is_pending());
    world.note("the pipeline is running for id 1");
    world.notify(1, AudioLifecycleEvent::Idle);
    world.note("the pipeline now finishes and runs its own terminal");
    world.open_gate("pipeline");
    let reply = complete(&mut stop);
    world.reply("stop", &reply);
}

// -------------------------- 10: ready vs context rendezvous, both orders
// Witnesses PD-4.

/// (a) The context is installed first, then `Ready` arrives (R207.5).
fn s10a_context_then_ready(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    started(world);
    world.note("the context is installed and the command has returned; Ready arrives now");
    world.release_ready();
}

/// (b) `Ready` first, then the context.
fn s10b_ready_then_context(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyBeforeContext;
    started(world);
}

/// (c) C8 forced (C9.5), known-bad. The fake supervisor sets the phase to
/// `Recording` and holds before notifying `Ready`. The command's continuation
/// publishes `Recording`; only then is the hold released, so the bridge sees
/// `Ready` for a recording that is already `Recording`.
fn s10c_c8_forced(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::HoldBeforeReady;
    world.knobs().stop_result = Ok(16_000);
    started(world);
    world.note("observable: the status emit `recording` is in the trace above");
    world.release_ready();
    world.note("Ready was handled; now the recording runs to Success");
    stopped(world, "stop", None);
}

// --------------- 11: Idle on an already-finished recording (C9)
// Witnesses PD-2.

/// After S3 (the start failure path) already wrote Idle and claimed a terminal.
fn s11a_idle_after_a_failed_start(world: &Arc<World>) {
    world.knobs().start_ack = Some(AudioStartError::SpawnFailed(
        "scripted start failure".to_string(),
    ));
    started(world);
    world.notify(1, AudioLifecycleEvent::Idle);
}

/// After a stop that already completed with a terminal.
fn s11b_idle_after_a_finished_stop(world: &Arc<World>) {
    recording(world);
    stopped(world, "stop", None);
    world.notify(1, AudioLifecycleEvent::Idle);
}

/// With no earlier terminal: the stale-owner terminal is the only one.
fn s11c_idle_when_no_terminal_exists(world: &Arc<World>) {
    world.accept_silently(1);
    world.notify(1, AudioLifecycleEvent::Idle);
}

// ------- 12: start failure vs InitializationFailed + Idle
// Witnesses PD-3 (S3's unconditional idle emit) and SP1.

/// (a) The failure ack arrives before the supervisor's own failure terminal.
fn s12a_ack_before_the_supervisor_failure(world: &Arc<World>) {
    world.knobs().start_ack = Some(AudioStartError::SpawnFailed(
        "scripted start failure".to_string(),
    ));
    started(world);
    world.notify(
        1,
        AudioLifecycleEvent::InitializationFailed {
            error: "scripted initialization failure".to_string(),
            kind: audio::AudioFailureKind::BackendError,
            recovery_reason: None,
        },
    );
    world.notify(1, AudioLifecycleEvent::Idle);
}

/// (b) The supervisor's failure terminal and `Idle` arrive before the ack.
fn s12b_supervisor_failure_before_the_ack(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SpawnFailed(
            "scripted start failure".to_string(),
        ));
        knobs.before_ack = vec![
            AudioLifecycleEventScript::InitializationFailed,
            AudioLifecycleEventScript::Idle,
        ];
    }
    started(world);
}

/// (c) The 2 s start timeout: a supervisor `Cancel{User}` makes the bridge
/// publish Recovering, then S3 writes Idle and emits `idle`.
fn s12c_start_timeout_with_a_user_cancel(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SpawnFailed(
            "scripted 2 s start timeout".to_string(),
        ));
        knobs.before_ack = vec![AudioLifecycleEventScript::Recovering(
            AudioCancelReason::User,
        )];
    }
    started(world);
}

// ------------------------------------------------------------- the goldens

fn sub(title: &'static str, body: fn(&Arc<World>)) -> SubRun {
    SubRun {
        title,
        setup: Setup::idle(),
        body,
    }
}

fn sub_from(
    title: &'static str,
    status: DictationStatus,
    id: u64,
    body: fn(&Arc<World>),
) -> SubRun {
    SubRun {
        title,
        setup: Setup {
            status,
            recording_id: id,
        },
        body,
    }
}

fn nothing(_: &str) {}

#[test]
fn golden_s01_stop_during_start() {
    check_golden(
        "s01-stop-during-start",
        "1 stop during start",
        "PD-12",
        None,
        &[
            sub("1a stop queued behind start", s01a_stop_queued_behind_start),
            sub(
                "1b stop finds the start still Starting",
                s01b_stop_finds_starting,
            ),
            sub_from(
                "1c stop with a stale expected id",
                DictationStatus::Recording,
                2,
                s01c_stop_with_a_stale_expected_id,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s02_cancel_in_each_phase() {
    check_golden(
        "s02-cancel-in-each-phase",
        "2 cancel in each phase",
        "PD-16",
        None,
        &[
            sub("2a cancel while Starting", s02a_cancel_starting),
            sub("2b cancel while Recording", s02b_cancel_recording),
            sub(
                "2c cancel while Processing, pipeline running",
                s02c_cancel_processing_with_the_pipeline_running,
            ),
            sub(
                "2d cancel while Processing, supervisor event in the tail (PD-16)",
                s02d_cancel_processing_with_a_supervisor_event_in_the_tail,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s03_interruption_of_8000_samples_or_more() {
    check_golden(
        "s03-interruption-8000-or-more",
        "3 interruption of 8,000 samples or more",
        "PD-15",
        None,
        &[
            sub(
                "3a Interrupted accepted, auto-stop transcribes",
                s03a_interrupted_is_accepted_and_auto_stops,
            ),
            sub(
                "3b auto-stop races a user stop: the user stop wins",
                s03b_user_stop_wins_the_race,
            ),
            sub(
                "3c auto-stop races a user stop: the auto-stop wins",
                s03c_auto_stop_wins_the_race,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s04_interruption_under_8000_samples() {
    check_golden(
        "s04-interruption-under-8000",
        "4 interruption under 8,000 samples",
        "PD-1 (4b), PD-15, PD-18 (a) (4d is the golden witness for the family)",
        None,
        &[
            sub("4a plain S12", s04a_plain_short_interruption),
            sub(
                "4b S12 with a user stop between the check and the Idle write (PD-1)",
                s04b_user_stop_between_the_check_and_the_idle_write,
            ),
            sub(
                "4c plain S12 observed at the supervisor: no Idle follows",
                s04c_no_idle_follows_an_interrupted_end,
            ),
            sub(
                "4d start takes R101.12 before the owner is cleared (PD-18 a)",
                s04d_start_before_the_owner_is_cleared,
            ),
        ],
        &|text| {
            let section = text
                .split("=== ")
                .find(|part| part.starts_with("4d"))
                .expect("the 4d sub-run is in the golden");
            assert!(section.contains("~ status Idle -> Recovering"));
            assert_eq!(
                section.matches("audio_recovering").count(),
                2,
                "4d: the start is refused audio_recovering and so is every later start"
            );
        },
    );
}

#[test]
fn golden_s05_timeout_teardown() {
    check_golden(
        "s05-timeout-teardown",
        "5 timeout teardown (#384)",
        "PD-6, PD-7",
        None,
        &[sub(
            "5 stop times out, supervisor Idle follows, the guard finishes",
            s05_stop_times_out_and_idle_follows,
        )],
        &nothing,
    );
}

#[test]
fn golden_s06_stale_stop() {
    check_golden(
        "s06-stale-stop",
        "6 stale stop",
        "PD-12",
        None,
        &[
            sub(
                "6a stale stop for id-1 after a newer start",
                s06a_stale_stop_after_a_newer_start,
            ),
            sub_from(
                "6b stale stop for an unterminated id",
                DictationStatus::Recording,
                2,
                s06b_stale_stop_for_an_unterminated_id,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s07_meeting_conflict() {
    check_golden(
        "s07-meeting-conflict",
        "7 meeting conflict",
        "none (admission only, SP9)",
        None,
        &[
            sub(
                "7a dictation start while a meeting is active",
                s07a_dictation_start_while_a_meeting_is_active,
            ),
            sub(
                "7b meeting start while dictation is Starting",
                s07b_meeting_start_while_dictation_is_starting,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s08_preview_handoff() {
    check_golden(
        "s08-preview-handoff",
        "8 preview handoff",
        "none",
        None,
        &[sub(
            "8 kept preview handed to the dictation start",
            s08_kept_preview_handed_to_the_dictation_start,
        )],
        &nothing,
    );
}

#[test]
fn golden_s09_supervisor_idle_during_processing() {
    check_golden(
        "s09-supervisor-idle-during-processing",
        "9 supervisor Idle during Processing",
        "PD-7, PD-6",
        None,
        &[sub(
            "9 Idle for the current id while the pipeline runs",
            s09_supervisor_idle_while_the_pipeline_runs,
        )],
        &nothing,
    );
}

/// G5 (C9.5): the C8 trace holds a `Superseded` terminal for an id whose
/// status is `Recording` at that moment, and two ticker registrations.
fn assert_c8_recorded(text: &str) {
    let section = text
        .split("=== ")
        .find(|part| part.starts_with("10c"))
        .expect("the 10c sub-run is in the golden");
    let position = |needle: &str| {
        section
            .find(needle)
            .unwrap_or_else(|| panic!("10c lacks `{needle}`"))
    };
    let recording = position("~ status Starting -> Recording");
    let superseded = position("~ terminal 1 Superseded/StaleOwner");
    let processing = position("~ status Recording -> Processing");
    assert!(
        recording < superseded && superseded < processing,
        "the Superseded terminal must be claimed while the status is Recording"
    );
    assert_eq!(
        section.matches("SD ticker 1 registered=true").count(),
        2,
        "the second ticker registration is not suppressed"
    );
}

#[test]
fn golden_s10_ready_vs_context_rendezvous() {
    check_golden(
        "s10-ready-vs-context",
        "10 ready vs context rendezvous, both orders",
        "PD-4",
        Some("10c records C8 under today's behaviour; it flips only in BC-2's fix PR (C11.1)"),
        &[
            sub(
                "10a context first, then Ready (R207.5)",
                s10a_context_then_ready,
            ),
            sub("10b Ready first, then context", s10b_ready_then_context),
            sub("10c C8 forced (C9.5), known-bad", s10c_c8_forced),
        ],
        &assert_c8_recorded,
    );
}

#[test]
fn golden_s11_idle_on_a_finished_recording() {
    check_golden(
        "s11-idle-on-finished-recording",
        "11 Idle on an already-finished recording (C9)",
        "PD-2",
        None,
        &[
            sub("11a after S3 wrote Idle", s11a_idle_after_a_failed_start),
            sub("11b after a stop finished", s11b_idle_after_a_finished_stop),
            sub_from(
                "11c with no earlier terminal",
                DictationStatus::Idle,
                1,
                s11c_idle_when_no_terminal_exists,
            ),
        ],
        &nothing,
    );
}

#[test]
fn golden_s12_start_failure_vs_initialization_failed() {
    check_golden(
        "s12-start-failure-vs-initialization-failed",
        "12 start failure vs InitializationFailed + Idle",
        "PD-3 (S3), SP1",
        None,
        &[
            sub(
                "12a failure ack before the supervisor's failure terminal",
                s12a_ack_before_the_supervisor_failure,
            ),
            sub(
                "12b supervisor failure terminal before the ack",
                s12b_supervisor_failure_before_the_ack,
            ),
            sub(
                "12c 2 s start timeout with a supervisor Cancel{User}",
                s12c_start_timeout_with_a_user_cancel,
            ),
        ],
        &nothing,
    );
}

// ------------------------------------------------------------------- G3

/// G3 (C9.3): no scenario waits on a clock. The needles are assembled here so
/// this file does not contain them.
#[test]
fn golden_scenarios_never_wait_on_a_clock() {
    let needles = [
        ["sle", "ep("].concat(),
        ["sle", "ep_until"].concat(),
        ["park_", "timeout"].concat(),
        ["time::", "timeout"].concat(),
        ["recv_", "timeout"].concat(),
        ["wait_", "timeout"].concat(),
        ["Instant", "::now"].concat(),
    ];
    for file in ["recording_golden_harness.rs", "recording_golden_tests.rs"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/commands")
            .join(file);
        let source = std::fs::read_to_string(&path).expect("read scenario source");
        for needle in &needles {
            assert!(
                !source.contains(needle.as_str()),
                "{file} contains `{needle}`: a scenario must order by barriers"
            );
        }
    }
}

/// Every golden file names the preserved defects its scenario witnesses
/// (C9.4) and the known-bad scenario says so (C9.5).
#[test]
fn golden_files_name_their_preserved_defects() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/recording_golden");
    let mut files = 0;
    for entry in std::fs::read_dir(&dir).expect("golden dir") {
        let path = entry.expect("golden entry").path();
        if path
            .extension()
            .is_none_or(|extension| extension != "golden")
        {
            continue;
        }
        files += 1;
        let text = std::fs::read_to_string(&path).expect("read golden");
        assert!(
            text.lines().any(|line| line.starts_with("# witnesses: ")),
            "{} has no `# witnesses:` header",
            path.display()
        );
    }
    assert_eq!(files, 12, "one golden per scenario");
    let c8 = std::fs::read_to_string(dir.join("s10-ready-vs-context.golden")).expect("s10 golden");
    assert!(c8.contains("# known-bad: "), "10c is recorded as known-bad");
}
