//! The stop cutover (Stage 2c, appendix C9.7 and C8.6): the stop family's D entries decide
//! on the **persisted core** (M1 `status`, M2 `current_id`, `DictationState.core`), not on
//! `DictationCore::project`. The entries are:
//!
//! | label | site | where |
//! |---|---|---|
//! | `stop_native_recording_admitted#0` | 201 | the request's D section |
//! | `stop_native_recording_admitted#1` | 202 (no context) | the missing-context D section |
//! | `run_stop_capture_rows#0` | 203 | the capture result's D section |
//! | `finish_section#0` | 204a | the `IdleGuard` finish, all four origins (and 302.1) |
//! | `run_pipeline_idle_emit#0` | 204b | the shared idle tail (and 302.2) |
//! | `process_audio_inner#0` | 301 | the legacy import's start |
//! | `process_audio_inner#1` | 301.6 | the legacy import's destination row |
//!
//! Sites 202.1/202.2 (context present), the stale stop (R201.1) and 206 are D-less: their
//! lock-free reads stay. The delivery and capture queries (402, 403) are shared with the
//! start family and move with the **start** cutover, so the import still reads `project()`
//! once, for query 402 (counted below, by name).
//!
//! What each test proves, and why the golden invariant alone is not enough:
//!
//! * `no_stop_entry_projects` - none of the entries calls the `project()` entry
//!   (`FieldAdapter::project_lifecycle_view`), by source and, for every fixture, by the
//!   **count** (`probe::projections()`), so a helper that projects for an entry is caught
//!   too, and each fixture reads exactly the entries it crosses, by name.
//! * `every_stop_entry_reads_the_persisted_fields_and_resyncs_a_divergence` - the evidence
//!   is captured **at the entry**, from the actual persisted fields, before any adapter
//!   write could repair them: a core planted off live is read as planted, counted once
//!   (`core_divergences`, C8.6), re-synced before the decision, and the run then reads
//!   exactly as it does for an equal core.
//! * `every_idle_guard_origin_...` - the four `IdleGuard` origins through the switched
//!   204a entry.
//! * the PD-25 and #216 / late-idle tests - the preserved ledger winners and the fences,
//!   through the switched entries.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, stopped, sub, sub_from};
use super::pd25_tests::{
    import_audio, pd25_import, pd25_start, perf_claim, terminal, ImportHost, SUPERSEDED,
};
use super::*;
use crate::dictation_executor::adapter::core_divergences;
use crate::dictation_executor::probe;
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;
use std::cell::RefCell;
use std::sync::Mutex as StdMutex;

// ----------------------------------------------------------------------- entry labels

const E201: &str = "stop_native_recording_admitted#0";
const E202: &str = "stop_native_recording_admitted#1";
const E203: &str = "run_stop_capture_rows#0";
const E204A: &str = "finish_section#0";
const E204B: &str = "run_pipeline_idle_emit#0";
const E301: &str = "process_audio_inner#0";
const E3016: &str = "process_audio_inner#1";

type Pair = (Status, RecordingId);

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

/// The entry label of a `stop_view` call at `line`: the enclosing function and the call's
/// ordinal inside it.
fn label(source: &str, spans: &[(String, usize, usize)], line: usize) -> String {
    let (name, _, _) = spans
        .iter()
        .filter(|(_, first, last)| (*first..=*last).contains(&line))
        .min_by_key(|(_, first, last)| last - first)
        .unwrap_or_else(|| panic!("line {line} is inside a function"));
    let calls = lines_with(source, spans, name, "FieldAdapter::stop_view(");
    let ordinal = calls
        .iter()
        .position(|call| *call == line)
        .unwrap_or_else(|| panic!("line {line} is a `stop_view` call in `{name}`: {calls:?}"));
    format!("{name}#{ordinal}")
}

// ------------------------------------------------------------------------ probes

