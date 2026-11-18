//! The C3 effect fixture and the per-decision check against it (SP3, D-S2b-9 (d)).
//!
//! `c3_fixture.txt` is transcribed by hand from appendix C3 (C3.1-C3.7): one line per
//! row of [`crate::rows::C3_ROWS`], the statuses the row reads and each outcome the row
//! may have (the status writes, terminals and performance completions it carries). It
//! is **not** generated from the core and not a snapshot of what a run observed, so an
//! input the model never reaches is still valid, and a row the model reaches with an
//! outcome the appendix does not list fails.
//!
//! [`check_decision`] compares **one decision's whole effect list** with the row: the
//! counted effects must equal an alternative exactly, each as often as the appendix
//! says, so a duplicate terminal, an extra `Idle -> Idle` write, a missing performance
//! completion or a terminal claiming another outcome all fail even when each effect
//! alone is listed. Two things stay out of this file on purpose: SP5 (an effect that
//! reads the shell's own state) is structural and enforced by the #1152 ratchet, and
//! file admission is a unit test of the admission crate.

use crate::decision::{Decision, SiteId};
use crate::effects::Effect;
use crate::rows::C3_ROWS;
use crate::status::Status;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const FIXTURE: &str = include_str!("c3_fixture.txt");

/// Whether the alternative applies to the current recording's input, another one's, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subject {
    Fresh,
    Stale,
    Either,
}

/// One allowed outcome of a row: the counted effects (`ST:A>B`, `TERM:o/c`, `PC:o`,
/// `PGF:o`; an argument may be `*` or `a|b`).
#[derive(Clone, Debug)]
struct Alternative {
    subject: Subject,
    tokens: Vec<String>,
}

/// One row of the fixture.
#[derive(Clone, Debug)]
pub struct FixtureRow {
    /// `None`: the row does not read status (`*`).
    statuses: Option<Vec<String>>,
    alternatives: Vec<Alternative>,
}

/// Parse the fixture text.
fn parse(text: &str) -> Result<BTreeMap<String, FixtureRow>, String> {
    let mut rows = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cells = line.splitn(3, '|').map(str::trim);
        let (Some(row), Some(statuses), Some(alternatives)) =
            (cells.next(), cells.next(), cells.next())
        else {
            return Err(format!("line {}: expected `row | statuses | alternatives`", number + 1));
        };
        let statuses = (statuses != "*")
            .then(|| statuses.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>());
        let alternatives = alternatives
            .split(" / ")
            .map(|alternative| {
                let mut words = alternative.split_whitespace().peekable();
                let subject = match words.peek().copied() {
                    Some("[fresh]") => {
                        words.next();
                        Subject::Fresh
                    }
                    Some("[stale]") => {
                        words.next();
                        Subject::Stale
                    }
                    _ => Subject::Either,
                };
                let tokens: Vec<String> = words.filter(|word| *word != "-").map(String::from).collect();
                Alternative { subject, tokens }
            })
            .collect::<Vec<_>>();
        if rows
            .insert(
                row.to_string(),
                FixtureRow {
                    statuses,
                    alternatives,
                },
            )
            .is_some()
        {
            return Err(format!("line {}: row {row} listed twice", number + 1));
        }
    }
    Ok(rows)
}

fn fixture() -> &'static BTreeMap<String, FixtureRow> {
    static ROWS: OnceLock<BTreeMap<String, FixtureRow>> = OnceLock::new();
    ROWS.get_or_init(|| parse(FIXTURE).expect("c3_fixture.txt parses"))
}

/// The counted effects of a decision, as fixture tokens. `from` is the status the
/// decision read: every `ST` of the list is written over it.
pub fn tokens(effects: impl IntoIterator<Item = Effect>, from: Status) -> Vec<String> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::SetStatus(to) => Some(format!("ST:{from:?}>{to:?}")),
            Effect::Terminal(t) => Some(format!("TERM:{:?}/{:?}", t.outcome, t.code)),
            Effect::PerfComplete(c) => Some(format!("PC:{:?}", c.outcome)),
            Effect::PipelineGuardFinish(outcome) => Some(format!("PGF:{outcome:?}")),
            _ => None,
        })
        .collect()
}

