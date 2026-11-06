//! Windowless, synthetic AppKit termination check. No product profile, capture,
//! webview, or input automation. A watchdog makes a lost modal callback fail.
#[cfg(target_os = "macos")]
#[path = "../src/main_thread_queue.rs"]
mod main_thread_queue;
#[cfg(not(target_os = "macos"))]
fn main() {}
#[cfg(target_os = "macos")]
fn main() {
    use murmur_core::context::MainThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    use tao::{
        event::Event,
        event_loop::EventLoopBuilder,
        platform::{
            macos::{
                set_application_termination_handler, ActivationPolicy, ApplicationTerminationReply,
                EventLoopExtMacOS,
            },
            run_return::EventLoopExtRunReturn,
        },
    };
    let mtm = MainThreadMarker::new().unwrap();
    let mut event_loop = EventLoopBuilder::<()>::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Prohibited);
    let application = NSApplication::sharedApplication(mtm);
    let reply = Arc::new(AtomicBool::new(false));
    let callbacks = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let queue = main_thread_queue::MainThreadQueue::default();
    let start = Instant::now();
    let held = entered.clone();
    let released = reply.clone();
    let count = callbacks.clone();
    set_application_termination_handler(move || {
        if released.load(Ordering::Acquire) {
            return ApplicationTerminationReply::Now;
        }
        if !held.swap(true, Ordering::AcqRel) {
            let queue = queue.clone();
            let released = released.clone();
            let count = count.clone();
            std::thread::spawn(move || {
                assert!(MainThreadMarker::new().is_none());
                for _ in 0..65 {
                    let count = count.clone();
                    queue
                        .dispatch(Box::new(move || {
                            assert!(MainThreadMarker::new().is_some());
                            count.fetch_add(1, Ordering::AcqRel);
                        }))
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(100));
                }
                queue
                    .dispatch(Box::new(move || {
                        assert!(start.elapsed() >= Duration::from_secs(6));
                        assert_eq!(count.load(Ordering::Acquire), 65);
                        released.store(true, Ordering::Release);
                        NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
                            .replyToApplicationShouldTerminate(true);
                    }))
                    .unwrap();
            });
        }
        ApplicationTerminationReply::Later
    })
    .unwrap();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        std::process::exit(99);
    });
    let mut requested = false;
    event_loop.run_return(move |event, _, _| {
        if matches!(event, Event::MainEventsCleared) && !requested {
            requested = true;
            application.terminate(None);
        }
        if matches!(event, Event::LoopDestroyed) {
            assert!(entered.load(Ordering::Acquire));
            assert!(reply.load(Ordering::Acquire));
            assert_eq!(callbacks.load(Ordering::Acquire), 65);
        }
    });
}
