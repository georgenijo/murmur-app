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

/// The regular property tests run exactly this many seeds, whatever the environment says
/// (D-S2b-18): the CI gate is one fixed set. A deeper sweep is the ignored
/// [`the_deep_sweep_holds_the_properties`].
const DEFAULT_SEEDS: u64 = 10_000;

// Witness seeds for the delivery/stop competition (#1159, #1167), re-pinned by scanning the gated
// seeds. Each change of the harness's draws or schedule moves them: routing every flight through
// `DictationCore::handle` (#1160) and the start caller and reply (#1162) moved 176 -> 217,
// 147 -> 25, 473 -> 164, 169 -> 147; the Smart Auto and import gates, the main window's start guard
// and the generation tracking (#1169 review) moved them to 86, 100, 175 and 25.
const MAIN_DELIVERY_FIRST: u64 = 86;
const OVERLAY_DELIVERY_FIRST: u64 = 100;
const MAIN_STOP_FIRST: u64 = 175;
const OVERLAY_STOP_FIRST: u64 = 25;
/// #1167: the main window's second stop waits for the first (the issue's seed 1454 before the
/// re-pins), and the overlay, which has no guard, sends two.
const MAIN_SECOND_STOP_WAITS: u64 = 456;
const OVERLAY_TWO_STOPS: u64 = 162;
/// #1162, #1169 review: the main window's start `S` is in flight (it issued no second start), the
/// main window sends a stop, `S`'s reply is processed while that stop is in flight and puts
/// `Recording` over its `Processing`, the stop returns, and the second stop gesture is sent. It is
/// the first of 3,000,000 seeds that does (41 of them do), so it is a fixed seed.
const LATE_START_REPLY_SECOND_STOP: u64 = 73_499;
/// The rows of the start gate that the Smart Auto selection round makes reachable (#1169
/// review): the pre-D refusal of Smart Auto with a fixed device (R101.2) and the failed resolve
/// (R101.14). The gated seeds must decide each of them, so a `handle` that stops asking for the
/// selection fails the gate even if the flight's own twin agreed.
const START_GATE_ROWS: [&str; 2] = ["R101.2", "R101.14"];
/// The `NeedObservation` rounds the gated seeds answer: the Smart Auto resolve (R101.13) and the
/// import's benchmark read (R301.3).
const GATE_OBSERVATIONS: [&str; 2] = ["R101.13", "R301.3"];
/// The legacy import's rows (sites 301 and 302, Appendix C's stop cutover): the refusals, the
/// benchmark observation (R301.3 is decided twice for an import that reaches it), the accepted
/// import, its Smart Destination recheck and both finish branches.
const IMPORT_GATE_ROWS: [&str; 8] = [
    "R301.1", "R301.2", "R301.3", "R301.4", "R301.5", "R301.6", "R302.1", "R302.2",
];
/// The deep sweep's default size; `DEEP_SWEEP_SEEDS` sets another.
const DEEP_SWEEP_SEEDS: u64 = 300_000;
const CI_BUDGET: Duration = Duration::from_secs(20);

/// Seeds that take a signature, a split or a PD entry no seed below `DEFAULT_SEEDS - len`
/// takes (D-S2b-11 (d)). The run is `0..(10,000 - len)` plus these, 10,000 seeds in all,
/// so every witness is produced by the run itself: there is no pinned exception to the
/// converse check, and a witness that drifted away fails it. Each is the first seed of a
/// 3,000,000-seed sweep that takes the witness.
///
/// * 14,033: PD-18 (c), the unsealed stop.
/// * 81,584: PD-19 through a dropped start (no context was ever installed).
/// * 492,192: PD-19 through `TerminationUnconfirmed`.
/// * 2,071,065: PD-19's refused auto-stop (the capture stays in the store and no ordinary
///   control sends the stop that would take it; D-S2b-15 (c), (d)). It is rare because the
///   unscoped stop IPC, one in four generated stops, takes the capture when it lands, and
///   a window sends no stop that it has not seen `Recording` for.
/// * 289,110: the fixed PD-21 with a short stop (R206.2, R203.4): a failed capture cancel that
///   claims no terminal, then the stop's own claim wins (`FAILED_CANCEL_SEEDS`).
/// * 31,086: PD-22 against a stop's performance completion (R211.2, R203.2).
/// * 154,052: PD-22 against the cancel's own `TERM` (R211.2, R205.5).
/// * 24,199: PD-24 through an environmental recovery (R102.3, R209.5). The user's cancel
///   (R102.3, R209.3) is now taken by seed 3,651, inside the range.
/// * 73,499: the late main-window start reply and its second stop (#1162, #1169 review), the first
///   of 3,000,000 seeds that has the whole sequence tied to one start and one stop.
///
/// Re-pinned for #1160 and #1162 (every flight decides through `DictationCore::handle`, a start
/// draws its caller) and again for the #1169 review (the main window's start guard, the reply
/// identity and generation tracking, the Smart Auto and import gates). Old -> new, from the
/// state before #1160: 10,724 -> 11,612 -> 14,033, 19,737 -> 21,272 -> 3,651 (now in range),
/// 39,219 -> 10,882 -> 492,192, 52,153 -> 81,353 -> 31,086, 101,117 -> 161,052 -> 154,052,
/// 259,286 -> 2,569,791 -> 2,071,065, 262,762 -> 179,248 -> 81,584,
/// 273,164 -> 48,827 -> 289,110; 203,928 -> 73,499; 24,199 is the PD-24 (R209.5) witness,
/// formerly seed 5,206 in range.
const FIXED_SEEDS: [u64; 9] = [
    14_033, 81_584, 492_192, 2_071_065, 289_110, 31_086, 154_052, 24_199, 73_499,
];

/// PD-18's witnesses, one seed per instance (C7.5): the first seed of the fixed
/// range that ends stuck in `Recovering` through that instance, with the generation's
/// own evidence (R101.12 in the gap after its last restorer).
const PD18_SEEDS: [(&str, u64); 3] = [
    ("(a) S12", 654),
    ("(b) Idle-arm tail", 121),
    ("(c) unsealed stop", 14_033),
];

/// A witness of one of PD-19..PD-24 (PD-21 is fixed and has none): the PD, the seed that takes it, the matched
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