/// Tests share the process-global probes; one cutover test at a time. The entry reads are
/// remembered only while the guard lives (remembering allocates, which the allocation pins
/// under D must not see), and are cleared when it starts and ends.
struct Probes(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

impl Drop for Probes {
    fn drop(&mut self) {
        probe::record_stop_entries(false);
        probe::take_stop_entries();
    }
}

fn serialized() -> Probes {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let gate = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe::take_stop_entries();
    crate::commands::settings_store::test_data_dir::take_returned();
    probe::record_stop_entries(true);
    Probes(gate)
}

/// What a run read: each stop entry as `(label, persisted, live)`, oldest first, the
/// `DictationCore::project` entries made while it ran, and the divergences it counted.
struct Run {
    entries: Vec<(String, Pair, Pair)>,
    projections: u64,
    divergences: u64,
    trace: Vec<String>,
}

fn labelled(reads: Vec<probe::StopEntryRead>) -> Vec<(String, Pair, Pair)> {
    let source = recording_source();
    let spans = fn_spans(&source);
    reads
        .into_iter()
        .map(|((persisted, live), (file, line))| {
            assert!(
                file.ends_with("commands/recording.rs"),
                "a stop entry outside recording.rs: {file}:{line}"
            );
            (label(&source, &spans, line as usize), persisted, live)
        })
        .collect()
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
    /// The next time production reaches a `barrier_hook` point (D is free there).
    AtBarrier(&'static str),
}

thread_local! {
    static PLAN: RefCell<Plant> = const { RefCell::new(Plant::Nowhere) };
}

/// The `(planted, live)` pair the plant wrote, whichever thread ran it.
static PLANTED: StdMutex<Option<(Pair, Pair)>> = StdMutex::new(None);

fn planted_now(world: &Arc<World>) {
    let pair = world.plant_core_divergence();
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pair);
}

/// Called by a body where the plan says: arms the plant. The returned guard (a barrier
/// hook) must live to the end of the body.
fn arm(world: &Arc<World>) -> Option<crate::barrier_hook::Installed> {
    match PLAN.with(|plan| *plan.borrow()) {
        Plant::Nowhere => None,
        Plant::Now => {
            planted_now(world);
            None
        }
        Plant::AtPort(label) => {
            let world = Arc::clone(world);
            let w = Arc::clone(&world);
            world.on(label, move || planted_now(&w));
            None
        }
        Plant::AtBarrier(point) => {
            let world = Arc::clone(world);
            let mut done = false;
            Some(crate::barrier_hook::install(move |reached| {
                if reached == point && !done {
                    done = true;
                    planted_now(&world);
                }
            }))
        }
    }
}

// ------------------------------------------------------------------------ the fixtures

/// A stop fixture: what it starts from, what it does, and the entries it crosses.
struct Case {
    name: &'static str,
    setup: fn() -> Setup,
    body: fn(&Arc<World>),
    /// The stop entries the run reads, in order.
    entries: &'static [&'static str],
    /// `DictationCore::project` entries the run is still expected to make: the other
    /// families' queries that stay on `project()` until their own cutover.
    queries: u64,
    /// Where a divergence is planted so that `entries[planted_entry]` is the first read
    /// after it: `(plant, index into entries)`.
    plant: Option<(Plant, usize)>,
}

fn recording_1() -> Setup {
    Setup {
        status: DictationStatus::Recording,
        recording_id: 1,
    }
}

fn idle() -> Setup {
    Setup::idle()
}

/// A recording, as the stop command finds it: `Recording`, its context installed and the
/// supervisor holding its attempt. The start's own entries are not run (the start family
/// is not switched), so what the stop reads is the persisted core the setup seeded.
fn ready_for_stop(world: &Arc<World>) {
    world.install_context(1);
    world.accepted_start(1);
}

/// R201.x + R203.9 + 204a + 204b: a long capture runs the pipeline.
fn stop_long(world: &Arc<World>) {
    let _plant = arm(world);
    ready_for_stop(world);
    world.knobs().stop_result = Ok(10_000);
    stopped(world, "stop", None);
}

/// R201 + R202.3: the context is missing.
fn stop_without_context(world: &Arc<World>) {
    let _plant = arm(world);
    world.accept_silently(1);
    stopped(world, "stop", None);
}

