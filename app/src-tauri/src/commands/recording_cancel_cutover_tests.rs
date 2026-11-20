//! The cancel cutover (Stage 2c, appendix C9.7 and C8.6): the cancel family's one D entry,
//! site 205 (`decide_cancel_request`, R205.1-.5), decides on the **persisted core** (M1
//! `status`, M2 `current_id`, `DictationState.core`), not on `DictationCore::project`.
//!
//! | label | site | where |
//! |---|---|---|
//! | `cancel_native_recording_inner#0` | 205 | the cancel command's D section, T held |
//!
//! Site 206 (the capture-cancel result, shared by the cancel command's R205.4 and the stop
//! command's R201.4) is D-less: it reads no live field, so it has no entry and its lock-free
//! reads stay. The stop caller of 206 is the stop family's own switched entry 201.
//!
//! What each test proves, and why the golden invariant alone is not enough:
//!
//! * `no_cancel_entry_projects` - the entry does not call the `project()` entry
//!   (`FieldAdapter::project_lifecycle_view`), by source and, for every fixture, by the
//!   **count** (`probe::projections()` over the cancel command), so a helper that projects
//!   for the entry is caught too, and each fixture reads exactly the entry by name.
//! * `every_cancel_row_reads_the_persisted_fields_and_resyncs_a_divergence` - the evidence is
//!   captured **at the entry**, from the actual persisted fields, before any adapter write
//!   could repair them: a core planted off live is read as planted, counted once
//!   (`core_divergences`, C8.6), re-synced before the decision, and the run then reads as an
//!   equal core does.
//! * the R205.5 tests - #1170's exact tail order and the independent-start barrier, run again
//!   through the switched entry, with the exact entries that ran.
//! * the R206 tests - the delegated result, the owner consumed and the ledger winner, for
//!   both callers (the cancel command and the stop-during-Starting), including the case in
//!   which the supervisor claims the terminal first.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, started, stopped};
use super::*;
use crate::core_history::ContinuationGuard;
use crate::dictation_executor::adapter::{core_divergences_counted, forgive_planted_divergences};
use crate::dictation_executor::probe;
use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Mutex as StdMutex;

const E205: &str = "cancel_native_recording_inner#0";

/// A `(status, recording id)` pair: M1 and M2.
type Pair = (Status, RecordingId);

// ------------------------------------------------------------------------ source helpers
//
// Owned by this module (D-S2c-18 amendment 1: a cutover's tests import nothing from another
// cutover's test code), so reverting any one cutover leaves the others' tests compiling.

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

/// Generation `id`'s terminal winner in the telemetry ledger.
fn terminal(world: &Arc<World>, id: u64) -> Option<(DictationTerminalOutcome, DictationErrorCode)> {
    world
        .state()
        .app_state
        .dictation_telemetry
        .terminal_for_test(id)
}

// ------------------------------------------------------------------------ probes

