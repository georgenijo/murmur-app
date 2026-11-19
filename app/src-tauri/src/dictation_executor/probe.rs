//! Test builds only: counts the reads a family entry makes of the clock and of the live
//! fields (D-S2c-10 R4). A pin on these counts catches an extra `core_now()` or an extra view
//! borrow added under D (they allocate nothing, so an allocation pin cannot).

use std::sync::atomic::{AtomicU64, Ordering};

static CORE_NOW: AtomicU64 = AtomicU64::new(0);
static VIEWS: AtomicU64 = AtomicU64::new(0);
static SHADOW_READS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn core_now() {
    CORE_NOW.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn view() {
    VIEWS.fetch_add(1, Ordering::Relaxed);
}

/// The shadow read its own per-thread state (the keyboard write sequence, the ticker
/// mailbox). A disarmed shadow makes none (D-S2c-10 R4).
#[cfg(feature = "dictation-shadow")]
pub(crate) fn shadow_read() {
    SHADOW_READS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn shadow_reads() -> u64 {
    SHADOW_READS.load(Ordering::Relaxed)
}

/// `(core_now calls, view projections)` so far.
pub(crate) fn counts() -> (u64, u64) {
    (
        CORE_NOW.load(Ordering::Relaxed),
        VIEWS.load(Ordering::Relaxed),
    )
}