const FINDING_SEEDS: [Witness; 12] = [
    Witness {
        pd: 19,
        seed: 492_192,
        split: None,
        shell_test: Some("pd19_termination_unconfirmed_after_retained_pcm_leaves_recovering"),
        sig: None,
    },
    Witness {
        pd: 19,
        seed: 81_584,
        split: None,
        shell_test: Some("pd19_a_dropped_start_and_a_retained_runtime_failure_leave_recovering"),
        sig: None,
    },
    Witness {
        pd: 19,
        seed: 2_071_065,
        split: None,
        shell_test: Some("pd19_a_refused_auto_stop_leaves_the_capture_and_recovering"),
        sig: None,
    },
    Witness {
        pd: 20,
        seed: 140,
        split: Some(("R205.5", "R203.5")),
        shell_test: Some("pd20_cancel_terminal_and_a_short_stop_performance_outcome_disagree"),
        sig: None,
    },
    Witness {
        pd: 22,
        seed: 31_086,
        split: Some(("R211.2", "R203.2")),
        shell_test: Some("pd22_expired_stop_idle_supersedes_and_the_stop_completes_performance"),
        sig: None,
    },
    Witness {
        pd: 22,
        seed: 154_052,
        split: Some(("R211.2", "R205.5")),
        shell_test: Some("pd22_expired_stop_idle_supersedes_and_the_cancel_completes_performance"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 450,
        split: Some(("R207.3", "R203.5")),
        shell_test: Some("pd23_superseded_terminal_and_an_empty_stop_no_speech_disagree"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 1_291,
        split: Some(("R207.3", "R203.1")),
        shell_test: Some("pd23_superseded_terminal_and_a_refused_stop_failed_disagree"),
        sig: None,
    },
    Witness {
        pd: 23,
        seed: 169,
        split: Some(("R207.3", "R205.5")),
        shell_test: Some("pd23_superseded_terminal_and_a_cancel_of_processing_cancelled_disagree"),
        sig: None,
    },
    // PENDING, source-only: R207.3's claim, the ticker's gate refusing the second ticker,
    // the pipeline completing `Success`. No fake-host schedule makes the gate refuse after
    // the claim, so it has no shell fixture; it stays allowlisted (D-S2b-13 (e)).
    Witness {
        pd: 23,
        seed: 1_610,
        split: Some(("R207.3", "R204b.6")),
        shell_test: None,
        sig: Some(2),
    },
    Witness {
        pd: 24,
        seed: 3_651,
        split: Some(("R102.3", "R209.3")),
        shell_test: Some(
            "pd24_start_timeout_terminal_and_the_users_cancel_arm_performance_outcome_disagree",
        ),
        sig: Some(3),
    },
    Witness {
        pd: 24,
        seed: 24_199,
        split: Some(("R102.3", "R209.5")),
        shell_test: Some(
            "pd24_start_timeout_terminal_and_an_environmental_recovery_arm_performance_outcome_disagree",
        ),
        sig: Some(4),
    },
];

/// The seeds the regular tests run: `0..(10,000 - FIXED_SEEDS.len())` and [`FIXED_SEEDS`],
/// 10,000 in all. Nothing in the environment changes it.
fn seed_list() -> Vec<u64> {
    let fixed = FIXED_SEEDS.len() as u64;
    (0..DEFAULT_SEEDS - fixed).chain(FIXED_SEEDS).collect()
}

/// Every seed a regular (non-ignored) test runs by number goes through this: it asserts the
/// seed is one of the gated [`seed_list`] seeds and returns it, so a regular test can never
/// fail on a seed the 10,000-seed gate does not cover (D-S2b-19 (a)). Exploratory and ignored
/// sweeps are exempt; the `TRACE_SEED` diagnostic is not a test.
fn gated(seed: u64) -> u64 {
    static GATE: OnceLock<std::collections::BTreeSet<u64>> = OnceLock::new();
    let gate = GATE.get_or_init(|| seed_list().into_iter().collect());
    assert!(
        gate.contains(&seed),
        "seed {seed} is outside the gated {DEFAULT_SEEDS} seeds: add it to FIXED_SEEDS or use another"
    );
    seed
}

/// The deep sweep's seeds: `0..n` plus the fixed seeds beyond it.
fn sweep_list(n: u64) -> Vec<u64> {
    (0..n)
        .chain(FIXED_SEEDS.into_iter().filter(|seed| *seed >= n))
        .collect()
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
    /// Decisions the flights acted on, by row: occurrences and first seed.
    decided: BTreeMap<&'static str, (u32, u64)>,
    /// `NeedObservation` rounds the flights answered, by row.
    observed: BTreeMap<&'static str, (u32, u64)>,
    /// Start replies that put `Recording` over the `Processing` of a stop in flight
    /// (`RunHits::reply_over_processing`): occurrences and first seed.
    reply_over_processing: (u32, Option<u64>),
    /// The second stop after such a reply: occurrences and first seed.
    second_stop_after_late_reply: (u32, Option<u64>),
    /// `starting` replies ignored for an ended generation: occurrences and first seed.
    reply_ended_generation: (u32, Option<u64>),
    steps: u64,
}

fn summary() -> &'static Summary {
    static SUMMARY: OnceLock<Summary> = OnceLock::new();
    SUMMARY.get_or_init(|| summarize(seed_list()))
}

fn summarize(list: Vec<u64>) -> Summary {
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
        for (row, count) in &report.hits.decided {
            summary.decided.entry(*row).or_insert((0, seed)).0 += count;
        }
        for (row, count) in &report.hits.observed {
            summary.observed.entry(*row).or_insert((0, seed)).0 += count;
        }
        if report.hits.reply_over_processing > 0 {
            summary.reply_over_processing.0 += report.hits.reply_over_processing;
            summary.reply_over_processing.1.get_or_insert(seed);
        }
        if report.hits.second_stop_after_late_reply > 0 {
            summary.second_stop_after_late_reply.0 += report.hits.second_stop_after_late_reply;
            summary.second_stop_after_late_reply.1.get_or_insert(seed);
        }
        if report.hits.reply_ended_generation > 0 {
            summary.reply_ended_generation.0 += report.hits.reply_ended_generation;
            summary.reply_ended_generation.1.get_or_insert(seed);
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
    for row in START_GATE_ROWS.iter().chain(&IMPORT_GATE_ROWS) {
        eprintln!(
            "property: row {row}: {:?} (occurrences, first seed)",
            summary.decided.get(row)
        );
    }
    for row in GATE_OBSERVATIONS {
        eprintln!(
            "property: observation round {row}: {:?}",
            summary.observed.get(row)
        );
    }
    eprintln!(
        "property: a start reply over a stop's Processing: {:?}; a second stop after it: {:?}; an ended generation's starting reply: {:?}",
        summary.reply_over_processing,
        summary.second_stop_after_late_reply,
        summary.reply_ended_generation
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

/// The deep sweep (the amendment's 300,000 seeds): the same properties on `0..300,000`
/// seeds plus the fixed ones, or `0..DEEP_SWEEP_SEEDS`. It is ignored, so a plain
/// `cargo test` never runs it and no environment variable changes the regular tests:
///
/// ```text
/// cargo test -p murmur-dictation-core --lib --release -- --ignored the_deep_sweep --nocapture
/// DEEP_SWEEP_SEEDS=3000000 cargo test -p murmur-dictation-core --lib --release -- --ignored the_deep_sweep --nocapture
/// ```
///
/// Findings it makes outside the allowlist are reported, not hidden: no seed exemption
/// (D-S2b-11 (d), D-S2b-16).
#[test]
#[ignore = "deep sweep: run on request (see the doc comment)"]
fn the_deep_sweep_holds_the_properties() {
    let n = std::env::var("DEEP_SWEEP_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEEP_SWEEP_SEEDS);
    let summary = summarize(sweep_list(n));
    let seeds: Vec<u64> = summary.failing.iter().map(|(seed, _)| *seed).collect();
    assert!(
        summary.failing.is_empty(),
        "{} of {} seeds violate a property outside the allowlist: {seeds:?}; first: {:?}",
        summary.failing.len(),
        summary.seeds,
        summary.failing.first()
    );
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
/// production schedule) and has no witness; PD-18 has one per instance (a later test); PD-21 is
/// fixed and has none (`a_failed_capture_cancel_claims_no_terminal`).
#[test]
fn every_preserved_defect_is_witnessed() {
    let summary = summary();
    if summary.seeds < DEFAULT_SEEDS {
        return; // a short local run does not owe every witness
    }
    let missing: Vec<usize> = (1..=24)
        .filter(|pd| !matches!(*pd, 17 | 21) && summary.pd[*pd].0 == 0)
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
    assert_eq!(
        summary.pd[21].0, 0,
        "PD-21 was fixed: a failed capture cancel claims no terminal, so no run splits on it"
    );
    // The gates of the cutover (#1169 review): every row of the Smart Auto round and of the legacy
    // import is decided by the gated seeds, and the late start reply the main window writes over
    // a stop's `Processing` happens in them.
    let silent: Vec<&str> = START_GATE_ROWS
        .iter()
        .chain(&IMPORT_GATE_ROWS)
        .filter(|row| !summary.decided.contains_key(**row))
        .copied()
        .collect();
    assert!(silent.is_empty(), "rows no gated seed decides: {silent:?}");
    let unasked: Vec<&str> = GATE_OBSERVATIONS
        .iter()
        .filter(|row| !summary.observed.contains_key(**row))
        .copied()
        .collect();
    assert!(
        unasked.is_empty(),
        "observation rounds no gated seed answers: {unasked:?}"
    );
    assert!(
        summary.reply_over_processing.0 > 0,
        "no gated seed has a start reply put `Recording` over the `Processing` of a stop in flight"
    );
}

/// PD-18's three instances each end a run stuck in `Recovering` through the
/// generation's own evidence. One pinned seed per instance.
#[test]
fn pd18_has_one_seed_per_instance() {
    for (index, (name, seed)) in PD18_SEEDS.iter().enumerate() {
        let report = run_seed(gated(*seed));
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
        let report = run_seed(gated(witness.seed));
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

/// The fixed PD-21: a failed (or timed-out) capture cancel, R206.2, claims no terminal for a
/// recording that goes on, so the terminal and the performance record both come from the
/// path that ends it. One pinned seed per follow-up: a stop that took the capture and
/// completed with a short outcome (R203.4), and the supervisor's late `Recovering{User}`
/// (R209.4). Both used to split (`PipelineFailure` against `NoSpeech` / `Cancelled`). The
/// second now agrees (`UserCancelledRecording` and `Cancelled`); the first falls to PD-5.
const FAILED_CANCEL_SEEDS: [(&str, u64, &str); 3] = [
    ("a short stop wins the capture", 289_110, "R203.4"),
    ("the supervisor recovers late", 3, "R209.4"),
    // The stop caller of R206.2 (`CancelOrigin::Stop`): a stop cancels a `Starting` recording
    // (R201.4) and no cancel command runs (no R205.4), so the only R206.2 is the stop's.
    ("a stop's capture cancel fails", 185, "R201.4"),
];

#[test]
fn a_failed_capture_cancel_claims_no_terminal() {
    for (name, seed, follow_up) in FAILED_CANCEL_SEEDS {
        let report = run_seed(gated(seed));
        assert!(
            report.hits.decided.get("R206.2").copied().unwrap_or(0) > 0,
            "seed {seed} ({name}) no longer decides a failed capture cancel: {:?}",
            report.hits
        );
        assert!(
            report.hits.decided.contains_key(follow_up),
            "seed {seed} ({name}) no longer reaches {follow_up}: {:?}",
            report.hits
        );
        assert!(
            report
                .hits
                .pairs
                .keys()
                .all(|(terminal, _)| *terminal != "R206.2"),
            "seed {seed} ({name}): R206.2 claimed a terminal: {:?}",
            report.hits.pairs
        );
        assert_eq!(report.hits.pd[21], 0);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }
    let stop_caller = run_seed(gated(185));
    assert!(
        !stop_caller.hits.decided.contains_key("R205.4"),
        "seed 185 runs a cancel command: {:?}",
        stop_caller.hits
    );
    // The short-stop seed ends in another preserved defect's split now: the worker's runtime
    // failure reaches R209.2 at `Processing`, which claims `Superseded` for the current id
    // (PD-5) ahead of the stop's `NoSpeech`. The failed cancel used to claim first.
    let report = run_seed(gated(289_110));
    assert!(report.hits.pd[5] > 0, "{:?}", report.hits);
}

#[test]
fn a_seed_is_one_run() {
    for seed in [0u64, 59, 886, 4903, 9_990] {
        let seed = gated(seed);
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
        (492_192, [1, 0, 0]),
        (81_584, [0, 1, 0]),
        (2_071_065, [0, 0, 1]),
    ] {
        let report = run_seed(gated(seed));
        assert!(report.hits.pd[19] > 0, "{seed}: {:?}", report.hits);
        assert_eq!(report.hits.pd19, entries, "{seed}: {:?}", report.hits);
        assert_eq!(report.hits.pd18, [0, 0, 0]);
        assert!(report.violations.is_empty(), "{:?}", report.violations);
    }
}

/// D-S2b-15 (c), D-S2b-17 (b), (c), D-S2b-18: stops follow their caller. A window stop
/// acts on the status its own window last *received* and is sent only on `Recording`: it
/// is dropped once `Recovering` has been received, and still issued while the window shows
/// `Recording` after the shell entered `Recovering` (delivery is a step of its own). The
/// unscoped `stop_native_recording` IPC keeps the schedule. All occur in the seeded runs and
/// every property holds in each.
#[test]
fn stops_follow_their_caller() {
    let (mut suppressed, mut not_sent, mut stale, mut unscoped, mut in_flight, mut coalesced) =
        (0, 0, 0, 0, 0, 0);
    for seed in 0..5_000 {
        let (report, trace) = run_seed_with(gated(seed), true);
        assert!(
            report.violations.is_empty(),
            "{seed}: {:?}",
            report.violations
        );
        let count = |needle: &str| trace.iter().filter(|line| line.contains(needle)).count();
        suppressed += count("frontend stop suppressed (");
        not_sent += count("frontend stop not sent (");
        stale += count("frontend stop issued on a stale observation");
        unscoped += count("unscoped stop IPC issued");
        in_flight += count("window has a stop in flight");
        coalesced += count("frontend start coalesced (the main window's start is still in flight)");
    }
    assert!(
        suppressed > 0,
        "no received-Recovering stop was ever suppressed"
    );
    assert!(
        not_sent > 0,
        "no stop on a non-Recording window was dropped"
    );
    assert!(stale > 0, "no stop was ever issued on a stale observation");
    assert!(
        unscoped > 0,
        "no unscoped stop IPC was ever issued in Recovering"
    );
    assert!(
        in_flight > 0,
        "no main-window stop was ever held back by its in-flight guard"
    );
    assert!(
        coalesced > 0,
        "no main-window start gesture was ever coalesced into the start in flight"
    );
}

/// #1167: the main window's `handleStop` sets `isStoppingRef` and writes `Processing` to its
/// own status before it awaits the IPC (`useRecordingState.ts`), clears the guard when the
/// IPC returns (`finally`) and when it receives `Idle`. The overlay keeps neither
/// (`useRecordingControls.ts`).
#[test]
fn the_main_window_guards_its_stop_and_the_overlay_does_not() {
    let mut w = World::new();
    apply_effect(&mut w, 1, "R101.15", Effect::SetStatus(Status::Recording));
    apply_effect(&mut w, 1, "R101.15", emit(Status::Recording));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(deliver(&mut w, OVERLAY_WINDOW), Poll::Progress);
    // The overlay sends a stop: no guard, no local status.
    w.window_stop_sent(OVERLAY_WINDOW);
    assert!(!w.windows[OVERLAY_WINDOW].stop_in_flight);
    assert_eq!(w.windows[OVERLAY_WINDOW].observed, Status::Recording);
    // The main window sends one: the guard and `Processing` are set before any flight runs.
    w.window_stop_sent(MAIN_WINDOW);
    assert!(w.windows[MAIN_WINDOW].stop_in_flight);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Processing);
    // The IPC returns: `finally` clears the guard; the local status stays until an event.
    w.window_stop_returned(MAIN_WINDOW);
    assert!(!w.windows[MAIN_WINDOW].stop_in_flight);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Processing);
    // Receiving `Idle` also clears it, while the IPC is still in flight.
    w.window_stop_sent(MAIN_WINDOW);
    apply_effect(&mut w, 1, "R203.6", emit(Status::Idle));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert!(!w.windows[MAIN_WINDOW].stop_in_flight);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Idle);
}

/// Drive a stop flight alone to completion over `w` (no other actor runs): every step must
/// progress or finish, and it must finish within a bounded number of steps.
fn drive_stop_flight(w: &mut World, flight: super::flights::StopFlight) {
    let mut flight = flight;
    let mut out = StepOut::default();
    let mut spawned: Vec<Box<dyn Task>> = Vec::new();
    let mut mailbox = Vec::new();
    for step in 0..200 {
        w.step = step;
        out.clear();
        let poll = flight.step(&mut Cx {
            w,
            out: &mut out,
            spawn: &mut spawned,
            tid: 3,
            mailbox: &mut mailbox,
        });
        match poll {
            Poll::Done => return,
            Poll::Progress => {}
            Poll::Blocked => panic!("the stop flight blocked at step {step}"),
        }
    }
    panic!("the stop flight did not finish");
}

/// #1167 review: the guard is cleared by the *flight* finishing, not by a model helper. A
/// main-window stop the shell refuses (the status is `Recovering`) finishes without any
/// `Idle` event; its `handleStop` `finally` must still clear `isStoppingRef`. This drives the
/// real `StopFlight` and delivers no status: it fails if `Ts::Finish` stops clearing it.
#[test]
fn a_finished_main_window_stop_flight_clears_its_guard() {
    use super::flights::{StopCfg, StopFlight};
    let cfg = StopCfg {
        panic_after_cp: false,
        pipeline: super::flights::PipelineCfg {
            work: 0,
            end: super::flights::PipelineEnd::Err,
        },
    };
    let new_world = || {
        let mut w = World::new();
        w.status = Status::Recovering;
        w.current_id = 1;
        w
    };
    let mut w = new_world();
    w.window_stop_sent(MAIN_WINDOW);
    assert!(w.windows[MAIN_WINDOW].stop_in_flight);
    drive_stop_flight(
        &mut w,
        StopFlight::new(None, None, cfg).with_window(MAIN_WINDOW),
    );
    assert!(
        !w.windows[MAIN_WINDOW].stop_in_flight,
        "the finished flight did not clear the guard"
    );
    // The shell refused it: no `Idle` was emitted, so only the flight's finish can have cleared it.
    assert!(w
        .windows
        .iter()
        .all(|view| !view.inbox.iter().any(|event| event.status == Status::Idle)));
    // A stop from the IPC or the auto-stop belongs to no window and leaves the guard alone.
    let mut w = new_world();
    w.windows[MAIN_WINDOW].stop_in_flight = true;
    drive_stop_flight(&mut w, StopFlight::new(None, None, cfg));
    assert!(w.windows[MAIN_WINDOW].stop_in_flight);
    // The overlay has no guard: its flight clears nothing and sets nothing. The main window's
    // guard is set while the overlay's flight runs to completion (#1162): a flight that
    // cleared "the" guard whatever its window would clear this one, so it must still be set.
    let mut w = new_world();
    w.window_stop_sent(MAIN_WINDOW);
    w.window_stop_sent(OVERLAY_WINDOW);
    assert!(w.windows[MAIN_WINDOW].stop_in_flight);
    drive_stop_flight(
        &mut w,
        StopFlight::new(None, None, cfg).with_window(OVERLAY_WINDOW),
    );
    assert!(!w.windows[OVERLAY_WINDOW].stop_in_flight);
    assert!(
        w.windows[MAIN_WINDOW].stop_in_flight,
        "the overlay's finished flight cleared the main window's guard"
    );
}

/// Drive a start flight alone to completion over `w` (no other actor runs): every step must
/// progress or finish, and it must finish within a bounded number of steps. The supervisor
/// never answers, so only a start that is refused before it sends a request can finish.
fn drive_start_flight(w: &mut World, flight: super::flights::StartFlight) {
    let mut flight = flight;
    let mut out = StepOut::default();
    let mut spawned: Vec<Box<dyn Task>> = Vec::new();
    let mut mailbox = Vec::new();
    for step in 0..200 {
        w.step = step;
        out.clear();
        let poll = flight.step(&mut Cx {
            w,
            out: &mut out,
            spawn: &mut spawned,
            tid: 4,
            mailbox: &mut mailbox,
        });
        match poll {
            Poll::Done => return,
            Poll::Progress => {}
            Poll::Blocked => panic!("the start flight blocked at step {step}"),
        }
    }
    panic!("the start flight did not finish");
}

/// #1162: a start's reply reaches the main window only when the main window sent the start, and
/// as a step of its own. The overlay and the IPC write no window state from it. A start the
/// shell refuses because a recording is already running replies `already_recording` with state
/// `recording` (`dictation_admission.rs:936`).
#[test]
fn a_main_window_start_returns_its_reply_for_the_window_to_process() {
    use super::flights::{Blockers, StartFlight};
    use super::gen::StartCaller;
    let recording_world = || {
        let mut w = World::new();
        w.status = Status::Recording;
        w.current_id = 1;
        w
    };
    let flight = |caller| {
        StartFlight::new(external_attempt_plan(), Blockers::default(), false, false)
            .with_caller(caller)
    };
    let mut w = recording_world();
    drive_start_flight(&mut w, flight(StartCaller::Window(MAIN_WINDOW)));
    assert_eq!(
        w.start_replies.iter().copied().collect::<Vec<_>>(),
        [reply(
            4,
            crate::replies::ReplyType::AlreadyRecording,
            Status::Recording,
            None
        )],
        "the refused start's reply waits for the main window, tied to that one start (t4)"
    );
    for caller in [StartCaller::Window(OVERLAY_WINDOW), StartCaller::Ipc] {
        let mut w = recording_world();
        drive_start_flight(&mut w, flight(caller));
        assert!(
            w.start_replies.is_empty(),
            "{caller:?} writes no window state"
        );
    }
    // A start future dropped at its await returns a failed invoke: the wrapper's error
    // response, which has no state (`lib/dictation.ts`), so it writes nothing but still
    // ends the main window's start operation.
    let mut w = recording_world();
    let mut dropped = flight(StartCaller::Window(MAIN_WINDOW));
    let mut out = StepOut::default();
    let mut spawned: Vec<Box<dyn Task>> = Vec::new();
    let mut mailbox = Vec::new();
    dropped.drop_now(&mut Cx {
        w: &mut w,
        out: &mut out,
        spawn: &mut spawned,
        tid: 4,
        mailbox: &mut mailbox,
    });
    drive_start_flight(&mut w, dropped);
    assert_eq!(
        w.start_replies.iter().copied().collect::<Vec<_>>(),
        [StartReply {
            tid: 4,
            response: StartResponse::Error
        }]
    );
}

/// The typed response of start `tid`.
fn reply(
    tid: TaskId,
    ty: crate::replies::ReplyType,
    state: Status,
    recording_id: Option<Rid>,
) -> StartReply {
    StartReply {
        tid,
        response: StartResponse::Typed {
            ty,
            state,
            recording_id,
        },
    }
}

/// #1162, `useRecordingState.ts:465-486`: the main window writes the reply's status unless the
/// reply does not own dictation state or is a stale `starting`, and the write ignores the stop
/// guard. The issue's sequence: the overlay's start won, the main window's start was refused
/// `already_recording`, its reply is delayed past the main window's stop, and then rewrites
/// `Recording` over the local `Processing` while the stop IPC is still in flight.
#[test]
fn the_main_window_writes_a_late_start_reply_over_its_processing_status() {
    use crate::replies::ReplyType;
    let mut w = World::new();
    w.window_stop_sent(MAIN_WINDOW);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Processing);
    w.window_start_sent();
    w.start_replied(reply(
        4,
        ReplyType::AlreadyRecording,
        Status::Recording,
        None,
    ));
    assert_eq!(
        w.windows[MAIN_WINDOW].observed,
        Status::Processing,
        "the reply has not been processed yet"
    );
    assert!(
        w.windows[MAIN_WINDOW].start_in_flight,
        "the start operation lasts until its reply is processed"
    );
    let (_, write) = w.deliver_start_reply().expect("a reply is pending");
    assert_eq!(write, ReplyWrite::Wrote(Status::Recording));
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Recording);
    assert!(
        !w.windows[MAIN_WINDOW].start_in_flight,
        "`beginRecording`'s finally ends the start operation"
    );
    assert!(
        w.windows[MAIN_WINDOW].stop_in_flight,
        "the write does not touch the guard: the stop IPC is still in flight"
    );
    assert!(w.deliver_start_reply().is_none());
    // `audio_recovering` and `already_starting` never own the state (`:471-472`).
    for ty in [ReplyType::AudioRecovering, ReplyType::AlreadyStarting] {
        let mut w = World::new();
        w.windows[MAIN_WINDOW].observed = Status::Processing;
        w.start_replied(reply(4, ty, Status::Idle, None));
        assert_eq!(
            w.deliver_start_reply().map(|(_, write)| write),
            Some(ReplyWrite::NotOwner)
        );
        assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Processing);
    }
    // A `starting` reply is stale once the window left `Idle` and `Starting` (`:481-483`).
    for observed in [Status::Recording, Status::Processing, Status::Recovering] {
        let mut w = World::new();
        w.windows[MAIN_WINDOW].observed = observed;
        w.start_replied(reply(
            4,
            ReplyType::RecordingStarting,
            Status::Starting,
            None,
        ));
        assert_eq!(
            w.deliver_start_reply().map(|(_, write)| write),
            Some(ReplyWrite::StaleStarting)
        );
        assert_eq!(w.windows[MAIN_WINDOW].observed, observed);
    }
    for observed in [Status::Idle, Status::Starting] {
        let mut w = World::new();
        w.windows[MAIN_WINDOW].observed = observed;
        w.start_replied(reply(
            4,
            ReplyType::RecordingStarting,
            Status::Starting,
            None,
        ));
        assert_eq!(
            w.deliver_start_reply().map(|(_, write)| write),
            Some(ReplyWrite::Wrote(Status::Starting))
        );
        assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Starting);
    }
}

/// #1169 review: the reply carries its recording id, and the window tracks the generations it saw
/// start and end (`useRecordingState.ts:174-178`, `:244`, `:474-476`). A generation ends in the
/// window when it receives `recovering`, `idle` or `processing`; a delayed `starting` reply of that
/// generation is then ignored, where the window that had received `Idle` would otherwise restore
/// `Starting`. A newer generation's reply, and a reply with no recording id, are not affected.
#[test]
fn the_main_window_ignores_a_starting_reply_of_a_generation_it_saw_end() {
    use crate::effects::EventKind;
    use crate::replies::ReplyType;
    let start_reply = |id| reply(4, ReplyType::RecordingStarting, Status::Starting, Some(id));
    let mut w = World::new();
    w.window_start_sent();
    // Generation 1 starts: its event, then `Starting`, reach the window.
    apply_effect_for(
        &mut w,
        3,
        1,
        "R101.15",
        Effect::EmitEvent(EventKind::GenStarted),
    );
    apply_effect_for(&mut w, 3, 1, "R101.15", emit(Status::Starting));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].latest_generation, 1);
    assert_eq!(w.windows[MAIN_WINDOW].ended_generation, 0);
    // It ends: the window receives `Idle`, with no reply processed yet.
    apply_effect_for(&mut w, 3, 1, "R102.3", emit(Status::Idle));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].ended_generation, 1);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Idle);
    // Its delayed `starting` reply is ignored: the window stays `Idle`.
    w.start_replied(start_reply(1));
    let (_, write) = w.deliver_start_reply().expect("a reply is pending");
    assert_eq!(write, ReplyWrite::EndedGeneration);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Idle);
    assert!(!w.windows[MAIN_WINDOW].start_in_flight);
    // The next generation's `starting` reply is not ended, nor is a reply with no recording id.
    apply_effect_for(
        &mut w,
        3,
        2,
        "R101.15",
        Effect::EmitEvent(EventKind::GenStarted),
    );
    w.start_replied(start_reply(2));
    assert_eq!(
        w.deliver_start_reply().map(|(_, write)| write),
        Some(ReplyWrite::Wrote(Status::Starting))
    );
    w.windows[MAIN_WINDOW].observed = Status::Idle;
    w.start_replied(reply(
        4,
        ReplyType::RecordingStarting,
        Status::Starting,
        None,
    ));
    assert_eq!(
        w.deliver_start_reply().map(|(_, write)| write),
        Some(ReplyWrite::Wrote(Status::Starting))
    );
    // `recovering` and `processing` end the newest generation the window knows, as `idle` does.
    for status in [Status::Recovering, Status::Processing] {
        let mut w = World::new();
        apply_effect_for(
            &mut w,
            3,
            7,
            "R101.15",
            Effect::EmitEvent(EventKind::GenStarted),
        );
        apply_effect_for(&mut w, 3, 7, "R101.15", emit(status));
        assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
        assert_eq!(w.windows[MAIN_WINDOW].ended_generation, 7, "{status:?}");
    }
    // A status event does not end a generation that started after it was emitted.
    let mut w = World::new();
    apply_effect_for(&mut w, 3, 1, "R101.15", emit(Status::Idle));
    apply_effect_for(
        &mut w,
        3,
        2,
        "R101.15",
        Effect::EmitEvent(EventKind::GenStarted),
    );
    apply_effect_for(&mut w, 3, 2, "R101.15", emit(Status::Starting));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].ended_generation, 0);
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].latest_generation, 2);
    assert_eq!(w.windows[MAIN_WINDOW].ended_generation, 0);
}