/// Tests share the process-global probes; one cutover test at a time. The cancel entries are
/// remembered only while the guard lives (remembering allocates, which the allocation pins
/// under D must not see).
struct Probes(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

impl Drop for Probes {
    fn drop(&mut self) {
        probe::record_cancel_entries(false);
        probe::take_cancel_entries();
        probe::record_projections(false);
        probe::take_projection_sites();
        forgive_planted_divergences(0);
    }
}

fn serialized() -> Probes {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let gate = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe::take_cancel_entries();
    crate::commands::settings_store::test_data_dir::take_returned();
    probe::record_cancel_entries(true);
    Probes(gate)
}

/// The label of a `cancel_view` call at `line`: the enclosing function and the call's
/// ordinal inside it.
fn label(source: &str, spans: &[(String, usize, usize)], line: usize) -> String {
    let name = enclosing(spans, line);
    let calls = lines_with(source, spans, &name, "FieldAdapter::cancel_view(");
    let ordinal = calls
        .iter()
        .position(|call| *call == line)
        .unwrap_or_else(|| panic!("line {line} is a `cancel_view` call in `{name}`: {calls:?}"));
    format!("{name}#{ordinal}")
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

fn cancel_labelled(reads: Vec<probe::CancelEntryRead>) -> Vec<(String, Pair, Pair)> {
    let source = recording_source();
    let spans = fn_spans(&source);
    reads
        .into_iter()
        .map(|((persisted, live), (file, line))| {
            assert!(
                file.ends_with("commands/recording.rs"),
                "a cancel entry outside recording.rs: {file}:{line}"
            );
            (label(&source, &spans, line as usize), persisted, live)
        })
        .collect()
}

/// The functions the `DictationCore::project` entries since the last
/// `take_projection_sites` were made in, sorted.
fn projecting_functions() -> Vec<String> {
    let source = recording_source();
    let spans = fn_spans(&source);
    let mut functions: Vec<String> = probe::take_projection_sites()
        .into_iter()
        .map(|(file, line)| {
            assert!(
                file.ends_with("commands/recording.rs"),
                "a projection outside recording.rs: {file}:{line}"
            );
            enclosing(&spans, line as usize)
        })
        .collect();
    functions.sort();
    functions
}

// ------------------------------------------------------------------------ the plants

thread_local! {
    static PLANT: RefCell<bool> = const { RefCell::new(false) };
}

/// The `(planted, live)` pair the plant wrote.
static PLANTED: StdMutex<Option<(Pair, Pair)>> = StdMutex::new(None);

/// `DictationCore::project` entries made while the command under test ran.
static COMMAND_PROJECTIONS: AtomicU64 = AtomicU64::new(0);

/// A **planted** C8.6 divergence: put both persisted fields off live, as a write that
/// bypassed the adapter would, and allow the run's golden invariant exactly one counted
/// divergence (`forgive_planted_divergences`). The entry that next reads the persisted core
/// must count it and re-sync it; the test asserts that it did. The persisted fields are
/// checked equal to live first, so a plant never forgives an earlier violation. Returns the
/// planted pair and live's.
fn plant_core_divergence(world: &Arc<World>) -> (Pair, Pair) {
    let state = world.state();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    let live = (
        crate::dictation_admission::core_status(dictation.status),
        RecordingId(state.app_state.recording_id.load(AtomicOrdering::SeqCst)),
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
    forgive_planted_divergences(1);
    ((other, id), live)
}

/// Plants a divergence when the run asks for one: right before the command, so the entry is
/// the first read after it.
fn arm(world: &Arc<World>) {
    if PLANT.with(|plant| *plant.borrow()) {
        let pair = plant_core_divergence(world);
        *PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pair);
    }
}

/// The cancel command, announced by a marker line, with the `project()` entries it made
/// counted.
fn cancel_command(world: &Arc<World>, expected: Option<u64>) {
    // The marker first: a trace line samples the harness's persisted-core invariant, and the
    // plant must be followed by the entry that reads it with nothing sampled in between.
    world.note("cancel requested");
    arm(world);
    let before = probe::projections();
    let mut cancel = world.cancel(expected);
    let reply = complete(&mut cancel);
    COMMAND_PROJECTIONS.store(probe::projections() - before, AtomicOrdering::SeqCst);
    world.reply_unit("cancel", &reply);
}

/// The stop command (the other caller of site 206).
fn stop_command(world: &Arc<World>) {
    world.note("stop requested");
    stopped(world, "stop", None);
}

// ------------------------------------------------------------------------ the fixtures

struct Case {
    name: &'static str,
    setup: fn() -> Setup,
    body: fn(&Arc<World>),
}

fn idle() -> Setup {
    Setup::idle()
}

fn processing_3() -> Setup {
    Setup {
        status: DictationStatus::Processing,
        recording_id: 3,
    }
}

fn recording_2() -> Setup {
    Setup {
        status: DictationStatus::Recording,
        recording_id: 2,
    }
}

fn recovering_3() -> Setup {
    Setup {
        status: DictationStatus::Recovering,
        recording_id: 3,
    }
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "R205.2 idle",
            setup: idle,
            body: |w| cancel_command(w, None),
        },
        Case {
            name: "R205.1 idle with a stale expectation",
            setup: idle,
            body: |w| cancel_command(w, Some(5)),
        },
        Case {
            name: "R205.3 recovering",
            setup: recovering_3,
            body: |w| cancel_command(w, None),
        },
        Case {
            name: "R205.1 a stale expected id over a recording",
            setup: recording_2,
            body: |w| {
                w.install_context(2);
                cancel_command(w, Some(1));
            },
        },
        Case {
            name: "R205.4 starting, capture cancel accepted",
            setup: idle,
            body: |w| {
                w.knobs().cold_start = ColdStart::ReadyAfterContext;
                started(w);
                cancel_command(w, None);
                w.release_cancel_finish();
            },
        },
        Case {
            name: "R205.4 recording, capture cancel accepted",
            setup: idle,
            body: |w| {
                recording(w);
                cancel_command(w, Some(1));
                w.release_cancel_finish();
            },
        },
        Case {
            name: "R205.4 + R206.2 recording, capture cancel fails",
            setup: idle,
            body: |w| {
                recording(w);
                w.knobs().cancel_result = Err("boom");
                cancel_command(w, None);
            },
        },
        Case {
            name: "R205.4 + R206.1 recording, capture cancel not accepted",
            setup: idle,
            body: |w| {
                recording(w);
                w.knobs().cancel_result = Ok(false);
                cancel_command(w, None);
            },
        },
        Case {
            name: "R205.5 processing",
            setup: processing_3,
            body: |w| {
                w.install_context(3);
                cancel_command(w, None);
            },
        },
        Case {
            name: "R205.5 processing with the matching expectation",
            setup: processing_3,
            body: |w| {
                w.install_context(3);
                cancel_command(w, Some(3));
            },
        },
    ]
}

