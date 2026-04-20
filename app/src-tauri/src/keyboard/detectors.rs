//! Keyboard detector types and logic (platform-neutral).
//!
//! Contains the local `Key` and `EventType` enums, both detector structs
//! (`DoubleTapDetector`, `HoldDownDetector`), all shared global statics, and
//! the `handle_event` dispatch function used by the listener thread.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::Emitter;
#[cfg(not(target_os = "macos"))]
use rdev;

// ---------------------------------------------------------------------------
// Platform-neutral key / event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    ShiftLeft,
    ShiftRight,
    Alt,
    AltGr,
    ControlLeft,
    ControlRight,
    MetaLeft,
    MetaRight,
    Escape,
    OtherNonModifier,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    KeyPress(Key),
    KeyRelease(Key),
}

// ---------------------------------------------------------------------------
// Platform conversions
// ---------------------------------------------------------------------------

/// Convert an rdev key to the local Key type.
/// Only used on non-macOS platforms; macOS uses raw keycodes via CGEvent tap.
#[cfg(not(target_os = "macos"))]
pub fn from_rdev_key(k: rdev::Key) -> Key {
    match k {
        rdev::Key::ShiftLeft => Key::ShiftLeft,
        rdev::Key::ShiftRight => Key::ShiftRight,
        rdev::Key::Alt => Key::Alt,
        rdev::Key::AltGr => Key::AltGr,
        rdev::Key::ControlLeft => Key::ControlLeft,
        rdev::Key::ControlRight => Key::ControlRight,
        rdev::Key::MetaLeft => Key::MetaLeft,
        rdev::Key::MetaRight => Key::MetaRight,
        rdev::Key::Escape => Key::Escape,
        _ => Key::OtherNonModifier,
    }
}

/// Convert a raw macOS virtual keycode (from CGEventTap) to the local Key type.
#[cfg(target_os = "macos")]
pub fn from_macos_keycode(code: i64) -> Key {
    match code {
        53 => Key::Escape,
        54 => Key::MetaRight,
        55 => Key::MetaLeft,
        56 => Key::ShiftLeft,
        58 => Key::Alt,
        59 => Key::ControlLeft,
        60 => Key::ShiftRight,
        61 => Key::AltGr,
        62 => Key::ControlRight,
        _ => Key::OtherNonModifier,
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Max duration a single tap can be held before it's rejected
pub(crate) const MAX_HOLD_DURATION_MS: u128 = 200;

/// Max gap between first key-up and second key-down
pub(crate) const DOUBLE_TAP_WINDOW_MS: u128 = 400;

/// Cooldown after firing to prevent triple-tap spam
pub(crate) const COOLDOWN_MS: u128 = 50;

/// Cooldown after hold-down stop to prevent accidental re-trigger
pub(crate) const HOLD_DOWN_COOLDOWN_MS: u128 = 50;

// ---------------------------------------------------------------------------
// Rejection feedback types
// ---------------------------------------------------------------------------

/// The reason a double-tap sequence was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectReason {
    HeldTooLong,
    GapTooLong,
    SecondTapHeldTooLong,
}

impl RejectReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RejectReason::HeldTooLong => "held_too_long",
            RejectReason::GapTooLong => "gap_too_long",
            RejectReason::SecondTapHeldTooLong => "second_tap_held_too_long",
        }
    }
}

/// The outcome of a `DoubleTapDetector::handle_event` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TapOutcome {
    Fired,
    Rejected(RejectReason),
    None,
}

impl TapOutcome {
    pub(crate) fn fired(self) -> bool {
        matches!(self, TapOutcome::Fired)
    }
}

// ---------------------------------------------------------------------------
// Double-tap detector
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

pub(crate) struct DoubleTapDetector {
    pub(crate) state: DetectorState,
    pub(crate) target_key: Option<Key>,
    pub(crate) recording: bool,
    pub(crate) state_entered_at: Instant,
    pub(crate) last_fired_at: Option<Instant>,
}

impl DoubleTapDetector {
    pub(crate) fn new() -> Self {
        Self {
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: Instant::now(),
            last_fired_at: None,
        }
    }

