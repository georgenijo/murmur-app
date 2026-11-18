//! A counting allocator hook for the shadow's no-allocation test (D-S2c-1 a).
//!
//! The lib's own test binary, built with the `dictation-shadow` feature, installs
//! [`CountingAllocator`] as the global allocator (`lib.rs`). It forwards every call
//! to the allocator the app really uses (the macOS Rust-zone allocator, or the
//! system one elsewhere) and, only on a thread that armed it through
//! [`count_heap_calls`], counts allocation **and deallocation** calls (D-S2c-2 b: the
//! live path must neither allocate nor free). The counters are const-initialised
//! thread-locals without destructors, so reading them from inside the allocator
//! cannot allocate or recurse, and other test threads never disturb a measurement.

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;

#[cfg(target_os = "macos")]
static INNER: crate::alloc::RustZoneAllocator = crate::alloc::RustZoneAllocator;
#[cfg(not(target_os = "macos"))]
static INNER: std::alloc::System = std::alloc::System;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    /// An optional condition a counted call must also meet (it must not allocate): the
    /// probe at the shell counts only the calls made while the dictation lock is held.
    static ONLY_WHEN: Cell<Option<fn() -> bool>> = const { Cell::new(None) };
    static IN_GATE: Cell<bool> = const { Cell::new(false) };
}

/// Forwards to the app's allocator, counting allocation calls on an armed thread.
pub(crate) struct CountingAllocator;

// Installed for the lib's test binary only (this module is `cfg(test)` inside the
// `dictation-shadow` feature). `lib.rs` leaves the macOS Rust-zone allocator out of
// that configuration so there is exactly one global allocator.
#[global_allocator]
static COUNTING: CountingAllocator = CountingAllocator;

fn note() {
    // `try_with`: a thread that is being torn down has no counters left.
    let _ = ARMED.try_with(|armed| {
        if armed.get() && gate_open() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

fn gate_open() -> bool {
    // The gate runs inside the allocator: a call it makes into the allocator must not
    // evaluate it again.
    if IN_GATE.try_with(Cell::get).unwrap_or(true) {
        return false;
    }
    let _ = IN_GATE.try_with(|busy| busy.set(true));
    let open = ONLY_WHEN
        .try_with(|gate| gate.get().is_none_or(|gate| gate()))
        .unwrap_or(true);
    let _ = IN_GATE.try_with(|busy| busy.set(false));
    open
}

fn note_free() {
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            let _ = DEALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

// SAFETY: every method forwards unchanged to the allocator the app already uses; the
// only addition is a counter update that performs no allocation.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { INNER.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_free();
        unsafe { INNER.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { INNER.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { INNER.realloc(ptr, layout, new_size) }
    }
}

/// Run `f` and return its result with the number of allocation calls **this thread**
/// made while it ran.
pub(crate) fn count_allocations<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let (result, allocations, _) = count_heap_calls(f);
    (result, allocations)
}

/// Run `f` and return its result with the number of allocation calls **this thread**
/// made while `only_when()` held (it must not allocate or lock): the calls made while
/// a lock of interest was held.
pub(crate) fn count_allocations_when<R>(
    only_when: fn() -> bool,
    f: impl FnOnce() -> R,
) -> (R, u64) {
    ONLY_WHEN.with(|gate| gate.set(Some(only_when)));
    let (result, allocations, _) = count_heap_calls(f);
    ONLY_WHEN.with(|gate| gate.set(None));
    (result, allocations)
}

/// Run `f` and return its result with the allocation and the deallocation calls
/// **this thread** made while it ran. Anything `f` drops is freed inside the
/// measurement; the returned value is not.
pub(crate) fn count_heap_calls<R>(f: impl FnOnce() -> R) -> (R, u64, u64) {
    ALLOCATIONS.with(|count| count.set(0));
    DEALLOCATIONS.with(|count| count.set(0));
    ARMED.with(|armed| armed.set(true));
    let result = f();
    ARMED.with(|armed| armed.set(false));
    (
        result,
        ALLOCATIONS.with(Cell::get),
        DEALLOCATIONS.with(Cell::get),
    )
}
