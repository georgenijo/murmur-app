//! The shadow's counters, `dictation-shadow-summary.json` and the file-only mismatch
//! event sink (appendix C8.4 steps 4 and 5, decision D-S2c-1 b and f).
//!
//! Both files are **numeric only** and are written by the comparison thread, never by
//! the live path and never through `tracing`: nothing here reaches
//! `TauriEmitterLayer`, app `State`, capture health, the event buffer or a window.
//!
//! * `dictation-shadow-summary.json`: counters keyed by fixed names, `per_row` keyed
//!   by `"<site_id>.<row ordinal>"`, `lost_by_site` keyed by site and `pd_witness`
//!   keyed by `"PD-<n>"`. No content and no recording or operation ids. Written every
//!   five minutes and once on a clean exit, after the thread drained its queue; every
//!   write goes to a unique temporary file that is renamed over the previous summary,
//!   so a reader never sees a half-written file and two writers never share a name.
//! * `dictation-shadow-events.jsonl`: one line per mismatch event the rate limit let
//!   through, `{"event_code":..,"site_id":..,"input_kind":..,"diff_kind":..}`,
//!   validated by the telemetry sanitizer's own range check and truncated at the
//!   start of every session (at most 50 lines).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Map, Value};

use super::types::TickerSkipReason;

/// The summary's file name, in the log directory.
pub(crate) const SUMMARY_FILE: &str = "dictation-shadow-summary.json";

/// The mismatch events' file name, in the log directory.
pub(crate) const EVENTS_FILE: &str = "dictation-shadow-events.jsonl";

/// The registered event code the sink writes (C8.4, unchanged).
pub(crate) const EVENT_CODE: &str = "pipeline.dictation_shadow_mismatch";

/// Per C3 row: how many decisions were compared, how many mismatched and how many
/// times a preserved defect (C7.1) was witnessed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RowStats {
    pub compared: u64,
    pub mismatched: u64,
    pub pd_witness: u64,
    /// `PostDT:SD` spawns that returned before the gate, by reason code - 1
    /// ([`TickerSkipReason`]); recorded, never compared with the core (D-S2c-6).
    pub ticker_skipped: [u64; TickerSkipReason::ALL.len()],
}

/// Every counter of C8.4 step 5, plus the loss counters of D-S2c-3.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Decisions compared (a header was applied and its decision compared).
    pub compared: u64,
    /// Compared decisions that closed with no mismatch at any comparison point.
    pub matched: u64,
    /// Mismatches that were not explained by an earlier mismatch of the same
    /// recording (E1 requires 0).
    pub mismatched_root: u64,
    /// Mismatches with the `(site, row, diff_kind)` signature of an earlier root, or
    /// a later level of a record that already mismatched.
    pub followup: u64,
    /// Decisions not compared because a capture list overflowed: the header was
    /// incomplete.
    pub skipped_incomplete: u64,
    /// Messages the ring could not take: full, a slot still in flight, or a lost race.
    pub dropped: u64,
    /// The part of `dropped` that was contention (a slot in flight or a lost race),
    /// not a full ring.
    pub dropped_contended: u64,
    /// Messages of recorders created before the transport was published (the first
    /// enabled startup): counted, never queued, never blocking.
    pub dropped_unavailable: u64,
    /// Open records evicted past the 64-record cap. Each is closed `unverifiable`, so
    /// it is also counted there (D-S2c-5 b).
    pub lost_open: u64,
    /// Flights given up because a message was lost (or a header gap was seen with no
    /// counted loss): the staged batch, every open or held record, and a header that
    /// took its ticket before the loss, counted once per flight (D-S2c-3 b,
    /// Sol round 5, major 1).
    pub abandoned_by_loss: u64,
    /// Flights given up because the comparison itself panicked (`shadow_panic`): the
    /// panic can leave a record half applied, so everything open is given up, as for
    /// a loss.
    pub abandoned_by_panic: u64,
    /// Messages of a flight that was already given up, closed or evicted: dropped
    /// before they reach any record or cache.
    pub late_message_discarded: u64,
    /// Headers that arrived after a reset moved the sequence past them: discarded,
    /// never applied.
    pub late_header_discarded: u64,
    /// Pipeline-owned finishes closed without the owner's H1 token (no registration
    /// witnessed, or an ambiguous one): neither clean nor a mismatch (D-S2c-3 f,
    /// D-S2c-4 a).
    pub unverifiable: u64,
    /// Recorders the comparison saw open: every one is closed exactly once, by one of
    /// `matched`, `closed_mismatched`, `unverifiable`, `abandoned_by_loss`,
    /// `abandoned_by_panic`, `abandoned_pre_d` or `skipped_incomplete`,
    /// or is still open (D-S2c-4 b).
    pub opened: u64,
    /// Flights that closed with a mismatch already reported (a close kind of its own,
    /// so the conservation `opened == sum of closes` holds).
    pub closed_mismatched: u64,
    /// A second close of a recorder already closed: a no-op, counted. Zero outside a
    /// test that plants one.
    pub double_close_suppressed: u64,
    /// Messages claimed in the ring and never consumed when the final summary was
    /// written (shutdown only).
    pub unconsumed: u64,
    /// Of `unconsumed`, the slots a producer reserved and never published.
    pub reserved_unpublished: u64,
    /// Flights closed with `completed:false` (not an error).
    pub abandoned: u64,
    /// Flights that entered `run(Pre)` and were dropped before D entry, reported by
    /// `PreAbandoned` (not an error).
    pub abandoned_pre_d: u64,
    /// A capture list overflowed, or more than 32 messages were held for a header
    /// that has not arrived.
    pub witness_overflow: u64,
    /// The shadow's own decide panicked (`diff_kind 8`).
    pub shadow_panic: u64,
    /// Mismatch events the rate limit suppressed (C8.4 step 4).
    pub events_suppressed: u64,
    /// Losses by the site of the flight that lost them, for E4. Never keyed `0`.
    pub lost_by_site: BTreeMap<u16, u64>,
    /// Losses with no site (a message of a flight with no ticket, or a site out of
    /// range).
    pub loss_unattributed: u64,
    /// By `(site_id, row ordinal)`.
    pub per_row: BTreeMap<(u16, u16), RowStats>,
    /// Preserved-defect witnesses by PD number (E8).
    pub pd_witness: BTreeMap<u8, u64>,
}

