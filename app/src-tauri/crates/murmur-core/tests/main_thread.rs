//! The core can dispatch while holding controller state, without a native host.
use murmur_core::context::{ContextError, MainThread, Task};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct QueuedHost(Mutex<VecDeque<Task>>);

impl MainThread for QueuedHost {
    fn dispatch(&self, task: Task) -> Result<(), ContextError> {
        self.0.lock().unwrap().push_back(task);
        Ok(())
    }
}

#[test]
fn dispatch_while_holding_state_defers_lock_acquisition_and_preserves_fifo() {
    let host = QueuedHost::default();
    let state = Arc::new(Mutex::new(Vec::new()));
    let mut guard = state.lock().unwrap();
    for value in [1, 2] {
        let state = state.clone();
        host.dispatch(Box::new(move || {
            // Fail promptly if a host ever invokes this inline while locked.
            state
                .try_lock()
                .expect("dispatch must defer lock acquisition")
                .push(value);
        }))
        .unwrap();
    }
    assert!(guard.is_empty());
    guard.push(0); // Complete the caller's state update after dispatch returns.
    drop(guard);
    loop {
        let task = host.0.lock().unwrap().pop_front();
        match task {
            Some(task) => task(),
            None => break,
        }
    }
    assert_eq!(*state.lock().unwrap(), [0, 1, 2]);
}
