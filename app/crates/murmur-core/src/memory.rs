/// Get process RSS in bytes via `memory-stats` (task_info on macOS).
pub fn get_process_rss_bytes() -> Option<u64> {
    memory_stats::memory_stats().map(|statistics| statistics.physical_mem as u64)
}

/// Legacy internal helper retained for existing model/benchmark logs.
pub fn get_process_rss_mb() -> u64 {
    get_process_rss_bytes().unwrap_or(0) / 1_048_576
}

/// Bytes read from the executable's Rust malloc zone and all process zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapUsage {
    pub rust_bytes: u64,
    pub total_bytes: u64,
}

impl HeapUsage {
    pub fn ffi_bytes(self) -> u64 {
        self.total_bytes.saturating_sub(self.rust_bytes)
    }
}

/// Read statistics without creating or installing an allocator.
///
/// # Safety
/// `rust_zone` must be a live, non-null macOS malloc zone that remains valid
/// throughout this call. The executable owns that zone for its entire lifetime.
#[cfg(target_os = "macos")]
pub unsafe fn read_heap_usage(rust_zone: *mut std::ffi::c_void) -> HeapUsage {
    #[repr(C)]
    #[derive(Default)]
    struct MallocStatistics {
        blocks_in_use: std::ffi::c_uint,
        size_in_use: usize,
        max_size_in_use: usize,
        size_allocated: usize,
    }
    unsafe extern "C" {
        fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut MallocStatistics);
    }
    let mut rust = MallocStatistics::default();
    let mut total = MallocStatistics::default();
    unsafe {
        malloc_zone_statistics(rust_zone, &mut rust);
        malloc_zone_statistics(std::ptr::null_mut(), &mut total);
    }
    HeapUsage {
        rust_bytes: rust.size_in_use as u64,
        total_bytes: total.size_in_use as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_difference_saturates_when_samples_cross() {
        assert_eq!(
            HeapUsage {
                rust_bytes: 2,
                total_bytes: 5
            }
            .ffi_bytes(),
            3
        );
        assert_eq!(
            HeapUsage {
                rust_bytes: 5,
                total_bytes: 2
            }
            .ffi_bytes(),
            0
        );
    }

    #[test]
    fn process_rss_is_available() {
        assert!(get_process_rss_bytes().unwrap() > 0);
    }
}
