//! Content-free diagnostics for what occupies the macOS main thread.
//!
//! Slow pastes spend their time waiting for the main thread, not pasting.
//! This module records, with static labels only:
//!
//! - every job this app queues with `run_on_main_thread` (queue wait and run
//!   time), through [`job`];
//! - every Tauri command handled on the main thread, by command name;
//! - main-thread callbacks and nested native calls (window show/hide/resize,
//!   notification blocks, window and run events), through [`span`];
//! - a run-loop observer that measures every busy span (time the main thread
//!   is not waiting in a run loop), so AppKit, WebKit, Core Animation, and
//!   event-loop drains the app did not label are still visible.
//!
//! The main thread only pushes fixed-size records into a preallocated ring.
//! A background reporter thread does every allocation and every log write: it
//! logs spans over [`LOG_SPAN_MIN_US`], and joins each dictation paste's
//! main-thread wait to the spans that overlapped it.
//!
//! Three further diagnostics share the reporter:
//!
//! - **Stack samples.** When one run-loop busy stretch passes
//!   [`STACK_FIRST_SAMPLE_US`], the reporter reads the main thread's stack
//!   (see [`stack`]) and logs symbol names only.
//! - **Rates.** Lock-free counters for `audio-level` emits and IPC invokes,
//!   logged once per second of activity.
//! - **Applied markers.** tao applies window geometry as queued main-queue
//!   blocks, so a [`section`] around the setter times only the enqueue.
//!   [`queued_section`] also queues a marker behind the setter's block and
//!   records how long the block took to come up.

use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A standalone span at least this long is always logged.
pub(crate) const LOG_SPAN_MIN_US: u64 = 10_000;
/// A queued job that waited at least this long is always logged.
pub(crate) const LOG_QUEUE_MIN_US: u64 = 20_000;
/// A paste whose main-thread wait reaches this gets every overlapping span.
pub(crate) const SLOW_PASTE_US: u64 = 20_000;
/// Run-loop busy spans and segments shorter than this are not recorded.
const RUNLOOP_RECORD_MIN_US: u64 = 1_000;
/// Callbacks and nested sections shorter than this are not recorded.
const SPAN_RECORD_MIN_US: u64 = 100;
const RING_CAPACITY: usize = 4096;
const HISTORY_RETAIN_US: u64 = 20_000_000;
const REPORT_DELAY: Duration = Duration::from_millis(1_200);
/// Ring drain, history upkeep and paste reports run this often.
const DRAIN_INTERVAL_US: u64 = 200_000;
/// A busy stretch this long gets its first stack sample.
pub(crate) const STACK_FIRST_SAMPLE_US: u64 = 50_000;
/// A stretch still busy this long gets one more sample.
pub(crate) const STACK_SECOND_SAMPLE_US: u64 = 250_000;
/// While a stretch is being watched the reporter wakes at least this often,
/// so a stretch that ends and is replaced is noticed within one interval.
const WATCH_POLL_MAX_US: u64 = STACK_FIRST_SAMPLE_US;
/// The shortest sleep the reporter takes.
const WATCH_POLL_MIN_US: u64 = 1_000;
/// Stack events allowed per [`STACK_WINDOW_US`]; the rest are counted.
const MAX_STACKS_PER_WINDOW: u32 = 8;
/// One capture may hold the main thread this long (it normally takes a few
/// microseconds). Past it, sampling turns itself off for the process.
pub(crate) const STACK_MAX_HOLD_US: u64 = 2_000;
const STACK_WINDOW_US: u64 = 60_000_000;
/// Marker blocks that came up faster than this are not recorded.
const APPLIED_RECORD_MIN_US: u64 = 1_000;
/// A rate bucket is logged when it holds audio-level emits, or at most this
/// often when only IPC invokes ran at [`RATE_IDLE_MIN_INVOKES`] or more.
const RATE_IDLE_LOG_US: u64 = 10_000_000;
const RATE_IDLE_MIN_INVOKES: u64 = 10;
const RATE_BUCKET_US: u64 = 1_000_000;
const MAX_PASTE_SPANS: usize = 48;
/// Standalone span events allowed per [`STANDALONE_WINDOW_US`].
const MAX_STANDALONE_PER_WINDOW: u32 = 30;
const STANDALONE_WINDOW_US: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A closure queued with `run_on_main_thread`.
    Job,
    /// A Tauri command executed by the invoke handler on the main thread.
    Command,
    /// A main-thread callback the app registered (notification, event).
    Callback,
    /// A nested native call inside other work, e.g. a window hide.
    Section,
    /// The main thread was not waiting in a run loop.
    RunloopBusy,
    /// One contiguous phase of a busy span, between observer callbacks.
    RunloopSegment,
}