/// An `Err` reply is a failed invoke: `lib/dictation.ts` `startRecording` catches the rejection and
/// returns `{type: 'error', error}` with no `state`, so the window writes no status (the hook's outer
/// `catch`, which would write `idle`, is not reached) but its start operation still ends.
#[test]
fn an_error_response_writes_no_status_and_ends_the_start() {
    let mut w = World::new();
    w.windows[MAIN_WINDOW].observed = Status::Recording;
    w.window_start_sent();
    w.start_replied(StartReply {
        tid: 4,
        response: StartResponse::Error,
    });
    assert_eq!(
        w.deliver_start_reply().map(|(_, write)| write),
        Some(ReplyWrite::NoState)
    );
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Recording);
    assert!(!w.windows[MAIN_WINDOW].start_in_flight);
}

/// `entry::once` reports a `handle` decision that differs from the direct `decide_*` one, and is
/// silent when they agree.
#[test]
fn the_cross_check_reports_a_difference() {
    use crate::decision::Decision;
    let w = World::new();
    let view = w.view();
    let input = || {
        crate::inputs::Input::StartFlightEnd(crate::inputs::StartFlightEndInput {
            rid: crate::ids::RecordingId(1),
        })
    };
    let at = crate::ids::Now(0);
    let direct: Decision = crate::start::decide_start_flight_end(crate::ids::RecordingId(1), at);
    let decided = super::entry::once(&w.handle_faults, &view, input(), at, direct.clone());
    assert_eq!(decided, direct);
    assert!(w.handle_faults.borrow().is_empty());
    let other = crate::start::decide_start_flight_end(crate::ids::RecordingId(2), at);
    let decided = super::entry::once(&w.handle_faults, &view, input(), at, other);
    assert_eq!(decided, direct, "the `handle` decision is the one acted on");
    assert_eq!(w.handle_faults.borrow().len(), 1);
}

