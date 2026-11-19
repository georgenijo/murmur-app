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
//! * `no_start_entry_or_query_projects` - no production function names the `project()` entry
//!   (`FieldAdapter::project_lifecycle_view`) or `DictationCore::project`, by source and, for
//!   every fixture, by the **count** (`probe::projections()`), so a helper that projects for an
//!   entry or a query is caught too, and each fixture reads exactly the entries and queries it
//!   crosses, by name.
//! * `every_start_entry_reads_the_persisted_fields_and_resyncs_a_divergence` - the evidence is
//!   captured **at the entry**, from the actual persisted fields, before any adapter write could
//!   repair them: a core planted off live is read as planted, counted once (`core_divergences`,
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
use crate::dictation_executor::adapter::core_divergences;
use crate::dictation_executor::probe;
use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
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
const Q401: &str = "register_partial_ticker_locked#0";

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
    /// Now, when the body starts.
    Now,
    /// The next time a port reaches the label (D is free at every label used).
    AtPort(&'static str),
}

thread_local! {
    static PLAN: RefCell<Plant> = const { RefCell::new(Plant::Nowhere) };
}

/// The `(planted, live)` pair the plant wrote, whichever thread ran it.
static PLANTED: StdMutex<Option<(Pair, Pair)>> = StdMutex::new(None);

/// `DictationCore::project` entries made while the command under test ran.
static RUN_PROJECTIONS: AtomicU64 = AtomicU64::new(0);

fn planted_now(world: &Arc<World>) {
    let pair = world.plant_core_divergence();
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pair);
}