/// What a run read: each cancel entry as `(label, persisted, live)`, oldest first, the
/// `DictationCore::project` entries the cancel command made, the divergences counted, and
/// the trace.
struct Run {
    entries: Vec<(String, Pair, Pair)>,
    projections: u64,
    divergences: u64,
    trace: Vec<String>,
}

fn run_case(case: &Case, plant: bool) -> Run {
    PLANT.with(|slot| *slot.borrow_mut() = plant);
    *PLANTED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    probe::take_cancel_entries();
    crate::commands::settings_store::test_data_dir::take_returned();
    COMMAND_PROJECTIONS.store(u64::MAX, AtomicOrdering::SeqCst);
    let divergences = core_divergences_counted();
    let trace = run_sub_run(&SubRun {
        title: case.name,
        setup: (case.setup)(),
        body: case.body,
    });
    PLANT.with(|slot| *slot.borrow_mut() = false);
    forgive_planted_divergences(0);
    assert_no_data_dir(case.name);
    Run {
        entries: cancel_labelled(probe::take_cancel_entries()),
        projections: COMMAND_PROJECTIONS.load(AtomicOrdering::SeqCst),
        divergences: core_divergences_counted() - divergences,
        trace,
    }
}

/// The cancel fixtures read no store and no data directory (the harness's pipeline, ticker and
/// VAD are stand-ins): `data_dir` returned nothing since the last drain, sandboxed or real. A
/// fixture that needed one would take `vad_stream::StoreOverride` and a sandbox first.
fn assert_no_data_dir(name: &str) {
    assert_eq!(
        crate::commands::settings_store::test_data_dir::take_returned(),
        Vec::<std::path::PathBuf>::new(),
        "{name}: a cancel fixture resolved the data directory"
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
fn no_cancel_entry_projects() {
    let _gate = serialized();
    let source = recording_source();
    let spans = fn_spans(&source);

    // By source: the cancel command names the persisted entry once (site 205 is its only D
    // entry), and neither `project()` entry.
    let function = "cancel_native_recording_inner";
    assert_eq!(
        lines_with(&source, &spans, function, "FieldAdapter::cancel_view(").len(),
        1,
        "`{function}` reads the persisted core once, at its D entry"
    );
    assert!(
        lines_with(&source, &spans, function, "project_lifecycle_view").is_empty(),
        "`{function}` builds a view with `project()`"
    );
    assert!(
        lines_with(&source, &spans, function, "DictationCore::project").is_empty(),
        "`{function}` calls `DictationCore::project`"
    );
    // The view exists once, as the entry: no other production function reads through it.
    assert_eq!(
        source
            .lines()
            .filter(|line| line.contains("FieldAdapter::cancel_view("))
            .count(),
        1,
        "`cancel_view` is the cancel entry's alone"
    );

    // At run time: every fixture reads exactly the entry, by name, and the cancel command
    // reaches `project()` not at all. A count, not a line span: a helper outside the
    // function that projects for the entry is caught.
    for case in cases() {
        let run = run_case(&case, false);
        assert_eq!(
            names(&run),
            [E205],
            "{}: the entries it read (by name)",
            case.name
        );
        assert_eq!(
            run.projections, 0,
            "{}: the cancel command reached `DictationCore::project`",
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
fn every_cancel_row_reads_the_persisted_fields_and_resyncs_a_divergence() {
    let _gate = serialized();
    for case in cases() {
        // The reference: the same fixture on an equal core.
        let reference = run_case(&case, false);
        assert_eq!(reference.divergences, 0, "{}", case.name);
        let planted = run_case(&case, true);
        let (pair_planted, pair_live) = PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| panic!("{}: the plant ran", case.name));
        assert_ne!(pair_planted, pair_live, "{}: the plant diverges", case.name);

        // Captured at the entry, from the actual persisted fields: the planted core and
        // live as the fixture left it, before anything repaired it.
        assert_eq!(names(&planted), [E205], "{}", case.name);
        let (name, persisted, live) = &planted.entries[0];
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
        // ... and re-synced before the decision: the run is the one an equal core gives.
        assert_eq!(
            planted.trace, reference.trace,
            "{}: the divergence was re-synced before the decision, not decided on",
            case.name
        );
        assert_eq!(planted.projections, 0, "{}", case.name);
    }
}

// ------------------------------------------------- R205.5: the tail's order and fence

/// The cancel entry read `(Processing, 3)` for both the persisted core and live, the one
/// entry of a processing cancel.
fn assert_read_processing_3(entries: &[(String, Pair, Pair)], what: &str) {
    let processing_3 = (Status::Processing, RecordingId(3));
    assert_eq!(
        entries,
        [(E205.to_string(), processing_3, processing_3)],
        "{what}: the one switched entry, reading (Processing, 3) as persisted and as live"
    );
}

/// #1170's tail-order test, in the PD-20 order (R205.5: `In:ST→Idle`; `PDT:CX-, MC, TERM+PC`
/// (one pair, claimed together), `KB, EM, EV`; T held from `ST` through `EV`), run again through
/// the switched entry. The entries that ran are exact: the cancel's one D entry, once, reading
/// the persisted core, and the run reaches `project()` not at all (a processing cancel starts
/// nothing).
#[test]
fn the_r205_5_tail_order_holds_through_the_switched_entry() {
    let _gate = serialized();
    probe::record_projections(true);
    probe::take_projection_sites();
    let projections = probe::projections();
    let divergences = core_divergences_counted();
    super::cancel_tests::processing_cancel_tail_runs_in_r205_5_order();
    assert_read_processing_3(
        &cancel_labelled(probe::take_cancel_entries()),
        "the tail-order run",
    );
    assert_eq!(
        probe::projections() - projections,
        0,
        "the tail-order run reached `DictationCore::project`: {:?}",
        projecting_functions()
    );
    assert_eq!(
        core_divergences_counted(),
        divergences,
        "an equal core is never counted"
    );
    assert_no_data_dir("the tail-order run");
}

/// #1170's independent-start barrier (a processing cancel keeps T through its trailing
/// `idle`, so a newer start cannot publish `starting` and then be overwritten by the old
/// `idle`), run again through the switched entry. The cancel's entry is exact and reads the
/// persisted core. The start that is polled from an independent task is the start family's
/// and projects until the start cutover is applied: the functions that projected are the
/// start's and only the start's (none once it is), and the cancel is not among them.
#[test]
fn the_independent_start_barrier_holds_through_the_switched_entry() {
    let _gate = serialized();
    probe::record_projections(true);
    probe::take_projection_sites();
    let divergences = core_divergences_counted();
    super::cancel_tests::a_new_start_stays_blocked_until_the_trailing_idle();
    assert_read_processing_3(
        &cancel_labelled(probe::take_cancel_entries()),
        "the independent-start run",
    );
    let projecting = projecting_functions();
    assert!(
        projecting
            .iter()
            .all(|function| START_PROJECTING_FUNCTIONS.contains(&function.as_str())),
        "the functions that reached `DictationCore::project`: only the start's (which stops \
         projecting when its cutover is applied), never the cancel's: {projecting:?}"
    );
    assert_eq!(
        core_divergences_counted(),
        divergences,
        "an equal core is never counted"
    );
    assert_no_data_dir("the independent-start run");
}

/// The start family's own `project()` entries (it moves with the start cutover), as the
/// independent start of the barrier test runs them, sorted.
const START_PROJECTING_FUNCTIONS: &[&str] = &[
    "publish_recording_ready_with",
    // The ticker registration is the observed variant when the shadow is compiled in.
    #[cfg(not(feature = "dictation-shadow"))]
    "register_partial_ticker_locked",
    #[cfg(feature = "dictation-shadow")]
    "register_partial_ticker_locked_observed",
    "start_after_capture",
    "start_after_capture",
    "start_native_recording_inner",
];

fn names_of(entries: Vec<(String, Pair, Pair)>) -> Vec<String> {
    entries.into_iter().map(|(name, _, _)| name).collect()
}

// ----------------------------------------------- R206: delegation, owner and ledger winner

/// Who issues the capture cancel whose result site 206 decides.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Caller {
    /// The cancel command (R205.4).
    Cancel,
    /// The stop command during `Starting` (R201.4).
    Stop,
}

#[derive(Clone, Copy)]
struct Delegation {
    name: &'static str,
    caller: Caller,
    /// What the supervisor answers to the capture cancel.
    result: Result<bool, &'static str>,
    /// The supervisor emits its own terminal while the capture cancel is in flight.
    supervisor_first: bool,
    /// The ledger winner for generation 1, as the caller leaves it.
    terminal: Option<(DictationTerminalOutcome, DictationErrorCode)>,
    /// Site 206's recorded effect (`probe`-free: the executor's real result); `none` for
    /// both results now: R206.1 and R206.2 have no effect.
    witness: &'static str,
    /// What the command replies, as a trace line prefix.
    reply: &'static str,
}

const SUPERVISOR_WINS: (DictationTerminalOutcome, DictationErrorCode) = (
    DictationTerminalOutcome::UserCancelledStarting,
    DictationErrorCode::CancelledStarting,
);

fn delegations() -> Vec<Delegation> {
    let mut all = Vec::new();
    for caller in [Caller::Cancel, Caller::Stop] {
        let word = if caller == Caller::Cancel {
            "cancel"
        } else {
            "stop"
        };
        let name = |what: &'static str| -> &'static str {
            Box::leak(format!("{word}: {what}").into_boxed_str())
        };
        all.push(Delegation {
            name: name("the capture cancel fails, the command claims no terminal (R206.2)"),
            caller,
            result: Err("boom"),
            supervisor_first: false,
            terminal: None,
            witness: "none",
            reply: if caller == Caller::Cancel {
                "reply cancel Err boom"
            } else {
                "reply stop Err boom"
            },
        });
        all.push(Delegation {
            name: name("the supervisor claims first, its terminal stays the winner (R206.2)"),
            caller,
            result: Err("boom"),
            supervisor_first: true,
            terminal: Some(SUPERVISOR_WINS),
            witness: "none",
            reply: if caller == Caller::Cancel {
                "reply cancel Err boom"
            } else {
                "reply stop Err boom"
            },
        });
        all.push(Delegation {
            name: name("the capture cancel is not accepted (R206.1)"),
            caller,
            result: Ok(false),
            supervisor_first: false,
            terminal: None,
            witness: "none",
            reply: if caller == Caller::Cancel {
                "reply cancel Ok"
            } else {
                "reply stop Ok"
            },
        });
    }
    all
}

thread_local! {
    static DELEGATION: RefCell<Option<Delegation>> = const { RefCell::new(None) };
}

type Ops = Vec<(RecordingId, ContKind)>;

/// The outstanding operations seen while the capture cancel (`PT:CC`) was in flight.
static OPS_IN_CC: StdMutex<Option<Ops>> = StdMutex::new(None);

fn stash(slot: &StdMutex<Option<Ops>>, ops: Option<Ops>) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = ops;
}

fn stashed(slot: &StdMutex<Option<Ops>>, what: &str) -> Ops {
    slot.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap_or_else(|| panic!("{what} was never reached"))
}

/// `all` without one occurrence of each of `taken` (a multiset difference), and what of
/// `taken` was not in `all`.
fn without(all: &Ops, taken: &Ops) -> (Ops, Ops) {
    let mut rest = all.clone();
    let mut missing = Vec::new();
    for op in taken {
        match rest.iter().position(|candidate| candidate == op) {
            Some(at) => {
                rest.remove(at);
            }
            None => missing.push(*op),
        }
    }
    (rest, missing)
}

/// The body of a delegation fixture: a start in flight (`Starting`), a decoy continuation
/// another owner holds, then the caller's command with the supervisor's answer. Everything
/// the test asserts is read from the run (the ledger, the history, the real executor
/// witness), never from a value the test recorded for itself.
fn delegated_body(world: &Arc<World>) {
    let delegation = DELEGATION
        .with(|slot| *slot.borrow())
        .expect("the test set its delegation");
    world.knobs().cold_start = ColdStart::ReadyAfterContext;
    started(world);
    let state = world.state();
    let history = state.app_state.core_history.clone();
    // Another owner's operation: a command that consumed the wrong owner would retire it (or
    // leave its own behind).
    let decoy = ContinuationGuard::register(
        &history,
        ContinuationSpec {
            recording_id: RecordingId(99),
            kind: ContKind::CaptureCancel,
        },
    );
    let before = history.outstanding_for_test();
    assert!(
        before.contains(&(RecordingId(99), ContKind::CaptureCancel)),
        "the decoy is outstanding: {before:?}"
    );
    world.knobs().cancel_result = delegation.result;
    let claim = delegation
        .supervisor_first
        .then(|| super::cancel_tests::supervisor_claims_first(world));
    let during = history.clone();
    world.on("supervisor.cancel", move || {
        if let Some(claim) = claim {
            claim();
        }
        stash(&OPS_IN_CC, Some(during.outstanding_for_test()));
    });
    super::cancel_tests::forget_206_witness();
    match delegation.caller {
        Caller::Cancel => cancel_command(world, None),
        Caller::Stop => stop_command(world),
    }
    super::cancel_tests::note_206_witness(world);

    // The owner. While the capture cancel ran, the one new outstanding operation was the
    // command's own `K:CaptureCancel` for generation 1, and nothing that was outstanding had
    // gone (the owner is identified, not counted).
    let (new_in_cc, gone_in_cc) = without(&stashed(&OPS_IN_CC, "`PT:CC`"), &before);
    assert_eq!(
        new_in_cc,
        [(RecordingId(1), ContKind::CaptureCancel)],
        "{}: the operation the command registered during `PT:CC`",
        delegation.name
    );
    assert!(gone_in_cc.is_empty(), "{}: {gone_in_cc:?}", delegation.name);
    // Once the command returned: the same operations as before it, the decoy among them.
    assert_eq!(
        without(&history.outstanding_for_test(), &before),
        (Vec::new(), Vec::new()),
        "{}: site 206 consumed exactly the command's own operation",
        delegation.name
    );
    assert!(
        history.is_outstanding(decoy.token()),
        "{}: another owner's operation was consumed",
        delegation.name
    );
    // The ledger winner for the recording the command cancelled, and for no other.
    assert_eq!(
        terminal(world, 1),
        delegation.terminal,
        "{}: generation 1's terminal winner",
        delegation.name
    );
    for other in [0, 2, 99] {
        assert_eq!(
            terminal(world, other),
            None,
            "{}: a terminal was claimed for generation {other}",
            delegation.name
        );
    }
    drop(decoy);
    world.release_ready();
}

/// Runs one delegation fixture; its trace and the cancel entries it read.
fn run_delegation(delegation: Delegation) -> (Vec<String>, Vec<String>) {
    DELEGATION.with(|slot| *slot.borrow_mut() = Some(delegation));
    stash(&OPS_IN_CC, None);
    probe::take_cancel_entries();
    crate::commands::settings_store::test_data_dir::take_returned();
    COMMAND_PROJECTIONS.store(u64::MAX, AtomicOrdering::SeqCst);
    let trace = run_sub_run(&SubRun {
        title: delegation.name,
        setup: Setup::idle(),
        body: delegated_body,
    });
    DELEGATION.with(|slot| *slot.borrow_mut() = None);
    assert_no_data_dir(delegation.name);
    let cancel = names_of(cancel_labelled(probe::take_cancel_entries()));
    (trace, cancel)
}

#[test]
fn r206_preserves_the_delegated_result_and_consumes_its_owner_for_both_callers() {
    let _gate = serialized();
    for delegation in delegations() {
        let (trace, cancel) = run_delegation(delegation);
        let want_reply = delegation.reply;
        let command = if delegation.caller == Caller::Cancel {
            "reply cancel "
        } else {
            "reply stop "
        };
        let reply = trace
            .iter()
            .find(|line| line.starts_with(command))
            .unwrap_or_else(|| panic!("{}: the command replied: {trace:#?}", delegation.name));
        assert!(
            reply.starts_with(want_reply),
            "{}: the delegated result is the command's reply: {reply} (wanted {want_reply})",
            delegation.name
        );
        if delegation.caller == Caller::Stop && delegation.result == Ok(false) {
            assert!(
                reply.contains("recording_cancelled") && reply.contains("recovering"),
                "{}: R206.1's stop reply is `recording_cancelled` / `recovering`: {reply}",
                delegation.name
            );
        }
        assert!(
            trace.contains(&format!("# 206 witness: {}", delegation.witness)),
            "{}: site 206's real effect result: {trace:#?}",
            delegation.name
        );
        // The cancel entries that ran, exact. The cancel command crosses its own entry and
        // reaches `project()` not at all. The stop caller crosses none: 206 is D-less, and
        // the stop's own entry is the stop family's.
        match delegation.caller {
            Caller::Cancel => {
                assert_eq!(cancel, [E205], "{}", delegation.name);
                assert_eq!(
                    COMMAND_PROJECTIONS.load(AtomicOrdering::SeqCst),
                    0,
                    "{}: the cancel command reached `DictationCore::project`",
                    delegation.name
                );
            }
            Caller::Stop => assert!(cancel.is_empty(), "{}: {cancel:?}", delegation.name),
        }
    }
}

// ------------------------------------------------------ the shadow's role for the family

/// E3 and the verdict manifests for cancel stay consistent: R205.4 decides a reply another
/// site returns, so it closes as the announced no-reply close and is credited
/// `matched + delegated`; the receiving site's rows (206) carry the reply. Run through the
/// switched entry with the comparator armed (the sub-run's E7 asserts no flight diverged).
#[cfg(feature = "dictation-shadow")]
#[test]
fn the_shadow_credits_r205_4_as_delegated_through_the_switched_entry() {
    use crate::dictation_shadow::replay;
    let _gate = serialized();
    assert_eq!(
        replay::DELEGATED_ROWS,
        &[(201, 4), (205, 4)],
        "the delegated rows the E3 credit reads"
    );
    replay::reset_totals();
    let accepted = Case {
        name: "R205.4 recording, capture cancel accepted",
        setup: idle,
        body: |w| {
            recording(w);
            cancel_command(w, Some(1));
            w.release_cancel_finish();
        },
    };
    let failed = Case {
        name: "R205.4 + R206.2 recording, capture cancel fails",
        setup: idle,
        body: |w| {
            recording(w);
            w.knobs().cancel_result = Err("boom");
            cancel_command(w, None);
        },
    };
    let processing = Case {
        name: "R205.5 processing",
        setup: processing_3,
        body: |w| {
            w.install_context(3);
            cancel_command(w, None);
        },
    };
    for case in [&accepted, &failed, &processing] {
        run_case(case, false);
    }
    let totals = replay::totals();
    let credit = totals.e3_credit();
    // Two R205.4 flights ran: delegated, never `matched`, and credited as delegated.
    assert_eq!(totals.row_delegated.get(&(205, 4)), Some(&2), "{totals:?}");
    assert_eq!(
        totals.row_matched.get(&(205, 4)).copied().unwrap_or(0),
        0,
        "{totals:?}"
    );
    assert_eq!(credit.get(&(205, 4)), Some(&2), "{credit:?}");
    // The receiving site's rows carry the reply: 206.1 (accepted) and 206.2 (failed), and
    // the processing cancel's own R205.5.
    assert_eq!(totals.row_matched.get(&(206, 1)), Some(&1), "{totals:?}");
    assert_eq!(totals.row_matched.get(&(206, 2)), Some(&1), "{totals:?}");
    assert_eq!(totals.row_matched.get(&(205, 5)), Some(&1), "{totals:?}");
    // No flight diverged. The two abandoned flights are the two R205.4 flights themselves:
    // each closes as the announced no-reply close (its reply is site 206's), which the
    // comparator counts as abandoned and credits as delegated above, identically before and
    // after the switch.
    assert_eq!(totals.abandoned, 2, "{totals:?}");
    assert_eq!(totals.mismatched_root, 0, "{totals:?}");
    assert_eq!(totals.unverifiable, 0, "{totals:?}");
}