impl Kind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Job => "job",
            Self::Command => "command",
            Self::Callback => "callback",
            Self::Section => "section",
            Self::RunloopBusy => "runloop_busy",
            Self::RunloopSegment => "runloop_segment",
        }
    }

    const fn is_labelled_app_work(self) -> bool {
        matches!(
            self,
            Self::Job | Self::Command | Self::Callback | Self::Section
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct Span {
    kind: Kind,
    label: &'static str,
    start_us: u64,
    duration_us: u64,
    queue_us: u64,
    on_main: bool,
    /// Busy spans: the longest segment's phase label and duration.
    longest_phase: &'static str,
    longest_phase_us: u64,
    segments: u32,
}

impl Span {
    const EMPTY: Span = Span {
        kind: Kind::Section,
        label: "",
        start_us: 0,
        duration_us: 0,
        queue_us: 0,
        on_main: false,
        longest_phase: "",
        longest_phase_us: 0,
        segments: 0,
    };

    fn end_us(&self) -> u64 {
        self.start_us.saturating_add(self.duration_us)
    }
}

/// Single-producer, single-consumer ring: the main thread is the only
/// producer and the reporter thread the only consumer. The producer never
/// blocks or allocates: a full ring drops the record and counts it.
struct Ring {
    slots: Box<[UnsafeCell<Span>]>,
    /// Next slot the producer writes; only the producer stores it.
    tail: AtomicUsize,
    /// Next slot the consumer reads; only the consumer stores it.
    head: AtomicUsize,
    dropped: AtomicU64,
}

// SAFETY: a slot is written only by the single producer while it lies
// outside `head..tail`, and read only by the single consumer once it lies
// inside; the Release/Acquire pairs on `tail` and `head` order those accesses.
unsafe impl Sync for Ring {}

impl Ring {
    fn new() -> Self {
        Self {
            slots: (0..RING_CAPACITY)
                .map(|_| UnsafeCell::new(Span::EMPTY))
                .collect(),
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Producer side. Wait-free; returns `false` when the record was dropped.
    fn push(&self, span: Span) -> bool {
        let capacity = self.slots.len();
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= capacity {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // SAFETY: see `impl Sync for Ring`; this slot is outside head..tail.
        unsafe { *self.slots[tail % capacity].get() = span };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    /// Consumer side. Returns the number of records dropped since last drain.
    fn drain_into(&self, out: &mut Vec<Span>) -> u64 {
        let capacity = self.slots.len();
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        while head != tail {
            // SAFETY: see `impl Sync for Ring`; this slot is inside head..tail.
            out.push(unsafe { *self.slots[head % capacity].get() });
            head = head.wrapping_add(1);
        }
        self.head.store(tail, Ordering::Release);
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<(Instant, u64)> = OnceLock::new();
static RING: OnceLock<Ring> = OnceLock::new();
static PENDING_PASTES: Mutex<Vec<Arc<PasteState>>> = Mutex::new(Vec::new());
/// Start of the run-loop busy span in progress (µs since epoch), or 0 while
/// the main thread is waiting. Read by the reporter for an unclosed span.
static BUSY_SINCE_US: AtomicU64 = AtomicU64::new(0);
/// Lock-free per-second counters. Any thread adds; only the reporter swaps.
static AUDIO_LEVEL_EMITS: AtomicU64 = AtomicU64::new(0);
static IPC_INVOKES: AtomicU64 = AtomicU64::new(0);
static IPC_MAIN_INVOKES: AtomicU64 = AtomicU64::new(0);
static IPC_MAIN_US: AtomicU64 = AtomicU64::new(0);
/// The reporter's handle, so the run-loop observer can wake it.
static REPORTER_THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
/// True only while the reporter is parked for a long idle sleep.
static REPORTER_PARKED: AtomicBool = AtomicBool::new(false);
/// The main thread's stack, captured at install for the stack sampler.
static STACK_TARGET: OnceLock<stack::Target> = OnceLock::new();
/// One recent live-preview hide. Written only on the main thread, read by the
/// reporter; `id` is cleared while `at` changes so a reader never pairs an id
/// with another hide's time.
struct HideSlot {
    id: AtomicU64,
    at: AtomicU64,
}

impl HideSlot {
    const fn new() -> Self {
        Self {
            id: AtomicU64::new(0),
            at: AtomicU64::new(0),
        }
    }
}

const PREVIEW_HIDE_SLOTS: usize = 8;
/// The last few live-preview hides that ran.
static PREVIEW_HIDES: [HideSlot; PREVIEW_HIDE_SLOTS] =
    [const { HideSlot::new() }; PREVIEW_HIDE_SLOTS];
static PREVIEW_HIDE_NEXT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static IS_MAIN: Cell<bool> = const { Cell::new(false) };
}

fn epoch() -> &'static (Instant, u64) {
    EPOCH.get_or_init(|| {
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        (Instant::now(), unix_ms)
    })
}

/// Microseconds since this process's diagnostic epoch. Never 0 after install.
pub(crate) fn now_us() -> u64 {
    (epoch().0.elapsed().as_micros() as u64).max(1)
}

fn unix_ms_at(us: u64) -> u64 {
    epoch().1.saturating_add(us / 1_000)
}

fn is_main() -> bool {
    IS_MAIN.with(Cell::get)
}

/// Main thread only: the ring has a single producer. Never blocks.
fn record(span: Span) {
    if !ENABLED.load(Ordering::Relaxed) || !is_main() {
        return;
    }
    if let Some(ring) = RING.get() {
        ring.push(span);
    }
}

fn record_simple(kind: Kind, label: &'static str, start_us: u64, end_us: u64, queue_us: u64) {
    record(Span {
        kind,
        label,
        start_us,
        duration_us: end_us.saturating_sub(start_us),
        queue_us,
        on_main: is_main(),
        ..Span::EMPTY
    });
}

/// Wrap a closure passed to `run_on_main_thread` so its queue wait and run
/// time are recorded under `label`. Behavior is otherwise unchanged.
pub(crate) fn job<F>(label: &'static str, f: F) -> impl FnOnce() + Send + 'static
where
    F: FnOnce() + Send + 'static,
{
    let queued_us = if ENABLED.load(Ordering::Relaxed) {
        now_us()
    } else {
        0
    };
    move || {
        if queued_us == 0 {
            f();
            return;
        }
        let start = now_us();
        f();
        let end = now_us();
        record_simple(
            Kind::Job,
            label,
            start,
            end,
            start.saturating_sub(queued_us),
        );
    }
}

/// Time a main-thread callback or a nested native call. Only main-thread
/// calls of at least [`SPAN_RECORD_MIN_US`] are recorded.
pub(crate) fn span<T>(kind: Kind, label: &'static str, f: impl FnOnce() -> T) -> T {
    if !ENABLED.load(Ordering::Relaxed) || !is_main() {
        return f();
    }
    let start = now_us();
    let result = f();
    let end = now_us();
    if end.saturating_sub(start) >= SPAN_RECORD_MIN_US {
        record_simple(kind, label, start, end, 0);
    }
    result
}

/// Time a nested native call (window show/hide/resize and similar).
pub(crate) fn section<T>(label: &'static str, f: impl FnOnce() -> T) -> T {
    span(Kind::Section, label, f)
}

/// Time a nested native call whose work tao applies later, as a queued
/// main-queue block (`set_size`, `set_position`, `set_ignore_cursor_events`).
/// `label` times the call, which is only the enqueue. `applied_label` names a
/// marker block queued right behind it: when that block runs, the work queued
/// ahead of it, tao's block included, has finished, and the span records the
/// enqueue-to-applied latency in `queue_us`. That latency is an upper bound
/// for the block itself, because other queued work can sit in front of it.
///
/// The marker is queued only when the setter returned `Ok`: a failed call
/// queued no block, so there is nothing to wait behind.
pub(crate) fn queued_section<T, E>(
    label: &'static str,
    applied_label: &'static str,
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let result = section(label, f);
    if result.is_ok() {
        applied::queue_marker(applied_label);
    }
    result
}

/// Main thread, at the start of a busy stretch: wake the reporter if (and
/// only if) it is parked for a long idle sleep, so it can arm its 50 ms
/// check. A relaxed-cost load otherwise. Never blocks, locks, or allocates;
/// `Thread::unpark` is an atomic swap, plus a semaphore signal when the
/// target is parked.
///
/// The "no lock on the main thread" property relies on std's Darwin parker
/// being dispatch-semaphore based (current std): `unpark` is then an atomic
/// swap and, only when the target is parked, a `dispatch_semaphore_signal`.
/// Older std built the parker on a mutex and condvar, which would take a lock
/// here. Recheck this when the toolchain changes.
fn wake_reporter_if_parked() {
    if REPORTER_PARKED.load(Ordering::SeqCst) && REPORTER_PARKED.swap(false, Ordering::SeqCst) {
        if let Some(thread) = REPORTER_THREAD.get() {
            thread.unpark();
        }
    }
}

/// Count one `audio-level` event emitted to the webviews.
pub(crate) fn count_audio_level_emit() {
    AUDIO_LEVEL_EMITS.fetch_add(1, Ordering::Relaxed);
}

/// Time a main-thread callback the app registered.
pub(crate) fn callback<T>(label: &'static str, f: impl FnOnce() -> T) -> T {
    span(Kind::Callback, label, f)
}

/// Static label for a window event callback.
pub(crate) fn window_event_label(event: &tauri::WindowEvent) -> &'static str {
    match event {
        tauri::WindowEvent::Resized(_) => "tauri.window_event.resized",
        tauri::WindowEvent::Moved(_) => "tauri.window_event.moved",
        tauri::WindowEvent::CloseRequested { .. } => "tauri.window_event.close_requested",
        tauri::WindowEvent::Destroyed => "tauri.window_event.destroyed",
        tauri::WindowEvent::Focused(_) => "tauri.window_event.focused",
        tauri::WindowEvent::ScaleFactorChanged { .. } => "tauri.window_event.scale_factor",
        tauri::WindowEvent::ThemeChanged(_) => "tauri.window_event.theme_changed",
        _ => "tauri.window_event.other",
    }
}

/// Static label for an app run-event callback.
pub(crate) fn run_event_label(event: &tauri::RunEvent) -> &'static str {
    match event {
        tauri::RunEvent::Exit => "tauri.run_event.exit",
        tauri::RunEvent::ExitRequested { .. } => "tauri.run_event.exit_requested",
        tauri::RunEvent::WindowEvent { .. } => "tauri.run_event.window_event",
        tauri::RunEvent::WebviewEvent { .. } => "tauri.run_event.webview_event",
        tauri::RunEvent::Ready => "tauri.run_event.ready",
        tauri::RunEvent::Resumed => "tauri.run_event.resumed",
        tauri::RunEvent::MainEventsCleared => "tauri.run_event.main_events_cleared",
        tauri::RunEvent::MenuEvent(_) => "tauri.run_event.menu_event",
        tauri::RunEvent::TrayIconEvent(_) => "tauri.run_event.tray_icon_event",
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => "tauri.run_event.reopen",
        _ => "tauri.run_event.other",
    }
}

const MAX_LABEL_LEN: usize = 64;

/// Every command in `lib.rs`'s `generate_handler!`, sorted, as static labels.
/// A test keeps this in step with `lib.rs`, so instrumentation never interns
/// or allocates a name on the main thread.
const COMMAND_LABELS: &[&str] = &[
    "apply_meeting_calendar_event",
    "arm_next_dictation_diagnostic_capture",
    "cancel_audio_initialization",
    "cancel_benchmark",
    "cancel_code_vocab_scan",
    "cancel_microphone_preview",
    "cancel_microphone_startup_benchmark",
    "cancel_model_download",
    "cancel_native_recording",
    "check_accessibility_permission",
    "check_microphone_permission",
    "check_microphone_permission_status",
    "check_model_exists",
    "check_smart_turn",
    "check_specific_model_exists",
    "clear_event_history",
    "clear_history_blob",
    "clear_ide_context",
    "clear_logs",
    "clear_performance_diagnostics",
    "clear_stats_blob",
    "clear_theme_library_blob",
    "configure_dictation",
    "configure_meeting_suggestions",
    "configure_smart_auto_probe",
    "delete_all_meetings",
    "delete_dictation_diagnostic_capture",
    "delete_meeting",
    "disarm_next_dictation_diagnostic_capture",
    "dismiss_meeting_suggestion",
    "download_model",
    "download_smart_turn_model",
    "get_app_disabled",
    "get_audio_input_inventory",
    "get_benchmark_activity",
    "get_benchmark_models",
    "get_calendar_permission_status",
    "get_capture_health_history",
    "get_diarization_model_status",
    "get_dictation_diagnostic_capture",
    "get_dictation_diagnostic_capture_status",
    "get_event_history",
    "get_ide_context_status",
    "get_log_contents",
    "get_meeting",
    "get_meeting_audio_capture_busy",
    "get_meeting_audio_manifest",
    "get_meeting_bundle_preview",
    "get_meeting_calendar_events",
    "get_meeting_review_export",
    "get_meeting_status",
    "get_meeting_store_status",
    "get_meeting_suggestion",
    "get_microphone_preview_status",
    "get_model_hardware_guidance",
    "get_model_runtime_catalog",
    "get_model_runtime_status",
    "get_overlay_geometry",
    "get_performance_resource_window",
    "get_performance_run",
    "get_performance_store_health",
    "get_resource_usage",
    "get_smart_auto_microphone_status",
    "get_status",
    "get_system_audio_permission_status",
    "get_update_install_environment",
    "hide_overlay",
    "init_dictation",
    "is_notchpill_installed",
    "list_audio_devices",
    "list_dictation_diagnostic_captures",
    "list_meetings",
    "list_performance_runs",
    "list_running_applications",
    "load_history_blob",
    "load_settings_blob",
    "load_stats_blob",
    "load_theme_library_blob",
    "log_frontend",
    "open_benchmark_output_folder",
    "open_calendar_preferences",
    "open_system_audio_preferences",
    "open_system_preferences",
    "patch_overlay_setting",
    "preview_vocabulary_aliases",
    "process_audio",
    "prune_meetings",
    "read_meeting_audio_range",
    "read_theme_file",
    "recover_performance_store",
    "refresh_ide_context",
    "remove_diarization_model",
    "remove_model",
    "remove_smart_turn_model",
    "rename_meeting_remote_speaker",
    "request_accessibility_permission",
    "request_calendar_permission",
    "request_microphone_access",
    "request_microphone_permission",
    "request_system_audio_permission",
    "reset_accessibility_permission",
    "reset_calendar_permission",
    "reset_microphone_permission",
    "retry_last_delivery",
    "retry_smart_auto_probe",
    "run_benchmark",
    "run_microphone_startup_benchmark",
    "save_benchmark_report",
    "save_history_blob",
    "save_meeting_bundle_export",
    "save_meeting_metadata",
    "save_meeting_review",
    "save_meeting_review_export",
    "save_microphone_startup_benchmark_report",
    "save_settings_blob",
    "save_stats_blob",
    "save_text_export",
    "save_theme_library_blob",
    "scan_code_vocab",
    "set_app_disabled",
    "set_fn_capture_input",
    "set_keyboard_recording",
    "set_live_words_material",
    "set_overlay_expanded",
    "set_overlay_material",
    "set_overlay_vertical_offset",
    "set_paste_last_shortcut",
    "set_tray_update_available",
    "show_diagnostics_window",
    "show_dictation_preview",
    "show_main_window",
    "show_overlay",
    "smart_turn_model_status",
    "start_keyboard_listener",
    "start_meeting",
    "start_microphone_preview",
    "start_native_recording",
    "stop_keyboard_listener",
    "stop_meeting",
    "stop_microphone_preview",
    "stop_native_recording",
    "sync_window_chrome",
    "transcribe_file",
    "update_keyboard_key",
    "update_microphone_preview_vad_sensitivity",
    "update_tray_icon",
    "updater_canary",
    "upload_dictation_diagnostic_capture",
    "verify_microphone_preview_signal",
    "write_theme_file",
];

/// The static label for a registered command name; no allocation or copy.
///
/// Tauri routes `plugin:<name>|<command>` straight to the plugin
/// (`extend_api`), never through the app's invoke handler. The webview's
/// `plugin:window|outer_position` and `plugin:window|cursor_position` calls
/// are therefore invisible here: they run as async commands and ask the main
/// thread for the answer through tao's event-loop drain. The stack sampler
/// names that work instead.
fn command_label(name: &str) -> &'static str {
    COMMAND_LABELS
        .binary_search(&name)
        .map(|index| COMMAND_LABELS[index])
        .unwrap_or("command.other")
}

/// Record the run time of every Tauri command the invoke handler executes on
/// the main thread, labelled from the static [`COMMAND_LABELS`] table.
pub(crate) fn instrument_invoke_handler<R, H>(
    handler: H,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
where
    R: tauri::Runtime,
    H: Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke| {
        if !ENABLED.load(Ordering::Relaxed) {
            return handler(invoke);
        }
        IPC_INVOKES.fetch_add(1, Ordering::Relaxed);
        if !is_main() {
            return handler(invoke);
        }
        IPC_MAIN_INVOKES.fetch_add(1, Ordering::Relaxed);
        let label = command_label(invoke.message.command());
        let start = now_us();
        let handled = handler(invoke);
        let end = now_us();
        IPC_MAIN_US.fetch_add(end.saturating_sub(start), Ordering::Relaxed);
        record_simple(Kind::Command, label, start, end, 0);
        handled
    }
}

/// Labels are static identifiers: lowercase ASCII, digits, `_`, `.`.
pub(crate) fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Record that the live-preview hide for `recording_id` started now. Main
/// thread only (the single writer); lock-free, overwrites the oldest slot.
pub(crate) fn note_preview_hide(recording_id: u64) {
    if !ENABLED.load(Ordering::Relaxed) || !is_main() || recording_id == 0 {
        return;
    }
    let start = now_us();
    let index = PREVIEW_HIDE_NEXT.fetch_add(1, Ordering::Relaxed) % PREVIEW_HIDE_SLOTS;
    let slot = &PREVIEW_HIDES[index];
    slot.id.store(0, Ordering::SeqCst);
    slot.at.store(start, Ordering::SeqCst);
    slot.id.store(recording_id, Ordering::SeqCst);
}

/// The latest recorded hide start for `recording_id`, if any.
fn latest_preview_hide(recording_id: u64) -> Option<u64> {
    PREVIEW_HIDES
        .iter()
        .filter_map(|slot| {
            let before = slot.id.load(Ordering::SeqCst);
            let at = slot.at.load(Ordering::SeqCst);
            let after = slot.id.load(Ordering::SeqCst);
            (before == recording_id && after == recording_id).then_some(at)
        })
        .max()
}

/// Where the preview hide for this paste ran: before the paste was queued,
/// while it waited, after it started, or not at all.
fn preview_hide_order(recording_id: u64, queued_us: u64, started_us: u64) -> &'static str {
    classify_preview_hide(latest_preview_hide(recording_id), queued_us, started_us)
}

fn classify_preview_hide(hide_us: Option<u64>, queued_us: u64, started_us: u64) -> &'static str {
    match hide_us {
        None => "none",
        Some(at) if at < queued_us => "before_paste",
        Some(at) if started_us == 0 || at < started_us => "during_wait",
        Some(_) => "after_paste",
    }
}

// ---------------------------------------------------------------------------
// Paste wait join
// ---------------------------------------------------------------------------

struct PasteState {
    recording_id: u64,
    queued_us: u64,
    started_us: AtomicU64,
    finished_us: AtomicU64,
    submitted_us: AtomicU64,
}

/// Follows one paste job from dispatch to completion. Dropping the owner
/// submits it to the reporter, which logs the wait once the spans that
/// overlapped it have closed.
pub(crate) struct PasteProbe {
    state: Arc<PasteState>,
    owner: bool,
}

impl PasteProbe {
    /// Create right before `run_on_main_thread` dispatches the paste.
    pub(crate) fn new(recording_id: u64) -> Self {
        Self {
            state: Arc::new(PasteState {
                recording_id,
                queued_us: now_us(),
                started_us: AtomicU64::new(0),
                finished_us: AtomicU64::new(0),
                submitted_us: AtomicU64::new(0),
            }),
            owner: true,
        }
    }

    /// A handle for the main-thread closure; it never submits.
    pub(crate) fn worker(&self) -> Self {
        Self {
            state: self.state.clone(),
            owner: false,
        }
    }

    pub(crate) fn started(&self) {
        self.state.started_us.store(now_us(), Ordering::Release);
    }

    pub(crate) fn finished(&self) {
        self.state.finished_us.store(now_us(), Ordering::Release);
    }
}

impl Drop for PasteProbe {
    fn drop(&mut self) {
        if !self.owner || !ENABLED.load(Ordering::Relaxed) || self.state.recording_id == 0 {
            return;
        }
        self.state.submitted_us.store(now_us(), Ordering::Release);
        PENDING_PASTES
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(self.state.clone());
    }
}

// ---------------------------------------------------------------------------
// Install and reporter
// ---------------------------------------------------------------------------

/// Install the observers and start the reporter. Must run on the main thread,
/// after telemetry and after Tauri created its event loop.
pub(crate) fn install() {
    if RING.get().is_some() {
        return;
    }
    let _ = epoch();
    IS_MAIN.with(|flag| flag.set(true));
    let _ = RING.set(Ring::new());
    #[cfg(target_os = "macos")]
    let observers_installed = runloop::install();
    #[cfg(not(target_os = "macos"))]
    let observers_installed = false;
    // The calling thread is the main thread: remember its stack for sampling.
    let stack_sampling = observers_installed
        && !stacks_disabled_by_env(std::env::var_os(DISABLE_STACKS_ENV).as_deref())
        && stack::Target::current().is_some_and(|target| STACK_TARGET.set(target).is_ok());
    ENABLED.store(true, Ordering::Release);
    let spawned = std::thread::Builder::new()
        .name("main-thread-watch".into())
        .spawn(reporter_loop)
        .is_ok();
    tracing::info!(
        target: "system",
        event_code = "system.main_thread_watch_started",
        epoch_unix_ms = epoch().1,
        observers_installed,
        reporter_started = spawned,
        log_span_min_us = LOG_SPAN_MIN_US,
        slow_paste_us = SLOW_PASTE_US,
        stack_sampling,
        stack_first_sample_us = STACK_FIRST_SAMPLE_US,
        "main-thread watch started"
    );
}

/// Set to `1` or `true` to turn the stack sampler off.
const DISABLE_STACKS_ENV: &str = "MURMUR_DISABLE_MAIN_THREAD_STACKS";

fn stacks_disabled_by_env(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| {
        let value = value.to_string_lossy();
        let value = value.trim();
        value == "1" || value.eq_ignore_ascii_case("true")
    })
}

struct Reporter {
    history: VecDeque<Span>,
    scratch: Vec<Span>,
    standalone_window_start_us: u64,
    standalone_in_window: u32,
    suppressed: u64,
    dropped: u64,
    last_drain_us: u64,
    stacks: StackPacer,
    rates: RatePacer,
}

/// The reporter suspends the main thread for each stack sample, so it must
/// never be the one that waits to be scheduled: run it at the main thread's
/// own priority.
#[cfg(target_os = "macos")]
fn raise_reporter_priority() {
    // SAFETY: sets the calling thread's own QoS class.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_reporter_priority() {}

fn reporter_loop() {
    raise_reporter_priority();
    // Bind the Mach calls the sampler makes while the main thread is
    // suspended, so no lazy-binding stub takes dyld's lock then.
    stack::warm_up();
    let _ = REPORTER_THREAD.set(std::thread::current());
    let mut reporter = Reporter {
        history: VecDeque::with_capacity(RING_CAPACITY),
        scratch: Vec::with_capacity(RING_CAPACITY),
        standalone_window_start_us: 0,
        standalone_in_window: 0,
        suppressed: 0,
        dropped: 0,
        last_drain_us: 0,
        stacks: StackPacer::default(),
        rates: RatePacer::default(),
    };
    loop {
        let now = now_us();
        reporter.sample_stack(now);
        reporter.report_rates(now);
        if now.saturating_sub(reporter.last_drain_us) >= DRAIN_INTERVAL_US {
            reporter.last_drain_us = now;
            reporter.tick();
        }
        let now = now_us();
        let since = BUSY_SINCE_US.load(Ordering::SeqCst);
        let until_drain =
            DRAIN_INTERVAL_US.saturating_sub(now.saturating_sub(reporter.last_drain_us));
        let idle = Duration::from_micros(until_drain.max(WATCH_POLL_MIN_US));
        match reporter
            .stacks
            .park_plan(since, now, STACK_TARGET.get().is_some(), idle)
        {
            ParkPlan::Watch(wait) => {
                // A stretch is being watched: wake at its next sample point.
                std::thread::park_timeout(wait);
            }
            ParkPlan::Sleep(wait) => {
                // Sampling is off, so a busy stretch needs no reaction. Sleep
                // to the next drain without asking the main thread to wake us.
                std::thread::park_timeout(wait);
            }
            ParkPlan::Idle(wait) => {
                // Nothing to watch yet. Park until the next drain, or until
                // the main thread starts a stretch. The flag is set before the
                // last check, and the observer stores the stretch start before
                // reading the flag, so one of them sees the other and a
                // stretch is never slept through.
                REPORTER_PARKED.store(true, Ordering::SeqCst);
                if BUSY_SINCE_US.load(Ordering::SeqCst) == since {
                    std::thread::park_timeout(wait);
                }
                REPORTER_PARKED.store(false, Ordering::SeqCst);
            }
        }
    }
}

/// How the reporter waits between looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParkPlan {
    /// Wake at a busy stretch's next sample point.
    Watch(Duration),
    /// Sampling is on and nothing is being watched: park until the drain, and
    /// let the main thread wake us when a stretch begins.
    Idle(Duration),
    /// Sampling is off: sleep until the drain, and ask for no wakeups.
    Sleep(Duration),
}

/// What to do with one finished capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureVerdict {
    /// The capture held the main thread too long: sampling stays off for the
    /// rest of the process.
    stop_sampling: bool,
    /// Log the sample.
    publish: bool,
}

