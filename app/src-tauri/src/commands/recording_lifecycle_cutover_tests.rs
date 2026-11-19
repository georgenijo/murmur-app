//! The lifecycle cutover (Stage 2c, appendix C9.7 and C8.6): the six D entries of the
//! supervisor-lifecycle family (sites 207-211 and 216) decide on the **persisted core**
//! (M1 `status`, M2 `current_id`, `DictationState.core`), not on `DictationCore::project`.
//!
//! What each test proves, and why the golden invariant alone is not enough (the council's
//! point: `assert_persisted_core` passes while an entry still reads `project()`):
//!
//! * `no_lifecycle_entry_projects` - none of the six entries calls the `project()` entry
//!   (`FieldAdapter::project_lifecycle_view`), by call site at run time and by source, and
//!   each of them reads the persisted core exactly once.
//! * `every_entry_reads_the_persisted_fields_and_resyncs_a_divergence` - the evidence is
//!   captured **at the entry**, from the actual persisted fields, before any adapter write
//!   could repair them: a core that differs from live is read as planted, counted once
//!   (`core_divergences`, C8.6), re-synced before the decision, and the entry then decides
//!   exactly as it does for an equal core. A `project()` entry never reads the planted
//!   fields, so it records no divergence and fails this test.
//! * `an_equal_core_is_never_counted` - the fixtures the family's shell tests use run on
//!   an equal core and count nothing.
//! * `the_owned_auto_stop_runs_through_the_switched_entries` - #1170's owned interruption
//!   auto-stop, with the lifecycle entries around it switched.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, sub_from};
use super::lifecycle_tests::{run_with_core, CoreSeed};
use super::*;
use crate::dictation_executor::adapter::core_divergences;
use crate::dictation_executor::probe;
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;

const RID: u64 = 7;

type EventMaker = fn() -> AudioLifecycleEvent;

/// The six D entries, each with a fixture whose decision depends on **both** persisted
/// fields (the status, and the id: a non-current id is superseded), so a decision taken on
/// the planted core would differ from the one taken on live.
fn d_entries() -> Vec<(&'static str, EventMaker, DictationStatus)> {
    vec![
        (
            "207 Ready",
            || AudioLifecycleEvent::Ready,
            DictationStatus::Starting,
        ),
        (
            "208 StillConnecting",
            || AudioLifecycleEvent::StillConnecting,
            DictationStatus::Starting,
        ),
        (
            "209 Recovering",
            || AudioLifecycleEvent::Recovering {
                reason: AudioCancelReason::RuntimeFailure,
            },
            DictationStatus::Recording,
        ),
        (
            "210 CleanupFinished",
            || AudioLifecycleEvent::CleanupFinished,
            DictationStatus::Recovering,
        ),
        (
            "211 Idle",
            || AudioLifecycleEvent::Idle,
            DictationStatus::Recording,
        ),
        (
            "216 Interrupted (short)",
            || AudioLifecycleEvent::Interrupted {
                reason: AudioCancelReason::RuntimeFailure,
                delivered_samples: 100,
                duration_ms: 6,
            },
            DictationStatus::Recording,
        ),
    ]
}

/// The span of `handle_audio_lifecycle_events` in `recording.rs`, as `(first, last)` line.
fn bridge_span(source: &str) -> (usize, usize) {
    use syn::spanned::Spanned;
    let file = syn::parse_file(source).expect("recording.rs parses");
    let function = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "handle_audio_lifecycle_events" => {
                Some(function)
            }
            _ => None,
        })
        .expect("the lifecycle bridge is in recording.rs");
    (function.span().start().line, function.span().end().line)
}

fn recording_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/recording.rs");
    std::fs::read_to_string(path).expect("recording.rs")
}

