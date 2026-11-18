//! The prefix check (SP1-SP10 after every step), the preserved-defect detectors
//! (C7.1: an allowlisted violation is removed from the failure set only when its
//! full signature matches: the winning effects, the outcome classes and the causal
//! precondition, D-S2b-7 (a)) and the end-of-run liveness check (C7.3), which reads
//! per-generation evidence instead of exempting whole runs.

use super::model::*;
use super::task::*;
use crate::effects::{Effect, PerfOutcome};
use crate::status::{ErrCode, Status, TerminalOutcome};
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
    /// An `Interrupted` end of 8,000 samples or more: PD-19 when its own evidence
    /// is complete (decision D-S2b-5), never an instance of PD-18.
    Long,
}

#[derive(Debug, Default, Clone)]
pub struct RunHits {
    /// Occurrences per PD number (index 0 unused).
    pub pd: [u32; 23],
    /// PD-18 instances of runs that ended stuck in `Recovering` (a, b, c).
    pub pd18: [u32; 3],
    /// Occurrences per [`SIGNATURES`] row.
    pub sig: [u32; SIGNATURES.len()],
    /// Occurrences per matched split `(terminal row, performance row)`.
    pub pairs: BTreeMap<(&'static str, &'static str), u32>,
}

/// What a signature's causal precondition requires of the two winners. Every
/// variant reads the ledger context the winners recorded (`Won`), never the row names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pre {
    /// PD-4, PD-5: the claim was for the current recording, in the status the row's
    /// C3 line lists, and the recording's own path completed performance later.
    LiveClaimThenPerf,
    /// PD-20 (a): the cancel's claim won (its tail open); another path completed
    /// performance during the tail, before the cancel's own `PC`.
    CancelClaimedFirst,
    /// PD-20 (b): another path claimed the terminal first, the cancel's own claim was
    /// lost, and the cancel's tail still completed performance `Cancelled`.
    CancelPerfAfterLostClaim,
    /// PD-21: the cancel port's failure claimed `PipelineFailure` for the current
    /// recording, which then ended through a path with its own outcome.
    FailedCancelPort,
    /// PD-22: the supervisor's `Idle` claimed `Superseded` for a finished current id,
    /// a later claim of the real path was lost, and a cancel's tail was open when
    /// performance completed.
    SupervisorIdleWon,
    /// PD-4: the claim was for the current recording and a **second ticker** spawned
    /// for it after the claim; the recording's own path completed performance later.
    SecondTickerThenPerf,
    /// PD-4's claim half: the claim and the order as above, but the second ticker was
    /// refused because the recording had left `Recording` (or its id was no longer
    /// current) when R207.3's `SD` ran: the ticker's own gate (`q_ticker_may_register`),
    /// not an absent `SD`.
    TickerGatedThenPerf,
    /// PD-12: a stale stop claimed `Superseded` for an older recording (its input was
    /// not the current one) before that recording's own path completed performance.
    StaleClaimThenPerf,
}

/// The performance completions a signature admits.
#[derive(Clone, Copy, Debug)]
pub enum PerfSet {
    /// Any path's completion C3 lists (the effect check already pins each row's own
    /// outcome), with one of these outcome classes.
    Any(&'static [PerfOutcome]),
    /// Exactly these `(row, outcome)` completions.
    Only(&'static [(&'static str, PerfOutcome)]),
}

impl PerfSet {
    fn admits(&self, row: &str, outcome: PerfOutcome) -> bool {
        match self {
            Self::Any(classes) => classes.contains(&outcome),
            Self::Only(pairs) => pairs.iter().any(|(r, o)| *r == row && *o == outcome),
        }
    }
}

const ALL_PERF: &[PerfOutcome] = &[
    PerfOutcome::Failed,
    PerfOutcome::Cancelled,
    PerfOutcome::NoSpeech,
    PerfOutcome::Completed,
];

