//! Queued macOS host adapter, also exercised by the native run-loop test.
use murmur_core::context::{ContextError, MainThread, Task};
use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Pending {
    tasks: VecDeque<Task>,
    scheduled: bool,
}

#[derive(Clone, Default)]
pub(crate) struct MainThreadQueue(Arc<Mutex<Pending>>);

impl MainThread for MainThreadQueue {
    fn dispatch(&self, task: Task) -> Result<(), ContextError> {
        let mut pending = self.0.lock().map_err(|_| ContextError::SchedulingFailed)?;
        pending.tasks.push_back(task);
        if !pending.scheduled {
            pending.scheduled = true;
            let queue = self.0.clone();
            // NSOperationQueue does not itself promise FIFO. One drain and one
            // shared FIFO serialize callers from every thread (including main).
            // Taking the Option consumes the drain's FnOnce state even if a
            // native block is invoked more than once.
            let once = Mutex::new(Some(queue));
            let block = block2::RcBlock::new(move || {
                if let Some(queue) = once.lock().unwrap().take() {
                    drain(&queue);
                }
            });
            // Unlike Tauri's run_on_main_thread, this always queues. The main
            // NSOperationQueue is serviced by the AppKit main run loop that
            // tao/Tauri drives; it needs no extra runtime or caller-thread path.
            // Background callers may have their task drained before returning.
            // SAFETY: Foundation copies the block; it owns all captured state.
            unsafe {
                objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&block)
            };
        }
        Ok(())
    }
}

fn drain(queue: &Mutex<Pending>) {
    loop {
        let task = {
            let mut pending = queue.lock().unwrap();
            match pending.tasks.pop_front() {
                Some(task) => task,
                None => {
                    pending.scheduled = false;
                    return;
                }
            }
        };
        // Consume each FnOnce outside the queue lock, allowing nested dispatch.
        // Rust unwinding must stop here, before returning through the ObjC block.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(task)) {
            tracing::error!("Main-thread task panicked");
            // A user-defined panic payload can itself panic when dropped.
            std::mem::forget(payload);
        }
    }
}
