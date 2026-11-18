//! The preserved-defect allowlist as the shadow counts it (appendix C7.1, C8.3).
//!
//! A decision whose row is a PD signature is compared exactly like any other: both
//! sides already carry the PD's *preserved* behaviour (the `decide_*` functions
//! keep each defect on purpose), so a PD never produces a mismatch by itself. What
//! the allowlist adds is the witness count (`pd_witness[PD-n]`, E8): the PD's row
//! compared clean. A PD row whose effect list differs from the live path's **is** a
//! mismatch like any other, because the comparison is not relaxed.
//!
//! PD-11, PD-14 and PD-17 are not row signatures (they are properties of an
//! effect's implementation, or latent), and PD-9 and PD-10 are marked on rows whose
//! decided effects the comparison already covers; they are listed with the row
//! they are marked on.

/// `(PD number, the C3 rows whose signature it is)`, from the C7.1 table.
pub(crate) const PD_ROWS: &[(u8, &[&str])] = &[
    (
        1,
        &["R215.3", "R216.1", "R216.2", "R216.3", "R216.4", "R216.5"],
    ),
    (2, &["R211.2"]),
    (
        3,
        &[
            "R102.2", "R102.3", "R102.4", "R102.5", "R102.6", "R202.3", "R202.4",
        ],
    ),
    (4, &["R207.3"]),
    (5, &["R207.2", "R209.2"]),
    (
        6,
        &[
            "R204a.2", "R204a.3", "R204b.2", "R203.2", "R203.2r", "R203.5r", "R203.7r", "R102.3",
        ],
    ),
    (7, &["R211.3"]),
    (8, &["R103.2"]),
    (9, &["R101.15"]),
    (10, &["R102.2"]),
    (12, &["R201.1"]),
    (13, &["R204a.6"]),
    (15, &["R201.6", "R201.7", "R215.1", "R216.1"]),
    (16, &["R205.5"]),
    (
        18,
        &[
            "R101.12", "R216.1", "R211.2", "R211.3", "R211.6", "R203.4", "R203.6",
        ],
    ),
];

/// The PDs whose signature `row` is.
pub(crate) fn pds_for_row(row: &str) -> impl Iterator<Item = u8> + '_ {
    PD_ROWS
        .iter()
        .filter(move |(_, rows)| rows.contains(&row))
        .map(|(pd, _)| *pd)
}