/// R201 + R203.4/.6: a capture below the minimum duration.
fn stop_short(world: &Arc<World>) {
    let _plant = arm(world);
    ready_for_stop(world);
    world.knobs().stop_result = Ok(100);
    stopped(world, "stop", None);
}

/// R201 + R203.2: the capture stop fails.
fn stop_failed(world: &Arc<World>) {
    let _plant = arm(world);
    ready_for_stop(world);
    world.knobs().stop_result = Err("boom");
    stopped(world, "stop", None);
}

/// R201.2: not recording.
fn stop_idle(world: &Arc<World>) {
    let _plant = arm(world);
    stopped(world, "stop", None);
}

/// R201.3: already processing.
fn stop_processing(world: &Arc<World>) {
    let _plant = arm(world);
    stopped(world, "stop", None);
}

/// R201.1: a stale scoped stop. D-less: it reads no persisted core.
fn stop_stale(world: &Arc<World>) {
    world.install_context(2);
    world.accept_silently(1);
    stopped(world, "stop", Some(1));
}

/// The legacy import's reply, as the command returns it.
fn import(world: &Arc<World>, audio: &str, label: &str) -> Result<serde_json::Value, String> {
    let host = ImportHost::new();
    import_with(world, host, audio, label)
}

fn import_with(
    world: &Arc<World>,
    host: ImportHost,
    audio: &str,
    label: &str,
) -> Result<serde_json::Value, String> {
    let mut run: Fut<Result<serde_json::Value, String>> = Box::pin({
        let world = Arc::clone(world);
        let audio = audio.to_string();
        async move {
            let state = world.state();
            commanded_reply(process_audio_inner(&host, state.inner(), &audio)).await
        }
    });
    let reply = complete(&mut run);
    world.reply(label, &reply);
    reply
}

/// R301.5 + 302.1/.2: an accepted import runs the pipeline and its idle tail.
fn import_accepted(world: &Arc<World>) {
    let _plant = arm(world);
    let reply = import(world, &import_audio(), "import");
    assert!(reply.is_ok(), "{reply:?}");
}

/// R301.5 + R301.6: the same with Smart Destination on (the destination row).
fn import_with_destination(world: &Arc<World>) {
    {
        let state = world.state();
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.smart_destination_enabled = true;
        dictation.auto_paste = true;
        dictation.save_audio = false;
        dictation.save_transcript = false;
    }
    let host = ImportHost::new();
    if let Plant::AtPort("import.capture") = PLAN.with(|plan| *plan.borrow()) {
        let planted = Arc::clone(world);
        *host
            .at_capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Box::new(move || planted_now(&planted)));
    }
    let reply = import_with(world, host, &import_audio(), "import");
    assert!(reply.is_ok(), "{reply:?}");
}