/// A fixture token matches an observed one: equal, or `*` / `a|b` in the argument.
fn token_matches(pattern: &str, observed: &str) -> bool {
    if pattern == observed {
        return true;
    }
    let (Some((kind, args)), Some((observed_kind, observed_args))) =
        (pattern.split_once(':'), observed.split_once(':'))
    else {
        return false;
    };
    if kind != observed_kind {
        return false;
    }
    // `TERM:outcome/code` compares the two halves; `ST:from>to` and the rest compare whole.
    let (patterns, observed): (Vec<&str>, Vec<&str>) = match kind {
        "TERM" => (args.split('/').collect(), observed_args.split('/').collect()),
        _ => (vec![args], vec![observed_args]),
    };
    patterns.len() == observed.len()
        && patterns
            .iter()
            .zip(&observed)
            .all(|(p, o)| *p == "*" || p.split('|').any(|alternative| alternative == *o))
}

/// Match every pattern to a distinct observed token and the reverse (the same counts).
fn multiset_matches(patterns: &[String], observed: &[String]) -> bool {
    fn go(patterns: &[String], observed: &mut Vec<String>) -> bool {
        let Some((first, rest)) = patterns.split_first() else {
            return observed.is_empty();
        };
        for index in 0..observed.len() {
            if token_matches(first, &observed[index]) {
                let taken = observed.remove(index);
                if go(rest, observed) {
                    observed.insert(index, taken);
                    return true;
                }
                observed.insert(index, taken);
            }
        }
        false
    }
    patterns.len() == observed.len() && go(patterns, &mut observed.to_vec())
}

/// Check a decision against its fixture row. `stale`: the input names a recording that
/// is not the current one at the decision.
pub fn check_decision(
    row: &str,
    status: Status,
    stale: bool,
    effects: &[Effect],
) -> Result<(), String> {
    let Some(fixture_row) = fixture().get(row) else {
        return Err(format!("C3: row {row} is not in the C3 fixture"));
    };
    let observed = tokens(effects.iter().cloned(), status);
    if let Some(statuses) = &fixture_row.statuses {
        if !statuses.iter().any(|s| *s == format!("{status:?}")) {
            return Err(format!(
                "C3: {row} decided with status {status:?}, C3 lists {statuses:?}"
            ));
        }
    }
    let applicable = fixture_row.alternatives.iter().filter(|alternative| match alternative.subject {
        Subject::Fresh => !stale,
        Subject::Stale => stale,
        Subject::Either => true,
    });
    if applicable
        .clone()
        .any(|alternative| multiset_matches(&alternative.tokens, &observed))
    {
        return Ok(());
    }
    Err(format!(
        "C3: {row} ({} input, status {status:?}) has the effects {observed:?}; C3 allows {:?}",
        if stale { "stale" } else { "current" },
        applicable.map(|a| a.tokens.clone()).collect::<Vec<_>>()
    ))
}

/// The decision form of [`check_decision`].
pub fn check(decision: &Decision, status: Status, stale: bool) -> Result<(), String> {
    let effects: Vec<Effect> = decision.effects.iter().map(|p| p.effect.clone()).collect();
    check_decision(decision.row.0, status, stale, &effects)
}

/// The rows of the fixture, for the coverage test.
pub fn fixture_rows() -> Vec<&'static str> {
    fixture().keys().map(String::as_str).collect()
}

/// The statuses a fixture row reads (`None`: any).
pub fn fixture_statuses(row: &str) -> Option<Option<Vec<String>>> {
    fixture().get(row).map(|r| r.statuses.clone())
}

/// The site that owns a fixture row.
pub fn site_of(row: &str) -> Option<SiteId> {
    C3_ROWS.iter().find(|(id, _)| *id == row).map(|(_, site)| *site)
}