/// `held_us` is how long the main thread was suspended, `frames` how many
/// return addresses were read, `since_before` the busy stretch the capture
/// was for, and `since_after` the stretch in progress once it finished.
///
/// - A hold over [`STACK_MAX_HOLD_US`] means the reporter was descheduled
///   while it held the main thread: stop sampling. The sample is still
///   logged, with `sampling_stopped`, so the stop is visible.
/// - A capture that read nothing is never logged.
/// - Otherwise the sample is logged only if the same stretch was still
///   running when the capture ended; a stack taken after the stretch ended
///   or was replaced is not the stall's.
fn judge_capture(
    held_us: u64,
    frames: usize,
    since_before: u64,
    since_after: u64,
) -> CaptureVerdict {
    let stop_sampling = held_us > STACK_MAX_HOLD_US;
    CaptureVerdict {
        stop_sampling,
        publish: frames > 0 && (stop_sampling || since_after == since_before),
    }
}

/// Decides when a busy stretch is sampled, and keeps stack events rare.
#[derive(Default)]
struct StackPacer {
    /// The busy stretch (its start) the samples below belong to.
    stretch_start_us: u64,
    samples_taken: u8,
    window_start_us: u64,
    in_window: u32,
    suppressed: u64,
    /// Set once a capture held the main thread too long.
    disabled: bool,
}

impl StackPacer {
    /// Which sample (1 or 2) a stretch that began at `since_us` is due, at `now`.
    fn due(&mut self, since_us: u64, now: u64) -> Option<u8> {
        if since_us == 0 {
            return None;
        }
        if since_us != self.stretch_start_us {
            self.stretch_start_us = since_us;
            self.samples_taken = 0;
        }
        let busy_us = now.saturating_sub(since_us);
        match self.samples_taken {
            0 if busy_us >= STACK_FIRST_SAMPLE_US => Some(1),
            1 if busy_us >= STACK_SECOND_SAMPLE_US => Some(2),
            _ => None,
        }
    }

    /// Whether a busy stretch can still lead to a sample.
    fn sampling_active(&self, sampling: bool) -> bool {
        sampling && !self.disabled
    }

    /// How the reporter should wait next. `Sleep` is a plain timed sleep for
    /// when sampling is off: it never sets the parked flag, so the main
    /// thread's run-loop observer is never asked to signal the reporter for
    /// nothing. `Idle` and `Watch` are for an active sampler.
    fn park_plan(&self, since_us: u64, now: u64, sampling: bool, idle: Duration) -> ParkPlan {
        if !self.sampling_active(sampling) {
            return ParkPlan::Sleep(idle);
        }
        let wait = self.next_wait(since_us, now, sampling, idle);
        if wait < idle {
            ParkPlan::Watch(wait)
        } else {
            ParkPlan::Idle(wait)
        }
    }

    /// How long the reporter may sleep before it next has to look, given the
    /// busy stretch that began at `since_us` (0 when the thread is waiting).
    /// Returns `idle` when there is nothing to watch for, and otherwise the
    /// time to the stretch's next sample point, never more than
    /// [`WATCH_POLL_MAX_US`] and never less than [`WATCH_POLL_MIN_US`].
    fn next_wait(&self, since_us: u64, now: u64, sampling: bool, idle: Duration) -> Duration {
        if !sampling || self.disabled || since_us == 0 {
            return idle;
        }
        let taken = if since_us == self.stretch_start_us {
            self.samples_taken
        } else {
            0
        };
        let target = match taken {
            0 => STACK_FIRST_SAMPLE_US,
            1 => STACK_SECOND_SAMPLE_US,
            _ => return idle,
        };
        let busy_us = now.saturating_sub(since_us);
        let wait_us = target
            .saturating_sub(busy_us)
            .clamp(WATCH_POLL_MIN_US, WATCH_POLL_MAX_US);
        Duration::from_micros(wait_us).min(idle)
    }

    /// Apply a capture's verdict: a hold that was too long turns sampling off.
    fn apply(&mut self, verdict: CaptureVerdict) {
        if verdict.stop_sampling {
            self.disabled = true;
        }
    }

    /// Count a taken (or refused) sample; `false` once the window is full.
    fn admit(&mut self, sample: u8, now: u64) -> bool {
        self.samples_taken = sample;
        if now.saturating_sub(self.window_start_us) >= STACK_WINDOW_US {
            self.window_start_us = now;
            self.in_window = 0;
        }
        if self.in_window >= MAX_STACKS_PER_WINDOW {
            self.suppressed += 1;
            return false;
        }
        self.in_window += 1;
        true
    }
}

