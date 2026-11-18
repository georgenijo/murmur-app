//! The property suite (C7.5): 10,000 fixed seeds, the per-PD converse witness, the
//! PD-18 instance seeds, the PD-19..PD-22 witness seeds, determinism, and unit
//! tests of the checker itself (a checker that cannot fail proves nothing).

use super::checks::{Checker, SIGNATURES};
use super::model::*;
use super::run::{run_seed, run_seed_strict, run_seed_with, RunReport};
use super::task::*;
use crate::decision::{SiteId, Verdict};
use crate::effects::{Effect, PerfOutcome, Placement};
use crate::status::Status;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// CI runs this many seeds; `PROPTEST_CASES` raises (or lowers) it locally.
const DEFAULT_SEEDS: u64 = 10_000;
const CI_BUDGET: Duration = Duration::from_secs(20);

/// PD-18's witnesses, one seed per instance (C7.5): the first seed of the fixed
/// range that ends stuck in `Recovering` through that instance.
const PD18_SEEDS: [(&str, u64); 3] = [
    ("(a) S12", 796),
    ("(b) Idle-arm tail", 59),
    ("(c) unsealed stop", 2318),
];

/// PD-19..PD-22's witnesses (D-S2b-5): the first seed of the fixed range that
/// takes each row, and the shell test that reproduces it
/// (`recording_harness_findings_tests.rs`).
const FINDING_SEEDS: [(u8, u64, &str); 4] = [
    (
        19,
        633,
        "f4_interrupted_over_8000_with_a_refused_auto_stop_leaves_recovering",
    ),
    (
        20,
        470,
        "f1_cancel_terminal_and_a_short_stop_performance_outcome_disagree",
    ),
    (
        21,
        42,
        "f2_failed_capture_cancel_terminal_and_a_short_stop_performance_outcome_disagree",
    ),
    (
        22,
        331,
        "f3_superseded_terminal_and_a_cancels_performance_outcome_disagree",
    ),
];

fn seeds() -> u64 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SEEDS)
}

#[derive(Default)]
struct Summary {
    seeds: u64,
    elapsed: Duration,
    failing: Vec<(u64, Vec<String>)>,
    pd: Vec<(u32, Option<u64>)>,
    pd18: Vec<(u32, Option<u64>)>,
    sig: Vec<(u32, Option<u64>)>,
    liveness_checked: u64,
    liveness_filtered: u64,
    steps: u64,
}

fn summary() -> &'static Summary {
    static SUMMARY: OnceLock<Summary> = OnceLock::new();
    SUMMARY.get_or_init(|| {
        let n = seeds();
        let mut summary = Summary {
            seeds: n,
            pd: vec![(0, None); 23],
            pd18: vec![(0, None); 3],
            sig: vec![(0, None); SIGNATURES.len()],
            ..Summary::default()
        };
        let started = Instant::now();
        for seed in 0..n {
            let report = run_seed(seed);
            summary.steps += u64::from(report.steps);
            if report.filtered {
                summary.liveness_filtered += 1;
            } else {
                summary.liveness_checked += 1;
            }
            for (pd, count) in report.hits.pd.iter().enumerate() {
                if *count > 0 {
                    summary.pd[pd].0 += count;
                    summary.pd[pd].1.get_or_insert(seed);
                }
            }
            for (instance, count) in report.hits.pd18.iter().enumerate() {
                if *count > 0 {
                    summary.pd18[instance].0 += count;
                    summary.pd18[instance].1.get_or_insert(seed);
                }
            }
            for (row, count) in report.hits.sig.iter().enumerate() {
                if *count > 0 {
                    summary.sig[row].0 += count;
                    summary.sig[row].1.get_or_insert(seed);
                }
            }
            if !report.violations.is_empty() {
                summary.failing.push((seed, report.violations));
            }
        }
        summary.elapsed = started.elapsed();
        eprintln!(
            "property: {} seeds, {} steps, {:?}; liveness checked on {} runs ({} filtered by L4/L6)",
            summary.seeds,
            summary.steps,
            summary.elapsed,
            summary.liveness_checked,
            summary.liveness_filtered
        );
        for pd in 1..=22 {
            eprintln!(
                "property: PD-{pd}: {} occurrences, first at seed {:?}",
                summary.pd[pd].0, summary.pd[pd].1
            );
        }
        eprintln!(
            "property: PD-18 stuck runs (a, b, c): {:?}",
            summary.pd18
        );
        for (row, (pd, terminal, perf)) in SIGNATURES.iter().enumerate() {
            eprintln!(
                "property: PD-{pd} {terminal} -> {perf}: {} occurrences, first at seed {:?}",
                summary.sig[row].0, summary.sig[row].1
            );
        }
        summary
    })
}

