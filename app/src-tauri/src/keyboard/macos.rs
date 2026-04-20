// CGEventTap listener for macOS.
//
// Lifetime: the tap runs on a dedicated std::thread that calls CFRunLoopRun()
// which blocks forever. The current API deliberately does not expose a way to
// tear the tap down; stop_listener() only clears LISTENER_ACTIVE so callback
// events become no-ops. If a future change needs to stop the run loop, call
// CFRunLoopStop(run_loop) from any thread — do NOT simply drop the thread.
//
// Thread safety: the tap callback runs on the CFRunLoop thread (not main).
// CGEventGetIntegerValueField, CGEventGetFlags, and CGEventTapEnable are
// documented thread-safe. We do NOT call any TIS/TSM APIs, so no main-thread
// affinity is required. (Contrast with rdev on macOS which DOES call TIS via
// dispatch_sync to main, which can stall under contention and trigger the
// active-tap timeout.)
//
// Tap level: kCGHIDEventTap delivers events at the hardware-input layer,
// before window routing — system-wide regardless of which app is focused.
// kCGEventTapOptionListenOnly means the tap is passive and not subject to the
// macOS event-tap timeout penalty that disables active taps under load.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Emitter;

use super::{detectors, sys};

pub use super::detectors::{stop_listener, set_target_key, set_recording_state, set_processing};

// ── Module-level state ────────────────────────────────────────────────────────

/// Wrapper making CFMachPortRef (a raw pointer) usable in a Mutex static.
struct SendMachPort(sys::CFMachPortRef);
unsafe impl Send for SendMachPort {}
unsafe impl Sync for SendMachPort {}

static STORED_TAP: Mutex<Option<SendMachPort>> = Mutex::new(None);
static APP_HANDLE: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);

// Diagnostic counters (bumped for modifier/Escape callbacks and re-enables)
static MODIFIER_CALLBACK_INVOCATIONS: AtomicU64 = AtomicU64::new(0);
static TAP_REENABLE_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_MODIFIER_CALLBACK_AT_MILLIS: AtomicU64 = AtomicU64::new(0);

// Per-modifier latches for FlagsChanged press/release disambiguation.
// A FlagsChanged event for a modifier does not carry explicit press/release info;
// we derive it by comparing the event's flag bits to the latch (prior state).
static LATCH_SHIFT_L: AtomicBool = AtomicBool::new(false);
static LATCH_SHIFT_R: AtomicBool = AtomicBool::new(false);
static LATCH_CTRL_L: AtomicBool = AtomicBool::new(false);
static LATCH_CTRL_R: AtomicBool = AtomicBool::new(false);
static LATCH_OPT_L: AtomicBool = AtomicBool::new(false);
static LATCH_OPT_R: AtomicBool = AtomicBool::new(false);
static LATCH_CMD_L: AtomicBool = AtomicBool::new(false);
static LATCH_CMD_R: AtomicBool = AtomicBool::new(false);

static TAP_ENABLER: Mutex<Option<Arc<dyn TapEnabler + Send + Sync>>> = Mutex::new(None);

// ── TapEnabler trait (abstracts re-enable for testability) ────────────────────

pub(crate) trait TapEnabler {
    fn reenable(&self);
}

struct RealEnabler;

