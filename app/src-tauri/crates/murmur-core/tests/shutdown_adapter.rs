//! Exercise the same routing seam called by Tauri's runtime adapter, with
//! synthetic owner receipts and clock. No product host/profile is constructed.
#[path = "../../../src/shutdown/adapter.rs"]
mod adapter;
use adapter::{intercept_exit, reduce, Message};
use murmur_core::shutdown::*;
use std::sync::{mpsc::RecvTimeoutError, Mutex};
#[test]
fn exit_is_prevented_before_any_asynchronous_submission() {
    let order = std::cell::RefCell::new(vec![]);
    intercept_exit(
        false,
        || order.borrow_mut().push("prevent"),
        || order.borrow_mut().push("request"),
    );
    assert_eq!(*order.borrow(), ["prevent", "request"]);
    intercept_exit(
        true,
        || panic!("terminal permit prevented"),
        || panic!("second drain"),
    );
}
#[test]
fn independently_stalled_owner_never_blocks_deadline_routing() {
    for stalled in [Step::Admission, Step::Capture, Step::Helpers, Step::Writers] {
        let core = Mutex::new(Coordinator::default());
        let mut effect = reduce(&core, Ok(Message::Request(Request::Restart, 0)), 0).unwrap()[0];
        while effect.step != stalled {
            let outcome = match effect.step {
                Step::Admission => Outcome::Admission(true),
                Step::Capture => Outcome::Capture(true),
                Step::Helpers => Outcome::Helpers(true),
                Step::Writers => Outcome::Writers(WriteOutcome::Committed),
                _ => unreachable!(),
            };
            effect = reduce(&core, Ok(Message::Finished(Event { effect, outcome })), 1).unwrap()[0];
        }
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        let owner = std::thread::spawn(move || {
            blocked.recv().unwrap();
        });
        let effects = reduce(&core, Err(RecvTimeoutError::Timeout), 10_000).unwrap();
        assert_eq!(effects.last().unwrap().step, Step::Terminate);
        assert!(core.lock().unwrap().receipt.timed_out);
        assert_eq!(core.lock().unwrap().issue(), Some(Action::Quit));
        assert!(
            reduce(&core, Ok(Message::Request(Request::Restart, 0)), 10_001)
                .unwrap()
                .is_empty()
        );
        assert!(core.lock().unwrap().issue().is_none());
        release.send(()).unwrap();
        owner.join().unwrap();
    }
}
#[test]
fn mixed_requests_and_late_owner_receipts_cannot_rearm() {
    let core = Mutex::new(Coordinator::default());
    let admission = reduce(&core, Ok(Message::Request(Request::Restart, 0)), 0).unwrap()[0];
    reduce(&core, Ok(Message::Request(Request::System, 0)), 1);
    reduce(&core, Err(RecvTimeoutError::Timeout), 10_000);
    assert_eq!(core.lock().unwrap().issue(), Some(Action::Quit));
    assert!(reduce(
        &core,
        Ok(Message::Finished(Event {
            effect: admission,
            outcome: Outcome::Admission(true)
        })),
        10_001
    )
    .unwrap()
    .is_empty());
    assert!(!core.lock().unwrap().receipt.producers_fenced);
}

#[test]
fn request_delivery_delay_does_not_extend_the_absolute_budget() {
    let core = Mutex::new(Coordinator::default());
    let effects = reduce(&core, Ok(Message::Request(Request::Restart, 0)), 10_000).unwrap();
    assert_eq!(effects.first().unwrap().deadline_ms, 250);
    assert_eq!(effects.last().unwrap().step, Step::Terminate);
    assert!(core.lock().unwrap().receipt.timed_out);
    assert_eq!(core.lock().unwrap().issue(), Some(Action::Quit));
}
