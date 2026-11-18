//! Test builds: the **synchronous replay** of the live capture sites (appendix C8.5
//! E7, E8).
//!
//! A replay arms the shadow without `MURMUR_SHADOW` and compares **on the calling
//! thread** (`Transport::set_sync`): no queue, no comparison thread, fully
//! deterministic. The golden traces and the bug replays run the real shell, so every
//! live capture site feeds the real comparator; a replay ends by checking that every
//! flight was closed exactly once, and returns what the comparison counted.
//!
//! Replays are process-global and are serialized by the caller (the golden harness
//! holds `GOLDEN_LOCK`); a test that arms one must not run beside tests that drive
//! the shell (`cargo test --lib -- --test-threads=1`, or nextest's process per test).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::compare::{Comparator, MismatchEvent};
use super::summary::Stats;
use super::transport::{Transport, QUEUE_CAPACITY};

/// One armed replay. [`Replay::end`] disarms it.
pub(crate) struct Replay {
    events: Arc<Mutex<Vec<MismatchEvent>>>,
    transport: Arc<Transport>,
    ended: bool,
}

/// What a replay's comparison counted.
pub(crate) struct Report {
    pub stats: Stats,
    pub events: Vec<MismatchEvent>,
}

/// Counters summed over every replay of this process (E3 and E8 evidence).
static TOTALS: Mutex<Option<Totals>> = Mutex::new(None);

#[derive(Clone, Debug, Default)]
pub(crate) struct Totals {
    pub replays: u64,
    pub compared: u64,
    pub mismatched_root: u64,
    pub abandoned: u64,
    pub unverifiable: u64,
    pub per_row: BTreeMap<(u16, u16), u64>,
    pub pd_witness: BTreeMap<u8, u64>,
}

/// The totals so far.
pub(crate) fn totals() -> Totals {
    TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// Forget the totals (a test that measures from zero).
pub(crate) fn reset_totals() {
    *TOTALS.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

impl Replay {
    /// Arm the shadow in synchronous mode with the production comparator.
    pub(crate) fn begin() -> Self {
        let events: Arc<Mutex<Vec<MismatchEvent>>> = Arc::default();
        let sink_events = Arc::clone(&events);
        let comparator = Comparator::production(Box::new(move |event| {
            sink_events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event);
        }));
        // A transport of its own: `seq` starts at 1, in step with the comparator.
        let transport = Transport::new(QUEUE_CAPACITY);
        transport.set_sync(Some(comparator));
        super::set_test_transport(Some(Arc::clone(&transport)));
        Self {
            events,
            transport,
            ended: false,
        }
    }

    /// Disarm, check that every recorder opened was closed exactly once, and report.
    /// Call it after everything the replay drove has been dropped, so every flight
    /// that did not finish has sent its `Close{completed:false}`.
    pub(crate) fn end(mut self) -> Report {
        self.ended = true;
        super::set_test_transport(None);
        let stats = self.transport.with_sync(|comparator| {
            comparator.assert_conserved();
            comparator.stats.clone()
        });
        self.transport.set_sync(None);
        let events = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut totals = TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
        let totals = totals.get_or_insert_with(Totals::default);
        totals.replays += 1;
        totals.compared += stats.compared;
        totals.mismatched_root += stats.mismatched_root;
        totals.abandoned += stats.abandoned;
        totals.unverifiable += stats.unverifiable;
        for (row, row_stats) in &stats.per_row {
            *totals.per_row.entry(*row).or_default() += row_stats.compared;
        }
        for (pd, count) in &stats.pd_witness {
            *totals.pd_witness.entry(*pd).or_default() += count;
        }
        Report { stats, events }
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        // A replay abandoned by a panic must not leave the shadow armed for the next
        // test.
        if !self.ended {
            super::set_test_transport(None);
            self.transport.set_sync(None);
        }
    }
}

impl Report {
    /// The E7 gate for one replay: no mismatch, no loss, no panic, nothing skipped.
    pub(crate) fn assert_clean(&self, what: &str) {
        let s = &self.stats;
        // Findings mode (D-S2c-6 e): record every divergence (row, site, diff kind) and
        // go on, so one run reports all of them together. The gate itself never runs in
        // this mode.
        if std::env::var_os("MURMUR_E7_COLLECT").is_some() {
            for event in &self.events {
                let rows: Vec<_> = s
                    .per_row
                    .iter()
                    .filter(|(row, stats)| row.0 == event.site_id && stats.mismatched > 0)
                    .map(|(row, _)| row.1)
                    .collect();
                eprintln!(
                    "E7FINDING sub_run={what:?} site={} rows={rows:?} input_kind={} diff_kind={}",
                    event.site_id, event.input_kind, event.diff_kind
                );
            }
            return;
        }
        assert!(
            self.events.is_empty()
                && s.mismatched_root == 0
                && s.followup == 0
                && s.shadow_panic == 0
                && s.dropped == 0
                && s.lost_open == 0
                && s.abandoned_by_loss == 0
                && s.abandoned_by_panic == 0
                && s.skipped_incomplete == 0
                && s.witness_overflow == 0,
            "{what}: the shadow reported a divergence: events {:?}, stats {s:?}",
            self.events
        );
    }
}
