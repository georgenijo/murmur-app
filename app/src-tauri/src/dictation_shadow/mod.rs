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
//! The feature needs `panic = "unwind"`: the comparison catches the shadow's own
//! panics so they can never reach the live path. `lib.rs` refuses to compile it
//! under `panic = "abort"`, and a native soak builds with the `shadow-soak` cargo
//! profile (inherits `release`, `panic = "unwind"`).
//!
//! What the shadow is not: it never executes an effect and never touches a live
//! field, lock, port or window (it has no port at all: `DictationCore::handle` is
//! pure). It keeps no history copy (H1/H2 live in the shell's `core_history`) and
//! never draws an op id: it replays the ids the live shell recorded.
//!
//! The pieces:
//!
//! * [`types`] the messages (`ShadowHeader`, `ShadowMsg`) and their vocabulary, all
//!   by-value and heap-free;
//! * [`ring`] the in-crate bounded ring (Vyukov-style, one consumer) whose
//!   `try_push` makes exactly one attempt: no loop, backoff or yield;
//! * [`transport`] the live side: the preallocated ring (256), the loss counters
//!   (global and per site), the flight-owned [`transport::ShadowRecorder`], the
//!   header [`transport::ShadowTicket`] and `witness` / `mark` / `submit`, each a
//!   single `try_push`: no allocation, no free, no lock, no channel, no waker;
//! * [`compare`] the `murmur-dictation-shadow` thread's comparison (CP1-CP4,
//!   `diff_kind` 1-9, the loss epoch check that gives up everything in flight,
//!   cascade control) and [`pd`] the preserved defects it credits;
//! * [`summary`] the counters, `dictation-shadow-summary.json` and the numeric
//!   `dictation-shadow-events.jsonl`, both written by the comparison thread only.
//!
//! The comparison thread polls the queue on a fixed 5 ms sleep, owns the comparator
//! (no lock), and on a clean exit makes a bounded number of drain attempts before it
//! writes the final summary, which reports what the ring still held. The capture sites that feed it (each marked
//! `shadow_site!(issue = 1154)`) arrive in Stage 2c-2; until then nothing in the shell
//! sends a message, and the only callers are the startup hook and the clean-exit hook
//! in `lib.rs`. Removal is tracked in #1154.

// The capture sites that use most of this arrive in Stage 2c-2.
#![allow(dead_code)]

pub(crate) mod compare;
pub(crate) mod pd;
pub(crate) mod ring;
pub(crate) mod summary;
pub(crate) mod transport;
pub(crate) mod types;

#[cfg(test)]
pub(crate) mod alloc_probe;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use compare::{Comparator, EventSink, MismatchEvent};
use summary::{write_summary, EventFile, Stats};
use transport::{ShadowRecorder, Transport, QUEUE_CAPACITY};

/// The environment variable that arms the feature at process start.
pub(crate) const ENV_VAR: &str = "MURMUR_SHADOW";

/// The summary is written this often while the app runs, and once more on a clean
/// exit (C8.4 step 5).
pub(crate) const SUMMARY_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How long the comparison thread sleeps when the queue is empty (D-S2c-1 a). It
/// registers no waker: the producer never wakes it.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The clean-exit hook waits this long for the final drain and summary.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// Drain attempts the comparison thread makes at shutdown before it writes the final
/// summary, whatever the ring still holds (D-S2c-3 e). A producer paused between
/// reserving a slot and publishing it keeps the consumer at that slot; the summary
/// then reports what was left instead of waiting for it.
const SHUTDOWN_DRAIN_ATTEMPTS: u32 = 3;

/// Messages one drain takes, as a multiple of the ring's capacity, before the thread
/// looks at its stop flag again. A ring refilled as fast as it is emptied would
/// otherwise keep a drain, and so the final summary, from ever finishing (Sol round 5,
/// minor 2).
const DRAIN_BUDGET_RINGS: usize = 4;

/// Name of the comparison thread (C8.3).
pub(crate) const THREAD_NAME: &str = "murmur-dictation-shadow";

static ENABLED: OnceLock<bool> = OnceLock::new();
static TRANSPORT: OnceLock<Arc<Transport>> = OnceLock::new();
/// Messages sent by recorders created before the transport was published.
static UNAVAILABLE: AtomicU64 = AtomicU64::new(0);
static STARTED: OnceLock<()> = OnceLock::new();
static CONTROL: Control = Control::new();

/// `MURMUR_SHADOW=1` exactly. Anything else (unset, `0`, `true`) is off.
pub(crate) fn parse_flag(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Whether shadow capture is active: the env var read **once** at first use. The
/// feature gate is the `#[cfg]` that compiles this module at all, so both are
/// required in a real build.
pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| parse_flag(std::env::var(ENV_VAR).ok().as_deref()))
}

/// Allocate and publish the process-wide transport. **Runs on the comparison
/// thread**: the ring and the counters are allocated and initialised there, never on
/// the thread that started the app (D-S2c-3 minor 2).
fn publish_transport() -> Arc<Transport> {
    Arc::clone(TRANSPORT.get_or_init(|| Transport::new(QUEUE_CAPACITY)))
}

/// A new flight recorder, to be created **before** `run(Pre)` (C4.1 step 2); `None`
/// unless shadow mode is active, so a disabled run allocates nothing. Never blocks
/// and never allocates: until the comparison thread has published the transport the
/// recorder is an `Unavailable` one, whose messages are counted and dropped.
pub(crate) fn recorder() -> Option<ShadowRecorder> {
    enabled().then(|| recorder_from(&TRANSPORT, &UNAVAILABLE))
}

