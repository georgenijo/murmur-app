//! Idle-leak soak: the Mode watcher polls `query_frontmost_app_identity` every
//! 500 ms from a Tokio worker that has no AppKit run-loop autorelease pool.
//! Without a pool inside the AppKit query helpers, objects AppKit and Launch
//! Services autorelease internally accumulate in the default malloc zone.
//!
//! The test drives the off-main AppKit query helpers on a plain background
//! thread (no pool, like a Tokio worker) and asserts the non-Rust malloc zones
//! stay bounded. It measures process-wide zone growth, so run it alone:
//!
//! ```text
//! cargo test --lib frontmost::soak_tests -- --ignored --nocapture --test-threads=1
//! ```

use crate::alloc;

const ITERATIONS: usize = 5_000;
/// Growth budget across all iterations. Unpooled, the default zone grows by
/// ~1.7 KB per round (8.5 MB over 5000 rounds); pooled, only one-time cache
/// fill remains (~0.3 MB), so the budget sits well between the two.
const MAX_GROWTH_BYTES: u64 = 1024 * 1024;

fn poll_appkit_queries(iterations: usize) {
    for _ in 0..iterations {
        let _ = super::query_frontmost_app_identity();
        let _ = super::capture_delivery_target_snapshot();
        let _ = super::list_running_applications();
        let _ = crate::commands::integrations::notchpill_installed();
    }
}

#[test]
#[ignore = "soak: measures process-wide malloc zone growth; run locally"]
fn appkit_queries_on_background_thread_do_not_grow_the_default_zone() {
    std::thread::spawn(|| {
        // Warm AppKit/Launch Services caches so one-time allocations do not count.
        poll_appkit_queries(500);

        let before = alloc::ffi_heap_bytes();
        poll_appkit_queries(ITERATIONS);
        let after = alloc::ffi_heap_bytes();

        let growth = after.saturating_sub(before);
        println!(
            "appkit soak: {ITERATIONS} rounds, default-zone heap {before} -> {after} bytes (growth {growth} B, {} B/round)",
            growth / ITERATIONS as u64
        );
        assert!(
            growth <= MAX_GROWTH_BYTES,
            "default malloc zone grew {growth} bytes over {ITERATIONS} AppKit query rounds"
        );
    })
    .join()
    .expect("soak thread panicked");
}
