//! The property suite (C7.5): 10,000 fixed seeds, the per-PD converse witness, the
//! PD-18 instance seeds, the PD-19..PD-22 witness seeds, determinism, and unit
//! tests of the checker itself (a checker that cannot fail proves nothing).

use super::checks::{Checker, SIGNATURES};
use super::model::*;
use super::run::{run_seed, run_seed_strict, run_seed_with, RunReport};
use super::task::*;
use crate::decision::{SiteId, Verdict};
use crate::effects::{Effect, PerfComplete, PerfOutcome, Placement, Terminal};
use crate::status::{ErrCode, Status, TerminalOutcome};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// CI runs this many seeds; `PROPTEST_CASES` raises (or lowers) it locally.
const DEFAULT_SEEDS: u64 = 10_000;
const CI_BUDGET: Duration = Duration::from_secs(20);

/// PD-18's witnesses, one seed per instance (C7.5): the first seed of the fixed
/// range that ends stuck in `Recovering` through that instance, with the generation's
/// own evidence (R101.12 in the gap after its last restorer).
const PD18_SEEDS: [(&str, u64); 3] = [
    ("(a) S12", 746),
    ("(b) Idle-arm tail", 109),
    ("(c) unsealed stop", 166),
];

/// A witness of one of PD-19..PD-22: the PD, the seed that takes it, the matched
/// split `(terminal row, performance row)` where the PD is an SP4(b) split, and the
/// shell test that reproduces the sequence (`recording_harness_findings_tests.rs`,
/// D-S2b-5 and D-S2b-7).
struct Witness {
    pd: u8,
    seed: u64,
    split: Option<(&'static str, &'static str)>,
    shell_test: &'static str,
}