/// [`recorder`] over an explicit slot and counter. `OnceLock::get` never waits, even
/// while another thread is still initialising the value.
pub(crate) fn recorder_from(
    slot: &OnceLock<Arc<Transport>>,
    counter: &'static AtomicU64,
) -> ShadowRecorder {
    match slot.get() {
        Some(transport) => transport.recorder(),
        None => ShadowRecorder::unavailable(counter),
    }
}

/// The comparison thread's stop signal and its acknowledgement, atomics only.
pub(crate) struct Control {
    shutdown: AtomicBool,
    finished: AtomicBool,
}

impl Control {
    pub(crate) const fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    /// Ask the thread to drain, write its final summary and stop.
    pub(crate) fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// The thread wrote its final summary.
    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Wait for the final summary, up to `limit`. Returns whether it was written.
    pub(crate) fn wait_finished(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while !self.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

thread_local! {
    /// Set at the start of the comparison thread and never cleared: the app-wide
    /// panic hook reads it to keep a panic the shadow caught off the live event
    /// writer (D-S2c-2 c).
    static COMPARISON_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the calling thread is the comparison thread. The app's panic hook skips
/// its live-writer flush for a panic raised there.
pub(crate) fn on_comparison_thread() -> bool {
    COMPARISON_THREAD
        .try_with(std::cell::Cell::get)
        .unwrap_or(false)
}

/// Spawn the comparison thread, once. A no-op unless shadow mode is active.
///
/// **This only spawns.** Everything that touches the file system (the log directory
/// lookup, creating and truncating the mismatch file, the summary) and the allocation
/// of both rings happen inside the thread, so neither a slow disk nor a slow
/// allocation can delay the caller (app setup): D-S2c-2 c, D-S2c-3 minor 2.
pub(crate) fn start() {
    if !enabled() || STARTED.set(()).is_err() {
        return;
    }
    // A thread that cannot start leaves capture a no-op (the queue just fills and
    // counts drops); nothing is logged, because nothing of the shadow passes
    // through `tracing`.
    let _ = spawn_thread(
        publish_transport,
        &CONTROL,
        POLL_INTERVAL,
        &UNAVAILABLE,
        crate::telemetry::logs_dir,
    );
}

/// Spawn the comparison thread. `init` runs **on that thread**, first: it builds
/// and publishes the transport. `directory` runs next, on that thread too, and names
/// where the mismatch file and the summary go; the spawn itself neither allocates
/// the rings nor touches the file system.
pub(crate) fn spawn_thread(
    init: impl FnOnce() -> Arc<Transport> + Send + 'static,
    control: &'static Control,
    poll: Duration,
    unavailable: &'static AtomicU64,
    directory: impl FnOnce() -> Option<PathBuf> + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            COMPARISON_THREAD.with(|flag| flag.set(true));
            let transport = init();
            let directory = directory();
            let sink = file_sink(directory.as_deref());
            let mut comparator = Comparator::production(sink);
            comparator.count_unavailable_from(unavailable);
            run(&transport, comparator, directory.as_deref(), control, poll);
        })
}

/// The numeric, file-only sink of the mismatch event (D-S2c-1 b): it is written by
/// the comparison thread and never passes through `tracing`.
pub(crate) fn file_sink(directory: Option<&Path>) -> EventSink {
    let file = directory.and_then(|dir| EventFile::begin(dir).ok());
    Box::new(move |event: MismatchEvent| {
        if let Some(file) = &file {
            file.append(event.site_id, event.input_kind, event.diff_kind);
        }
    })
}

/// The comparison thread: poll the queue, apply each message, write the summary
/// every five minutes, and on a stop request drain (a bounded number of attempts)
/// and write the final summary (D-S2c-1 f, D-S2c-3 e). It is the only writer of the
/// summary.
pub(crate) fn run(
    transport: &Transport,
    mut comparator: Comparator,
    directory: Option<&Path>,
    control: &Control,
    poll: Duration,
) {
    let mut last_write = Instant::now();
    loop {
        // Read the stop flag *before* draining: every message pushed before the
        // request was made is then in the queue this pass drains.
        let stop = control.shutdown.load(Ordering::Acquire);
        let budget = transport.capacity() * DRAIN_BUDGET_RINGS;
        let progressed = comparator.drain_up_to(transport, budget);
        if stop {
            // Bounded: a slot a producer reserved and never published stops the
            // consumer at it, and the final summary must still be written.
            for _ in 1..SHUTDOWN_DRAIN_ATTEMPTS {
                if transport.unconsumed() == 0 {
                    break;
                }
                std::thread::sleep(poll);
                comparator.drain_up_to(transport, budget);
            }
            comparator.stats.unconsumed = transport.unconsumed() as u64;
            comparator.stats.reserved_unpublished = transport.reserved_unpublished() as u64;
            write_stats(directory, &comparator.stats);
            control.finished.store(true, Ordering::Release);
            return;
        }
        if last_write.elapsed() >= SUMMARY_INTERVAL {
            write_stats(directory, &comparator.stats);
            last_write = Instant::now();
        }
        if !progressed {
            std::thread::sleep(poll);
        }
    }
}

fn write_stats(directory: Option<&Path>, stats: &Stats) {
    let Some(directory) = directory else {
        return;
    };
    // A failed write keeps the previous summary (the rename never happened); the
    // next write retries. Nothing is logged: nothing of the shadow passes through
    // `tracing`.
    let _ = write_summary(directory, stats);
}

/// On a clean exit (C8.4 step 5): tell the comparison thread to drain and write the
/// final summary, and wait for it. A no-op unless shadow mode ran. The exit hook
/// writes nothing itself: the thread is the only writer.
pub(crate) fn finish_on_exit() {
    if enabled() && STARTED.get().is_some() {
        CONTROL.request_shutdown();
        CONTROL.wait_finished(EXIT_WAIT);
    }
}
