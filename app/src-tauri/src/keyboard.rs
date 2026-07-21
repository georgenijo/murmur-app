//! Keyboard event detection using rdev for low-level keyboard events.
//!
//! Two detection modes sharing a single rdev listener thread:
//!
//! **Double-tap mode** (to start/stop recording):
//!   Start: Idle → WaitingFirstUp → WaitingSecondDown → WaitingSecondUp → FIRE
//!   Stop:  Idle → WaitingFirstUp → FIRE on release (single tap)
//!
//! **Hold-down mode** (to start/stop recording):
//!   Start: Idle → KeyPress(target) → Held (emit start)
//!   Stop:  Held → KeyRelease(target) → Idle (emit stop)
//!
//! Both modes reject modifier+letter combos (e.g. Shift+A).

use crate::MutexExt;
#[cfg(target_os = "macos")]
use rdev::set_is_main_thread;
use rdev::{listen, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

/// Max duration a single tap can be held before it's rejected
const MAX_HOLD_DURATION_MS: u128 = 200;

/// Max gap between first key-up and second key-down
const DOUBLE_TAP_WINDOW_MS: u128 = 400;

/// Cooldown after firing to prevent triple-tap spam
const COOLDOWN_MS: u128 = 50;

/// Cooldown after hold-down stop to prevent accidental re-trigger
const HOLD_DOWN_COOLDOWN_MS: u128 = 50;

/// Warn if the active listener sees no callbacks for this long.
const TAP_SILENCE_WARNING_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RejectionReason {
    HeldTooLong,
    SecondTapExpired,
    ComboCancelled,
    SingleShortTapNoop,
    ProcessingSkipped,
}

impl RejectionReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::HeldTooLong => "held_too_long",
            Self::SecondTapExpired => "second_tap_expired",
            Self::ComboCancelled => "combo_cancelled",
            Self::SingleShortTapNoop => "single_short_tap_noop",
            Self::ProcessingSkipped => "processing_skipped",
        }
    }
}

struct DoubleTapDetector {
    state: DetectorState,
    target_key: Option<Key>,
    recording: bool,
    state_entered_at: Instant,
    last_fired_at: Option<Instant>,
    last_rejection: Option<RejectionReason>,
}

impl DoubleTapDetector {
    fn new() -> Self {
        Self {
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: Instant::now(),
            last_fired_at: None,
            last_rejection: None,
        }
    }

    fn set_target(&mut self, key: Option<Key>) {
        self.target_key = key;
        self.reset();
    }

    fn reset(&mut self) {
        self.state = DetectorState::Idle;
        self.state_entered_at = Instant::now();
    }

    fn transition(&mut self, new_state: DetectorState) {
        self.state = new_state;
        self.state_entered_at = Instant::now();
    }

    fn elapsed_ms(&self) -> u128 {
        self.state_entered_at.elapsed().as_millis()
    }

    fn in_cooldown(&self) -> bool {
        self.last_fired_at
            .map(|t| t.elapsed().as_millis() < COOLDOWN_MS)
            .unwrap_or(false)
    }

    fn log_rejection(&mut self, reason: RejectionReason, event_type: &EventType) {
        self.last_rejection = Some(reason);
        tracing::info!(
            target: "keyboard",
            detector = "double_tap",
            reason = reason.as_str(),
            state = ?self.state,
            elapsed_ms = self.elapsed_ms() as u64,
            recording = self.recording,
            target_key = ?self.target_key,
            event_type = ?event_type,
            "keyboard detector rejected sequence"
        );
    }

    /// Process a keyboard event. Returns true if a double-tap was detected.
    fn handle_event(&mut self, event_type: &EventType) -> bool {
        self.last_rejection = None;
        let target = match self.target_key {
            Some(k) => k,
            None => return false,
        };

        if self.in_cooldown() {
            return false;
        }

        match self.state {
            DetectorState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_same_modifier(*key, target) {
                        self.transition(DetectorState::WaitingFirstUp);
                    }
                }
                false
            }

            DetectorState::WaitingFirstUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            if self.recording {
                                // Single tap to stop — fire immediately
                                self.last_fired_at = Some(Instant::now());
                                self.reset();
                                return true;
                            }
                            self.transition(DetectorState::WaitingSecondDown);
                        } else {
                            // Held too long — not a tap
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat event — ignore, stay in same state
                        // But check if we've been held too long
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    _ => {
                        // Check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                }
                false
            }

            DetectorState::WaitingSecondDown => {
                if self.elapsed_ms() > DOUBLE_TAP_WINDOW_MS {
                    self.log_rejection(RejectionReason::SecondTapExpired, event_type);
                    self.reset();
                    return false;
                }
                match event_type {
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        self.transition(DetectorState::WaitingSecondUp);
                    }
                    EventType::KeyPress(_) => {
                        // Any other key press — abort
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.reset();
                    }
                    _ => {}
                }
                false
            }

            DetectorState::WaitingSecondUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            // Double-tap detected!
                            self.last_fired_at = Some(Instant::now());
                            self.reset();
                            return true;
                        } else {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // Combo like Shift+A on second press
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    _ => {
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                }
                false
            }
        }
    }

    fn take_rejection(&mut self) -> Option<RejectionReason> {
        self.last_rejection.take()
    }

    fn second_tap_wait_started_at(&self) -> Option<Instant> {
        (self.state == DetectorState::WaitingSecondDown).then_some(self.state_entered_at)
    }

    fn expire_second_tap_wait(&mut self, started_at: Instant) -> Option<u64> {
        if self.state != DetectorState::WaitingSecondDown
            || self.state_entered_at != started_at
            || self.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS
        {
            return None;
        }

        let elapsed_ms = self.elapsed_ms() as u64;
        self.reset();
        Some(elapsed_ms)
    }
}

/// Check if a key is any modifier key
fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::Alt
            | Key::AltGr
            | Key::ControlLeft
            | Key::ControlRight
            | Key::MetaLeft
            | Key::MetaRight
    )
}

/// Check if two keys are the same modifier, using strict equality
fn is_same_modifier(a: Key, b: Key) -> bool {
    a == b
}

// -- Hold-down detector --

#[derive(Debug, Clone, Copy, PartialEq)]
enum HoldDownEvent {
    None,
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HoldState {
    Idle,
    Held,
}

struct HoldDownDetector {
    state: HoldState,
    target_key: Option<Key>,
    last_stopped_at: Option<Instant>,
}

impl HoldDownDetector {
    fn new() -> Self {
        Self {
            state: HoldState::Idle,
            target_key: None,
            last_stopped_at: None,
        }
    }

    /// Set the target key. Returns `true` if the detector was in `Held` state
    /// (i.e. the caller should emit a stop event to the frontend).
    fn set_target(&mut self, key: Option<Key>) -> bool {
        let was_held = self.state == HoldState::Held;
        if was_held {
            self.state = HoldState::Idle;
            self.last_stopped_at = Some(Instant::now());
        }
        self.target_key = key;
        was_held
    }

    fn reset(&mut self) {
        self.state = HoldState::Idle;
    }

    fn in_cooldown(&self) -> bool {
        self.last_stopped_at
            .map(|t| t.elapsed().as_millis() < HOLD_DOWN_COOLDOWN_MS)
            .unwrap_or(false)
    }

    fn log_rejection(&self, reason: RejectionReason, event_type: &EventType) {
        tracing::info!(
            target: "keyboard",
            detector = "hold_down",
            reason = reason.as_str(),
            state = ?self.state,
            target_key = ?self.target_key,
            event_type = ?event_type,
            "keyboard detector rejected sequence"
        );
    }