/// R301.5 + R302.1: the audio does not parse. The import's own guard finishes it
/// (`IdleGuard` origin `LegacyParse`).
fn import_parse_failure(world: &Arc<World>) {
    let _plant = arm(world);
    let reply = import(world, "!!not base64!!", "import");
    assert!(reply.is_err(), "{reply:?}");
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "a long stop (201, 204a pipeline exit, 204b)",
            setup: recording_1,
            body: stop_long,
            entries: &[E201, E204A, E204B],
            queries: 0,
            plant: Some((Plant::Now, 0)),
        },
        Case {
            name: "a long stop, planted at the pipeline guard (204a)",
            setup: recording_1,
            body: stop_long,
            entries: &[E201, E204A, E204B],
            queries: 0,
            plant: Some((Plant::AtPort("pipeline"), 1)),
        },
        Case {
            name: "a long stop, planted at the idle tail (204b)",
            setup: recording_1,
            body: stop_long,
            entries: &[E201, E204A, E204B],
            queries: 0,
            plant: Some((Plant::AtBarrier("pipeline_idle_emit"), 2)),
        },
        Case {
            name: "a stop without a context (201, 202)",
            setup: recording_1,
            body: stop_without_context,
            entries: &[E201, E202],
            queries: 0,
            plant: Some((Plant::AtPort("keyboard.stop_gesture_take_for"), 1)),
        },
        Case {
            name: "a short capture (201, 203)",
            setup: recording_1,
            body: stop_short,
            entries: &[E201, E203],
            queries: 0,
            plant: Some((Plant::AtPort("supervisor.stop"), 1)),
        },
        Case {
            name: "a failed capture stop (201, 203)",
            setup: recording_1,
            body: stop_failed,
            entries: &[E201, E203],
            queries: 0,
            plant: None,
        },
        Case {
            name: "a stop while idle (201 refuses)",
            setup: idle,
            body: stop_idle,
            entries: &[E201],
            queries: 0,
            plant: Some((Plant::Now, 0)),
        },
        Case {
            name: "a stop while processing (201 refuses)",
            setup: || Setup {
                status: DictationStatus::Processing,
                recording_id: 3,
            },
            body: stop_processing,
            entries: &[E201],
            queries: 0,
            plant: None,
        },
        Case {
            name: "a stale scoped stop (R201.1, D-less)",
            setup: || Setup {
                status: DictationStatus::Recording,
                recording_id: 2,
            },
            body: stop_stale,
            entries: &[],
            queries: 0,
            plant: None,
        },
        Case {
            name: "an accepted import (301, 204a pipeline exit, 204b legacy tail)",
            setup: idle,
            body: import_accepted,
            entries: &[E301, E204A, E204B],
            queries: 1,
            plant: Some((Plant::Now, 0)),
        },
        Case {
            name: "an import, planted at its pipeline guard",
            setup: idle,
            body: import_accepted,
            entries: &[E301, E204A, E204B],
            queries: 1,
            plant: Some((Plant::AtBarrier("process_audio.accepted"), 1)),
        },
        Case {
            name: "an import, planted at its idle tail",
            setup: idle,
            body: import_accepted,
            entries: &[E301, E204A, E204B],
            queries: 1,
            plant: Some((Plant::AtBarrier("pipeline_idle_emit"), 2)),
        },
        Case {
            name: "an import with Smart Destination (301, 301.6, 204a, 204b)",
            setup: idle,
            body: import_with_destination,
            entries: &[E301, E3016, E204A, E204B],
            queries: 1,
            plant: Some((Plant::AtPort("import.capture"), 1)),
        },
        Case {
            name: "an import that does not parse (301, 204a legacy parse)",
            setup: idle,
            body: import_parse_failure,
            entries: &[E301, E204A],
            queries: 1,
            plant: Some((Plant::AtBarrier("process_audio.accepted"), 1)),
        },
    ]
}

/// Run `case` once with `plant`, serialised on the probes' gate.
fn run_case(case: &Case, plan: Plant) -> Run {
    PLAN.with(|slot| *slot.borrow_mut() = plan);
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    probe::take_stop_entries();
    crate::commands::settings_store::test_data_dir::take_returned();
    let projections = probe::projections();
    let divergences = core_divergences();
    let sub_run = SubRun {
        title: case.name,
        setup: (case.setup)(),
        body: case.body,
    };
    let trace = run_sub_run(&sub_run);
    PLAN.with(|slot| *slot.borrow_mut() = Plant::Nowhere);
    // The stop fixtures read no store and no data directory (the harness's pipeline, ticker
    // and VAD are stand-ins): `data_dir` returned nothing, sandboxed or real. A fixture
    // that needed one would take `vad_stream::StoreOverride` and a sandbox first.
    assert_eq!(
        crate::commands::settings_store::test_data_dir::take_returned(),
        Vec::<std::path::PathBuf>::new(),
        "{}: a stop fixture resolved the data directory",
        case.name
    );
    Run {
        entries: labelled(probe::take_stop_entries()),
        projections: probe::projections() - projections,
        divergences: core_divergences() - divergences,
        trace,
    }
}

