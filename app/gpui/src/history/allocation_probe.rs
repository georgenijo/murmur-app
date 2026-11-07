// Gallery/test-only allocator forwarding. The scoped, thread-local probe detects a
// transcript-sized clone in the real renderer without sampling process memory.
pub struct RenderAllocator;
std::thread_local! {
    static LARGE_RENDER_ALLOCATIONS: std::cell::Cell<Option<(usize, usize)>> = const {
        std::cell::Cell::new(None)
    };
}
fn record_allocation(bytes: usize) {
    if bytes < 1024 * 1024 {
        return;
    }
    let _ = LARGE_RENDER_ALLOCATIONS.try_with(|probe| {
        if let Some((minimum, count)) = probe.get()
            && bytes >= minimum
        {
            probe.set(Some((minimum, count + 1)));
        }
    });
}
// SAFETY: every operation delegates the original pointer/layout to System;
// the probe uses only non-allocating, const-initialized thread-local Cells.
unsafe impl std::alloc::GlobalAlloc for RenderAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, bytes: usize) -> *mut u8 {
        record_allocation(bytes);
        unsafe { std::alloc::System.realloc(ptr, layout, bytes) }
    }
}
pub fn count_large_render_allocations(minimum: usize, render: impl FnOnce()) -> usize {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            LARGE_RENDER_ALLOCATIONS.with(|probe| probe.set(None));
        }
    }
    let _reset = Reset;
    LARGE_RENDER_ALLOCATIONS.with(|probe| probe.set(Some((minimum, 0))));
    render();
    LARGE_RENDER_ALLOCATIONS.with(|probe| probe.get().unwrap().1)
}
