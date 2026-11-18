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

/// Seeds that take a signature or a PD entry no seed below `DEFAULT_SEEDS - len` takes
/// (D-S2b-11 (d)). The default run is `0..(10,000 - len)` plus these, 10,000 seeds in
/// all, so every witness is produced by the run itself: there is no pinned exception to
/// the converse check, and a witness that drifted away fails it.
///
/// * 15,632: PD-22 with the cancel completing performance (R211.2, R205.5).
/// * 43,329: PD-19 through a dropped start (no context was ever installed).
/// * 176,140: PD-21 with a short stop (R206.2, R203.4).
/// * 1,769,144: PD-19's refused auto-stop (the capture stays in the store and no ordinary
///   control sends the stop that would take it; D-S2b-15 (c), (d)). It is rare because the
///   unscoped stop IPC, one in four generated stops, takes the capture when it lands.
const FIXED_SEEDS: [u64; 4] = [15_632, 43_329, 176_140, 1_769_144];

/// PD-18's witnesses, one seed per instance (C7.5): the first seed of the fixed
/// range that ends stuck in `Recovering` through that instance, with the generation's
/// own evidence (R101.12 in the gap after its last restorer).
const PD18_SEEDS: [(&str, u64); 3] = [
    ("(a) S12", 169),
    ("(b) Idle-arm tail", 312),
    ("(c) unsealed stop", 1_759),
];

/// A witness of one of PD-19..PD-24: the PD, the seed that takes it, the matched
/// split `(terminal row, performance row)` where the PD is an SP4(b) split, and the
/// shell test that reproduces the sequence (`recording_harness_findings_tests.rs`,
/// D-S2b-5, D-S2b-7 and D-S2b-9 (b)). A `None` shell test is PROVISIONAL "reachable
/// by source inspection": its fixture is pending and the model witness stands alone.
struct Witness {
    pd: u8,
    seed: u64,
    split: Option<(&'static str, &'static str)>,
    shell_test: Option<&'static str>,
    /// The `SIGNATURES` row the seed must take, where the PD has more than one signature
    /// for the same terminal row (the variants bound to different causes).
    sig: Option<usize>,
}