/// Step `task` over `w` (no other actor runs) until it finishes, blocks, or `max` steps ran:
/// the rows it decided, the observation rounds it answered, and whether it finished.
fn drive_rows(
    w: &mut World,
    task: &mut dyn Task,
    tid: TaskId,
    max: u32,
) -> (Vec<&'static str>, Vec<&'static str>, bool) {
    let mut rows = Vec::new();
    let mut observations = Vec::new();
    let mut out = StepOut::default();
    let mut spawned: Vec<Box<dyn Task>> = Vec::new();
    let mut mailbox = Vec::new();
    for step in 0..max {
        w.step = step;
        out.clear();
        let poll = task.step(&mut Cx {
            w,
            out: &mut out,
            spawn: &mut spawned,
            tid,
            mailbox: &mut mailbox,
        });
        assert!(out.violations.is_empty(), "{:?}", out.violations);
        rows.extend(out.decisions.iter().map(|decision| decision.row));
        observations.extend(out.observations.iter().copied());
        match poll {
            Poll::Done => return (rows, observations, true),
            Poll::Blocked => return (rows, observations, false),
            Poll::Progress => {}
        }
    }
    (rows, observations, false)
}

/// #1169 review: the Smart Auto observation round is part of the start gate. A Smart Auto start
/// asks for the cleanup check, then for the resolve, and is admitted only after the resolve
/// answered; a failed resolve is R101.14 and burns no id; Smart Auto with a fixed device is refused
/// before D (R101.2); a start with no Smart Auto never asks for the resolve.
#[test]
fn a_smart_auto_start_asks_for_the_selection_after_the_cleanup_check() {
    use super::flights::{Blockers, StartFlight};
    use super::gen::Mic;
    let run = |mic| {
        let mut w = World::new();
        let mut flight =
            StartFlight::new(external_attempt_plan(), Blockers::default(), false, false)
                .with_mic(mic);
        let (rows, observations, done) = drive_rows(&mut w, &mut flight, 4, 12);
        assert!(w.handle_faults.borrow().is_empty(), "{:?}", w.handle_faults);
        (rows, observations, done, w.current_id)
    };
    let (rows, observations, _, id) = run(Mic::SmartAutoResolved);
    assert_eq!(
        observations,
        ["R101.12", "R101.13"],
        "cleanup check, then resolve"
    );
    assert_eq!(rows.first(), Some(&"R101.15"));
    assert_eq!(id, 1, "admitted");
    let (rows, observations, done, id) = run(Mic::SmartAutoFailed);
    assert_eq!(observations, ["R101.12", "R101.13"]);
    assert_eq!(rows, ["R101.14"]);
    assert!(done);
    assert_eq!(id, 0, "a failed resolve burns no id");
    let (rows, observations, done, id) = run(Mic::SmartAutoWithFixed);
    assert_eq!(
        (rows.as_slice(), observations.as_slice()),
        (&["R101.2"][..], &[][..])
    );
    assert!(done);
    assert_eq!(id, 0);
    for mic in [Mic::SystemDefault, Mic::Explicit] {
        let (rows, observations, _, id) = run(mic);
        assert_eq!(observations, ["R101.12"], "{mic:?} asks no resolve");
        assert_eq!(rows.first(), Some(&"R101.15"));
        assert_eq!(id, 1);
    }
}