    /// Process a keyboard event. Returns Start, Stop, or None.
    fn handle_event(&mut self, event_type: &EventType) -> HoldDownEvent {
        let target = match self.target_key {
            Some(k) => k,
            None => return HoldDownEvent::None,
        };

        match self.state {
            HoldState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_same_modifier(*key, target) && !self.in_cooldown() {
                        self.state = HoldState::Held;
                        return HoldDownEvent::Start;
                    }
                }
                HoldDownEvent::None
            }

            HoldState::Held => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(Instant::now());
                        HoldDownEvent::Stop
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — ignore, stay held
                        HoldDownEvent::None
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A — cancel hold
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(Instant::now());
                        HoldDownEvent::Stop
                    }
                    _ => HoldDownEvent::None,
                }
            }
        }
    }
}

// -- Shared types --

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectorMode {
    DoubleTap,
    HoldDown,
    Both,
}

impl DetectorMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::DoubleTap => "double_tap",
            Self::HoldDown => "hold_down",
            Self::Both => "both",
        }
    }
}

fn should_surface_hotkey_rejection(reason: RejectionReason, mode: DetectorMode) -> bool {
    reason == RejectionReason::SecondTapExpired
        && matches!(mode, DetectorMode::DoubleTap | DetectorMode::Both)
}

fn emit_hotkey_rejection(
    app_handle: &tauri::AppHandle,
    reason: RejectionReason,
    mode: DetectorMode,
) {
    if !should_surface_hotkey_rejection(reason, mode) {
        return;
    }

    let _ = app_handle.emit(
        "hotkey-tap-rejected",
        serde_json::json!({
            "reason": reason.as_str(),
            "mode": mode.as_str(),
        }),
    );
}

fn schedule_second_tap_expiry(
    app_handle: tauri::AppHandle,
    mode: DetectorMode,
    listener_generation: u64,
    started_at: Instant,
) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(
            DOUBLE_TAP_WINDOW_MS as u64 + 5,
        ));

        if !listener_context_matches(mode, listener_generation) {
            return;
        }

        let elapsed_ms = {
            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
            det.as_mut()
                .and_then(|d| d.expire_second_tap_wait(started_at))
        };

        if let Some(elapsed_ms) =
            elapsed_ms.filter(|_| listener_context_matches(mode, listener_generation))
        {
            tracing::info!(
                target: "keyboard",
                detector = "double_tap",
                reason = RejectionReason::SecondTapExpired.as_str(),
                state = ?DetectorState::WaitingSecondDown,
                elapsed_ms,
                recording = false,
                mode = mode.as_str(),
                "keyboard detector rejected sequence"
            );
            emit_hotkey_rejection(&app_handle, RejectionReason::SecondTapExpired, mode);
        }
    });
}

fn listener_context_matches(mode: DetectorMode, generation: u64) -> bool {
    listener_context_is_current(
        LISTENER_ACTIVE.load(Ordering::SeqCst),
        *ACTIVE_MODE.lock_or_recover(),
        LISTENER_GENERATION.load(Ordering::SeqCst),
        mode,
        generation,
    )
}

fn listener_context_is_current(
    active: bool,
    active_mode: DetectorMode,
    active_generation: u64,
    expected_mode: DetectorMode,
    expected_generation: u64,
) -> bool {
    active && active_mode == expected_mode && active_generation == expected_generation
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Hotkey ids reserved for the dictation listener (`DoubleTapKey` in
/// settings.ts). The transform hotkey commands (`start_transform_listener`,
/// `set_transform_key` in `commands/keyboard.rs`) reject these so the two key
/// sets stay disjoint at the Rust boundary too, not just in the TS type —
/// `hotkey_to_rdev_key` below accepts either set with no id-ownership check
/// of its own.
pub const DICTATION_KEY_IDS: &[&str] = &["shift_l", "alt_l", "ctrl_r"];

/// Whether `hotkey` is one of the ids reserved for the dictation listener
/// (see `DICTATION_KEY_IDS`). Pure so it's unit-testable without a listener
/// or `tauri::AppHandle`.
pub fn is_dictation_key_id(hotkey: &str) -> bool {
    DICTATION_KEY_IDS.contains(&hotkey)
}

/// Map hotkey string from settings to rdev Key.
///
/// `shift_l` / `alt_l` / `ctrl_r` back the dictation hotkey (`DoubleTapKey` in
/// settings.ts). `alt_r` / `ctrl_l` / `shift_r` back the independent transform
/// hotkey (`TransformKey`, issue #312) — same function since both listeners
/// share this mapping. The two id sets are kept disjoint by convention plus
/// an explicit `is_dictation_key_id` check at the transform command boundary
/// (see `commands/keyboard.rs`) — nothing here would reject the overlap on
/// its own.
fn hotkey_to_rdev_key(hotkey: &str) -> Option<Key> {
    match hotkey {
        "shift_l" => Some(Key::ShiftLeft),
        "alt_l" => Some(Key::Alt),
        "ctrl_r" => Some(Key::ControlRight),
        "shift_r" => Some(Key::ShiftRight),
        "alt_r" => Some(Key::AltGr),
        "ctrl_l" => Some(Key::ControlLeft),
        _ => None,
    }
}

fn event_key(event_type: &EventType) -> Option<Key> {
    match event_type {
        EventType::KeyPress(key) | EventType::KeyRelease(key) => Some(*key),
        _ => None,
    }
}

fn event_kind(event_type: &EventType) -> &'static str {
    match event_type {
        EventType::KeyPress(_) => "key_press",
        EventType::KeyRelease(_) => "key_release",
        EventType::ButtonPress(_) => "button_press",
        EventType::ButtonRelease(_) => "button_release",
        EventType::MouseMove { .. } => "mouse_move",
        EventType::Wheel { .. } => "wheel",
    }
}

fn modifier_edge(event_type: &EventType) -> &'static str {
    match event_type {
        EventType::KeyPress(key) if is_modifier(*key) => "press",
        EventType::KeyRelease(key) if is_modifier(*key) => "release",
        _ => "not_modifier",
    }
}

fn detector_state_snapshot() -> (Option<DetectorState>, Option<HoldState>) {
    let double_tap_state = {
        let det = DOUBLE_TAP_DETECTOR.lock_or_recover();
        det.as_ref().map(|d| d.state)
    };
    let hold_state = {
        let det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        det.as_ref().map(|d| d.state)
    };
    (double_tap_state, hold_state)
}

fn trace_raw_callback(event: &Event, mode: DetectorMode) {
    let (double_tap_state, hold_state) = detector_state_snapshot();
    tracing::trace!(
        target: "keyboard",
        event_type = ?event.event_type,
        event_kind = event_kind(&event.event_type),
        key = ?event_key(&event.event_type),
        event_name = ?event.name,
        mode = ?mode,
        double_tap_state = ?double_tap_state,
        hold_state = ?hold_state,
        modifier_edge = modifier_edge(&event.event_type),
        is_processing = IS_PROCESSING.load(Ordering::SeqCst),
        app_disabled = APP_DISABLED.load(Ordering::SeqCst),
        "raw rdev callback"
    );
}

fn log_rejection(reason: RejectionReason, mode: DetectorMode, event_type: &EventType) {
    let (double_tap_state, hold_state) = detector_state_snapshot();
    tracing::info!(
        target: "keyboard",
        reason = reason.as_str(),
        mode = ?mode,
        event_type = ?event_type,
        key = ?event_key(event_type),
        double_tap_state = ?double_tap_state,
        hold_state = ?hold_state,
        "keyboard event rejected"
    );
}

// -- Both-mode arbitration state --

