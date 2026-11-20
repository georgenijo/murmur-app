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

/// One `FieldAdapter::project_lifecycle_view` call (the `DictationCore::project` entry),
/// with its caller. Counted always; the caller is remembered only while a test records
/// (`record_projections`). The lifecycle cutover's not-called test reads these.
pub(crate) fn projected(caller: &'static std::panic::Location<'static>) {
    PROJECTED.fetch_add(1, Ordering::Relaxed);
    if PROJECTION_SITES_ON.load(Ordering::Relaxed) {
        PROJECTION_SITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((caller.file(), caller.line()));
    }
}

static PROJECTED: AtomicU64 = AtomicU64::new(0);
static PROJECTION_SITES_ON: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static PROJECTION_SITES: std::sync::Mutex<Vec<(&'static str, u32)>> =
    std::sync::Mutex::new(Vec::new());

/// Start or stop remembering where `project_lifecycle_view` calls came from (all threads).
pub(crate) fn record_projections(on: bool) {
    PROJECTION_SITES_ON.store(on, Ordering::Relaxed);
}

/// The `(file, line)` of every projection since the last call, oldest first.
pub(crate) fn take_projection_sites() -> Vec<(&'static str, u32)> {
    std::mem::take(
        &mut *PROJECTION_SITES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// `DictationCore::project` entries so far.
pub(crate) fn projections() -> u64 {
    PROJECTED.load(Ordering::Relaxed)
}

/// What a lifecycle entry's C8.6 assertion read: the persisted M1/M2 and the live
/// `status`/`recording_id`, **as read at the entry**, before the entry re-synced a
/// divergence or ran any effect.
pub(crate) type EntryRead = (
    (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
);

static LIFECYCLE_ENTRIES: std::sync::Mutex<Vec<EntryRead>> = std::sync::Mutex::new(Vec::new());
static LIFECYCLE_ENTRIES_ON: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Start or stop remembering each lifecycle entry's read (all threads). Off by default:
/// remembering allocates, which the allocation pins under D must not see.
pub(crate) fn record_lifecycle_entries(on: bool) {
    LIFECYCLE_ENTRIES_ON.store(on, Ordering::Relaxed);
}

pub(crate) fn lifecycle_entry(
    persisted: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    live: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
) {
    if !LIFECYCLE_ENTRIES_ON.load(Ordering::Relaxed) {
        return;
    }
    LIFECYCLE_ENTRIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((persisted, live));
}

/// Every lifecycle entry's read since the last call, oldest first.
pub(crate) fn take_lifecycle_entries() -> Vec<EntryRead> {
    std::mem::take(
        &mut *LIFECYCLE_ENTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// What a stop entry's C8.6 assertion read ([`EntryRead`]) and where the entry is: the
/// `(file, line)` of its `FieldAdapter::stop_view` call.
pub(crate) type StopEntryRead = (EntryRead, (&'static str, u32));

static STOP_ENTRIES: std::sync::Mutex<Vec<StopEntryRead>> = std::sync::Mutex::new(Vec::new());
static STOP_ENTRIES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`record_lifecycle_entries`] for the stop family's entries (`FieldAdapter::stop_view`).
pub(crate) fn record_stop_entries(on: bool) {
    STOP_ENTRIES_ON.store(on, Ordering::Relaxed);
}

/// [`lifecycle_entry`] for the stop family: what a stop entry's C8.6 assertion read, as
/// read at the entry, with the entry's call site.
pub(crate) fn stop_entry(
    persisted: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    live: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    caller: &'static std::panic::Location<'static>,
) {
    if !STOP_ENTRIES_ON.load(Ordering::Relaxed) {
        return;
    }
    STOP_ENTRIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(((persisted, live), (caller.file(), caller.line())));
}

/// Every stop entry's read since the last call, oldest first.
pub(crate) fn take_stop_entries() -> Vec<StopEntryRead> {
    std::mem::take(
        &mut *STOP_ENTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// What a cancel entry's C8.6 assertion read ([`EntryRead`]) and where the entry is: the
/// `(file, line)` of its `FieldAdapter::cancel_view` call.
pub(crate) type CancelEntryRead = (EntryRead, (&'static str, u32));

static CANCEL_ENTRIES: std::sync::Mutex<Vec<CancelEntryRead>> = std::sync::Mutex::new(Vec::new());
static CANCEL_ENTRIES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`record_lifecycle_entries`] for the cancel family's entry (`FieldAdapter::cancel_view`).
pub(crate) fn record_cancel_entries(on: bool) {
    CANCEL_ENTRIES_ON.store(on, Ordering::Relaxed);
}

/// What a cancel entry's C8.6 assertion read, as read at the entry, with the entry's call
/// site.
pub(crate) fn cancel_entry(
    persisted: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    live: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    caller: &'static std::panic::Location<'static>,
) {
    if !CANCEL_ENTRIES_ON.load(Ordering::Relaxed) {
        return;
    }
    CANCEL_ENTRIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(((persisted, live), (caller.file(), caller.line())));
}

/// Every cancel entry's read since the last call, oldest first.
pub(crate) fn take_cancel_entries() -> Vec<CancelEntryRead> {
    std::mem::take(
        &mut *CANCEL_ENTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// What a start entry's (or query's) C8.6 read saw, and where: the persisted and the live
/// `(status, recording id)` **as read**, before the entry re-synced a divergence or ran any
/// effect, and the `(file, line)` of the `start_view` / `start_query_view` call. The start
/// cutover owns this type (D-S2c-18 amendment 1): no other cutover's probe types are shared.
pub(crate) type StartEntryRead = (
    (
        (
            murmur_dictation_core::status::Status,
            murmur_dictation_core::ids::RecordingId,
        ),
        (
            murmur_dictation_core::status::Status,
            murmur_dictation_core::ids::RecordingId,
        ),
    ),
    (&'static str, u32),
);

static START_ENTRIES: std::sync::Mutex<Vec<StartEntryRead>> = std::sync::Mutex::new(Vec::new());
static START_ENTRIES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static START_QUERIES: std::sync::Mutex<Vec<StartEntryRead>> = std::sync::Mutex::new(Vec::new());
static START_QUERIES_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// [`record_lifecycle_entries`] for the start family's entries (`FieldAdapter::start_view`).
pub(crate) fn record_start_entries(on: bool) {
    START_ENTRIES_ON.store(on, Ordering::Relaxed);
}

/// [`stop_entry`] for the start family: what a start entry's C8.6 assertion read, as read
/// at the entry, with the entry's call site.
pub(crate) fn start_entry(
    persisted: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    live: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    caller: &'static std::panic::Location<'static>,
) {
    if !START_ENTRIES_ON.load(Ordering::Relaxed) {
        return;
    }
    START_ENTRIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(((persisted, live), (caller.file(), caller.line())));
}

/// Every start entry's read since the last call, oldest first.
pub(crate) fn take_start_entries() -> Vec<StartEntryRead> {
    std::mem::take(
        &mut *START_ENTRIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// [`record_start_entries`] for the queries 401-404 (`FieldAdapter::start_query_view`).
pub(crate) fn record_start_queries(on: bool) {
    START_QUERIES_ON.store(on, Ordering::Relaxed);
}

/// What a query's persisted read saw (persisted, then live, as read), with its call site.
pub(crate) fn start_query(
    persisted: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    live: (
        murmur_dictation_core::status::Status,
        murmur_dictation_core::ids::RecordingId,
    ),
    caller: &'static std::panic::Location<'static>,
) {
    if !START_QUERIES_ON.load(Ordering::Relaxed) {
        return;
    }
    START_QUERIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(((persisted, live), (caller.file(), caller.line())));
}

/// Every query read since the last call, oldest first.
pub(crate) fn take_start_queries() -> Vec<StartEntryRead> {
    std::mem::take(
        &mut *START_QUERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
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