    pub(crate) fn set_target(&mut self, key: Option<Key>) {
        self.target_key = key;
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.state = DetectorState::Idle;
        self.state_entered_at = Instant::now();
    }

    fn transition(&mut self, new_state: DetectorState) {
        self.state = new_state;
        self.state_entered_at = Instant::now();
    }

    pub(crate) fn elapsed_ms(&self) -> u128 {
        self.state_entered_at.elapsed().as_millis()
    }

    fn in_cooldown(&self) -> bool {
        self.last_fired_at
            .map(|t| t.elapsed().as_millis() < COOLDOWN_MS)
            .unwrap_or(false)
    }

    /// Process a keyboard event. Returns `TapOutcome::Fired` if a double-tap was
    /// detected, `TapOutcome::Rejected(reason)` if a timing miss was detected on
    /// the user-visible terminator event, or `TapOutcome::None` otherwise.
    ///
    /// Rejection is emitted only on the sequence-ending event (a release that came
    /// too late, or a target-key press after the gap window). Silent resets (key
    /// repeats, combo keys, mouse events) return `None` to avoid false alarms.
    pub(crate) fn handle_event(&mut self, event_type: &EventType) -> TapOutcome {
        let target = match self.target_key {
            Some(k) => k,
            None => return TapOutcome::None,
        };

        if self.in_cooldown() {
            return TapOutcome::None;
        }

        match self.state {
            DetectorState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_same_modifier(*key, target) {
                        self.transition(DetectorState::WaitingFirstUp);
                    }
                }
                TapOutcome::None
            }