/// Monotonic counter to invalidate stale hold-promotion timers.
static HOLD_PRESS_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Set to true by the timer thread when it promotes a press to a real hold.
static HOLD_PROMOTED: AtomicBool = AtomicBool::new(false);
/// When true, the Both-mode callback ignores all key events.
/// Set by lib.rs when the transcription pipeline is running.
static IS_PROCESSING: AtomicBool = AtomicBool::new(false);

/// When true, the rdev callback ignores all keyboard events except Escape
/// (which still cancels in-progress recordings). Set by the `set_app_disabled`
/// Tauri command. Thread-safe, lock-free.
static APP_DISABLED: AtomicBool = AtomicBool::new(false);
static LAST_RDEV_CALLBACK_AT_MS: AtomicU64 = AtomicU64::new(0);
static LAST_TAP_SILENCE_WARNING_AT_MS: AtomicU64 = AtomicU64::new(0);

/// Called by lib.rs to tell the keyboard module whether the app is processing.
/// When transitioning out of processing, reset both detectors and apply a
/// cooldown so rapid post-processing taps don't immediately toggle.
#[track_caller]
pub fn set_processing(processing: bool) {
    let caller = std::panic::Location::caller();
    let was_processing = IS_PROCESSING.swap(processing, Ordering::SeqCst);
    tracing::info!(
        target: "keyboard",
        processing = processing,
        was_processing = was_processing,
        caller_file = caller.file(),
        caller_line = caller.line(),
        "set_processing"
    );
    if !was_processing && processing {
        // Entering processing: invalidate any pending hold-promotion timer
        // so it can't fire hold-down-start during active processing.
        HOLD_PROMOTED.store(false, Ordering::SeqCst);
        HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut det) = HOLD_DOWN_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
            }
        }
        if let Ok(mut det) = DOUBLE_TAP_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
            }
        }
    } else if was_processing && !processing {
        // Exiting processing: reset detectors with cooldown so rapid
        // post-processing taps don't immediately toggle.
        if let Ok(mut det) = HOLD_DOWN_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
                d.last_stopped_at = Some(Instant::now());
            }
        }
        if let Ok(mut det) = DOUBLE_TAP_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
                d.last_fired_at = Some(Instant::now());
            }
        }
        HOLD_PROMOTED.store(false, Ordering::SeqCst);
        HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
    }
}

/// Returns whether the app is currently in the processing state.
#[cfg(test)]
pub fn is_processing() -> bool {
    IS_PROCESSING.load(Ordering::SeqCst)
}

/// Set or clear the global disabled flag. When transitioning to disabled,
/// resets both detectors and invalidates any pending hold-promotion timer so
/// re-enabling doesn't produce phantom events.
pub fn set_app_disabled(disabled: bool) {
    let was_disabled = APP_DISABLED.swap(disabled, Ordering::SeqCst);
    if !was_disabled && disabled {
        // Transitioning false → true: clean up any partial detector state
        HOLD_PROMOTED.store(false, Ordering::SeqCst);
        HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut det) = HOLD_DOWN_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
            }
        }
        if let Ok(mut det) = DOUBLE_TAP_DETECTOR.lock() {
            if let Some(d) = det.as_mut() {
                d.reset();
            }
        }
        tracing::info!(target: "keyboard", "app disabled: hotkey events gated");
    } else if was_disabled && !disabled {
        tracing::info!(target: "keyboard", "app enabled: hotkey events resumed");
    }
}

/// Returns whether the app is currently disabled.
pub fn is_app_disabled() -> bool {
    APP_DISABLED.load(Ordering::SeqCst)
}

// -- Global listener state --

static LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static LISTENER_THREAD_SPAWNED: AtomicBool = AtomicBool::new(false);
static LISTENER_GENERATION: AtomicU64 = AtomicU64::new(0);

static ACTIVE_MODE: Mutex<DetectorMode> = Mutex::new(DetectorMode::DoubleTap);
static DOUBLE_TAP_DETECTOR: Mutex<Option<DoubleTapDetector>> = Mutex::new(None);
static HOLD_DOWN_DETECTOR: Mutex<Option<HoldDownDetector>> = Mutex::new(None);

// -- Transform hotkey (issue #312) --
//
// A second, independent hold-down detector for the "transform" shortcut
// (AX selected-text capture + LLM transform, PR-B1). It reuses the plain
// `HoldDownDetector` state machine but is entirely separate from the
// dictation detectors above: its own target key, its own Mutex, its own
// active flag. It is fed from the SAME shared rdev callback (see the
// `TRANSFORM_DETECTOR` handling block in `start_listener`'s callback) so no
// second `rdev::listen()` thread is ever spawned — rdev only tolerates one
// listener per process. Starting/stopping the transform listener never
// touches `DOUBLE_TAP_DETECTOR` / `HOLD_DOWN_DETECTOR` or `ACTIVE_MODE`.
static TRANSFORM_DETECTOR: Mutex<Option<HoldDownDetector>> = Mutex::new(None);
/// Gates transform-detector processing independent of `LISTENER_ACTIVE`
/// (which only reflects the dictation listener). Lets the transform hotkey
/// work even if, hypothetically, it were started before the dictation
/// listener.
static TRANSFORM_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Start the keyboard listener. Spawns the rdev listener thread if not already running.
/// If already running, just updates the target key, mode, and re-enables.
///
/// `mode` should be `"double_tap"`, `"hold_down"`, or `"both"`.
pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str, mode: &str) {
    LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);
    let target = hotkey_to_rdev_key(hotkey);

    let detector_mode = match mode {
        "hold_down" => DetectorMode::HoldDown,
        "both" => DetectorMode::Both,
        _ => DetectorMode::DoubleTap,
    };

    // Set active mode
    {
        let mut m = ACTIVE_MODE.lock().unwrap_or_else(|p| p.into_inner());
        *m = detector_mode;
    }

    // Initialize or update the appropriate detector(s)
    match detector_mode {
        DetectorMode::DoubleTap => {
            let mut det = DOUBLE_TAP_DETECTOR
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            match det.as_mut() {
                Some(d) => d.set_target(target),
                None => {
                    let mut d = DoubleTapDetector::new();
                    d.set_target(target);
                    *det = Some(d);
                }
            }
        }
        DetectorMode::HoldDown => {
            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            match det.as_mut() {
                Some(d) => {
                    let _ = d.set_target(target);
                }
                None => {
                    let mut d = HoldDownDetector::new();
                    let _ = d.set_target(target);
                    *det = Some(d);
                }
            }
        }
        DetectorMode::Both => {
            // Initialize both detectors with the same target key
            {
                let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                match det.as_mut() {
                    Some(d) => {
                        let _ = d.set_target(target);
                    }
                    None => {
                        let mut d = HoldDownDetector::new();
                        let _ = d.set_target(target);
                        *det = Some(d);
                    }
                }
            }
            {
                let mut det = DOUBLE_TAP_DETECTOR
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                match det.as_mut() {
                    Some(d) => d.set_target(target),
                    None => {
                        let mut d = DoubleTapDetector::new();
                        d.set_target(target);
                        *det = Some(d);
                    }
                }
            }
        }
    }

    LISTENER_ACTIVE.store(true, Ordering::SeqCst);
    LAST_RDEV_CALLBACK_AT_MS.store(now_unix_ms(), Ordering::SeqCst);
    LAST_TAP_SILENCE_WARNING_AT_MS.store(0, Ordering::SeqCst);

    ensure_listener_thread_spawned(app_handle);
}

