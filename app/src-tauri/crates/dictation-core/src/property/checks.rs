//! The prefix check (SP1-SP10 after every step), the preserved-defect detectors
//! (C7.1: an allowlisted violation is removed from the failure set only when its
//! signature matches exactly) and the end-of-run liveness check (C7.3).

use super::model::*;
use super::task::*;
use crate::effects::Effect;
use crate::status::{Status, TerminalOutcome};
use std::collections::{BTreeMap, BTreeSet};

/// A preserved defect occurrence seen in one step.
/// Fields are read through `Debug` in traces.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub pd: u8,
    pub row: &'static str,
    pub rid: Rid,
}

/// PD-18's instances (C7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instance {
    /// S12: an `Interrupted` end under 8,000 samples.
    A,
    /// The `Idle`-arm tail.
    B,
    /// An unsealed delivered stop.
    C,
    /// An `Interrupted` end of 8,000 samples or more whose spawned stop is refused
    /// (R201.6): the appendix names no such instance (a finding).
    Other,
}

#[derive(Debug, Default, Clone)]
pub struct RunHits {
    /// Occurrences per PD number (index 0 unused).
    pub pd: [u32; 19],
    /// PD-18 instances of runs that ended stuck in `Recovering` (a, b, c).
    pub pd18: [u32; 4],
    /// Behaviours the C7.1 allowlist does not name (F-1..F-3, index 1..3): see
    /// [`Finding`]. They are counted here and are **not** accepted silently: the
    /// suite reports each one and pins the known set.
    pub findings: [u32; 4],
}

/// A violation of a property that no C7.1 row covers, reproduced by the harness
/// and classified by its exact signature. Each is a candidate preserved defect
/// (or a spec gap in C7.2's `Exempt` column) awaiting George's decision; none is
/// fixed here (test-only PR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finding {
    /// F-1: the cancel at `Processing` (R205.5) and another completing path split the
    /// terminal and the performance outcome between them (SP4(b)).
    CancelTailSplit,
    /// F-2: a failed capture cancel's `TERM(PipelineFailure)` (R206.2) wins while the
    /// recording goes on and a later path completes performance (SP4(b)).
    FailedCancelTerminal,
    /// F-3: a supervisor `Idle` for a finished current id emits the winning
    /// `Superseded` (R211.2, PD-2) while another path completes performance (SP4(b)).
    SupersededWinner,
}

impl Finding {
    pub fn id(self) -> &'static str {
        match self {
            Self::CancelTailSplit => "F-1",
            Self::FailedCancelTerminal => "F-2",
            Self::SupersededWinner => "F-3",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::CancelTailSplit => 1,
            Self::FailedCancelTerminal => 2,
            Self::SupersededWinner => 3,
        }
    }

    /// The signature: the row that won the terminal claim and the row that completed
    /// performance first.
    fn classify(terminal_row: &str, perf_row: &str) -> Option<Self> {
        if terminal_row == "R211.2" {
            Some(Self::SupersededWinner)
        } else if terminal_row == "R206.2" {
            Some(Self::FailedCancelTerminal)
        } else if terminal_row == "R205.5" || perf_row == "R205.5" {
            Some(Self::CancelTailSplit)
        } else {
            None
        }
    }
}

#[derive(Debug, Default)]
pub struct Checker {
    pub violations: Vec<String>,
    pub hits: RunHits,
    /// Recordings each PD touched, for the end-of-run checks.
    pd_ids: BTreeMap<u8, BTreeSet<Rid>>,
    /// R101.12 decisions taken after the last restorer, by instance.
    pd18_candidates: Vec<Instance>,
    prev_current: Rid,
    prev_ticker: Rid,
    /// A start future was dropped / a task panicked (liveness exemptions).
    pub dropped: bool,
    /// Recordings whose start future was dropped at its await.
    dropped_ids: BTreeSet<Rid>,
    pub panicked: bool,
    pub steps_checked: u32,
    /// A readable event trace (for reproducing a seed by hand).
    pub trace: Option<Vec<String>>,
}