/// #1169 review: the legacy import (sites 301 and 302) is decided through `handle`. Its refusals
/// follow today's order (meeting, file, benchmark through the observation, status), the accepted
/// import allocates its id and writes `Processing`, and the parse failure finishes through
/// `GuardFinish` with `LegacyParse` (R302.1) and restores `Idle`.
#[test]
fn the_legacy_import_is_gated_through_handle() {
    use super::flights::{Blockers, ImportFlight, PipelineCfg, PipelineEnd};
    use super::gen::ImportPlan;
    let plan = |blockers: Blockers, parses: bool| ImportPlan {
        blockers,
        smart: false,
        parses,
        pipeline: PipelineCfg {
            work: 0,
            end: PipelineEnd::Err,
        },
    };
    let run = |blockers: Blockers, status: Status, parses: bool| {
        let mut w = World::new();
        w.status = status;
        let mut flight = ImportFlight::new(plan(blockers, parses));
        let (rows, observations, done) = drive_rows(&mut w, &mut flight, 4, 40);
        assert!(w.handle_faults.borrow().is_empty(), "{:?}", w.handle_faults);
        (rows, observations, done, w)
    };
    let blocked = |meeting, file, benchmark| Blockers {
        meeting,
        file,
        benchmark,
        ..Blockers::default()
    };
    let (rows, observations, done, _) = run(blocked(true, true, true), Status::Idle, true);
    assert_eq!(
        (rows.as_slice(), observations.len(), done),
        (&["R301.1"][..], 0, true)
    );
    let (rows, observations, done, _) = run(blocked(false, true, true), Status::Idle, true);
    assert_eq!(
        (rows.as_slice(), observations.len(), done),
        (&["R301.2"][..], 0, true)
    );
    let (rows, observations, done, _) = run(blocked(false, false, true), Status::Idle, true);
    assert_eq!(rows, ["R301.3"]);
    assert_eq!(
        observations,
        ["R301.3"],
        "the benchmark is read through the observation"
    );
    assert!(done);
    let (rows, observations, done, _) = run(Blockers::default(), Status::Recording, true);
    assert_eq!(
        (rows.as_slice(), observations.len(), done),
        (&["R301.4"][..], 1, true)
    );
    // Accepted, then the parse fails: R301.5, R302.1, `Idle` again, no history left.
    let (rows, observations, done, w) = run(Blockers::default(), Status::Idle, false);
    assert_eq!(rows, ["R301.5", "R302.1"]);
    assert_eq!(observations, ["R301.3"]);
    assert!(done);
    assert_eq!(w.current_id, 1, "the import allocated its id");
    assert_eq!(w.status, Status::Idle, "the parse failure restored Idle");
    assert!(w.history.ops.is_empty() && w.history.pipelines.is_empty());
    assert!(w.t_holder.is_none());
}