/// Nothing since the last drain resolved the data directory (sandboxed or real).
fn assert_no_data_dir(name: &str) {
    assert_eq!(
        crate::commands::settings_store::test_data_dir::take_returned(),
        Vec::<std::path::PathBuf>::new(),
        "{name}: a stop fixture resolved the data directory"
    );
}

fn names(run: &Run) -> Vec<&str> {
    run.entries
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect()
}

// ------------------------------------------------------------------------ the tests

#[test]
fn no_stop_entry_projects() {
    let _gate = serialized();
    let source = recording_source();
    let spans = fn_spans(&source);

    // By source: none of the stop entries names the `project()` entry; each names the
    // persisted one once per D entry it holds. `process_audio_inner` keeps exactly one
    // `project_lifecycle_view`: query 402, shared with the start family.
    for (function, views, projections) in [
        ("stop_native_recording_admitted", 2, 0),
        ("run_stop_capture_rows", 1, 0),
        ("finish_section", 1, 0),
        ("run_pipeline_idle_emit", 1, 0),
        ("process_audio_inner", 2, 1),
    ] {
        assert_eq!(
            lines_with(&source, &spans, function, "FieldAdapter::stop_view(").len(),
            views,
            "`{function}` reads the persisted core once per D entry"
        );
        assert_eq!(
            lines_with(&source, &spans, function, "project_lifecycle_view").len(),
            projections,
            "`{function}` still builds a view with `project()`"
        );
        assert!(
            lines_with(&source, &spans, function, "DictationCore::project").is_empty(),
            "`{function}` calls `DictationCore::project`"
        );
    }

    // At run time: every fixture reads exactly the entries it crosses, by name, and
    // reaches `project()` only for the query the import shares with start. A count, not a
    // line span: a helper outside these functions that projects for an entry is caught.
    for case in cases() {
        let run = run_case(&case, Plant::Nowhere);
        assert_eq!(
            names(&run),
            case.entries,
            "{}: the entries it read (by name)",
            case.name
        );
        assert_eq!(
            run.projections, case.queries,
            "{}: `DictationCore::project` was reached beyond the shared query",
            case.name
        );
        assert_eq!(
            run.divergences, 0,
            "{}: an equal core is never counted",
            case.name
        );
        for (name, persisted, live) in &run.entries {
            assert_eq!(
                persisted, live,
                "{}: {name} read persisted != live on an equal core",
                case.name
            );
        }
    }
}

#[test]
fn every_stop_entry_reads_the_persisted_fields_and_resyncs_a_divergence() {
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

        // Captured at the entry, from the actual persisted fields: the planted core and
        // live as the fixture left it, before anything repaired it.
        assert_eq!(names(&planted), case.entries, "{}", case.name);
        let (name, persisted, live) = &planted.entries[at];
        assert_eq!(name, case.entries[at]);
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
        // Counted once (C8.6) ...
        assert_eq!(
            planted.divergences, 1,
            "{}: the divergence was counted exactly once",
            case.name
        );
        // ... and re-synced before the decision: every other entry reads persisted ==
        // live, and the run is the one an equal core gives.
        for (other, persisted, live) in planted.entries.iter().filter(|(other, _, _)| other != name)
        {
            assert_eq!(
                persisted, live,
                "{}: {other} read an unrepaired core",
                case.name
            );
        }
        assert_eq!(
            planted.trace, reference.trace,
            "{}: the divergence was re-synced before the decision, not decided on",
            case.name
        );
        covered.insert(name.clone());
    }
    for entry in [E201, E202, E203, E204A, E204B, E301, E3016] {
        assert!(covered.contains(entry), "{entry} was never planted");
    }
}

// ------------------------------------------------------------- the four IdleGuard origins

