//! Failure bounds for the golden harness's helper threads (a parked stop, cancel or
//! delivery). A scenario orders by barriers, never by a clock (G3,
//! `golden_scenarios_never_wait_on_a_clock`); this file is not a scenario and the bound
//! below orders nothing. It exists so a helper that panics before its barrier (a
//! persisted-core assertion, say) or never finishes fails the test within
//! [`HELPER_BOUND`] instead of blocking until the CI runner kills the process.
#![cfg(test)]

/// How long a helper thread may take to reach its barrier or to finish. A scenario
/// that exceeds it fails, instead of blocking until CI kills the process.
const HELPER_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

/// Wait for a helper thread to signal that it reached its barrier. A helper that
/// finishes first (its panic, or a return that skipped the barrier) or a wait past
/// [`HELPER_BOUND`] fails the scenario at once, and a helper's panic is re-raised on
/// the scenario's thread. The arrival sender lives in a hook the world keeps, so a
/// helper that panics before the barrier never disconnects the channel: the finished
/// thread is what is checked.
pub(crate) fn await_arrival<T>(
    entered: &std::sync::mpsc::Receiver<()>,
    thread: &std::thread::JoinHandle<T>,
    what: &str,
) -> Result<(), HelperEnded> {
    let deadline = std::time::Instant::now() + HELPER_BOUND;
    loop {
        match entered.recv_timeout(std::time::Duration::from_millis(5)) {
            Ok(()) => return Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(HelperEnded),
        }
        if thread.is_finished() {
            // A signal sent just before it finished still counts.
            return entered.try_recv().map_err(|_| HelperEnded);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: the helper did not reach its barrier within {HELPER_BOUND:?}"
        );
    }
}

/// A helper thread ended before it reached its barrier.
pub(crate) struct HelperEnded;

/// Fail the scenario because `thread` ended without arriving: re-raise its panic, or
/// name the early return.
pub(crate) fn helper_failed<T>(thread: std::thread::JoinHandle<T>, what: &str) -> ! {
    match thread.join() {
        Err(payload) => std::panic::resume_unwind(payload),
        Ok(_) => panic!("{what}: the helper returned without reaching its barrier"),
    }
}

/// Join a helper that has been released, bounded by [`HELPER_BOUND`]; a helper's
/// panic is re-raised here.
pub(crate) fn join_bounded<T>(thread: std::thread::JoinHandle<T>, what: &str) -> T {
    let deadline = std::time::Instant::now() + HELPER_BOUND;
    let mut spins = 0_u32;
    while !thread.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: the helper did not finish within {HELPER_BOUND:?}"
        );
        // A helper that is about to finish is joined without a sleep's latency.
        if spins < 2_000 {
            spins += 1;
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    match thread.join() {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Park a helper thread until the scenario releases it (a message, or the scenario
/// dropping the sender), for at most [`HELPER_BOUND`]. A helper that holds a lock for
/// the scenario (the delivery fence D, a parked cancel's T) is thereby never parked for
/// ever when the scenario fails before it releases the helper: past the bound it runs
/// on and lets the lock go.
pub(crate) fn await_release(release: &std::sync::mpsc::Receiver<()>) {
    let _ = release.recv_timeout(HELPER_BOUND);
}

/// Spin while `keep_waiting` holds, failing the scenario after [`HELPER_BOUND`]. For a
/// wait on a lock state that a regression in lock order would never reach.
pub(crate) fn spin_bounded(what: &str, mut keep_waiting: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + HELPER_BOUND;
    while keep_waiting() {
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: the lock state was not reached within {HELPER_BOUND:?}"
        );
        std::thread::yield_now();
    }
}

#[test]
fn a_helper_that_panics_before_its_barrier_fails_the_wait_at_once() {
    // The arrival sender stays alive, as it does in the hook the world keeps, so the
    // channel never disconnects: only the finished thread can end the wait.
    let (entered, arrived) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(|| -> () { panic!("a persisted-core assertion") });
    assert!(await_arrival(&arrived, &thread, "probe").is_err());
    let raised = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        helper_failed(thread, "probe")
    }))
    .expect_err("the helper's panic is re-raised on the waiting thread");
    assert_eq!(
        raised.downcast_ref::<&str>().copied(),
        Some("a persisted-core assertion")
    );
    drop(entered);
}

#[test]
fn a_helper_that_signals_then_finishes_still_arrives() {
    let (entered, arrived) = std::sync::mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        let _ = entered.send(());
        7
    });
    assert!(await_arrival(&arrived, &thread, "probe").is_ok());
    assert_eq!(join_bounded(thread, "probe"), 7);
}

#[test]
fn join_bounded_re_raises_a_helpers_panic() {
    let thread = std::thread::spawn(|| -> () { panic!("late failure") });
    let raised = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        join_bounded(thread, "probe")
    }))
    .expect_err("re-raised");
    assert_eq!(raised.downcast_ref::<&str>().copied(), Some("late failure"));
}