/// Spawn the single shared `rdev::listen()` thread if it hasn't been spawned
/// yet (idempotent — rdev only tolerates one listener per process). Both the
/// dictation listener (`start_listener`) and the transform hotkey
/// (`start_transform_listener`) call this; whichever runs first wins the
/// spawn, the other is a no-op via the `compare_exchange` guard.
fn ensure_listener_thread_spawned(app_handle: tauri::AppHandle) {
    // Only spawn the thread once
    if LISTENER_THREAD_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // Two clones: one moves into the callback closure, one stays in the
        // outer thread closure for use after listen() returns with an error.
        let handle = app_handle.clone();
        let error_handle = app_handle.clone();
        std::thread::spawn(move || {
            // CRITICAL: rdev's keyboard translation calls TIS/TSM APIs that must
            // run on the main thread on macOS. This flag tells rdev to dispatch
            // those calls to the main queue via dispatch_sync instead of calling
            // them directly from this background thread.
            #[cfg(target_os = "macos")]
            set_is_main_thread(false);
            tracing::info!(target: "keyboard", "rdev listener thread started");

            let callback = move |event: Event| {
                // The dictation listener (LISTENER_ACTIVE) and the transform
                // hotkey (TRANSFORM_ACTIVE) are independent; either one being
                // active is enough to keep processing events on this thread.
                if !LISTENER_ACTIVE.load(Ordering::SeqCst) && !TRANSFORM_ACTIVE.load(Ordering::SeqCst)
                {
                    return;
                }
                LAST_RDEV_CALLBACK_AT_MS.store(now_unix_ms(), Ordering::SeqCst);
                LAST_TAP_SILENCE_WARNING_AT_MS.store(0, Ordering::SeqCst);

                let mode = {
                    let m = ACTIVE_MODE.lock_or_recover();
                    *m
                };
                let listener_generation = LISTENER_GENERATION.load(Ordering::SeqCst);
                trace_raw_callback(&event, mode);

                // Escape key: cancel recording/transcription regardless of mode.
                // Must be checked before mode-specific logic so it works even
                // during IS_PROCESSING (which gates the Both-mode block).
                if let EventType::KeyPress(Key::Escape) = event.event_type {
                    // Reset both detectors with cooldown timestamps so that the
                    // subsequent trigger-key release (if user was holding it) is
                    // treated as a no-op instead of firing hold-down-stop.
                    {
                        let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(d) = det.as_mut() {
                            d.reset();
                            d.last_stopped_at = Some(Instant::now());
                        }
                    }
                    {
                        let mut det = DOUBLE_TAP_DETECTOR
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        if let Some(d) = det.as_mut() {
                            d.reset();
                            d.last_fired_at = Some(Instant::now());
                        }
                    }
                    // Also reset the transform detector (issue #312) — this
                    // branch returns before the transform block below runs, so
                    // without this the detector could be left mid-hold (stale
                    // `Held` state) across an Escape, exactly like the
                    // dictation detectors above would be without their resets.
                    {
                        let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(d) = det.as_mut() {
                            d.reset();
                            d.last_stopped_at = Some(Instant::now());
                        }
                    }
                    // Invalidate pending hold-promotion timers
                    HOLD_PROMOTED.store(false, Ordering::SeqCst);
                    HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);

                    tracing::info!(target: "keyboard", "Escape pressed — emitting escape-cancel");
                    let _ = handle.emit("escape-cancel", ());
                    return;
                }

                if APP_DISABLED.load(Ordering::SeqCst) {
                    return;
                }

                // Transform hotkey (issue #312): an independent hold-down
                // detector fed unconditionally of `mode`/dictation state, since
                // it targets a distinct key from the dictation hotkey and has
                // its own start/stop lifecycle. Runs on every event so it keeps
                // working across dictation mode switches. Not gated by
                // IS_PROCESSING — the transform shortcut targets already-typed
                // text and its use is independent of live dictation.
                //
                // Gated by TRANSFORM_ACTIVE so that `set_transform_key` alone
                // (which arms TRANSFORM_DETECTOR with a target key but does not
                // start the listener) can never cause event emission — only
                // `start_transform_listener` flips TRANSFORM_ACTIVE. Without
                // this gate, a stray `set_transform_key` call before the
                // listener starts would let the detector fire silently.
                if TRANSFORM_ACTIVE.load(Ordering::SeqCst) {
                    let transform_result = {
                        let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(d) = det.as_mut() {
                            d.handle_event(&event.event_type)
                        } else {
                            HoldDownEvent::None
                        }
                    };
                    match transform_result {
                        HoldDownEvent::Start => {
                            // A new transform pass invalidates any stale session
                            // (issue #312 PR-B2) left over from a previous
                            // capture/review/apply — this is the earliest point
                            // in the transform lifecycle, before capture even runs.
                            crate::transform_apply::clear_session(
                                &handle.state::<crate::State>().app_state,
                            );
                            let _ = handle.emit("transform-key-pressed", ());
                        }
                        HoldDownEvent::Stop => {
                            let _ = handle.emit("transform-key-released", ());
                        }
                        HoldDownEvent::None => {}
                    }
                }

                // The dictation dispatch below is only relevant while the
                // dictation listener itself is active (it may be false here if
                // only the transform hotkey brought this callback past the top
                // gate).
                if !LISTENER_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }

                match mode {
                    DetectorMode::DoubleTap => {
                        let (fired, rejection, wait_started_at) = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
                            if let Some(d) = det.as_mut() {
                                let previous_wait = d.second_tap_wait_started_at();
                                let fired = d.handle_event(&event.event_type);
                                let wait_started_at = d
                                    .second_tap_wait_started_at()
                                    .filter(|started_at| Some(*started_at) != previous_wait);
                                (fired, d.take_rejection(), wait_started_at)
                            } else {
                                (false, None, None)
                            }
                        };
                        if let Some(reason) = rejection {
                            emit_hotkey_rejection(&handle, reason, mode);
                        }
                        if let Some(started_at) = wait_started_at {
                            schedule_second_tap_expiry(
                                handle.clone(),
                                mode,
                                listener_generation,
                                started_at,
                            );
                        }
                        if fired {
                            let _ = handle.emit("double-tap-toggle", ());
                        }
                    }
                    DetectorMode::HoldDown => {
                        let result = {
                            let mut det =
                                HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                HoldDownEvent::None
                            }
                        };
                        match result {
                            HoldDownEvent::Start => {
                                let _ = handle.emit("hold-down-start", ());
                            }
                            HoldDownEvent::Stop => {
                                let _ = handle.emit("hold-down-stop", ());
                            }
                            HoldDownEvent::None => {}
                        }
                    }
                    DetectorMode::Both => {
                        // Skip all events while the app is processing a transcription.
                        if IS_PROCESSING.load(Ordering::SeqCst) {
                            log_rejection(
                                RejectionReason::ProcessingSkipped,
                                mode,
                                &event.event_type,
                            );
                            return;
                        }

                        // Deferred hold: on press, start a background timer.
                        // After MAX_HOLD_DURATION_MS, if the key is still held,
                        // the timer emits hold-down-start (promoting to a real hold).
                        // Short taps never start recording → no state thrash during double-tap.

                        // Check dtap phase BEFORE feeding — also verify the window hasn't expired
                        let dtap_second_phase = {
                            let det = DOUBLE_TAP_DETECTOR
                                .lock()
                                .unwrap_or_else(|p| p.into_inner());
                            det.as_ref()
                                .map(|d| {
                                    matches!(
                                        d.state,
                                        DetectorState::WaitingSecondDown
                                            | DetectorState::WaitingSecondUp
                                    ) && d.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS
                                })
                                .unwrap_or(false)
                        };

                        // Only feed hold-down when NOT in second phase
                        let hold_result = if !dtap_second_phase {
                            let mut det =
                                HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                HoldDownEvent::None
                            }
                        } else {
                            HoldDownEvent::None
                        };

                        // Always feed double-tap
                        let (dtap_fired, rejection, wait_started_at) = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
                            if let Some(d) = det.as_mut() {
                                let previous_wait = d.second_tap_wait_started_at();
                                let fired = d.handle_event(&event.event_type);
                                let wait_started_at = d
                                    .second_tap_wait_started_at()
                                    .filter(|started_at| Some(*started_at) != previous_wait);
                                (fired, d.take_rejection(), wait_started_at)
                            } else {
                                (false, None, None)
                            }
                        };
                        if let Some(reason) = rejection {
                            emit_hotkey_rejection(&handle, reason, mode);
                        }
                        if let Some(started_at) = wait_started_at {
                            schedule_second_tap_expiry(
                                handle.clone(),
                                mode,
                                listener_generation,
                                started_at,
                            );
                        }

                        match hold_result {
                            HoldDownEvent::Start => {
                                // Don't emit hold-down-start yet — start a timer.
                                // The timer will promote after MAX_HOLD_DURATION_MS.
                                HOLD_PROMOTED.store(false, Ordering::SeqCst);
                                let press_id =
                                    HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
                                let timer_handle = handle.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_millis(
                                        MAX_HOLD_DURATION_MS as u64,
                                    ));
                                    if HOLD_PRESS_COUNTER.load(Ordering::SeqCst) == press_id {
                                        let still_held = {
                                            let det = HOLD_DOWN_DETECTOR
                                                .lock()
                                                .unwrap_or_else(|p| p.into_inner());
                                            det.as_ref()
                                                .map(|d| d.state == HoldState::Held)
                                                .unwrap_or(false)
                                        };
                                        if still_held {
                                            HOLD_PROMOTED.store(true, Ordering::SeqCst);
                                            tracing::info!(target: "keyboard", "BOTH -> timer promoted to hold-down-start");
                                            let _ = timer_handle.emit("hold-down-start", ());
                                        }
                                    }
                                });
                            }
                            HoldDownEvent::Stop => {
                                let promoted = HOLD_PROMOTED.load(Ordering::SeqCst);
                                HOLD_PROMOTED.store(false, Ordering::SeqCst);
                                // Invalidate any pending timer
                                HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);

                                if promoted {
                                    // Recorder transitions are serialized, so a stop safely
                                    // waits for an in-flight start even on an immediate release.
                                    tracing::info!(target: "keyboard", "BOTH -> emit hold-down-stop (promoted hold)");
                                    let _ = handle.emit("hold-down-stop", ());
                                } else if dtap_fired {
                                    // Double-tap completed
                                    tracing::info!(target: "keyboard", "BOTH -> emit double-tap-toggle");
                                    let _ = handle.emit("double-tap-toggle", ());
                                } else {
                                    log_rejection(
                                        RejectionReason::SingleShortTapNoop,
                                        mode,
                                        &event.event_type,
                                    );
                                }
                            }
                            HoldDownEvent::None => {
                                if dtap_fired {
                                    tracing::info!(target: "keyboard", "BOTH -> emit double-tap-toggle (hold=None)");
                                    let _ = handle.emit("double-tap-toggle", ());
                                }
                            }
                        }
                    }
                }
            };

            if let Err(e) = listen(callback) {
                tracing::error!(target: "keyboard", "rdev listener error: {:?}", e);
                LISTENER_THREAD_SPAWNED.store(false, Ordering::SeqCst);
                LISTENER_ACTIVE.store(false, Ordering::SeqCst);
                let _ = error_handle.emit("keyboard-listener-error", format!("{:?}", e));
            }
        });

        // Heartbeat monitor: logs every 60 s while the listener is supposed to
        // be active, so app.log shows a gap if the thread goes silent.
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            if LISTENER_ACTIVE.load(Ordering::SeqCst) {
                let now = now_unix_ms();
                let last_callback_at = LAST_RDEV_CALLBACK_AT_MS.load(Ordering::SeqCst);
                let silent_for_ms = now.saturating_sub(last_callback_at);
                let last_warning_at = LAST_TAP_SILENCE_WARNING_AT_MS.load(Ordering::SeqCst);
                let warning_due = last_warning_at == 0
                    || now.saturating_sub(last_warning_at) >= TAP_SILENCE_WARNING_MS;
                if last_callback_at != 0 && silent_for_ms >= TAP_SILENCE_WARNING_MS && warning_due {
                    LAST_TAP_SILENCE_WARNING_AT_MS.store(now, Ordering::SeqCst);
                    tracing::warn!(
                        target: "keyboard",
                        silent_for_ms = silent_for_ms,
                        threshold_ms = TAP_SILENCE_WARNING_MS,
                        "listener heartbeat — no rdev callbacks observed"
                    );
                }
                tracing::trace!(target: "keyboard", "listener heartbeat — active");
            } else if !LISTENER_THREAD_SPAWNED.load(Ordering::SeqCst) {
                // Listener thread has exited; stop monitoring.
                break;
            }
        });
    }
}

