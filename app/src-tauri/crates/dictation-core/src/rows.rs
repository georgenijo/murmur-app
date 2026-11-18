//! The C3 row index: every row id of appendix C3 and the decision site that
//! owns it, so the "C3 complete for every site" exit (plan §6 Stage 1) is a test
//! and not a claim.
//!
//! The table below is the list of row ids in the C3 tables (`R<site>.<n>`,
//! C3.1-C3.7), in appendix order, 141 rows over 24 sites (the 140 of the appendix tables and R301.6, the import's own
//! Smart Destination section, added by Stage 2a-5). It names no behaviour:
//! it only pins that each row id exists once, belongs to exactly one site, and
//! that every site is both reachable by an [`crate::inputs::Input`] and owns at
//! least one row. A `Decision` carries its row as a [`crate::decision::RowId`];
//! [`site_of_row`] is how a test or the shadow summary (C8.5) checks the row
//! belongs to the site that produced it.
//!
//! Site 202 is split by the `[pre]` context read: R202.1-.2 are the D-less
//! entry (`StopContextNoLock`, 2021) and R202.3-.5 the entry that takes D
//! (`StopContext`, 202).

use crate::decision::SiteId;

/// Every C3 row id and its owning site, in appendix order.
pub const C3_ROWS: &[(&str, SiteId)] = &[
    ("R101.1", SiteId::StartRequest),
    ("R101.2", SiteId::StartRequest),
    ("R101.3", SiteId::StartRequest),
    ("R101.4", SiteId::StartRequest),
    ("R101.5", SiteId::StartRequest),
    ("R101.6", SiteId::StartRequest),
    ("R101.7", SiteId::StartRequest),
    ("R101.8", SiteId::StartRequest),
    ("R101.9", SiteId::StartRequest),
    ("R101.10", SiteId::StartRequest),
    ("R101.11", SiteId::StartRequest),
    ("R101.12", SiteId::StartRequest),
    ("R101.13", SiteId::StartRequest),
    ("R101.14", SiteId::StartRequest),
    ("R101.15", SiteId::StartRequest),
    ("R102.1", SiteId::StartCaptureResult),
    ("R102.2", SiteId::StartCaptureResult),
    ("R102.3", SiteId::StartCaptureResult),
    ("R102.4", SiteId::StartCaptureResult),
    ("R102.5", SiteId::StartCaptureResult),
    ("R102.6", SiteId::StartCaptureResult),
    ("R103.0", SiteId::StartContext),
    ("R103.1", SiteId::StartContext),
    ("R103.2", SiteId::StartContext),
    ("R103.3", SiteId::StartContext),
    ("R103.4", SiteId::StartContext),
    ("R103.5", SiteId::StartContext),
    ("R103.6", SiteId::StartContext),
    ("R104.1", SiteId::ReadyPublish),
    ("R104.2", SiteId::ReadyPublish),
    ("R104.3", SiteId::ReadyPublish),
    ("R104.4", SiteId::ReadyPublish),
    ("R104.5", SiteId::ReadyPublish),
    ("R104.6", SiteId::ReadyPublish),
    ("R104.7", SiteId::ReadyPublish),
    ("R201.1", SiteId::StopRequest),
    ("R201.2", SiteId::StopRequest),
    ("R201.3", SiteId::StopRequest),
    ("R201.4", SiteId::StopRequest),
    ("R201.5", SiteId::StopRequest),
    ("R201.6", SiteId::StopRequest),
    ("R201.7", SiteId::StopRequest),
    ("R201.8", SiteId::StopRequest),
    ("R202.1", SiteId::StopContextNoLock),
    ("R202.2", SiteId::StopContextNoLock),
    ("R202.3", SiteId::StopContext),
    ("R202.4", SiteId::StopContext),
    ("R202.5", SiteId::StopContext),
    ("R203.1", SiteId::StopCaptureResult),
    ("R203.2", SiteId::StopCaptureResult),
    ("R203.2r", SiteId::StopCaptureResult),
    ("R203.3a", SiteId::StopCaptureResult),
    ("R203.3b", SiteId::StopCaptureResult),
    ("R203.3c", SiteId::StopCaptureResult),
    ("R203.3d", SiteId::StopCaptureResult),
    ("R203.3e", SiteId::StopCaptureResult),
    ("R203.4", SiteId::StopCaptureResult),
    ("R203.5", SiteId::StopCaptureResult),
    ("R203.5r", SiteId::StopCaptureResult),
    ("R203.6", SiteId::StopCaptureResult),
    ("R203.7", SiteId::StopCaptureResult),
    ("R203.7r", SiteId::StopCaptureResult),
    ("R203.8a", SiteId::StopCaptureResult),
    ("R203.8b", SiteId::StopCaptureResult),
    ("R203.8c", SiteId::StopCaptureResult),
    ("R203.8d", SiteId::StopCaptureResult),
    ("R203.8e", SiteId::StopCaptureResult),
    ("R203.9", SiteId::StopCaptureResult),
    ("R203.10", SiteId::StopCaptureResult),
    ("R204a.1", SiteId::GuardFinish),
    ("R204a.2", SiteId::GuardFinish),
    ("R204a.3", SiteId::GuardFinish),
    ("R204a.4", SiteId::GuardFinish),
    ("R204a.5a", SiteId::GuardFinish),
    ("R204a.5b", SiteId::GuardFinish),
    ("R204a.5c", SiteId::GuardFinish),
    ("R204a.5d", SiteId::GuardFinish),
    ("R204a.5e", SiteId::GuardFinish),
    ("R204a.6", SiteId::GuardFinish),
    ("R204b.1", SiteId::PipelineIdleEmit),
    ("R204b.2", SiteId::PipelineIdleEmit),
    ("R204b.3", SiteId::PipelineIdleEmit),
    ("R204b.4a", SiteId::PipelineIdleEmit),
    ("R204b.4b", SiteId::PipelineIdleEmit),
    ("R204b.4c", SiteId::PipelineIdleEmit),
    ("R204b.4d", SiteId::PipelineIdleEmit),
    ("R204b.4e", SiteId::PipelineIdleEmit),
    ("R204b.5", SiteId::PipelineIdleEmit),
    ("R204b.6", SiteId::PipelineIdleEmit),
    ("R205.1", SiteId::CancelRequest),
    ("R205.2", SiteId::CancelRequest),
    ("R205.3", SiteId::CancelRequest),
    ("R205.4", SiteId::CancelRequest),
    ("R205.5", SiteId::CancelRequest),
    ("R206.1", SiteId::CancelCaptureResult),
    ("R206.2", SiteId::CancelCaptureResult),
    ("R212.1", SiteId::SupAccepted),
    ("R207.1", SiteId::SupReady),
    ("R207.2", SiteId::SupReady),
    ("R207.3", SiteId::SupReady),
    ("R207.4", SiteId::SupReady),
    ("R207.5", SiteId::SupReady),
    ("R208.1", SiteId::SupStillConnecting),
    ("R208.2", SiteId::SupStillConnecting),
    ("R209.1", SiteId::SupRecovering),
    ("R209.2", SiteId::SupRecovering),
    ("R209.3", SiteId::SupRecovering),
    ("R209.4", SiteId::SupRecovering),
    ("R209.5", SiteId::SupRecovering),
    ("R209.6", SiteId::SupRecovering),
    ("R209.7", SiteId::SupRecovering),
    ("R210.1", SiteId::SupCleanupFinished),
    ("R210.2", SiteId::SupCleanupFinished),
    ("R210.3", SiteId::SupCleanupFinished),
    ("R210.4", SiteId::SupCleanupFinished),
    ("R213.1", SiteId::SupInitFailed),
    ("R213.2", SiteId::SupInitFailed),
    ("R214.1", SiteId::SupStalled),
    ("R214.2", SiteId::SupStalled),
    ("R215.1", SiteId::SupInterrupted),
    ("R215.2", SiteId::SupInterrupted),
    ("R215.3", SiteId::SupInterrupted),
    ("R216.1", SiteId::SupInterruptedShortIdle),
    ("R216.2", SiteId::SupInterruptedShortIdle),
    ("R216.3", SiteId::SupInterruptedShortIdle),
    ("R216.4", SiteId::SupInterruptedShortIdle),
    ("R216.5", SiteId::SupInterruptedShortIdle),
    ("R211.1", SiteId::SupIdle),
    ("R211.2", SiteId::SupIdle),
    ("R211.3", SiteId::SupIdle),
    ("R211.4", SiteId::SupIdle),
    ("R211.5", SiteId::SupIdle),
    ("R211.6", SiteId::SupIdle),
    ("R301.1", SiteId::ProcessAudioStart),
    ("R301.2", SiteId::ProcessAudioStart),
    ("R301.3", SiteId::ProcessAudioStart),
    ("R301.4", SiteId::ProcessAudioStart),
    ("R301.5", SiteId::ProcessAudioStart),
    ("R301.6", SiteId::ProcessAudioStart),
    ("R302.1", SiteId::ProcessAudioFinish),
    ("R302.2", SiteId::ProcessAudioFinish),
];

