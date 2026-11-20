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
    pub pd: [u32; 25],
    /// PD-18 instances of runs that ended stuck in `Recovering` (a, b, c).
    pub pd18: [u32; 3],
    /// PD-19 runs by entry: `[0]` a `TerminationUnconfirmed` end (the bridge's failure
    /// arm cleared the context); `[1]` another runtime failure whose recording never had
    /// a context (a dropped start, PD-9). In both the user's stop took the preserved
    /// capture (R201.5), found no context and wrote `Idle`; `[2]` the `Interrupted` arm's
    /// auto-stop took nothing (the status was `Idle`), a start entered `Recovering` before
    /// the owner cleared, and the capture stays in the store with no user-reachable stop.
    pub pd19: [u32; 3],
    /// Occurrences per [`SIGNATURES`] row.
    pub sig: [u32; SIGNATURES.len()],
    /// Occurrences per matched split `(terminal row, performance row)`.
    pub pairs: BTreeMap<(&'static str, &'static str), u32>,
    /// Every decision the flights acted on, by row: the rows the gate must see.
    pub decided: BTreeMap<&'static str, u32>,
    /// Every `NeedObservation` round a flight answered, by the row the verdict names.
    pub observed: BTreeMap<&'static str, u32>,
    /// The main window processed a start reply that took its status from `Processing` to
    /// `Recording` while its stop IPC was still in flight (#1162). Counted from what the window
    /// shows before and after, so a model that suppressed the write counts none.
    pub reply_over_processing: u32,
    /// The main window sent a second stop gesture after such a reply, with no `Idle` received
    /// since: the stop the reply made possible (#1162).
    pub second_stop_after_late_reply: u32,
    /// The main window ignored a `starting` reply of a generation it had seen end (#1162).
    pub reply_ended_generation: u32,
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
    /// PD-23 (Completed): the claim was for the current recording, R207.3's `SD` ran and
    /// the ticker's gate **refused** a second ticker (none spawned), and the recording's
    /// own path completed performance later.
    SecondTickerGatedOff,
    /// PD-4: the claim was for the current recording and a **second ticker** spawned
    /// for it after the claim; the recording's own path completed performance later.
    SecondTickerThenPerf,
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
    // PD-4 (C7.1): R207.3 on a live `Recording` claims `Superseded/StaleOwner`, the
    // wrapper's `SD` spawns a **second ticker** for the same id, and performance later
    // completes `Success` (`Completed`): exactly the definition, with no narrower or
    // wider piece (D-S2b-11 (a)). A split of R207.3 without the second ticker, or with
    // another outcome, is not PD-4: it is a failure to report.
    Signature {
        pd: 4,
        terminal: &[("R207.3", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Recording],
        perf: PerfSet::Only(&[("R204b.6", PerfOutcome::Completed)]),
        pre: Pre::SecondTickerThenPerf,
    },
    // PD-23 (F-5, D-S2b-11 (a), D-S2b-13 (e)): the claim of PD-4 (R207.3, `Superseded` for
    // a live `Recording`) split from the outcome of the recording's *own* end, other than
    // PD-4's `Success` with a second ticker. Each outcome is bound to the stop, cancel,
    // pipeline or supervisor row that completes it (the paths C3 lists), never to a class:
    // an outcome from a row outside the table is an SP4(b) failure.
    Signature {
        pd: 23,
        terminal: &[("R207.3", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Recording],
        perf: PerfSet::Only(&[
            // The stop's capture failed (a stop failure, the supervisor's recovery
            // refusal, an expired receiver): `Failed`.
            ("R202.3", PerfOutcome::Failed),
            ("R203.1", PerfOutcome::Failed),
            ("R203.2", PerfOutcome::Failed),
            ("R211.3", PerfOutcome::Failed),
            ("R211.6", PerfOutcome::Failed),
            ("R213.2", PerfOutcome::Failed),
            ("R216.pre", PerfOutcome::Failed),
            // The stop's capture was empty or short: `NoSpeech`.
            ("R203.4", PerfOutcome::NoSpeech),
            ("R203.5", PerfOutcome::NoSpeech),
            ("R203.6", PerfOutcome::NoSpeech),
            ("R203.7", PerfOutcome::NoSpeech),
            ("R203.7r", PerfOutcome::NoSpeech),
            ("R203.8b", PerfOutcome::NoSpeech),
            // The user cancelled (the cancel's own `PC`, or the supervisor's recovery arm).
            ("R205.5", PerfOutcome::Cancelled),
            ("R209.4", PerfOutcome::Cancelled),
            // The pipeline's own end, as the stop's guard reports it.
            ("R204b.6", PerfOutcome::NoSpeech),
            ("R204b.6", PerfOutcome::Cancelled),
        ]),
        pre: Pre::LiveClaimThenPerf,
    },
    // PD-23, the `Success` variant: the same claim, the ticker's gate refused the second
    // ticker, and the pipeline completes `Completed`. Without the refused second ticker it
    // is PD-4, and from any row but the pipeline's R204b.6 it is not a C3 outcome. The
    // shell fixture of this variant is PENDING (source-only): no fake-host schedule makes
    // the gate refuse after R207.3's claim.
    Signature {
        pd: 23,
        terminal: &[("R207.3", SUPERSEDED.0, SUPERSEDED.1)],
        claim_status: &[Status::Recording],
        perf: PerfSet::Only(&[("R204b.6", PerfOutcome::Completed)]),
        pre: Pre::SecondTickerGatedOff,
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
    /// The checked-step stamp of each piece of evidence (the first time it happened), so
    /// PD-19's story is judged in order (D-S2b-20): the stop's take, the stop path's `Idle`,
    /// the start in the gap, then the auto-stop's refusal.
    take_at: Option<u32>,
    stop_idle_at: Option<u32>,
    gap_at: Option<u32>,
    auto_refused_at: Option<u32>,
    /// The user's stop path (R202 / R203 / R204) wrote `Idle` for the generation while
    /// its owner was still cleaning up: a context-less stop (R202.3), or the stop's
    /// pipeline guard (R204a).
    stop_idle: bool,
    /// `Interrupted` of 8,000 samples or more ended the generation.
    long_interrupt: bool,
    /// The stop the `Interrupted` arm spawned took nothing: the capture stayed in the
    /// store (R201.3 / R201.4 refused it, or it found none to take).
    auto_stop_took_nothing: bool,
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
    /// Recordings whose R207.3 `SD` ran (the second ticker, if any, exists by then).
    pub(super) r207_3_side_tasks: BTreeSet<Rid>,
    /// Recordings whose SP4(b) split waits for R207.3's `SD` to run.
    sp4b_deferred: BTreeSet<Rid>,
    /// Recordings whose PD-22 split (the supervisor `Idle`'s claim, `R211.2`, inside
    /// another cancel's tail) is judged when that tail closes: the window's other
    /// events (the cancel's lost claim, the performance completions) may land in
    /// either order inside it (D-S2b-11 (f)).
    sp4b_tail_deferred: BTreeSet<Rid>,
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
        // `handle` against the direct `decide_*` (`entry::once`): every difference fails the seed.
        for fault in w.handle_faults.borrow_mut().drain(..) {
            self.violations.push(fault);
        }
        for row in &out.observations {
            *self.hits.observed.entry(*row).or_default() += 1;
        }
        let mut hits: Vec<Hit> = Vec::new();
        for d in &out.decisions {
            *self.hits.decided.entry(d.row).or_default() += 1;
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
                    Effect::Terminal(_)
                        | Effect::PerfComplete(_)
                        | Effect::PerfBegin
                        | Effect::PipelineGuardFinish(_)
                ) || (e.row == "R207.3" && matches!(e.effect, Effect::StartSideTasks))
            })
            .map(|e| e.input_rid)
            .collect();
        for rid in touched {
            self.sp4b(w, rid);
        }
        // A PD-22 window that closed is judged now.
        let closed: Vec<Rid> = self
            .sp4b_tail_deferred
            .iter()
            .copied()
            .filter(|rid| {
                w.ledger.ctx[rid]
                    .tail
                    .is_none_or(|tail| w.cancel_tail.get(rid) != Some(&tail))
            })
            .collect();
        for rid in closed {
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
            let stamp = self.steps_checked;
            let gen = self.gens.entry(stranded).or_default();
            gen.gap = Some(instance);
            gen.gap_at.get_or_insert(stamp);
        }
    }

    /// Effects that leave evidence for a generation's liveness.
    fn note_effect(&mut self, e: &EffectEv, w: &World) {
        if let Effect::SetStatus(Status::Idle) = e.effect {
            let stop_family = ["R202", "R203", "R204"]
                .iter()
                .any(|family| e.row.starts_with(family));
            if stop_family && w.sup.cleanup_pending(e.input_rid) {
                let stamp = self.steps_checked;
                let gen = self.gens.entry(e.input_rid).or_default();
                gen.stop_idle = true;
                gen.stop_idle_at.get_or_insert(stamp);
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
                    let stamp = self.steps_checked;
                    let gen = self.gens.entry(*rid).or_default();
                    gen.taken_by = Some(row);
                    gen.take_at.get_or_insert(stamp);
                } else if earlier.is_some() {
                    self.hit(hits, 15, row, *rid);
                }
            }
            Note::AutoStopTookNothing { rid } => {
                let stamp = self.steps_checked;
                let gen = self.gens.entry(*rid).or_default();
                gen.auto_stop_took_nothing = true;
                gen.auto_refused_at.get_or_insert(stamp);
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
        self.sp4b_at(w, rid, false);
    }

    fn sp4b_at(&mut self, w: &World, rid: Rid, last: bool) {
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
        let claim = w.ledger.ctx[&rid];
        // PD-22 defers judgment to the end of its window: the winning `Idle` claim, the
        // named losing claim and the performance completion all fall in one cancel's tail,
        // in any order (the stop's failure tail may complete performance before the
        // cancel's own `TERM` is lost, or after). Judged when the tail closes.
        if !last
            && *terminal_row == "R211.2"
            && claim.other_cancel_tail
            && claim
                .tail
                .is_some_and(|tail| w.cancel_tail.get(&rid) == Some(&tail))
        {
            self.sp4b_tail_deferred.insert(rid);
            return;
        }
        self.sp4b_tail_deferred.remove(&rid);
        self.sp4b_done.insert(rid);
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
        // D-S2b-13 (d): SP4(b) compares the outcomes a generation actually has. A
        // completion with no run behind it (an aborted start, a generation whose run was
        // never begun) is a claim of the coordinator and leaves no performance row, so
        // there is no outcome to disagree with the terminal and it is not an SP4(b)
        // split. Except the supervisor's `Recovering` arm out of `Starting` (R209.3,
        // R209.5), which completes only a run that exists (PD-24 fixed, `only_if_run`):
        // a bare claim by it is a failure, with no signature to credit it.
        if w.perf.no_run.contains(&rid) {
            if matches!(*perf_row, "R209.3" | "R209.5") {
                self.fail(format!(
                    "SP4(b): id {rid}: terminal {outcome:?}/{code:?} by {terminal_row} but {perf_row} completed performance {perf:?} for a run that does not exist"
                ));
            }
            return;
        }
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
                && !w.perf.no_run.contains(&rid)
                && self.precondition(signature.pre, w, rid)
        })
    }

    fn precondition(&self, pre: Pre, w: &World, rid: Rid) -> bool {
        let second_ticker = self.second_ticker.contains(&rid);
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
            Pre::SecondTickerGatedOff => {
                claim_current
                    && claim.step < completion.step
                    && self.r207_3_side_tasks.contains(&rid)
                    && !second_ticker
            }
        }
    }

    /// End-of-run: SP4(a) and liveness (C7.3), per generation. The evidence each
    /// exemption needs is the recording's own (never a whole run's).
    pub fn on_end(&mut self, w: &World, quiescent_tasks: &[&'static str]) {
        self.sp4a(w);
        // A window still open at the end of the run is judged as it stands.
        for rid in std::mem::take(&mut self.sp4b_tail_deferred) {
            self.sp4b_at(w, rid, true);
        }
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
            // A preserved capture nobody took is not an exemption: the stop that could
            // take it is not one an ordinary control sends in `Recovering` (the frontend
            // suppresses it; D-S2b-13 (c), D-S2b-15 (c)). A run in which the unscoped
            // `stop_native_recording` IPC does take it ends in `Idle` and never gets
            // here; a run in which nothing does is stuck, and the generation's own
            // evidence decides whether it is PD-19.
            Status::Recovering => match self.stranded_instance(current, w) {
                Some(instance) => self.count_stranded(instance, current, w),
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
        match instance {
            Instance::Long => {
                // PD-19: the user's stop took the preserved capture (R201.5), found no
                // context and wrote `Idle` while the owner was still recovering (R202.3
                // or the R204a guard), a start took R101.12 before the owner cleared, and
                // the `Interrupted` end was 8,000 samples or more, so its auto-stop is the
                // only restorer and is refused. A stop that did not take the capture
                // leaves it for a later stop (see `on_end`): not stuck, so not this
                // evidence.
                // The other entry: the `Interrupted` arm's auto-stop ran after the status
                // had become `Idle` (the finished stop's `Idle`, R203.2) and took nothing,
                // and no stop took the capture either; it is still in the store, and the
                // only stop that could take it is the user's in `Recovering`.
                //
                // The first entry is judged in order, on the generation's own evidence
                // (D-S2b-20): the stop took the capture, then wrote its `Idle`, then a
                // start took R101.12 in the gap, then the `Interrupted` arm's auto-stop
                // ran and was refused. A run whose auto-stop never executes (the bridge
                // never dispatched it) is not stuck through this story. The second
                // variant (no `TerminationUnconfirmed` end) is a dropped start, and needs
                // the recording's own `StartDropped`.
                let ordered = matches!(
                    (gen.take_at, gen.stop_idle_at, gen.gap_at, gen.auto_refused_at),
                    (Some(take), Some(idle), Some(gap), Some(refused))
                        if take < idle && idle < gap && gap < refused
                );
                let unconfirmed =
                    w.sup.finished_fail.get(&rid) == Some(&ErrCode::TerminationUnconfirmed);
                let took_by_stop = gen.stop_idle
                    && gen.taken_by == Some("R201.5")
                    && gen.long_interrupt
                    && gen.auto_stop_took_nothing
                    && ordered
                    && (unconfirmed || self.dropped_ids.contains(&rid));
                let refused_auto_stop = gen.long_interrupt
                    && gen.auto_stop_took_nothing
                    && gen.taken_by.is_none()
                    && w.preserved.contains_key(&rid);
                (took_by_stop || refused_auto_stop).then_some(Instance::Long)
            }
            other => Some(other),
        }
    }

    fn count_stranded(&mut self, instance: Instance, rid: Rid, w: &World) {
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
            Instance::Long => {
                self.hits.pd[19] += 1;
                // Entries: `[0]` a `TerminationUnconfirmed` end, `[1]` another runtime
                // failure (a dropped start), both through the user's stop; `[2]` the
                // refused auto-stop that left the capture in the store.
                let entry = if w.preserved.contains_key(&rid) {
                    2
                } else if w.sup.finished_fail.get(&rid) == Some(&ErrCode::TerminationUnconfirmed) {
                    0
                } else {
                    // `stranded_instance` admitted it only with the recording's own
                    // `StartDropped`.
                    1
                };
                self.hits.pd19[entry] += 1;
            }
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

/// D-S2b-20 (finding 3): PD-19's first two entries are judged on the generation's own ordered
/// evidence. The evidence is planted here directly, as the checker records it.
#[cfg(test)]
mod pd19_evidence_tests {
    use super::*;

    /// A stuck `Recovering` world for recording 1, ended by a long `Interrupted`.
    fn world(failure: Option<ErrCode>) -> World {
        let mut w = World::new();
        w.current_id = 1;
        w.status = Status::Recovering;
        if let Some(code) = failure {
            w.sup.finished_fail.insert(1, code);
        }
        w
    }

    /// The story with each piece at the given stamp; `None` leaves it out.
    fn checker(
        take: Option<u32>,
        idle: Option<u32>,
        gap: Option<u32>,
        refused: Option<u32>,
        dropped: bool,
    ) -> Checker {
        let mut checker = Checker::default();
        let gen = checker.gens.entry(1).or_default();
        gen.long_interrupt = true;
        gen.gap = Some(Instance::Long);
        gen.taken_by = take.map(|_| "R201.5");
        gen.take_at = take;
        gen.stop_idle = idle.is_some();
        gen.stop_idle_at = idle;
        gen.gap_at = gap;
        gen.auto_stop_took_nothing = refused.is_some();
        gen.auto_refused_at = refused;
        if dropped {
            checker.dropped_ids.insert(1);
        }
        checker
    }

    fn end(mut checker: Checker, w: &World) -> Checker {
        checker.on_end(w, &[]);
        checker
    }

    fn stuck(checker: &Checker) -> bool {
        checker
            .violations
            .iter()
            .any(|v| v.contains("without PD-18/PD-19 evidence"))
    }

    #[test]
    fn the_whole_ordered_story_is_pd19() {
        let unconfirmed = world(Some(ErrCode::TerminationUnconfirmed));
        let done = end(
            checker(Some(1), Some(2), Some(3), Some(4), false),
            &unconfirmed,
        );
        assert!(done.violations.is_empty(), "{:?}", done.violations);
        assert_eq!(done.hits.pd19, [1, 0, 0]);
        // The second variant: no `TerminationUnconfirmed`, a dropped start.
        let other = world(Some(ErrCode::StreamInvalidated));
        let done = end(checker(Some(1), Some(2), Some(3), Some(4), true), &other);
        assert!(done.violations.is_empty(), "{:?}", done.violations);
        assert_eq!(done.hits.pd19, [0, 1, 0]);
    }

    /// The bridge never dispatches the auto-stop: the run is stuck, but not through
    /// PD-19's story, so it is a liveness failure.
    #[test]
    fn a_run_whose_auto_stop_never_executes_is_not_pd19() {
        let unconfirmed = world(Some(ErrCode::TerminationUnconfirmed));
        let done = end(
            checker(Some(1), Some(2), Some(3), None, false),
            &unconfirmed,
        );
        assert!(stuck(&done), "{:?}", done.violations);
        let other = world(Some(ErrCode::StreamInvalidated));
        let done = end(checker(Some(1), Some(2), Some(3), None, true), &other);
        assert!(stuck(&done), "{:?}", done.violations);
    }

    /// The second variant needs the recording's own dropped start, and every piece must come
    /// in order.
    #[test]
    fn the_second_variant_needs_a_dropped_start_and_the_story_its_order() {
        let other = world(Some(ErrCode::StreamInvalidated));
        let done = end(checker(Some(1), Some(2), Some(3), Some(4), false), &other);
        assert!(stuck(&done), "a non-unconfirmed end without StartDropped");
        let unconfirmed = world(Some(ErrCode::TerminationUnconfirmed));
        for (take, idle, gap, refused) in [
            (Some(2), Some(1), Some(3), Some(4)), // `Idle` before the take
            (Some(1), Some(3), Some(2), Some(4)), // the restart before the stop's `Idle`
            (Some(1), Some(2), Some(4), Some(3)), // the refusal before the restart
            (None, Some(2), Some(3), Some(4)),    // no take
            (Some(1), None, Some(3), Some(4)),    // no `Idle` write
            (Some(1), Some(2), None, Some(4)),    // no restart in the gap
        ] {
            let done = end(checker(take, idle, gap, refused, true), &unconfirmed);
            assert!(
                stuck(&done),
                "{take:?} {idle:?} {gap:?} {refused:?}: {:?}",
                done.violations
            );
        }
    }
}