/// Paces the per-second rate events.
#[derive(Default)]
struct RatePacer {
    bucket_start_us: u64,
    last_idle_log_us: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RateBucket {
    window_us: u64,
    audio_level_emits: u64,
    ipc_invokes: u64,
    ipc_main_invokes: u64,
    ipc_main_us: u64,
}

impl RatePacer {
    /// Close the bucket once it is a second old; `Some` when it is worth a log.
    fn close(&mut self, now: u64, take: impl FnOnce() -> RateBucket) -> Option<RateBucket> {
        if self.bucket_start_us == 0 {
            self.bucket_start_us = now;
            // Discard what built up before the first bucket.
            take();
            return None;
        }
        let window_us = now.saturating_sub(self.bucket_start_us);
        if window_us < RATE_BUCKET_US {
            return None;
        }
        self.bucket_start_us = now;
        let mut bucket = take();
        bucket.window_us = window_us;
        let recording = bucket.audio_level_emits > 0;
        let busy_idle = bucket.ipc_invokes >= RATE_IDLE_MIN_INVOKES
            && self
                .last_idle_log_us
                .is_none_or(|last| now.saturating_sub(last) >= RATE_IDLE_LOG_US);
        if recording {
            return Some(bucket);
        }
        if busy_idle {
            self.last_idle_log_us = Some(now);
            return Some(bucket);
        }
        None
    }
}

fn take_rate_counters() -> RateBucket {
    RateBucket {
        window_us: 0,
        audio_level_emits: AUDIO_LEVEL_EMITS.swap(0, Ordering::Relaxed),
        ipc_invokes: IPC_INVOKES.swap(0, Ordering::Relaxed),
        ipc_main_invokes: IPC_MAIN_INVOKES.swap(0, Ordering::Relaxed),
        ipc_main_us: IPC_MAIN_US.swap(0, Ordering::Relaxed),
    }
}

impl Reporter {
    /// A1: while one busy stretch is in progress, read the main thread's
    /// stack and log symbol names. Never touches the main thread's state
    /// beyond [`stack::capture`], which suspends it for microseconds.
    fn sample_stack(&mut self, now: u64) {
        let Some(target) = STACK_TARGET.get() else {
            return;
        };
        if self.stacks.disabled {
            return;
        }
        let since = BUSY_SINCE_US.load(Ordering::Acquire);
        let Some(sample) = self.stacks.due(since, now) else {
            return;
        };
        if !self.stacks.admit(sample, now) {
            return;
        }
        let mut raw = [0u64; stack::MAX_FRAMES];
        let captured = stack::capture(*target, &mut raw);
        let count = captured.frames;
        let captured_at = now_us();
        let verdict = judge_capture(
            captured.held_us,
            count,
            since,
            BUSY_SINCE_US.load(Ordering::Acquire),
        );
        let stopped = verdict.stop_sampling;
        self.stacks.apply(verdict);
        if !verdict.publish {
            return;
        }
        let symbolised = stack::symbolise(&raw[..count]);
        let suppressed = std::mem::take(&mut self.stacks.suppressed);
        tracing::info!(
            target: "system",
            event_code = "system.main_thread_stack",
            sample = sample as u64,
            busy_us = captured_at.saturating_sub(since),
            start_us = since,
            start_unix_ms = unix_ms_at(since),
            frame_count = count as u64,
            frames = symbolised.as_str(),
            held_us = captured.held_us,
            sampling_stopped = stopped,
            suppressed,
            "main-thread stack sample"
        );
    }

    /// A3: per-second `audio-level` and IPC counters.
    fn report_rates(&mut self, now: u64) {
        let Some(bucket) = self.rates.close(now, take_rate_counters) else {
            return;
        };
        tracing::info!(
            target: "system",
            event_code = "system.main_thread_rates",
            window_us = bucket.window_us,
            start_unix_ms = unix_ms_at(now.saturating_sub(bucket.window_us)),
            audio_level_emits = bucket.audio_level_emits,
            ipc_invokes = bucket.ipc_invokes,
            ipc_main_invokes = bucket.ipc_main_invokes,
            ipc_main_us = bucket.ipc_main_us,
            "main-thread rates"
        );
    }

    fn tick(&mut self) {
        let Some(ring) = RING.get() else {
            return;
        };
        self.scratch.clear();
        let dropped = ring.drain_into(&mut self.scratch);
        self.dropped += dropped;
        let now = now_us();
        let drained = std::mem::take(&mut self.scratch);
        for span in &drained {
            if should_log_standalone(span) {
                self.log_standalone(span, now);
            }
            self.history.push_back(*span);
        }
        self.scratch = drained;
        while self
            .history
            .front()
            .is_some_and(|span| span.end_us().saturating_add(HISTORY_RETAIN_US) < now)
        {
            self.history.pop_front();
        }
        let due: Vec<Arc<PasteState>> = {
            let mut pending = PENDING_PASTES.lock().unwrap_or_else(|p| p.into_inner());
            let delay = REPORT_DELAY.as_micros() as u64;
            let (due, waiting): (Vec<_>, Vec<_>) = pending.drain(..).partition(|paste| {
                paste
                    .submitted_us
                    .load(Ordering::Acquire)
                    .saturating_add(delay)
                    <= now
            });
            *pending = waiting;
            due
        };
        for paste in due {
            self.report_paste(&paste, now);
        }
    }

    fn log_standalone(&mut self, span: &Span, now: u64) {
        if now.saturating_sub(self.standalone_window_start_us) >= STANDALONE_WINDOW_US {
            self.standalone_window_start_us = now;
            self.standalone_in_window = 0;
        }
        if self.standalone_in_window >= MAX_STANDALONE_PER_WINDOW {
            self.suppressed += 1;
            return;
        }
        self.standalone_in_window += 1;
        let suppressed = std::mem::take(&mut self.suppressed);
        let dropped = std::mem::take(&mut self.dropped);
        tracing::info!(
            target: "system",
            event_code = "system.main_thread_span",
            kind = span.kind.as_str(),
            label = span.label,
            start_us = span.start_us,
            start_unix_ms = unix_ms_at(span.start_us),
            duration_us = span.duration_us,
            queue_us = span.queue_us,
            on_main = span.on_main,
            longest_phase = span.longest_phase,
            longest_phase_us = span.longest_phase_us,
            segments = span.segments as u64,
            suppressed,
            dropped,
            "main-thread span"
        );
    }

    fn report_paste(&mut self, paste: &PasteState, now: u64) {
        let queued = paste.queued_us;
        let started = paste.started_us.load(Ordering::Acquire);
        let finished = paste.finished_us.load(Ordering::Acquire);
        let window_end = if started > 0 { started } else { now };
        let open_busy_since = BUSY_SINCE_US.load(Ordering::Acquire);
        let summary = summarize_window(
            self.history.iter().copied(),
            queued,
            window_end,
            open_busy_since,
            now,
        );
        let wait_us = window_end.saturating_sub(queued);
        let slow = wait_us >= SLOW_PASTE_US;
        let mut overlapping: Vec<(Span, u64)> = if slow {
            self.history
                .iter()
                .filter_map(|span| {
                    let overlap = overlap_us(span.start_us, span.end_us(), queued, window_end);
                    (overlap > 0).then_some((*span, overlap))
                })
                .collect()
        } else {
            Vec::new()
        };
        let truncated = overlapping.len() > MAX_PASTE_SPANS;
        if truncated {
            overlapping.sort_by_key(|(_, overlap)| std::cmp::Reverse(*overlap));
            overlapping.truncate(MAX_PASTE_SPANS);
        }
        overlapping.sort_by_key(|(span, _)| span.start_us);
        tracing::info!(
            target: "pipeline",
            event_code = "pipeline.main_thread_paste_wait",
            recording_id = paste.recording_id,
            outcome = if started > 0 { "started" } else { "not_started" },
            inject_queue_us = wait_us,
            inject_job_us = if started > 0 && finished >= started { finished - started } else { 0 },
            queued_us = queued,
            started_us = started,
            queued_unix_ms = unix_ms_at(queued),
            slow,
            runloop_busy_us = summary.busy_us,
            runloop_idle_us = wait_us.saturating_sub(summary.busy_us),
            labelled_us = summary.labelled_us,
            unlabelled_busy_us = summary.busy_us.saturating_sub(summary.labelled_us),
            job_us = summary.job_us,
            command_us = summary.command_us,
            callback_us = summary.callback_us,
            section_us = summary.section_us,
            event_loop_drain_us = summary.drain_us,
            longest_label = summary.longest_label,
            longest_kind = summary.longest_kind,
            longest_us = summary.longest_us,
            span_count = summary.span_count,
            spans_logged = overlapping.len() as u64,
            spans_truncated = truncated,
            preview_hide = preview_hide_order(paste.recording_id, queued, started),
            "main-thread paste wait"
        );
        for (span, overlap) in overlapping {
            tracing::info!(
                target: "system",
                event_code = "system.main_thread_paste_span",
                recording_id = paste.recording_id,
                kind = span.kind.as_str(),
                label = span.label,
                start_us = span.start_us,
                start_unix_ms = unix_ms_at(span.start_us),
                duration_us = span.duration_us,
                overlap_us = overlap,
                queue_us = span.queue_us,
                on_main = span.on_main,
                longest_phase = span.longest_phase,
                longest_phase_us = span.longest_phase_us,
                segments = span.segments as u64,
                "main-thread span during paste wait"
            );
        }
    }
}

fn should_log_standalone(span: &Span) -> bool {
    match span.kind {
        // Busy spans carry their longest segment; segments are logged only
        // inside a slow paste window.
        Kind::RunloopSegment => false,
        Kind::Job => span.duration_us >= LOG_SPAN_MIN_US || span.queue_us >= LOG_QUEUE_MIN_US,
        _ => span.duration_us >= LOG_SPAN_MIN_US,
    }
}

fn overlap_us(start: u64, end: u64, window_start: u64, window_end: u64) -> u64 {
    end.min(window_end).saturating_sub(start.max(window_start))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WindowSummary {
    busy_us: u64,
    labelled_us: u64,
    job_us: u64,
    command_us: u64,
    callback_us: u64,
    section_us: u64,
    drain_us: u64,
    longest_label: &'static str,
    longest_kind: &'static str,
    longest_us: u64,
    span_count: u64,
}

/// Merge `intervals` (each already clipped to the window) and return their
/// union length.
fn union_len(intervals: &mut [(u64, u64)]) -> u64 {
    intervals.sort_unstable();
    let mut total = 0;
    let mut current: Option<(u64, u64)> = None;
    for &(start, end) in intervals.iter() {
        match current {
            Some((cs, ce)) if start <= ce => current = Some((cs, ce.max(end))),
            Some((cs, ce)) => {
                total += ce - cs;
                current = Some((start, end));
            }
            None => current = Some((start, end)),
        }
    }
    if let Some((cs, ce)) = current {
        total += ce - cs;
    }
    total
}

fn summarize_window(
    spans: impl Iterator<Item = Span>,
    window_start: u64,
    window_end: u64,
    open_busy_since: u64,
    now: u64,
) -> WindowSummary {
    let mut summary = WindowSummary {
        longest_label: "none",
        longest_kind: "none",
        ..WindowSummary::default()
    };
    let mut busy: Vec<(u64, u64)> = Vec::new();
    let mut labelled: Vec<(u64, u64)> = Vec::new();
    for span in spans {
        let overlap = overlap_us(span.start_us, span.end_us(), window_start, window_end);
        if overlap == 0 {
            continue;
        }
        summary.span_count += 1;
        let clipped = (
            span.start_us.max(window_start),
            span.end_us().min(window_end),
        );
        match span.kind {
            Kind::RunloopBusy => busy.push(clipped),
            Kind::RunloopSegment => {
                if span.label == runloop_labels::DRAIN {
                    summary.drain_us += overlap;
                }
            }
            Kind::Job => summary.job_us += overlap,
            Kind::Command => summary.command_us += overlap,
            Kind::Callback => summary.callback_us += overlap,
            Kind::Section => summary.section_us += overlap,
        }
        if span.kind.is_labelled_app_work() {
            labelled.push(clipped);
            // Prefer the outermost labelled work: sections nest inside jobs.
            if overlap > summary.longest_us
                || (overlap == summary.longest_us && span.kind != Kind::Section)
            {
                summary.longest_us = overlap;
                summary.longest_label = span.label;
                summary.longest_kind = span.kind.as_str();
            }
        }
    }
    if open_busy_since > 0 && open_busy_since < window_end {
        busy.push((open_busy_since.max(window_start), window_end.min(now)));
    }
    summary.labelled_us = union_len(&mut labelled);
    // Labelled work always occupies the thread, even where the observer's
    // ordering missed part of it, so busy time includes it.
    busy.extend_from_slice(&labelled);
    summary.busy_us = union_len(&mut busy);
    summary
}

// ---------------------------------------------------------------------------
// Applied markers
// ---------------------------------------------------------------------------

mod applied {
    use super::*;

    /// Test builds count queued markers instead of queueing them: the test
    /// harness's main thread never runs the main queue.
    #[cfg(test)]
    pub(super) static MARKERS_QUEUED: AtomicU64 = AtomicU64::new(0);

    /// Queue a marker block on the main queue, behind the blocks already
    /// there. When it runs it records `label` with the enqueue-to-run latency
    /// in `queue_us` and a zero duration. Main thread only; never blocks.
    pub(super) fn queue_marker(label: &'static str) {
        if !ENABLED.load(Ordering::Relaxed) || !is_main() {
            return;
        }
        #[cfg(test)]
        MARKERS_QUEUED.fetch_add(1, Ordering::Relaxed);
        #[cfg(all(target_os = "macos", not(test)))]
        macos::queue(label, now_us());
        #[cfg(all(target_os = "macos", test))]
        let _ = (label, now_us());
        #[cfg(not(target_os = "macos"))]
        let _ = label;
    }

    /// Record a marker that ran at `ran_us` after being queued at `queued_us`.
    pub(super) fn record_marker(label: &'static str, queued_us: u64, ran_us: u64) {
        let latency = ran_us.saturating_sub(queued_us);
        if latency < APPLIED_RECORD_MIN_US {
            return;
        }
        record(Span {
            kind: Kind::Job,
            label,
            start_us: ran_us,
            duration_us: 0,
            queue_us: latency,
            on_main: true,
            ..Span::EMPTY
        });
    }

    #[cfg(target_os = "macos")]
    #[cfg_attr(test, allow(dead_code))]
    mod macos {
        use super::*;
        use std::ffi::c_void;

        extern "C" {
            // `dispatch_get_main_queue()` is this symbol's address.
            static _dispatch_main_q: c_void;
            fn dispatch_async_f(
                queue: *const c_void,
                context: *mut c_void,
                work: extern "C" fn(*mut c_void),
            );
        }

        struct Marker {
            label: &'static str,
            queued_us: u64,
        }

        extern "C" fn run(context: *mut c_void) {
            // SAFETY: `context` is the `Box<Marker>` leaked in `queue`, and
            // GCD runs each block exactly once.
            let marker = unsafe { Box::from_raw(context.cast::<Marker>()) };
            record_marker(marker.label, marker.queued_us, now_us());
        }

        pub(super) fn queue(label: &'static str, queued_us: u64) {
            let context = Box::into_raw(Box::new(Marker { label, queued_us }));
            // SAFETY: the main queue outlives the process; `run` takes
            // ownership of `context`.
            unsafe {
                dispatch_async_f(
                    std::ptr::addr_of!(_dispatch_main_q),
                    context.cast::<c_void>(),
                    run,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stack sampling
// ---------------------------------------------------------------------------

/// Reads another thread's stack as raw return addresses, then names them.
///
/// The reporter thread is the only caller. It never walks while the target is
/// running, and while the target is suspended it does nothing that can wait
/// on a lock the target holds: no allocation, no logging, no symbol lookup,
/// no `dladdr` (which takes dyld's lock). Symbol lookup happens after
/// `thread_resume`.
pub(crate) mod stack {
    /// Frames kept per sample.
    pub(crate) const MAX_FRAMES: usize = 24;
    /// Longest text kept per frame.
    const MAX_FRAME_LEN: usize = 120;
    /// Longest `frames` string.
    pub(crate) const MAX_FRAMES_LEN: usize = 2_400;

    /// One frame's text, or the whole `frames` string, may hold only these:
    /// ASCII letters and digits and the punctuation of C, C++, Rust and
    /// Objective-C symbols. Paths, quotes, and `=` are excluded.
    fn is_frame_char(c: char) -> bool {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '_' | '.'
                    | '$'
                    | ':'
                    | '['
                    | ']'
                    | '-'
                    | '+'
                    | '<'
                    | '>'
                    | ','
                    | '('
                    | ')'
                    | '*'
                    | '&'
                    | '!'
                    | '@'
                    | '?'
                    | ' '
            )
    }

    /// An image file name: no spaces, no path separators.
    fn is_image_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | '?')
    }

    /// One `image!symbol` item. The image is non-empty and uses
    /// [`is_image_char`]; the symbol is non-empty, uses [`is_frame_char`], and
    /// has a space only inside Objective-C `[...]`.
    fn is_safe_item(item: &str) -> bool {
        let Some((image, symbol)) = item.split_once('!') else {
            return false;
        };
        if image.is_empty() || symbol.is_empty() || !image.chars().all(is_image_char) {
            return false;
        }
        let mut depth = 0u32;
        for c in symbol.chars() {
            match c {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                ' ' if depth == 0 => return false,
                _ => {}
            }
            if !is_frame_char(c) {
                return false;
            }
        }
        true
    }

    /// `frames` is a `;`-joined list of `image!symbol` items, bounded.
    pub(crate) fn is_safe_frames(value: &str) -> bool {
        !value.is_empty() && value.len() <= MAX_FRAMES_LEN && value.split(';').all(is_safe_item)
    }

    /// Keep an image file name to its alphabet; anything else becomes `_`.
    #[cfg(target_os = "macos")]
    fn clean_image(text: &str, out: &mut String) {
        let start = out.len();
        for c in text.chars().take(MAX_FRAME_LEN) {
            out.push(if is_image_char(c) { c } else { '_' });
        }
        if out.len() == start {
            out.push('?');
        }
    }

    /// Keep a symbol to its alphabet (anything else becomes `?`), with spaces
    /// only inside `[...]` (elsewhere they become `_`), and bound the length.
    #[cfg(target_os = "macos")]
    fn clean_symbol(text: &str, out: &mut String) {
        let mut depth = 0u32;
        for c in text.chars().take(MAX_FRAME_LEN) {
            let kept = match c {
                '[' => {
                    depth += 1;
                    c
                }
                ']' => {
                    depth = depth.saturating_sub(1);
                    c
                }
                ' ' if depth == 0 => '_',
                c if is_frame_char(c) => c,
                _ => '?',
            };
            out.push(kept);
        }
    }

    /// Name each address as `module!symbol`: the image's file name (never its
    /// path) and the nearest symbol. Runs while the target is not suspended.
    pub(crate) fn symbolise(addresses: &[u64]) -> String {
        let mut out = String::new();
        for &address in addresses {
            if out.len() >= MAX_FRAMES_LEN - MAX_FRAME_LEN * 2 - 2 {
                break;
            }
            if !out.is_empty() {
                out.push(';');
            }
            name_one(address, &mut out);
        }
        if out.is_empty() {
            out.push('?');
        }
        out
    }

    #[cfg(target_os = "macos")]
    fn name_one(address: u64, out: &mut String) {
        use std::ffi::CStr;
        // SAFETY: `Dl_info` is plain data; `dladdr` fills it or fails.
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        let found = unsafe { libc::dladdr(address as *const libc::c_void, &mut info) } != 0;
        if !found {
            out.push_str("?!?");
            return;
        }
        let image = (!info.dli_fname.is_null())
            .then(|| {
                unsafe { CStr::from_ptr(info.dli_fname) }
                    .to_string_lossy()
                    .into_owned()
            })
            .map(|path| path.rsplit('/').next().unwrap_or("").to_string())
            .filter(|name| !name.is_empty());
        clean_image(image.as_deref().unwrap_or("?"), out);
        out.push('!');
        if info.dli_sname.is_null() {
            out.push('?');
        } else {
            // SAFETY: `dli_sname` is a NUL-terminated string owned by dyld.
            let symbol = unsafe { CStr::from_ptr(info.dli_sname) }.to_string_lossy();
            let start = out.len();
            clean_symbol(&symbol, out);
            if out.len() == start {
                out.push('?');
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn name_one(_address: u64, out: &mut String) {
        out.push('?');
    }

    /// The thread to sample and the bounds of its stack. Frame-pointer
    /// chains are followed only inside these bounds, so a corrupt frame
    /// pointer can never make the reporter read unmapped memory.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct Target {
        #[cfg_attr(
            not(all(target_os = "macos", target_arch = "aarch64")),
            allow(dead_code)
        )]
        port: u32,
        #[cfg_attr(
            not(all(target_os = "macos", target_arch = "aarch64")),
            allow(dead_code)
        )]
        stack_low: u64,
        #[cfg_attr(
            not(all(target_os = "macos", target_arch = "aarch64")),
            allow(dead_code)
        )]
        stack_high: u64,
    }