/// The site that owns `row`, if `row` is a C3 row id.
pub fn site_of_row(row: &str) -> Option<SiteId> {
    C3_ROWS
        .iter()
        .find(|(id, _)| *id == row)
        .map(|(_, site)| *site)
}

/// The rows a site owns, in appendix order.
pub fn rows_of_site(site: SiteId) -> impl Iterator<Item = &'static str> {
    C3_ROWS
        .iter()
        .filter(move |(_, owner)| *owner == site)
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The number of rows each C3 site owns, as counted in the appendix tables.
    const ROWS_PER_SITE: [(SiteId, usize); 24] = [
        (SiteId::StartRequest, 15),
        (SiteId::StartCaptureResult, 6),
        (SiteId::StartContext, 7),
        (SiteId::ReadyPublish, 7),
        (SiteId::StopRequest, 8),
        (SiteId::StopContext, 3),
        (SiteId::StopContextNoLock, 2),
        (SiteId::StopCaptureResult, 21),
        (SiteId::GuardFinish, 10),
        (SiteId::PipelineIdleEmit, 10),
        (SiteId::CancelRequest, 5),
        (SiteId::CancelCaptureResult, 2),
        (SiteId::SupReady, 5),
        (SiteId::SupStillConnecting, 2),
        (SiteId::SupRecovering, 7),
        (SiteId::SupCleanupFinished, 4),
        (SiteId::SupIdle, 6),
        (SiteId::SupAccepted, 1),
        (SiteId::SupInitFailed, 2),
        (SiteId::SupStalled, 2),
        (SiteId::SupInterrupted, 3),
        (SiteId::SupInterruptedShortIdle, 5),
        (SiteId::ProcessAudioStart, 6),
        (SiteId::ProcessAudioFinish, 2),
    ];

    #[test]
    fn row_ids_are_unique() {
        let mut seen = HashSet::new();
        for (id, _) in C3_ROWS {
            assert!(seen.insert(*id), "duplicate row id {id}");
        }
        assert_eq!(C3_ROWS.len(), 141);
    }

    #[test]
    fn every_site_owns_the_rows_the_appendix_counts() {
        let mut total = 0;
        for site in SiteId::ALL {
            let expected = ROWS_PER_SITE
                .iter()
                .find(|(s, _)| *s == site)
                .map(|(_, n)| *n)
                .unwrap_or_else(|| panic!("{site:?} missing from the count table"));
            assert!(expected > 0, "{site:?} owns no C3 row");
            assert_eq!(rows_of_site(site).count(), expected, "{site:?}");
            total += expected;
        }
        assert_eq!(total, C3_ROWS.len());
    }

    #[test]
    fn a_row_id_names_its_site_code() {
        // `R<code>.<n>`: the code is the site's `code()` (a trailing `a`/`b` is
        // 204a/204b), except 202 which is split into 2021 (R202.1-.2) and 202.
        for (id, site) in C3_ROWS {
            let prefix = id
                .strip_prefix('R')
                .and_then(|rest| rest.split('.').next())
                .unwrap_or_else(|| panic!("{id} is not R<site>.<n>"));
            let expected = match site {
                SiteId::StopContextNoLock => "202".to_string(),
                SiteId::GuardFinish => "204a".to_string(),
                SiteId::PipelineIdleEmit => "204b".to_string(),
                other => other.code().to_string(),
            };
            assert_eq!(prefix, expected, "{id} is owned by {site:?}");
            let ordinal = id.split('.').nth(1).unwrap_or("");
            assert!(
                ordinal.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "{id} has no row ordinal"
            );
        }
    }

    #[test]
    fn row_ordinals_are_distinct_within_a_site_and_follow_the_appendix() {
        // The shadow event carries the ordinal as `input_kind` (C8.4): R207.2 is
        // 2, and a suffixed row keeps its number and adds 100 per suffix step.
        assert_eq!(crate::decision::RowId("R207.2").ordinal(), 2);
        assert_eq!(crate::decision::RowId("R101.15").ordinal(), 15);
        assert_eq!(crate::decision::RowId("R103.0").ordinal(), 0);
        assert_eq!(crate::decision::RowId("R203.3a").ordinal(), 103);
        assert_eq!(crate::decision::RowId("R203.3e").ordinal(), 503);
        assert_eq!(crate::decision::RowId("R203.2r").ordinal(), 602);
        assert_eq!(crate::decision::RowId("R204b.4c").ordinal(), 304);
        let mut seen = HashSet::new();
        for (id, site) in C3_ROWS {
            let (_, tail) = id.split_once('.').expect("R<site>.<n>");
            assert!(
                tail.starts_with(|c: char| c.is_ascii_digit()),
                "{id} has no numeric ordinal"
            );
            assert!(
                seen.insert((site.code(), crate::decision::RowId(id).ordinal())),
                "{id} shares an ordinal with another row of {site:?}"
            );
        }
    }

    #[test]
    fn the_split_stop_context_rows_follow_the_context_read() {
        assert_eq!(site_of_row("R202.1"), Some(SiteId::StopContextNoLock));
        assert_eq!(site_of_row("R202.2"), Some(SiteId::StopContextNoLock));
        for id in ["R202.3", "R202.4", "R202.5"] {
            assert_eq!(site_of_row(id), Some(SiteId::StopContext), "{id}");
        }
        assert_eq!(site_of_row("R999.1"), None);
    }

    #[test]
    fn every_site_with_rows_is_reachable_by_an_input() {
        // The input side of the same proof is `inputs::tests::
        // every_decision_site_has_an_input`; together: each of the 24 sites has
        // an Input, an owner of rows, and each row an owner.
        let sites_with_rows: HashSet<u16> = C3_ROWS.iter().map(|(_, s)| s.code()).collect();
        let all: HashSet<u16> = SiteId::ALL.iter().map(|s| s.code()).collect();
        assert_eq!(sites_with_rows, all);
    }
}
