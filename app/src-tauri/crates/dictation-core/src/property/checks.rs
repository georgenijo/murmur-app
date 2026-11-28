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

#[derive(Debug, Default, Clone)]
pub struct RunHits {
    /// Occurrences per PD number (index 0 unused).
    pub pd: [u32; 25],
    /// Starts refused by R101.12 in the teardown gap of a retiring capture: the cleanup
    /// is pending, no notification that restores `Idle` is still coming and no pipeline
    /// runs. These are the schedules that used to strand the app in `Recovering` (PD-18,
    /// PD-19, fixed): `[0]` a short `Interrupted` end, `[1]` the `Idle`-arm tail (any end
    /// that is not one of the others), `[2]` an unsealed delivered stop, `[3]` an
    /// `Interrupted` end of 8,000 samples or more. The fix is the refusal that writes
    /// nothing, so every such run must end in `Idle`.
    pub refused_in_teardown_gap: [u32; 4],
    /// Occurrences per [`SIGNATURES`] row.
    pub sig: [u32; SIGNATURES.len()],
    /// Occurrences per matched split `(terminal row, performance row)`.
    pub pairs: BTreeMap<(&'static str, &'static str), u32>,
    /// Completions dropped by the row that decided them: `only_if_run` for want of a run
    /// (PD-24's fix: R209.3, R209.5 out of `Starting` after the start's own failure), and
    /// `follows_terminal` after a claim lost to the recording's own end (PD-20's fix: R205.5).
    pub dropped_completions: BTreeMap<&'static str, u32>,
    /// Completions that arrived after the cancel of a processing recording (R205.5) won both
    /// ledgers, by the row that made them (PD-20's fix: the pair completes performance with the
    /// terminal, so these decide nothing). Each used to win performance inside the cancel's tail.
    pub late_after_cancel: BTreeMap<&'static str, u32>,
    /// The cancel of a processing recording (R205.5) lost its terminal claim and completed no
    /// performance, by the row whose claim won (PD-20's fix: the winner's own path completes).
    pub cancel_lost_to: BTreeMap<&'static str, u32>,
    /// Every recording's recorded performance outcome at the end of the run, by the row whose
    /// completion it is (R204b.5: the pipeline guard's `Drop` after the pipeline failed).
    pub completed_by: BTreeMap<&'static str, u32>,
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
    /// Recordings whose stop or pipeline task panicked and whose end the supervisor's `Idle`
    /// (R211.2) also notified, and whose terminal the unwinding guard claimed (R204a.7,
    /// `PipelineFailure/WorkerPanicked`). R211.2 claims none since PD-22 was fixed, so before
    /// the PD-13 fix (#1191) these recordings ended with no terminal. Counted at the end of
    /// the run, from the ledger.
    pub panicked_task_idle_claimed_by_guard: u32,
}

/// What a signature's causal precondition requires of the two winners. Every
/// variant reads the ledger context the winners recorded (`Won`), never the row names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pre {
    /// PD-5: the claim was for the current recording, in the status the row's
    /// C3 line lists, and the recording's own path completed performance later.
    LiveClaimThenPerf,
}