    impl Target {
        /// The calling thread. `None` where sampling is unsupported.
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        pub(crate) fn current() -> Option<Self> {
            // SAFETY: queries about the calling thread only.
            let (port, high, size) = unsafe {
                let me = libc::pthread_self();
                (
                    libc::pthread_mach_thread_np(me),
                    libc::pthread_get_stackaddr_np(me) as u64,
                    libc::pthread_get_stacksize_np(me) as u64,
                )
            };
            (port != 0 && high != 0 && size != 0).then(|| Self {
                port,
                stack_low: high.saturating_sub(size),
                stack_high: high,
            })
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        pub(crate) fn current() -> Option<Self> {
            None
        }
    }

    /// Bind the Mach calls [`capture`] makes while the main thread is
    /// suspended, from the reporter's own thread and before any sampling.
    ///
    /// A binary built for macOS 11 (the arm64 default deployment target, which
    /// dev and test builds use) binds imported functions lazily: the first
    /// call goes through a stub into dyld, which takes dyld's lock. If that
    /// first call ran while the main thread was suspended in the middle of
    /// its own dyld work, the two would deadlock. So the reporter makes each
    /// call once here. `thread_get_state` on the caller's own thread is
    /// legal (the state it returns is unused). `thread_resume` on a thread
    /// that is not suspended returns an error and changes nothing. This is
    /// preferred over a `.cargo/config.toml` deployment target because it
    /// holds for every build configuration, including the bundle's
    /// 14.0 minimum, where binding behaviour is the linker's choice.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub(crate) fn warm_up() {
        extern "C" {
            fn thread_resume(thread: u32) -> i32;
            fn thread_get_state(thread: u32, flavor: i32, state: *mut u32, count: *mut u32) -> i32;
        }
        // SAFETY: queries about the calling thread only; the port needs no
        // release (`pthread_mach_thread_np` adds no reference).
        unsafe {
            let me = libc::pthread_mach_thread_np(libc::pthread_self());
            let mut state = [0u32; 68];
            let mut count = state.len() as u32;
            let _ = thread_get_state(me, 6, state.as_mut_ptr(), &mut count);
            let _ = thread_resume(me);
        }
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn warm_up() {}

    /// What one capture produced.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Capture {
        /// Return addresses written, innermost first; 0 when the thread could
        /// not be read.
        pub(crate) frames: usize,
        /// Wall time the target stayed suspended, in microseconds.
        pub(crate) held_us: u64,
    }

    /// Raw return addresses of `target`, innermost first, written to `out`.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub(crate) fn capture(target: Target, out: &mut [u64; MAX_FRAMES]) -> Capture {
        #[repr(C)]
        struct ArmThreadState64 {
            x: [u64; 29],
            fp: u64,
            lr: u64,
            sp: u64,
            pc: u64,
            cpsr: u32,
            flags: u32,
        }
        const ARM_THREAD_STATE64: i32 = 6;
        const STATE_COUNT: u32 = (std::mem::size_of::<ArmThreadState64>() / 4) as u32;
        // Addresses carry no pointer-authentication bits in a plain arm64
        // process; the mask keeps a stray high bit from reaching `dladdr`.
        const ADDRESS_MASK: u64 = 0x0000_ffff_ffff_ffff;

        extern "C" {
            fn thread_suspend(thread: u32) -> i32;
            fn thread_resume(thread: u32) -> i32;
            fn thread_get_state(thread: u32, flavor: i32, state: *mut u32, count: *mut u32) -> i32;
        }

        // Everything the suspended section needs exists before it starts.
        let mut state = ArmThreadState64 {
            x: [0; 29],
            fp: 0,
            lr: 0,
            sp: 0,
            pc: 0,
            cpsr: 0,
            flags: 0,
        };
        let mut count = STATE_COUNT;
        let mut written = 0usize;
        let suspended_at = std::time::Instant::now();

        // SAFETY: Mach thread calls on a port the process owns. Between
        // suspend and resume this code only reads registers and loads from
        // the target's own stack, inside the bounds checked below. It calls
        // nothing that allocates, locks, logs, or can panic.
        unsafe {
            if thread_suspend(target.port) != 0 {
                return Capture::default();
            }
            let got = thread_get_state(
                target.port,
                ARM_THREAD_STATE64,
                (&mut state as *mut ArmThreadState64).cast::<u32>(),
                &mut count,
            );
            if got == 0 {
                let pc = state.pc & ADDRESS_MASK;
                let lr = state.lr & ADDRESS_MASK;
                if let Some(slot) = out.get_mut(written) {
                    *slot = pc;
                    written += 1;
                }
                // The innermost function may not have built its frame yet, so
                // its caller is in `lr`; the chain below skips a repeat.
                if lr != 0 {
                    if let Some(slot) = out.get_mut(written) {
                        *slot = lr;
                        written += 1;
                    }
                }
                let sp_inside = state.sp >= target.stack_low && state.sp < target.stack_high;
                let mut fp = state.fp;
                while sp_inside && written < MAX_FRAMES {
                    // Aligned, above the stack pointer, and wholly inside the stack.
                    if fp & 0xf != 0
                        || fp < state.sp
                        || fp < target.stack_low
                        || fp.saturating_add(16) > target.stack_high
                    {
                        break;
                    }
                    let record = fp as *const u64;
                    let next = std::ptr::read_volatile(record);
                    let ret = std::ptr::read_volatile(record.add(1)) & ADDRESS_MASK;
                    if ret == 0 {
                        break;
                    }
                    let repeats = written
                        .checked_sub(1)
                        .and_then(|last| out.get(last))
                        .is_some_and(|prev| *prev == ret);
                    if !repeats {
                        match out.get_mut(written) {
                            Some(slot) => {
                                *slot = ret;
                                written += 1;
                            }
                            None => break,
                        }
                    }
                    if next <= fp {
                        break;
                    }
                    fp = next;
                }
            }
            // A failed resume would leave the main thread stopped: retry.
            let mut attempts = 0;
            while thread_resume(target.port) != 0 && attempts < 8 {
                attempts += 1;
            }
        }
        let held_us = u64::try_from(suspended_at.elapsed().as_micros()).unwrap_or(u64::MAX);
        if written > 0 && out.first().copied().unwrap_or(0) == 0 {
            return Capture { frames: 0, held_us };
        }
        Capture {
            frames: written,
            held_us,
        }
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    pub(crate) fn capture(_target: Target, _out: &mut [u64; MAX_FRAMES]) -> Capture {
        Capture::default()
    }
}