/// Plants as the run asks for, called by a body right before its command.
fn arm(world: &Arc<World>) {
    match PLAN.with(|plan| *plan.borrow()) {
        Plant::Nowhere => {}
        Plant::Now => planted_now(world),
        Plant::AtPort(port) => {
            let world = Arc::clone(world);
            let planted = Arc::clone(&world);
            world.on(port, move || planted_now(&planted));
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

/// A start, announced by a marker line, with its `project()` entries counted.
fn start_command(world: &Arc<World>) {
    arm(world);
    world.note("start requested");
    let before = probe::projections();
    let mut start = world.start();
    let reply = complete(&mut start);
    RUN_PROJECTIONS.fetch_add(probe::projections() - before, AtomicOrdering::SeqCst);
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
    arm(world);
    world.note("start requested");
    let before = probe::projections();
    let mut start = world.start_smart_auto(Some("fixed-device".to_string()));
    let reply = complete(&mut start);
    RUN_PROJECTIONS.fetch_add(probe::projections() - before, AtomicOrdering::SeqCst);
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
            plant: Some((Plant::AtPort("supervisor.start"), 1)),
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
            plant: Some((Plant::AtPort("supervisor.start"), 1)),
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
/// likewise, the `DictationCore::project` entries the command made, the divergences counted,
/// and the trace.
struct Run {
    entries: Vec<(String, Pair, Pair)>,
    queries: Vec<(String, Pair, Pair)>,
    projections: u64,
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
    RUN_PROJECTIONS.store(0, AtomicOrdering::SeqCst);
    let divergences = core_divergences();
    let trace = run_sub_run(&SubRun {
        title: case.name,
        setup: (case.setup)(),
        body: case.body,
    });
    PLAN.with(|slot| *slot.borrow_mut() = Plant::Nowhere);
    assert_no_data_dir(case.name);
    Run {
        entries: entries_read(),
        queries: queries_read(),
        projections: RUN_PROJECTIONS.load(AtomicOrdering::SeqCst),
        divergences: core_divergences() - divergences,
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

fn names(reads: &[(String, Pair, Pair)]) -> Vec<&str> {
    reads.iter().map(|(name, _, _)| name.as_str()).collect()
}

// ------------------------------------------------------------------------ the tests

#[test]
fn no_start_entry_or_query_projects() {
    let _gate = serialized();
    let source = recording_source();
    let spans = fn_spans(&source);

    // By source: no function of `recording.rs` (the shell, its ports and its entries) names
    // the `project()` entry, and the persisted entries are where the table above says.
    for (function, _, _) in &spans {
        assert!(
            lines_with(&source, &spans, function, "project_lifecycle_view").is_empty(),
            "`{function}` builds a view with `project()`"
        );
        assert!(
            lines_with(&source, &spans, function, "DictationCore::project").is_empty(),
            "`{function}` calls `DictationCore::project`"
        );
    }
    for (function, views, queries) in [
        ("start_native_recording_inner", 2, 0),
        ("start_after_capture", 3, 0),
        ("publish_recording_ready_with", 1, 0),
        ("process_audio_inner", 1, 0),
        ("register_partial_ticker_locked", 0, 1),
        ("register_partial_ticker_locked_observed", 0, 1),
        ("owns_delivery", 0, 1),
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
    // The ports and the other production files hold no `project()` entry either.
    for file in [
        "src/commands/recording/start_ports.rs",
        "src/commands/recording/ready_ports.rs",
        "src/commands/recording/lifecycle_ports.rs",
        "src/commands/recording/stop_ports.rs",
        "src/commands/recording/cancel_ports.rs",
        "src/commands/recording/process_audio_ports.rs",
        "src/dictation_executor/executor.rs",
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
            run.projections, 0,
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
        assert_eq!(planted.projections, 0, "{}", case.name);
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

/// Queries 401 (the ticker registration, 404's shape) and 403 (the delivery fence) answer from
/// the **persisted** M1/M2: a core planted off live decides them, they count the divergence
/// (they cannot re-sync, they hold only `&DictationState`) and they never project.
#[test]
fn the_queries_follow_the_persisted_core() {
    use crate::dictation_executor::FieldAdapter;
    let _gate = serialized();
    let projections = probe::projections();

    // 403: live says `Processing` for id 5 (the old code answers true); the persisted core
    // says `Recording` for id 6, which does not own the delivery.
    let app_state = AppState::default();
    app_state.recording_id.store(5, Ordering::SeqCst);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
        FieldAdapter::seed_core(&app_state, &mut dictation);
        assert!(owns_delivery(&app_state, &dictation, 5), "equal core");
        assert!(
            !owns_delivery(&app_state, &dictation, 6),
            "id 6 is not current"
        );
    }
    assert_eq!(queries_read().len(), 2);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.core.set_status(Status::Recording);
        let before = core_divergences();
        let persisted = (dictation.core.status(), dictation.core.current_id());
        assert!(
            !owns_delivery(&app_state, &dictation, 5),
            "decided on the persisted `Recording`, not on live `Processing`"
        );
        assert_eq!(core_divergences() - before, 1, "counted once");
        let reads = queries_read();
        assert_eq!(names(&reads), ["owns_delivery#0"]);
        assert_eq!(reads[0].1, persisted, "read as planted");
        assert_eq!(
            reads[0].2,
            (Status::Processing, RecordingId(5)),
            "live as left"
        );
        // Not re-synced by a query: the next entry does it.
        assert_eq!(dictation.core.status(), Status::Recording);
    }
    // The delivery fence goes through the same query: a divergent core refuses to run the
    // operation that a current delivery owner would.
    assert_eq!(
        with_current_delivery_owner(&app_state, 5, || 7),
        None,
        "the fence follows the persisted core"
    );
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        FieldAdapter::seed_core(&app_state, &mut dictation);
    }
    assert_eq!(with_current_delivery_owner(&app_state, 5, || 7), Some(7));
    queries_read();

    // 401: live says `Recording` for id 5 (the old code registers the ticker); the persisted
    // core says `Processing`, which is not recording.
    let app_state = AppState::default();
    app_state.recording_id.store(5, Ordering::SeqCst);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Recording;
        FieldAdapter::seed_core(&app_state, &mut dictation);
        dictation.core.set_status(Status::Processing);
        let before = core_divergences();
        assert!(
            !register_partial_ticker_locked(&app_state, &dictation, 5),
            "decided on the persisted `Processing`, not on live `Recording`"
        );
        assert_eq!(core_divergences() - before, 1, "counted once");
        let reads = queries_read();
        assert_eq!(names(&reads), [Q401]);
        assert_eq!(reads[0].1, (Status::Processing, RecordingId(5)));
        assert_eq!(reads[0].2, (Status::Recording, RecordingId(5)));
    }

    assert_eq!(probe::projections(), projections, "a query projected");
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
    let projections = probe::projections();
    let divergences = core_divergences();
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
    assert_eq!(probe::projections(), projections, "something projected");
    assert_eq!(
        core_divergences(),
        divergences,
        "an equal core is never counted"
    );
}

/// #1170's flowing speculative capture (R101.15's `PDT:EM(starting)` carries
/// `UnlessSpeculativeFlowing`), run again through the switched entries.
#[test]
fn a_flowing_speculative_capture_holds_through_the_switched_entry() {
    let _gate = serialized();
    let projections = probe::projections();
    let divergences = core_divergences();
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
    assert_eq!(probe::projections(), projections, "something projected");
    assert_eq!(
        core_divergences(),
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
    let projections = probe::projections();
    let divergences = core_divergences();
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
        },
    });
    assert!(
        lines
            .iter()
            .any(|line| line == "publish_recording_ready 2 -> true"),
        "the new attempt published: {lines:#?}"
    );
    assert_eq!(probe::projections(), projections, "something projected");
    assert_eq!(
        core_divergences(),
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