const ALL_STATUSES: &[Status] = &[
    Status::Idle,
    Status::Starting,
    Status::Recording,
    Status::Recovering,
    Status::Processing,
];

/// One allowlisted SP4(b) split (C7.1): the PD it belongs to, the terminal winners
/// (row, outcome, code) with the statuses the claim saw, the performance winners and
/// the causal precondition. All must match for the split to leave the failure set; a
/// pair outside the table is an SP4(b) failure.
#[derive(Clone, Copy, Debug)]
pub struct Signature {
    pub pd: u8,
    pub terminal: &'static [(&'static str, TerminalOutcome, ErrCode)],
    pub claim_status: &'static [Status],
    pub perf: PerfSet,
    pub pre: Pre,
}

const SUPERSEDED: (TerminalOutcome, ErrCode) = (TerminalOutcome::Superseded, ErrCode::StaleOwner);

pub const SIGNATURES: [Signature; 8] = [
    // PD-4 (C7.1): R207.3 on a live `Recording` claims `Superseded/StaleOwner` (its
    // specified terminal), the wrapper's `SD` spawns a **second ticker** for the same
    // id, and the recording's own path completes performance later. C7.1 names
    // `Success` as the outcome the live recording reaches; the recording can instead
    // end in any other class (a stop of silence, a crash), and the disagreement is the
    // same defect, so the performance class is not narrowed. The claim, the status,
    // the ticker and the order are all required.
    Signature {
        pd: 4,
        terminal: &[("R207.3", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Recording],
        perf: PerfSet::Any(ALL_PERF),
        pre: Pre::SecondTickerThenPerf,
    },
    // PD-4, claim half: the same claim and status, and R207.3's `SD` ran but the ticker
    // gate refused (the recording had already left `Recording`).
    Signature {
        pd: 4,
        terminal: &[("R207.3", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Recording],
        perf: PerfSet::Any(ALL_PERF),
        pre: Pre::TickerGatedThenPerf,
    },
    // PD-5: R207.2 / R209.2 on the current id in `Idle`, `Recovering` or `Processing`
    // (the statuses C3 lists for those rows).
    Signature {
        pd: 5,
        terminal: &[
            ("R207.2", SUPERSEDED.0, SUPERSEDED.1),
            ("R209.2", SUPERSEDED.0, SUPERSEDED.1),
        ],
        claim_status: &[Status::Idle, Status::Recovering, Status::Processing],
        perf: PerfSet::Any(ALL_PERF),
        pre: Pre::LiveClaimThenPerf,
    },
    // PD-20 (a): the cancel at `Processing` (R205.5) claimed; the stop or pipeline
    // completed performance during the cancel's tail.
    Signature {
        pd: 20,
        terminal: &[(
            "R205.5",
            TerminalOutcome::UserCancelledProcessing,
            ErrCode::CancelledProcessing,
        )],
        claim_status: &[Status::Idle],
        perf: PerfSet::Any(&[
            PerfOutcome::Failed,
            PerfOutcome::NoSpeech,
            PerfOutcome::Completed,
        ]),
        pre: Pre::CancelClaimedFirst,
    },
    // PD-20 (b): another path claimed first; the cancel's `PC(Cancelled)` completed
    // performance.
    Signature {
        pd: 20,
        terminal: &[
            (
                "R203.pre",
                TerminalOutcome::TooShort,
                ErrCode::BelowMinimumDuration,
            ),
            ("R203.pre", TerminalOutcome::NoSpeech, ErrCode::EmptyAudio),
            (
                "R203.pre",
                TerminalOutcome::StopFailure,
                ErrCode::StopFinalizationFailed,
            ),
            (
                "R204b.5",
                TerminalOutcome::PipelineFailure,
                ErrCode::TranscriptionFailed,
            ),
            (
                "R213.2",
                TerminalOutcome::CaptureInitFailure,
                ErrCode::BackendError,
            ),
        ],
        claim_status: ALL_STATUSES,
        perf: PerfSet::Only(&[("R205.5", PerfOutcome::Cancelled)]),
        pre: Pre::CancelPerfAfterLostClaim,
    },
    // PD-21: R206.2, the failed capture cancel's `TERM(PipelineFailure)`.
    Signature {
        pd: 21,
        terminal: &[(
            "R206.2",
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
        )],
        claim_status: ALL_STATUSES,
        perf: PerfSet::Any(&[
            PerfOutcome::Cancelled,
            PerfOutcome::NoSpeech,
            PerfOutcome::Completed,
        ]),
        pre: Pre::FailedCancelPort,
    },
    // PD-12 (C7.1): the stale-id stop (R201.1) emits `Superseded` for an older id that
    // is still finishing; its own path completes performance with another outcome.
    Signature {
        pd: 12,
        terminal: &[("R201.1", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: ALL_STATUSES,
        perf: PerfSet::Any(ALL_PERF),
        pre: Pre::StaleClaimThenPerf,
    },
    // PD-22: the supervisor `Idle` (R211.2) won `Superseded` for a finished current id.
    Signature {
        pd: 22,
        terminal: &[("R211.2", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Idle],
        perf: PerfSet::Only(&[
            ("R203.2", PerfOutcome::Failed),
            ("R205.5", PerfOutcome::Cancelled),
        ]),
        pre: Pre::SupervisorIdleWon,
    },
];

/// Evidence one recording (generation) left for the end-of-run checks.
#[derive(Debug, Default, Clone)]
struct Gen {
    /// R101.12 taken after the generation's last restorer and before its owner cleared.
    gap: Option<Instance>,
    /// The row that took the preserved capture.
    taken_by: Option<&'static str>,
    /// The user's stop path (R202 / R203 / R204) wrote `Idle` for the generation while
    /// its owner was still cleaning up: a context-less stop (R202.3), or the stop's
    /// pipeline guard (R204a).
    stop_idle: bool,
    /// `Interrupted` of 8,000 samples or more ended the generation.
    long_interrupt: bool,
}

#[derive(Debug, Default)]
pub struct Checker {
    pub violations: Vec<String>,
    pub hits: RunHits,
    /// Recordings each PD touched, for the end-of-run checks.
    pd_ids: BTreeMap<u8, BTreeSet<Rid>>,
    gens: BTreeMap<Rid, Gen>,
    sp4b_done: BTreeSet<Rid>,
    prev_current: Rid,
    prev_lease: Option<(Rid, TaskId)>,
    /// Recordings whose start future was dropped at its await.
    dropped_ids: BTreeSet<Rid>,
    /// Recordings whose pipeline or stop task panicked (PD-13).
    panicked_ids: BTreeSet<Rid>,
    pub steps_checked: u32,
    /// Recordings for which a second ticker spawned after R207.3's claim (PD-4).
    pub(super) second_ticker: BTreeSet<Rid>,
    /// Recordings whose R207.3 `SD` ran after the recording left `Recording`: the
    /// ticker gate refused, so no second ticker exists (PD-4's claim half).
    pub(super) ticker_gated: BTreeSet<Rid>,
    /// Recordings whose R207.3 `SD` ran (the second ticker, if any, exists by then).
    pub(super) r207_3_side_tasks: BTreeSet<Rid>,
    /// Recordings whose SP4(b) split waits for R207.3's `SD` to run.
    sp4b_deferred: BTreeSet<Rid>,
    /// How many of `World::registry_faults` were already reported.
    faults_seen: usize,
    /// A readable event trace (for reproducing a seed by hand).
    pub trace: Option<Vec<String>>,
}

const SP2_EXEMPT: [u8; 4] = [1, 5, 7, 16];

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
        for fault in w.registry_faults.iter().skip(self.faults_seen) {
            self.violations.push(fault.clone());
        }
        self.faults_seen = w.registry_faults.len();
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
            self.note_effect(e, w);
        }
        for note in &out.notes {
            self.detect_note(note, &mut hits);
        }
        for e in &out.effects {
            // PD-4's defining behaviour: after R207.3 claimed `Superseded` for the live
            // recording, its side tasks spawned a second ticker for the same id.
            if e.row == "R207.3" && matches!(e.effect, Effect::StartSideTasks) {
                self.r207_3_side_tasks.insert(e.input_rid);
                if w.tickers.get(&e.input_rid).copied().unwrap_or(0) >= 2 {
                    self.second_ticker.insert(e.input_rid);
                } else if e.status_before != Status::Recording || e.current != e.input_rid {
                    self.ticker_gated.insert(e.input_rid);
                }
            }
            self.check_effect(e, &hits);
        }
        self.check_lease(w, out);
        // SP4(b) at the prefix where both outcomes exist.
        let touched: BTreeSet<Rid> = out
            .effects
            .iter()
            .filter(|e| {
                matches!(
                    e.effect,
                    Effect::Terminal(_) | Effect::PerfComplete(_) | Effect::PipelineGuardFinish(_)
                ) || (e.row == "R207.3" && matches!(e.effect, Effect::StartSideTasks))
            })
            .map(|e| e.input_rid)
            .collect();
        for rid in touched {
            self.sp4b(w, rid);
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
        // SP1: the ledger.
        for (rid, wins) in &w.ledger.wins {
            if *wins > 1 || !w.ledger.accepted.contains(rid) {
                self.fail(format!("SP1: id {rid} was claimed {wins} times"));
            }
        }
        // SP7 high-water values are diagnostics, not limits.
    }

    /// SP10: the partial-decode lease changes only by its holder's own acquire and
    /// drop. No decision or effect of a step (a terminal, a cancel, an `IdleGuard`
    /// finish, a `CX-`) releases it, and a second ticker's `try_begin` is refused
    /// while it is held.
    fn check_lease(&mut self, w: &World, out: &StepOut) {
        let mut expected = self.prev_lease;
        for note in &out.notes {
            if let Note::Lease { rid, tid, acquire } = note {
                if *acquire {
                    if expected.is_some() {
                        self.fail(format!(
                            "SP10: ticker {tid} for id {rid} took the lease while {expected:?} held it"
                        ));
                    }
                    expected = Some((*rid, *tid));
                } else {
                    if expected != Some((*rid, *tid)) {
                        self.fail(format!(
                            "SP10: ticker {tid} for id {rid} dropped a lease it does not hold ({expected:?})"
                        ));
                    }
                    expected = None;
                }
            }
        }
        if w.partial_lease != expected {
            self.fail(format!(
                "SP10: the lease changed from {expected:?} to {:?} outside its holder's acquire or drop",
                w.partial_lease
            ));
        }
        self.prev_lease = w.partial_lease;
    }

    fn detect_decision(&mut self, d: &DecisionEv, hits: &mut Vec<Hit>) {
        let row = d.row;
        let rid = d.input_rid;
        let from_to = d.transition;
        match row {
            "R216.3" | "R216.4" | "R216.5" => self.hit(hits, 1, row, rid),
            "R211.2" => self.hit(hits, 2, row, rid),
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
            let stranded = d.current_before;
            let instance = match d.ended_by {
                Some(EndedBy::Recover {
                    interrupted: Some(samples),
                }) if samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES => Instance::A,
                Some(EndedBy::Recover {
                    interrupted: Some(_),
                }) => {
                    self.gens.entry(stranded).or_default().long_interrupt = true;
                    Instance::Long
                }
                Some(EndedBy::Stop(Delivery::Unsealed)) => Instance::C,
                _ => Instance::B,
            };
            self.gens.entry(stranded).or_default().gap = Some(instance);
        }
    }

    /// Effects that leave evidence for a generation's liveness.
    fn note_effect(&mut self, e: &EffectEv, w: &World) {
        if let Effect::SetStatus(Status::Idle) = e.effect {
            let stop_family = ["R202", "R203", "R204"]
                .iter()
                .any(|family| e.row.starts_with(family));
            if stop_family && w.sup.cleanup_pending(e.input_rid) {
                self.gens.entry(e.input_rid).or_default().stop_idle = true;
            }
        }
    }

    fn detect_note(&mut self, note: &Note, hits: &mut Vec<Hit>) {
        match note {
            Note::Take {
                found,
                earlier,
                rid,
                row,
                ..
            } => {
                if *found {
                    self.gens.entry(*rid).or_default().taken_by = Some(row);
                } else if earlier.is_some() {
                    self.hit(hits, 15, row, *rid);
                }
            }
            Note::StartDropped {
                rid,
                registered,
                begun,
            } => {
                self.dropped_ids.insert(*rid);
                if *registered && !*begun {
                    self.hit(hits, 9, "R101.15", *rid);
                }
            }
            Note::TaskPanicked { rid, .. } => {
                self.panicked_ids.insert(*rid);
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

    /// SP2 and SP3 per effect. The C3 outcomes of a decision (statuses, terminals,
    /// performance completions) are compared with the fixture once per decision, in
    /// `task::log_decision`.
    fn check_effect(&mut self, e: &EffectEv, hits: &[Hit]) {
        let stale = e.input_rid != e.current;
        if let Effect::SetStatus(to) = &e.effect {
            // SP3: the edge set.
            if !edge_allowed(e.status_before, *to) {
                self.fail(format!(
                    "SP3: {} wrote {:?} -> {:?}",
                    e.row, e.status_before, to
                ));
            }
            // SP2: a stale input never changes the current recording's status.
            if stale && !Self::exempt(hits, &SP2_EXEMPT, e.row) {
                self.fail(format!(
                    "SP2: {} (input {}) wrote {:?} while current is {}",
                    e.row, e.input_rid, to, e.current
                ));
            }
        }
    }

    fn pd_touched(&self, pd: u8, rid: Rid) -> bool {
        self.pd_ids.get(&pd).is_some_and(|ids| ids.contains(&rid))
    }

    /// SP4(b), at the prefix where the terminal and the performance outcome of `rid`
    /// both exist: a disagreement fails unless one signature of the table matches in
    /// full (the PD's rows, outcomes and causal precondition).
    pub(super) fn sp4b(&mut self, w: &World, rid: Rid) {
        let (Some((outcome, code, terminal_row)), Some((perf, perf_row))) =
            (w.ledger.claimed.get(&rid), w.perf.outcome.get(&rid))
        else {
            return;
        };
        if perf_class_allowed(*outcome, *perf) || self.sp4b_done.contains(&rid) {
            return;
        }
        // PD-4 needs its second ticker: R207.3's `SD` is the claim's own `PostDT` step,
        // so while it is still to run the split is not judged yet (it is, at that step,
        // or fails at the end of the run).
        if *terminal_row == "R207.3" && !self.r207_3_side_tasks.contains(&rid) {
            self.sp4b_deferred.insert(rid);
            return;
        }
        self.sp4b_deferred.remove(&rid);
        self.sp4b_done.insert(rid);
        let claim = w.ledger.ctx[&rid];
        let completion = w.perf.ctx[&rid];
        if std::env::var("DUMP_SP4B").is_ok() && !self.would_match(w, rid) {
            eprintln!(
                "SP4B {terminal_row} {outcome:?}/{code:?} st={:?} stale={} cur={} tail(other={},own={}) step={} | {perf_row} {perf:?} st={:?} stale={} tail(other={},own={}) step={} tries={:?}",
                claim.status_before,
                claim.input_rid != claim.current,
                claim.current,
                claim.other_cancel_tail,
                claim.own_cancel_tail,
                claim.step,
                completion.status_before,
                completion.input_rid != completion.current,
                completion.other_cancel_tail,
                completion.own_cancel_tail,
                completion.step,
                w.ledger.tries[&rid].iter().map(|t| (t.row, t.won)).collect::<Vec<_>>()
            );
        }
        let matched = self.matching(w, rid);
        match matched {
            Some(index) => {
                self.hits.pd[usize::from(SIGNATURES[index].pd)] += 1;
                self.hits.sig[index] += 1;
                *self
                    .hits
                    .pairs
                    .entry((terminal_row, perf_row))
                    .or_default() += 1;
                self.pd_ids
                    .entry(SIGNATURES[index].pd)
                    .or_default()
                    .insert(rid);
            }
            None => self.fail(format!(
                "SP4(b): id {rid}: terminal {outcome:?}/{code:?} by {terminal_row} but performance {perf:?} by {perf_row} (no signature matches in full)"
            )),
        }
    }

    fn would_match(&self, w: &World, rid: Rid) -> bool {
        self.matching(w, rid).is_some()
    }

    fn matching(&self, w: &World, rid: Rid) -> Option<usize> {
        let (outcome, code, terminal_row) = w.ledger.claimed.get(&rid)?;
        let (perf, perf_row) = w.perf.outcome.get(&rid)?;
        let claim = w.ledger.ctx.get(&rid)?;
        SIGNATURES.iter().position(|signature| {
            signature
                .terminal
                .contains(&(*terminal_row, *outcome, *code))
                && signature.claim_status.contains(&claim.status_before)
                && signature.perf.admits(perf_row, *perf)
                && self.precondition(signature.pre, w, rid)
        })
    }

    fn precondition(&self, pre: Pre, w: &World, rid: Rid) -> bool {
        let second_ticker = self.second_ticker.contains(&rid);
        let ticker_gated = self.ticker_gated.contains(&rid);
        let claim = w.ledger.ctx[&rid];
        let completion = w.perf.ctx[&rid];
        let tries = &w.ledger.tries[&rid];
        let claim_current = claim.input_rid == claim.current;
        match pre {
            Pre::LiveClaimThenPerf => claim_current && claim.step < completion.step,
            Pre::CancelClaimedFirst => {
                claim.own_cancel_tail
                    && completion.other_cancel_tail
                    && claim.step < completion.step
            }
            Pre::CancelPerfAfterLostClaim => {
                completion.own_cancel_tail
                    && tries
                        .iter()
                        .any(|t| t.row == completion.row && !t.won && t.step > claim.step)
            }
            Pre::FailedCancelPort => claim_current,
            Pre::SupervisorIdleWon => {
                // One window: the winning `Idle` claim, the named losing claim (the
                // cancel's own `TERM`, R205.5) and the performance completion all fall
                // in the tail of the same cancel.
                claim_current
                    && claim.step < completion.step
                    && claim.other_cancel_tail
                    && claim.tail.is_some_and(|tail| {
                        completion.tail == Some(tail)
                            && tries.iter().any(|t| {
                                t.row == "R205.5"
                                    && !t.won
                                    && t.step > claim.step
                                    && t.tail == Some(tail)
                            })
                    })
            }
            Pre::SecondTickerThenPerf => {
                claim_current && claim.step < completion.step && second_ticker
            }
            Pre::TickerGatedThenPerf => {
                claim_current && claim.step < completion.step && ticker_gated
            }
            Pre::StaleClaimThenPerf => !claim_current && claim.step < completion.step,
        }
    }

    /// End-of-run: SP4(a) and liveness (C7.3), per generation. The evidence each
    /// exemption needs is the recording's own (never a whole run's).
    pub fn on_end(&mut self, w: &World, quiescent_tasks: &[&'static str]) {
        self.sp4a(w);
        for rid in std::mem::take(&mut self.sp4b_deferred) {
            self.fail(format!(
                "SP4(b): id {rid}: R207.3's claim split from performance and its side tasks never ran"
            ));
        }
        if w.partial_lease.is_some() {
            self.fail(format!(
                "SP10: the lease {:?} was never released",
                w.partial_lease
            ));
        }
        let current = w.current_id;
        // L4: a recording nobody stops. Only that recording may stay open.
        let open_l4: Option<Rid> = w
            .sup
            .attempt
            .as_ref()
            .filter(|a| matches!(a.phase, Phase::Starting | Phase::Recording))
            .map(|a| a.rid);
        let mut problems: Vec<String> = Vec::new();
        if !quiescent_tasks.is_empty() {
            problems.push(format!("tasks never finished: {quiescent_tasks:?}"));
        }
        if w.t_holder.is_some() {
            problems.push("T is still held".into());
        }
        match w.status {
            Status::Idle => {}
            Status::Recovering => match self.stranded_instance(current, w) {
                Some(instance) => self.count_stranded(instance),
                None => problems.push(format!(
                    "status Recovering at the end for id {current} without PD-18/PD-19 evidence"
                )),
            },
            Status::Starting | Status::Recording
                if open_l4 == Some(current) || self.dropped_ids.contains(&current) => {}
            other => problems.push(format!("status {other:?} at the end")),
        }
        for rid in &w.ledger.accepted {
            if !w.ledger.claimed.contains_key(rid)
                && !self.pd_touched(13, *rid)
                && !self.dropped_ids.contains(rid)
                && open_l4 != Some(*rid)
            {
                problems.push(format!("id {rid} accepted but never claimed"));
            }
        }
        for (op, (rid, kind)) in &w.history.ops {
            if open_l4 != Some(*rid) && !self.dropped_ids.contains(rid) {
                problems.push(format!("op {op} ({kind:?}) of id {rid} unresolved"));
            }
        }
        for rid in &w.history.pipelines {
            if open_l4 != Some(*rid) {
                problems.push(format!("pipeline of id {rid} unresolved"));
            }
        }
        for problem in problems {
            self.fail(format!("liveness: {problem}"));
        }
    }

    /// The PD-18 instance or PD-19 the generation `rid` ended stuck in `Recovering`
    /// through, with the generation's own evidence.
    fn stranded_instance(&self, rid: Rid, w: &World) -> Option<Instance> {
        let gen = self.gens.get(&rid)?;
        let instance = gen.gap?;
        let _ = w;
        match instance {
            Instance::Long => {
                // PD-19: a stop took the preserved capture (its `TakeInterrupted`,
                // R201.5; the user's stop in the store-then-notify gap, or the arm's own
                // auto-stop), the stop path wrote `Idle` (R202.3 or the R204a guard)
                // while the owner was still recovering, and a start took R101.12 before
                // the owner cleared. The arm's auto-stop is refused (R201.6) in the
                // user-stop variant; when the auto-stop is the one that took the capture
                // there is nothing to refuse.
                (gen.long_interrupt && gen.taken_by == Some("R201.5") && gen.stop_idle)
                    .then_some(Instance::Long)
            }
            other => Some(other),
        }
    }

    fn count_stranded(&mut self, instance: Instance) {
        match instance {
            Instance::A => {
                self.hits.pd18[0] += 1;
                self.hits.pd[18] += 1;
            }
            Instance::B => {
                self.hits.pd18[1] += 1;
                self.hits.pd[18] += 1;
            }
            Instance::C => {
                self.hits.pd18[2] += 1;
                self.hits.pd[18] += 1;
            }
            Instance::Long => self.hits.pd[19] += 1,
        }
    }

    fn sp4a(&mut self, w: &World) {
        for (rid, (outcome, _, _)) in &w.ledger.claimed {
            if let Some(diag) = w.ledger.diagnostics.get(rid) {
                if diag != outcome && !self.pd_touched(11, *rid) {
                    self.fail(format!(
                        "SP4(a): id {rid} claimed {outcome:?} but diagnostics took {diag:?}"
                    ));
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