// ---------------------------------------------------------------------------
// Run-loop observer (macOS)
// ---------------------------------------------------------------------------

pub(crate) mod runloop_labels {
    pub(crate) const BUSY: &str = "runloop.busy";
    /// From `kCFRunLoopEntry` to the next activity.
    pub(crate) const ENTRY: &str = "runloop.entry";
    /// Timer and source0 notifications through their handlers.
    pub(crate) const TIMERS: &str = "runloop.timers";
    /// Source0 handlers, blocks, and the main dispatch queue.
    pub(crate) const SOURCES: &str = "runloop.sources";
    /// What woke the loop: timers, mach-port sources (window server, WebKit),
    /// or the main dispatch queue.
    pub(crate) const WAKE: &str = "runloop.wake";
    /// Before-waiting observers that run ahead of Tao: Core Animation commit,
    /// AppKit display and layout.
    pub(crate) const OBSERVERS: &str = "runloop.observers";
    /// Tao's event-loop drain: queued `run_on_main_thread` jobs, window
    /// messages sent from other threads, webview script evaluation, events.
    pub(crate) const DRAIN: &str = "runloop.event_loop_drain";
    /// Between run-loop exit and re-entry: AppKit event dispatch (sendEvent).
    pub(crate) const OUTSIDE: &str = "runloop.outside";
}

