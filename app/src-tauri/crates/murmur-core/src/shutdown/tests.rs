use super::*;
#[test]
fn ordered_drain_and_single_terminal_action() {
    let mut c = Coordinator::default();
    let mut e = c.request(Request::Restart, 100)[0];
    for (step, result) in [
        (Step::Admission, Outcome::Admission(true)),
        (Step::Capture, Outcome::Capture(true)),
        (Step::Helpers, Outcome::Helpers(true)),
        (
            Step::Writers,
            Outcome::Writers(WriteOutcome::PreservedForReplay),
        ),
    ] {
        assert_eq!(e.step, step);
        let previous = e;
        e = c.advance(
            Event {
                effect: e,
                outcome: result,
            },
            101,
        )[0];
        assert!(c
            .advance(
                Event {
                    effect: previous,
                    outcome: result
                },
                102
            )
            .is_empty());
    }
    assert_eq!(e.step, Step::Terminate);
    assert_eq!(c.issue(), Some(Action::Restart));
    assert!(c.issue().is_none());
    assert!(c.request(Request::Quit, 103).is_empty());
}
#[test]
fn absolute_deadlines_and_stalled_owners() {
    let mut c = Coordinator::default();
    let admission = c.request(Request::Restart, 50)[0];
    assert!(c.tick(299).is_empty());
    assert_eq!(c.tick(300)[0].deadline_ms, 5050);
    assert!(c.request(Request::Restart, 5049).is_empty());
    assert_eq!(c.tick(5050)[0].deadline_ms, 7050);
    assert_eq!(c.tick(7050)[0].deadline_ms, 10050);
    assert_eq!(c.tick(10050)[0].step, Step::Terminate);
    assert!(c
        .advance(
            Event {
                effect: admission,
                outcome: Outcome::Admission(true)
            },
            10051
        )
        .is_empty());
    assert_eq!(c.issue(), Some(Action::Quit));
    assert!(c.receipt.timed_out);
}
#[test]
fn quit_and_system_always_suppress_restart_before_commit() {
    for request in [Request::Quit, Request::System] {
        let mut c = Coordinator::default();
        c.request(Request::Restart, 0);
        c.request(request, 1);
        c.receipt = Receipt {
            producers_fenced: true,
            capture_stopped: true,
            helpers_confirmed_empty: true,
            writes: WriteOutcome::Committed,
            timed_out: false,
        };
        assert_eq!(c.action(), Action::Quit);
        c.request(Request::Restart, 2);
        assert_eq!(c.action(), Action::Quit);
    }
}
#[test]
fn restart_requires_every_receipt() {
    let mut c = Coordinator::default();
    c.request(Request::Restart, 0);
    c.receipt = Receipt {
        producers_fenced: true,
        capture_stopped: true,
        helpers_confirmed_empty: true,
        writes: WriteOutcome::Committed,
        timed_out: false,
    };
    assert_eq!(c.action(), Action::Restart);
    c.receipt.helpers_confirmed_empty = false;
    assert_eq!(c.action(), Action::Quit);
    c.receipt.helpers_confirmed_empty = true;
    c.receipt.writes = WriteOutcome::UnpreservedFailure;
    assert_eq!(c.action(), Action::Quit);
    assert_eq!(Coordinator::default().state, State::Running);
}
#[test]
fn wrong_epoch_or_receipt_cannot_advance() {
    let mut c = Coordinator::default();
    let mut e = c.request(Request::Quit, 0)[0];
    assert!(c
        .advance(
            Event {
                effect: e,
                outcome: Outcome::Capture(true)
            },
            0
        )
        .is_empty());
    e.epoch += 1;
    assert!(c
        .advance(
            Event {
                effect: e,
                outcome: Outcome::Admission(true)
            },
            0
        )
        .is_empty());
    assert_eq!(c.state, State::ClosingAdmission);
}

#[test]
fn updater_ticket_is_bound_to_current_restart_and_epoch() {
    let mut original = Coordinator::default();
    original.request(Request::Restart, 0);
    let ticket = original.restart_ticket().unwrap();
    assert!(original.owns_restart_ticket(ticket));
    let mut next = Coordinator::default();
    next.request(Request::Restart, 0);
    assert!(!next.owns_restart_ticket(ticket));
    original.request(Request::Quit, 1);
    assert!(!original.owns_restart_ticket(ticket));
}
