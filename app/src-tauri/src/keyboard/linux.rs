// rdev-based keyboard listener for Linux (and other non-macOS platforms).
// On macOS this module is replaced by `macos.rs` which uses a native CGEventTap.

use rdev::{listen, Event, EventType as RdevEventType};
use std::sync::atomic::Ordering;
use super::detectors;

pub use super::detectors::{stop_listener, set_target_key, set_recording_state, set_processing};

pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str, mode: &str) {
    let detector_mode = match mode {
        "hold_down" => detectors::DetectorMode::HoldDown,
        "both" => detectors::DetectorMode::Both,
        _ => detectors::DetectorMode::DoubleTap,
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

    detectors::LISTENER_ACTIVE.store(true, Ordering::SeqCst);

    if detectors::LISTENER_THREAD_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        let handle = app_handle.clone();
        let error_handle = app_handle.clone();

        std::thread::spawn(move || {
            tracing::info!(target: "keyboard", "rdev listener thread started");

            let callback = move |event: Event| {
                if !detectors::LISTENER_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }
                let ev = match event.event_type {
                    RdevEventType::KeyPress(k) => {
                        detectors::EventType::KeyPress(detectors::from_rdev_key(k))
                    }
                    RdevEventType::KeyRelease(k) => {
                        detectors::EventType::KeyRelease(detectors::from_rdev_key(k))
                    }
                    _ => return,
                };
                detectors::handle_event(&handle, ev);
            };

            if let Err(e) = listen(callback) {
                tracing::error!(target: "keyboard", "rdev listener error: {:?}", e);
                detectors::LISTENER_THREAD_SPAWNED.store(false, Ordering::SeqCst);
                detectors::LISTENER_ACTIVE.store(false, Ordering::SeqCst);
                let _ = {
                    use tauri::Emitter;
                    error_handle.emit("keyboard-listener-error", format!("{:?}", e))
                };
            }
        });

        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            if detectors::LISTENER_ACTIVE.load(Ordering::SeqCst) {
                tracing::trace!(target: "keyboard", "listener heartbeat — active");
            } else if !detectors::LISTENER_THREAD_SPAWNED.load(Ordering::SeqCst) {
                break;
            }
        });
    }
}
