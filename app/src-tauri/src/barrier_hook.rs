//! A test-only barrier: a named point inside a production call that a test can
//! stand on to run something while that call is in flight (compiled out of every
//! non-test build).
//!
//! Stage 2a-1 uses it to advance the recording generation *during* the
//! mutex-taking operations the supervisor-lifecycle arms run before they read the
//! current id (`DictationTelemetry::accepted`, `DictationDiagnostics::claim`,
//! `PerformanceMetrics::production_capture_failure`). The points are one line each
//! at the top of those functions, so the same barrier tests run unchanged against
//! the pre-refactor code (the oracle).
//!
//! Thread-local: the lifecycle bridge runs on the test's own thread.
#![cfg(test)]

use std::cell::RefCell;

type Hook = Box<dyn FnMut(&'static str)>;

thread_local! {
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

/// Called at a barrier point. Does nothing when no test installed a hook. The hook
/// is taken out while it runs, so a nested barrier point does not recurse.
pub(crate) fn at(point: &'static str) {
    let Some(mut hook) = HOOK.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    hook(point);
    HOOK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(hook);
        }
    });
}

/// Removes the installed hook when dropped (including by a failing assertion).
pub(crate) struct Installed(());

/// Install `hook` for the current thread until the returned guard drops.
pub(crate) fn install(hook: impl FnMut(&'static str) + 'static) -> Installed {
    HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    Installed(())
}

impl Drop for Installed {
    fn drop(&mut self) {
        HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}