const SP2_EXEMPT: [u8; 5] = [1, 4, 5, 7, 16];

fn edge_allowed(from: Status, to: Status) -> bool {
    use Status::*;
    matches!(
        (from, to),
        (Idle, Starting)
            | (Starting, Recording)
            | (Starting, Idle)
            | (Starting, Recovering)
            | (Recording, Processing)
            | (Recording, Recovering)
            | (Recording, Idle)
            | (Recovering, Idle)
            | (Recovering, Processing)
            | (Processing, Idle)
            | (Idle, Recovering)
            | (Idle, Processing)
            | (Idle, Idle)
    )
}

impl Checker {
    fn hit(&mut self, step: &mut Vec<Hit>, pd: u8, row: &'static str, rid: Rid) {
        self.hits.pd[usize::from(pd)] += 1;
        self.pd_ids.entry(pd).or_default().insert(rid);
        step.push(Hit { pd, row, rid });
    }

    fn fail(&mut self, message: String) {
        self.violations.push(message);
    }

    /// Detect the PDs of one step, then check SP1-SP10 on its events.
    pub fn on_step(&mut self, w: &World, out: &StepOut) {
        self.steps_checked += 1;
        if let Some(trace) = self.trace.as_mut() {
            for d in &out.decisions {
                trace.push(format!(
                    "{:>4} t{} DECIDE {} {:?} in={} cur={} st={:?} owner={:?} phase={:?} cleanup={} restorer={} ended={:?}",
                    d.step, d.tid, d.row, d.verdict, d.input_rid, d.current_before,
                    d.status_before, d.owner, d.phase, d.cleanup_pending, d.restorer_pending, d.ended_by
                ));
            }
            for e in &out.effects {
                trace.push(format!(
                    "{:>4} t{}   fx {} {} {:?} rid={} {:?}->{:?} claim={:?}",
                    e.step,
                    e.tid,
                    e.row,
                    e.at.label(),
                    e.effect,
                    e.input_rid,
                    e.status_before,
                    e.status_after,
                    e.claim
                ));
            }
            for n in &out.notes {
                trace.push(format!("     note {n:?}"));
            }
        }
        for violation in &out.violations {
            self.fail(violation.clone());
        }
        let mut hits: Vec<Hit> = Vec::new();
        for d in &out.decisions {
            self.detect_decision(d, &mut hits);
        }
        for e in &out.effects {
            if let Effect::Terminal(_) = e.effect {
                if e.claim == Some(false) && !e.accepted {
                    self.hit(&mut hits, 10, e.row, e.input_rid);
                }
            }
        }
        for note in &out.notes {
            self.detect_note(w, note, &mut hits);
        }
        for e in &out.effects {
            self.check_effect(e, &hits);
        }
        // SP6: ids.
        if w.current_id < self.prev_current {
            self.fail(format!(
                "SP6: current_id fell {} -> {}",
                self.prev_current, w.current_id
            ));
        }
        self.prev_current = w.current_id;
        if w.cancelled_through > w.current_id {
            self.fail(format!(
                "SP6: cancelled_through {} above current {}",
                w.cancelled_through, w.current_id
            ));
        }
        // SP10: the partial-decode lease is never released.
        if w.ticker_started < self.prev_ticker {
            self.fail("SP10: ticker_started was lowered".into());
        }
        self.prev_ticker = w.ticker_started;
        // SP1: the ledger.
        for (rid, wins) in &w.ledger.wins {
            if *wins > 1 || !w.ledger.accepted.contains(rid) {
                self.fail(format!("SP1: id {rid} was claimed {wins} times"));
            }
        }
        // SP7 high-water values are diagnostics, not limits.
    }