/// Tests share the process-global probes; one cutover test at a time. The entry reads are
/// remembered only while the guard lives (remembering allocates, which the allocation pins
/// under D must not see), and are cleared when it starts and ends.
struct Probes(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

impl Drop for Probes {
    fn drop(&mut self) {
        probe::record_lifecycle_entries(false);
        probe::record_projections(false);
        probe::take_lifecycle_entries();
        probe::take_projection_sites();
    }
}

fn serialized() -> Probes {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let gate = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe::take_lifecycle_entries();
    probe::take_projection_sites();
    probe::record_lifecycle_entries(true);
    Probes(gate)
}

#[test]
fn no_lifecycle_entry_projects() {
    let _gate = serialized();
    let source = recording_source();
    let (first, last) = bridge_span(&source);

    // By source: the bridge never names the `project()` entry, and names the persisted one
    // once per D entry.
    let body: String = source
        .lines()
        .skip(first - 1)
        .take(last - first + 1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !body.contains("project_lifecycle_view") && !body.contains("DictationCore::project"),
        "a lifecycle entry still builds its view with `project()`"
    );
    assert_eq!(
        body.matches("FieldAdapter::lifecycle_view(").count(),
        6,
        "each of the six D entries reads the persisted core, once"
    );

    // At run time: every entry, run through the production bridge, reads the persisted
    // core once and `project()` is not reached from the bridge.
    probe::take_projection_sites();
    probe::take_lifecycle_entries();
    probe::record_projections(true);
    for (name, event, status) in d_entries() {
        let before = probe::projections();
        run_with_core(event(), status, RID, true, CoreSeed::Live);
        let reads = probe::take_lifecycle_entries();
        assert_eq!(
            reads.len(),
            1,
            "{name}: the entry read the persisted core exactly once"
        );
        let from_the_bridge: Vec<_> = probe::take_projection_sites()
            .into_iter()
            .filter(|(file, line)| {
                file.ends_with("commands/recording.rs")
                    && (first..=last).contains(&(*line as usize))
            })
            .collect();
        assert!(
            from_the_bridge.is_empty(),
            "{name}: `DictationCore::project` was called from the lifecycle bridge at {from_the_bridge:?}"
        );
        // Other callers (the ticker's `register` query) are other families' reads and
        // stay on `project()`; this entry added none of its own.
        let _ = before;
    }
    probe::record_projections(false);
}

/// The persisted fields planted by [`CoreSeed::Diverged`]: the status other than the
/// fixture's, and the id one past it.
fn planted(status: DictationStatus) -> (Status, RecordingId) {
    let other = if status == DictationStatus::Recording {
        Status::Processing
    } else {
        Status::Recording
    };
    (other, RecordingId(RID + 1))
}

#[test]
fn every_entry_reads_the_persisted_fields_and_resyncs_a_divergence() {
    let _gate = serialized();
    for (name, event, status) in d_entries() {
        // The reference: the same event on an equal core.
        let equal_before = core_divergences();
        let reference = run_with_core(event(), status, RID, true, CoreSeed::Live);
        assert_eq!(
            core_divergences(),
            equal_before,
            "{name}: an equal core is not a divergence"
        );

        probe::take_lifecycle_entries();
        let before = core_divergences();
        let outcome = run_with_core(event(), status, RID, true, CoreSeed::Diverged);
        let reads = probe::take_lifecycle_entries();

        // Captured at the entry, from the actual persisted fields: the planted core, and
        // live as the fixture set it.
        assert_eq!(reads.len(), 1, "{name}: one persisted read");
        let (persisted, live) = reads[0];
        assert_eq!(
            persisted,
            planted(status),
            "{name}: the entry read the persisted fields as planted, before any repair"
        );
        assert_eq!(
            live,
            (
                crate::dictation_admission::core_status(status),
                RecordingId(RID)
            ),
            "{name}: and live as the fixture set it"
        );
        // Counted once (C8.6) ...
        assert_eq!(
            core_divergences(),
            before + 1,
            "{name}: the divergence was counted exactly once"
        );
        // ... and re-synced before the decision: the entry decides exactly as it does for
        // an equal core (and `run_with_core` asserts the core equals live afterwards).
        assert_eq!(
            outcome, reference,
            "{name}: the divergence was re-synced before the decision, not decided on"
        );
    }
}

#[test]
fn an_equal_core_is_never_counted() {
    let _gate = serialized();
    let before = core_divergences();
    for (name, event, status) in d_entries() {
        for current in [RID, RID + 1] {
            run_with_core(event(), status, current, true, CoreSeed::Live);
        }
        assert_eq!(
            core_divergences(),
            before,
            "{name}: fixtures on an equal core count nothing"
        );
    }
}

/// #1170's owned interruption auto-stop with the lifecycle entries around it switched: the
/// start's `Ready` (207) publishes, the runtime failure's `Recovering` (209) and the
/// `Interrupted` (215.2, D-less) schedule the auto-stop, whose stop is admitted and
/// consumes its owner, and the trailing supervisor `Idle` (211) and `CleanupFinished`
/// (210) then run through the persisted core. Every D entry reads the persisted fields
/// equal to live (nothing repaired them: a divergence would be counted), and none of them
/// projects.
fn auto_stop_through_the_switched_entries(w: &Arc<World>) {
    use murmur_dictation_core::decision::ContKind;
    let divergences = core_divergences();
    probe::take_lifecycle_entries();
    w.knobs().carry_auto_stop_owner = true;
    recording(w);
    w.runtime_failure(1);
    w.finish_interrupted(1, 9_000);
    assert_eq!(
        w.auto_stop_requests(),
        vec![1],
        "the bridge scheduled the auto-stop"
    );
    let (token, mut stop) = w.owned_auto_stop(1);
    assert_eq!(token.recording_id, RecordingId(1));
    assert_eq!(token.kind, ContKind::AutoStop);
    // The stop runs through its own family (not switched here): it takes T, consumes the
    // owner at admission, and parks in the pipeline until the gate opens.
    w.close_gate("pipeline");
    assert!(poll_once(&mut stop).is_pending());
    assert!(
        !w.state().app_state.core_history.is_outstanding(token),
        "the stop consumed the owner at admission"
    );
    w.open_gate("pipeline");
    let reply = complete(&mut stop);
    w.reply("auto-stop", &reply);
    assert!(reply.is_ok(), "{reply:?}");
    w.release_owner_clear();
    // The supervisor's trailing events for the attempt, through the switched entries.
    w.notify(1, AudioLifecycleEvent::CleanupFinished);
    w.notify(1, AudioLifecycleEvent::Idle);

    let reads = probe::take_lifecycle_entries();
    assert!(
        reads.len() >= 3,
        "Ready, Recovering, CleanupFinished and Idle are D entries: {reads:?}"
    );
    for (persisted, live) in &reads {
        assert_eq!(
            persisted, live,
            "every switched entry read persisted == live: nothing had to repair it"
        );
    }
    assert_eq!(
        core_divergences(),
        divergences,
        "the whole auto-stop run counted no divergence"
    );
}

#[test]
fn the_owned_auto_stop_runs_through_the_switched_entries() {
    let _gate = serialized();
    super::pd_witness_tests::repeat(&sub_from(
        "cutover: the owned auto-stop through the switched lifecycle entries",
        DictationStatus::Idle,
        0,
        auto_stop_through_the_switched_entries,
    ));
}
