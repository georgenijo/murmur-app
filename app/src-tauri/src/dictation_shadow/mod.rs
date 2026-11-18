//! Dictation shadow mode (Stage 2c, appendix C8): an off-thread comparison of the
//! extracted `DictationCore` decisions against what the live path actually did.
//!
//! **Compiled only with the `dictation-shadow` Cargo feature** (a `#[cfg]` on this
//! module's declaration in `lib.rs`), and **active only when `MURMUR_SHADOW=1`** was
//! set at process start (read once into a `OnceLock<bool>`). Neither alone enables
//! it. A default build contains none of this code: its only footprint is the
//! mismatch event's registry arm and the empty `shadow_site!` marker, both of which
//! are data, not behaviour.
//!
//! What the shadow is not: it never executes an effect and never touches a live
//! field, lock, port or window (it has no port at all: `DictationCore::handle` is
//! pure). It keeps no history copy (H1/H2 live in the shell's `core_history`) and
//! never draws an op id: it replays the ids the live shell recorded.
//!
//! The pieces:
//!
//! * [`types`] the messages (`ShadowHeader`, `ShadowMsg`) and their vocabulary;
//! * [`transport`] the live side: the bounded channel (256), the flight-owned
//!   [`transport::ShadowRecorder`], the header [`transport::ShadowTicket`] and
//!   `witness` / `mark` / `submit`, which use `try_send` only, take no lock and
//!   never block;
//! * [`compare`] the `murmur-dictation-shadow` thread's comparison (CP1-CP4,
//!   `diff_kind` 1-9, cascade control, the PD allowlist);
//! * [`summary`] the counters and `dictation-shadow-summary.json`.
//!
//! The capture sites that feed it (each marked `shadow_site!(issue = 1154)`) arrive
//! in Stage 2c-2; until then nothing in the shell sends a message, and the only
//! callers are the startup hook and the clean-exit summary write in `lib.rs`.
//! Removal is tracked in #1154.

// The capture sites that use most of this arrive in Stage 2c-2.
#![allow(dead_code)]

pub(crate) mod compare;
pub(crate) mod pd;
pub(crate) mod summary;
pub(crate) mod transport;
pub(crate) mod types;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use compare::{Comparator, MismatchEvent};
use transport::{ShadowRecorder, Transport, CHANNEL_CAPACITY};
use types::ShadowMsg;

/// The environment variable that arms the feature at process start.
pub(crate) const ENV_VAR: &str = "MURMUR_SHADOW";

/// The summary is written this often while the app runs, and once more on a clean
/// exit (C8.4 step 5).
pub(crate) const SUMMARY_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Name of the comparison thread (C8.3).
pub(crate) const THREAD_NAME: &str = "murmur-dictation-shadow";

static ENABLED: OnceLock<bool> = OnceLock::new();
static SYNC: AtomicBool = AtomicBool::new(false);

/// `MURMUR_SHADOW=1` exactly. Anything else (unset, `0`, `true`) is off.
pub(crate) fn parse_flag(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether shadow capture is active: the env var read **once** at first use, or the
/// synchronous test mode. The feature gate is the `#[cfg]` that compiles this module
/// at all, so both are required in a real build.
pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| parse_flag(std::env::var(ENV_VAR).ok().as_deref()))
        || SYNC.load(Ordering::Relaxed)
}

struct Global {
    transport: Arc<Transport>,
    receiver: Mutex<Option<Receiver<ShadowMsg>>>,
}

static GLOBAL: OnceLock<Global> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();

fn global() -> &'static Global {
    GLOBAL.get_or_init(|| {
        let (transport, receiver) = Transport::new(CHANNEL_CAPACITY, Comparator::production());
        Global {
            transport,
            receiver: Mutex::new(Some(receiver)),
        }
    })
}

/// The process-wide transport the live path sends on.
pub(crate) fn transport() -> &'static Arc<Transport> {
    &global().transport
}