    fn detect_decision(&mut self, d: &DecisionEv, hits: &mut Vec<Hit>) {
        let row = d.row;
        let rid = d.input_rid;
        let from_to = d.transition;
        match row {
            "R216.3" | "R216.4" | "R216.5" => self.hit(hits, 1, row, rid),
            "R211.2" => self.hit(hits, 2, row, rid),
            "R207.3" => self.hit(hits, 4, row, rid),
            "R207.2" | "R209.2" => self.hit(hits, 5, row, rid),
            "R211.3" => self.hit(hits, 7, row, rid),
            "R103.2" => self.hit(hits, 8, row, rid),
            "R201.1" => self.hit(hits, 12, row, rid),
            _ => {}
        }
        // PD-3: the idle emit and `set_processing(false)` run whatever the status was:
        // the capture-start failure on a status other than `Starting`, and the
        // context-less stop on `Idle`.
        if matches!(row, "R102.3" | "R102.4" | "R102.5" | "R102.6" | "R202.4") {
            self.hit(hits, 3, row, rid);
        }
        if matches!(
            row,
            "R204a.2"
                | "R204a.3"
                | "R204b.2"
                | "R203.2"
                | "R203.2r"
                | "R203.5r"
                | "R203.7r"
                | "R102.3"
        ) && matches!(
            from_to,
            Some((Status::Idle | Status::Recovering, Status::Idle))
        ) {
            self.hit(hits, 6, row, rid);
        }
        if d.in_cancel_tail && is_supervisor_site(d.site) {
            self.hit(hits, 16, row, rid);
        }
        if row == "R101.12" && d.cleanup_pending && !d.restorer_pending && !d.pipeline_live {
            let instance = match d.ended_by {
                Some(EndedBy::Recover {
                    interrupted: Some(samples),
                }) if samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES => Instance::A,
                Some(EndedBy::Recover {
                    interrupted: Some(_),
                }) => Instance::Other,
                Some(EndedBy::Stop(Delivery::Unsealed)) => Instance::C,
                _ => Instance::B,
            };
            self.pd18_candidates.push(instance);
        }
    }

    fn detect_note(&mut self, _w: &World, note: &Note, hits: &mut Vec<Hit>) {
        match note {
            Note::Take {
                found: false,
                earlier: Some(_),
                rid,
                row,
                ..
            } => self.hit(hits, 15, row, *rid),
            Note::StartDropped {
                rid,
                registered,
                begun,
            } => {
                self.dropped = true;
                self.dropped_ids.insert(*rid);
                if *registered && !*begun {
                    self.hit(hits, 9, "R101.15", *rid);
                }
            }
            Note::TaskPanicked { rid, .. } => {
                self.panicked = true;
                self.hit(hits, 13, "R204a.6", *rid);
            }
            Note::TickerPanicked { rid } => self.hit(hits, 14, "SD", *rid),
            // The claim of this `TERM` was lost (to an earlier winner, or because the id
            // was never accepted) yet its `complete` was the first: `D` and `T` come
            // from different `TERM` calls.
            Note::DiagnosticsCompleted {
                rid,
                by,
                first: true,
                claim_winner,
            } if claim_winner.as_ref() != Some(by) => self.hit(hits, 11, "TERM", *rid),
            _ => {}
        }
    }

    fn exempt(hits: &[Hit], pds: &[u8], row: &str) -> bool {
        hits.iter()
            .any(|hit| pds.contains(&hit.pd) && (hit.row == row || hit.row == "TERM"))
    }

    fn check_effect(&mut self, e: &EffectEv, hits: &[Hit]) {
        if let Effect::SetStatus(to) = e.effect {
            // SP3: the edge set.
            if !edge_allowed(e.status_before, to) {
                self.fail(format!(
                    "SP3: {} wrote {:?} -> {:?}",
                    e.row, e.status_before, to
                ));
            }
            // SP2: a stale input never changes the current recording's status.
            if e.input_rid != e.current && !Self::exempt(hits, &SP2_EXEMPT, e.row) {
                self.fail(format!(
                    "SP2: {} (input {}) wrote {:?} while current is {}",
                    e.row, e.input_rid, to, e.current
                ));
            }
        }
    }