/// Every `IdleGuard` origin finishes through the one 204a section (`finish_section`), so
/// each reads the persisted core once there, counts a divergence and re-syncs it before it
/// decides (C1.4.3: "all three origins switch with the stop family"; the fourth, a stop
/// guard's drop, is `Dropped`). Built straight on an `AppState`: the section is the same
/// whoever calls it.
#[test]
fn every_idle_guard_origin_reads_the_persisted_core_and_resyncs_a_divergence() {
    use murmur_dictation_core::inputs::GuardOrigin;
    let _gate = serialized();

    type Make = fn(&AppState, u64) -> IdleGuard<'_>;
    let origins: [(&str, GuardOrigin, Make, bool); 5] = [
        (
            "StopCapture, finished",
            GuardOrigin::StopCapture,
            |a, r| IdleGuard::new(a, r),
            true,
        ),
        (
            "Dropped, a stop guard dropped",
            GuardOrigin::Dropped,
            |a, r| IdleGuard::new(a, r),
            false,
        ),
        (
            "PipelineExit, finished",
            GuardOrigin::PipelineExit,
            |a, r| IdleGuard::for_pipeline(a, r, None),
            true,
        ),
        (
            "LegacyParse, finished",
            GuardOrigin::LegacyParse,
            |a, r| IdleGuard::for_legacy_parse(a, r, None),
            true,
        ),
        (
            "Dropped, a legacy parse guard dropped",
            GuardOrigin::Dropped,
            |a, r| IdleGuard::for_legacy_parse(a, r, None),
            false,
        ),
    ];

    for (name, _origin, make, finish) in origins {
        for diverged in [false, true] {
            let app_state = AppState::default();
            let rid = app_state.next_recording_id();
            {
                let mut dictation = app_state.dictation.lock_or_recover();
                dictation.status = DictationStatus::Processing;
                crate::dictation_executor::FieldAdapter::seed_core(&app_state, &mut dictation);
                if diverged {
                    dictation.core.set_status(Status::Recording);
                    dictation.core.alloc_id();
                }
            }
            probe::take_stop_entries();
            let projections = probe::projections();
            let divergences = core_divergences();
            {
                let mut guard = make(&app_state, rid);
                if finish {
                    guard.finish(|_| {});
                }
                // Not finished: the drop is the finish.
            }
            let reads = labelled(probe::take_stop_entries());
            assert_eq!(
                reads.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>(),
                [E204A],
                "{name}: one read of the persisted core, in the 204a section"
            );
            let (_, persisted, live) = reads[0];
            assert_eq!(live, (Status::Processing, RecordingId(rid)), "{name}");
            if diverged {
                assert_eq!(
                    persisted,
                    (Status::Recording, RecordingId(rid + 1)),
                    "{name}: read as planted, before any repair"
                );
            } else {
                assert_eq!(persisted, live, "{name}");
            }
            assert_eq!(
                core_divergences() - divergences,
                u64::from(diverged),
                "{name}: a divergence is counted exactly once, an equal core never"
            );
            assert_eq!(
                probe::projections(),
                projections,
                "{name}: `DictationCore::project` was not reached"
            );
            // Decided as an equal core is: the id matches, so `Idle` is written, and the
            // persisted core follows live through the effect (`ST`).
            let dictation = app_state.dictation.lock_or_recover();
            assert_eq!(dictation.status, DictationStatus::Idle, "{name}");
            assert_eq!(
                (dictation.core.status(), dictation.core.current_id()),
                (Status::Idle, RecordingId(rid)),
                "{name}: re-synced, then written through the adapter"
            );
        }
    }
}

// ---------------------------------------------------------------------------- PD-25