/// #1167: the main window's second stop is held back by `isStoppingRef` before either flight
/// runs (seed 456; the issue's seed 1454 before the re-pins moved every seed). The overlay, with no
/// guard, can send two (seed 162).
#[test]
fn a_second_main_window_stop_waits_for_the_first() {
    let (report, trace) = run_seed_with(gated(MAIN_SECOND_STOP_WAITS), true);
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    let sent = trace
        .iter()
        .position(|line| line.contains("frontend stop sent (main window shows Recording)"))
        .expect("the main window sends a stop");
    assert!(
        trace[sent + 1].contains("frontend stop not sent (main window has a stop in flight)"),
        "{trace:#?}"
    );
    let (_, overlay) = run_seed_with(gated(OVERLAY_TWO_STOPS), true);
    let count = |needle: &str| overlay.iter().filter(|line| line.contains(needle)).count();
    assert!(
        count("frontend stop sent (overlay window") >= 2,
        "{overlay:#?}"
    );
    assert_eq!(count("window has a stop in flight"), 0);
}

fn deliver(w: &mut World, window: usize) -> Poll {
    if w.windows[window].inbox.is_empty() {
        return Poll::Blocked;
    }
    w.deliver_status(window);
    Poll::Progress
}

/// Apply `effect` as row `row` of recording 1 (a step of task `tid`).
fn apply_effect(w: &mut World, tid: TaskId, row: &'static str, effect: Effect) {
    apply_effect_for(w, tid, 1, row, effect);
}

/// Apply `effect` as row `row` of recording `rid` (a step of task `tid`).
fn apply_effect_for(w: &mut World, tid: TaskId, rid: Rid, row: &'static str, effect: Effect) {
    let mut out = StepOut::default();
    let mut spawned: Vec<Box<dyn Task>> = Vec::new();
    let mut mailbox = Vec::new();
    let mut cx = Cx {
        w,
        out: &mut out,
        spawn: &mut spawned,
        tid,
        mailbox: &mut mailbox,
    };
    let fx = FxCtx {
        tid,
        rid,
        row,
        site: SiteId::SupIdle,
    };
    apply(&mut cx, &fx, Placement::In, &effect);
}

fn emit(status: Status) -> Effect {
    Effect::EmitStatus {
        status,
        cond: crate::effects::Cond::Always,
    }
}

/// D-S2b-17 (b): the window's observation is a separate step. The shell writes a status
/// and emits it; the window shows it only after its delivery step.
#[test]
fn a_status_reaches_the_window_in_a_step_of_its_own() {
    let mut w = World::new();
    apply_effect(&mut w, 1, "R209.2", Effect::SetStatus(Status::Recovering));
    apply_effect(&mut w, 1, "R209.2", emit(Status::Recovering));
    assert_eq!(w.status, Status::Recovering);
    assert_eq!(
        w.windows[MAIN_WINDOW].observed,
        Status::Idle,
        "emitted, not yet delivered"
    );
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Recovering);
    assert_eq!(
        deliver(&mut w, MAIN_WINDOW),
        Poll::Blocked,
        "nothing left to deliver"
    );
}

/// D-S2b-18 (1): the window queue follows *emits*, not writes. R209 writes `Recovering`,
/// releases the lock and pauses before its emit; R102.3 then writes and emits `Idle`; R209
/// emits `Recovering` last. A window receives `Idle -> Recovering` (with FIFO delivery) while
/// the shell's own status is `Idle`: an ordering the write order `Recovering -> Idle`
/// cannot represent.
#[test]
fn a_window_receives_statuses_in_emit_order_not_write_order() {
    let mut w = World::new();
    apply_effect(&mut w, 1, "R209.2", Effect::SetStatus(Status::Recovering));
    apply_effect(&mut w, 2, "R102.3", Effect::SetStatus(Status::Idle));
    apply_effect(&mut w, 2, "R102.3", emit(Status::Idle));
    assert!(
        w.windows[MAIN_WINDOW]
            .inbox
            .iter()
            .map(|event| event.status)
            .eq([Status::Idle]),
        "R209 has not emitted yet: {:?}",
        w.windows[MAIN_WINDOW].inbox
    );
    apply_effect(&mut w, 1, "R209.2", emit(Status::Recovering));
    for window in [MAIN_WINDOW, OVERLAY_WINDOW] {
        let mut seen = Vec::new();
        while deliver(&mut w, window) == Poll::Progress {
            seen.push(w.windows[window].observed);
        }
        assert_eq!(seen, [Status::Idle, Status::Recovering], "window {window}");
    }
    assert_eq!(w.status, Status::Idle, "the shell's own status is Idle");
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Recovering);
    // A write alone queues nothing.
    let mut w = World::new();
    apply_effect(&mut w, 1, "R209.2", Effect::SetStatus(Status::Recovering));
    assert!(w.windows.iter().all(|view| view.inbox.is_empty()));
}