    /// End-of-run: SP4 (outcome agreement), liveness (C7.3) and the converse of the
    /// L4-L6 filter. Returns `true` when the run was filtered out of the liveness
    /// check (a recording nobody stops, a panicking pipeline, a dropped start).
    pub fn on_end(&mut self, w: &World, quiescent_tasks: &[&'static str]) -> bool {
        self.sp4(w);
        let l4 = w.sup.attempt.as_ref().is_some_and(|a| a.awaiting_external);
        let filtered = l4 || self.panicked;
        if filtered {
            return true;
        }
        let mut problems: Vec<String> = Vec::new();
        if !quiescent_tasks.is_empty() {
            problems.push(format!("tasks never finished: {quiescent_tasks:?}"));
        }
        if w.t_holder.is_some() {
            problems.push("T is still held".into());
        }
        let stuck_recovering = w.status == Status::Recovering && !self.pd18_candidates.is_empty();
        if w.status != Status::Idle && !(stuck_recovering || self.dropped) {
            problems.push(format!("status {:?} at the end", w.status));
        }
        if stuck_recovering {
            // PD-18: one witness per instance the run took.
            let mut seen = [false; 4];
            for instance in &self.pd18_candidates {
                let index = match instance {
                    Instance::A => 0,
                    Instance::B => 1,
                    Instance::C => 2,
                    Instance::Other => 3,
                };
                if !seen[index] {
                    seen[index] = true;
                    self.hits.pd18[index] += 1;
                }
            }
            self.hits.pd[18] += 1;
        }
        for rid in &w.ledger.accepted {
            if !w.ledger.claimed.contains_key(rid)
                && !self.pd_touched(13, *rid)
                && !self.dropped_ids.contains(rid)
            {
                problems.push(format!("id {rid} accepted but never claimed"));
            }
        }
        if !w.history.ops.is_empty() {
            problems.push(format!("ops unresolved: {:?}", w.history.ops));
        }
        if !w.history.pipelines.is_empty() {
            problems.push(format!("pipelines unresolved: {:?}", w.history.pipelines));
        }
        for problem in problems {
            self.fail(format!("liveness: {problem}"));
        }
        false
    }

    fn pd_touched(&self, pd: u8, rid: Rid) -> bool {
        self.pd_ids.get(&pd).is_some_and(|ids| ids.contains(&rid))
    }

    fn sp4(&mut self, w: &World) {
        for (rid, (outcome, _, _)) in &w.ledger.claimed {
            if let Some(diag) = w.ledger.diagnostics.get(rid) {
                if diag != outcome && !self.pd_touched(11, *rid) {
                    self.fail(format!(
                        "SP4(a): id {rid} claimed {outcome:?} but diagnostics took {diag:?}"
                    ));
                }
            }
            if let Some((perf, perf_row)) = w.perf.outcome.get(rid) {
                let exempt = [4u8, 5, 11, 13].iter().any(|pd| self.pd_touched(*pd, *rid));
                if !perf_class_allowed(*outcome, *perf) && !exempt {
                    let terminal_row = w.ledger.claimed[rid].2;
                    match Finding::classify(terminal_row, perf_row) {
                        Some(finding) => self.hits.findings[finding.index()] += 1,
                        None => self.fail(format!(
                            "SP4(b): terminal {outcome:?} by {terminal_row} but performance {perf:?} by {perf_row}"
                        )),
                    }
                }
            }
        }
    }
}

fn is_supervisor_site(site: crate::decision::SiteId) -> bool {
    use crate::decision::SiteId::*;
    matches!(
        site,
        SupReady
            | SupStillConnecting
            | SupRecovering
            | SupCleanupFinished
            | SupIdle
            | SupAccepted
            | SupInitFailed
            | SupStalled
            | SupInterrupted
            | SupInterruptedShortIdle
    )
}

#[allow(dead_code)]
fn _outcome_is_used(_: TerminalOutcome) {}
