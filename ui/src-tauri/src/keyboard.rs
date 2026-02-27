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

use rdev::{listen, set_is_main_thread, Event, EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;
use crate::{log_error, log_info};

/// Max duration a single tap can be held before it's rejected
const MAX_HOLD_DURATION_MS: u128 = 200;

/// Max gap between first key-up and second key-down
const DOUBLE_TAP_WINDOW_MS: u128 = 400;

/// Cooldown after firing to prevent triple-tap spam
const COOLDOWN_MS: u128 = 50;

/// Cooldown after hold-down stop to prevent accidental re-trigger
const HOLD_DOWN_COOLDOWN_MS: u128 = 50;

#[derive(Debug, Clone, Copy, PartialEq)]
enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

struct DoubleTapDetector {
    state: DetectorState,
    target_key: Option<Key>,
    recording: bool,
    state_entered_at: Instant,
    last_fired_at: Option<Instant>,
}

impl DoubleTapDetector {
    fn new() -> Self {
        Self {
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: Instant::now(),
            last_fired_at: None,
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

    /// Process a keyboard event. Returns true if a double-tap was detected.
    fn handle_event(&mut self, event_type: &EventType) -> bool {
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
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat event — ignore, stay in same state
                        // But check if we've been held too long
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                    _ => {
                        // Check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                }
                false
            }

            DetectorState::WaitingSecondDown => {
                if self.elapsed_ms() > DOUBLE_TAP_WINDOW_MS {
                    self.reset();
                    return false;
                }
                match event_type {
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        self.transition(DetectorState::WaitingSecondUp);
                    }
                    EventType::KeyPress(_) => {
                        // Any other key press — abort
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
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // Combo like Shift+A on second press
                        self.reset();
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                    _ => {
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                    }
                }
                false
            }
        }
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

/// Map hotkey string from settings to rdev Key
fn hotkey_to_rdev_key(hotkey: &str) -> Option<Key> {
    match hotkey {
        "shift_l" => Some(Key::ShiftLeft),
        "alt_l" => Some(Key::Alt),
        "ctrl_r" => Some(Key::ControlRight),
        _ => None,
    }
}

// -- Both-mode arbitration state --


/// Monotonic counter to invalidate stale hold-promotion timers.
static HOLD_PRESS_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Set to true by the timer thread when it promotes a press to a real hold.
static HOLD_PROMOTED: AtomicBool = AtomicBool::new(false);
/// When true, the Both-mode callback ignores all key events.
/// Set by lib.rs when the transcription pipeline is running.
static IS_PROCESSING: AtomicBool = AtomicBool::new(false);

/// Called by lib.rs to tell the keyboard module whether the app is processing.
/// When transitioning out of processing, reset both detectors and apply a
/// cooldown so rapid post-processing taps don't immediately toggle.
pub fn set_processing(processing: bool) {
    let was_processing = IS_PROCESSING.swap(processing, Ordering::SeqCst);
    if !was_processing && processing {
        // Entering processing: invalidate any pending hold-promotion timer
        // so it can't fire hold-down-start during active processing.
        HOLD_PROMOTED.store(false, Ordering::SeqCst);
        HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut det) = HOLD_DOWN_DETECTOR.lock() {
            if let Some(d) = det.as_mut() { d.reset(); }
        }
        if let Ok(mut det) = DOUBLE_TAP_DETECTOR.lock() {
            if let Some(d) = det.as_mut() { d.reset(); }
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

// -- Global listener state --

static LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static LISTENER_THREAD_SPAWNED: AtomicBool = AtomicBool::new(false);

static ACTIVE_MODE: Mutex<DetectorMode> = Mutex::new(DetectorMode::DoubleTap);
static DOUBLE_TAP_DETECTOR: Mutex<Option<DoubleTapDetector>> = Mutex::new(None);
static HOLD_DOWN_DETECTOR: Mutex<Option<HoldDownDetector>> = Mutex::new(None);

/// Start the keyboard listener. Spawns the rdev listener thread if not already running.
/// If already running, just updates the target key, mode, and re-enables.
///
/// `mode` should be `"double_tap"` or `"hold_down"`.
pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str, mode: &str) {
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
            let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
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
                Some(d) => { let _ = d.set_target(target); },
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
                    Some(d) => { let _ = d.set_target(target); },
                    None => {
                        let mut d = HoldDownDetector::new();
                        let _ = d.set_target(target);
                        *det = Some(d);
                    }
                }
            }
            {
                let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
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
            set_is_main_thread(false);
            log_info!("keyboard: rdev listener thread started");

            let callback = move |event: Event| {
                if !LISTENER_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }

                let mode = {
                    let m = ACTIVE_MODE.lock().unwrap_or_else(|p| p.into_inner());
                    *m
                };

                match mode {
                    DetectorMode::DoubleTap => {
                        let fired = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                false
                            }
                        };
                        if fired {
                            let _ = handle.emit("double-tap-toggle", ());
                        }
                    }
                    DetectorMode::HoldDown => {
                        let result = {
                            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                HoldDownEvent::None
                            }
                        };
                        match result {
                            HoldDownEvent::Start => { let _ = handle.emit("hold-down-start", ()); }
                            HoldDownEvent::Stop => { let _ = handle.emit("hold-down-stop", ()); }
                            HoldDownEvent::None => {}
                        }
                    }
                    DetectorMode::Both => {
                        // Skip all events while the app is processing a transcription.
                        if IS_PROCESSING.load(Ordering::SeqCst) {
                            return;
                        }

                        // Deferred hold: on press, start a background timer.
                        // After MAX_HOLD_DURATION_MS, if the key is still held,
                        // the timer emits hold-down-start (promoting to a real hold).
                        // Short taps never start recording → no state thrash during double-tap.

                        // Check dtap phase BEFORE feeding — also verify the window hasn't expired
                        let dtap_second_phase = {
                            let det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            det.as_ref().map(|d| matches!(d.state,
                                DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
                            ) && d.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS).unwrap_or(false)
                        };

                        // Only feed hold-down when NOT in second phase
                        let hold_result = if !dtap_second_phase {
                            let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                HoldDownEvent::None
                            }
                        } else {
                            HoldDownEvent::None
                        };

                        // Always feed double-tap
                        let dtap_fired = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                d.handle_event(&event.event_type)
                            } else {
                                false
                            }
                        };

                        match hold_result {
                            HoldDownEvent::Start => {
                                // Don't emit hold-down-start yet — start a timer.
                                // The timer will promote after MAX_HOLD_DURATION_MS.
                                HOLD_PROMOTED.store(false, Ordering::SeqCst);
                                let press_id = HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
                                let timer_handle = handle.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_millis(MAX_HOLD_DURATION_MS as u64));
                                    if HOLD_PRESS_COUNTER.load(Ordering::SeqCst) == press_id {
                                        let still_held = {
                                            let det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                                            det.as_ref().map(|d| d.state == HoldState::Held).unwrap_or(false)
                                        };
                                        if still_held {
                                            HOLD_PROMOTED.store(true, Ordering::SeqCst);
                                            log_info!("keyboard: BOTH -> timer promoted to hold-down-start");
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
                                    // Real hold ended — stop + transcribe
                                    log_info!("keyboard: BOTH -> emit hold-down-stop (promoted hold)");
                                    let _ = handle.emit("hold-down-stop", ());
                                } else if dtap_fired {
                                    // Double-tap completed
                                    log_info!("keyboard: BOTH -> emit double-tap-toggle");
                                    let _ = handle.emit("double-tap-toggle", ());
                                }
                                // else: short single tap, no recording was started, nothing to do
                            }
                            HoldDownEvent::None => {
                                if dtap_fired {
                                    log_info!("keyboard: BOTH -> emit double-tap-toggle (hold=None)");
                                    let _ = handle.emit("double-tap-toggle", ());
                                }
                            }
                        }
                    }
                }
            };

            if let Err(e) = listen(callback) {
                log_error!("keyboard: rdev listener error: {:?}", e);
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
                log_info!("keyboard: listener heartbeat — active");
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

    // Reset both detectors and Both-mode state
    {
        let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
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
            let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
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
                let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
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
    let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(d) = det.as_mut() {
        d.recording = recording;
    }
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

        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_no_target_key_never_fires() {
        let mut d = HoldDownDetector::new();
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::None);
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
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);
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
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_stop() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        // Hold and release
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);

        // Immediately press again — should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_expires() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), HoldDownEvent::Stop);

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

        assert_eq!(d.handle_event(&press(Key::ControlRight)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&release(Key::ControlRight)), HoldDownEvent::Stop);
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
        let dtap_second_phase = matches!(dtap.state,
            DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp)
            && dtap.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS;

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

        // Release — promoted hold → stop
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
}