/// A new flight recorder, to be created **before** `run(Pre)` (C4.1 step 2); `None`
/// unless shadow mode is active, so a disabled run allocates nothing.
pub(crate) fn recorder() -> Option<ShadowRecorder> {
    enabled().then(|| transport().recorder())
}

/// Compare on the calling thread instead of through the channel and the shadow
/// thread: the E7 golden replay needs it fully deterministic (C8.5). It also arms
/// capture without the environment variable, because the replay runs under test.
pub(crate) fn set_sync(on: bool) {
    SYNC.store(on, Ordering::SeqCst);
    transport().set_sync(on);
}

/// A copy of the counters, for the summary and for tests.
pub(crate) fn stats_snapshot() -> summary::Stats {
    let transport = transport();
    match transport.comparator().lock() {
        Ok(mut comparator) => {
            comparator.note_dropped(transport.take_dropped());
            comparator.note_overflow(transport.take_overflow());
            comparator.stats.clone()
        }
        Err(poisoned) => poisoned.into_inner().stats.clone(),
    }
}

/// Spawn the comparison thread, once. A no-op unless shadow mode is active.
pub(crate) fn start() {
    if !enabled() || STARTED.set(()).is_err() {
        return;
    }
    let global = global();
    let Some(receiver) = global.receiver.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    let transport = Arc::clone(&global.transport);
    if let Err(error) = std::thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || run(&transport, &receiver))
    {
        tracing::warn!(target: "system", "dictation shadow thread did not start: {error}");
    }
}

/// The comparison thread: apply each message, and write the summary every five
/// minutes. Spawned once and never joined.
fn run(transport: &Arc<Transport>, receiver: &Receiver<ShadowMsg>) {
    let mut last_write = Instant::now();
    loop {
        let wait = SUMMARY_INTERVAL.saturating_sub(last_write.elapsed());
        match receiver.recv_timeout(wait) {
            Ok(message) => apply(transport, message),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if last_write.elapsed() >= SUMMARY_INTERVAL {
            write_summary(transport);
            last_write = Instant::now();
        }
    }
}

/// Apply one message under the comparator lock the shadow thread holds. The
/// comparator catches the shadow's own panics per header; this catch is the last
/// line so a bug in the comparison can never end the thread.
fn apply(transport: &Transport, message: ShadowMsg) {
    let Ok(mut comparator) = transport.comparator().lock() else {
        return;
    };
    comparator.note_dropped(transport.take_dropped());
    comparator.note_overflow(transport.take_overflow());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        comparator.handle_msg(message);
    }));
    if outcome.is_err() {
        comparator.stats.shadow_panic += 1;
    }
}

fn write_summary(transport: &Transport) {
    let Some(directory) = crate::telemetry::logs_dir() else {
        return;
    };
    let stats = match transport.comparator().lock() {
        Ok(mut comparator) => {
            comparator.note_dropped(transport.take_dropped());
            comparator.note_overflow(transport.take_overflow());
            comparator.stats.clone()
        }
        Err(poisoned) => poisoned.into_inner().stats.clone(),
    };
    if let Err(error) = summary::write_summary(&directory, &stats) {
        tracing::warn!(target: "system", "dictation shadow summary was not written: {error}");
    }
}

/// Write the summary on a clean exit (C8.4 step 5). A no-op unless shadow mode ran.
pub(crate) fn write_summary_on_exit() {
    if enabled() && STARTED.get().is_some() {
        write_summary(transport());
    }
}

/// The registered mismatch event: three numbers, no string, no id (C8.4).
pub(crate) fn emit_mismatch_event(event: MismatchEvent) {
    tracing::warn!(
        target: "pipeline",
        event_code = "pipeline.dictation_shadow_mismatch",
        site_id = u64::from(event.site_id),
        input_kind = u64::from(event.input_kind),
        diff_kind = u64::from(event.diff_kind),
        "dictation shadow mismatch"
    );
}
