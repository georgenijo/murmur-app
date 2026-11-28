//! Shell fixtures for the fixed PD-25 (issue #1157; D-S2c-15 C4 held them as the preserved
//! defect's witnesses, and the fix PR flipped them to assert the fixed behaviour).
//!
//! PD-25 (issue #1157, confirmed real by the #1169 round-2 check): a scoped stop for
//! generation 1 is still queued ahead of the recording transition `T` while another
//! stop has already taken generation 1's retained capture and run its pipeline. The
//! pipeline's guard has written `Idle` (R204a.1) and the supervisor has cleared its
//! owner when a **new generation 2** is allocated, either by a legacy import (R301.5)
//! or by an ordinary microphone start (R101.15). The scoped stop then resumes:
//! R201.1 sees `expected = 1` against the current id 2, with the capture already
//! consumed, and (before the fix) won generation 1's terminal as `Superseded/StaleOwner`
//! while the original stop's R204b.6 completed generation 1's performance as `Success`:
//! one recording ended with a terminal and a performance outcome that disagreed.
//!
//! **Fixed.** A stale stop whose take found nothing because an earlier take consumed the
//! capture claims no terminal (R201.1 in `decide_stop_request_stale`, `taken_earlier`):
//! the stop that took the capture owns the terminal. The scoped stop still replies
//! `recording_superseded`; the original stop's R204b.6 wins generation 1's terminal
//! (`RuntimeInterruption/StreamInvalidated`: the capture was an interrupted one) and
//! completes its performance as `Success`, both from the one stop.
//!
//! The interleaving (one fixture per allocator, the same body):
//!
//! 1. Generation 1 records; a runtime failure puts it in `Recovering`; the worker's
//!    exit stores 9,000 samples and notifies `Interrupted`, so the bridge schedules the
//!    scoped stop `expected = Some(1)` (`audio_lifecycle.rs:2236,2404`,
//!    `recording.rs:6041`). The fixture keeps **the bridge's own task**, with its
//!    `AutoStop` owner (`carry_auto_stop_owner`, `World::owned_auto_stop`), and does
//!    not poll it: it is delayed ahead of `T`. A production dispatch that expected
//!    another id would change what that task does, and the fixtures would fail.
//! 2. An unscoped stop takes the capture (R201.7) and runs the pipeline. It parks at
//!    the `pipeline_idle_emit` barrier, after the pipeline's guard wrote `Idle` and
//!    before R204b.
//! 3. At the barrier, in order: the supervisor clears its owner; generation 2 is
//!    allocated; the scoped stop is polled to its reply; the barrier returns and the
//!    parked stop runs R204b.
//!
//! Ordering is by causal barriers only: no clock, no sleep.
#![cfg(test)]

use super::dictation_host::HostFuture;
use super::golden_harness::*;
use super::golden_tests::{recording, sub, sub_from};
use super::pd_witness_tests::repeat;
use super::process_audio_ports::{ImportedAudioHost, OwnedIdleEmit};
use super::*;
use murmur_dictation_core::decision::ContKind;
use murmur_dictation_core::ids::RecordingId;
use std::cell::RefCell;
use std::rc::Rc;

type ParkedStop = Rc<RefCell<Option<Fut<Result<serde_json::Value, String>>>>>;
type Reply = Rc<RefCell<Option<Result<serde_json::Value, String>>>>;

/// The original stop's terminal: the capture it took was an interrupted one, so R204b.6
/// claims the interruption's code (`interrupted` wins over `Success`).
pub(super) const INTERRUPTED: (DictationTerminalOutcome, DictationErrorCode) = (
    DictationTerminalOutcome::RuntimeInterruption,
    DictationErrorCode::StreamInvalidated,
);

pub(super) fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
}

pub(super) fn terminal(
    world: &Arc<World>,
    id: u64,
) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
    world
        .state()
        .app_state
        .dictation_telemetry
        .terminal_for_test(id)
}

pub(super) fn perf_claim(id: u64) -> Option<RunOutcomeV1> {
    dictation_performance_begins().claimed_outcome(id)
}

// ------------------------------------------------ the legacy import as an allocator

/// The legacy import's host (`process_audio_inner`'s own seam). It supplies what the
/// import needs from outside the shell, and the real pipeline `IdleGuard` its fake
/// pipeline takes, so the import allocates generation 2 through production's R301.5.
pub(super) struct ImportHost {
    /// Runs once, when the import reaches its destination capture: after its first D
    /// section (301) and before the one that follows (301.6). The stop cutover's tests
    /// plant a persisted-core divergence there.
    pub(super) at_capture: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl ImportHost {
    pub(super) fn new() -> Self {
        Self {
            at_capture: std::sync::Mutex::new(None),
        }
    }
}

impl ImportedAudioHost for ImportHost {
    fn emit_import_json(
        &self,
        _event: &str,
        _payload: serde_json::Value,
    ) -> Result<(), super::dictation_host::HostEmitError> {
        Ok(())
    }

