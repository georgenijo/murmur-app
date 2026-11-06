//! Exercise the production adapter on the process main thread without opening
//! the app, user data, capture devices, or a Tauri runtime.
#[cfg(target_os = "macos")]
#[path = "../src/main_thread_queue.rs"]
mod main_thread_queue;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use main_thread_queue::MainThreadQueue;
    use murmur_core::context::MainThread;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSDate, NSOperationQueue, NSRunLoop};
    use std::{
        sync::{mpsc, Arc, Mutex},
        time::{Duration, Instant},
    };

    let mtm = MainThreadMarker::new().expect("test must own the process main thread");
    let _app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let queue = MainThreadQueue::default();
    let state = Arc::new(Mutex::new(Vec::new()));
    let mut guard = state.lock().unwrap();
    let output = state.clone();
    let nested_queue = queue.clone();
    queue
        .dispatch(Box::new(move || {
            output
                .lock()
                .unwrap()
                .push((0, MainThreadMarker::new().is_some()));
            let nested_output = output.clone();
            nested_queue
                .dispatch(Box::new(move || {
                    nested_output
                        .lock()
                        .unwrap()
                        .push((3, MainThreadMarker::new().is_some()));
                }))
                .unwrap();
        }))
        .unwrap();
    // An inline callback would deadlock on the held lock. Check explicitly
    // that no partial state was exposed before returning from dispatch.
    assert!(guard.is_empty());

    let background_queue = queue.clone();
    let output = state.clone();
    std::thread::spawn(move || {
        background_queue
            .dispatch(Box::new(move || {
                output
                    .lock()
                    .unwrap()
                    .push((1, MainThreadMarker::new().is_some()));
            }))
            .unwrap();
    })
    .join()
    .unwrap();
    assert!(guard.is_empty());
    // A task panic must not unwind across the ObjC block or stop the FIFO.
    queue
        .dispatch(Box::new(|| panic!("synthetic dispatch panic")))
        .unwrap();
    let output = state.clone();
    queue
        .dispatch(Box::new(move || {
            output
                .lock()
                .unwrap()
                .push((2, MainThreadMarker::new().is_some()));
        }))
        .unwrap();
    guard.push((99, true));
    drop(guard);

    let deadline = Instant::now() + Duration::from_secs(5);
    while state.lock().unwrap().len() < 5 && Instant::now() < deadline {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert_eq!(
        *state.lock().unwrap(),
        [(99, true), (0, true), (1, true), (2, true), (3, true)]
    );

    // The empty-to-scheduled transition must also work after a completed drain.
    let output = state.clone();
    queue
        .dispatch(Box::new(move || output.lock().unwrap().push((4, true))))
        .unwrap();
    assert_eq!(state.lock().unwrap().len(), 5);
    while state.lock().unwrap().len() < 6 && Instant::now() < deadline {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert_eq!(state.lock().unwrap().last(), Some(&(4, true)));

    // Start with an empty, unscheduled adapter. Release the background caller
    // from a native main-queue callback, proving the main run loop is already
    // servicing work. Do not join the caller until its task has run.
    let empty_queue = MainThreadQueue::default();
    let executions = Arc::new(Mutex::new(Vec::new()));
    let output = executions.clone();
    let (start, started) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        empty_queue
            .dispatch(Box::new(move || {
                output
                    .lock()
                    .unwrap()
                    .push(MainThreadMarker::new().is_some());
            }))
            .unwrap();
    });
    let start_worker = block2::RcBlock::new(move || {
        let _ = start.send(());
    });
    // SAFETY: Foundation copies the block, which owns its channel sender.
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&start_worker) };

    let deadline = Instant::now() + Duration::from_secs(5);
    while executions.lock().unwrap().is_empty() && Instant::now() < deadline {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert_eq!(
        *executions.lock().unwrap(),
        [true],
        "background dispatch into an empty queue must run once on main within five seconds"
    );
    worker.join().unwrap();
    // Continue servicing the loop to detect an accidental duplicate callback.
    let deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < deadline {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert_eq!(*executions.lock().unwrap(), [true]);
    println!("main-thread queue: deferred main callers, FIFO, main-thread affinity, nested dispatch, panic containment, rescheduling, active-run-loop background dispatch passed");
}
