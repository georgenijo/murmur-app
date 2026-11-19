//! The one place the shadow reads the live side's own counters (D-S2c-15 S3, S4).
//!
//! Everything here is a relaxed atomic load of a counter the live code already keeps
//! (the persisted-core divergence count, the entry/header books) or a read of the
//! acquisition-site table. The shadow adds nothing to the live path for them and holds
//! no port, lock or window; `shadow_never_executes` allows `dictation_executor` in this
//! file alone, and checks that it only loads.

use super::sites;
use super::summary::{ConservationCounts, ConservationReport};

/// `adapter::core_divergences()`: times a persisted M1/M2 copy differed from its live
/// field when an effect committed. A native pass needs 0.
pub(crate) fn core_divergences() -> u64 {
    crate::dictation_executor::adapter::core_divergences()
}

/// The live shell's entry/header books (D-S2c-15 S3). The shell's outcome counters are read
/// first and its D-less entries next; the acquisitions of D come from one snapshot of the
/// site table, read last, and the entries are that snapshot's total less its named
/// non-family part. A non-family read (the resource monitor's, say) that lands while the
/// summary is written is in the snapshot's total and its non-family part, or in neither:
/// it can never subtract from entries something the entries did not count.
pub(crate) fn conservation() -> ConservationReport {
    let raw: ConservationCounts = crate::dictation_executor::shadow::conservation_counts();
    let sites = sites::report();
    ConservationReport {
        total: ConservationCounts {
            entries: raw.entries.saturating_add(sites.family_entries()),
            ..raw
        },
        non_family: sites.non_family,
        unclassified: sites.unclassified,
    }
}