const FINDING_SEEDS: [Witness; 14] = [
    Witness {
        pd: 19,
        seed: 9_652,
        split: None,
        shell_test: Some("pd19_termination_unconfirmed_after_retained_pcm_leaves_recovering"),
        sig: None,
    },
    Witness {
        pd: 19,
        seed: 43_329,
        split: None,
        shell_test: Some("pd19_a_dropped_start_and_a_retained_runtime_failure_leave_recovering"),
        sig: None,
    },
    Witness {
        pd: 19,
        seed: 1_769_144,
        split: None,
        shell_test: Some("pd19_a_refused_auto_stop_leaves_the_capture_and_recovering"),
        sig: None,
    },
    Witness {
        pd: 20,
        seed: 127,
        split: Some(("R205.5", "R203.5")),
        shell_test: Some("pd20_cancel_terminal_and_a_short_stop_performance_outcome_disagree"),
        sig: None,
    },
    Witness {
        pd: 21,
        seed: 176_140,
        split: Some(("R206.2", "R203.4")),
        shell_test: Some("pd21_timed_out_cancel_then_a_short_stop_disagree"),
        sig: None,
    },
    Witness {
        pd: 21,
        seed: 39,
        split: Some(("R206.2", "R209.4")),
        shell_test: Some("pd21_timed_out_cancel_and_the_late_recovery_disagree"),
        sig: None,
    },
    Witness {
        pd: 22,
        seed: 3_125,
        split: Some(("R211.2", "R203.2")),
        shell_test: Some("pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance"),
        sig: None,
    },
    Witness {
        pd: 22,
        seed: 15_632,
        split: Some(("R211.2", "R205.5")),
        shell_test: Some("pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 286,
        split: Some(("R207.3", "R203.5")),
        shell_test: Some("pd23_superseded_terminal_and_an_empty_stop_no_speech_disagree"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 74,
        split: Some(("R207.3", "R203.1")),
        shell_test: Some("pd23_superseded_terminal_and_a_refused_stop_failed_disagree"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 76,
        split: Some(("R207.3", "R205.5")),
        shell_test: Some("pd23_superseded_terminal_and_a_cancel_of_processing_cancelled_disagree"),
        sig: None,
    },
    // PENDING, source-only: R207.3's claim, the ticker's gate refusing the second ticker,
    // the pipeline completing `Success`. No fake-host schedule makes the gate refuse after
    // the claim, so it has no shell fixture; it stays allowlisted (D-S2b-13 (e)).
    Witness {
        pd: 23,
        seed: 537,
        split: Some(("R207.3", "R204b.6")),
        shell_test: None,
        sig: Some(2),
    },
    Witness {
        pd: 24,
        seed: 3_946,
        split: Some(("R102.3", "R209.3")),
        shell_test: Some(
            "pd24_start_timeout_terminal_and_the_users_cancel_arm_performance_outcome_disagree",
        ),
        sig: Some(3),
    },
    Witness {
        pd: 24,
        seed: 8_419,
        split: Some(("R102.3", "R209.5")),
        shell_test: Some(
            "pd24_start_timeout_terminal_and_an_environmental_recovery_arm_performance_outcome_disagree",
        ),
        sig: Some(4),
    },
];

fn seeds() -> u64 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SEEDS)
}

/// The seeds one run executes: the default 10,000 are `0..(10,000 - FIXED_SEEDS.len())`
/// and [`FIXED_SEEDS`]; a raised or lowered `PROPTEST_CASES` is `0..n` plus the fixed
/// seeds beyond it.
fn seed_list() -> Vec<u64> {
    let n = seeds();
    let fixed = FIXED_SEEDS.len() as u64;
    if n == DEFAULT_SEEDS {
        (0..DEFAULT_SEEDS - fixed).chain(FIXED_SEEDS).collect()
    } else {
        (0..n)
            .chain(FIXED_SEEDS.into_iter().filter(|seed| *seed >= n))
            .collect()
    }
}

#[derive(Default)]
struct Summary {
    seeds: u64,
    elapsed: Duration,
    failing: Vec<(u64, Vec<String>)>,
    pd: Vec<(u32, Option<u64>)>,
    pd18: Vec<(u32, Option<u64>)>,
    /// PD-19 runs per entry (`RunHits::pd19`): occurrences and first seed.
    pd19: Vec<(u32, Option<u64>)>,
    sig: Vec<(u32, Option<u64>)>,
    /// Matched SP4(b) splits `(terminal row, performance row)`: occurrences and first seed.
    pairs: BTreeMap<(&'static str, &'static str), (u32, u64)>,
    steps: u64,
}

fn summary() -> &'static Summary {
    static SUMMARY: OnceLock<Summary> = OnceLock::new();
    SUMMARY.get_or_init(|| {
        let list = seed_list();
        let mut summary = Summary {
            seeds: list.len() as u64,
            pd: vec![(0, None); 25],
            pd18: vec![(0, None); 3],
            pd19: vec![(0, None); 3],
            sig: vec![(0, None); SIGNATURES.len()],
            ..Summary::default()
        };
        let started = Instant::now();
        for seed in list {
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
            for (entry, count) in report.hits.pd19.iter().enumerate() {
                if *count > 0 {
                    summary.pd19[entry].0 += count;
                    summary.pd19[entry].1.get_or_insert(seed);
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
        for pd in 1..=24 {
            eprintln!(
                "property: PD-{pd}: {} occurrences, first at seed {:?}",
                summary.pd[pd].0, summary.pd[pd].1
            );
        }
        eprintln!("property: PD-18 stuck runs (a, b, c): {:?}", summary.pd18);
        eprintln!(
            "property: PD-19 entries (TerminationUnconfirmed, dropped start, refused auto-stop): {:?}",
            summary.pd19
        );
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
    let missing: Vec<usize> = (1..=24)
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
                "the split {pair:?} of PD-{} was never witnessed",
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

/// PD-19..PD-24 each have pinned seeds, and the shell test that reproduces the
/// sequence is named beside each (D-S2b-5, D-S2b-7). There is no "known findings"
/// set: a behaviour outside C7.1 fails `seeds_hold_the_properties`.
#[test]
fn pd19_to_pd24_have_pinned_seeds() {
    for witness in &FINDING_SEEDS {
        let report = run_seed(witness.seed);
        assert!(
            report.hits.pd[usize::from(witness.pd)] > 0,
            "seed {} no longer takes PD-{} (shell witness {:?}): {:?}",
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
        if let Some(row) = witness.sig {
            assert!(
                report.hits.sig[row] > 0,
                "seed {} no longer takes signature {row} of PD-{}: {:?}",
                witness.seed,
                witness.pd,
                report.hits
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
fn terminal(outcome: TerminalOutcome, code: ErrCode) -> Effect {
    Effect::Terminal(Terminal::new(outcome, code))
}

fn perf(outcome: PerfOutcome) -> Effect {
    Effect::PerfComplete(PerfComplete {
        outcome,
        stage: crate::effects::PerfStage::CaptureFinalization,
        cause: None,
        with_audio_duration: false,
    })
}

/// D-S2b-9 (d): the fixture is the appendix's rows, one line per row of `C3_ROWS`, and
/// nothing else (a snapshot of what a run observed would silently drop a row).
#[test]
fn the_c3_fixture_lists_every_c3_row_once() {
    let mut fixture = super::c3::fixture_rows();
    fixture.sort_unstable();
    let mut rows: Vec<&str> = crate::rows::C3_ROWS.iter().map(|(id, _)| *id).collect();
    rows.sort_unstable();
    assert_eq!(fixture, rows, "c3_fixture.txt and rows::C3_ROWS differ");
}

/// Both R211.2 mutants of the earlier round and the duplicate-terminal mutant fail,
/// per decision, while the unmutated row passes.
#[test]
fn the_c3_fixture_fails_the_r211_2_mutants_and_a_duplicate_terminal() {
    let check = |row: &str, status, stale, effects: &[Effect]| {
        super::c3::check_decision(row, status, stale, effects, None)
    };
    let superseded = || terminal(TerminalOutcome::Superseded, ErrCode::StaleOwner);
    assert_eq!(
        check("R211.2", Status::Idle, false, &[superseded()]),
        Ok(())
    );

    // Mutant A: claims `Success` instead of `Superseded`.
    let success = check(
        "R211.2",
        Status::Idle,
        false,
        &[terminal(TerminalOutcome::Success, ErrCode::None)],
    );
    assert!(success.unwrap_err().contains("R211.2"), "claim Success");
    // Mutant B: an extra `Idle -> Idle` write.
    let write = check(
        "R211.2",
        Status::Idle,
        false,
        &[superseded(), Effect::SetStatus(Status::Idle)],
    );
    assert!(
        write.unwrap_err().contains("ST:Idle>Idle"),
        "extra Idle write"
    );
    // Mutant C: an identical second terminal (the old snapshot accepted it).
    let twice = check("R211.2", Status::Idle, false, &[superseded(), superseded()]);
    assert!(twice.is_err(), "a duplicate terminal must fail");
    // A missing terminal fails too, and R211.2 does not run for a stale input.
    assert!(check("R211.2", Status::Idle, false, &[]).is_err());
    assert!(check("R211.2", Status::Idle, true, &[superseded()]).is_err());
    // The row reads `Idle`: another status is not R211.2.
    assert!(check("R211.2", Status::Processing, false, &[superseded()]).is_err());
}

/// Valid inputs the model never observed are accepted (the fixture is the appendix,
/// not the set of runs): the review's R213.2 and R203.8a cases among them.
#[test]
fn the_c3_fixture_accepts_valid_inputs_the_model_never_observed() {
    let check = |row: &str, status, stale, effects: &[Effect]| {
        super::c3::check_decision(row, status, stale, effects, None)
    };
    // R213.2 with `recovery_reason == RuntimeFailure`: `RuntimeInterruption/<the
    // input's code>`.
    assert_eq!(
        super::c3::check_decision(
            "R213.2",
            Status::Starting,
            false,
            &[
                terminal(
                    TerminalOutcome::RuntimeInterruption,
                    ErrCode::TerminationUnconfirmed
                ),
                perf(PerfOutcome::Failed)
            ],
            Some(ErrCode::TerminationUnconfirmed)
        ),
        Ok(())
    );
    // R203.8a: a stale stop whose capture is empty finishes the guard with `NoSpeech`.
    assert_eq!(
        check(
            "R203.8a",
            Status::Idle,
            true,
            &[
                terminal(TerminalOutcome::NoSpeech, ErrCode::EmptyAudio),
                Effect::PipelineGuardFinish(PerfOutcome::NoSpeech)
            ]
        ),
        Ok(())
    );
    // R102.3: the write over `Recovering`; R209.5: a system-wake recovery.
    assert_eq!(
        check(
            "R102.3",
            Status::Recovering,
            false,
            &[
                Effect::SetStatus(Status::Idle),
                terminal(TerminalOutcome::CaptureInitFailure, ErrCode::RuntimeFailure)
            ]
        ),
        Ok(())
    );
    assert_eq!(
        check(
            "R209.5",
            Status::Starting,
            false,
            &[
                Effect::SetStatus(Status::Recovering),
                perf(PerfOutcome::Cancelled),
                terminal(TerminalOutcome::RuntimeInterruption, ErrCode::SystemWake)
            ]
        ),
        Ok(())
    );
    // A row the appendix does not have is an error, not a pass.
    assert!(check("R999.1", Status::Idle, false, &[]).is_err());
}

/// D-S2b-11 (b): a terminal code written `@in` equals the code the decision's input
/// carries. Sol's mutant (R213.2 rewrites `TerminationUnconfirmed` to
/// `HostUnavailable`) fails, and so does a row that reports a code its input lacks.
#[test]
fn the_c3_fixture_checks_the_exact_input_code() {
    use super::c3::check_decision as check;
    let effects = |code| {
        [
            terminal(TerminalOutcome::RuntimeInterruption, code),
            perf(PerfOutcome::Failed),
        ]
    };
    let input = Some(ErrCode::TerminationUnconfirmed);
    assert_eq!(
        check(
            "R213.2",
            Status::Starting,
            false,
            &effects(ErrCode::TerminationUnconfirmed),
            input
        ),
        Ok(())
    );
    let rewritten = check(
        "R213.2",
        Status::Starting,
        false,
        &effects(ErrCode::HostUnavailable),
        input,
    );
    assert!(
        rewritten.unwrap_err().contains("HostUnavailable"),
        "the rewritten code fails"
    );
    // No input code: `@in` matches nothing.
    assert!(check(
        "R213.2",
        Status::Starting,
        false,
        &effects(ErrCode::TerminationUnconfirmed),
        None
    )
    .is_err());
    // The interrupted capture's reason is the code of R204b.5 and R203.4's terminal.
    assert!(check(
        "R204b.5",
        Status::Processing,
        false,
        &[terminal(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::StreamInvalidated
        )],
        Some(ErrCode::BackendError)
    )
    .is_err());
}

/// SP6 at every registration (D-S2b-9 (d)), not only R101.15's.
#[test]
fn the_continuation_id_bound_is_checked_at_every_registration() {
    use crate::decision::{ContKind, ContinuationSpec};
    use crate::ids::RecordingId;
    let spec = |rid: u64, kind: ContKind| ContinuationSpec {
        recording_id: RecordingId(rid),
        kind,
    };
    let mut w = World::new();
    w.current_id = 4;
    w.register(spec(4, ContKind::StopContext));
    w.register(spec(5, ContKind::CaptureStart)); // before ID+: `current + 1`
    assert!(w.registry_faults.is_empty(), "{:?}", w.registry_faults);
    w.register(spec(5, ContKind::ResolveContext));
    w.register(spec(6, ContKind::CaptureStart));
    w.register(spec(9, ContKind::Pipeline));
    assert_eq!(w.registry_faults.len(), 3, "{:?}", w.registry_faults);
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
    /// The lost claim lands after the performance completion rather than before it.
    lost_after_perf: bool,
    /// The cancel task whose tail was open at the claim, at the lost claim and at the
    /// performance completion (the window's identity).
    claim_tail: Option<TaskId>,
    lost_tail: Option<TaskId>,
    perf_tail: Option<TaskId>,
    /// A second ticker spawned for the recording after R207.3's claim.
    second_ticker: bool,
    /// A performance run was begun for the recording (the completion is the run's outcome),
    /// or none was (an aborted start: only the coordinator's claim).
    run: bool,
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
            lost_later: Some("R205.5"),
            lost_after_perf: false,
            claim_tail: Some(7),
            lost_tail: Some(7),
            perf_tail: Some(7),
            second_ticker: false,
            run: true,
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
            lost_after_perf: false,
            claim_tail: Some(7),
            lost_tail: None,
            perf_tail: Some(7),
            second_ticker: false,
            run: true,
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
            lost_after_perf: false,
            claim_tail: None,
            lost_tail: None,
            perf_tail: None,
            second_ticker: true,
            run: true,
        }
    }

    /// The world after both outcomes exist, and the `sp4b` verdict for id 1.
    fn run(&self) -> Checker {
        let w = self.world();
        let mut checker = Checker::default();
        checker.r207_3_side_tasks.insert(1);
        if self.second_ticker {
            checker.second_ticker.insert(1);
        }
        checker.sp4b(&w, 1);
        checker
    }

    fn world(&self) -> World {
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
            other_cancel_tail: self.claim_tail.is_some() && !self.own_tail,
            own_cancel_tail: self.own_tail,
            tail: self.claim_tail,
        };
        assert!(w.claim_terminal(1, Terminal::new(outcome, code), row, claim));
        if let Some(lost) = self.lost_later {
            let late = Won {
                row: lost,
                step: if self.lost_after_perf {
                    self.perf_step + 1
                } else {
                    self.claim_step + 1
                },
                tail: self.lost_tail,
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
            tail: self.perf_tail,
        };
        if self.run {
            w.perf.registered.insert(1);
            w.perf.begun.insert(1);
        }
        w.perf_complete(1, perf, perf_row, completion);
        w
    }
}

fn split_pd(checker: &Checker) -> Option<usize> {
    (1..25).find(|pd| checker.hits.pd[*pd] > 0)
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
                perf_tail: None,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 without the real path's later claim being lost",
            Split {
                lost_later: None,
                lost_after_perf: false,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 whose Idle claim fell outside any cancel tail",
            Split {
                claim_tail: None,
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 whose losing claim fell in another cancel's tail",
            Split {
                lost_tail: Some(8),
                ..Split::pd22()
            },
            None,
        ),
        (
            "PD-22 whose losing claim is not the cancel's own TERM",
            Split {
                lost_later: Some("R203.pre"),
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
        // D-S2b-11 (a), D-S2b-13 (e): PD-4 is exactly its C7.1 text. The same R207.3 claim
        // is PD-23 only for the outcome of the recording's own end the entry binds to a row.
        (
            "PD-4's claim whose recording then ends in another class is PD-23",
            Split {
                perf: (PerfOutcome::Failed, "R213.2"),
                ..Split::pd4()
            },
            Some(23),
        ),
        (
            "PD-23 with a stop's empty capture",
            Split {
                perf: (PerfOutcome::NoSpeech, "R203.5"),
                ..Split::pd4()
            },
            Some(23),
        ),
        (
            "PD-23 with a cancel of a processing recording",
            Split {
                perf: (PerfOutcome::Cancelled, "R205.5"),
                ..Split::pd4()
            },
            Some(23),
        ),
        (
            "PD-23 with `Completed` needs the second ticker refused: PD-4's own ticker is PD-4",
            Split::pd4(),
            Some(4),
        ),
        (
            "PD-23 with `Completed` and the ticker's gate refused",
            Split {
                second_ticker: false,
                ..Split::pd4()
            },
            Some(23),
        ),
        // The counterexample of the review: `Completed` from a row that cannot complete
        // `Success`, with or without a second ticker, is no PD.
        (
            "`Completed` from R203.2 with a second ticker is no PD",
            Split {
                perf: (PerfOutcome::Completed, "R203.2"),
                second_ticker: true,
                ..Split::pd4()
            },
            None,
        ),
        (
            "`Completed` from R203.2 without a second ticker is no PD",
            Split {
                perf: (PerfOutcome::Completed, "R203.2"),
                second_ticker: false,
                ..Split::pd4()
            },
            None,
        ),
        (
            "a performance outcome from a row outside PD-23's table",
            Split {
                perf: (PerfOutcome::Failed, "R999.9"),
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

/// D-S2b-11 (f): PD-22's window is the tail of one cancel; its events (the performance
/// completion, the cancel's lost claim) land in either order, so the split is judged when
/// the tail closes, not at the first prefix where both outcomes exist.
#[test]
fn pd22_defers_its_judgment_until_the_cancel_tail_closes() {
    // The stop's failure tail completes performance (step 6) before the cancel's `TERM`
    // is lost (step 7), both inside tail 7.
    let split = Split {
        perf_step: 6,
        lost_after_perf: true,
        ..Split::pd22()
    };
    let mut w = split.world();
    w.cancel_tail.insert(1, 7);
    let mut checker = Checker::default();
    checker.sp4b(&w, 1);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    assert_eq!(
        split_pd(&checker),
        None,
        "no verdict while the tail is open"
    );
    w.cancel_tail.remove(&1);
    checker.sp4b(&w, 1);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    assert_eq!(split_pd(&checker), Some(22));

    // A window whose cancel never lost a claim is no PD-22: judged at the close, it fails.
    let no_loser = Split {
        lost_later: None,
        ..split
    };
    let mut w = no_loser.world();
    w.cancel_tail.insert(1, 7);
    let mut checker = Checker::default();
    checker.sp4b(&w, 1);
    assert!(checker.violations.is_empty());
    w.cancel_tail.remove(&1);
    checker.sp4b(&w, 1);
    assert!(
        violations(&checker).contains("SP4(b)"),
        "{:?}",
        checker.violations
    );

    // At the end of the run an open window is judged as it stands.
    let mut w = no_loser.world();
    w.cancel_tail.insert(1, 7);
    let mut checker = Checker::default();
    checker.sp4b(&w, 1);
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("SP4(b)"),
        "{:?}",
        checker.violations
    );
}

/// D-S2b-13 (d): there is no blanket stale-input skip. SP4(b) compares the outcomes a
/// generation actually has: a stale-input claim for a recording whose run began and ended
/// differently fails; a completion with no run behind it (an aborted start: seed 8096) is
/// the coordinator's claim, leaves no performance row and is no outcome to compare.
#[test]
fn sp4b_applies_to_a_stale_claim_when_the_generation_has_both_outcomes() {
    let stale = Split {
        terminal: (TerminalOutcome::Superseded, ErrCode::StaleOwner, "R201.1"),
        claim_status: Status::Idle,
        claim_current: false,
        ..Split::pd4()
    };
    // A run exists: the stale claim and the run's outcome disagree. No signature covers a
    // stale claim, so this is an SP4(b) failure (it was skipped before).
    assert!(violations(&stale.run()).contains("SP4(b)"));
    assert_eq!(split_pd(&stale.run()), None);
    // No run was begun (the start aborted): nothing to compare.
    let aborted = Split {
        run: false,
        ..stale
    };
    let checker = aborted.run();
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    assert_eq!(split_pd(&checker), None);
}

/// The performance run's lifecycle (`request_completion`, `recording.rs:3114`): a
/// completion is the run's outcome only once the run began; while the begin is in flight
/// it waits and is applied when the begin resolves; after an abort it is dropped; with no
/// run registered at all it is a bare claim of the coordinator.
#[test]
fn a_completion_follows_the_performance_run_lifecycle() {
    let won = |step| Won {
        row: "R203.2",
        step,
        input_rid: 1,
        current: 1,
        status_before: Status::Idle,
        other_cancel_tail: false,
        own_cancel_tail: false,
        tail: None,
    };
    // Begun: the outcome.
    let mut w = World::new();
    w.perf.registered.insert(1);
    w.perf.begun.insert(1);
    w.perf_complete(1, PerfOutcome::Failed, "R203.2", won(1));
    assert_eq!(w.perf.outcome[&1].0, PerfOutcome::Failed);
    assert!(!w.perf.no_run.contains(&1));
    // Registered, begin in flight: held, the first request wins when the begin resolves.
    let mut w = World::new();
    w.perf.registered.insert(2);
    w.perf_complete(2, PerfOutcome::Cancelled, "R209.4", won(1));
    w.perf_complete(2, PerfOutcome::Failed, "R203.2", won(2));
    assert!(w.perf.outcome.is_empty());
    assert_eq!(w.perf.pending[&2].0, PerfOutcome::Cancelled);
    // Not registered (an aborted start, or an id that never began): a bare claim.
    let mut w = World::new();
    w.perf_complete(3, PerfOutcome::Failed, "R215.1", won(1));
    assert_eq!(w.perf.outcome[&3].0, PerfOutcome::Failed);
    assert!(w.perf.no_run.contains(&3));
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
        tail: None,
    };
    w.claim_terminal(
        1,
        Terminal::new(TerminalOutcome::Superseded, ErrCode::StaleOwner),
        "R999.1",
        claim,
    );
    w.perf.registered.insert(1);
    w.perf.begun.insert(1);
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
            connecting: false,
            ready: true,
            stalled: false,
            end: End::External,
            stop_delivery: Delivery::Sealed,
            stop_samples: 0,
            fail: ErrCode::BackendError,
        },
    );
    attempt.phase = Phase::Recording;
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

    // D-S2b-13 (c): a preserved capture nobody took is no exemption. With the long end and
    // the R101.12 gap alone it is still a stuck run, and a later stop does not count: the
    // user's stop in `Recovering` is suppressed by the frontend.
    let mut w = World::new();
    w.current_id = 1;
    w.status = Status::Recovering;
    w.preserved.insert(
        1,
        Preserved {
            samples: 12_000,
            code: ErrCode::StreamInvalidated,
        },
    );
    let mut checker = stranded(1, long);
    checker.on_end(&w, &[]);
    assert!(
        violations(&checker).contains("without PD-18/PD-19 evidence"),
        "{:?}",
        checker.violations
    );
    // With the refused auto-stop that took nothing, the capture still in the store and no
    // stop that took it, it is PD-19's third entry.
    let mut checker = stranded(1, long);
    let mut out = StepOut::default();
    out.notes.push(Note::AutoStopTookNothing { rid: 1 });
    checker.on_step(&World::new(), &out);
    checker.on_end(&w, &[]);
    assert!(checker.violations.is_empty(), "{:?}", checker.violations);
    assert_eq!(checker.hits.pd19, [0, 0, 1]);
    // A stop that did take the capture leaves no capture in the store: another entry.
    let mut taken = World::new();
    taken.current_id = 1;
    taken.status = Status::Recovering;
    let mut checker = stranded(1, long);
    let mut out = StepOut::default();
    out.notes.push(Note::AutoStopTookNothing { rid: 1 });
    checker.on_step(&World::new(), &out);
    checker.on_end(&taken, &[]);
    assert!(
        violations(&checker).contains("without PD-18/PD-19 evidence"),
        "{:?}",
        checker.violations
    );
}

/// The seeded runs witness PD-19 with the whole story, so the evidence is real.
#[test]
fn pd19_is_witnessed_with_its_whole_story() {
    // One seed per entry: `TerminationUnconfirmed`, a dropped start, the refused auto-stop
    // that leaves the capture in the store (no user-accessible recovery).
    for (seed, entries) in [
        (9_652, [1, 0, 0]),
        (43_329, [0, 1, 0]),
        (1_769_144, [0, 0, 1]),
    ] {
        let report = run_seed(seed);
        assert!(report.hits.pd[19] > 0, "{seed}: {:?}", report.hits);
        assert_eq!(report.hits.pd19, entries, "{seed}: {:?}", report.hits);
        assert_eq!(report.hits.pd18, [0, 0, 0]);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }
}

/// D-S2b-15 (c): stops follow their caller. A frontend-gated stop issued while the status
/// is `Recovering` is dropped, and the unscoped `stop_native_recording` IPC keeps the
/// schedule. Both occur in the seeded runs, and the IPC's stop in `Recovering` is a
/// flight that runs (a run that issues one still passes every property).
#[test]
fn stops_follow_their_caller() {
    let (mut suppressed, mut unscoped) = (0, 0);
    for seed in 0..3_000 {
        let (report, trace) = run_seed_with(seed, true);
        assert!(
            report.violations.is_empty(),
            "{seed}: {:?}",
            report.violations
        );
        suppressed += trace
            .iter()
            .filter(|line| line.contains("frontend stop suppressed"))
            .count();
        unscoped += trace
            .iter()
            .filter(|line| line.contains("unscoped stop IPC issued"))
            .count();
    }
    assert!(suppressed > 0, "no frontend stop was ever suppressed");
    assert!(
        unscoped > 0,
        "no unscoped stop IPC was ever issued in Recovering"
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

// ------------------------------------------------------------ the supervisor mirror

/// The mirror-table key of every message, worker event and supervisor step of the
/// model. The matches are exhaustive: a new variant does not compile until it names a
/// row, and the test below requires that row to exist.
fn mirror_keys() -> Vec<&'static str> {
    use super::supervisor::StartAnswer;
    let _ = StartAnswer::Started;
    let step = |step: SupStep| match step {
        SupStep::Accepted => "SupStep::Accepted",
        SupStep::StillConnecting => "Timer::StillConnecting",
        SupStep::Ready => "SupStep::Ready",
        SupStep::RecoveryStalled => "SupStep::RecoveryStalled",
        SupStep::Recovering(_) => "SupStep::Recovering",
        SupStep::InitFailed { .. } => "SupStep::InitFailed",
        SupStep::Interrupted(_) => "SupStep::Interrupted",
        SupStep::Idle => "SupStep::Idle",
        SupStep::ClearOwner => "SupStep::ClearOwner",
        SupStep::CleanupFinished => "SupStep::CleanupFinished",
        SupStep::StartAnswer(_) => "SupStep::StartAnswer",
        SupStep::Reply(..) => "SupStep::Reply",
        SupStep::CancelAnswer(_) => "SupStep::CancelAnswer",
        SupStep::StopAnswer(_) => "SupStep::StopAnswer",
    };
    let worker = |ev: WorkerEv| match ev {
        WorkerEv::FirstBuffer => "WorkerEv::FirstBuffer",
        WorkerEv::RuntimeFailed => "WorkerEv::RuntimeFailed",
        WorkerEv::TerminationUnconfirmed => "WorkerEv::TerminationUnconfirmed",
        WorkerEv::InitFailed => "WorkerEv::InitFailed",
        WorkerEv::RecordingSealed => "WorkerEv::RecordingSealed",
        WorkerEv::ThreadExited => "WorkerEv::ThreadExited",
    };
    let message = |msg: &Msg| match msg {
        Msg::Start { .. } => "Msg::Start (client enqueue)",
        Msg::Stop { .. } => "Msg::Stop (client enqueue)",
        Msg::Cancel { .. } => "Msg::Cancel (client enqueue)",
        Msg::Worker { .. } => "supervisor loop",
    };
    let mut keys = vec![
        step(SupStep::Accepted),
        step(SupStep::StillConnecting),
        step(SupStep::Ready),
        step(SupStep::RecoveryStalled),
        step(SupStep::Recovering(crate::status::AudioCancelReason::User)),
        step(SupStep::InitFailed {
            recovery: None,
            code: ErrCode::BackendError,
        }),
        step(SupStep::Interrupted(0)),
        step(SupStep::Idle),
        step(SupStep::ClearOwner),
        step(SupStep::CleanupFinished),
        step(SupStep::StartAnswer(0)),
        step(SupStep::Reply(0, ReplyVal::Start)),
        step(SupStep::CancelAnswer(0)),
        step(SupStep::StopAnswer(0)),
    ];
    for ev in [
        WorkerEv::FirstBuffer,
        WorkerEv::RuntimeFailed,
        WorkerEv::TerminationUnconfirmed,
        WorkerEv::InitFailed,
        WorkerEv::RecordingSealed,
        WorkerEv::ThreadExited,
    ] {
        keys.push(worker(ev));
    }
    for msg in [
        Msg::Start {
            rid: 0,
            tid: 0,
            plan: Box::new(external_attempt_plan()),
        },
        Msg::Stop { rid: 0, tid: 0 },
        Msg::Cancel {
            rid: 0,
            tid: None,
            reason: crate::status::AudioCancelReason::User,
            starting_only: false,
        },
        Msg::Worker {
            rid: 0,
            ev: WorkerEv::ThreadExited,
        },
    ] {
        keys.push(message(&msg));
    }
    keys
}

fn external_attempt_plan() -> AttemptPlan {
    AttemptPlan {
        kind: StartKind::Live,
        connecting: false,
        ready: true,
        stalled: false,
        end: End::External,
        stop_delivery: Delivery::Sealed,
        stop_samples: 0,
        fail: ErrCode::BackendError,
    }
}

/// D-S2b-11 (f): the start call has a receiver deadline like the stop and the cancel. A
/// start whose receiver timed out revokes its caller and enqueues `Cancel{User}`
/// (`await_start_response`, `3065-3079`); the supervisor that handles the start later
/// refuses it (`handle_start`, `1402-1405`) and creates no attempt, then handles the
/// cancel, which finds nothing to abandon.
#[test]
fn a_start_whose_receiver_timed_out_is_refused_and_a_cancel_follows() {
    use super::supervisor::{ReplyPoll, StartAnswer};
    let mut w = World::new();
    w.sup_send_start(1, 7, external_attempt_plan());
    let mut polls = 0;
    loop {
        match w.sup_poll_reply(7) {
            ReplyPoll::Waiting => polls += 1,
            ReplyPoll::TimedOut => break,
            other => panic!("nothing answered the start yet: {other:?}"),
        }
        assert!(polls < 1_000, "the start's receiver never timed out");
    }
    // The continuation of the timeout: the caller flag is revoked, `Cancel{User}` queued.
    w.sup_start_timed_out(1, 7);
    assert!(w.sup.caller_dead.contains(&7));
    assert_eq!(w.sup.queue.len(), 2);
    w.sup_accept();
    assert!(
        w.sup.attempt.is_none(),
        "a revoked start creates no attempt"
    );
    assert_eq!(w.sup.owner, None);
    assert_eq!(
        w.sup.start_pending.get(&7),
        Some(&StartAnswer::Failed(crate::inputs::CaptureErr::Other))
    );
    assert_eq!(w.sup.handling.len(), 1, "the refusal is a step of its own");
    // Its delivery fails: nobody waits any more.
    assert_eq!(w.sup.replies.get(&7), None);
    assert!(!w.send_reply_pub(7, ReplyVal::Start));
    w.sup.handling.clear();
    w.sup_accept();
    assert!(w.sup.attempt.is_none());
    assert!(
        w.sup.handling.is_empty(),
        "a cancel with no response and no attempt has nothing to answer"
    );
}

/// D-S2b-11 (f): an immediate refusal is not part of the acceptance. The reply is a step
/// of its own, so the client's receiver can expire between the two and the reply is then
/// dropped (`send` on a dropped receiver fails).
#[test]
fn an_immediate_reply_is_a_step_after_the_acceptance_and_can_miss_its_receiver() {
    let mut w = World::new();
    w.sup_send_stop(1, 5);
    w.sup_accept();
    assert_eq!(
        w.sup.handling.front(),
        Some(&SupStep::Reply(5, ReplyVal::Stop(Ok(0)))),
        "a stop with no attempt answers an empty capture in a step of its own"
    );
    assert!(
        matches!(w.sup.replies.get(&5), Some(ReplyState::Waiting { .. })),
        "nothing was delivered at the acceptance"
    );
    // The receiver times out first: the step's send fails and delivers nothing.
    w.sup.replies.insert(5, ReplyState::TimedOut);
    assert!(!w.send_reply_pub(5, ReplyVal::Stop(Ok(0))));
    assert_eq!(w.sup.replies.get(&5), Some(&ReplyState::TimedOut));
    // A cancel with nothing to abandon answers `Ok(false)` the same way.
    let mut w = World::new();
    w.sup_send_cancel(1, 6);
    w.sup_accept();
    assert_eq!(
        w.sup.handling.front(),
        Some(&SupStep::Reply(6, ReplyVal::Cancel(Ok(false))))
    );
}

/// D-S2b-9 (a): every message, worker event and supervisor step of the model has a row in
/// the mirror table. The rows themselves are checked against the source in the app crate
/// (`src/audio_lifecycle_mirror_tests.rs`, D-S2b-15): the function, the lines and the
/// fragments as tokens, and the whole-file pins.
#[test]
fn every_modeled_step_has_a_mirror_row() {
    for key in mirror_keys() {
        assert!(
            super::supervisor::MIRROR
                .iter()
                .any(|row| row.step.starts_with(key)),
            "the model step `{key}` has no mirror row"
        );
    }
    for row in super::supervisor::MIRROR {
        assert!(
            !row.what.is_empty(),
            "{}: the row says what the model does",
            row.step
        );
    }
}
