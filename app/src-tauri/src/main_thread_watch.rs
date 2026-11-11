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

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
const REPORTER_TICK: Duration = Duration::from_millis(200);
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

struct Ring {
    slots: Box<[Span]>,
    head: usize,
    len: usize,
    dropped: u64,
}

impl Ring {
    fn new() -> Self {
        Self {
            slots: vec![Span::EMPTY; RING_CAPACITY].into_boxed_slice(),
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, span: Span) {
        let capacity = self.slots.len();
        let index = (self.head + self.len) % capacity;
        self.slots[index] = span;
        if self.len == capacity {
            self.head = (self.head + 1) % capacity;
            self.dropped += 1;
        } else {
            self.len += 1;
        }
    }

    fn drain_into(&mut self, out: &mut Vec<Span>) -> u64 {
        let capacity = self.slots.len();
        for offset in 0..self.len {
            out.push(self.slots[(self.head + offset) % capacity]);
        }
        self.head = 0;
        self.len = 0;
        std::mem::take(&mut self.dropped)
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<(Instant, u64)> = OnceLock::new();
static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
static PENDING_PASTES: Mutex<Vec<Arc<PasteState>>> = Mutex::new(Vec::new());
/// Start of the run-loop busy span in progress (µs since epoch), or 0 while
/// the main thread is waiting. Read by the reporter for an unclosed span.
static BUSY_SINCE_US: AtomicU64 = AtomicU64::new(0);
/// The last few live-preview hides that ran: (recording id, start µs).
static PREVIEW_HIDES: Mutex<[(u64, u64); 8]> = Mutex::new([(0, 0); 8]);

thread_local! {
    static IS_MAIN: Cell<bool> = const { Cell::new(false) };
    static COMMAND_NAMES: RefCell<std::collections::HashSet<&'static str>> =
        RefCell::new(std::collections::HashSet::new());
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

fn record(span: Span) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(ring) = RING.get() {
        ring.lock().unwrap_or_else(|p| p.into_inner()).push(span);
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

const MAX_COMMAND_NAMES: usize = 512;
const MAX_COMMAND_NAME_LEN: usize = 64;

/// Record the run time of every Tauri command the invoke handler executes on
/// the main thread. Only names the handler accepted are interned, so labels
/// are registered command identifiers, never caller-supplied text.
pub(crate) fn instrument_invoke_handler<R, H>(
    handler: H,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
where
    R: tauri::Runtime,
    H: Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke| {
        if !ENABLED.load(Ordering::Relaxed) || !is_main() {
            return handler(invoke);
        }
        let name = invoke.message.command();
        let known = COMMAND_NAMES.with(|names| names.borrow().get(name).copied());
        let mut copy = [0u8; MAX_COMMAND_NAME_LEN];
        let copied = if known.is_none() && name.len() <= MAX_COMMAND_NAME_LEN {
            copy[..name.len()].copy_from_slice(name.as_bytes());
            Some(name.len())
        } else {
            None
        };
        let start = now_us();
        let handled = handler(invoke);
        let end = now_us();
        let label = known
            .or_else(|| {
                let len = copied.filter(|_| handled)?;
                let name = std::str::from_utf8(&copy[..len]).ok()?;
                intern_command_name(name)
            })
            .unwrap_or("command.other");
        record_simple(Kind::Command, label, start, end, 0);
        handled
    }
}

fn intern_command_name(name: &str) -> Option<&'static str> {
    if !is_safe_label(name) {
        return None;
    }
    COMMAND_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        if let Some(existing) = names.get(name) {
            return Some(*existing);
        }
        if names.len() >= MAX_COMMAND_NAMES {
            return None;
        }
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        names.insert(leaked);
        Some(leaked)
    })
}

/// Labels are static identifiers: lowercase ASCII, digits, `_`, `.`.
pub(crate) fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_COMMAND_NAME_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Record that the live-preview hide for `recording_id` started now. Called
/// on the main thread inside the hide job; overwrites the oldest entry.
pub(crate) fn note_preview_hide(recording_id: u64) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let start = now_us();
    let mut hides = PREVIEW_HIDES.lock().unwrap_or_else(|p| p.into_inner());
    let slot = hides
        .iter()
        .position(|(id, _)| *id == recording_id)
        .unwrap_or_else(|| {
            hides
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(index, _)| index)
                .unwrap_or(0)
        });
    hides[slot] = (recording_id, start);
}

/// Where the preview hide for this paste ran: before the paste was queued,
/// while it waited, after it started, or not at all.
fn preview_hide_order(recording_id: u64, queued_us: u64, started_us: u64) -> &'static str {
    let hides = PREVIEW_HIDES.lock().unwrap_or_else(|p| p.into_inner());
    classify_preview_hide(
        hides
            .iter()
            .find(|(id, _)| *id == recording_id)
            .map(|(_, at)| *at),
        queued_us,
        started_us,
    )
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
    let _ = RING.set(Mutex::new(Ring::new()));
    #[cfg(target_os = "macos")]
    let observers_installed = runloop::install();
    #[cfg(not(target_os = "macos"))]
    let observers_installed = false;
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
        "main-thread watch started"
    );
}

struct Reporter {
    history: VecDeque<Span>,
    scratch: Vec<Span>,
    standalone_window_start_us: u64,
    standalone_in_window: u32,
    suppressed: u64,
    dropped: u64,
}

fn reporter_loop() {
    let mut reporter = Reporter {
        history: VecDeque::with_capacity(RING_CAPACITY),
        scratch: Vec::with_capacity(RING_CAPACITY),
        standalone_window_start_us: 0,
        standalone_in_window: 0,
        suppressed: 0,
        dropped: 0,
    };
    loop {
        std::thread::sleep(REPORTER_TICK);
        reporter.tick();
    }
}

impl Reporter {
    fn tick(&mut self) {
        let Some(ring) = RING.get() else {
            return;
        };
        self.scratch.clear();
        let dropped = ring
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain_into(&mut self.scratch);
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
                BUSY_SINCE_US.store(now, Ordering::Release);
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
        let _ = RING.set(Mutex::new(Ring::new()));
        ENABLED.store(true, Ordering::Release);
        drain_ring();
        guard
    }

    pub(super) fn drain_ring() -> Vec<Span> {
        let mut out = Vec::new();
        if let Some(ring) = RING.get() {
            ring.lock().unwrap().drain_into(&mut out);
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
    fn ring_overwrites_oldest_and_counts_drops() {
        let mut ring = Ring::new();
        for i in 0..(RING_CAPACITY as u64 + 3) {
            ring.push(span(Kind::Job, "a", i, i + 1));
        }
        let mut out = Vec::new();
        assert_eq!(ring.drain_into(&mut out), 3);
        assert_eq!(out.len(), RING_CAPACITY);
        assert_eq!(out[0].start_us, 3);
        assert_eq!(ring.drain_into(&mut out), 0);
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
        note_preview_hide(91);
        for id in 100..110 {
            note_preview_hide(id);
        }
        let hides = PREVIEW_HIDES.lock().unwrap();
        assert!(hides.iter().any(|(id, _)| *id == 109));
        assert_eq!(hides.iter().filter(|(id, _)| *id == 109).count(), 1);
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
}