#[cfg(target_os = "macos")]
mod runloop {
    use super::*;
    use core_foundation::base::CFIndex;
    use core_foundation::runloop::{
        kCFRunLoopAfterWaiting, kCFRunLoopBeforeSources, kCFRunLoopBeforeTimers,
        kCFRunLoopBeforeWaiting, kCFRunLoopCommonModes, kCFRunLoopEntry, kCFRunLoopExit,
        CFRunLoopActivity, CFRunLoopAddObserver, CFRunLoopGetMain, CFRunLoopObserverCreate,
        CFRunLoopObserverRef,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        None,
        Sleep,
        Labelled(&'static str),
    }

    struct State {
        last_mark_us: u64,
        phase: Phase,
        busy_start_us: u64,
        longest_phase: &'static str,
        longest_phase_us: u64,
        segments: u32,
    }

    thread_local! {
        static STATE: RefCell<State> = const { RefCell::new(State {
            last_mark_us: 0,
            phase: Phase::None,
            busy_start_us: 0,
            longest_phase: "",
            longest_phase_us: 0,
            segments: 0,
        }) };
    }

    const EARLY: usize = 1;
    const PRE_DRAIN: usize = 2;
    const LATE: usize = 3;

    extern "C" fn observe(
        _observer: CFRunLoopObserverRef,
        activity: CFRunLoopActivity,
        info: *mut std::ffi::c_void,
    ) {
        let which = info as usize;
        let next = match (which, activity) {
            (EARLY, a) if a == kCFRunLoopEntry => Phase::Labelled(runloop_labels::ENTRY),
            (EARLY, a) if a == kCFRunLoopBeforeTimers => Phase::Labelled(runloop_labels::TIMERS),
            (EARLY, a) if a == kCFRunLoopBeforeSources => Phase::Labelled(runloop_labels::SOURCES),
            (EARLY, a) if a == kCFRunLoopAfterWaiting => Phase::Labelled(runloop_labels::WAKE),
            (EARLY, _) => Phase::Labelled(runloop_labels::OBSERVERS),
            (PRE_DRAIN, _) => Phase::Labelled(runloop_labels::DRAIN),
            (LATE, a) if a == kCFRunLoopBeforeWaiting => Phase::Sleep,
            (LATE, _) => Phase::Labelled(runloop_labels::OUTSIDE),
            _ => return,
        };
        let now = now_us();
        let _ = STATE.try_with(|state| {
            let Ok(mut state) = state.try_borrow_mut() else {
                return;
            };
            transition(&mut state, next, now);
        });
    }

    fn transition(state: &mut State, next: Phase, now: u64) {
        if let Phase::Labelled(label) = state.phase {
            let duration = now.saturating_sub(state.last_mark_us);
            state.segments = state.segments.saturating_add(1);
            if duration > state.longest_phase_us {
                state.longest_phase_us = duration;
                state.longest_phase = label;
            }
            if duration >= RUNLOOP_RECORD_MIN_US {
                record(Span {
                    kind: Kind::RunloopSegment,
                    label,
                    start_us: state.last_mark_us,
                    duration_us: duration,
                    on_main: true,
                    ..Span::EMPTY
                });
            }
        }
        match next {
            Phase::Sleep => {
                if state.busy_start_us > 0 {
                    let duration = now.saturating_sub(state.busy_start_us);
                    if duration >= RUNLOOP_RECORD_MIN_US {
                        record(Span {
                            kind: Kind::RunloopBusy,
                            label: runloop_labels::BUSY,
                            start_us: state.busy_start_us,
                            duration_us: duration,
                            on_main: true,
                            longest_phase: state.longest_phase,
                            longest_phase_us: state.longest_phase_us,
                            segments: state.segments,
                            ..Span::EMPTY
                        });
                    }
                }
                state.busy_start_us = 0;
                state.longest_phase = "";
                state.longest_phase_us = 0;
                state.segments = 0;
                BUSY_SINCE_US.store(0, Ordering::Release);
            }
            Phase::Labelled(_) if state.busy_start_us == 0 => {
                state.busy_start_us = now;
                BUSY_SINCE_US.store(now, Ordering::SeqCst);
                wake_reporter_if_parked();
            }
            _ => {}
        }
        state.last_mark_us = now;
        state.phase = next;
    }

    /// Three observers bracket the phases of each loop iteration. Tao drains
    /// its user events (queued jobs and window messages) in a before-waiting
    /// observer at `CFIndex::MAX`; observers added later at the same order
    /// run after it, so `PRE_DRAIN` (MAX - 1) and `LATE` (MAX) bracket it.
    pub(super) fn install() -> bool {
        let all_early = kCFRunLoopEntry
            | kCFRunLoopBeforeTimers
            | kCFRunLoopBeforeSources
            | kCFRunLoopBeforeWaiting
            | kCFRunLoopAfterWaiting
            | kCFRunLoopExit;
        let ends = kCFRunLoopBeforeWaiting | kCFRunLoopExit;
        let specs: [(usize, CFRunLoopActivity, CFIndex); 3] = [
            (EARLY, all_early, CFIndex::MIN),
            (PRE_DRAIN, ends, CFIndex::MAX - 1),
            (LATE, ends, CFIndex::MAX),
        ];
        let mut installed = true;
        unsafe {
            let main = CFRunLoopGetMain();
            for (which, activities, order) in specs {
                let mut context = core_foundation::runloop::CFRunLoopObserverContext {
                    version: 0,
                    info: which as *mut std::ffi::c_void,
                    retain: None,
                    release: None,
                    copyDescription: None,
                };
                let observer = CFRunLoopObserverCreate(
                    std::ptr::null(),
                    activities,
                    1,
                    order,
                    observe,
                    &mut context,
                );
                if observer.is_null() {
                    installed = false;
                    continue;
                }
                // App-lifetime observer; the run loop retains it.
                CFRunLoopAddObserver(main, observer, kCFRunLoopCommonModes);
            }
        }
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let now = now_us();
            state.last_mark_us = now;
            state.phase = Phase::Labelled(runloop_labels::OUTSIDE);
            state.busy_start_us = now;
            BUSY_SINCE_US.store(now, Ordering::Release);
        });
        installed
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn sleep_closes_a_busy_span_with_its_longest_segment() {
            let _guard = super::super::tests::enable_for_test();
            let mut state = State {
                last_mark_us: 0,
                phase: Phase::None,
                busy_start_us: 0,
                longest_phase: "",
                longest_phase_us: 0,
                segments: 0,
            };
            transition(&mut state, Phase::Labelled(runloop_labels::WAKE), 1_000);
            transition(&mut state, Phase::Labelled(runloop_labels::DRAIN), 3_000);
            transition(&mut state, Phase::Sleep, 40_000);
            let spans = super::super::tests::drain_ring();
            let busy = spans
                .iter()
                .find(|s| s.kind == Kind::RunloopBusy)
                .expect("busy span");
            assert_eq!(busy.start_us, 1_000);
            assert_eq!(busy.duration_us, 39_000);
            assert_eq!(busy.longest_phase, runloop_labels::DRAIN);
            assert_eq!(busy.longest_phase_us, 37_000);
            assert_eq!(busy.segments, 2);
            assert!(spans
                .iter()
                .any(|s| s.kind == Kind::RunloopSegment && s.label == runloop_labels::WAKE));
            assert_eq!(state.busy_start_us, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(super) fn enable_for_test() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _ = RING.set(Ring::new());
        ENABLED.store(true, Ordering::Release);
        // The test thread is the ring's single producer for this test.
        IS_MAIN.with(|flag| flag.set(true));
        drain_ring();
        guard
    }

    pub(super) fn drain_ring() -> Vec<Span> {
        let mut out = Vec::new();
        if let Some(ring) = RING.get() {
            ring.drain_into(&mut out);
        }
        out
    }

    fn span(kind: Kind, label: &'static str, start: u64, end: u64) -> Span {
        Span {
            kind,
            label,
            start_us: start,
            duration_us: end - start,
            ..Span::EMPTY
        }
    }

    #[test]
    fn a_full_ring_drops_new_records_without_blocking() {
        let ring = Ring::new();
        for i in 0..(RING_CAPACITY as u64 + 3) {
            ring.push(span(Kind::Job, "a", i, i + 1));
        }
        let mut out = Vec::new();
        assert_eq!(ring.drain_into(&mut out), 3);
        assert_eq!(out.len(), RING_CAPACITY);
        assert_eq!(out[0].start_us, 0);
        assert!(ring.push(span(Kind::Job, "b", 7, 8)));
        out.clear();
        assert_eq!(ring.drain_into(&mut out), 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "b");
    }

    #[test]
    fn the_ring_hands_records_across_threads_in_order() {
        let ring = std::sync::Arc::new(Ring::new());
        let producer = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                let mut pushed = 0u64;
                while pushed < 20_000 {
                    if ring.push(span(Kind::Job, "a", pushed, pushed + 1)) {
                        pushed += 1;
                    } else {
                        std::thread::yield_now();
                    }
                }
            })
        };
        let mut seen = Vec::new();
        while seen.len() < 20_000 {
            ring.drain_into(&mut seen);
        }
        producer.join().unwrap();
        assert!(seen.iter().enumerate().all(|(i, s)| s.start_us == i as u64));
    }

    #[test]
    fn off_main_threads_never_record() {
        let _guard = enable_for_test();
        std::thread::spawn(|| record_simple(Kind::Job, "off.main", 1, 2, 0))
            .join()
            .unwrap();
        assert!(drain_ring().iter().all(|s| s.label != "off.main"));
    }

    #[test]
    fn command_labels_match_the_registered_handler_exactly() {
        let lib_rs = include_str!("lib.rs");
        let start = lib_rs
            .find("tauri::generate_handler![")
            .expect("generate_handler! in lib.rs")
            + "tauri::generate_handler![".len();
        let end = start + lib_rs[start..].find(']').expect("handler end");
        let mut registered: Vec<&str> = lib_rs[start..end]
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.rsplit("::").next().unwrap_or(entry))
            .collect();
        registered.sort_unstable();
        assert_eq!(COMMAND_LABELS, registered.as_slice());
        assert!(COMMAND_LABELS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(COMMAND_LABELS.iter().all(|label| is_safe_label(label)));
        assert_eq!(command_label("get_status"), "get_status");
        assert_eq!(command_label("plugin:window|show"), "command.other");
    }

    #[test]
    fn job_records_queue_wait_and_run_time() {
        let _guard = enable_for_test();
        let wrapped = job("test.job", || std::thread::sleep(Duration::from_millis(2)));
        std::thread::sleep(Duration::from_millis(3));
        wrapped();
        let spans = drain_ring();
        let recorded = spans.iter().find(|s| s.label == "test.job").expect("job");
        assert_eq!(recorded.kind, Kind::Job);
        assert!(recorded.queue_us >= 3_000, "{recorded:?}");
        assert!(recorded.duration_us >= 2_000, "{recorded:?}");
    }

    #[test]
    fn window_summary_separates_labelled_work_from_unlabelled_busy_time() {
        let spans = vec![
            span(Kind::RunloopBusy, runloop_labels::BUSY, 0, 100_000),
            span(Kind::RunloopSegment, runloop_labels::DRAIN, 20_000, 100_000),
            span(Kind::Job, "preview.hide", 30_000, 70_000),
            span(Kind::Section, "window.hide", 31_000, 69_000),
            span(Kind::Command, "get_status", 200_000, 201_000),
        ];
        let summary = summarize_window(spans.into_iter(), 10_000, 90_000, 0, 1_000_000);
        assert_eq!(summary.busy_us, 80_000);
        assert_eq!(summary.labelled_us, 40_000);
        assert_eq!(summary.job_us, 40_000);
        assert_eq!(summary.section_us, 38_000);
        assert_eq!(summary.drain_us, 70_000);
        assert_eq!(summary.command_us, 0);
        assert_eq!(summary.longest_label, "preview.hide");
        assert_eq!(summary.longest_kind, "job");
        assert_eq!(summary.span_count, 4);
    }

    #[test]
    fn window_summary_counts_an_unclosed_busy_span() {
        let summary = summarize_window(std::iter::empty(), 10_000, 50_000, 30_000, 60_000);
        assert_eq!(summary.busy_us, 20_000);
    }

    #[test]
    fn idle_window_reports_no_busy_time() {
        let summary = summarize_window(
            vec![span(Kind::RunloopBusy, runloop_labels::BUSY, 0, 5_000)].into_iter(),
            10_000,
            50_000,
            0,
            60_000,
        );
        assert_eq!(summary.busy_us, 0);
        assert_eq!(summary.longest_label, "none");
    }

    #[test]
    fn preview_hide_order_is_relative_to_the_paste_window() {
        assert_eq!(classify_preview_hide(None, 10, 20), "none");
        assert_eq!(classify_preview_hide(Some(5), 10, 20), "before_paste");
        assert_eq!(classify_preview_hide(Some(15), 10, 20), "during_wait");
        assert_eq!(classify_preview_hide(Some(15), 10, 0), "during_wait");
        assert_eq!(classify_preview_hide(Some(20), 10, 20), "after_paste");
    }

    #[test]
    fn preview_hides_keep_the_latest_per_recording() {
        let _guard = enable_for_test();
        note_preview_hide(91);
        let first = latest_preview_hide(91).expect("recorded");
        std::thread::sleep(Duration::from_millis(1));
        note_preview_hide(91);
        assert!(latest_preview_hide(91).expect("recorded") > first);
        for id in 100..110 {
            note_preview_hide(id);
        }
        assert!(latest_preview_hide(109).is_some());
        // Eight slots: the oldest hides age out.
        assert!(latest_preview_hide(91).is_none());
        assert!(latest_preview_hide(0).is_none());
    }

    #[test]
    fn labels_are_static_identifiers_only() {
        assert!(is_safe_label("show_dictation_preview"));
        assert!(is_safe_label("runloop.event_loop_drain"));
        assert!(!is_safe_label(""));
        assert!(!is_safe_label("Hello world"));
        assert!(!is_safe_label("plugin:window|show"));
        assert!(!is_safe_label(&"a".repeat(65)));
    }

    #[test]
    fn standalone_logging_thresholds() {
        assert!(!should_log_standalone(&span(Kind::Job, "a", 0, 9_999)));
        assert!(should_log_standalone(&span(Kind::Job, "a", 0, 10_000)));
        let mut queued = span(Kind::Job, "a", 0, 10);
        queued.queue_us = LOG_QUEUE_MIN_US;
        assert!(should_log_standalone(&queued));
        assert!(!should_log_standalone(&span(
            Kind::RunloopSegment,
            runloop_labels::DRAIN,
            0,
            500_000
        )));
        assert!(should_log_standalone(&span(
            Kind::RunloopBusy,
            runloop_labels::BUSY,
            0,
            10_000
        )));
    }
    #[test]
    fn a_busy_stretch_is_sampled_once_at_50ms_and_once_more_at_250ms() {
        let mut pacer = StackPacer::default();
        let since = 1_000_000;
        assert_eq!(pacer.due(0, since + 900_000), None);
        assert_eq!(pacer.due(since, since + STACK_FIRST_SAMPLE_US - 1), None);
        assert_eq!(pacer.due(since, since + STACK_FIRST_SAMPLE_US), Some(1));
        assert!(pacer.admit(1, since + STACK_FIRST_SAMPLE_US));
        // The same stretch is not sampled again before the second mark.
        assert_eq!(pacer.due(since, since + 200_000), None);
        assert_eq!(pacer.due(since, since + STACK_SECOND_SAMPLE_US), Some(2));
        assert!(pacer.admit(2, since + STACK_SECOND_SAMPLE_US));
        assert_eq!(pacer.due(since, since + 5_000_000), None);
        // A new stretch starts over.
        let next = since + 6_000_000;
        assert_eq!(pacer.due(next, next + STACK_FIRST_SAMPLE_US), Some(1));
    }

    #[test]
    fn stack_events_are_rate_limited_per_minute_and_the_rest_are_counted() {
        let mut pacer = StackPacer::default();
        let start = 10_000_000;
        for i in 0..MAX_STACKS_PER_WINDOW {
            assert!(pacer.admit(1, start + u64::from(i)));
        }
        assert!(!pacer.admit(1, start + 1_000));
        assert!(!pacer.admit(2, start + 2_000));
        assert_eq!(pacer.suppressed, 2);
        // A refused sample is still consumed: the stretch is not retried.
        assert_eq!(pacer.samples_taken, 2);
        assert!(pacer.admit(1, start + STACK_WINDOW_US));
        assert_eq!(pacer.in_window, 1);
    }

    #[test]
    fn rates_skip_the_startup_remainder_then_log_each_second_of_recording() {
        let mut pacer = RatePacer::default();
        let busy = |emits| RateBucket {
            audio_level_emits: emits,
            ..RateBucket::default()
        };
        // The first call only starts the clock and discards what it finds.
        assert_eq!(pacer.close(5_000_000, || busy(99)), None);
        assert_eq!(pacer.close(5_500_000, || busy(3)), None);
        let logged = pacer
            .close(6_000_000, || busy(12))
            .expect("a second of audio");
        assert_eq!(logged.audio_level_emits, 12);
        assert_eq!(logged.window_us, 1_000_000);
        // Idle seconds with a few invokes stay quiet.
        let quiet = |invokes| RateBucket {
            ipc_invokes: invokes,
            ..RateBucket::default()
        };
        assert_eq!(
            pacer.close(7_000_000, || quiet(RATE_IDLE_MIN_INVOKES - 1)),
            None
        );
        // A burst of invokes while idle logs, at most once per idle interval.
        assert!(pacer.close(8_000_000, || quiet(40)).is_some());
        assert_eq!(pacer.close(9_000_000, || quiet(40)), None);
        assert!(pacer
            .close(8_000_000 + RATE_IDLE_LOG_US, || quiet(40))
            .is_some());
    }

    #[test]
    fn counters_are_swapped_to_zero_by_the_reporter() {
        let _guard = enable_for_test();
        take_rate_counters();
        count_audio_level_emit();
        count_audio_level_emit();
        IPC_INVOKES.fetch_add(5, Ordering::Relaxed);
        let bucket = take_rate_counters();
        assert_eq!(bucket.audio_level_emits, 2);
        assert_eq!(bucket.ipc_invokes, 5);
        assert_eq!(take_rate_counters(), RateBucket::default());
    }

    #[test]
    fn an_applied_marker_is_recorded_only_when_it_waited() {
        let _guard = enable_for_test();
        applied::record_marker("applied.test.fast", 100, 100 + APPLIED_RECORD_MIN_US - 1);
        applied::record_marker("applied.test.slow", 100, 100 + 40_000);
        let spans = drain_ring();
        assert!(spans.iter().all(|s| s.label != "applied.test.fast"));
        let slow = spans
            .iter()
            .find(|s| s.label == "applied.test.slow")
            .expect("slow marker");
        assert_eq!(slow.queue_us, 40_000);
        assert_eq!(slow.duration_us, 0);
        assert!(is_safe_label(slow.label));
        // A marker that waited 20 ms is logged on its own, like a slow job.
        assert!(should_log_standalone(slow) == (slow.queue_us >= LOG_QUEUE_MIN_US));
    }

    #[test]
    fn every_applied_label_is_a_safe_static_identifier() {
        for label in [
            "applied.window.set_size",
            "applied.window.set_position",
            "applied.window.set_ignore_cursor_events",
            "applied.overlay.set_size",
            "applied.overlay.set_position",
            "applied.overlay.set_ignore_cursor_events",
            "overlay.detect_notch",
            "overlay.dock_prefs",
            "overlay.primary_monitor",
            "preview.follow_overlay",
            "overlay_material.apply",
            "overlay_material.install",
            "overlay_material.attach",
            "overlay_material.refit",
        ] {
            assert!(is_safe_label(label), "{label}");
        }
    }

    #[test]
    fn frame_text_is_image_bang_symbol_items_and_nothing_that_carries_content() {
        assert!(stack::is_safe_frames(
            "AppKit!-[NSView layout];ui!core::ops::function::FnOnce::call_once<T>;?!?"
        ));
        assert!(stack::is_safe_frames(
            "libsystem_kernel.dylib!mach_msg2_trap;AppKit!__99-[NSApplication(NSPersistentUIRestorationSupport) _restoreWindowWithRestoration:completionHandler:]_block_invoke.375"
        ));
        assert!(stack::is_safe_frames(
            "ui_lib-1138d6402c0ded59!_ZN2ui4main17h0123456789abcdefE"
        ));
        for bad in [
            "",
            ";",
            "ui!f;",
            ";ui!f",
            "ui!f;;ui!g",
            "no_bang",
            "!symbol",
            "image!",
            "ui!f; /Users/george/private.txt",
            "/Users/george/ui!f",
            "Some Image!f",
            "ui!two words",
            "ui!f [x] y",
            "ui!f\"secret\"",
            "ui!f=secret",
            "ui!f\nnext",
            "ui!f\u{00e9}",
            "ui!f%20",
        ] {
            assert!(!stack::is_safe_frames(bad), "{bad:?}");
        }
        assert!(!stack::is_safe_frames(&format!(
            "ui!{}",
            "a".repeat(stack::MAX_FRAMES_LEN)
        )));
    }

    #[test]
    fn symbolised_frames_are_bounded_names_without_paths() {
        let addresses = [
            symbolised_frames_are_bounded_names_without_paths as *const () as usize as u64,
            1,
            u64::MAX,
        ];
        let text = stack::symbolise(&addresses);
        assert!(stack::is_safe_frames(&text), "{text}");
        assert!(!text.contains('/'), "{text}");
        assert_eq!(text.split(';').count(), addresses.len());
        assert_eq!(stack::symbolise(&[]), "?");
        let many = vec![
            symbolised_frames_are_bounded_names_without_paths as *const () as usize
                as u64;
            200
        ];
        assert!(stack::symbolise(&many).len() <= stack::MAX_FRAMES_LEN);
    }

    /// Sample a thread that is spinning in this process and check the result is
    /// bounded, named, and quick. The sampled thread keeps running afterwards.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn a_running_thread_can_be_sampled_without_holding_it() {
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;

        #[inline(never)]
        fn spin_here_for_the_sampler(stop: &AtomicBool, ticks: &AtomicU64) {
            while !stop.load(Ordering::Relaxed) {
                ticks.fetch_add(1, Ordering::Relaxed);
                std::hint::spin_loop();
            }
        }

        let stop = Arc::new(AtomicBool::new(false));
        let ticks = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel();
        let worker = {
            let (stop, ticks) = (stop.clone(), ticks.clone());
            std::thread::spawn(move || {
                tx.send(stack::Target::current().expect("target")).unwrap();
                spin_here_for_the_sampler(&stop, &ticks);
            })
        };
        let target = rx.recv().unwrap();
        while ticks.load(Ordering::Relaxed) == 0 {
            std::thread::yield_now();
        }
        let mut best_held_us = u64::MAX;
        let mut frames_seen = 0;
        for _ in 0..50 {
            let mut raw = [0u64; stack::MAX_FRAMES];
            let captured = stack::capture(target, &mut raw);
            best_held_us = best_held_us.min(captured.held_us);
            frames_seen = frames_seen.max(captured.frames);
            if captured.frames > 0 {
                let text = stack::symbolise(&raw[..captured.frames]);
                assert!(stack::is_safe_frames(&text), "{text}");
                assert!(captured.frames <= stack::MAX_FRAMES);
            }
        }
        assert!(frames_seen >= 2, "frames {frames_seen}");
        // The usual hold is microseconds; the limit leaves room for a loaded CI host.
        assert!(best_held_us < 5_000, "held {best_held_us} us");
        // The sampled thread was resumed: it is still counting.
        let before = ticks.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(20));
        assert!(ticks.load(Ordering::Relaxed) > before);
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn a_sample_names_the_function_the_thread_is_running() {
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;

        #[inline(never)]
        fn spin_in_a_named_function(stop: &AtomicBool) {
            while !stop.load(Ordering::Relaxed) {
                std::hint::black_box(0u64);
            }
        }

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let worker = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                tx.send(stack::Target::current().expect("target")).unwrap();
                spin_in_a_named_function(&stop);
            })
        };
        let target = rx.recv().unwrap();
        let mut named = false;
        for _ in 0..200 {
            let mut raw = [0u64; stack::MAX_FRAMES];
            let captured = stack::capture(target, &mut raw);
            if captured.frames > 0
                && stack::symbolise(&raw[..captured.frames]).contains("spin_in_a_named_function")
            {
                named = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        assert!(named, "no sample contained the spinning function");
    }
    #[test]
    fn the_reporter_sleeps_long_when_idle_and_arms_the_50ms_check_when_busy() {
        let idle = Duration::from_millis(200);
        let mut pacer = StackPacer::default();
        // Waiting main thread, sampling off, or sampler disabled: idle.
        assert_eq!(pacer.next_wait(0, 1_000_000, true, idle), idle);
        assert_eq!(pacer.next_wait(1_000_000, 1_000_010, false, idle), idle);
        // A stretch that just began: sleep until its 50 ms mark, not 200 ms.
        let since = 5_000_000;
        assert_eq!(
            pacer.next_wait(since, since + 100, true, idle),
            Duration::from_micros(STACK_FIRST_SAMPLE_US - 100)
        );
        // Already past the mark (the wake was late): the minimum sleep.
        assert_eq!(
            pacer.next_wait(since, since + 80_000, true, idle),
            Duration::from_micros(WATCH_POLL_MIN_US)
        );
        // Never sleeps past a drain that is nearer.
        assert_eq!(
            pacer.next_wait(since, since + 10, true, Duration::from_millis(5)),
            Duration::from_millis(5)
        );
        // After sample 1 the next point is 250 ms, but the poll stays at 50 ms.
        assert_eq!(pacer.due(since, since + STACK_FIRST_SAMPLE_US), Some(1));
        assert!(pacer.admit(1, since + STACK_FIRST_SAMPLE_US));
        assert_eq!(
            pacer.next_wait(since, since + STACK_FIRST_SAMPLE_US + 10, true, idle),
            Duration::from_micros(WATCH_POLL_MAX_US)
        );
        assert_eq!(
            pacer.next_wait(since, since + STACK_SECOND_SAMPLE_US - 5_000, true, idle),
            Duration::from_micros(5_000)
        );
        // After sample 2 there is nothing left to watch for in this stretch.
        assert_eq!(pacer.due(since, since + STACK_SECOND_SAMPLE_US), Some(2));
        assert!(pacer.admit(2, since + STACK_SECOND_SAMPLE_US));
        assert_eq!(pacer.next_wait(since, since + 400_000, true, idle), idle);
        // A new stretch starts over at the 50 ms mark.
        let next = since + 900_000;
        assert_eq!(
            pacer.next_wait(next, next + 1_000, true, idle),
            Duration::from_micros(STACK_FIRST_SAMPLE_US - 1_000)
        );
        // Once disabled, always idle.
        pacer.disabled = true;
        assert_eq!(pacer.next_wait(next, next + 1_000, true, idle), idle);
        // Every watch wait is well under 100 ms.
        let fresh = StackPacer::default();
        for busy in [0, 1, 10_000, 49_999, 50_000, 90_000] {
            let wait = fresh.next_wait(9_000_000, 9_000_000 + busy, true, idle);
            assert!(wait <= Duration::from_micros(WATCH_POLL_MAX_US), "{busy}");
        }
    }

    #[test]
    fn the_reporter_is_woken_only_while_parked_and_only_once() {
        use std::sync::mpsc;
        let (to_main, from_worker) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = REPORTER_THREAD.set(std::thread::current());
            // Phase 1: the flag is clear. A wake call must not unpark us.
            REPORTER_PARKED.store(false, Ordering::SeqCst);
            to_main.send(()).unwrap();
            let started = Instant::now();
            std::thread::park_timeout(Duration::from_millis(900));
            let unflagged = started.elapsed();
            // Phase 2: the flag is set. A wake call unparks us, once.
            REPORTER_PARKED.store(true, Ordering::SeqCst);
            to_main.send(()).unwrap();
            let started = Instant::now();
            std::thread::park_timeout(Duration::from_secs(10));
            (unflagged, started.elapsed())
        });
        from_worker.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        wake_reporter_if_parked();
        assert!(!REPORTER_PARKED.load(Ordering::SeqCst));
        from_worker.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        wake_reporter_if_parked();
        assert!(!REPORTER_PARKED.load(Ordering::SeqCst));
        // A second call finds the flag clear and does nothing.
        wake_reporter_if_parked();
        let (unflagged, flagged) = worker.join().unwrap();
        assert!(
            unflagged >= Duration::from_millis(800),
            "woken with the flag clear after {unflagged:?}"
        );
        assert!(flagged < Duration::from_secs(5), "{flagged:?}");
    }

    #[test]
    fn sampling_off_sleeps_to_the_drain_and_never_asks_to_be_woken() {
        let idle = Duration::from_millis(200);
        let since = 4_000_000;
        // Sampling not installed (no target, or the kill switch): plain sleep,
        // busy or not.
        let pacer = StackPacer::default();
        assert_eq!(
            pacer.park_plan(0, 5_000_000, false, idle),
            ParkPlan::Sleep(idle)
        );
        assert_eq!(
            pacer.park_plan(since, since + 10, false, idle),
            ParkPlan::Sleep(idle)
        );
        // Disabled by a long hold: plain sleep too.
        let mut disabled = StackPacer::default();
        disabled.apply(judge_capture(STACK_MAX_HOLD_US + 1, 3, 10, 10));
        assert_eq!(
            disabled.park_plan(since, since + 10, true, idle),
            ParkPlan::Sleep(idle)
        );
        assert_eq!(
            disabled.park_plan(0, 5_000_000, true, idle),
            ParkPlan::Sleep(idle)
        );
        // Sampling on: idle parks ask to be woken, a busy stretch is watched.
        assert_eq!(
            pacer.park_plan(0, 5_000_000, true, idle),
            ParkPlan::Idle(idle)
        );
        assert_eq!(
            pacer.park_plan(since, since + 100, true, idle),
            ParkPlan::Watch(Duration::from_micros(STACK_FIRST_SAMPLE_US - 100))
        );
    }

    #[test]
    fn a_hold_over_2ms_stops_sampling_and_a_stale_stretch_is_dropped() {
        let ok = STACK_MAX_HOLD_US;
        // Normal: same stretch, frames read, short hold.
        assert_eq!(
            judge_capture(7, 12, 1_000, 1_000),
            CaptureVerdict {
                stop_sampling: false,
                publish: true
            }
        );
        // Exactly at the limit still counts as bounded.
        assert!(!judge_capture(ok, 5, 1_000, 1_000).stop_sampling);
        // A hold over the limit stops sampling, and the sample is still logged
        // (even if the stretch ended meanwhile) so the stop shows in the log.
        assert_eq!(
            judge_capture(ok + 1, 5, 1_000, 1_000),
            CaptureVerdict {
                stop_sampling: true,
                publish: true
            }
        );
        assert_eq!(
            judge_capture(ok + 1, 5, 1_000, 0),
            CaptureVerdict {
                stop_sampling: true,
                publish: true
            }
        );
        // Over the limit with nothing read: stop, but nothing to log.
        assert_eq!(
            judge_capture(50_000, 0, 1_000, 1_000),
            CaptureVerdict {
                stop_sampling: true,
                publish: false
            }
        );
        // The stretch ended (0) or was replaced while reading: dropped.
        assert!(!judge_capture(7, 12, 1_000, 0).publish);
        assert!(!judge_capture(7, 12, 1_000, 2_000).publish);
        // Nothing read: dropped.
        assert!(!judge_capture(7, 0, 1_000, 1_000).publish);
    }

    #[test]
    fn a_hold_over_2ms_sets_disabled_and_a_short_hold_does_not() {
        let idle = Duration::from_millis(200);
        let mut pacer = StackPacer::default();
        pacer.apply(judge_capture(STACK_MAX_HOLD_US, 3, 10, 10));
        assert!(!pacer.disabled);
        pacer.apply(judge_capture(STACK_MAX_HOLD_US + 1, 3, 10, 10));
        assert!(pacer.disabled);
        // Disabled stays disabled, and the reporter then sleeps idle.
        pacer.apply(judge_capture(1, 3, 10, 10));
        assert!(pacer.disabled);
        assert_eq!(pacer.next_wait(10, 20, true, idle), idle);
        // A capture that read nothing but held too long also disables.
        let mut empty = StackPacer::default();
        empty.apply(judge_capture(STACK_MAX_HOLD_US + 1, 0, 10, 10));
        assert!(empty.disabled);
    }

    #[test]
    fn the_kill_switch_reads_its_value() {
        use std::ffi::OsStr;
        assert!(!stacks_disabled_by_env(None));
        assert!(stacks_disabled_by_env(Some(OsStr::new("1"))));
        assert!(stacks_disabled_by_env(Some(OsStr::new("true"))));
        assert!(stacks_disabled_by_env(Some(OsStr::new(" TRUE "))));
        for off in ["0", "", "false", "no", "2"] {
            assert!(!stacks_disabled_by_env(Some(OsStr::new(off))), "{off:?}");
        }
    }

    #[test]
    fn an_applied_marker_is_queued_only_after_the_setter_succeeded() {
        let _guard = enable_for_test();
        let before = applied::MARKERS_QUEUED.load(Ordering::Relaxed);
        let ok: Result<u8, &str> = queued_section("test.setter", "applied.test.setter", || Ok(7));
        assert_eq!(ok, Ok(7));
        assert_eq!(applied::MARKERS_QUEUED.load(Ordering::Relaxed), before + 1);
        let failed: Result<u8, &str> =
            queued_section("test.setter", "applied.test.setter", || Err("closed"));
        assert_eq!(failed, Err("closed"));
        assert_eq!(applied::MARKERS_QUEUED.load(Ordering::Relaxed), before + 1);
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn warming_up_leaves_the_calling_thread_running_and_sampleable() {
        use std::sync::atomic::AtomicBool;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                stack::warm_up();
                stack::warm_up();
                tx.send(stack::Target::current().expect("target")).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    std::hint::spin_loop();
                }
            })
        };
        let target = rx.recv().unwrap();
        let mut raw = [0u64; stack::MAX_FRAMES];
        let captured = stack::capture(target, &mut raw);
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        assert!(captured.frames > 0);
    }
}