    fn idle_emitter(&self) -> OwnedIdleEmit {
        OwnedIdleEmit::new(|| Ok(()))
    }

    fn transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>> {
        Box::pin(async move { Ok(state.app_state.recording_transition.lock().await) })
    }

    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        crate::frontmost::DeliveryTargetSnapshot::Incomplete
    }

    fn set_processing(&self, processing: bool) {
        keyboard::set_processing(processing);
    }

    fn capture_smart_destination<'a>(
        &'a self,
        _expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        _enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        let hook = self
            .at_capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        Box::pin(async move {
            if let Some(hook) = hook {
                hook();
            }
            crate::smart_destination::Snapshot::Disabled
        })
    }

    fn emit_smart_destination(
        &self,
        _recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) -> Result<(), String> {
        Ok(())
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        _samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        _performance_guard: &'a mut PerformanceRunGuard,
        _context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>> {
        Box::pin(async move {
            let mut guard = IdleGuard::for_pipeline(app_state, recording_id, stop.pipeline_op);
            guard.finish(|_| {});
            Ok(PipelineResult {
                text: String::new(),
                timings: PipelineTimings::default(),
                terminal: PipelineTerminal::Success,
                history: None,
            })
        })
    }
}

/// A WAV of 8,000 silent-ish 16 kHz samples, as the import command receives it.
pub(super) fn import_audio() -> String {
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut bytes, spec).expect("wav writer");
        for index in 0..8_000_usize {
            writer
                .write_sample(((index % 100) as i16) * 50)
                .expect("sample");
        }
        writer.finalize().expect("finalize");
    }
    base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        bytes.into_inner(),
    )
}

/// R301.5: an accepted legacy import allocates the next generation, runs its pipeline
/// and returns to `Idle`. Run to its end inside the barrier (it is synchronous there).
pub(super) fn allocate_by_import(world: &Arc<World>) {
    let audio = import_audio();
    let mut import: Fut<Result<serde_json::Value, String>> = Box::pin({
        let world = Arc::clone(world);
        async move {
            let state = world.state();
            commanded_reply(process_audio_inner(
                &ImportHost::new(),
                state.inner(),
                &audio,
            ))
            .await
        }
    });
    let reply = complete(&mut import);
    assert!(reply.is_ok(), "the import completes: {reply:?}");
    assert_eq!(
        world.state().app_state.recording_id.load(Ordering::SeqCst),
        2,
        "R301.5 allocated generation 2"
    );
}

/// R101.15: an ordinary microphone start allocates the next generation.
pub(super) fn allocate_by_start(world: &Arc<World>) {
    recording(world);
    assert_eq!(
        world.state().app_state.recording_id.load(Ordering::SeqCst),
        2,
        "R101.15 allocated generation 2"
    );
    assert_eq!(status(world), DictationStatus::Recording);
}

// ------------------------------------------------------------------ the fixture