impl TapEnabler for RealEnabler {
    fn reenable(&self) {
        let guard = STORED_TAP.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(port) = guard.as_ref() {
            unsafe { sys::CGEventTapEnable(port.0, true) };
            tracing::info!(target: "keyboard", "tap re-enabled");
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Returns the device-specific CGEventFlags bit mask for a modifier key.
fn modifier_flag_mask(key: detectors::Key) -> u64 {
    match key {
        detectors::Key::ShiftLeft    => sys::NX_DEVICELSHIFTKEYMASK,
        detectors::Key::ShiftRight   => sys::NX_DEVICERSHIFTKEYMASK,
        detectors::Key::ControlLeft  => sys::NX_DEVICELCTLKEYMASK,
        detectors::Key::ControlRight => sys::NX_DEVICERCTLKEYMASK,
        detectors::Key::Alt          => sys::NX_DEVICELALTKEYMASK,
        detectors::Key::AltGr        => sys::NX_DEVICERALTKEYMASK,
        detectors::Key::MetaLeft     => sys::NX_DEVICELCMDKEYMASK,
        detectors::Key::MetaRight    => sys::NX_DEVICERCMDKEYMASK,
        _ => 0,
    }
}

fn get_latch(key: detectors::Key) -> bool {
    match key {
        detectors::Key::ShiftLeft    => LATCH_SHIFT_L.load(Ordering::Relaxed),
        detectors::Key::ShiftRight   => LATCH_SHIFT_R.load(Ordering::Relaxed),
        detectors::Key::ControlLeft  => LATCH_CTRL_L.load(Ordering::Relaxed),
        detectors::Key::ControlRight => LATCH_CTRL_R.load(Ordering::Relaxed),
        detectors::Key::Alt          => LATCH_OPT_L.load(Ordering::Relaxed),
        detectors::Key::AltGr        => LATCH_OPT_R.load(Ordering::Relaxed),
        detectors::Key::MetaLeft     => LATCH_CMD_L.load(Ordering::Relaxed),
        detectors::Key::MetaRight    => LATCH_CMD_R.load(Ordering::Relaxed),
        _ => false,
    }
}

fn set_latch(key: detectors::Key, val: bool) {
    match key {
        detectors::Key::ShiftLeft    => LATCH_SHIFT_L.store(val, Ordering::Relaxed),
        detectors::Key::ShiftRight   => LATCH_SHIFT_R.store(val, Ordering::Relaxed),
        detectors::Key::ControlLeft  => LATCH_CTRL_L.store(val, Ordering::Relaxed),
        detectors::Key::ControlRight => LATCH_CTRL_R.store(val, Ordering::Relaxed),
        detectors::Key::Alt          => LATCH_OPT_L.store(val, Ordering::Relaxed),
        detectors::Key::AltGr        => LATCH_OPT_R.store(val, Ordering::Relaxed),
        detectors::Key::MetaLeft     => LATCH_CMD_L.store(val, Ordering::Relaxed),
        detectors::Key::MetaRight    => LATCH_CMD_R.store(val, Ordering::Relaxed),
        _ => {}
    }
}

fn reset_modifier_latches() {
    LATCH_SHIFT_L.store(false, Ordering::Relaxed);
    LATCH_SHIFT_R.store(false, Ordering::Relaxed);
    LATCH_CTRL_L.store(false, Ordering::Relaxed);
    LATCH_CTRL_R.store(false, Ordering::Relaxed);
    LATCH_OPT_L.store(false, Ordering::Relaxed);
    LATCH_OPT_R.store(false, Ordering::Relaxed);
    LATCH_CMD_L.store(false, Ordering::Relaxed);
    LATCH_CMD_R.store(false, Ordering::Relaxed);
}

fn any_modifier_held() -> bool {
    LATCH_SHIFT_L.load(Ordering::Relaxed)
        || LATCH_SHIFT_R.load(Ordering::Relaxed)
        || LATCH_CTRL_L.load(Ordering::Relaxed)
        || LATCH_CTRL_R.load(Ordering::Relaxed)
        || LATCH_OPT_L.load(Ordering::Relaxed)
        || LATCH_OPT_R.load(Ordering::Relaxed)
        || LATCH_CMD_L.load(Ordering::Relaxed)
        || LATCH_CMD_R.load(Ordering::Relaxed)
}

// ── Disable-event recovery ────────────────────────────────────────────────────

/// Called when a kCGEventTapDisabledBy* event arrives. Increments the reenable
/// counter, calls the enabler, and updates diagnostic timestamps. Does NOT touch
/// LISTENER_ACTIVE — the tap being re-enabled is transparent to callers.
pub(crate) fn handle_disable_event(enabler: &dyn TapEnabler, reason: &str) {
    TAP_REENABLE_COUNT.fetch_add(1, Ordering::Relaxed);
    tracing::warn!(target: "keyboard", "tap disabled ({}); re-enabling", reason);
    enabler.reenable();
    MODIFIER_CALLBACK_INVOCATIONS.fetch_add(1, Ordering::Relaxed);
    LAST_MODIFIER_CALLBACK_AT_MILLIS.store(now_millis(), Ordering::Relaxed);
    reset_modifier_latches();
}

// ── CGEventTap callback ───────────────────────────────────────────────────────

unsafe extern "C" fn tap_callback(
    _proxy: sys::CGEventTapProxy,
    etype: sys::CGEventType,
    event: sys::CGEventRef,
    _user_info: *mut c_void,
) -> sys::CGEventRef {
    // Handle tap-disable meta-events first
    if etype == sys::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT
        || etype == sys::K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        let enabler_opt = {
            let guard = TAP_ENABLER.lock().unwrap_or_else(|p| p.into_inner());
            guard.clone()
        };
        if let Some(enabler) = enabler_opt {
            let reason = if etype == sys::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
                "timeout"
            } else {
                "user_input"
            };
            handle_disable_event(enabler.as_ref(), reason);
        }
        return event;
    }

    if !detectors::LISTENER_ACTIVE.load(Ordering::Relaxed) {
        return event;
    }

    let keycode = sys::CGEventGetIntegerValueField(event, sys::K_CG_KEYBOARD_EVENT_KEYCODE);
    let key = detectors::from_macos_keycode(keycode);

    match etype {
        sys::K_CG_EVENT_FLAGS_CHANGED => {
            // Only modifier keys generate FlagsChanged. Determine press vs release
            // by comparing the current event flags against the per-modifier latch.
            if detectors::is_modifier(key) {
                let flags = sys::CGEventGetFlags(event);
                let mask = modifier_flag_mask(key);
                let now_held = mask != 0 && (flags & mask) != 0;
                let was_held = get_latch(key);

                if now_held && !was_held {
                    set_latch(key, true);
                    MODIFIER_CALLBACK_INVOCATIONS.fetch_add(1, Ordering::Relaxed);
                    LAST_MODIFIER_CALLBACK_AT_MILLIS.store(now_millis(), Ordering::Relaxed);
                    let app_opt = {
                        let guard = APP_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
                        guard.as_ref().cloned()
                    };
                    if let Some(app) = app_opt {
                        detectors::handle_event(&app, detectors::EventType::KeyPress(key));
                    }
                } else if !now_held && was_held {
                    set_latch(key, false);
                    MODIFIER_CALLBACK_INVOCATIONS.fetch_add(1, Ordering::Relaxed);
                    LAST_MODIFIER_CALLBACK_AT_MILLIS.store(now_millis(), Ordering::Relaxed);
                    let app_opt = {
                        let guard = APP_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
                        guard.as_ref().cloned()
                    };
                    if let Some(app) = app_opt {
                        detectors::handle_event(&app, detectors::EventType::KeyRelease(key));
                    }
                }
            }
        }

        sys::K_CG_EVENT_KEY_DOWN => {
            match key {
                detectors::Key::Escape => {
                    MODIFIER_CALLBACK_INVOCATIONS.fetch_add(1, Ordering::Relaxed);
                    LAST_MODIFIER_CALLBACK_AT_MILLIS.store(now_millis(), Ordering::Relaxed);
                    let app_opt = {
                        let guard = APP_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
                        guard.as_ref().cloned()
                    };
                    if let Some(app) = app_opt {
                        detectors::handle_event(&app, detectors::EventType::KeyPress(detectors::Key::Escape));
                    }
                }
                detectors::Key::OtherNonModifier => {
                    // Only dispatch if a modifier is currently held so we can
                    // cancel an in-progress double-tap or hold sequence without
                    // bumping diagnostics on every regular keystroke.
                    if any_modifier_held() {
                        let app_opt = {
                            let guard = APP_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
                            guard.as_ref().cloned()
                        };
                        if let Some(app) = app_opt {
                            detectors::handle_event(&app, detectors::EventType::KeyPress(detectors::Key::OtherNonModifier));
                        }
                    }
                }
                _ => {}
            }
        }

        _ => {}
    }

    event
}

// ── Listener lifecycle ────────────────────────────────────────────────────────

pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str, mode: &str) {
    let detector_mode = match mode {
        "hold_down" => detectors::DetectorMode::HoldDown,
        "both"      => detectors::DetectorMode::Both,
        _           => detectors::DetectorMode::DoubleTap,
    };

    {
        let mut m = detectors::ACTIVE_MODE.lock().unwrap_or_else(|p| p.into_inner());
        *m = detector_mode;
    }

    let target = detectors::hotkey_to_key(hotkey);

    match detector_mode {
        detectors::DetectorMode::DoubleTap => {
            let mut det = detectors::DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            match det.as_mut() {
                Some(d) => d.set_target(target),
                None => {
                    let mut d = detectors::DoubleTapDetector::new();
                    d.set_target(target);
                    *det = Some(d);
                }
            }
        }
        detectors::DetectorMode::HoldDown => {
            let mut det = detectors::HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
            match det.as_mut() {
                Some(d) => { let _ = d.set_target(target); }
                None => {
                    let mut d = detectors::HoldDownDetector::new();
                    let _ = d.set_target(target);
                    *det = Some(d);
                }
            }
        }
        detectors::DetectorMode::Both => {
            {
                let mut det = detectors::HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                match det.as_mut() {
                    Some(d) => { let _ = d.set_target(target); }
                    None => {
                        let mut d = detectors::HoldDownDetector::new();
                        let _ = d.set_target(target);
                        *det = Some(d);
                    }
                }
            }
            {
                let mut det = detectors::DOUBLE_TAP_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                match det.as_mut() {
                    Some(d) => d.set_target(target),
                    None => {
                        let mut d = detectors::DoubleTapDetector::new();
                        d.set_target(target);
                        *det = Some(d);
                    }
                }
            }
        }
    }

    // Store the AppHandle for use in the callback
    {
        let mut guard = APP_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
        *guard = Some(app_handle.clone());
    }

    detectors::LISTENER_ACTIVE.store(true, Ordering::SeqCst);
    reset_modifier_latches();
    LAST_MODIFIER_CALLBACK_AT_MILLIS.store(now_millis(), Ordering::Relaxed);

    if detectors::LISTENER_THREAD_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // Set the real tap enabler
        {
            let mut guard = TAP_ENABLER.lock().unwrap_or_else(|p| p.into_inner());
            *guard = Some(Arc::new(RealEnabler));
        }

        let error_handle = app_handle.clone();

        std::thread::Builder::new()
            .name("keyboard-tap".into())
            .spawn(move || {
                let event_mask = sys::cg_event_mask_bit(sys::K_CG_EVENT_KEY_DOWN)
                    | sys::cg_event_mask_bit(sys::K_CG_EVENT_KEY_UP)
                    | sys::cg_event_mask_bit(sys::K_CG_EVENT_FLAGS_CHANGED)
                    | sys::cg_event_mask_bit(sys::K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT)
                    | sys::cg_event_mask_bit(sys::K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT);

                let tap = unsafe {
                    sys::CGEventTapCreate(
                        sys::K_CG_HID_EVENT_TAP,
                        sys::K_CG_HEAD_INSERT_EVENT_TAP,
                        sys::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        event_mask,
                        tap_callback,
                        std::ptr::null_mut(),
                    )
                };

                let tap = if tap.is_null() {
                    tracing::warn!(target: "keyboard",
                        "HID tap creation failed; retrying with session tap (degraded focus behavior)");
                    let session_tap = unsafe {
                        sys::CGEventTapCreate(
                            sys::K_CG_SESSION_EVENT_TAP,
                            sys::K_CG_HEAD_INSERT_EVENT_TAP,
                            sys::K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                            event_mask,
                            tap_callback,
                            std::ptr::null_mut(),
                        )
                    };
                    if session_tap.is_null() {
                        tracing::error!(target: "keyboard",
                            "Both HID and session CGEventTap creation failed — keyboard listener unavailable");
                        detectors::LISTENER_THREAD_SPAWNED.store(false, Ordering::SeqCst);
                        detectors::LISTENER_ACTIVE.store(false, Ordering::SeqCst);
                        let _ = error_handle.emit(
                            "keyboard-listener-error",
                            "CGEventTapCreate failed for both HID and session tap levels",
                        );
                        return;
                    }
                    let _ = error_handle.emit(
                        "keyboard-listener-degraded",
                        serde_json::json!({ "reason": "session_tap_fallback" }),
                    );
                    tracing::warn!(target: "keyboard",
                        "running on session tap — hotkeys may not work when Desktop is focused");
                    session_tap
                } else {
                    tap
                };

                // Retain the port so it stays alive for the duration of the app
                unsafe { sys::CFRetain(tap as *const c_void) };
                {
                    let mut guard = STORED_TAP.lock().unwrap_or_else(|p| p.into_inner());
                    *guard = Some(SendMachPort(tap));
                }

                let source = unsafe {
                    sys::CFMachPortCreateRunLoopSource(std::ptr::null_mut(), tap, 0)
                };
                let rl = unsafe { sys::CFRunLoopGetCurrent() };
                unsafe {
                    sys::CFRunLoopAddSource(rl, source, sys::kCFRunLoopCommonModes);
                    sys::CGEventTapEnable(tap, true);
                    tracing::info!(target: "keyboard", "CGEventTap started on keyboard-tap thread");
                    sys::CFRunLoopRun(); // blocks forever on this thread
                }
            })
            .expect("failed to spawn keyboard-tap thread");

        // Heartbeat thread: logs diagnostic counters every 60 s and attempts
        // a belt-and-suspenders re-enable if the tap appears silent for 120 s.
        std::thread::spawn(|| {
            let mut prev_invocations = MODIFIER_CALLBACK_INVOCATIONS.load(Ordering::Relaxed);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                if !detectors::LISTENER_THREAD_SPAWNED.load(Ordering::Relaxed) {
                    break;
                }
                if detectors::LISTENER_ACTIVE.load(Ordering::Relaxed) {
                    let invocations = MODIFIER_CALLBACK_INVOCATIONS.load(Ordering::Relaxed);
                    let reenable_count = TAP_REENABLE_COUNT.load(Ordering::Relaxed);
                    let last_ms = LAST_MODIFIER_CALLBACK_AT_MILLIS.load(Ordering::Relaxed);
                    let now_ms = now_millis();
                    let ms_since = now_ms.saturating_sub(last_ms);

                    tracing::info!(
                        target: "keyboard",
                        modifier_invocations = invocations,
                        reenable_count = reenable_count,
                        ms_since_last_callback = ms_since,
                        "tap heartbeat"
                    );

                    if invocations == prev_invocations && ms_since >= 120_000 {
                        tracing::warn!(target: "keyboard",
                            "tap silent — zero modifier callbacks in last heartbeat window; attempting re-enable");
                        let guard = STORED_TAP.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(port) = guard.as_ref() {
                            unsafe { sys::CGEventTapEnable(port.0, true) };
                        }
                    }
                    prev_invocations = invocations;
                }
            }
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    struct MockEnabler {
        reenables: AtomicU64,
    }

    impl TapEnabler for MockEnabler {
        fn reenable(&self) {
            self.reenables.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn test_handle_disable_event_reenables_and_counts() {
        let prev_reenable_count = TAP_REENABLE_COUNT.load(Ordering::Relaxed);
        let was_active = detectors::LISTENER_ACTIVE.load(Ordering::Relaxed);

        let enabler = MockEnabler { reenables: AtomicU64::new(0) };
        handle_disable_event(&enabler, "test");

        assert_eq!(
            enabler.reenables.load(Ordering::Relaxed),
            1,
            "reenable() should be called exactly once"
        );
        assert_eq!(
            TAP_REENABLE_COUNT.load(Ordering::Relaxed),
            prev_reenable_count + 1,
            "TAP_REENABLE_COUNT should increment by 1"
        );
        assert_eq!(
            detectors::LISTENER_ACTIVE.load(Ordering::Relaxed),
            was_active,
            "handle_disable_event must not modify LISTENER_ACTIVE"
        );
    }
}
