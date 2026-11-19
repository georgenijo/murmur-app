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

/// The live shell's entry/header books: the raw counters less the named non-family
/// acquisitions of D, which are reported on their own.
pub(crate) fn conservation() -> ConservationReport {
    let raw: ConservationCounts = crate::dictation_executor::shadow::conservation_counts();
    let (non_family, unclassified) = sites::non_family_counts();
    let named: u64 = non_family.values().sum();
    ConservationReport {
        total: ConservationCounts {
            entries: raw.entries.saturating_sub(named),
            ..raw
        },
        non_family,
        unclassified,
    }
}
