//! Test builds only: counts the reads a family entry makes of the clock and of the live
//! fields (D-S2c-10 R4). A pin on these counts catches an extra `core_now()` or an extra view
//! borrow added under D (they allocate nothing, so an allocation pin cannot).

use std::sync::atomic::{AtomicU64, Ordering};

static CORE_NOW: AtomicU64 = AtomicU64::new(0);
static VIEWS: AtomicU64 = AtomicU64::new(0);
static SHADOW_READS: AtomicU64 = AtomicU64::new(0);
static FLAG_LOADS: AtomicU64 = AtomicU64::new(0);

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

/// The enable flag was read (`dictation_shadow::enabled()`). Each inserted block may make
/// exactly one such read, as its first operation: that read is the active check. A pin on the
/// count catches a second one (D-S2c-10 amendment 1 i).
#[cfg(feature = "dictation-shadow")]
#[track_caller]
pub(crate) fn flag_load() {
    FLAG_LOADS.fetch_add(1, Ordering::Relaxed);
    if RECORDING.with(std::cell::Cell::get) {
        let site = std::panic::Location::caller();
        SITES.with(|sites| sites.borrow_mut().push((site.file(), site.line())));
    }
}

/// Start (or stop) remembering the call sites of this thread's flag reads. Off by default:
/// remembering allocates, which the allocation pins must not see.
#[cfg(feature = "dictation-shadow")]
pub(crate) fn record_flag_sites(on: bool) {
    RECORDING.with(|recording| recording.set(on));
}

thread_local! {
    static RECORDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Where each flag read of this thread came from, for the pin's failure message.
    static SITES: std::cell::RefCell<Vec<(&'static str, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The call sites of this thread's flag reads since the last call, oldest first.
#[cfg(feature = "dictation-shadow")]
pub(crate) fn take_flag_sites() -> Vec<(&'static str, u32)> {
    SITES.with(|sites| std::mem::take(&mut *sites.borrow_mut()))
}

pub(crate) fn flag_loads() -> u64 {
    FLAG_LOADS.load(Ordering::Relaxed)
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
