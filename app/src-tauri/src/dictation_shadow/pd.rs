//! The preserved-defect allowlist as the shadow counts it (appendix C7.1, C8.3,
//! decision D-S2c-1 d).
//!
//! A decision whose row is a PD signature is compared exactly like any other: both
//! sides already carry the PD's *preserved* behaviour (the `decide_*` functions keep
//! each defect on purpose), so a PD never produces a mismatch by itself. What the
//! allowlist adds is the witness count (`pd_witness[PD-n]`, E8), and **a PD is
//! credited only after its defining behaviour was witnessed** (D-S2b-7 a and d):
//!
//! * the record closed with every comparison point clean (full comparison on both
//!   sides), never merely because its row was decided;
//! * every effect that defines the PD ran (`need`) and none that would contradict it
//!   did (`absent`);
//! * a result that defines it was the real one (`term_lost`: the terminal claim
//!   lost, PD-10; `ticker_duplicate`: a second ticker registered for the same id,
//!   PD-4);
//! * a defect that needs two flights (PD-4 after the start's rendezvous, PD-18 after
//!   an attempt's teardown) needs an earlier **clean** row of the same recording
//!   (`prior`), so an ordinary short idle never credits PD-18 by itself.
//!
//! PD-11, PD-14 and PD-17 are not row signatures (properties of an effect's
//! implementation, or latent) and are not counted here. PD-1's row list drops
//! R215.3: that row hands over to R216, where the unguarded write actually happens.

use super::types::{EffectResult, TickerGate};

/// How a terminal claim must have resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TermNeed {
    /// Either result.
    Any,
    /// The claim lost the ledger (`claim_terminal` returned `false`).
    Lost,
}

/// One PD signature: the row and what must have been witnessed for it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PdDef {
    pub pd: u8,
    pub row: &'static str,
    /// Effect codes (`Effect::code`) that must have run.
    pub need: &'static [&'static str],
    /// Effect codes that must not have run.
    pub absent: &'static [&'static str],
    pub term: TermNeed,
    /// A second ticker registered for an id whose ticker already was.
    pub ticker_duplicate: bool,
    /// At least one of these rows closed clean earlier for the same recording id.
    /// Empty: no earlier row is needed.
    pub prior: &'static [&'static str],
    /// Credited when the flight was abandoned (`Close{completed:false}`) rather
    /// than completed. PD-9 is an abandonment defect.
    pub abandoned: bool,
}

const fn def(pd: u8, row: &'static str, need: &'static [&'static str]) -> PdDef {
    PdDef {
        pd,
        row,
        need,
        absent: &[],
        term: TermNeed::Any,
        ticker_duplicate: false,
        prior: &[],
        abandoned: false,
    }
}

const TEARDOWN: &[&str] = &["R216.1", "R211.2", "R211.3", "R211.6", "R203.4", "R203.6"];
const IDLE_WRITE: &[&str] = &["CX-", "ST", "KB", "EM"];

/// Every PD signature, from the C7.1 table.
pub(crate) const PD_DEFS: &[PdDef] = &[
    def(1, "R216.1", IDLE_WRITE),
    def(1, "R216.2", IDLE_WRITE),
    def(1, "R216.3", IDLE_WRITE),
    def(1, "R216.4", IDLE_WRITE),
    def(1, "R216.5", IDLE_WRITE),
    def(2, "R211.2", &["TERM"]),
    def(3, "R102.2", &["EM"]),
    def(3, "R102.3", &["EM"]),
    def(3, "R102.4", &["EM"]),
    def(3, "R102.5", &["EM"]),
    def(3, "R102.6", &["EM"]),
    def(3, "R202.3", &["KB", "EM"]),
    def(3, "R202.4", &["KB", "EM"]),
    PdDef {
        ticker_duplicate: true,
        prior: &["R104.2"],
        ..def(4, "R207.3", &["TERM", "SD"])
    },
    def(5, "R207.2", &["TERM"]),
    def(5, "R209.2", &["TERM"]),
    def(6, "R204a.2", &["EM"]),
    def(6, "R204a.3", &["EM"]),
    def(6, "R204b.2", &["EM"]),
    def(6, "R203.2", &["EM"]),
    def(6, "R203.2r", &["EM"]),
    def(6, "R203.5r", &["EM"]),
    def(6, "R203.7r", &["EM"]),
    def(6, "R102.3", &["EM"]),
    def(7, "R211.3", &["TERM", "ST"]),
    def(8, "R103.2", &["CX+"]),
    PdDef {
        absent: &["CS"],
        abandoned: true,
        ..def(9, "R101.15", &["PR"])
    },
    PdDef {
        term: TermNeed::Lost,
        ..def(10, "R102.2", &["TERM"])
    },
    def(12, "R201.1", &["TERM"]),
    PdDef {
        absent: &["TERM", "EM"],
        ..def(13, "R204a.6", &[])
    },
    def(15, "R201.6", &[]),
    def(15, "R201.7", &["ST"]),
    def(15, "R215.1", &["TI"]),
    def(15, "R216.1", &["TI"]),
    def(16, "R205.5", &["CX-", "MC", "TERM", "KB", "EM"]),
    PdDef {
        prior: TEARDOWN,
        ..def(18, "R101.12", &["ST", "EM"])
    },
];

/// What a closed record witnessed, as far as the allowlist cares.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PdEvidence<'a> {
    /// The C3 row the decision took.
    pub row: &'a str,
    /// The effects that ran, by legend code, with their results.
    pub ran: &'a [(&'static str, EffectResult)],
    /// The ticker registration attempts that were captured.
    pub tickers: &'a [TickerGate],
    /// The flight completed (`true`) or was abandoned (`false`).
    pub completed: bool,
    /// Rows that closed clean earlier for the same recording id.
    pub prior_rows: &'a [&'static str],
}

impl PdDef {
    fn witnessed(&self, evidence: &PdEvidence<'_>) -> bool {
        if evidence.row != self.row || evidence.completed == self.abandoned {
            return false;
        }
        let ran = |code: &str| evidence.ran.iter().any(|(c, _)| *c == code);
        if !self.need.iter().all(|code| ran(code)) || self.absent.iter().any(|code| ran(code)) {
            return false;
        }
        if self.term == TermNeed::Lost
            && !evidence
                .ran
                .iter()
                .any(|(c, r)| *c == "TERM" && *r == EffectResult::Claimed { won: false })
        {
            return false;
        }
        if self.ticker_duplicate
            && !evidence
                .tickers
                .iter()
                .any(|gate| gate.registered && gate.ticker_before == gate.id)
        {
            return false;
        }
        self.prior.is_empty()
            || evidence
                .prior_rows
                .iter()
                .any(|row| self.prior.contains(row))
    }
}

/// The PDs this evidence credits (each at most once per record).
pub(crate) fn credited(evidence: &PdEvidence<'_>) -> Vec<u8> {
    let mut pds: Vec<u8> = PD_DEFS
        .iter()
        .filter(|def| def.witnessed(evidence))
        .map(|def| def.pd)
        .collect();
    pds.sort_unstable();
    pds.dedup();
    pds
}