/// PD-25 (#1157), characterized and **preserved**: a scoped stop for generation 1 that was
/// queued ahead of `T` resumes after another stop took the retained capture and a new
/// generation 2 was allocated. R201.1 wins generation 1's terminal as
/// `Superseded/StaleOwner` and the original stop's R204b.6 completes its performance as
/// `Success`. The fixtures (one per allocator: the legacy import R301.5 and an ordinary
/// start R101.15) are the PD-25 fixtures unchanged. What this adds is the run through the
/// switched entries: the ledger winners are asserted from the finished run, and the stop
/// entries are pinned by name and by what they read, so a different path winning either
/// outcome through the persisted core would show.
///
/// The scoped stop's R201.1 is the D-less stale stop: it reads no persisted core and takes
/// no stop entry. A fix PR deletes the PD-25 row and these assertions together.
fn pd25_through_the_switched_entries(
    title: &'static str,
    body: fn(&Arc<World>),
    expected: &[(&str, Pair)],
) {
    let _gate = serialized();
    probe::take_stop_entries();
    let divergences = core_divergences();
    run_sub_run_observed(&sub_from(title, DictationStatus::Idle, 0, body), &|world| {
        // The terminal winner of generation 1: the scoped stop's R201.1.
        assert_eq!(
            terminal(world, 1),
            Some(SUPERSEDED),
            "the terminal winner of generation 1"
        );
        // The performance winner of generation 1: the original stop's R204b.6.
        assert_eq!(
            perf_claim(1),
            Some(RunOutcomeV1::Success),
            "the performance winner of generation 1"
        );
    });
    let reads = labelled(probe::take_stop_entries());
    assert_eq!(
        reads
            .iter()
            .map(|(name, persisted, live)| (name.as_str(), *persisted, *live))
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(name, pair)| (*name, *pair, *pair))
            .collect::<Vec<_>>(),
        "{title}: the stop entries it read, by name, each on a persisted core equal to live"
    );
    assert_eq!(
        core_divergences(),
        divergences,
        "{title}: nothing was repaired"
    );
}

#[test]
fn pd25_through_a_start_allocator_runs_the_switched_entries_and_keeps_both_winners() {
    use Status::{Processing, Recording, Recovering};
    pd25_through_the_switched_entries(
        "PD-25 through a start (R101.15), switched entries",
        pd25_start,
        &[
            // The unscoped stop finds generation 1 `Recovering` (the runtime failure) and
            // R201.7 takes the retained capture ...
            (E201, (Recovering, RecordingId(1))),
            // ... the pipeline's guard writes Idle (R204a.1) ...
            (E204A, (Processing, RecordingId(1))),
            // ... and, after generation 2 was allocated at the barrier, R204b.6 reads the
            // new generation through the persisted core.
            (E204B, (Recording, RecordingId(2))),
        ],
    );
}

#[test]
fn pd25_through_an_import_allocator_runs_the_switched_entries_and_keeps_both_winners() {
    use Status::{Idle, Processing, Recovering};
    pd25_through_the_switched_entries(
        "PD-25 through an import (R301.5), switched entries",
        pd25_import,
        &[
            (E201, (Recovering, RecordingId(1))),
            (E204A, (Processing, RecordingId(1))),
            // The import runs inside the barrier, between 204a and 204b: its R301.5 reads
            // the Idle generation 1 and allocates generation 2, its pipeline guard and its
            // legacy tail (302.1, 302.2) read the new generation ...
            (E301, (Idle, RecordingId(1))),
            (E204A, (Processing, RecordingId(2))),
            (E204B, (Idle, RecordingId(2))),
            // ... and the original stop's R204b.6 then reads it too.
            (E204B, (Idle, RecordingId(2))),
        ],
    );
}

// ------------------------------------------------------------- #216 and the late-idle fence

fn status_emits(trace: &[String]) -> Vec<String> {
    trace
        .iter()
        .filter_map(|line| line.strip_prefix("emit recording-status-changed \""))
        .map(|rest| rest.trim_end_matches('"').to_string())
        .collect()
}