impl Stats {
    pub(crate) fn row(&mut self, site_id: u16, row: u16) -> &mut RowStats {
        self.per_row.entry((site_id, row)).or_default()
    }

    /// The sum of every way a recorder is counted out (D-S2c-4 b).
    #[cfg(test)]
    pub(crate) fn closed_total(&self) -> u64 {
        self.matched
            + self.closed_mismatched
            + self.unverifiable
            + self.abandoned_by_loss
            + self.abandoned_by_panic
            + self.abandoned_pre_d
            + self.skipped_incomplete
    }

    /// The summary document. Numbers only.
    pub(crate) fn to_json(&self) -> Value {
        let per_row: Map<String, Value> = self
            .per_row
            .iter()
            .map(|((site, row), stats)| {
                // One counter per closed reason code, keyed by the code.
                let ticker_skipped: Map<String, Value> = TickerSkipReason::ALL
                    .iter()
                    .map(|reason| {
                        (
                            reason.code().to_string(),
                            json!(stats.ticker_skipped[reason.index()]),
                        )
                    })
                    .collect();
                (
                    format!("{site}.{row}"),
                    json!({
                        "compared": stats.compared,
                        "mismatched": stats.mismatched,
                        "pd_witness": stats.pd_witness,
                        "ticker_skipped": ticker_skipped,
                    }),
                )
            })
            .collect();
        let pd_witness: Map<String, Value> = self
            .pd_witness
            .iter()
            .map(|(pd, count)| (format!("PD-{pd}"), json!(count)))
            .collect();
        let lost_by_site: Map<String, Value> = self
            .lost_by_site
            .iter()
            .map(|(site, count)| (site.to_string(), json!(count)))
            .collect();
        json!({
            "compared": self.compared,
            "matched": self.matched,
            "mismatched_root": self.mismatched_root,
            "followup": self.followup,
            "skipped_incomplete": self.skipped_incomplete,
            "dropped": self.dropped,
            "dropped_contended": self.dropped_contended,
            "dropped_unavailable": self.dropped_unavailable,
            "lost_open": self.lost_open,
            "abandoned_by_loss": self.abandoned_by_loss,
            "abandoned_by_panic": self.abandoned_by_panic,
            "late_message_discarded": self.late_message_discarded,
            "late_header_discarded": self.late_header_discarded,
            "unverifiable": self.unverifiable,
            "opened": self.opened,
            "closed_mismatched": self.closed_mismatched,
            "double_close_suppressed": self.double_close_suppressed,
            "unconsumed": self.unconsumed,
            "reserved_unpublished": self.reserved_unpublished,
            "loss_unattributed": self.loss_unattributed,
            "abandoned": self.abandoned,
            "abandoned_pre_d": self.abandoned_pre_d,
            "witness_overflow": self.witness_overflow,
            "shadow_panic": self.shadow_panic,
            "events_suppressed": self.events_suppressed,
            "lost_by_site": lost_by_site,
            "per_row": per_row,
            "pd_witness": pd_witness,
        })
    }
}

/// A temporary name no other writer in this process or another one shares.
fn unique_temporary(dir: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    dir.join(format!(
        "{SUMMARY_FILE}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Write the summary into `dir`: a unique temporary file, then an atomic rename.
pub(crate) fn write_summary(dir: &Path, stats: &Stats) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(SUMMARY_FILE);
    let temporary = unique_temporary(dir);
    let body = serde_json::to_vec_pretty(&stats.to_json())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        // Never leave a stray temporary behind; the previous summary is untouched.
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The mismatch event file, written by the comparison thread only.
pub(crate) struct EventFile {
    path: PathBuf,
}

impl EventFile {
    /// Start a session's event file in `dir`: any earlier session's lines are
    /// truncated, so the file never grows past one session's 50 events.
    pub(crate) fn begin(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(EVENTS_FILE);
        std::fs::File::create(&path)?;
        Ok(Self { path })
    }

    /// Append one event. Values outside the documented ranges are dropped whole
    /// (the same check the telemetry sanitizer applies to the registered event), and
    /// the line is built from numbers only.
    pub(crate) fn append(&self, site_id: u16, input_kind: u16, diff_kind: u8) -> bool {
        if !crate::telemetry::is_valid_dictation_shadow_mismatch(
            u64::from(site_id),
            u64::from(input_kind),
            u64::from(diff_kind),
        ) {
            return false;
        }
        let line = json!({
            "event_code": EVENT_CODE,
            "site_id": site_id,
            "input_kind": input_kind,
            "diff_kind": diff_kind,
        });
        let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&self.path) else {
            return false;
        };
        writeln!(file, "{line}").is_ok()
    }
}