/// SP1-SP10 and C7.3 liveness hold on every prefix of every seed, apart from the
/// C7.1 allowlist. The first failing seeds are printed with the command that traces them.
#[test]
fn seeds_hold_the_properties() {
    let summary = summary();
    if let Some((seed, violations)) = summary.failing.first() {
        let shown: Vec<_> = summary.failing.iter().take(5).collect();
        panic!(
            "{} of {} seeds violate a property outside the allowlist; first: seed {seed}: {violations:?}\n\
             first five: {shown:?}\n\
             trace one with: TRACE_SEED={seed} cargo test -p murmur-dictation-core --lib property::tests::trace_one -- --nocapture",
            summary.failing.len(),
            summary.seeds
        );
    }
}

/// C7.5 budget: 10,000 seeds in under 20 s of the 60 s suite limit.
#[test]
fn ten_thousand_seeds_fit_the_ci_budget() {
    let summary = summary();
    if summary.seeds == DEFAULT_SEEDS {
        assert!(
            summary.elapsed < CI_BUDGET,
            "{} seeds took {:?} (budget {CI_BUDGET:?})",
            summary.seeds,
            summary.elapsed
        );
    }
}

/// The converse (C7.0, C7.5): every PD signature occurs at least once in the
/// seeds, so an allowlist entry cannot rot, and so does every row of the PD-20..22
/// signature table. PD-17 is latent (no production schedule) and has no witness;
/// PD-18 has one per instance (a later test).
#[test]
fn every_preserved_defect_is_witnessed() {
    let summary = summary();
    if summary.seeds < DEFAULT_SEEDS {
        return; // a short local run does not owe every witness
    }
    let missing: Vec<usize> = (1..=22)
        .filter(|pd| *pd != 17 && summary.pd[*pd].0 == 0)
        .collect();
    assert!(missing.is_empty(), "PDs never witnessed: {missing:?}");
    let stale: Vec<_> = SIGNATURES
        .iter()
        .zip(&summary.sig)
        .filter(|(_, seen)| seen.0 == 0)
        .map(|(signature, _)| signature)
        .collect();
    assert!(
        stale.is_empty(),
        "signature rows never witnessed: {stale:?}"
    );
    assert_eq!(
        summary.pd[17].0, 0,
        "PD-17 is latent: no schedule reaches it"
    );
}

/// PD-18's three instances each end a run stuck in `Recovering` (liveness failure
/// by the exempt signature). One pinned seed per instance.
#[test]
fn pd18_has_one_seed_per_instance() {
    for (index, (name, seed)) in PD18_SEEDS.iter().enumerate() {
        let report = run_seed(*seed);
        assert!(
            report.hits.pd18[index] > 0,
            "seed {seed} no longer ends stuck through PD-18 {name}: {:?}",
            report.hits
        );
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }
}

/// PD-19..PD-22 each have a pinned seed, and the shell test that reproduces the
/// behaviour is named beside it (D-S2b-5). There is no "known findings" set: a
/// behaviour outside C7.1 fails `seeds_hold_the_properties`.
#[test]
fn pd19_to_pd22_have_one_seed_each() {
    for (pd, seed, shell_test) in FINDING_SEEDS {
        let report = run_seed(seed);
        assert!(
            report.hits.pd[usize::from(pd)] > 0,
            "seed {seed} no longer takes PD-{pd} (shell witness {shell_test}): {:?}",
            report.hits
        );
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }
}