/// #216 (`06296f79`) through the switched entries: a hold-down stop reaches the command
/// while the start is still parked after the supervisor's ack, and queues behind it on the
/// recording transition; none of it runs until the start finished. The stop then reads the
/// persisted core the start's `ST` wrote through the adapter (`Recording`, generation 1) at
/// its 201, and decides on it: the reply is the pipeline's, not `not_recording`.
#[test]
fn bug_216_a_stop_queued_behind_a_start_runs_the_switched_entries_on_the_started_core() {
    use Status::{Idle, Processing, Recording};
    let _gate = serialized();
    probe::take_stop_entries();
    let lines = run_sub_run(&sub("216 stop queued behind start, switched", |world| {
        world.knobs().stop_result = Ok(16_000);
        world.close_gate("smart_capture");
        let mut start = world.start();
        assert!(poll_once(&mut start).is_pending());
        let mut stop = world.stop(None);
        assert!(
            poll_once(&mut stop).is_pending(),
            "the stop must queue behind the start on the recording transition"
        );
        assert!(
            probe::take_stop_entries().is_empty(),
            "no stop entry ran while the start holds the transition"
        );
        assert!(
            !world
                .trace()
                .iter()
                .any(|line| line.starts_with("supervisor.stop")),
            "nothing of the stop has run while the start holds the transition"
        );
        world.open_gate("smart_capture");
        let reply = complete(&mut start);
        world.reply("start", &reply);
        let reply = complete(&mut stop);
        world.reply("stop", &reply);
        let reply = reply.expect("the stop completes the pipeline");
        assert_eq!(reply["type"], "transcription", "not `not_recording`");
        assert_eq!(reply["state"], "idle");
    }));
    let emits = status_emits(&lines);
    assert_eq!(
        emits.get(..3),
        Some(
            &[
                "starting".to_string(),
                "recording".into(),
                "processing".into()
            ][..]
        ),
        "{emits:?}"
    );
    let processing = emits.iter().position(|s| s == "processing").unwrap();
    assert!(
        !emits[processing..].iter().any(|s| s == "recording"),
        "no `recording` after `processing`: {emits:?}"
    );
    assert_eq!(
        labelled(probe::take_stop_entries())
            .iter()
            .map(|(name, persisted, live)| (name.as_str(), *persisted, *live))
            .collect::<Vec<_>>(),
        [
            (
                E201,
                (Recording, RecordingId(1)),
                (Recording, RecordingId(1))
            ),
            (
                E204A,
                (Processing, RecordingId(1)),
                (Processing, RecordingId(1))
            ),
            (E204B, (Idle, RecordingId(1)), (Idle, RecordingId(1))),
        ],
        "the stop's entries read the core the start published"
    );
    assert_no_data_dir("216");
}

/// The late-idle fence (R204b.4, C3.5) through the switched entries: a newer start lands
/// between the pipeline guard's finish (204a) and the stop's own idle section (204b). 204b
/// reads the **newer** generation through the persisted core and so stays silent: no `idle`
/// emit and no keyboard-gate clear for the old recording after the new start's reply. A
/// 204b that read a stale persisted core (generation 1) would emit that `idle` over the new
/// `recording`.
#[test]
fn the_late_idle_fence_holds_through_the_switched_entries() {
    use Status::{Processing, Recording};
    let _gate = serialized();
    probe::take_stop_entries();
    let lines = run_sub_run(&sub("late idle fence, switched", |world| {
        recording(world);
        world.knobs().stop_result = Ok(16_000);
        let starter = Arc::clone(world);
        let _barrier = crate::barrier_hook::install(move |point| {
            if point == "pipeline_idle_emit" {
                starter.note("a newer start lands between 204a and 204b");
                starter.finish_sealed_stop(1);
                recording(&starter);
            }
        });
        stopped(world, "stop", None);
    }));
    let note = lines
        .iter()
        .position(|line| line.contains("a newer start lands between 204a and 204b"))
        .expect("the barrier fired");
    let start_reply = lines
        .iter()
        .rposition(|line| line.starts_with("reply start"))
        .expect("the newer start replied");
    assert!(note < start_reply, "{}", lines.join("\n"));
    let after_start = &lines[start_reply..];
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
    assert_eq!(
        labelled(probe::take_stop_entries())
            .iter()
            .map(|(name, persisted, live)| (name.as_str(), *persisted, *live))
            .collect::<Vec<_>>(),
        [
            (
                E201,
                (Recording, RecordingId(1)),
                (Recording, RecordingId(1))
            ),
            (
                E204A,
                (Processing, RecordingId(1)),
                (Processing, RecordingId(1))
            ),
            // The newer generation, read through the persisted core.
            (
                E204B,
                (Recording, RecordingId(2)),
                (Recording, RecordingId(2))
            ),
        ],
        "204b read the newer generation"
    );
    assert_no_data_dir("late idle fence");
}