/// D-S2b-18 (3): each window has its own delivery cursor. One window can have received
/// `Recovering` (and send no stop) while the other still shows `Recording` (and does).
#[test]
fn two_windows_can_disagree_about_the_status() {
    let mut w = World::new();
    apply_effect(&mut w, 1, "R101.15", Effect::SetStatus(Status::Recording));
    apply_effect(&mut w, 1, "R101.15", emit(Status::Recording));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(deliver(&mut w, OVERLAY_WINDOW), Poll::Progress);
    apply_effect(&mut w, 2, "R209.2", Effect::SetStatus(Status::Recovering));
    apply_effect(&mut w, 2, "R209.2", emit(Status::Recovering));
    assert_eq!(deliver(&mut w, MAIN_WINDOW), Poll::Progress);
    assert_eq!(w.windows[MAIN_WINDOW].observed, Status::Recovering);
    assert_eq!(
        w.windows[OVERLAY_WINDOW].observed,
        Status::Recording,
        "the overlay has not received it"
    );
    assert_eq!(w.windows[OVERLAY_WINDOW].inbox.len(), 1);
}

/// D-S2b-18 (3) in the seeded runs: a window sends a stop on `Recording` after the shell
/// entered `Recovering` while the other window has already received `Recovering` (and would
/// send none). One cursor could not represent both. The witness is pinned at seed 852
/// (re-pinned from 2,386 by #1160 and #1162, inside the 10,000-seed gate); the regular suite asserts nothing beyond that gate
/// (D-S2b-19 (a)).
#[test]
fn the_windows_disagree_in_the_seeded_runs() {
    const WITNESS: u64 = 852;
    let (report, trace) = run_seed_with(gated(WITNESS), true);
    assert!(
        report.violations.is_empty(),
        "{WITNESS}: {:?}",
        report.violations
    );
    assert!(
        trace.iter().any(|line| {
            line.contains("frontend stop issued on a stale observation")
                && line.contains("the other window shows Recovering")
        }),
        "seed {WITNESS} no longer has one window send a stop while the other had received Recovering"
    );
}

/// Delivery-first at a scheduler boundary (#1159, #1167): `window` receives `Recording` at the
/// boundary of its *own* due stop, with only more deliveries before that same stop at that
/// boundary, and then sends it. A delivery before another window's stop, or before a due
/// start or cancel, does not count.
fn delivery_first(trace: &[String], window: &str) -> bool {
    let before = format!("(before the due stop from the {window} window)");
    let received = format!("{window} window receives Recording {before}");
    let sent = format!("frontend stop sent ({window} window shows Recording)");
    (0..trace.len())
        .filter(|at| trace[*at].contains(&received))
        .any(|at| {
            trace[at + 1..]
                .iter()
                .find(|line| !line.contains(&before))
                .is_some_and(|line| line.contains(&sent))
        })
}

/// Stop-first at a scheduler boundary (#1159, #1167): the due stop from `window` runs while
/// `Recording` is still queued for that window, so it shows `Starting` and sends nothing,
/// and that window did not receive `Recording` between the emit and the dropped stop. The
/// other window may have: only the caller's own delivery is what the stop ran ahead of.
fn stop_first(trace: &[String], window: &str) -> bool {
    let dropped = format!(
        "frontend stop not sent ({window} window shows Starting) while Recording is still queued for it"
    );
    let own_receive = format!("{window} window receives Recording");
    (0..trace.len())
        .filter(|at| trace[*at].contains(&dropped))
        .any(|at| {
            trace[..at]
                .iter()
                .rposition(|line| line.contains("fx R104.2 In EmitStatus { status: Recording"))
                .is_some_and(|emit| {
                    !trace[emit..at]
                        .iter()
                        .any(|line| line.contains(&own_receive))
                })
        })
}

/// #1159, D-S2b-19 (c): at a scheduler boundary a due command and a ready delivery compete,
/// and the run's RNG picks the order, so production's two orders are both explored. This
/// is the delivery-first side: the caller's window receives `Recording` at the boundary of
/// its own due stop and only then sends it (the model used to run the due stop first and drop
/// it). Matching the caller and the boundary (#1167) keeps another window's delivery, or one
/// before a due start, from satisfying it. Both windows are pinned.
#[test]
fn a_ready_delivery_can_run_before_a_due_stop() {
    for (window, seed) in [
        ("main", MAIN_DELIVERY_FIRST),
        ("overlay", OVERLAY_DELIVERY_FIRST),
    ] {
        let (report, trace) = run_seed_with(gated(seed), true);
        assert!(
            report.violations.is_empty(),
            "{seed}: {:?}",
            report.violations
        );
        assert!(
            delivery_first(&trace, window),
            "seed {seed} no longer delivers `Recording` to the {window} window before its own due stop: {trace:#?}"
        );
    }
}

/// The stop-first side of the same competition, the shape of the issue's seed 473: R104.2
/// emits `Recording` and the caller's due stop runs before its window receives it, so the
/// window still shows `Starting` and sends nothing. The other window receiving `Recording`
/// first does not change that, so a trace where only the other window received it is
/// accepted (#1167). Both windows are pinned.
#[test]
fn a_due_stop_can_run_before_a_ready_delivery() {
    for (window, seed) in [("main", MAIN_STOP_FIRST), ("overlay", OVERLAY_STOP_FIRST)] {
        let (report, trace) = run_seed_with(gated(seed), true);
        assert!(
            report.violations.is_empty(),
            "{seed}: {:?}",
            report.violations
        );
        assert!(
            stop_first(&trace, window),
            "seed {seed} no longer runs the {window} window's due stop before its `Recording` delivery: {trace:#?}"
        );
    }
}

/// The matchers name the caller and the boundary: each rejects the mismatch it was written
/// to reject (#1167, seed 828: the main window received `Recording` and the overlay sent a
/// stop on its earlier delivery) and accepts the valid trace the old stop-first check rejected.
#[test]
fn the_ordering_matchers_name_their_caller_and_boundary() {
    let lines = |lines: &[&str]| {
        lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
    };
    // Main receives at a boundary of the overlay's stop; the overlay sends on an earlier
    // delivery: neither window's delivery-first holds.
    let mismatched = lines(&[
        "   9 overlay window receives Recording",
        "  37 main window receives Recording (before the due stop from the overlay window)",
        "  37 frontend stop sent (overlay window shows Recording)",
    ]);
    assert!(!delivery_first(&mismatched, "main"));
    assert!(!delivery_first(&mismatched, "overlay"));
    // A delivery before a due start is not a delivery before the stop.
    let other_command = lines(&[
        "  37 main window receives Recording (before the due start)",
        "  37 frontend stop sent (main window shows Recording)",
    ]);
    assert!(!delivery_first(&other_command, "main"));
    let matched = lines(&[
        "  37 overlay window receives Recording (before the due stop from the main window)",
        "  37 main window receives Recording (before the due stop from the main window)",
        "  37 frontend stop sent (main window shows Recording)",
    ]);
    assert!(delivery_first(&matched, "main"));
    // Only the overlay received `Recording` first: the main window's stop still ran ahead of
    // its own delivery.
    let overlay_only = lines(&[
        "  30 t3   fx R104.2 In EmitStatus { status: Recording, cond: Always } rid=1 Recording->Recording claim=None",
        "  31 overlay window receives Recording",
        "  32 frontend stop not sent (main window shows Starting) while Recording is still queued for it",
    ]);
    assert!(stop_first(&overlay_only, "main"));
    assert!(!stop_first(&overlay_only, "overlay"));
    let main_received = lines(&[
        "  30 t3   fx R104.2 In EmitStatus { status: Recording, cond: Always } rid=1 Recording->Recording claim=None",
        "  31 main window receives Recording",
        "  32 frontend stop not sent (main window shows Starting) while Recording is still queued for it",
    ]);
    assert!(!stop_first(&main_received, "main"));
}