            DetectorState::WaitingFirstUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        let elapsed = self.elapsed_ms();
                        if elapsed <= MAX_HOLD_DURATION_MS {
                            if self.recording {
                                // Single tap to stop — fire immediately
                                self.last_fired_at = Some(Instant::now());
                                self.reset();
                                return TapOutcome::Fired;
                            }
                            self.transition(DetectorState::WaitingSecondDown);
                            TapOutcome::None
                        } else {
                            // Held too long — sequence-ending release event
                            self.reset();
                            tracing::debug!(target: "keyboard", reason = RejectReason::HeldTooLong.as_str(), elapsed_ms = elapsed as u64, "tap rejected");
                            TapOutcome::Rejected(RejectReason::HeldTooLong)
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A
                        self.reset();
                        TapOutcome::None
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat event — check if we've been held too long (silent reset)
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                        TapOutcome::None
                    }
                    _ => {
                        // Check timeout (silent reset — emission happens at release)
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                        TapOutcome::None
                    }
                }
            }

            DetectorState::WaitingSecondDown => {
                let elapsed = self.elapsed_ms();
                if elapsed > DOUBLE_TAP_WINDOW_MS {
                    self.reset();
                    // Emit rejection only when the user is clearly retrying the target modifier
                    if let EventType::KeyPress(key) = event_type {
                        if is_same_modifier(*key, target) {
                            tracing::debug!(target: "keyboard", reason = RejectReason::GapTooLong.as_str(), elapsed_ms = elapsed as u64, "tap rejected");
                            return TapOutcome::Rejected(RejectReason::GapTooLong);
                        }
                    }
                    return TapOutcome::None;
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
                TapOutcome::None
            }

            DetectorState::WaitingSecondUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_same_modifier(*key, target) => {
                        let elapsed = self.elapsed_ms();
                        if elapsed <= MAX_HOLD_DURATION_MS {
                            // Double-tap detected!
                            self.last_fired_at = Some(Instant::now());
                            self.reset();
                            TapOutcome::Fired
                        } else {
                            // Second tap held too long — sequence-ending release event
                            self.reset();
                            tracing::debug!(target: "keyboard", reason = RejectReason::SecondTapHeldTooLong.as_str(), elapsed_ms = elapsed as u64, "tap rejected");
                            TapOutcome::Rejected(RejectReason::SecondTapHeldTooLong)
                        }
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // Combo like Shift+A on second press
                        self.reset();
                        TapOutcome::None
                    }
                    EventType::KeyPress(key) if is_same_modifier(*key, target) => {
                        // Key repeat — silent reset if overlong
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                        TapOutcome::None
                    }
                    _ => {
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.reset();
                        }
                        TapOutcome::None
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hold-down detector
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HoldDownEvent {
    None,
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HoldState {
    Idle,
    Held,
}

pub(crate) struct HoldDownDetector {
    pub(crate) state: HoldState,
    pub(crate) target_key: Option<Key>,
    pub(crate) last_stopped_at: Option<Instant>,
}

impl HoldDownDetector {
    pub(crate) fn new() -> Self {
        Self {
            state: HoldState::Idle,
            target_key: None,
            last_stopped_at: None,
        }
    }

    /// Set the target key. Returns `true` if the detector was in `Held` state
    /// (i.e. the caller should emit a stop event to the frontend).
    pub(crate) fn set_target(&mut self, key: Option<Key>) -> bool {
        let was_held = self.state == HoldState::Held;
        if was_held {
            self.state = HoldState::Idle;
            self.last_stopped_at = Some(Instant::now());
        }
        self.target_key = key;
        was_held
    }

    pub(crate) fn reset(&mut self) {
        self.state = HoldState::Idle;
    }

    fn in_cooldown(&self) -> bool {
        self.last_stopped_at
            .map(|t| t.elapsed().as_millis() < HOLD_DOWN_COOLDOWN_MS)
            .unwrap_or(false)
    }

    /// Process a keyboard event. Returns Start, Stop, or None.
    pub(crate) fn handle_event(&mut self, event_type: &EventType) -> HoldDownEvent {
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

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DetectorMode {
    DoubleTap,
    HoldDown,
    Both,
}

/// Map hotkey string from settings to local Key
pub(crate) fn hotkey_to_key(hotkey: &str) -> Option<Key> {
    match hotkey {
        "shift_l" => Some(Key::ShiftLeft),
        "alt_l" => Some(Key::Alt),
        "ctrl_r" => Some(Key::ControlRight),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Check if a key is any modifier key
pub(crate) fn is_modifier(key: Key) -> bool {
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
pub(crate) fn is_same_modifier(a: Key, b: Key) -> bool {
    a == b
}

// ---------------------------------------------------------------------------
// Both-mode arbitration state
// ---------------------------------------------------------------------------

/// Monotonic counter to invalidate stale hold-promotion timers.
pub(crate) static HOLD_PRESS_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Set to true by the timer thread when it promotes a press to a real hold.
pub(crate) static HOLD_PROMOTED: AtomicBool = AtomicBool::new(false);
/// When true, the Both-mode callback ignores all key events.
/// Set by lib.rs when the transcription pipeline is running.
pub(crate) static IS_PROCESSING: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Global listener state
// ---------------------------------------------------------------------------

pub(crate) static LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static LISTENER_THREAD_SPAWNED: AtomicBool = AtomicBool::new(false);

pub(crate) static ACTIVE_MODE: Mutex<DetectorMode> = Mutex::new(DetectorMode::DoubleTap);
pub(crate) static DOUBLE_TAP_DETECTOR: Mutex<Option<DoubleTapDetector>> = Mutex::new(None);
pub(crate) static HOLD_DOWN_DETECTOR: Mutex<Option<HoldDownDetector>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

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

/// Returns whether the app is currently in the processing state.
#[cfg(test)]
pub fn is_processing() -> bool {
    IS_PROCESSING.load(Ordering::SeqCst)
}

/// Update the target key without stopping/restarting the listener.
/// Returns `true` if a hold-down stop event should be emitted (key changed while held).
pub fn set_target_key(hotkey: &str) -> bool {
    let target = hotkey_to_key(hotkey);
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

/// Stop processing keyboard events (the thread stays alive but idle).
pub fn stop_listener() {
    LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    { let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner()); if let Some(d) = det.as_mut() { d.reset(); } }
    { let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner()); if let Some(d) = det.as_mut() { d.reset(); } }
    HOLD_PROMOTED.store(false, Ordering::SeqCst);
    HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);
}

/// Dispatch a keyboard event to the appropriate detector(s) and emit Tauri events.
///
/// This contains the full dispatch logic from the rdev listener callback,
/// adapted to use the local `EventType`/`Key` types instead of rdev types.
pub fn handle_event(app: &tauri::AppHandle, ev: EventType) {
    if !LISTENER_ACTIVE.load(Ordering::SeqCst) {
        return;
    }

    // Escape key: cancel recording/transcription regardless of mode.
    // Must be checked before mode-specific logic so it works even
    // during IS_PROCESSING (which gates the Both-mode block).
    if let EventType::KeyPress(Key::Escape) = ev {
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
            let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(d) = det.as_mut() {
                d.reset();
                d.last_fired_at = Some(Instant::now());
            }
        }
        // Invalidate pending hold-promotion timers
        HOLD_PROMOTED.store(false, Ordering::SeqCst);
        HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);

        tracing::info!(target: "keyboard", "Escape pressed — emitting escape-cancel");
        let _ = app.emit("escape-cancel", ());
        return;
    }

    let mode = {
        let m = ACTIVE_MODE.lock().unwrap_or_else(|p| p.into_inner());
        *m
    };

    match mode {
        DetectorMode::DoubleTap => {
            let outcome: TapOutcome = {
                let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.handle_event(&ev)
                } else {
                    TapOutcome::None
                }
            };
            match outcome {
                TapOutcome::Fired => {
                    tracing::info!(target: "keyboard", "DOUBLE_TAP -> emit double-tap-toggle");
                    let _ = app.emit("double-tap-toggle", ());
                }
                TapOutcome::Rejected(reason) => {
                    tracing::info!(target: "keyboard", reason = reason.as_str(), "DOUBLE_TAP -> emit tap-rejected");
                    let _ = app.emit("tap-rejected", reason.as_str());
                }
                TapOutcome::None => {}
            }
        }
        // hold_down-only mode has no timing rejection — any press is a valid start.
        DetectorMode::HoldDown => {
            let result = {
                let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.handle_event(&ev)
                } else {
                    HoldDownEvent::None
                }
            };
            match result {
                HoldDownEvent::Start => { let _ = app.emit("hold-down-start", ()); }
                HoldDownEvent::Stop => { let _ = app.emit("hold-down-stop", ()); }
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
                    d.handle_event(&ev)
                } else {
                    HoldDownEvent::None
                }
            } else {
                HoldDownEvent::None
            };

            // Always feed double-tap
            let dtap_outcome: TapOutcome = {
                let mut det = DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.handle_event(&ev)
                } else {
                    TapOutcome::None
                }
            };
            let dtap_fired = dtap_outcome.fired();

            match hold_result {
                HoldDownEvent::Start => {
                    // Don't emit hold-down-start yet — start a timer.
                    // The timer will promote after MAX_HOLD_DURATION_MS.
                    HOLD_PROMOTED.store(false, Ordering::SeqCst);
                    let press_id = HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
                    let timer_handle = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(MAX_HOLD_DURATION_MS as u64));
                        if HOLD_PRESS_COUNTER.load(Ordering::SeqCst) == press_id {
                            let still_held = {
                                let det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                                det.as_ref().map(|d| d.state == HoldState::Held).unwrap_or(false)
                            };
                            if still_held {
                                HOLD_PROMOTED.store(true, Ordering::SeqCst);
                                tracing::info!(target: "keyboard", "BOTH -> timer promoted to hold-down-start");
                                let _ = timer_handle.emit("hold-down-start", ());
                            }
                        }
                    });
                    // Emit tap-rejected if dtap returned a rejection (e.g. GapTooLong on delayed retry)
                    if let TapOutcome::Rejected(r) = dtap_outcome {
                        tracing::info!(target: "keyboard", reason = r.as_str(), "BOTH -> emit tap-rejected (on Start)");
                        let _ = app.emit("tap-rejected", r.as_str());
                    }
                }
                HoldDownEvent::Stop => {
                    let promoted = HOLD_PROMOTED.load(Ordering::SeqCst);
                    HOLD_PROMOTED.store(false, Ordering::SeqCst);
                    // Invalidate any pending timer
                    HOLD_PRESS_COUNTER.fetch_add(1, Ordering::SeqCst);

                    if promoted {
                        // Real hold ended — stop + transcribe
                        tracing::info!(target: "keyboard", "BOTH -> emit hold-down-stop (promoted hold)");
                        let _ = app.emit("hold-down-stop", ());
                    } else if dtap_fired {
                        // Double-tap completed
                        tracing::info!(target: "keyboard", "BOTH -> emit double-tap-toggle");
                        let _ = app.emit("double-tap-toggle", ());
                    }
                    // Short single tap emits nothing — only delayed second-tap via GapTooLong produces feedback.
                    if !promoted {
                        if let TapOutcome::Rejected(r) = dtap_outcome {
                            tracing::info!(target: "keyboard", reason = r.as_str(), "BOTH -> emit tap-rejected (on Stop)");
                            let _ = app.emit("tap-rejected", r.as_str());
                        }
                    }
                }
                HoldDownEvent::None => {
                    if dtap_fired {
                        tracing::info!(target: "keyboard", "BOTH -> emit double-tap-toggle (hold=None)");
                        let _ = app.emit("double-tap-toggle", ());
                    }
                    if let TapOutcome::Rejected(r) = dtap_outcome {
                        tracing::info!(target: "keyboard", reason = r.as_str(), "BOTH -> emit tap-rejected (on None)");
                        let _ = app.emit("tap-rejected", r.as_str());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // ── DoubleTapDetector tests ──────────────────────────────────────────────

    #[test]
    fn basic_double_tap_fires() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap: press then release quickly
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Second tap: press then release quickly
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn no_target_key_never_fires() {
        let mut d = DoubleTapDetector::new();
        // target_key is None
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
    }

    #[test]
    fn wrong_key_ignored() {
        let mut d = make_detector(Key::ShiftLeft);

        // Press Alt instead of Shift — should stay idle
        assert_eq!(d.handle_event(&press(Key::Alt)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn modifier_plus_letter_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // Shift down
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Then a non-modifier key while Shift held — user is typing Shift+A (intentional combo)
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn held_too_long_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Wait longer than MAX_HOLD_DURATION_MS
        sleep(Duration::from_millis(350));

        // Release after too long — rejected (sequence-ending release event)
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::HeldTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn slow_gap_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap — quick
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait longer than DOUBLE_TAP_WINDOW_MS
        sleep(Duration::from_millis(450));

        // Second press after too long a gap — rejected (delayed retry of target modifier)
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::GapTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn cooldown_prevents_triple_tap() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());

        // Immediately try another double-tap — should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        // in_cooldown() returns true, so handle_event returns None early
    }

    #[test]
    fn cooldown_expires() {
        let mut d = make_detector(Key::ShiftLeft);

        // Successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());

        // Wait for cooldown to expire
        sleep(Duration::from_millis(550));

        // Now another double-tap should work
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());
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

        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::SecondTapHeldTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn letter_during_second_tap_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — Shift down then non-modifier key (intentional combo, not timing miss)
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn other_key_between_taps_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Press a different (non-modifier) key in the gap
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn key_repeat_during_first_tap_within_hold_duration() {
        let mut d = make_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Key repeat (same key press again) — should stay in state
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        // Release quickly
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingSecondDown);
    }

    #[test]
    fn alt_key_double_tap() {
        let mut d = make_detector(Key::Alt);

        d.handle_event(&press(Key::Alt));
        d.handle_event(&release(Key::Alt));
        d.handle_event(&press(Key::Alt));
        assert!(d.handle_event(&release(Key::Alt)).fired());
    }

    #[test]
    fn ctrl_key_double_tap() {
        let mut d = make_detector(Key::ControlRight);

        d.handle_event(&press(Key::ControlRight));
        d.handle_event(&release(Key::ControlRight));
        d.handle_event(&press(Key::ControlRight));
        assert!(d.handle_event(&release(Key::ControlRight)).fired());
    }

    #[test]
    fn single_tap_does_not_fire() {
        let mut d = make_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
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
        assert_eq!(hotkey_to_key("shift_l"), Some(Key::ShiftLeft));
        assert_eq!(hotkey_to_key("alt_l"), Some(Key::Alt));
        assert_eq!(hotkey_to_key("ctrl_r"), Some(Key::ControlRight));
        assert_eq!(hotkey_to_key("unknown"), None);
    }

    #[test]
    fn is_modifier_classification() {
        assert!(is_modifier(Key::ShiftLeft));
        assert!(is_modifier(Key::ShiftRight));
        assert!(is_modifier(Key::Alt));
        assert!(is_modifier(Key::ControlLeft));
        assert!(is_modifier(Key::ControlRight));
        assert!(is_modifier(Key::MetaLeft));
        assert!(!is_modifier(Key::OtherNonModifier));
        assert!(!is_modifier(Key::Escape));
    }

    // ── Single-tap-to-stop tests (recording=true) ────────────────────────────

    #[test]
    fn single_tap_stops_when_recording() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        // Single tap: press then release quickly
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::WaitingFirstUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_held_too_long_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        sleep(Duration::from_millis(350));

        // Held too long — rejected (user tried to stop-tap but held too long)
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::HeldTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn single_tap_with_letter_does_not_stop() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        // User types non-modifier — should not stop recording (intentional combo)
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
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
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());

        // Wait for cooldown
        sleep(Duration::from_millis(550));

        // Now recording — single tap to stop
        d.recording = true;
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());
    }

    // ── New DoubleTapDetector rejection reason tests ─────────────────────────

    #[test]
    fn held_too_long_reports_held_too_long() {
        let mut d = make_detector(Key::ShiftLeft);

        d.handle_event(&press(Key::ShiftLeft));
        sleep(Duration::from_millis(250));
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::HeldTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn gap_too_long_letter_press_is_noop() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait for the gap window to expire
        sleep(Duration::from_millis(450));

        // Press a non-target key — not a delayed retry, so no rejection
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn gap_too_long_target_modifier_press_rejects() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait for the gap window to expire
        sleep(Duration::from_millis(450));

        // Press the target modifier — delayed retry, emit GapTooLong
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::GapTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn second_tap_held_too_long_reports_reason() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap (quick)
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));

        // Second tap — press quickly, hold too long before release
        d.handle_event(&press(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        sleep(Duration::from_millis(250));
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::Rejected(RejectReason::SecondTapHeldTooLong));
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn long_hold_with_key_repeats_emits_at_most_one_rejection() {
        let mut d = make_detector(Key::ShiftLeft);

        // Press and feed key repeats until we exceed MAX_HOLD_DURATION_MS
        let mut outcomes = Vec::new();
        outcomes.push(d.handle_event(&press(Key::ShiftLeft)));
        for _ in 0..5 {
            sleep(Duration::from_millis(50));
            outcomes.push(d.handle_event(&press(Key::ShiftLeft)));
        }
        // Release after all the key repeats
        outcomes.push(d.handle_event(&release(Key::ShiftLeft)));

        // At most one Rejected variant observed across the whole sequence
        let rejection_count = outcomes.iter().filter(|o| matches!(o, TapOutcome::Rejected(_))).count();
        assert!(rejection_count <= 1, "expected at most one rejection, got {rejection_count}: {outcomes:?}");
    }

    #[test]
    fn mouse_event_during_gap_does_not_reject() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Wait for the gap window to expire
        sleep(Duration::from_millis(450));

        // Feed a non-modifier, non-target press (simulates a non-retry event) — no rejection
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), TapOutcome::None);
        assert_eq!(d.state, DetectorState::Idle);
    }

    #[test]
    fn no_target_key_returns_none() {
        let mut d = DoubleTapDetector::new(); // no target set

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
        assert_eq!(d.handle_event(&release(Key::ShiftLeft)), TapOutcome::None);
    }

    #[test]
    fn cooldown_returns_none() {
        let mut d = make_detector(Key::ShiftLeft);

        // Complete a successful double-tap
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)).fired());

        // Immediately press — cooldown active, returns None
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), TapOutcome::None);
    }

    // ── Hold-down detector tests ─────────────────────────────────────────────

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

        // User types non-modifier key — should cancel and stop
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), HoldDownEvent::Stop);
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

        // Random non-modifier key presses while idle — nothing happens
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), HoldDownEvent::None);
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Idle);
    }

    #[test]
    fn hold_cooldown_after_letter_cancel() {
        let mut d = make_hold_detector(Key::ShiftLeft);

        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        // Cancel with non-modifier key
        assert_eq!(d.handle_event(&press(Key::OtherNonModifier)), HoldDownEvent::Stop);

        // Immediate re-press should be blocked by cooldown
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::None);
    }

    // ── Both-mode tests (deferred hold with second-phase suppression) ─────────

    /// Events that the Both-mode callback would emit synchronously.
    /// hold-down-start is emitted asynchronously by a timer thread and is
    /// NOT part of the synchronous return value.
    #[derive(Debug, PartialEq)]
    enum BothEmit {
        HoldStop,
        DoubleTapToggle,
        TapRejected(&'static str),
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
        let dtap_outcome = dtap.handle_event(event_type);
        let dtap_fired = dtap_outcome.fired();
        let mut emitted = Vec::new();

        match hold_result {
            HoldDownEvent::Start => {
                // In real code: spawns a timer thread, no synchronous emission
                if let TapOutcome::Rejected(r) = dtap_outcome {
                    emitted.push(BothEmit::TapRejected(r.as_str()));
                }
            }
            HoldDownEvent::Stop => {
                if promoted {
                    emitted.push(BothEmit::HoldStop);
                } else if dtap_fired {
                    emitted.push(BothEmit::DoubleTapToggle);
                }
                // Short single tap emits nothing — only delayed second-tap via GapTooLong produces feedback.
                if !promoted {
                    if let TapOutcome::Rejected(r) = dtap_outcome {
                        emitted.push(BothEmit::TapRejected(r.as_str()));
                    }
                }
            }
            HoldDownEvent::None => {
                if dtap_fired {
                    emitted.push(BothEmit::DoubleTapToggle);
                }
                if let TapOutcome::Rejected(r) = dtap_outcome {
                    emitted.push(BothEmit::TapRejected(r.as_str()));
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

        // Release — promoted hold → stop (dtap HeldTooLong is suppressed by !promoted guard)
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), true);
        assert_eq!(e, vec![BothEmit::HoldStop]);
    }

    #[test]
    fn both_short_tap_emits_nothing() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Quick press + release — no promotion, no emission
        // Short single tap is silent — the user's first tap of an intended double-tap.
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
    fn both_gap_too_long_emits_rejection_after_expired_window() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // First tap
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);

        // Wait for double-tap window + hold cooldown to expire
        sleep(Duration::from_millis(550));

        // Next press — dtap was in WaitingSecondDown with elapsed > 400ms, target modifier pressed
        // → GapTooLong rejection emitted from the HoldDownEvent::Start arm
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![BothEmit::TapRejected("gap_too_long")]);
    }

    // ── New Both-mode rejection tests ────────────────────────────────────────

    #[test]
    fn both_promoted_hold_suppresses_tap_rejection() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Press — no sync emission
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // Wait past the tap threshold (timer would have promoted)
        sleep(Duration::from_millis(250));

        // Release with promoted=true — "hold wins" rule suppresses the HeldTooLong rejection
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), true);
        assert_eq!(e, vec![BothEmit::HoldStop]);
    }

    #[test]
    fn both_delayed_second_tap_rejects_on_modifier_press() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // First tap
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);

        // Wait for double-tap window to expire
        sleep(Duration::from_millis(450));

        // Press target modifier — dtap returns GapTooLong rejection
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![BothEmit::TapRejected("gap_too_long")]);
    }

    #[test]
    fn both_delayed_second_tap_with_letter_is_noop() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // First tap
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);

        // Wait for double-tap window to expire
        sleep(Duration::from_millis(450));

        // Press a non-modifier key — not a delayed retry of the target, so no rejection
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::OtherNonModifier), false);
        assert_eq!(e, vec![]);
    }

    // Note: In practice a 250ms hold almost always triggers the deferred timer promotion,
    // but this test passes promoted=false to isolate the !promoted guard behavior.
    #[test]
    fn both_first_tap_held_too_long_not_promoted_emits_rejection() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Press — no sync emission
        let e = both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(e, vec![]);

        // Wait past the tap threshold
        sleep(Duration::from_millis(250));

        // Release with promoted=false — dtap in WaitingFirstUp, elapsed > 200ms → HeldTooLong
        // Since !promoted, the rejection is emitted
        let e = both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(e, vec![BothEmit::TapRejected("held_too_long")]);
    }
}