/// The shared body. `allocate` is the allocator that runs at the barrier.
fn stale_scoped_stop_after_a_new_generation(world: &Arc<World>, allocate: fn(&Arc<World>)) {
    // The bridge keeps the task it schedules, and the `AutoStop` owner it carries, instead
    // of dropping them: the scoped stop below is that task, not a stop the fixture builds.
    world.knobs().carry_auto_stop_owner = true;
    recording(world);
    world.runtime_failure(1);
    // The worker's exit stores the capture and notifies `Interrupted`; the supervisor
    // is parked just before its owner clear.
    world.finish_interrupted(1, 9_000);
    let requests = world.auto_stop_requests();
    assert_eq!(requests, vec![1], "the bridge scheduled the scoped stop");
    // The scoped stop: the bridge's own dispatched task (production's `auto_stop_dispatch`,
    // `Some(1)` as its expected id, its owner moved in), delayed ahead of `T`, not polled.
    let (owner, scoped_stop) = world.owned_auto_stop(requests[0]);
    assert_eq!(owner.recording_id, RecordingId(1), "the owner's generation");
    assert_eq!(owner.kind, ContKind::AutoStop);
    assert!(
        world.state().app_state.core_history.is_outstanding(owner),
        "the owner is outstanding until the stop is admitted"
    );
    let scoped: ParkedStop = Rc::new(RefCell::new(Some(scoped_stop)));
    let scoped_reply: Reply = Rc::new(RefCell::new(None));
    world.knobs().stop_result = Ok(16_000);
    let barrier = {
        let (world, scoped, scoped_reply) = (
            Arc::clone(world),
            Rc::clone(&scoped),
            Rc::clone(&scoped_reply),
        );
        crate::barrier_hook::install(move |point| {
            if point != "pipeline_idle_emit" {
                return;
            }
            // The pipeline's guard has written Idle (R204a.1): the owner is Stopping's
            // and the stop is parked before R204b.
            assert_eq!(status(&world), DictationStatus::Idle);
            world.note("the unscoped stop is parked before R204b; the supervisor clears its owner");
            world.release_owner_clear();
            world.note("a new generation is allocated");
            allocate(&world);
            world.note("the delayed scoped stop for generation 1 resumes");
            let mut stop = scoped
                .borrow_mut()
                .take()
                .expect("the scoped stop is parked");
            let reply = complete(&mut stop);
            world.reply("scoped auto-stop(expected=1)", &reply);
            *scoped_reply.borrow_mut() = Some(reply);
        })
    };
    // The unscoped stop: takes the retained capture (R201.7) and runs the pipeline.
    let mut stop = world.stop(None);
    let reply = complete(&mut stop);
    world.reply("unscoped stop", &reply);
    drop(barrier);
    assert!(
        reply.is_ok(),
        "the original stop finishes its pipeline: {reply:?}"
    );
    // The scoped stop answered `recording_superseded` for the id it was scoped to.
    assert_eq!(
        scoped_reply.borrow_mut().take(),
        Some(Ok(
            serde_json::json!({ "state": "idle", "type": "recording_superseded" })
        )),
        "R201.1: the scoped stop is superseded by the new generation"
    );
    // PD-25 fixed: the original stop keeps generation 1's terminal through R204b.6, so the
    // terminal and the performance outcome of generation 1 both come from that stop. The
    // scoped stop found the capture already taken, so it claims nothing.
    assert_eq!(
        terminal(world, 1),
        Some(INTERRUPTED),
        "the original stop's R204b.6 won generation 1's terminal as RuntimeInterruption"
    );
    assert_eq!(
        perf_claim(1),
        Some(RunOutcomeV1::Success),
        "the original stop's R204b.6 completed generation 1's performance as Success"
    );
    let trace = world.trace();
    let claims: Vec<&String> = trace
        .iter()
        .filter(|line| line.starts_with("~ terminal 1 "))
        .collect();
    assert_eq!(
        claims,
        ["~ terminal 1 RuntimeInterruption/StreamInvalidated"],
        "exactly one terminal claim for generation 1, the original stop's:\n{}",
        trace.join("\n")
    );
    assert!(
        !trace
            .iter()
            .any(|line| line == "~ terminal 1 Superseded/StaleOwner"),
        "the scoped stop claimed no Superseded terminal:\n{}",
        trace.join("\n")
    );
}

pub(super) fn stale_scoped_stop_after_import(world: &Arc<World>) {
    stale_scoped_stop_after_a_new_generation(world, allocate_by_import);
}

pub(super) fn stale_scoped_stop_after_start(world: &Arc<World>) {
    stale_scoped_stop_after_a_new_generation(world, allocate_by_start);
}

/// PD-25 (fixed) through the legacy import (R301.5).
#[test]
pub(super) fn a_stale_scoped_stop_after_an_import_leaves_the_terminal_to_the_original_stop() {
    repeat(&sub(
        "PD-25 fixed: a scoped stop for generation 1 resumes after an import allocated generation 2 (R201.1 / R204b.6)",
        stale_scoped_stop_after_import,
    ));
}

/// PD-25 (fixed) through an ordinary microphone start (R101.15).
#[test]
pub(super) fn a_stale_scoped_stop_after_a_start_leaves_the_terminal_to_the_original_stop() {
    repeat(&sub(
        "PD-25 fixed: a scoped stop for generation 1 resumes after a start allocated generation 2 (R201.1 / R204b.6)",
        stale_scoped_stop_after_start,
    ));
}

// ------------------------------------- the earlier-take tombstone's other writes (#1186)
//
// The fixtures above cover the tombstone written by the stop that takes the capture at
// R201.5 / R201.7. These cover the other two take sites that write it: a stale stop's own
// take (R201.1) and the lifecycle's disposal (`TI(d)`, R216 here; R215.1 runs the same
// port). Each asserts the tombstone directly: a stale stop that resumes after either take
// claims no terminal because of it, and today the earlier taker has always claimed its
// terminal by then, so first-wins hides a missing write from the ledger alone.

fn tombstone(world: &Arc<World>, id: u64) -> bool {
    world.state().app_state.interrupted_taken_earlier(id)
}

fn superseded_reply() -> Option<Result<serde_json::Value, String>> {
    Some(Ok(
        serde_json::json!({ "state": "idle", "type": "recording_superseded" }),
    ))
}

/// The terminal lines the trace recorded for `id` (one per change of its terminal).
fn terminal_lines(world: &Arc<World>, id: u64) -> Vec<String> {
    let prefix = format!("~ terminal {id} ");
    world
        .trace()
        .into_iter()
        .filter(|line| line.starts_with(&prefix))
        .collect()
}

