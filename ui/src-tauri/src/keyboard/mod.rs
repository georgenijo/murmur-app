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

mod double_tap;
mod hold_down;
mod listener;

use rdev::Key;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use double_tap::DoubleTapDetector;
use hold_down::HoldDownDetector;

/// Max duration a single tap can be held before it's rejected
const MAX_HOLD_DURATION_MS: u128 = 200;

/// Max gap between first key-up and second key-down
const DOUBLE_TAP_WINDOW_MS: u128 = 400;

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

// -- Global listener state --

static LISTENER_ACTIVE: AtomicBool = AtomicBool::new(false);
static LISTENER_THREAD_SPAWNED: AtomicBool = AtomicBool::new(false);

static ACTIVE_MODE: Mutex<DetectorMode> = Mutex::new(DetectorMode::DoubleTap);
static DOUBLE_TAP_DETECTOR: Mutex<Option<DoubleTapDetector>> = Mutex::new(None);
static HOLD_DOWN_DETECTOR: Mutex<Option<HoldDownDetector>> = Mutex::new(None);

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
                Some(d) => {
                    if d.set_target(target) {
                        // Detector was held — caller should emit stop event
                        // Note: app_handle is available here if needed
                    }
                },
                None => {
                    let mut d = HoldDownDetector::new();
                    d.set_target(target); // New detector, can't be held
                    *det = Some(d);
                }
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
        listener::spawn_listener_thread(app_handle);
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
}
