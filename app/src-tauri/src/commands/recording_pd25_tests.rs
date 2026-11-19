//! Shell fixtures for PD-25 (D-S2c-15 C4; the PD list in `recording_pd_witness_map.md`).
//!
//! PD-25 (issue #1157, confirmed real by the #1169 round-2 check): a scoped stop for
//! generation 1 is still queued ahead of the recording transition `T` while another
//! stop has already taken generation 1's retained capture and run its pipeline. The
//! pipeline's guard has written `Idle` (R204a.1) and the supervisor has cleared its
//! owner when a **new generation 2** is allocated, either by a legacy import (R301.5)
//! or by an ordinary microphone start (R101.15). The scoped stop then resumes:
//! R201.1 sees `expected = 1` against the current id 2, with the capture already
//! consumed, and wins generation 1's terminal as `Superseded/StaleOwner`; the original
//! stop resumes after it and R204b.6 completes generation 1's performance as `Success`.
//! One recording ends with a terminal and a performance outcome that disagree.
//!
//! The interleaving (one fixture per allocator, the same body):
//!
//! 1. Generation 1 records; a runtime failure puts it in `Recovering`; the worker's
//!    exit stores 9,000 samples and notifies `Interrupted`, so the bridge schedules the
//!    scoped stop `expected = Some(1)` (`audio_lifecycle.rs:2236,2404`,
//!    `recording.rs:6041`). The fixture builds that stop and **does not poll it**: it
//!    is delayed ahead of `T`.
//! 2. An unscoped stop takes the capture (R201.7) and runs the pipeline. It parks at
//!    the `pipeline_idle_emit` barrier, after the pipeline's guard wrote `Idle` and
//!    before R204b.
//! 3. At the barrier, in order: the supervisor clears its owner; generation 2 is
//!    allocated; the scoped stop is polled to its reply; the barrier returns and the
//!    parked stop runs R204b.
//!
//! Ordering is by causal barriers only: no clock, no sleep. A fix PR deletes the PD-25
//! row and these fixtures together (C7.0).
#![cfg(test)]

use super::dictation_host::HostFuture;
use super::golden_harness::*;
use super::golden_tests::{recording, sub};
use super::pd_witness_tests::repeat;
use super::process_audio_ports::{ImportedAudioHost, OwnedIdleEmit};
use super::*;
use std::cell::RefCell;
use std::rc::Rc;

const SUPERSEDED: (DictationTerminalOutcome, DictationErrorCode) = (
    DictationTerminalOutcome::Superseded,
    DictationErrorCode::StaleOwner,
);

fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
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

// ------------------------------------------------ the legacy import as an allocator

/// The legacy import's host (`process_audio_inner`'s own seam). It supplies what the
/// import needs from outside the shell, and the real pipeline `IdleGuard` its fake
/// pipeline takes, so the import allocates generation 2 through production's R301.5.
struct ImportHost;

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
        Box::pin(async { crate::smart_destination::Snapshot::Disabled })
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
fn import_audio() -> String {
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
fn allocate_by_import(world: &Arc<World>) {
    let audio = import_audio();
    let mut import: Fut<Result<serde_json::Value, String>> = Box::pin({
        let world = Arc::clone(world);
        async move {
            let state = world.state();
            commanded_reply(process_audio_inner(&ImportHost, state.inner(), &audio)).await
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
fn allocate_by_start(world: &Arc<World>) {
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
fn pd25_scoped_stop_after_a_new_generation(world: &Arc<World>, allocate: fn(&Arc<World>)) {
    recording(world);
    world.runtime_failure(1);
    // The worker's exit stores the capture and notifies `Interrupted`; the supervisor
    // is parked just before its owner clear.
    world.finish_interrupted(1, 9_000);
    let requests = world.auto_stop_requests();
    assert_eq!(requests, vec![1], "the bridge scheduled the scoped stop");
    // The scoped stop (`expected = Some(1)`): built, delayed ahead of `T`, not polled.
    let scoped: Rc<RefCell<Option<Fut<Result<serde_json::Value, String>>>>> =
        Rc::new(RefCell::new(Some(world.stop(Some(requests[0])))));
    let scoped_reply: Rc<RefCell<Option<Result<serde_json::Value, String>>>> =
        Rc::new(RefCell::new(None));
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
            world.reply("scoped stop(expected=1)", &reply);
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
    // PD-25: generation 1 ends with a terminal and a performance outcome that disagree.
    assert_eq!(
        terminal(world, 1),
        Some(SUPERSEDED),
        "the scoped stop won generation 1's terminal as Superseded/StaleOwner"
    );
    assert_eq!(
        perf_claim(1),
        Some(RunOutcomeV1::Success),
        "the original stop's R204b.6 completed generation 1's performance as Success"
    );
    // The scoped stop's own claim lost nothing it should keep: it took no capture.
    let trace = world.trace();
    assert!(
        trace
            .iter()
            .any(|line| line == "~ terminal 1 Superseded/StaleOwner"),
        "the terminal claim is in the trace:\n{}",
        trace.join("\n")
    );
}

fn pd25_import(world: &Arc<World>) {
    pd25_scoped_stop_after_a_new_generation(world, allocate_by_import);
}

fn pd25_start(world: &Arc<World>) {
    pd25_scoped_stop_after_a_new_generation(world, allocate_by_start);
}

/// PD-25 through the legacy import (R301.5).
#[test]
pub(super) fn pd25_import_allocates_while_a_scoped_stop_for_the_old_generation_is_queued() {
    repeat(&sub(
        "PD-25: a scoped stop for generation 1 resumes after an import allocated generation 2 (R201.1 / R204b.6)",
        pd25_import,
    ));
}

/// PD-25 through an ordinary microphone start (R101.15).
#[test]
pub(super) fn pd25_start_allocates_while_a_scoped_stop_for_the_old_generation_is_queued() {
    repeat(&sub(
        "PD-25: a scoped stop for generation 1 resumes after a start allocated generation 2 (R201.1 / R204b.6)",
        pd25_start,
    ));
}