/// The signature table is the allowlist for SP4(b): a terminal and a performance
/// outcome that disagree through any other pair of rows fail the seed.
#[test]
fn an_sp4b_split_outside_the_signature_table_fails() {
    use crate::status::{ErrCode, TerminalOutcome};
    for (terminal_row, perf_row, listed) in [
        ("R205.5", "R203.7", true),
        ("R205.5", "R999.1", false),
        ("R999.1", "R203.7", false),
    ] {
        let mut w = World::new();
        w.ledger.accepted.insert(1);
        w.ledger.claimed.insert(
            1,
            (
                TerminalOutcome::UserCancelledProcessing,
                ErrCode::None,
                terminal_row,
            ),
        );
        w.perf.outcome.insert(1, (PerfOutcome::NoSpeech, perf_row));
        let mut checker = Checker::default();
        checker.on_end(&w, &[]);
        let failed = checker.violations.iter().any(|v| v.starts_with("SP4(b)"));
        assert_eq!(
            failed, !listed,
            "{terminal_row} / {perf_row}: {:?}",
            checker.violations
        );
        assert_eq!(checker.hits.pd[20] > 0, listed);
    }
}

#[test]
fn a_seed_is_one_run() {
    for seed in [0u64, 59, 886, 4903, 9_999] {
        let (first, first_trace) = run_seed_with(seed, true);
        let (second, second_trace) = run_seed_with(seed, true);
        assert_eq!(
            first_trace, second_trace,
            "seed {seed} is not deterministic"
        );
        assert_eq!(first.steps, second.steps);
        assert_eq!(first.violations, second.violations);
    }
}

#[test]
fn trace_one() {
    let Ok(seed) = std::env::var("TRACE_SEED") else {
        return;
    };
    let seed: u64 = seed.parse().expect("TRACE_SEED is a number");
    let (report, lines): (RunReport, Vec<String>) = run_seed_with(seed, true);
    for line in lines {
        eprintln!("{line}");
    }
    eprintln!("{report:?}");
}

// ------------------------------------------------------ the checker's own tests

fn effect(
    row: &'static str,
    effect: Effect,
    input_rid: Rid,
    current: Rid,
    from: Status,
) -> EffectEv {
    let after = match effect {
        Effect::SetStatus(to) => to,
        _ => from,
    };
    EffectEv {
        tid: 2,
        row,
        site: SiteId::SupIdle,
        at: Placement::In,
        effect,
        input_rid,
        current,
        status_before: from,
        status_after: after,
        claim: None,
        accepted: true,
        step: 1,
    }
}

fn decision(row: &'static str, input_rid: Rid, current: Rid) -> DecisionEv {
    DecisionEv {
        tid: 2,
        site: SiteId::SupIdle,
        row,
        verdict: Verdict::Applied,
        input_rid,
        origin: None,
        current_before: current,
        status_before: Status::Idle,
        cleanup_pending: false,
        restorer_pending: false,
        transition: None,
        in_cancel_tail: false,
        pipeline_live: false,
        owner: None,
        ended_by: None,
        phase: Phase::Idle,
        step: 1,
    }
}

fn check(out: StepOut) -> Checker {
    let mut checker = Checker::default();
    checker.on_step(&World::new(), &out);
    checker
}