const FINDING_SEEDS: [Witness; 6] = [
    Witness {
        pd: 19,
        seed: 8147,
        split: None,
        shell_test: "f4_interrupted_over_8000_with_a_refused_auto_stop_leaves_recovering",
    },
    Witness {
        pd: 20,
        seed: 949,
        split: Some(("R205.5", "R203.5")),
        shell_test: "f1_cancel_terminal_and_a_short_stop_performance_outcome_disagree",
    },
    Witness {
        pd: 21,
        seed: 127,
        split: Some(("R206.2", "R203.4")),
        shell_test: "f2_failed_capture_cancel_terminal_and_a_short_stop_performance_outcome_disagree",
    },
    Witness {
        pd: 21,
        seed: 161,
        split: Some(("R206.2", "R209.4")),
        shell_test: "f2_timed_out_capture_cancel_terminal_and_the_late_recovery_performance_outcome_disagree",
    },
    Witness {
        pd: 22,
        seed: 2883,
        split: Some(("R211.2", "R203.2")),
        shell_test: "f3_superseded_terminal_and_a_failed_stop_performance_outcome_disagree",
    },
    Witness {
        pd: 22,
        seed: 3517,
        split: Some(("R211.2", "R205.5")),
        shell_test: "f3_superseded_terminal_and_a_cancels_performance_outcome_disagree",
    },
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
    /// Matched SP4(b) splits `(terminal row, performance row)`: occurrences and first seed.
    pairs: BTreeMap<(&'static str, &'static str), (u32, u64)>,
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
            for (pair, count) in &report.hits.pairs {
                let entry = summary.pairs.entry(*pair).or_insert((0, seed));
                entry.0 += count;
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
        if std::env::var("DUMP_VIOLATIONS").is_ok() {
            for (seed, violations) in &summary.failing {
                for violation in violations {
                    eprintln!("VIOL {seed} {violation}");
                }
            }
        }
        summary.elapsed = started.elapsed();
        eprintln!(
            "property: {} seeds, {} steps, {:?}; liveness checked on every run",
            summary.seeds, summary.steps, summary.elapsed
        );
        for pd in 1..=22 {
            eprintln!(
                "property: PD-{pd}: {} occurrences, first at seed {:?}",
                summary.pd[pd].0, summary.pd[pd].1
            );
        }
        eprintln!("property: PD-18 stuck runs (a, b, c): {:?}", summary.pd18);
        for ((terminal, perf), (count, first)) in &summary.pairs {
            eprintln!(
                "property: split {terminal} -> {perf}: {count} occurrences, first at seed {first}"
            );
        }
        for (row, signature) in SIGNATURES.iter().enumerate() {
            eprintln!(
                "property: signature {row} (PD-{}, {:?}): {} occurrences, first at seed {:?}",
                signature.pd, signature.pre, summary.sig[row].0, summary.sig[row].1
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
/// seeds, so an allowlist entry cannot rot, and so does every row of the SP4(b)
/// signature table and every split a pinned witness names. PD-17 is latent (no
/// production schedule) and has no witness; PD-18 has one per instance (a later test).
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
        .map(|(signature, _)| (signature.pd, signature.pre))
        .collect();
    assert!(
        stale.is_empty(),
        "signature rows never witnessed: {stale:?}"
    );
    for witness in &FINDING_SEEDS {
        if let Some(pair) = witness.split {
            assert!(
                summary.pairs.contains_key(&pair),
                "PD-{}: the split {pair:?} was never matched",
                witness.pd
            );
        }
    }
    assert_eq!(
        summary.pd[17].0, 0,
        "PD-17 is latent: no schedule reaches it"
    );
}

/// PD-18's three instances each end a run stuck in `Recovering` through the
/// generation's own evidence. One pinned seed per instance.
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

/// PD-19..PD-22 each have pinned seeds, and the shell test that reproduces the
/// sequence is named beside each (D-S2b-5, D-S2b-7). There is no "known findings"
/// set: a behaviour outside C7.1 fails `seeds_hold_the_properties`.
#[test]
fn pd19_to_pd22_have_pinned_seeds() {
    for witness in &FINDING_SEEDS {
        let report = run_seed(witness.seed);
        assert!(
            report.hits.pd[usize::from(witness.pd)] > 0,
            "seed {} no longer takes PD-{} (shell witness {}): {:?}",
            witness.seed,
            witness.pd,
            witness.shell_test,
            report.hits
        );
        if let Some(pair) = witness.split {
            assert!(
                report.hits.pairs.contains_key(&pair),
                "seed {} no longer takes the split {pair:?}: {:?}",
                witness.seed,
                report.hits.pairs
            );
        }
        assert!(report.violations.is_empty(), "{:?}", report.violations);
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

fn effect_at(
    row: &'static str,
    effect: Effect,
    at: Placement,
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
        at,
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

fn effect(
    row: &'static str,
    effect: Effect,
    input_rid: Rid,
    current: Rid,
    from: Status,
) -> EffectEv {
    effect_at(row, effect, Placement::In, input_rid, current, from)
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

fn violations(checker: &Checker) -> String {
    checker.violations.join(" | ")
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

    // The PD-1 row R216.4 writes `Idle` over a newer recording: allowlisted.
    let mut out = StepOut::default();
    out.decisions.push(decision("R216.4", 1, 2));
    out.effects.push(effect(
        "R216.4",
        Effect::SetStatus(Status::Idle),
        1,
        2,
        Status::Processing,
    ));
    assert!(
        check(out).violations.is_empty(),
        "the PD-1 row's own write is allowlisted"
    );

    // The same write from a different row, in the same step as a PD-1 hit: not exempt.
    let mut out = StepOut::default();
    out.decisions.push(decision("R216.4", 1, 2));
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
    for (row, from, to) in [
        ("R101.12", Status::Idle, Status::Recovering),
        ("R202.4", Status::Idle, Status::Idle),
        ("R201.7", Status::Recovering, Status::Processing),
    ] {
        let mut out = StepOut::default();
        out.effects
            .push(effect(row, Effect::SetStatus(to), 2, 2, from));
        assert!(check(out).violations.is_empty(), "{row} {from:?} -> {to:?}");
    }
}

/// D-S2b-7 (d): an edge in the allowed set is still checked against the `(row, from,
/// to)` C3 lists for the row. The two mutants of R211.2 Sol applied to the core: a
/// row that claims `Success` instead of `Superseded`, and one that also writes
/// `Idle -> Idle` (a consistent `transition`, so the decision still validates).
#[test]
fn the_c3_table_fails_both_r211_2_mutants() {
    let r211_2 = |effect_kind: Effect| {
        let mut out = StepOut::default();
        out.effects
            .push(effect("R211.2", effect_kind, 1, 1, Status::Idle));
        check(out)
    };
    let unmutated = r211_2(Effect::Terminal(Terminal::new(
        TerminalOutcome::Superseded,
        ErrCode::StaleOwner,
    )));
    assert!(
        unmutated.violations.is_empty(),
        "{:?}",
        unmutated.violations
    );

    let success = r211_2(Effect::Terminal(Terminal::new(
        TerminalOutcome::Success,
        ErrCode::None,
    )));
    assert!(
        violations(&success).contains("C3: R211.2 claimed Success"),
        "{:?}",
        success.violations
    );

    let extra_write = r211_2(Effect::SetStatus(Status::Idle));
    assert!(
        violations(&extra_write).contains("SP3: R211.2 wrote Idle -> Idle"),
        "{:?}",
        extra_write.violations
    );
}

/// SP2 for terminals: under D a stale input claims only what C3 lists for a stale
/// input of the row; later placements may see a newer id and are checked on outcome.
#[test]
fn a_stale_terminal_under_d_must_be_listed_for_a_stale_input() {
    let superseded = || {
        Effect::Terminal(Terminal::new(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
        ))
    };
    let mut out = StepOut::default();
    out.effects
        .push(effect("R211.2", superseded(), 1, 2, Status::Idle));
    let checker = check(out);
    assert!(
        violations(&checker).contains("C3: R211.2 claimed"),
        "{:?}",
        checker.violations
    );
    let mut out = StepOut::default();
    out.effects.push(effect_at(
        "R211.2",
        superseded(),
        Placement::PostDT,
        1,
        2,
        Status::Idle,
    ));
    assert!(check(out).violations.is_empty());
}

#[test]
fn a_step_violation_is_a_failure() {
    let mut out = StepOut::default();
    out.violations
        .push("SP7: R104.2 differs by op status".into());
    assert_eq!(check(out).violations.len(), 1);
}

// ----------------------------------------------------------------- SP4(b)

struct Split {
    terminal: (TerminalOutcome, ErrCode, &'static str),
    claim_status: Status,
    claim_step: u32,
    claim_current: bool,
    own_tail: bool,
    perf: (PerfOutcome, &'static str),
    perf_step: u32,
    perf_other_tail: bool,
    perf_own_tail: bool,
    /// A later claim by another path that lost.
    lost_later: Option<&'static str>,
}

impl Split {
    fn pd22() -> Self {
        Self {
            terminal: (TerminalOutcome::Superseded, ErrCode::StaleOwner, "R211.2"),
            claim_status: Status::Idle,
            claim_step: 5,
            claim_current: true,
            own_tail: false,
            perf: (PerfOutcome::Failed, "R203.2"),
            perf_step: 9,
            perf_other_tail: true,
            perf_own_tail: false,
            lost_later: Some("R203.pre"),
        }
    }

    fn pd20a() -> Self {
        Self {
            terminal: (
                TerminalOutcome::UserCancelledProcessing,
                ErrCode::CancelledProcessing,
                "R205.5",
            ),
            claim_status: Status::Idle,
            claim_step: 3,
            claim_current: true,
            own_tail: true,
            perf: (PerfOutcome::Completed, "R204b.6"),
            perf_step: 6,
            perf_other_tail: true,
            perf_own_tail: false,
            lost_later: None,
        }
    }

    fn pd4() -> Self {
        Self {
            terminal: (TerminalOutcome::Superseded, ErrCode::StaleOwner, "R207.3"),
            claim_status: Status::Recording,
            claim_step: 3,
            claim_current: true,
            own_tail: false,
            perf: (PerfOutcome::Completed, "R204b.6"),
            perf_step: 8,
            perf_other_tail: false,
            perf_own_tail: false,
            lost_later: None,
        }
    }

    /// The world after both outcomes exist, and the `sp4b` verdict for id 1.
    fn run(&self) -> Checker {
        let mut w = World::new();
        w.current_id = 1;
        w.ledger.accepted.insert(1);
        let current = if self.claim_current { 1 } else { 2 };
        let (outcome, code, row) = self.terminal;
        let claim = Won {
            row,
            step: self.claim_step,
            input_rid: 1,
            current,
            status_before: self.claim_status,
            other_cancel_tail: false,
            own_cancel_tail: self.own_tail,
        };
        assert!(w.claim_terminal(1, Terminal::new(outcome, code), row, claim));
        if let Some(lost) = self.lost_later {
            let late = Won {
                row: lost,
                step: self.claim_step + 1,
                ..claim
            };
            assert!(!w.claim_terminal(
                1,
                Terminal::new(TerminalOutcome::StopFailure, ErrCode::None),
                lost,
                late
            ));
        }
        let (perf, perf_row) = self.perf;
        let completion = Won {
            row: perf_row,
            step: self.perf_step,
            input_rid: 1,
            current,
            status_before: Status::Idle,
            other_cancel_tail: self.perf_other_tail,
            own_cancel_tail: self.perf_own_tail,
        };
        w.perf_complete(1, perf, perf_row, completion);
        let mut checker = Checker::default();
        checker.sp4b(&w, 1);
        checker
    }
}

fn split_pd(checker: &Checker) -> Option<usize> {
    (1..23).find(|pd| checker.hits.pd[*pd] > 0)
}

/// D-S2b-7 (a): an allowlist entry is the full signature. The rows and outcomes, the
/// statuses the claim saw and the causal precondition must all match; any one
/// different and the split is an SP4(b) failure, not a preserved defect.
#[test]
fn an_sp4b_split_needs_its_full_signature() {
    for (name, split, expected) in [
        ("PD-22 in full", Split::pd22(), Some(22)),
        ("PD-20 (a) in full", Split::pd20a(), Some(20)),
        ("PD-4 in full", Split::pd4(), Some(4)),
        (
            "PD-22 without a cancel tail open at the completion",
            Split {
                perf_other_tail: false,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 without the real path's later claim being lost",
            Split {
                lost_later: None,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 claimed in a status C3 does not list for R211.2",
            Split {
                claim_status: Status::Processing,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 with a performance row outside the signature",
            Split {
                perf: (PerfOutcome::Failed, "R213.2"),
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 with another outcome class",
            Split {
                perf: (PerfOutcome::NoSpeech, "R203.2"),
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-20 (a) whose completion landed before the cancel's claim",
            Split {
                perf_step: 2,
                ..Split::pd20a()
            },
            None,
        ),
        (
            "PD-20 (a) whose claim is not the cancel's own tail",
            Split {
                own_tail: false,
                ..Split::pd20a()
            },
            None,
        ),
        (
            "PD-4 on a stale input",
            Split {
                claim_current: false,
                ..Split::pd4()
            },
            None,
        ),
        (
            "PD-4's row on a status it does not list",
            Split {
                claim_status: Status::Processing,
                ..Split::pd4()
            },
            None,
        ),
        (
            "an unlisted terminal row",
            Split {
                terminal: (TerminalOutcome::Superseded, ErrCode::StaleOwner, "R999.1"),
                ..Split::pd4()
            },
            None,
        ),
    ] {
        let checker = split.run();
        match expected {
            Some(pd) => {
                assert_eq!(
                    split_pd(&checker),
                    Some(pd),
                    "{name}: {:?}",
                    checker.violations
                );
                assert!(
                    checker.violations.is_empty(),
                    "{name}: {:?}",
                    checker.violations
                );
            }
            None => {
                assert!(
                    violations(&checker).contains("SP4(b)"),
                    "{name} must fail SP4(b): {:?}",
                    checker.violations
                );
                assert_eq!(split_pd(&checker), None, "{name}");
            }
        }
    }
}

/// The split is judged at the step where the second outcome arrives, not at the end of
/// the run: a seed whose later steps would hide it still fails.
#[test]
fn an_sp4b_split_is_checked_at_the_prefix_where_both_outcomes_exist() {
    let mut w = World::new();
    w.current_id = 1;
    w.ledger.accepted.insert(1);
    let claim = Won {
        row: "R999.1",
        step: 1,
        input_rid: 1,
        current: 1,
        status_before: Status::Idle,
        other_cancel_tail: false,
        own_cancel_tail: false,
    };
    w.claim_terminal(
        1,
        Terminal::new(TerminalOutcome::Superseded, ErrCode::StaleOwner),
        "R999.1",
        claim,
    );
    w.perf_complete(
        1,
        PerfOutcome::Failed,
        "R203.2",
        Won {
            row: "R203.2",
            step: 2,
            ..claim
        },
    );
    let mut out = StepOut::default();
    out.effects.push(effect(
        "R203.2",
        Effect::PerfComplete(PerfComplete {
            outcome: PerfOutcome::Failed,
            stage: crate::effects::PerfStage::CaptureFinalization,
            cause: None,
            with_audio_duration: false,
        }),
        1,
        1,
        Status::Idle,
    ));
    let mut checker = Checker::default();
    checker.on_step(&w, &out);
    assert!(
        violations(&checker).contains("SP4(b): id 1"),
        "{:?}",
        checker.violations
    );
}

// -------------------------------------------------------------------- SP10

#[test]
fn sp10_fails_when_the_lease_moves_outside_its_holders_acquire_and_drop() {
    // Released by a step that is not the holder's drop.
    let mut checker = Checker::default();
    let mut w = World::new();
    let mut out = StepOut::default();
    out.notes.push(Note::Lease {
        rid: 1,
        tid: 5,
        acquire: true,
    });
    w.partial_lease = Some((1, 5));
    checker.on_step(&w, &out);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    w.partial_lease = None; // a terminal's path "released" it: no note
    checker.on_step(&w, &StepOut::default());
    assert!(
        violations(&checker).contains("SP10: the lease changed"),
        "{:?}",
        checker.violations
    );

    // Dropped by a ticker that does not hold it.
    let mut checker = Checker::default();
    let mut w = World::new();
    w.partial_lease = Some((1, 5));
    let mut out = StepOut::default();
    out.notes.push(Note::Lease {
        rid: 1,
        tid: 5,
        acquire: true,
    });
    checker.on_step(&w, &out);
    let mut out = StepOut::default();
    out.notes.push(Note::Lease {
        rid: 1,
        tid: 6,
        acquire: false,
    });
    w.partial_lease = None;
    checker.on_step(&w, &out);
    assert!(
        violations(&checker).contains("dropped a lease it does not hold"),
        "{:?}",
        checker.violations
    );

    // A second acquire while it is held.
    let mut checker = Checker::default();
    let mut w = World::new();
    for tid in [5, 6] {
        let mut out = StepOut::default();
        out.notes.push(Note::Lease {
            rid: 1,
            tid,
            acquire: true,
        });
        w.partial_lease = Some((1, tid));
        checker.on_step(&w, &out);
    }
    assert!(
        violations(&checker).contains("took the lease while"),
        "{:?}",
        checker.violations
    );

    // A lease still held at the end of the run leaked.
    let mut w = World::new();
    w.partial_lease = Some((1, 5));
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("never released"),
        "{:?}",
        checker.violations
    );
}

// ---------------------------------------------------------------- liveness

fn external_attempt(rid: Rid) -> Attempt {
    let mut attempt = Attempt::new(
        rid,
        AttemptPlan {
            kind: StartKind::Live,
            connecting: 0,
            ready: true,
            stalled: 0,
            end: End::External,
            stop_delivery: Delivery::Sealed,
            stop_samples: 0,
            cancel: CancelMode::Normal,
        },
        Default::default(),
    );
    attempt.awaiting_external = true;
    attempt
}

#[test]
fn liveness_fails_on_a_stuck_status_an_open_op_or_a_held_lock() {
    let mut w = World::new();
    w.status = Status::Processing;
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(violations(&checker).contains("status Processing"));

    let mut w = World::new();
    w.t_holder = Some(4);
    w.history.pipelines.insert(1);
    let mut checker = Checker::default();
    checker.on_end(&w, &["start"]);
    let all = violations(&checker);
    assert!(all.contains("T is still held"), "{all}");
    assert!(all.contains("pipeline of id 1 unresolved"), "{all}");
    assert!(all.contains("tasks never finished"), "{all}");
}

/// L4 (a recording nobody stops) exempts exactly that recording: the status of the
/// recording whose attempt awaits a stop or a cancel, and nothing else.
#[test]
fn a_recording_nobody_stops_exempts_only_that_recording() {
    let mut w = World::new();
    w.current_id = 1;
    w.sup.attempt = Some(external_attempt(1));
    w.status = Status::Recording;
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);

    // The attempt awaits recording 1, but recording 2 is the one stuck.
    w.current_id = 2;
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("status Recording at the end"),
        "{:?}",
        checker.violations
    );

    // A status other than Starting or Recording is never L4.
    let mut w = World::new();
    w.current_id = 1;
    w.sup.attempt = Some(external_attempt(1));
    w.status = Status::Processing;
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(violations(&checker).contains("status Processing"));

    // An accepted id of another recording that never claimed fails.
    let mut w = World::new();
    w.current_id = 2;
    w.sup.attempt = Some(external_attempt(2));
    w.status = Status::Recording;
    w.ledger.accepted.insert(1);
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("id 1 accepted but never claimed"),
        "{:?}",
        checker.violations
    );
}

/// A dropped start exempts a `Starting` or `Recording` status of its own recording
/// only, never `Processing`, never another recording's status.
#[test]
fn a_dropped_start_exempts_only_its_own_recording_in_starting_or_recording() {
    let dropped = |rid: Rid| {
        let mut checker = Checker::default();
        let mut out = StepOut::default();
        out.notes.push(Note::StartDropped {
            rid,
            registered: true,
            begun: true,
        });
        checker.on_step(&World::new(), &out);
        checker
    };
    let mut w = World::new();
    w.current_id = 1;
    w.status = Status::Starting;
    let mut checker = dropped(1);
    checker.on_end(&w, &[]);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);

    let mut checker = dropped(2);
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("status Starting at the end"),
        "{:?}",
        checker.violations
    );

    w.status = Status::Processing;
    let mut checker = dropped(1);
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("status Processing at the end"),
        "{:?}",
        checker.violations
    );
}

/// A run that ends in `Recovering` needs the stranded generation's own evidence
/// (C7.3, PD-18 and PD-19); evidence of another generation, or part of PD-19's, does not
/// count.
#[test]
fn a_stuck_recovering_run_needs_the_generations_own_pd18_or_pd19_evidence() {
    let stranded = |rid: Rid, ended_by: EndedBy| {
        let mut checker = Checker::default();
        let mut out = StepOut::default();
        let mut d = decision("R101.12", rid, rid);
        d.cleanup_pending = true;
        d.ended_by = Some(ended_by);
        out.decisions.push(d);
        checker.on_step(&World::new(), &out);
        checker
    };
    let mut w = World::new();
    w.current_id = 1;
    w.status = Status::Recovering;

    // No evidence at all.
    let mut checker = Checker::default();
    checker.on_end(&w, &[]);
    assert!(violations(&checker).contains("without PD-18/PD-19 evidence"));

    // PD-18 (a): a short `Interrupted` end and R101.12 in the gap, for this recording.
    let short = EndedBy::Recover {
        interrupted: Some(1_000),
    };
    let mut checker = stranded(1, short);
    checker.on_end(&w, &[]);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    assert_eq!(checker.hits.pd18, [1, 0, 0]);

    // The same evidence for another recording does not cover this one.
    let mut checker = stranded(2, short);
    checker.on_end(&w, &[]);
    assert!(violations(&checker).contains("without PD-18/PD-19 evidence"));

    // A long `Interrupted` end is PD-19 only with the whole story: the stop's capture
    // take, the stop path's own `Idle`, the refused auto-stop. The R101.12 alone fails.
    let long = EndedBy::Recover {
        interrupted: Some(12_000),
    };
    let mut checker = stranded(1, long);
    checker.on_end(&w, &[]);
    assert!(violations(&checker).contains("without PD-18/PD-19 evidence"));
}

/// The seeded runs witness PD-19 with the whole story, so the evidence is real.
#[test]
fn pd19_is_witnessed_with_its_whole_story() {
    let report = run_seed(8147);
    assert!(report.hits.pd[19] > 0, "{:?}", report.hits);
    assert_eq!(report.hits.pd18, [0, 0, 0]);
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