/// The performance completions a signature admits.
#[derive(Clone, Copy, Debug)]
pub enum PerfSet {
    /// Any path's completion C3 lists (the effect check already pins each row's own
    /// outcome), with one of these outcome classes.
    Any(&'static [PerfOutcome]),
}

impl PerfSet {
    fn admits(&self, outcome: PerfOutcome) -> bool {
        match self {
            Self::Any(classes) => classes.contains(&outcome),
        }
    }
}

const ALL_PERF: &[PerfOutcome] = &[
    PerfOutcome::Failed,
    PerfOutcome::Cancelled,
    PerfOutcome::NoSpeech,
    PerfOutcome::Completed,
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

pub const SIGNATURES: [Signature; 1] = [
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
];

#[derive(Debug, Default)]
pub struct Checker {
    pub violations: Vec<String>,
    pub hits: RunHits,
    /// Recordings each PD touched, for the end-of-run checks.
    pd_ids: BTreeMap<u8, BTreeSet<Rid>>,
    sp4b_done: BTreeSet<Rid>,
    prev_current: Rid,
    prev_lease: Option<(Rid, TaskId)>,
    /// Recordings whose start future was dropped at its await.
    dropped_ids: BTreeSet<Rid>,
    /// Recordings whose pipeline or stop task panicked.
    panicked_ids: BTreeSet<Rid>,
    /// Recordings the supervisor's `Idle` was handled for while already `Idle` (R211.2).
    idle_for_finished_ids: BTreeSet<Rid>,
    pub steps_checked: u32,
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
        }
        for note in &out.notes {
            self.detect_note(note, &mut hits);
        }
        for e in &out.effects {
            // PD-4's remaining behaviour: a repeat `Ready` for the live recording (R207.3,
            // which claims no terminal) still attempts the side tasks, and they spawn a
            // second ticker for the same id.
            if e.row == "R207.3"
                && matches!(e.effect, Effect::StartSideTasks)
                && w.tickers.get(&e.input_rid).copied().unwrap_or(0) >= 2
            {
                self.hit(&mut hits, 4, e.row, e.input_rid);
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
                )
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
            "R211.2" => {
                self.idle_for_finished_ids.insert(rid);
            }
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
            let window = match d.ended_by {
                Some(EndedBy::Recover {
                    interrupted: Some(samples),
                }) if samples < crate::lifecycle::AUTO_TRANSCRIBE_MIN_SAMPLES => 0,
                Some(EndedBy::Recover {
                    interrupted: Some(_),
                }) => 3,
                Some(EndedBy::Stop(Delivery::Unsealed)) => 2,
                _ => 1,
            };
            self.hits.refused_in_teardown_gap[window] += 1;
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
                if !*found && earlier.is_some() {
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
            // PD-13 is fixed (#1191): the unwinding guard claims the terminal (R204a.7), so
            // a panic is no PD; the end-of-run liveness check holds it to a terminal.
            Note::TaskPanicked { rid, .. } => {
                self.panicked_ids.insert(*rid);
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
        let claim = w.ledger.ctx[&rid];
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
        let (perf, _) = w.perf.outcome.get(&rid)?;
        let claim = w.ledger.ctx.get(&rid)?;
        SIGNATURES.iter().position(|signature| {
            signature
                .terminal
                .contains(&(*terminal_row, *outcome, *code))
                && signature.claim_status.contains(&claim.status_before)
                && signature.perf.admits(*perf)
                && !w.perf.no_run.contains(&rid)
                && self.precondition(signature.pre, w, rid)
        })
    }

    fn precondition(&self, pre: Pre, w: &World, rid: Rid) -> bool {
        let claim = w.ledger.ctx[&rid];
        let completion = w.perf.ctx[&rid];
        let claim_current = claim.input_rid == claim.current;
        match pre {
            Pre::LiveClaimThenPerf => claim_current && claim.step < completion.step,
        }
    }

    /// End-of-run: SP4(a) and liveness (C7.3), per generation. The evidence each
    /// exemption needs is the recording's own (never a whole run's).
    pub fn on_end(&mut self, w: &World, quiescent_tasks: &[&'static str]) {
        self.sp4a(w);
        for (_, row) in &w.perf.dropped {
            *self.hits.dropped_completions.entry(row).or_default() += 1;
        }
        for (rid, row) in &w.perf.dropped {
            if *row == "R205.5" {
                if let Some((_, _, winner)) = w.ledger.claimed.get(rid) {
                    *self.hits.cancel_lost_to.entry(winner).or_default() += 1;
                }
            }
        }
        for (_, row) in w.perf.outcome.values() {
            *self.hits.completed_by.entry(row).or_default() += 1;
        }
        for (rid, row) in &w.perf.late {
            let cancelled = |ledger_row: Option<&&'static str>| ledger_row == Some(&"R205.5");
            if cancelled(w.ledger.claimed.get(rid).map(|(_, _, row)| row))
                && cancelled(w.perf.outcome.get(rid).map(|(_, row)| row))
            {
                *self.hits.late_after_cancel.entry(row).or_default() += 1;
            }
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
            // A run that ends in `Recovering` is stuck: nothing restores `Idle` (the
            // PD-18 / PD-19 stranding is fixed, so no run is exempt).
            Status::Recovering => {
                problems.push(format!("status Recovering at the end for id {current}"))
            }
            Status::Starting | Status::Recording
                if open_l4 == Some(current) || self.dropped_ids.contains(&current) => {}
            other => problems.push(format!("status {other:?} at the end")),
        }
        // The PD-13 fix, pinned (#1191): a panicked task whose recording the supervisor's
        // `Idle` also ended has the unwinding guard's terminal (claimed in its `Pre`).
        for rid in self.panicked_ids.intersection(&self.idle_for_finished_ids) {
            if matches!(
                w.ledger.claimed.get(rid),
                Some((
                    TerminalOutcome::PipelineFailure,
                    ErrCode::WorkerPanicked,
                    "R204a.pre"
                ))
            ) {
                self.hits.panicked_task_idle_claimed_by_guard += 1;
            }
        }
        for rid in &w.ledger.accepted {
            if !w.ledger.claimed.contains_key(rid)
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