/// Stop processing keyboard events (the thread stays alive but idle).
pub fn stop_listener() {
    LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);

    // Reset both detectors and Both-mode state
    {
        let mut det = DOUBLE_TAP_DETECTOR
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(d) = det.as_mut() {
            d.reset();
        }
    }
    {
        let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(d) = det.as_mut() {
            d.reset();
        }
    }
    HOLD_PROMOTED.store(false, Ordering::SeqCst);
    HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst); // invalidate pending timers
}

/// Update the target key without stopping/restarting the listener.
/// Returns `true` if a hold-down stop event should be emitted (key changed while held).
pub fn set_target_key(hotkey: &str) -> bool {
    let target = hotkey_to_rdev_key(hotkey);
    let mode = {
        let m = ACTIVE_MODE.lock().unwrap_or_else(|p| p.into_inner());
        *m
    };
    match mode {
        DetectorMode::DoubleTap => {
            let mut det = DOUBLE_TAP_DETECTOR
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(d) = det.as_mut() {
                d.set_target(target);
            }
            false
        }
        DetectorMode::HoldDown => {
            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(d) = det.as_mut() {
                d.set_target(target)
            } else {
                false
            }
        }
        DetectorMode::Both => {
            let was_held = {
                let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.set_target(target)
                } else {
                    false
                }
            };
            {
                let mut det = DOUBLE_TAP_DETECTOR
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.set_target(target);
                }
            }
            was_held
        }
    }
}

/// Tell the double-tap detector whether we're currently recording.
/// When recording, a single tap fires (to stop). When idle, double-tap fires (to start).
/// Only relevant for double-tap mode; hold-down mode is stateless.
pub fn set_recording_state(recording: bool) {
    let mut det = DOUBLE_TAP_DETECTOR
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        d.recording = recording;
    }
}

// -- Transform hotkey lifecycle (issue #312, PR-B1) --
//
// Independent of `start_listener` / `stop_listener` / `set_target_key` above:
// none of these three functions touch `DOUBLE_TAP_DETECTOR`, `HOLD_DOWN_DETECTOR`,
// `ACTIVE_MODE`, or `LISTENER_ACTIVE`.

/// Start (or reconfigure) the transform hold-down detector and ensure the
/// shared rdev thread is running. Safe to call whether or not the dictation
/// listener has been started — spawning is idempotent (see
/// `ensure_listener_thread_spawned`).
pub fn start_transform_listener(app_handle: tauri::AppHandle, hotkey: &str) {
    let target = hotkey_to_rdev_key(hotkey);
    {
        let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        match det.as_mut() {
            Some(d) => {
                let _ = d.set_target(target);
            }
            None => {
                let mut d = HoldDownDetector::new();
                let _ = d.set_target(target);
                *det = Some(d);
            }
        }
    }
    TRANSFORM_ACTIVE.store(true, Ordering::SeqCst);
    ensure_listener_thread_spawned(app_handle);
}