/// The number after `prefix` in `line` (a task id: `... as t7`).
fn id_after(line: &str, prefix: &str) -> Option<u32> {
    let rest = &line[line.find(prefix)? + prefix.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

const MAIN_START: &str = "start issued by the main window as t";
const MAIN_STOP: &str = "frontend stop sent (main window shows Recording) as t";

/// #1162: the issue's sequence, read from a trace and tied to one start and one stop (#1169
/// review). The main window issues start `S`; it sends a stop `F` while `S` is still in flight
/// (`startOperationRef` set, so it issued no second start); `S`'s own reply is processed while `F`
/// is still in flight and puts `Recording` over `F`'s local `Processing`; `F` returns; and the
/// window, which has received no `Idle` since, sends a second stop. The reply must be `S`'s, must
/// change the window's status from `Processing` to `Recording`, and must precede `F`'s return.
fn late_start_reply_second_stop(trace: &[String]) -> bool {
    let after = |from: usize, pred: &dyn Fn(&str) -> bool| {
        trace[from..]
            .iter()
            .position(|line| pred(line))
            .map(|offset| from + offset)
    };
    (0..trace.len()).any(|issue| {
        let Some(start) = id_after(&trace[issue], MAIN_START) else {
            return false;
        };
        let Some(first_stop) = after(issue + 1, &|line| line.contains(MAIN_STOP)) else {
            return false;
        };
        let Some(stop) = id_after(&trace[first_stop], MAIN_STOP) else {
            return false;
        };
        let reply_of = format!("main window receives the reply of its start t{start} (");
        let Some(reply) = after(first_stop + 1, &|line| {
            line.contains(&reply_of)
                && line.contains("writes Recording; the window shows Processing -> Recording")
        }) else {
            return false;
        };
        let returned = format!("WindowStopReturned {{ window: 0, tid: {stop} }}");
        let Some(returned) = after(first_stop + 1, &|line| line.contains(&returned)) else {
            return false;
        };
        // The guard coalesces a second start gesture: no other main start before the reply,
        // and the reply is processed before the first stop returns.
        let one_start = !trace[issue + 1..reply]
            .iter()
            .any(|line| line.contains(MAIN_START));
        let Some(second) = after(returned + 1, &|line| {
            line.contains(MAIN_STOP) && id_after(line, MAIN_STOP) != Some(stop)
        }) else {
            return false;
        };
        one_start
            && reply < returned
            && !trace[first_stop..second]
                .iter()
                .any(|line| line.contains("main window receives Idle"))
    })
}

/// #1162: the main window's start reply is a step of its own, so it can arrive while the main
/// window's stop is in flight and put `Recording` back: the second gesture is then sent, where the
/// model that never delivered a start reply dropped it (the guard or the `Processing` observation
/// refused it).
#[test]
fn a_late_start_reply_lets_the_main_window_send_a_second_stop() {
    let seed = gated(LATE_START_REPLY_SECOND_STOP);
    let (report, trace) = run_seed_with(seed, true);
    assert!(
        report.violations.is_empty(),
        "{seed}: {:?}",
        report.violations
    );
    assert!(
        late_start_reply_second_stop(&trace),
        "seed {seed} no longer sends the second stop after the late start reply: {trace:#?}"
    );
    // The run's own counters agree: the reply overwrote `Processing`, and the second stop followed.
    assert!(report.hits.reply_over_processing > 0, "{:?}", report.hits);
    assert!(
        report.hits.second_stop_after_late_reply > 0,
        "{:?}",
        report.hits
    );
}

/// The matcher needs every step of the sequence, tied to the same start and the same stop.
#[test]
fn the_late_reply_matcher_needs_the_whole_sequence() {
    let full: Vec<String> = [
        "   0 start issued by the overlay window as t3",
        "   3 start issued by the main window as t4",
        "  30 t3   fx R104.2 In EmitStatus { status: Recording, cond: Always } rid=1 Recording->Recording claim=None",
        "  34 frontend stop sent (main window shows Recording) as t5",
        "  35 main window receives the reply of its start t4 (already_recording, state Recording): writes Recording; the window shows Processing -> Recording",
        "  36 frontend start coalesced (the main window's start is still in flight)",
        "     note WindowStopReturned { window: 0, tid: 5 }",
        "  40 frontend stop sent (main window shows Recording) as t6",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    assert!(late_start_reply_second_stop(&full));
    // Each of the start, the first stop, the reply, the stop's return and the second stop is
    // needed.
    for skip in [1, 3, 4, 6, 7] {
        let mut partial = full.clone();
        partial.remove(skip);
        assert!(
            !late_start_reply_second_stop(&partial),
            "accepted without line {skip}"
        );
    }
    let with = |index: usize, line: &str| {
        let mut changed = full.clone();
        changed[index] = line.to_string();
        changed
    };
    // The reply of another start (`t40` is not `t4`) is not this start's.
    assert!(!late_start_reply_second_stop(&with(
        4,
        "  35 main window receives the reply of its start t40 (already_recording, state Recording): writes Recording; the window shows Processing -> Recording"
    )));
    // A reply that changes nothing (the write suppressed, or the window not on `Processing`).
    assert!(!late_start_reply_second_stop(&with(
        4,
        "  35 main window receives the reply of its start t4 (already_recording, state Recording): writes Recording; the window shows Processing -> Processing"
    )));
    // A reply that does not write (not the owner, stale, an ended generation).
    assert!(!late_start_reply_second_stop(&with(
        4,
        "  35 main window receives the reply of its start t4 (already_starting, state Idle): not the owner of the state; the window shows Processing -> Processing"
    )));
    // The reply must come before the first stop returns, not after.
    let mut late = full.clone();
    late.swap(4, 6);
    assert!(!late_start_reply_second_stop(&late));
    // The first stop's return is the stop's own (`t5`), not another stop's.
    assert!(!late_start_reply_second_stop(&with(
        6,
        "     note WindowStopReturned { window: 0, tid: 9 }"
    )));
    // A second main-window start before the reply: the guard would have coalesced it.
    let mut second_start = full.clone();
    second_start.insert(4, "  34 start issued by the main window as t7".to_string());
    assert!(!late_start_reply_second_stop(&second_start));
    // The main window received `Idle` between the stops.
    let mut idle = full.clone();
    idle.insert(7, "  37 main window receives Idle".to_string());
    assert!(!late_start_reply_second_stop(&idle));
    // The second stop is another stop, not the first again.
    assert!(!late_start_reply_second_stop(&with(
        7,
        "  40 frontend stop sent (main window shows Recording) as t5"
    )));
}

/// The competition draws only when a command and a delivery are both ready, and it is part
/// of the seed: the same seed gives the same order.
#[test]
fn the_competition_is_deterministic_per_seed() {
    for seed in [
        MAIN_DELIVERY_FIRST,
        OVERLAY_DELIVERY_FIRST,
        MAIN_STOP_FIRST,
        OVERLAY_STOP_FIRST,
    ] {
        let seed = gated(seed);
        let (_, first) = run_seed_with(seed, true);
        let (_, second) = run_seed_with(seed, true);
        assert_eq!(first, second, "seed {seed}");
    }
}

/// Mutation check on the harness's own scheduler: a blocked step must change
/// nothing in the world (the README's "poll once, do not follow the self-wake").
#[test]
fn a_blocked_step_changes_nothing() {
    for seed in 0..300u64 {
        let report = run_seed_strict(gated(seed));
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
