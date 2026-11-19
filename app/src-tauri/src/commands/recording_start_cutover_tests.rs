//! The start cutover (Stage 2c, appendix C9.7 and C8.6): the start family's D entries, and
//! the queries 401-404 that stayed on `project()` until now, decide on the **persisted core**
//! (M1 `status`, M2 `current_id`, `DictationState.core`), not on `DictationCore::project`.
//!
//! The entries (`FieldAdapter::start_view`; a label is the enclosing function and the call's
//! ordinal in it):
//!
//! | label | site | where |
//! |---|---|---|
//! | `start_native_recording_inner#0` | 101 | the request's D section (R101.3-.15) |
//! | `start_native_recording_inner#1` | 102 | the capture-failure section |
//! | `start_after_capture#0` | 402 | the short D read after the destination capture |
//! | `start_after_capture#1` | 103.0 | the destination's own D section (Smart Destination) |
//! | `start_after_capture#2` | 103 | the context install |
//! | `publish_recording_ready_with#0` | 104 | the ready publish (R104.2-.7) |
//! | `process_audio_inner#0` | 402 | the legacy import's capture check |
//!
//! The queries that only hold `&DictationState` (`FieldAdapter::start_query_view`, which counts
//! a divergence and cannot re-sync): `register_partial_ticker_locked#0` (401/404, R104.2's `SD`
//! and the lifecycle bridge's) and `owns_delivery#0` (403, the delivery fence). Site 104.1 (the
//! audio is not ready) is D-less and keeps its lock-free reads.
//!
//! What each test proves, and why the golden invariant alone is not enough:
//!
//! * `no_start_entry_or_query_projects` - no function in **start's scope** names the `project()`
//!   entry (`FieldAdapter::project_lifecycle_view`) or `DictationCore::project`, by source and,
//!   for every fixture, by the **call sites** it reached (`probe::take_projection_sites()`,
//!   filtered to start's scope: everything in `recording.rs` but the other families' own
//!   functions, the import's only inside its capture check), so a helper that projects for an
//!   entry or a query is caught too, and each fixture reads exactly the entries and queries it
//!   crosses, by name. The scope keeps the stop, cancel and lifecycle cutovers independently
//!   revertible: a family whose cutover is not applied still projects, and says so in its own
//!   tests. That no production code projects at all is the #1154 cleanup's check.
//! * `every_start_entry_reads_the_persisted_fields_and_resyncs_a_divergence` - the evidence is
//!   captured **at the entry**, from the actual persisted fields, before any adapter write could
//!   repair them: a core planted off live is read as planted, counted once (`raw_core_divergences`,
//!   C8.6), re-synced before the decision, and the run then reads exactly as an equal core does.
//! * `the_queries_follow_the_persisted_core` - queries 401 and 403 answer from the persisted
//!   fields, count a divergence, and never project.
//! * the R104.5 and speculative-capture tests - #1170's late publish and flowing capture, run
//!   again through the switched entries, with the entries that ran.
//! * `a_start_reply_names_the_generation_a_scoped_cancel_acts_on` - CP4 compares the reply kind
//!   only: the reply's generation is the persisted one, a cancel scoped to another generation is
//!   refused, and the cancel scoped to this one cancels exactly this attempt.
#![cfg(test)]

use super::dictation_host::HostFuture;
use super::golden_harness::*;
use super::process_audio_ports::{ImportedAudioHost, OwnedIdleEmit};
use super::*;
use crate::core_history::ContinuationGuard;
use crate::dictation_executor::adapter::{raw_core_divergences, start_forgive_planted_divergences};
use crate::dictation_executor::probe;
use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;
use std::cell::RefCell;
use std::sync::Mutex as StdMutex;

/// A `(status, generation)` pair, as an entry read it of the persisted core or of live.
type Pair = (Status, RecordingId);

/// `recording.rs`, read from the source tree.
fn recording_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/recording.rs");
    std::fs::read_to_string(path).expect("recording.rs")
}

