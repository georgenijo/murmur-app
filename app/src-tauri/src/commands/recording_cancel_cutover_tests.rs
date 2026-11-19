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
use super::pd25_tests::terminal;
use super::stop_cutover_tests::{fn_spans, lines_with, recording_source, Pair};
use super::*;
use crate::core_history::ContinuationGuard;
use crate::dictation_executor::adapter::core_divergences;
use crate::dictation_executor::probe;
use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
use murmur_dictation_core::ids::RecordingId;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Mutex as StdMutex;

const E205: &str = "cancel_native_recording_inner#0";
const STOP_E201: &str = "stop_native_recording_admitted#0";

// ------------------------------------------------------------------------ probes

/// Tests share the process-global probes; one cutover test at a time. The stop family's
/// gate is the one gate (a stop-caller fixture reads the stop entries too), so this module
/// and the stop cutover's never interleave. The cancel entries are remembered only while
/// the guard lives (remembering allocates, which the allocation pins under D must not see).
struct Probes {
    _stop: super::stop_cutover_tests::Probes,
}

impl Drop for Probes {
    fn drop(&mut self) {
        probe::record_cancel_entries(false);
        probe::take_cancel_entries();
        probe::record_projections(false);
        probe::take_projection_sites();
    }
}

fn serialized() -> Probes {
    let stop = super::stop_cutover_tests::serialized();
    probe::take_cancel_entries();
    probe::record_cancel_entries(true);
    Probes { _stop: stop }
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

fn cancel_labelled(reads: Vec<probe::StopEntryRead>) -> Vec<(String, Pair, Pair)> {
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
/// `take_projection_sites` were made in.
fn projecting_functions() -> Vec<String> {
    let source = recording_source();
    let spans = fn_spans(&source);
    probe::take_projection_sites()
        .into_iter()
        .map(|(file, line)| {
            assert!(
                file.ends_with("commands/recording.rs"),
                "a projection outside recording.rs: {file}:{line}"
            );
            enclosing(&spans, line as usize)
        })
        .collect()
}

// ------------------------------------------------------------------------ the plants

thread_local! {
    static PLANT: RefCell<bool> = const { RefCell::new(false) };
}

/// The `(planted, live)` pair the plant wrote.
static PLANTED: StdMutex<Option<(Pair, Pair)>> = StdMutex::new(None);

/// `DictationCore::project` entries made while the command under test ran.
static COMMAND_PROJECTIONS: AtomicU64 = AtomicU64::new(0);

/// Plants a divergence (the persisted core put off live) when the run asks for one: right
/// before the command, so the entry is the first read after it.
fn arm(world: &Arc<World>) {
    if PLANT.with(|plant| *plant.borrow()) {
        let pair = world.plant_core_divergence();
        *PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pair);
    }
}

/// The cancel command, announced by a marker line, with the `project()` entries it made
/// counted.
fn cancel_command(world: &Arc<World>, expected: Option<u64>) {
    arm(world);
    world.note("cancel requested");
    let before = probe::projections();
    let mut cancel = world.cancel(expected);
    let reply = complete(&mut cancel);
    COMMAND_PROJECTIONS.store(probe::projections() - before, AtomicOrdering::SeqCst);
    world.reply_unit("cancel", &reply);
}

/// The stop command (the other caller of site 206), counted the same way.
fn stop_command(world: &Arc<World>) {
    arm(world);
    world.note("stop requested");
    let before = probe::projections();
    stopped(world, "stop", None);
    COMMAND_PROJECTIONS.store(probe::projections() - before, AtomicOrdering::SeqCst);
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
    COMMAND_PROJECTIONS.store(u64::MAX, AtomicOrdering::SeqCst);
    let divergences = core_divergences();
    let trace = run_sub_run(&SubRun {
        title: case.name,
        setup: (case.setup)(),
        body: case.body,
    });
    PLANT.with(|slot| *slot.borrow_mut() = false);
    Run {
        entries: cancel_labelled(probe::take_cancel_entries()),
        projections: COMMAND_PROJECTIONS.load(AtomicOrdering::SeqCst),
        divergences: core_divergences() - divergences,
        trace,
    }
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

/// #1170's exact tail-order test (R205.5: `In:ST→Idle`; `PDT:CX-, MC, TERM, KB, EM, EV`;
/// `PT:PC`, T held from `ST` through `EV` and released only before `PC`), run again through
/// the switched entry. The entries that ran are exact: the cancel's one D entry, once.
#[test]
fn the_r205_5_tail_order_holds_through_the_switched_entry() {
    let _gate = serialized();
    probe::record_projections(true);
    probe::take_projection_sites();
    let divergences = core_divergences();
    super::cancel_tests::processing_cancel_tail_runs_in_r205_5_order();
    assert_eq!(
        names_of(cancel_labelled(probe::take_cancel_entries())),
        [E205],
        "the switched entries that ran"
    );
    assert!(
        !projecting_functions()
            .iter()
            .any(|function| function == "cancel_native_recording_inner"),
        "the cancel projected"
    );
    assert_eq!(
        core_divergences(),
        divergences,
        "an equal core is never counted"
    );
}

/// #1170's independent-start barrier (a processing cancel keeps T through its trailing
/// `idle`, so a newer start cannot publish `starting` and then be overwritten by the old
/// `idle`), run again through the switched entry. The cancel's entries are exact; the start
/// that is polled from an independent task is the start family's and still projects, which
/// the sites prove is not the cancel.
#[test]
fn the_independent_start_barrier_holds_through_the_switched_entry() {
    let _gate = serialized();
    probe::record_projections(true);
    probe::take_projection_sites();
    super::cancel_tests::a_new_start_stays_blocked_until_the_trailing_idle();
    assert_eq!(
        names_of(cancel_labelled(probe::take_cancel_entries())),
        [E205],
        "the switched entries that ran"
    );
    let projecting = projecting_functions();
    assert!(
        !projecting.is_empty(),
        "the start ran, and the start family still projects (the probe sees it)"
    );
    assert!(
        !projecting
            .iter()
            .any(|function| function == "cancel_native_recording_inner"),
        "the cancel projected: {projecting:?}"
    );
}

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
    /// Site 206's recorded effect (`probe`-free: the executor's real result).
    witness: &'static str,
    /// What the command replies, as a trace line prefix.
    reply: &'static str,
}

const PIPELINE_FAILURE: (DictationTerminalOutcome, DictationErrorCode) = (
    DictationTerminalOutcome::PipelineFailure,
    DictationErrorCode::RuntimeFailure,
);
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
            name: name("the capture cancel fails, the command's claim wins (R206.2)"),
            caller,
            result: Err("boom"),
            supervisor_first: false,
            terminal: Some(PIPELINE_FAILURE),
            witness: "TERM Ran",
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
            witness: "TERM Failed",
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

/// Outstanding history operations seen while the capture cancel (`PT:CC`) was in flight.
static OUTSTANDING_IN_CC: AtomicUsize = AtomicUsize::new(usize::MAX);

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
    let before = history.outstanding_ops();
    world.knobs().cancel_result = delegation.result;
    let claim = delegation
        .supervisor_first
        .then(|| super::cancel_tests::supervisor_claims_first(world));
    let during = history.clone();
    world.on("supervisor.cancel", move || {
        if let Some(claim) = claim {
            claim();
        }
        OUTSTANDING_IN_CC.store(during.outstanding_ops(), AtomicOrdering::SeqCst);
    });
    super::cancel_tests::forget_206_witness();
    match delegation.caller {
        Caller::Cancel => cancel_command(world, None),
        Caller::Stop => stop_command(world),
    }
    super::cancel_tests::note_206_witness(world);

    // The owner: the operation the command registered for its own capture cancel was
    // outstanding while the capture cancel ran, and was consumed once site 206 had decided;
    // the other owner's was neither.
    assert_eq!(
        OUTSTANDING_IN_CC.load(AtomicOrdering::SeqCst),
        before + 1,
        "{}: the command's own `K:CaptureCancel` was outstanding during `PT:CC`",
        delegation.name
    );
    assert_eq!(
        history.outstanding_ops(),
        before,
        "{}: site 206 consumed the command's own operation",
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

fn run_delegation(delegation: Delegation) -> (Vec<String>, Vec<String>, Vec<String>) {
    DELEGATION.with(|slot| *slot.borrow_mut() = Some(delegation));
    OUTSTANDING_IN_CC.store(usize::MAX, AtomicOrdering::SeqCst);
    probe::take_cancel_entries();
    COMMAND_PROJECTIONS.store(u64::MAX, AtomicOrdering::SeqCst);
    let trace = run_sub_run(&SubRun {
        title: delegation.name,
        setup: Setup::idle(),
        body: delegated_body,
    });
    DELEGATION.with(|slot| *slot.borrow_mut() = None);
    let cancel = names_of(cancel_labelled(probe::take_cancel_entries()));
    let stop = names_of(super::stop_cutover_tests::labelled(
        probe::take_stop_entries(),
    ));
    (trace, cancel, stop)
}

#[test]
fn r206_preserves_the_delegated_result_and_consumes_its_owner_for_both_callers() {
    let _gate = serialized();
    for delegation in delegations() {
        // Take what the setup (the start) read, so only the command's entries remain.
        let (trace, cancel, stop) = run_delegation(delegation);
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
        // The entries that ran, exact. The cancel command crosses its own entry; the stop
        // caller crosses the stop family's 201 and no cancel entry (206 is D-less).
        match delegation.caller {
            Caller::Cancel => {
                assert_eq!(cancel, [E205], "{}", delegation.name);
                assert!(stop.is_empty(), "{}: {stop:?}", delegation.name);
            }
            Caller::Stop => {
                assert!(cancel.is_empty(), "{}: {cancel:?}", delegation.name);
                assert_eq!(stop, [STOP_E201], "{}", delegation.name);
            }
        }
        assert_eq!(
            COMMAND_PROJECTIONS.load(AtomicOrdering::SeqCst),
            0,
            "{}: the command reached `DictationCore::project`",
            delegation.name
        );
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
    assert_eq!(totals.row_matched.get(&(205, 4)), None, "{totals:?}");
    assert_eq!(credit.get(&(205, 4)), Some(&2), "{credit:?}");
    // The receiving site's rows carry the reply: 206.1 (accepted) and 206.2 (failed), and
    // the processing cancel's own R205.5.
    assert_eq!(totals.row_matched.get(&(206, 1)), Some(&1), "{totals:?}");
    assert_eq!(totals.row_matched.get(&(206, 2)), Some(&1), "{totals:?}");
    assert_eq!(totals.row_matched.get(&(205, 5)), Some(&1), "{totals:?}");
    assert_eq!(totals.mismatched_root, 0, "{totals:?}");
    assert_eq!(totals.abandoned, 0, "{totals:?}");
}
