//! The shadow's counters and `dictation-shadow-summary.json` (appendix C8.4 step 5).
//!
//! The file is **numeric only**: counters keyed by fixed names, `per_row` keyed by
//! `"<site_id>.<row ordinal>"` and `pd_witness` keyed by `"PD-<n>"`. It carries no
//! content and no recording or operation ids. It sits in the app's log directory
//! (the one the log viewer reads) and is written on a clean exit and every five
//! minutes, atomically (a temporary file renamed over the previous summary), so a
//! reader never sees a half-written file.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde_json::{json, Map, Value};

/// The summary's file name, in the log directory.
pub(crate) const SUMMARY_FILE: &str = "dictation-shadow-summary.json";

/// Per C3 row: how many decisions were compared, how many mismatched and how many
/// times a preserved defect (C7.1) was witnessed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RowStats {
    pub compared: u64,
    pub mismatched: u64,
    pub pd_witness: u64,
}

/// Every counter of C8.4 step 5, plus `events_suppressed` (the rate limit's count).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    /// Decisions compared (a header was applied and its decision compared).
    pub compared: u64,
    /// Compared decisions that closed with no mismatch at any comparison point.
    pub matched: u64,
    /// Mismatches that were not explained by an earlier mismatch of the same
    /// recording (E1 requires 0).
    pub mismatched_root: u64,
    /// Mismatches with the `(site, row, diff_kind)` signature of an earlier root.
    pub followup: u64,
    /// Decisions not compared because a lost record could have touched them.
    pub skipped_tainted: u64,
    /// Messages the bounded channel could not queue (channel full).
    pub dropped: u64,
    /// Headers that never arrived: a gap declared lost after more than 64 later
    /// headers, or a `Lost{seq}` from a dropped ticket.
    pub lost_gap: u64,
    /// Open records evicted past the 64-record cap.
    pub lost_open: u64,
    /// Flights closed with `completed:false` (not an error).
    pub abandoned: u64,
    /// Flights dropped after `run(Pre)` and before D entry, reported by
    /// `PreAbandoned` (not an error).
    pub abandoned_pre_d: u64,
    /// A buffer overflowed: the inline `Pre` buffer, or more than 32 messages held
    /// for a header that has not arrived.
    pub witness_overflow: u64,
    /// The shadow's own decide panicked (`diff_kind 8`).
    pub shadow_panic: u64,
    /// Mismatch events the rate limit suppressed (C8.4 step 4).
    pub events_suppressed: u64,
    /// By `(site_id, row ordinal)`.
    pub per_row: BTreeMap<(u16, u16), RowStats>,
    /// Preserved-defect witnesses by PD number (E8).
    pub pd_witness: BTreeMap<u8, u64>,
}

impl Stats {
    pub(crate) fn row(&mut self, site_id: u16, row: u16) -> &mut RowStats {
        self.per_row.entry((site_id, row)).or_default()
    }

    /// The summary document. Numbers only.
    pub(crate) fn to_json(&self) -> Value {
        let per_row: Map<String, Value> = self
            .per_row
            .iter()
            .map(|((site, row), stats)| {
                (
                    format!("{site}.{row}"),
                    json!({
                        "compared": stats.compared,
                        "mismatched": stats.mismatched,
                        "pd_witness": stats.pd_witness,
                    }),
                )
            })
            .collect();
        let pd_witness: Map<String, Value> = self
            .pd_witness
            .iter()
            .map(|(pd, count)| (format!("PD-{pd}"), json!(count)))
            .collect();
        json!({
            "compared": self.compared,
            "matched": self.matched,
            "mismatched_root": self.mismatched_root,
            "followup": self.followup,
            "skipped_tainted": self.skipped_tainted,
            "dropped": self.dropped,
            "lost_gap": self.lost_gap,
            "lost_open": self.lost_open,
            "abandoned": self.abandoned,
            "abandoned_pre_d": self.abandoned_pre_d,
            "witness_overflow": self.witness_overflow,
            "shadow_panic": self.shadow_panic,
            "events_suppressed": self.events_suppressed,
            "per_row": per_row,
            "pd_witness": pd_witness,
        })
    }
}

/// Write the summary into `dir`, atomically.
pub(crate) fn write_summary(dir: &Path, stats: &Stats) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(SUMMARY_FILE);
    let temporary = dir.join(format!("{SUMMARY_FILE}.tmp"));
    let body = serde_json::to_vec_pretty(&stats.to_json())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, &target)
}
