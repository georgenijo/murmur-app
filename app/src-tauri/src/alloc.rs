//! Custom macOS malloc zone allocator for accurate Rust heap tracking.
//!
//! By routing all Rust allocations through a dedicated malloc zone ("RustHeapZone"),
//! we get kernel-level per-zone accounting via `malloc_zone_statistics()`.
//! C/C++ FFI code (whisper.cpp) continues using the default system zone.
//! This avoids the counter-drift problem that `cap` and other `GlobalAlloc` wrappers
//! suffer from when FFI code frees Rust-allocated memory.

use std::alloc::{GlobalAlloc, Layout};
use std::os::raw::{c_char, c_uint, c_void};
use std::sync::Once;

// --- macOS malloc zone FFI ---

#[repr(C)]
struct MallocZone {
    _opaque: [u8; 0],
}

unsafe extern "C" {
    fn malloc_create_zone(start_size: usize, flags: c_uint) -> *mut MallocZone;
    fn malloc_set_zone_name(zone: *mut MallocZone, name: *const c_char);
    fn malloc_zone_malloc(zone: *mut MallocZone, size: usize) -> *mut c_void;
    fn malloc_zone_memalign(zone: *mut MallocZone, align: usize, size: usize) -> *mut c_void;
    fn malloc_zone_realloc(zone: *mut MallocZone, ptr: *mut c_void, size: usize) -> *mut c_void;
    fn malloc_zone_free(zone: *mut MallocZone, ptr: *mut c_void);
}

// --- Zone singleton ---

static mut RUST_ZONE: *mut MallocZone = std::ptr::null_mut();
static INIT: Once = Once::new();

/// Zone name as a static null-terminated byte string.
/// MUST NOT use CString::new() here — it allocates via GlobalAlloc,
/// causing infinite recursion since the zone isn't created yet.
static ZONE_NAME: &[u8] = b"RustHeapZone\0";

fn rust_zone() -> *mut MallocZone {
    unsafe {
        INIT.call_once(|| {
            RUST_ZONE = malloc_create_zone(0, 0);
            malloc_set_zone_name(RUST_ZONE, ZONE_NAME.as_ptr() as *const c_char);
        });
        RUST_ZONE
    }
}

// --- GlobalAlloc implementation ---

pub struct RustZoneAllocator;

unsafe impl GlobalAlloc for RustZoneAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let zone = rust_zone();
        if layout.align() <= 16 {
            // macOS malloc returns 16-byte aligned pointers by default
            unsafe { malloc_zone_malloc(zone, layout.size()) as *mut u8 }
        } else {
            unsafe { malloc_zone_memalign(zone, layout.align(), layout.size()) as *mut u8 }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { malloc_zone_free(rust_zone(), ptr as *mut c_void) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let zone = rust_zone();
        if layout.align() <= 16 {
            // Default macOS malloc alignment — zone realloc preserves this.
            return unsafe { malloc_zone_realloc(zone, ptr as *mut c_void, new_size) as *mut u8 };
        }
        // Over-aligned: malloc_zone_realloc may not preserve stricter alignment
        // if the block moves, so allocate aligned + copy + free.
        let new_ptr = unsafe { malloc_zone_memalign(zone, layout.align(), new_size) as *mut u8 };
        if new_ptr.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, new_ptr, layout.size().min(new_size));
            malloc_zone_free(zone, ptr as *mut c_void);
        }
        new_ptr
    }
}

// --- Public query API ---

/// Memory breakdown: (rust_heap_bytes, total_heap_bytes).
///
/// - `rust_heap_bytes`: bytes currently allocated in the Rust zone
/// - `total_heap_bytes`: bytes across ALL malloc zones (Rust + C/FFI + system)
///
/// C/FFI heap ≈ total - rust.
fn memory_breakdown() -> murmur_core::memory::HeapUsage {
    // SAFETY: rust_zone initializes a process-lifetime malloc zone, also used by
    // the global allocator above. The core only reads its statistics.
    unsafe { murmur_core::memory::read_heap_usage(rust_zone().cast()) }
}

pub fn rust_heap_mb() -> u64 {
    rust_heap_bytes() / 1_048_576
}

pub fn ffi_heap_mb() -> u64 {
    ffi_heap_bytes() / 1_048_576
}

pub fn rust_heap_bytes() -> u64 {
    memory_breakdown().rust_bytes
}

pub fn ffi_heap_bytes() -> u64 {
    memory_breakdown().ffi_bytes()
}