/// Disable the transform hotkey (target key cleared, detector reset). Leaves
/// the shared rdev thread and the dictation listener untouched — dictation
/// keeps working exactly as before this was ever called.
pub fn stop_transform_listener() {
    TRANSFORM_ACTIVE.store(false, Ordering::SeqCst);
    let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        let _ = d.set_target(None);
        d.reset();
    }
}

/// Update the transform target key without stopping the detector. Returns
/// `true` if the detector was mid-hold (caller should emit
/// `transform-key-released`), mirroring `set_target_key`'s hold-down contract.
pub fn set_transform_key(hotkey: &str) -> bool {
    let target = hotkey_to_rdev_key(hotkey);
    let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    match det.as_mut() {
        Some(d) => d.set_target(target),
        None => {
            let mut d = HoldDownDetector::new();
            let was_held = d.set_target(target);
            *det = Some(d);
            was_held
        }
    }
}

/// Whether the transform hotkey is currently enabled (a listener was started
/// and not since stopped). Test/diagnostic surface only.
#[cfg(test)]
pub(crate) fn is_transform_active() -> bool {
    TRANSFORM_ACTIVE.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn make_detector(key: Key) -> DoubleTapDetector {
        let mut d = DoubleTapDetector::new();
        d.set_target(Some(key));
        d
    }

    fn press(key: Key) -> EventType {
        EventType::KeyPress(key)
    }

    fn release(key: Key) -> EventType {
        EventType::KeyRelease(key)
    }

    #[test]
    fn basic_double_tap_fires() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Second tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn no_target_key_never_fires() {
        let mut d = DoubleTapDetector::new();
        // target_key is None
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn wrong_key_ignored() {
        let mut d = make_detector(Key::ShiftLeft);

        // Press Alt instead of Shift — should stay idle
        assert!(!d.handle_event(&press(Key::Alt)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn modifier_plus_letter_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // Shift down
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Then 'A' while Shift held — user is typing Shift+A
        assert!(!d.handle_event(&press(Key::KeyA)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn held_too_long_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Wait longer than MAX_HOLD_DURATION_MS
        sleep(Duration::from_millis(350));

        // Release after too long
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn slow_gap_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap — quick
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait longer than DOUBLE_TAP_WINDOW_MS
        sleep(Duration::from_millis(450));

        // Second press after too long a gap — timeout resets to Idle,
        // the press event itself is consumed by the timeout check
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn second_tap_wait_expires_without_another_keyboard_event() {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        let started_at = d.second_tap_wait_started_at().unwrap();

        sleep(Duration::from_millis(DOUBLE_TAP_WINDOW_MS as u64 + 10));

        let elapsed_ms = d.expire_second_tap_wait(started_at);
        assert!(elapsed_ms.is_some());
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn stale_expiry_cannot_reject_a_new_double_tap_sequence() {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        let stale_started_at = d.second_tap_wait_started_at().unwrap();

        d.reset();
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        assert_eq!(d.expire_second_tap_wait(stale_started_at), None);
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
    }

    #[test]
    fn user_feedback_only_surfaces_expired_tap_windows() {
        for mode in [DetectorMode::DoubleTap, DetectorMode::Both] {
            assert!(should_surface_hotkey_rejection(
                RejectionReason::SecondTapExpired,
                mode,
            ));
            for reason in [
                RejectionReason::HeldTooLong,
                RejectionReason::ComboCancelled,
                RejectionReason::SingleShortTapNoop,
                RejectionReason::ProcessingSkipped,
            ] {
                assert!(!should_surface_hotkey_rejection(reason, mode));
            }
        }
        assert!(!should_surface_hotkey_rejection(
            RejectionReason::SecondTapExpired,
            DetectorMode::HoldDown,
        ));
    }

    #[test]
    fn expiry_feedback_requires_the_same_active_listener_generation_and_mode() {
        assert!(listener_context_is_current(
            true,
            DetectorMode::DoubleTap,
            7,
            DetectorMode::DoubleTap,
            7,
        ));
        assert!(!listener_context_is_current(
            false,
            DetectorMode::DoubleTap,
            7,
            DetectorMode::DoubleTap,
            7,
        ));
        assert!(!listener_context_is_current(
            true,
            DetectorMode::HoldDown,
            7,
            DetectorMode::DoubleTap,
            7,
        ));
        assert!(!listener_context_is_current(
            true,
            DetectorMode::DoubleTap,
            8,
            DetectorMode::DoubleTap,
            7,
        ));
    }

    #[test]
    fn cooldown_prevents_triple_tap() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Immediately try another double-tap — should be blocked by cooldown
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        // in_cooldown() returns true, so handle_event returns false early
    }

    #[test]
    fn cooldown_expires() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Wait for cooldown to expire
        sleep(Duration::from_millis(550));

        // Now another double-tap should work
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn second_tap_held_too_long_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap — quick
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — press quick but hold too long before release
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        sleep(Duration::from_millis(350));

        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn letter_during_second_tap_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — Shift down then letter
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        d.handle_event(&press(Key::KeyB));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn other_key_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Press a different key in the gap
        d.handle_event(&press(Key::KeyA));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn key_repeat_during_first_tap_within_hold_duration() {
        let mut d = make_detector(Key::ShiftLeft);

        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Key repeat (same key press again) — should stay in state
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Release quickly
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
    }

    #[test]
    fn alt_key_double_tap() {
        let mut d = make_detector(Key::Alt);

        d.handle_event(&press(Key::Alt));
        d.handle_event(&release(Key::Alt));
        d.handle_event(&press(Key::Alt));
        assert!(d.handle_event(&release(Key::Alt)));
    }

    #[test]
    fn ctrl_key_double_tap() {
        let mut d = make_detector(Key::ControlRight);

        d.handle_event(&press(Key::ControlRight));
        d.handle_event(&release(Key::ControlRight));
        d.handle_event(&press(Key::ControlRight));
        assert!(d.handle_event(&release(Key::ControlRight)));
    }

    #[test]
    fn single_tap_does_not_fire() {
        let mut d = make_detector(Key::ShiftLeft);

        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
        // No second tap — never fires
    }

    #[test]
    fn set_target_resets_state() {
        let mut d = make_detector(Key::ShiftLeft);

        // Start a first tap
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Change target — should reset
        d.set_target(Some(Key::Alt));
        assert_eq!(d.state, DetectorState::Idle);
        assert_eq!(d.target_key, Some(Key::Alt));
    }

    #[test]
    fn hotkey_string_mapping() {
        assert_eq!(hotkey_to_rdev_key("shift_l"), Some(Key::ShiftLeft));
        assert_eq!(hotkey_to_rdev_key("alt_l"), Some(Key::Alt));
        assert_eq!(hotkey_to_rdev_key("ctrl_r"), Some(Key::ControlRight));
        assert_eq!(hotkey_to_rdev_key("unknown"), None);
    }

    #[test]
    fn transform_hotkey_string_mapping() {
        // TransformKey ids (settings.ts) — distinct from the dictation
        // DoubleTapKey ids above, so both hotkeys can be configured at once
        // without colliding on the same physical key.
        assert_eq!(hotkey_to_rdev_key("shift_r"), Some(Key::ShiftRight));
        assert_eq!(hotkey_to_rdev_key("alt_r"), Some(Key::AltGr));
        assert_eq!(hotkey_to_rdev_key("ctrl_l"), Some(Key::ControlLeft));
    }

    #[test]
    fn diagnostic_event_helpers_classify_keyboard_events() {
        let press = EventType::KeyPress(Key::ShiftLeft);
        let release = EventType::KeyRelease(Key::ShiftLeft);
        let wheel = EventType::Wheel {
            delta_x: 0,
            delta_y: 1,
        };

        assert_eq!(event_key(&press), Some(Key::ShiftLeft));
        assert_eq!(event_kind(&press), "key_press");
        assert_eq!(modifier_edge(&press), "press");

        assert_eq!(event_key(&release), Some(Key::ShiftLeft));
        assert_eq!(event_kind(&release), "key_release");
        assert_eq!(modifier_edge(&release), "release");

        assert_eq!(event_key(&wheel), None);
        assert_eq!(event_kind(&wheel), "wheel");
        assert_eq!(modifier_edge(&wheel), "not_modifier");
    }

    #[test]
    fn rejection_reason_labels_are_stable() {
        assert_eq!(RejectionReason::HeldTooLong.as_str(), "held_too_long");
        assert_eq!(
            RejectionReason::SecondTapExpired.as_str(),
            "second_tap_expired"
        );
        assert_eq!(RejectionReason::ComboCancelled.as_str(), "combo_cancelled");
        assert_eq!(
            RejectionReason::SingleShortTapNoop.as_str(),
            "single_short_tap_noop"
        );
        assert_eq!(
            RejectionReason::ProcessingSkipped.as_str(),
            "processing_skipped"
        );
    }

    #[test]
    fn is_modifier_classification() {
        assert!(is_modifier(Key::ShiftLeft));
        assert!(is_modifier(Key::ShiftRight));
        assert!(is_modifier(Key::Alt));
        assert!(is_modifier(Key::ControlLeft));
        assert!(is_modifier(Key::ControlRight));
        assert!(is_modifier(Key::MetaLeft));
        assert!(!is_modifier(Key::KeyA));
        assert!(!is_modifier(Key::Space));
        assert!(!is_modifier(Key::Return));
    }

    // -- Single-tap-to-stop tests (recording=true) --

    #[test]
    fn single_tap_stops_when_recording() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        // Single tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_held_too_long_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        sleep(Duration::from_millis(350));

        // Held too long — not a tap, should not fire
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_with_letter_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        // User types Shift+A — should not stop recording
        assert!(!d.handle_event(&press(Key::KeyA)));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn double_tap_still_required_when_not_recording() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = false;

        // Single tap should NOT fire
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
        // Needs second tap to fire
    }

    #[test]
    fn full_cycle_double_tap_start_single_tap_stop() {
        let mut d = make_detector(Key::ShiftLeft);

        // Not recording — double tap to start
        d.recording = false;
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));

        // Wait for cooldown
        sleep(Duration::from_millis(550));

        // Now recording — single tap to stop
        d.recording = true;
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }

    // -- Hold-down detector tests --

    fn make_hold_detector(key: Key) -> HoldDownDetector {
        let mut d = HoldDownDetector::new();
        d.set_target(Some(key));
        d
    }

    #[test]
    fn hold_basic_press_starts_release_stops() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::Stop
        );
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_no_target_key_never_fires() {
        let mut d = HoldDownDetector::new();
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::None
        );
    }

    #[test]
    fn hold_wrong_key_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::Alt)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_key_repeat_ignored_while_held() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);

        // Key repeat events — should be ignored
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Held);

        // Release still works
        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::Stop
        );
    }

    #[test]
    fn hold_modifier_plus_letter_cancels() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        // User types Shift+A — should cancel and stop
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::Stop);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_release_without_press_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Release while idle — nothing happens
        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::None
        );
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_stop() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Hold and release
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::Stop
        );

        // Immediately press again — should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_expires() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(
            d.handle_event(&release(Key::ShiftLeft)),
            HoldDownEvent::Stop
        );

        // Wait for cooldown to expire
        sleep(Duration::from_millis(350));

        // Now press again — should work
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
    }

    #[test]
    fn hold_alt_key() {
        let mut d = make_hold_detector(Key::Alt);

        assert_eq!(d.handle_event(&press(Key::Alt)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::Alt)), HoldDownEvent::Stop);
    }

    #[test]
    fn hold_ctrl_key() {
        let mut d = make_hold_detector(Key::ControlRight);

        assert_eq!(
            d.handle_event(&press(Key::ControlRight)),
            HoldDownEvent::Start
        );
        assert_eq!(
            d.handle_event(&release(Key::ControlRight)),
            HoldDownEvent::Stop
        );
    }

    #[test]
    fn hold_set_target_while_held_stops() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.state, HoldState::Held);

        // Change target while held — resets to Idle, returns true (should emit stop)
        assert!(d.set_target(Some(Key::Alt)));
        assert_eq!(d.state, HoldState::Idle);

        // Changing target while idle — returns false
        assert!(!d.set_target(Some(Key::ControlRight)));
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_non_modifier_press_in_idle_ignored() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Random key presses while idle — nothing happens
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&press(Key::Space)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_letter_cancel() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        // Cancel with letter
        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::Stop);

        // Immediate re-press should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
    }

    // -- Both-mode tests (deferred hold with second-phase suppression) --

    /// Events that the Both-mode callback would emit synchronously.
    /// hold-down-start is emitted asynchronously by a timer thread and is
    /// NOT part of the synchronous return value.
    #[derive(Debug, PartialEq)]
    enum BothEmit {
        HoldStop,
        DoubleTapToggle,
    }

    /// Simulate the Both-mode deferred-hold arbitration logic.
    /// `promoted` simulates whether the timer thread promoted the press
    /// to a real hold (i.e. HOLD_PROMOTED was true).
    fn both_handle_event(
        hold: &mut HoldDownDetector,
        dtap: &mut DoubleTapDetector,
        event_type: &EventType,
        promoted: bool,
    ) -> Vec<BothEmit> {
        // Check dtap phase BEFORE feeding — also verify the window hasn't expired
        let dtap_second_phase = matches!(
            dtap.state,
            DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
        ) && dtap.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS;

        // Only feed hold-down when NOT in second phase
        let hold_result = if !dtap_second_phase {
            hold.handle_event(event_type)
        } else {
            HoldDownEvent::None
        };

        // Always feed double-tap
        let dtap_fired = dtap.handle_event(event_type);
        let mut emitted = Vec::new();

        match hold_result {
            HoldDownEvent::Start => {
                // In real code: spawns a timer thread, no synchronous emission
            }
            HoldDownEvent::Stop => {
                if promoted {
                    emitted.push(BothEmit::HoldStop);
                } else if dtap_fired {
                    emitted.push(BothEmit::DoubleTapToggle);
                }
                // else: short single tap, nothing to do
            }
            HoldDownEvent::None => {
                if dtap_fired {
                    emitted.push(BothEmit::DoubleTapToggle);
                }
            }
        }
        emitted
    }

    #[test]
    fn both_long_hold_starts_and_stops() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Press — no synchronous emission (timer deferred)
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // Wait past the tap threshold (timer would have promoted)
        sleep(Duration::from_millis(250));

        // Release after promotion → stop
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), true);
        assert_eq!(e, vec![BothEmit::HoldStop]);
    }

    #[test]
    fn both_short_tap_emits_nothing() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Quick press + release — no promotion, no emission
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);
        assert_eq!(dtap.state, DetectorState::WaitingSecondDown);
    }

    #[test]
    fn both_double_tap_fires() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // First tap
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(dtap.state, DetectorState::WaitingSecondDown);

        // Second tap — hold suppressed (second phase), dtap completes
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]); // hold suppressed
        assert_eq!(dtap.state, DetectorState::WaitingSecondUp);

        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(e, vec![BothEmit::DoubleTapToggle]);
    }

    #[test]
    fn both_single_tap_stops_when_recording() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        dtap.recording = true;

        // Press — no sync emission
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // Quick release — dtap fires (single tap to stop)
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(e, vec![BothEmit::DoubleTapToggle]);
    }

    #[test]
    fn both_no_phantom_toggle_after_expired_window() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // First tap
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);

        // Wait for double-tap window + hold cooldown to expire
        sleep(Duration::from_millis(550));

        // Next press — fresh sequence, timer would start (no sync emission)
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);
    }

    #[test]
    fn both_promoted_release_immediately_stops() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Press — timer would start (no sync emission)
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // Release 40ms after promotion — the promoted hold must stop and transcribe.
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), true);
        assert_eq!(e, vec![BothEmit::HoldStop]);
    }

    #[test]
    fn both_promoted_later_release_stops() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Press — timer would start (no sync emission)
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // A later release remains a normal stop.
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), true);
        assert_eq!(e, vec![BothEmit::HoldStop]);
    }

    #[test]
    fn app_disabled_setter_getter_roundtrip() {
        // Ensure clean initial state
        set_app_disabled(false);
        assert!(!is_app_disabled());

        set_app_disabled(true);
        assert!(is_app_disabled());

        set_app_disabled(false);
        assert!(!is_app_disabled());
    }

    #[test]
    fn app_disabled_resets_detectors() {
        // Prime the double-tap detector into a non-idle state
        {
            let mut det = DOUBLE_TAP_DETECTOR
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let d = det.get_or_insert_with(DoubleTapDetector::new);
            d.set_target(Some(Key::ShiftLeft));
            d.handle_event(&EventType::KeyPress(Key::ShiftLeft));
            assert_eq!(d.state, DetectorState::WaitingFirstUp);
        }

        set_app_disabled(true);

        {
            let det = DOUBLE_TAP_DETECTOR
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if let Some(d) = det.as_ref() {
                assert_eq!(d.state, DetectorState::Idle);
            }
        }

        // Restore
        set_app_disabled(false);
    }

    // -- Transform hotkey tests (issue #312, PR-B1) --
    //
    // These exercise the module-level wiring (`set_transform_key`,
    // `stop_transform_listener`, the `TRANSFORM_DETECTOR`/`TRANSFORM_ACTIVE`
    // statics) directly, without going through `start_transform_listener`
    // (which needs a real `tauri::AppHandle` to spawn the rdev thread and so
    // isn't exercised by these pure unit tests). The underlying state machine
    // (`HoldDownDetector`) is already covered exhaustively by the
    // `hold_*` tests above — these confirm the transform wiring reuses it
    // correctly and stays isolated from the dictation detectors.

    /// Reset all transform-related global state to a known baseline so tests
    /// don't leak state into each other (tests run --test-threads=1, but order
    /// is not guaranteed).
    fn reset_transform_state() {
        TRANSFORM_ACTIVE.store(false, Ordering::SeqCst);
        let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        *det = None;
    }

    #[test]
    fn dictation_key_ids_are_rejected_for_transform() {
        assert!(is_dictation_key_id("shift_l"));
        assert!(is_dictation_key_id("alt_l"));
        assert!(is_dictation_key_id("ctrl_r"));
        // The transform key set must remain distinct.
        assert!(!is_dictation_key_id("shift_r"));
        assert!(!is_dictation_key_id("alt_r"));
        assert!(!is_dictation_key_id("ctrl_l"));
        assert!(!is_dictation_key_id("not_a_real_key"));
        assert!(!is_dictation_key_id(""));
    }

    #[test]
    fn transform_set_key_arms_detector_independent_of_active_flag() {
        reset_transform_state();
        assert!(!is_transform_active());

        // set_transform_key alone (no start_transform_listener) should still
        // arm the detector with a target key; TRANSFORM_ACTIVE is untouched —
        // mirrors set_target_key's relationship to LISTENER_ACTIVE.
        let was_held = set_transform_key("alt_l");
        assert!(!was_held);
        assert!(!is_transform_active());

        {
            let det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            assert_eq!(det.as_ref().unwrap().target_key, Some(Key::Alt));
        }

        reset_transform_state();
    }

    #[test]
    fn transform_detector_starts_and_stops_like_hold_down() {
        reset_transform_state();
        set_transform_key("ctrl_r");

        let start = {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            det.as_mut()
                .unwrap()
                .handle_event(&press(Key::ControlRight))
        };
        assert_eq!(start, HoldDownEvent::Start);

        let stop = {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            det.as_mut()
                .unwrap()
                .handle_event(&release(Key::ControlRight))
        };
        assert_eq!(stop, HoldDownEvent::Stop);

        reset_transform_state();
    }

    #[test]
    fn transform_key_change_while_held_reports_should_release() {
        reset_transform_state();
        set_transform_key("shift_l");
        {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let started = det.as_mut().unwrap().handle_event(&press(Key::ShiftLeft));
            assert_eq!(started, HoldDownEvent::Start);
        }

        // Changing key while held should report `true` so the command layer
        // emits transform-key-released, exactly like update_keyboard_key does
        // for the dictation hotkey.
        let should_release = set_transform_key("alt_l");
        assert!(should_release);

        reset_transform_state();
    }

    #[test]
    fn stop_transform_listener_clears_active_and_resets_detector() {
        reset_transform_state();
        set_transform_key("shift_l");
        TRANSFORM_ACTIVE.store(true, Ordering::SeqCst);
        {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            det.as_mut().unwrap().handle_event(&press(Key::ShiftLeft));
        }

        stop_transform_listener();

        assert!(!is_transform_active());
        {
            let det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let d = det.as_ref().unwrap();
            assert_eq!(d.state, HoldState::Idle);
            assert_eq!(d.target_key, None);
        }

        reset_transform_state();
    }

    #[test]
    fn transform_detector_is_isolated_from_dictation_detectors() {
        reset_transform_state();
        set_transform_key("ctrl_r");

        // Prime the dictation hold-down detector on a DIFFERENT key.
        {
            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let d = det.get_or_insert_with(HoldDownDetector::new);
            let _ = d.set_target(Some(Key::ShiftLeft));
        }

        // Press the transform key — must not start the dictation detector.
        {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let started = det
                .as_mut()
                .unwrap()
                .handle_event(&press(Key::ControlRight));
            assert_eq!(started, HoldDownEvent::Start);
        }
        {
            let det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            assert_eq!(det.as_ref().unwrap().state, HoldState::Idle);
        }

        // Press the dictation key — must not affect the transform detector.
        {
            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let started = det.as_mut().unwrap().handle_event(&press(Key::ShiftLeft));
            assert_eq!(started, HoldDownEvent::Start);
        }
        {
            let det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            assert_eq!(det.as_ref().unwrap().state, HoldState::Held);
        }

        // Clean up both.
        {
            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            det.as_mut().unwrap().reset();
        }
        reset_transform_state();
    }

    #[test]
    fn no_transform_target_key_never_fires() {
        reset_transform_state();
        // Detector exists but with no target key set (e.g. "unset" hotkey).
        set_transform_key("not_a_real_key");
        {
            let mut det = TRANSFORM_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            let result = det
                .as_mut()
                .unwrap()
                .handle_event(&press(Key::ControlRight));
            assert_eq!(result, HoldDownEvent::None);
        }
        reset_transform_state();
    }
}