/// A stale stop for generation 1, run to its reply once generation 2 is current.
fn stale_stop_for_generation_1(world: &Arc<World>, label: &str) {
    let mut stop = world.stop(Some(1));
    let reply = complete(&mut stop);
    world.reply(label, &reply);
    assert_eq!(
        Some(reply),
        superseded_reply(),
        "R201.1: {label} is superseded by generation 2"
    );
}

/// Generation 1 has ended and generation 2 is recording, but the supervisor still holds
/// a 9,000-sample capture preserved for generation 1, and generation 1's terminal is
/// unclaimed (both planted: the fixture needs a stale stop to be the first taker). A stale
/// stop for generation 1 takes it at R201.1 and claims `Superseded`; a second stale stop
/// for generation 1 then finds the capture taken (the tombstone) and claims nothing.
fn stale_stop_takes_the_capture(world: &Arc<World>) {
    world.state().app_state.dictation_telemetry.accepted(1);
    allocate_by_start(world);
    audio_lifecycle::insert_interrupted_dictation_for_test(
        1,
        audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 9_000],
            reason: "stream_invalidated".to_string(),
            duration_ms: 9_000 / 16,
        },
    );
    assert!(
        !tombstone(world, 1),
        "nothing has taken generation 1's capture yet"
    );

    stale_stop_for_generation_1(world, "stale stop(expected=1), the taker");
    assert!(
        world
            .trace()
            .iter()
            .any(|line| line.starts_with("supervisor.take_interrupted 1 -> ")
                && !line.ends_with("none")),
        "the first stale stop took the capture at R201.1:\n{}",
        world.trace().join("\n")
    );
    assert!(
        tombstone(world, 1),
        "R201.1: the stale stop's take that found the capture is noted"
    );
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner
        )),
        "the stale stop that took the capture ends generation 1 as Superseded"
    );

    stale_stop_for_generation_1(world, "stale stop(expected=1), after the take");
    assert!(
        tombstone(world, 1),
        "a take that found nothing keeps the note"
    );
    assert_eq!(
        terminal_lines(world, 1),
        ["~ terminal 1 Superseded/StaleOwner"],
        "generation 1 has the one terminal, the taker's:\n{}",
        world.trace().join("\n")
    );
    assert_eq!(terminal(world, 2), None, "generation 2 is still recording");
}

/// Generation 1 is interrupted with 6,000 samples: too short to transcribe, so the
/// bridge's short-idle path (R215.3 / R216) disposes of the capture (`TI(d)`) and ends
/// generation 1 as `RuntimeInterruption`. Generation 2 starts; a stale scoped stop for
/// generation 1 then finds the capture taken by the disposal and claims nothing.
fn stale_stop_after_a_disposal(world: &Arc<World>) {
    recording(world);
    world.runtime_failure(1);
    world.finish_interrupted(1, 6_000);
    assert!(
        tombstone(world, 1),
        "R216 `TI(d)`: the disposal's take that found the capture is noted"
    );
    assert_eq!(
        terminal(world, 1),
        Some((
            DictationTerminalOutcome::RuntimeInterruption,
            DictationErrorCode::RuntimeFailure
        )),
        "the disposal ends generation 1"
    );
    world.release_owner_clear();
    allocate_by_start(world);

    stale_stop_for_generation_1(world, "stale stop(expected=1), after the disposal");
    assert!(
        world
            .trace()
            .iter()
            .any(|line| line == "supervisor.take_interrupted 1 -> none"),
        "the stale stop's take found nothing:\n{}",
        world.trace().join("\n")
    );
    assert!(tombstone(world, 1));
    assert_eq!(
        terminal_lines(world, 1),
        ["~ terminal 1 RuntimeInterruption/RuntimeFailure"],
        "generation 1 has the one terminal, the disposal's:\n{}",
        world.trace().join("\n")
    );
    assert_eq!(terminal(world, 2), None, "generation 2 is still recording");
}

/// #1186: a stale stop's own take at R201.1 writes the earlier-take tombstone.
#[test]
fn a_stale_stop_that_takes_the_capture_notes_the_take() {
    repeat(&sub_from(
        "PD-25 tombstone: a stale stop for generation 1 takes its capture (R201.1), a second finds it taken",
        DictationStatus::Idle,
        1,
        stale_stop_takes_the_capture,
    ));
}

/// #1186: the lifecycle's disposal of a short capture (R216 `TI(d)`) writes the tombstone.
#[test]
fn a_stale_scoped_stop_after_a_disposal_finds_the_capture_taken() {
    repeat(&sub(
        "PD-25 tombstone: a stale scoped stop for generation 1 resumes after R216 disposed of its capture",
        stale_stop_after_a_disposal,
    ));
}