/// Every free function and method of `recording.rs` (not those of its test modules), as
/// `(name, first line, last line)`.
fn fn_spans(source: &str) -> Vec<(String, usize, usize)> {
    use syn::spanned::Spanned;
    let file = syn::parse_file(source).expect("recording.rs parses");
    let mut spans = Vec::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(function) => spans.push((
                function.sig.ident.to_string(),
                function.span().start().line,
                function.span().end().line,
            )),
            syn::Item::Impl(implementation) => {
                for member in &implementation.items {
                    if let syn::ImplItem::Fn(function) = member {
                        spans.push((
                            function.sig.ident.to_string(),
                            function.span().start().line,
                            function.span().end().line,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

/// The lines of `source` (1-based) that hold `needle`, inside the function `name`.
fn lines_with(
    source: &str,
    spans: &[(String, usize, usize)],
    name: &str,
    needle: &str,
) -> Vec<usize> {
    let (_, first, last) = spans
        .iter()
        .find(|(candidate, _, _)| candidate == name)
        .unwrap_or_else(|| panic!("`{name}` is in recording.rs"));
    source
        .lines()
        .enumerate()
        .filter(|(at, line)| (*first..=*last).contains(&(at + 1)) && line.contains(needle))
        .map(|(at, _)| at + 1)
        .collect()
}

/// The terminal the telemetry ledger holds for `id`.
fn terminal(world: &Arc<World>, id: u64) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
    world
        .state()
        .app_state
        .dictation_telemetry
        .terminal_for_test(id)
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

/// The legacy import's host (`process_audio_inner`'s own seam): what the import needs from
/// outside the shell, and the real pipeline `IdleGuard` its fake pipeline takes.
struct ImportHost {
    /// Runs once, when the import reaches its destination capture: after its first D section
    /// (301) and before the capture check (402).
    at_capture: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl ImportHost {
    fn new() -> Self {
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

// ----------------------------------------------------------------------- entry labels

const E101: &str = "start_native_recording_inner#0";
const E102: &str = "start_native_recording_inner#1";
const E402: &str = "start_after_capture#0";
const E1030: &str = "start_after_capture#1";
const E103: &str = "start_after_capture#2";
const E104: &str = "publish_recording_ready_with#0";
const E402_IMPORT: &str = "process_audio_inner#0";
/// The ticker registration query (401/404): the shadow build's ports register through the
/// observed copy of the same gate.
#[cfg(not(feature = "dictation-shadow"))]
const Q401: &str = "register_partial_ticker_locked#0";
#[cfg(feature = "dictation-shadow")]
const Q401: &str = "register_partial_ticker_locked_observed#0";

// ------------------------------------------------------------------------ probes

/// Tests share the process-global probes; one start cutover test at a time. The entry and query
/// reads are remembered only while the guard lives (remembering allocates, which the allocation
/// pins under D must not see), and cleared when it starts and ends.
struct Probes(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

impl Drop for Probes {
    fn drop(&mut self) {
        probe::record_start_entries(false);
        probe::take_start_entries();
        probe::record_start_queries(false);
        probe::take_start_queries();
        probe::record_projections(false);
        probe::take_projection_sites();
        start_forgive_planted_divergences(0);
    }
}

fn serialized() -> Probes {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let gate = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe::take_start_entries();
    probe::take_start_queries();
    crate::commands::settings_store::test_data_dir::take_returned();
    probe::record_start_entries(true);
    probe::record_start_queries(true);
    probe::take_projection_sites();
    probe::record_projections(true);
    start_forgive_planted_divergences(0);
    Probes(gate)
}

fn enclosing(spans: &[(String, usize, usize)], line: usize) -> String {
    spans
        .iter()
        .filter(|(_, first, last)| (*first..=*last).contains(&line))
        .min_by_key(|(_, first, last)| last - first)
        .unwrap_or_else(|| panic!("line {line} is inside a function"))
        .0
        .clone()
}

/// The label of a call of `needle` at `line`: the enclosing function and the call's ordinal
/// inside it.
fn label(source: &str, spans: &[(String, usize, usize)], line: usize, needle: &str) -> String {
    let name = enclosing(spans, line);
    let calls = lines_with(source, spans, &name, needle);
    let ordinal = calls
        .iter()
        .position(|call| *call == line)
        .unwrap_or_else(|| panic!("line {line} is a `{needle}` call in `{name}`: {calls:?}"));
    format!("{name}#{ordinal}")
}

fn labelled(reads: Vec<probe::StartEntryRead>, needle: &str) -> Vec<(String, Pair, Pair)> {
    let source = recording_source();
    let spans = fn_spans(&source);
    reads
        .into_iter()
        .map(|((persisted, live), (file, line))| {
            assert!(
                file.ends_with("commands/recording.rs"),
                "a start read outside recording.rs: {file}:{line}"
            );
            (
                label(&source, &spans, line as usize, needle),
                persisted,
                live,
            )
        })
        .collect()
}

fn entries_read() -> Vec<(String, Pair, Pair)> {
    labelled(probe::take_start_entries(), "FieldAdapter::start_view(")
}

fn queries_read() -> Vec<(String, Pair, Pair)> {
    labelled(
        probe::take_start_queries(),
        "FieldAdapter::start_query_view(",
    )
}

// ------------------------------------------------------------------------ the plants

/// Where a test plants a divergence (the persisted core put off live), if it does.
#[derive(Clone, Copy)]
enum Plant {
    Nowhere,
    /// Right before the request's own D section (the `start.request` barrier, D free): the
    /// request's entry is the first read, with no trace line between.
    Now,
    /// The next time production reaches a `barrier_hook` point (D is free there), right before
    /// the D section whose entry reads the plant: no trace line is pushed between the two,
    /// which a plant at a port could not promise (the port's own lines follow its hook).
    AtBarrier(&'static str),
    /// The next time a port reaches the label (D is free at every label used).
    AtPort(&'static str),
}

thread_local! {
    static PLAN: RefCell<Plant> = const { RefCell::new(Plant::Nowhere) };
}

/// The `(planted, live)` pair the plant wrote, whichever thread ran it.
static PLANTED: StdMutex<Option<(Pair, Pair)>> = StdMutex::new(None);

/// A **planted** C8.6 divergence, start's own: put both persisted fields off live, as a write
/// that bypassed the adapter would, and allow the run's golden invariant exactly one counted
/// divergence (`start_forgive_planted_divergences`, the start cutover's own allowance). The
/// entry that next reads the persisted core must count it and re-sync it; the tests assert that
/// it did. The persisted fields are checked equal to live first, so a plant never forgives an
/// earlier violation. No trace line may be pushed between the plant and that entry (the
/// invariant samples the persisted core at every line). Returns the planted pair and live's.
fn plant_core_divergence(world: &Arc<World>) -> (Pair, Pair) {
    let state = world.state();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    let live = (
        crate::dictation_admission::core_status(dictation.status),
        RecordingId(state.app_state.recording_id.load(Ordering::SeqCst)),
    );
    assert_eq!(
        (dictation.core.status(), dictation.core.current_id()),
        live,
        "the persisted core equalled live before it was planted off"
    );
    let other = if live.0 == Status::Recording {
        Status::Processing
    } else {
        Status::Recording
    };
    dictation.core.set_status(other);
    let id = dictation.core.alloc_id();
    start_forgive_planted_divergences(1);
    ((other, id), live)
}

fn planted_now(world: &Arc<World>) {
    let pair = plant_core_divergence(world);
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pair);
}

/// Plants as the run asks for, called by a body right before its command. The returned guard (a
/// barrier hook) must live to the end of the command.
fn arm(world: &Arc<World>) -> Option<crate::barrier_hook::Installed> {
    match PLAN.with(|plan| *plan.borrow()) {
        Plant::Nowhere => None,
        Plant::Now => {
            let planted = Arc::clone(world);
            let mut done = false;
            Some(crate::barrier_hook::install(move |reached| {
                if reached == "start.request" && !done {
                    done = true;
                    planted_now(&planted);
                }
            }))
        }
        Plant::AtBarrier(point) => {
            let planted = Arc::clone(world);
            let mut done = false;
            Some(crate::barrier_hook::install(move |reached| {
                if reached == point && !done {
                    done = true;
                    planted_now(&planted);
                }
            }))
        }
        Plant::AtPort(port) => {
            let world = Arc::clone(world);
            let planted = Arc::clone(&world);
            world.on(port, move || planted_now(&planted));
            None
        }
    }
}

// ------------------------------------------------------------------------ the fixtures

struct Case {
    name: &'static str,
    setup: fn() -> Setup,
    body: fn(&Arc<World>),
    /// The start entries the run reads, in order.
    entries: &'static [&'static str],
    /// The queries (`start_query_view`) the run reads, in order.
    queries: &'static [&'static str],
    /// Where a divergence is planted, and which of `entries` reads it first.
    plant: Option<(Plant, usize)>,
}

fn idle() -> Setup {
    Setup::idle()
}

fn in_status(status: DictationStatus, recording_id: u64) -> Setup {
    Setup {
        status,
        recording_id,
    }
}

fn enable_smart_destination(world: &Arc<World>) {
    let state = world.state();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.smart_destination_enabled = true;
    dictation.auto_paste = true;
    dictation.save_audio = false;
    dictation.save_transcript = false;
}

/// A start, announced by a marker line.
fn start_command(world: &Arc<World>) {
    // The marker first: a trace line samples the persisted-core invariant, so the plant is made
    // right before the command and the entry that reads it, with no line in between.
    world.note("start requested");
    let _plant = arm(world);
    let mut start = world.start();
    let reply = complete(&mut start);
    world.reply("start", &reply);
}

/// R101.15 + R103.1 + R104.2: a start that publishes `recording`.
fn published(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyBeforeContext;
    start_command(world);
    assert_eq!(world.side_task_spawns(), 1, "the side tasks started once");
}

/// The same with Smart Destination on (R103.0).
fn published_with_destination(world: &Arc<World>) {
    enable_smart_destination(world);
    published(world);
}

/// R101.15 with `Ready` after the context: the start's own publish finds the audio not ready
/// (R104.1, D-less) and the lifecycle bridge's `Ready` publishes.
fn ready_after_the_context(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    start_command(world);
    world.release_ready();
}

/// R102: the supervisor refuses the capture start.
fn capture_failure(world: &Arc<World>) {
    world.knobs().start_ack = Some(AudioStartError::SpawnFailed(
        "scripted start failure".to_string(),
    ));
    start_command(world);
}

/// R101: a plain start, whatever the setup makes of it.
fn plain(world: &Arc<World>) {
    start_command(world);
}

fn meeting_active(world: &Arc<World>) {
    world
        .state()
        .app_state
        .meeting_active
        .store(true, Ordering::SeqCst);
    start_command(world);
}

fn file_transcription_busy(world: &Arc<World>) {
    world
        .state()
        .app_state
        .file_transcribing
        .store(true, Ordering::SeqCst);
    start_command(world);
}

fn app_disabled(world: &Arc<World>) {
    world.knobs().app_disabled = true;
    start_command(world);
}

fn fixed_device_with_smart_auto(world: &Arc<World>) {
    world.note("start requested");
    let _plant = arm(world);
    let mut start = world.start_smart_auto(Some("fixed-device".to_string()));
    let reply = complete(&mut start);
    world.reply("start", &reply);
}

/// R103.2: the runtime fails between the capture check and the context install, so the attempt
/// is `Recovering` at site 103.
fn recovering_at_the_context(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    let failing = Arc::clone(world);
    world.on("refresh_expired_ide_context", move || {
        failing.runtime_failure(1)
    });
    start_command(world);
    world.release_ready();
}

/// R103.3: the attempt ended (`Idle`) before the context was installed.
fn idle_at_the_context(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    let ended = Arc::clone(world);
    world.on("refresh_expired_ide_context", move || {
        ended.notify(1, AudioLifecycleEvent::Idle)
    });
    start_command(world);
    world.release_ready();
}

/// R104.5: the runtime fails between the audio check and the publish's D section. The plant,
/// when the run asks for one at that port, follows the failure in the same hook (a port runs
/// one hook per visit).
fn late_publish(world: &Arc<World>) {
    world.knobs().cold_start = ColdStart::ReadyBeforeContext;
    let failing = Arc::clone(world);
    let plant = matches!(
        PLAN.with(|plan| *plan.borrow()),
        Plant::AtPort("supervisor.is_dictation_recording")
    );
    world.on("supervisor.is_dictation_recording", move || {
        failing.runtime_failure(1);
        if plant {
            planted_now(&failing);
        }
    });
    PLAN.with(|plan| {
        if plant {
            *plan.borrow_mut() = Plant::Nowhere;
        }
    });
    start_command(world);
}

/// The import command, run to its reply on `host`.
fn run_import(world: &Arc<World>, host: ImportHost) {
    let reply = {
        let mut run: Fut<Result<serde_json::Value, String>> = Box::pin({
            let world = Arc::clone(world);
            let audio = import_audio();
            async move {
                let state = world.state();
                commanded_reply(process_audio_inner(&host, state.inner(), &audio)).await
            }
        });
        complete(&mut run)
    };
    world.reply("import", &reply);
    assert!(reply.is_ok(), "{reply:?}");
}

/// The legacy import's destination capture check (402 in `process_audio_inner`).
fn import(world: &Arc<World>) {
    let host = ImportHost::new();
    if let Plant::AtPort("import.capture") = PLAN.with(|plan| *plan.borrow()) {
        let planted = Arc::clone(world);
        *host
            .at_capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Box::new(move || planted_now(&planted)));
    }
    run_import(world, host);
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "R101.15 + R103.1 + R104.2, planted at the request (101)",
            setup: idle,
            body: published,
            entries: &[E101, E402, E103, E104],
            queries: &[Q401],
            plant: Some((Plant::Now, 0)),
        },
        Case {
            // `Ready` arrives later here: with it inside the supervisor's start call, the
            // lifecycle bridge's entry would be the first to read a plant made at that port.
            name: "R101.15 + R103.1, planted at the capture check (402)",
            setup: idle,
            body: ready_after_the_context,
            entries: &[E101, E402, E103],
            queries: &[Q401],
            plant: Some((Plant::AtBarrier("start.capture"), 1)),
        },
        Case {
            name: "R101.15 + R103.1 + R104.2, planted at the context install (103)",
            setup: idle,
            body: published,
            entries: &[E101, E402, E103, E104],
            queries: &[Q401],
            plant: Some((Plant::AtPort("refresh_expired_ide_context"), 2)),
        },
        Case {
            name: "R101.15 + R103.1 + R104.2, planted at the ready publish (104)",
            setup: idle,
            body: published,
            entries: &[E101, E402, E103, E104],
            queries: &[Q401],
            plant: Some((Plant::AtPort("supervisor.is_dictation_recording"), 3)),
        },
        Case {
            name: "R103.0 the destination section, planted before it",
            setup: idle,
            body: published_with_destination,
            entries: &[E101, E402, E1030, E103, E104],
            queries: &[Q401],
            plant: Some((Plant::AtPort("refresh_expired_ide_context"), 2)),
        },
        Case {
            name: "R104.1 the audio is not ready, then the bridge's Ready",
            setup: idle,
            body: ready_after_the_context,
            entries: &[E101, E402, E103],
            queries: &[Q401],
            plant: None,
        },
        Case {
            name: "R104.5 a late publish after a runtime failure, planted at the publish",
            setup: idle,
            body: late_publish,
            entries: &[E101, E402, E103, E104],
            queries: &[],
            plant: Some((Plant::AtPort("supervisor.is_dictation_recording"), 3)),
        },
        Case {
            name: "R102 the capture start fails, planted at the failure section",
            setup: idle,
            body: capture_failure,
            entries: &[E101, E102],
            queries: &[],
            plant: Some((Plant::AtBarrier("start.failure"), 1)),
        },
        Case {
            name: "R103.2 recovering at the context install",
            setup: idle,
            body: recovering_at_the_context,
            entries: &[E101, E402, E103],
            queries: &[Q401],
            plant: None,
        },
        Case {
            name: "R103.3 idle at the context install",
            setup: idle,
            body: idle_at_the_context,
            entries: &[E101, E402, E103],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.3-.4 a meeting is active",
            setup: idle,
            body: meeting_active,
            entries: &[E101],
            queries: &[],
            plant: Some((Plant::Now, 0)),
        },
        Case {
            name: "R101.5 a file transcription is running",
            setup: idle,
            body: file_transcription_busy,
            entries: &[E101],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.8 starting",
            setup: || in_status(DictationStatus::Starting, 2),
            body: plain,
            entries: &[E101],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.9 recording",
            setup: || in_status(DictationStatus::Recording, 2),
            body: plain,
            entries: &[E101],
            queries: &[],
            plant: Some((Plant::Now, 0)),
        },
        Case {
            name: "R101.10 recovering",
            setup: || in_status(DictationStatus::Recovering, 2),
            body: plain,
            entries: &[E101],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.11 processing",
            setup: || in_status(DictationStatus::Processing, 3),
            body: plain,
            entries: &[E101],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.1 the app is disabled (before D)",
            setup: idle,
            body: app_disabled,
            entries: &[],
            queries: &[],
            plant: None,
        },
        Case {
            name: "R101.2 a fixed device with Smart Auto (before D)",
            setup: idle,
            body: fixed_device_with_smart_auto,
            entries: &[],
            queries: &[],
            plant: None,
        },
        Case {
            name: "the legacy import's capture check (402), planted at the capture",
            setup: idle,
            body: import,
            entries: &[E402_IMPORT],
            queries: &[],
            plant: Some((Plant::AtPort("import.capture"), 0)),
        },
    ]
}

/// What a run read: each start entry as `(label, persisted, live)`, oldest first, the queries
/// likewise, the start-owned `DictationCore::project` call sites the run reached (none), the
/// divergences counted, and the trace.
struct Run {
    entries: Vec<(String, Pair, Pair)>,
    queries: Vec<(String, Pair, Pair)>,
    projections: Vec<String>,
    divergences: u64,
    trace: Vec<String>,
}

fn run_case(case: &Case, plant: Plant) -> Run {
    PLAN.with(|slot| *slot.borrow_mut() = plant);
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    probe::take_start_entries();
    probe::take_start_queries();
    crate::commands::settings_store::test_data_dir::take_returned();
    // Every `project()` call site start owns that the sub-run reached (its setup, its body, the
    // ports' hooks and the bridge's detached work), not a count a body keeps for itself: a body
    // that does not keep one (the import's) or a read outside the command's own window (the
    // bridge's 404 in `release_ready()`) is inside it too.
    probe::take_projection_sites();
    // A plant of an earlier run allowed one divergence in that run only.
    start_forgive_planted_divergences(0);
    let divergences = raw_core_divergences();
    let trace = run_sub_run(&SubRun {
        title: case.name,
        setup: (case.setup)(),
        body: case.body,
    });
    let projections = start_projection_sites();
    PLAN.with(|slot| *slot.borrow_mut() = Plant::Nowhere);
    assert_no_data_dir(case.name);
    Run {
        entries: entries_read(),
        queries: queries_read(),
        projections,
        divergences: raw_core_divergences() - divergences,
        trace,
    }
}

/// The start fixtures read no store and no data directory (the harness's pipeline, ticker and
/// VAD are stand-ins): `data_dir` returned nothing since the last drain, sandboxed or real. A
/// fixture that needed one would take `vad_stream::StoreOverride` and a sandbox first.
fn assert_no_data_dir(name: &str) {
    assert_eq!(
        crate::commands::settings_store::test_data_dir::take_returned(),
        Vec::<std::path::PathBuf>::new(),
        "{name}: a start fixture resolved the data directory"
    );
}

/// The functions of `recording.rs` that belong to the other families' entries. Each still
/// projects while its own cutover is not applied (stop's, cancel's and the lifecycle's, so
/// reverting one of them alone leaves this module passing), and the start cutover's tests say
/// nothing about them: theirs do. Everything else in `recording.rs` is start's scope, helpers
/// included, so a helper that projects for a start entry or query is caught. The legacy import's
/// function is shared: its own entries are the other families', and only its capture check (402)
/// is start's (see [`capture_check_lines`]).
const OTHER_FAMILIES: [&str; 6] = [
    "finish_section",
    "run_pipeline_idle_emit",
    "handle_audio_lifecycle_events",
    "stop_native_recording_admitted",
    "run_stop_capture_rows",
    "cancel_native_recording_inner",
];

/// The lines of the import's capture check, query 402 (the block `let capture_is_current = {`
/// to its `};`), the one place `process_audio_inner` is start's.
fn capture_check_lines(source: &str) -> std::ops::RangeInclusive<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let first = lines
        .iter()
        .position(|line| line.trim() == "let capture_is_current = {")
        .expect("the import's capture check")
        + 1;
    let last = lines[first..]
        .iter()
        .position(|line| line.trim() == "};")
        .expect("the capture check's end")
        + first;
    first..=last
}

/// Whether the `project()` call at `file:line` is **start's**: in `recording.rs`, a function not
/// listed in [`OTHER_FAMILIES`] (the import's only inside its capture check), or in any other
/// file (start's ports and the bridge's 404 closure; no other family projects there).
fn is_start_site(
    spans: &[(String, usize, usize)],
    capture: &std::ops::RangeInclusive<usize>,
    file: &str,
    line: usize,
) -> bool {
    if !file.ends_with("commands/recording.rs") {
        return true;
    }
    let function = enclosing(spans, line);
    if function == "process_audio_inner" {
        return capture.contains(&line);
    }
    !OTHER_FAMILIES.contains(&function.as_str())
}

/// The `file:line` of every start-owned `DictationCore::project` entry since the last
/// `take_projection_sites`, in order. Entries other families still make (when their cutover is
/// not applied) are not start's and not counted.
fn start_projection_sites() -> Vec<String> {
    let source = recording_source();
    let spans = fn_spans(&source);
    let capture = capture_check_lines(&source);
    probe::take_projection_sites()
        .into_iter()
        .filter(|(file, line)| is_start_site(&spans, &capture, file, *line as usize))
        .map(|(file, line)| format!("{file}:{line}"))
        .collect()
}

fn names(reads: &[(String, Pair, Pair)]) -> Vec<&str> {
    reads.iter().map(|(name, _, _)| name.as_str()).collect()
}

// ------------------------------------------------------------------------ the tests

#[test]
fn no_start_entry_or_query_projects() {
    let _gate = serialized();
    let source = recording_source();
    let spans = fn_spans(&source);

    // By source: no function of `recording.rs` that is start's scope (see `OTHER_FAMILIES`: the
    // shell, its helpers, its entries; the import's capture check) names the `project()` entry,
    // and the persisted entries are where the table above says. (That no production code
    // projects anywhere is the #1154 cleanup's check, not this cutover's.)
    let capture = capture_check_lines(&source);
    for (function, first, last) in &spans {
        if OTHER_FAMILIES.contains(&function.as_str()) {
            continue;
        }
        for needle in ["project_lifecycle_view", "DictationCore::project"] {
            let hits: Vec<usize> = lines_with(&source, &spans, function, needle)
                .into_iter()
                .filter(|line| function != "process_audio_inner" || capture.contains(line))
                .collect();
            assert!(
                hits.is_empty(),
                "`{function}` ({first}-{last}) names `{needle}` at {hits:?}"
            );
        }
    }
    for (function, views, queries) in [
        ("start_native_recording_inner", 2, 0),
        ("start_after_capture", 3, 0),
        ("publish_recording_ready_with", 1, 0),
        ("process_audio_inner", 1, 0),
        ("register_partial_ticker_locked", 0, 1),
        ("register_partial_ticker_locked_observed", 0, 1),
        ("owns_delivery", 1, 0),
    ] {
        assert_eq!(
            lines_with(&source, &spans, function, "FieldAdapter::start_view(").len(),
            views,
            "`{function}` reads the persisted core once per D entry"
        );
        assert_eq!(
            lines_with(&source, &spans, function, "FieldAdapter::start_query_view(").len(),
            queries,
            "`{function}` reads the persisted core once per query"
        );
    }
    // The start ports and the bridge's 404 closure hold no `project()` entry either.
    for file in [
        "src/commands/recording/start_ports.rs",
        "src/commands/recording/ready_ports.rs",
        "src/commands/recording/lifecycle_ports.rs",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{file}"));
        assert!(
            !text.contains("project_lifecycle_view") && !text.contains("DictationCore::project("),
            "{file} reaches the `project()` entry"
        );
    }

    // At run time: every fixture reads exactly the entries and queries it crosses, by name,
    // and reaches `project()` not at all. A count, not a line span: a helper outside the
    // functions that projects for an entry or a query is caught.
    for case in cases() {
        let run = run_case(&case, Plant::Nowhere);
        assert_eq!(
            names(&run.entries),
            case.entries,
            "{}: the entries it read (by name)",
            case.name
        );
        assert_eq!(
            names(&run.queries),
            case.queries,
            "{}: the queries it read (by name)",
            case.name
        );
        assert_eq!(
            run.projections,
            Vec::<String>::new(),
            "{}: the start command reached `DictationCore::project`",
            case.name
        );
        assert_eq!(
            run.divergences, 0,
            "{}: an equal core is never counted",
            case.name
        );
        for (name, persisted, live) in run.entries.iter().chain(&run.queries) {
            assert_eq!(
                persisted, live,
                "{}: {name} read persisted != live on an equal core",
                case.name
            );
        }
        // The request's entry reads the pair the fixture started from, generation included. (The
        // import's capture check runs after its own request allocated generation 1.)
        if let Some((name, persisted, _)) = run.entries.first() {
            if name == E101 {
                let setup = (case.setup)();
                assert_eq!(
                    *persisted,
                    (
                        crate::dictation_admission::core_status(setup.status),
                        RecordingId(setup.recording_id)
                    ),
                    "{}: {name} read the setup's pair",
                    case.name
                );
            } else {
                assert_eq!(name, E402_IMPORT, "{}", case.name);
                assert_eq!(
                    *persisted,
                    (Status::Processing, RecordingId(1)),
                    "{}: the import's capture check read generation 1 in `Processing`",
                    case.name
                );
            }
        }
    }
}

#[test]
fn every_start_entry_reads_the_persisted_fields_and_resyncs_a_divergence() {
    let _gate = serialized();
    let mut covered = std::collections::BTreeSet::new();
    for case in cases() {
        let Some((plan, at)) = case.plant else {
            continue;
        };
        // The reference: the same fixture on an equal core.
        let reference = run_case(&case, Plant::Nowhere);
        assert_eq!(reference.divergences, 0, "{}", case.name);
        let planted = run_case(&case, plan);
        let (pair_planted, pair_live) = PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| panic!("{}: the plant ran", case.name));
        assert_ne!(pair_planted, pair_live, "{}: the plant diverges", case.name);

        assert_eq!(names(&planted.entries), case.entries, "{}", case.name);
        // Captured at the entry, from the actual persisted fields: the planted core and live
        // as the fixture left it, before anything repaired it.
        let (name, persisted, live) = &planted.entries[at];
        assert_eq!(
            *persisted, pair_planted,
            "{}: {name} read the persisted fields as planted, before any repair",
            case.name
        );
        assert_eq!(
            *live, pair_live,
            "{}: {name} read live as the fixture left it",
            case.name
        );
        // Every read before it saw an equal core; every read after it, the re-synced one.
        for (earlier, persisted, live) in planted.entries[..at].iter() {
            assert_eq!(
                persisted, live,
                "{}: {earlier} read before the plant",
                case.name
            );
        }
        // Counted once (C8.6) ...
        assert_eq!(
            planted.divergences, 1,
            "{}: the divergence was counted exactly once",
            case.name
        );
        // ... and re-synced before the decision: the run is the one an equal core gives.
        assert_eq!(
            planted.trace, reference.trace,
            "{}: the divergence was re-synced before the decision, not decided on",
            case.name
        );
        assert_eq!(planted.projections, Vec::<String>::new(), "{}", case.name);
        covered.insert(name.clone());
    }
    // Every start entry of the table was planted at least once, by name.
    for entry in [E101, E102, E402, E1030, E103, E104, E402_IMPORT] {
        assert!(
            covered.contains(entry),
            "no case planted {entry}: {covered:?}"
        );
    }
}

/// The persisted core put off live, in M1 only, or in M1 and M2: live untouched.
fn diverge(app_state: &AppState, status: Status, id_too: bool) -> (Pair, Pair) {
    use crate::dictation_executor::FieldAdapter;
    let mut dictation = app_state.dictation.lock_or_recover();
    let live = (
        crate::dictation_admission::core_status(dictation.status),
        RecordingId(app_state.recording_id.load(Ordering::SeqCst)),
    );
    FieldAdapter::seed_core(app_state, &mut dictation);
    dictation.core.set_status(status);
    if id_too {
        dictation.core.alloc_id();
    }
    ((dictation.core.status(), dictation.core.current_id()), live)
}

fn core_pair(app_state: &AppState) -> Pair {
    let dictation = app_state.dictation.lock_or_recover();
    (dictation.core.status(), dictation.core.current_id())
}

/// A state whose live fields say `status` for generation 5, and an equal core.
fn live_state(status: DictationStatus) -> AppState {
    use crate::dictation_executor::FieldAdapter;
    let app_state = AppState::default();
    app_state.recording_id.store(5, Ordering::SeqCst);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = status;
        FieldAdapter::seed_core(&app_state, &mut dictation);
    }
    app_state
}

/// Query 403 (the delivery fence) holds a guard of its own, so it reads like an entry
/// (`start_view`): a core planted off live is **counted once and re-synced**, and the fence
/// decides on what live says, as it did before the cutover, never on the stale persisted pair
/// (which would drop a transcript whose recording is still `Processing`). Queries 401/404 hold
/// only `&DictationState`: they count, answer from the **live** M1/M2 and leave the re-sync to
/// the next entry, which counts the same divergence again (twice in all, once per read, by
/// design). Nothing here projects.
#[test]
fn the_queries_follow_the_persisted_core() {
    let _gate = serialized();
    crate::commands::dictation_preview::reset_ownership_for_test();

    // 403 on an equal core: the plain answers, two entry-style reads, nothing counted.
    let app_state = live_state(DictationStatus::Processing);
    {
        let before = raw_core_divergences();
        let mut dictation = app_state.dictation.lock_or_recover();
        assert!(owns_delivery(&app_state, &mut dictation, 5), "equal core");
        assert!(
            !owns_delivery(&app_state, &mut dictation, 6),
            "id 6 is not current"
        );
        assert_eq!(
            raw_core_divergences(),
            before,
            "an equal core is never counted"
        );
    }
    let reads = entries_read();
    assert_eq!(names(&reads), ["owns_delivery#0"; 2]);
    assert!(reads.iter().all(|(_, persisted, live)| persisted == live));
    assert!(queries_read().is_empty(), "403 is not a read-only query");

    // 403 on a stale persisted `Recording`: live says `Processing` for generation 5, which
    // owns the delivery. The fence delivers, as before the cutover; it is counted once, and the
    // core is re-synced.
    let planted = diverge(&app_state, Status::Recording, false);
    assert_eq!(
        planted,
        (
            (Status::Recording, RecordingId(5)),
            (Status::Processing, RecordingId(5))
        )
    );
    {
        let before = raw_core_divergences();
        let mut dictation = app_state.dictation.lock_or_recover();
        assert!(
            owns_delivery(&app_state, &mut dictation, 5),
            "decided on live `Processing`, not on the stale persisted `Recording`"
        );
        assert_eq!(raw_core_divergences() - before, 1, "counted once");
        assert_eq!(
            (dictation.core.status(), dictation.core.current_id()),
            planted.1,
            "re-synced to live"
        );
    }
    let reads = entries_read();
    assert_eq!(names(&reads), ["owns_delivery#0"]);
    assert_eq!(reads[0].1, planted.0, "read the persisted pair as planted");
    assert_eq!(reads[0].2, planted.1, "read live as the fixture left it");

    // ... and the other way round: live `Recording`, a stale persisted `Processing`. Live is
    // not `Processing`, so the fence refuses, as before.
    let app_state = live_state(DictationStatus::Recording);
    let planted = diverge(&app_state, Status::Processing, false);
    {
        let before = raw_core_divergences();
        let mut dictation = app_state.dictation.lock_or_recover();
        assert!(!owns_delivery(&app_state, &mut dictation, 5));
        assert_eq!(raw_core_divergences() - before, 1, "counted once");
        assert_eq!(
            (dictation.core.status(), dictation.core.current_id()),
            planted.1
        );
    }
    entries_read();

    // A stale generation in the persisted core (M2) is the same.
    let app_state = live_state(DictationStatus::Processing);
    let planted = diverge(&app_state, Status::Processing, true);
    assert_eq!(planted.0, (Status::Processing, RecordingId(6)));
    {
        let before = raw_core_divergences();
        let mut dictation = app_state.dictation.lock_or_recover();
        assert!(owns_delivery(&app_state, &mut dictation, 5));
        assert_eq!(raw_core_divergences() - before, 1, "counted once");
        assert_eq!(dictation.core.current_id(), RecordingId(5), "M2 re-synced");
    }
    entries_read();

    // The delivery fences go through it: a stale core does not stop the operation a current
    // delivery owner runs, and is counted once per fence.
    let app_state = live_state(DictationStatus::Processing);
    for timed in [None, Some(false), Some(true)] {
        diverge(&app_state, Status::Recording, false);
        let before = raw_core_divergences();
        let ran = match timed {
            None => with_current_delivery_owner(&app_state, 5, || 7),
            Some(preparing) => with_current_delivery_owner_timed(&app_state, 5, preparing, || 7),
        };
        assert_eq!(ran, Some(7), "the fence delivered ({timed:?})");
        assert_eq!(
            raw_core_divergences() - before,
            1,
            "counted once ({timed:?})"
        );
        assert_eq!(names(&entries_read()), ["owns_delivery#0"]);
        assert_eq!(core_pair(&app_state), (Status::Processing, RecordingId(5)));
    }

    // 401/404: only a shared reference. Live `Recording` for generation 5 (the ticker is
    // registered, as before the cutover), a stale persisted `Processing`: counted, answered
    // from live, the stored core left alone.
    let app_state = live_state(DictationStatus::Recording);
    let planted = diverge(&app_state, Status::Processing, false);
    let before = raw_core_divergences();
    {
        let dictation = app_state.dictation.lock_or_recover();
        assert!(
            register_partial_ticker_locked(&app_state, &dictation, 5),
            "decided on live `Recording`, not on the stale persisted `Processing`"
        );
    }
    assert_eq!(raw_core_divergences() - before, 1, "counted once");
    let reads = queries_read();
    assert_eq!(names(&reads), ["register_partial_ticker_locked#0"]);
    assert_eq!(reads[0].1, planted.0);
    assert_eq!(reads[0].2, planted.1);
    assert_eq!(
        core_pair(&app_state),
        planted.0,
        "a shared reference cannot re-sync: the stored core is left for the next entry"
    );
    // The double count: the next entry reads the same divergence, counts it again and re-syncs
    // it. Once per read, by design.
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        let _ = owns_delivery(&app_state, &mut dictation, 5);
    }
    assert_eq!(
        raw_core_divergences() - before,
        2,
        "the query counted it, and the next entry counted it again"
    );
    assert_eq!(core_pair(&app_state), planted.1, "the entry re-synced");
    entries_read();
    // An equal core is counted by nobody.
    {
        let dictation = app_state.dictation.lock_or_recover();
        let _ = register_partial_ticker_locked(&app_state, &dictation, 5);
    }
    assert_eq!(
        raw_core_divergences() - before,
        2,
        "an equal core is never counted"
    );
    queries_read();

    // The stale generation (M2) answers from live too, and a refused ticker stays refused.
    let app_state = live_state(DictationStatus::Recording);
    diverge(&app_state, Status::Recording, true);
    {
        let dictation = app_state.dictation.lock_or_recover();
        assert!(!register_partial_ticker_locked(&app_state, &dictation, 4));
    }
    queries_read();

    assert_eq!(
        start_projection_sites(),
        Vec::<String>::new(),
        "a query projected"
    );
    crate::commands::dictation_preview::reset_ownership_for_test();
}

// ------------------------------- R104.5 and the speculative capture, through the switched entries

/// Every read `entries` holds, in order: its label, and the persisted and the live pair it read
/// (equal here: an equal core), generation included.
fn assert_reads(entries: Vec<(String, Pair, Pair)>, expected: &[(&str, Status, u64)], what: &str) {
    let got: Vec<(String, Pair, Pair)> = entries;
    assert_eq!(
        got.len(),
        expected.len(),
        "{what}: the switched entries that ran: {:?}",
        names(&got)
    );
    for ((name, persisted, live), (want, status, generation)) in got.iter().zip(expected) {
        assert_eq!(name, want, "{what}: the entry that ran");
        let pair = (*status, RecordingId(*generation));
        assert_eq!(*persisted, pair, "{what}: {name} read the persisted pair");
        assert_eq!(*live, pair, "{what}: {name} read the live pair");
    }
}

/// #1170's late-publish test (R104.5: the runtime fails between the audio check and the
/// publish's D section, so the publish reads `Recovering` and publishes nothing), run again
/// through the switched entry. The entries that ran are exact, with the pair each read, and the
/// publish's own entry read the persisted `Recovering` of generation 1, which is what R104.5
/// decided on.
#[test]
fn the_r104_5_late_publish_holds_through_the_switched_entry() {
    let _gate = serialized();
    let divergences = raw_core_divergences();
    super::start_tests::a_late_ready_publish_after_a_runtime_failure_publishes_nothing();
    assert_reads(
        entries_read(),
        &[
            (E101, Status::Idle, 0),
            (E402, Status::Starting, 1),
            (E103, Status::Starting, 1),
            (E104, Status::Recovering, 1),
        ],
        "R104.5",
    );
    assert_eq!(
        start_projection_sites(),
        Vec::<String>::new(),
        "something projected"
    );
    assert_eq!(
        raw_core_divergences(),
        divergences,
        "an equal core is never counted"
    );
}

/// #1170's flowing speculative capture (R101.15's `PDT:EM(starting)` carries
/// `UnlessSpeculativeFlowing`), run again through the switched entries.
#[test]
fn a_flowing_speculative_capture_holds_through_the_switched_entry() {
    let _gate = serialized();
    let divergences = raw_core_divergences();
    super::start_tests::a_flowing_speculative_capture_suppresses_the_redundant_starting_event();
    // Two starts (not flowing, then flowing), each its own world: the same four entries, each
    // reading the pair it should.
    let one = [
        (E101, Status::Idle, 0),
        (E402, Status::Starting, 1),
        (E103, Status::Starting, 1),
        (E104, Status::Starting, 1),
    ];
    let both: Vec<(&str, Status, u64)> = one.iter().chain(one.iter()).copied().collect();
    assert_reads(entries_read(), &both, "speculative capture");
    assert_eq!(
        start_projection_sites(),
        Vec::<String>::new(),
        "something projected"
    );
    assert_eq!(
        raw_core_divergences(),
        divergences,
        "an equal core is never counted"
    );
}

// ------------------------------------------------- the generation-scoped cancel

/// The generation a `recording_starting` reply names.
fn replied_generation(reply: &Result<serde_json::Value, String>) -> u64 {
    let value = reply.as_ref().expect("the start replied");
    assert_eq!(value["type"], "recording_starting", "{value}");
    value["recordingId"].as_u64().expect("a generation")
}

fn persisted_and_live(world: &Arc<World>) -> (Pair, u64) {
    let state = world.state();
    let dictation = state.app_state.dictation.lock_or_recover();
    (
        (dictation.core.status(), dictation.core.current_id()),
        state.app_state.recording_id.load(Ordering::SeqCst),
    )
}

/// CP4 compares the reply **kind**, so a `recording_starting` reply naming the wrong
/// generation can pass while a released hold (the window cancels what it believes is the
/// attempt) cancels the wrong attempt. Through the switched entries: the reply names the
/// persisted generation; a cancel scoped to another generation is refused and leaves the attempt
/// (and its start guard) alone; the cancel scoped to this generation cancels exactly this
/// attempt; and the next start is allocated the next generation and is not refused by the old
/// cancel.
#[test]
fn a_start_reply_names_the_generation_a_scoped_cancel_acts_on() {
    let _gate = serialized();
    let divergences = raw_core_divergences();
    let lines = run_sub_run(&SubRun {
        title: "scoped cancel",
        setup: Setup::idle(),
        body: |w| {
            w.knobs().cold_start = ColdStart::ReadyAfterContext;
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            let generation = replied_generation(&reply);

            // The reply's generation is the core's, not a value the shell made up.
            let (persisted, live) = persisted_and_live(w);
            assert_eq!(persisted, (Status::Starting, RecordingId(generation)));
            assert_eq!(live, generation, "the reply, the core and live agree");

            // Another owner's capture-cancel operations, named by `(recording id, kind)`: the
            // cancel of this generation must neither consume nor retire any of them.
            let history = w.state().app_state.core_history.clone();
            let decoys = [generation + 1, generation + 2].map(|id| {
                ContinuationGuard::register(
                    &history,
                    ContinuationSpec {
                        recording_id: RecordingId(id),
                        kind: ContKind::CaptureCancel,
                    },
                )
            });

            // The start guard: a second start while this one is `Starting` is refused through
            // the switched entry, and changes neither generation nor status.
            let mut again = w.start();
            let second = complete(&mut again);
            w.reply("start while starting", &second);
            let second = second.expect("a typed refusal");
            assert_eq!(second["type"], "already_starting", "{second}");
            assert_eq!(
                persisted_and_live(w),
                ((Status::Starting, RecordingId(generation)), generation),
                "the refused start left the attempt alone"
            );

            // A hold released for another generation is refused: nothing is cancelled.
            let before = w.trace().len();
            w.note("a cancel scoped to another generation");
            let mut wrong = w.cancel(Some(generation + 1));
            let refused = complete(&mut wrong);
            w.reply_unit("cancel wrong generation", &refused);
            assert_eq!(refused, Ok(()));
            let after = &w.trace()[before..];
            assert!(
                !after
                    .iter()
                    .any(|line| line.starts_with("supervisor.cancel")),
                "a cancel scoped to generation {} cancelled the capture: {after:#?}",
                generation + 1
            );
            assert_eq!(
                persisted_and_live(w),
                ((Status::Starting, RecordingId(generation)), generation),
                "the attempt and its start guard are untouched"
            );
            assert_eq!(
                terminal(w, generation),
                None,
                "no terminal for a generation that was not cancelled"
            );
            assert_eq!(
                w.state().app_state.cancelled_id.load(Ordering::SeqCst),
                0,
                "nothing was marked cancelled"
            );
            for decoy in &decoys {
                assert!(
                    history.is_outstanding(decoy.token()),
                    "a refused cancel consumed another generation's operation"
                );
            }

            // The cancel scoped to this generation cancels exactly this attempt.
            w.note("a cancel scoped to this generation");
            let before = w.trace().len();
            let mut right = w.cancel(Some(generation));
            let cancelled = complete(&mut right);
            w.reply_unit("cancel", &cancelled);
            assert_eq!(cancelled, Ok(()));
            let after = &w.trace()[before..];
            assert!(
                after
                    .iter()
                    .any(|line| *line == format!("supervisor.cancel {generation} user")),
                "the capture of generation {generation} was cancelled: {after:#?}"
            );
            assert_eq!(
                after
                    .iter()
                    .filter(|line| line.starts_with("supervisor.cancel "))
                    .count(),
                1,
                "exactly one capture was cancelled: {after:#?}"
            );
            assert_eq!(
                terminal(w, generation),
                Some((
                    DictationTerminalOutcome::UserCancelledStarting,
                    DictationErrorCode::CancelledStarting
                ))
            );
            for decoy in &decoys {
                assert!(
                    history.is_outstanding(decoy.token()),
                    "the cancel of generation {generation} consumed another owner's operation"
                );
            }
            drop(decoys);
            w.release_cancel_finish();
            assert_eq!(
                persisted_and_live(w),
                ((Status::Idle, RecordingId(generation)), generation)
            );

            // The next start gets the next generation and is not cancelled by the old one.
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            let mut next = w.start();
            let reply = complete(&mut next);
            w.reply("start again", &reply);
            assert_eq!(replied_generation(&reply), generation + 1);
            assert_eq!(
                terminal(w, generation + 1),
                None,
                "the old cancel did not reach the new attempt"
            );
            assert_eq!(
                persisted_and_live(w),
                (
                    (Status::Recording, RecordingId(generation + 1)),
                    generation + 1
                )
            );

            // The old attempt's cancel arrives late, scoped to the older generation, while the
            // newer attempt records: refused, and the newer attempt is untouched. (The other
            // direction, a newer generation than the current one, is refused above.)
            let cancelled_through = w.state().app_state.cancelled_id.load(Ordering::SeqCst);
            let before = w.trace().len();
            w.note("a cancel scoped to the older generation");
            let mut late = w.cancel(Some(generation));
            let refused = complete(&mut late);
            w.reply_unit("cancel older generation", &refused);
            assert_eq!(refused, Ok(()));
            let after = &w.trace()[before..];
            assert!(
                !after
                    .iter()
                    .any(|line| line.starts_with("supervisor.cancel")),
                "a cancel scoped to the older generation {generation} cancelled the capture of \
                 {}: {after:#?}",
                generation + 1
            );
            assert_eq!(
                persisted_and_live(w),
                (
                    (Status::Recording, RecordingId(generation + 1)),
                    generation + 1
                ),
                "the newer attempt is untouched by the older generation's cancel"
            );
            assert_eq!(terminal(w, generation + 1), None);
            assert_eq!(
                w.state().app_state.cancelled_id.load(Ordering::SeqCst),
                cancelled_through,
                "nothing newer was marked cancelled"
            );
        },
    });
    assert!(
        lines
            .iter()
            .any(|line| line == "publish_recording_ready 2 -> true"),
        "the new attempt published: {lines:#?}"
    );
    assert_eq!(
        start_projection_sites(),
        Vec::<String>::new(),
        "something projected"
    );
    assert_eq!(
        raw_core_divergences(),
        divergences,
        "an equal core is never counted"
    );
    // The entries the starts and the cancel crossed, with the pair each read: the first start, the
    // refused second one (`Starting`, generation 1), the third after the cancel left `Idle` on
    // generation 1, then generation 2.
    assert_reads(
        entries_read(),
        &[
            (E101, Status::Idle, 0),
            (E402, Status::Starting, 1),
            (E103, Status::Starting, 1),
            (E101, Status::Starting, 1),
            (E101, Status::Idle, 1),
            (E402, Status::Starting, 2),
            (E103, Status::Starting, 2),
            (E104, Status::Starting, 2),
        ],
        "scoped cancel",
    );
}