#[test]
fn sp2_fails_on_a_stale_status_write_and_only_a_matching_pd_row_exempts_it() {
    // A stale `ST` from a row that is not a PD row: SP2.
    let mut out = StepOut::default();
    out.effects.push(effect(
        "R211.3",
        Effect::SetStatus(Status::Idle),
        1,
        2,
        Status::Processing,
    ));
    let checker = check(out);
    assert!(
        checker.violations.iter().any(|v| v.starts_with("SP2")),
        "{:?}",
        checker.violations
    );

    // The PD-1 row R216.5 writes `Idle` over a newer recording: allowlisted.
    let mut out = StepOut::default();
    out.decisions.push(decision("R216.5", 1, 2));
    out.effects.push(effect(
        "R216.5",
        Effect::SetStatus(Status::Idle),
        1,
        2,
        Status::Recording,
    ));
    assert!(check(out).violations.is_empty());

    // The same write from a different row, in the same step as a PD-1 hit: not exempt.
    let mut out = StepOut::default();
    out.decisions.push(decision("R216.5", 1, 2));
    out.effects.push(effect(
        "R101.15",
        Effect::SetStatus(Status::Idle),
        1,
        2,
        Status::Recording,
    ));
    let checker = check(out);
    assert!(
        checker.violations.iter().any(|v| v.starts_with("SP2")),
        "{:?}",
        checker.violations
    );
}

#[test]
fn sp3_fails_on_an_edge_outside_the_table() {
    for (from, to) in [
        (Status::Starting, Status::Processing),
        (Status::Recording, Status::Starting),
        (Status::Processing, Status::Recording),
        (Status::Recovering, Status::Starting),
    ] {
        let mut out = StepOut::default();
        out.effects
            .push(effect("R999.1", Effect::SetStatus(to), 2, 2, from));
        let checker = check(out);
        assert!(
            checker.violations.iter().any(|v| v.starts_with("SP3")),
            "{from:?} -> {to:?}: {:?}",
            checker.violations
        );
    }
    for (from, to) in [
        (Status::Idle, Status::Recovering),
        (Status::Idle, Status::Idle),
        (Status::Recovering, Status::Processing),
    ] {
        let mut out = StepOut::default();
        out.effects
            .push(effect("R101.12", Effect::SetStatus(to), 2, 2, from));
        assert!(check(out).violations.is_empty(), "{from:?} -> {to:?}");
    }
}

#[test]
fn a_step_violation_is_a_failure() {
    let mut out = StepOut::default();
    out.violations
        .push("SP7: R104.2 differs by op status".into());
    assert_eq!(check(out).violations.len(), 1);
}

#[test]
fn liveness_fails_on_a_stuck_status_an_open_op_or_a_held_lock() {
    let mut w = World::new();
    w.status = Status::Processing;
    let mut checker = Checker::default();
    assert!(!checker.on_end(&w, &[]));
    assert!(checker
        .violations
        .iter()
        .any(|v| v.contains("status Processing")));

    let mut w = World::new();
    w.t_holder = Some(4);
    w.history.pipelines.insert(1);
    let mut checker = Checker::default();
    checker.on_end(&w, &["start"]);
    let all = checker.violations.join(" | ");
    assert!(all.contains("T is still held"), "{all}");
    assert!(all.contains("pipelines unresolved"), "{all}");
    assert!(all.contains("tasks never finished"), "{all}");

    // A recording nobody stops is outside L4: the run is filtered, not failed.
    let mut w = World::new();
    w.sup.attempt = Some({
        let mut attempt = Attempt::new(
            1,
            AttemptPlan {
                kind: StartKind::Live,
                connecting: 0,
                ready: true,
                stalled: 0,
                end: End::External,
                stop_delivery: Delivery::Sealed,
                stop_samples: 0,
                cancel_err: false,
            },
            Default::default(),
        );
        attempt.awaiting_external = true;
        attempt
    });
    w.status = Status::Recording;
    let mut checker = Checker::default();
    assert!(checker.on_end(&w, &[]));
    assert!(checker.violations.is_empty());
}

#[test]
fn a_stuck_recovering_run_needs_the_pd18_signature() {
    let mut w = World::new();
    w.status = Status::Recovering;
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(
        checker.violations.iter().any(|v| v.contains("Recovering")),
        "{:?}",
        checker.violations
    );
}

/// Mutation check on the harness's own scheduler: a blocked step must change
/// nothing in the world (the README's "poll once, do not follow the self-wake").
#[test]
fn a_blocked_step_changes_nothing() {
    for seed in 0..300u64 {
        let report = run_seed_strict(seed);
        assert!(report.steps > 0);
    }
}
