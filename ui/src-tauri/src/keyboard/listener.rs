use rdev::{listen, set_is_main_thread, Event};
use std::sync::atomic::Ordering;
use tauri::Emitter;

use crate::{log_error, log_info};
use super::{
    LISTENER_ACTIVE, LISTENER_THREAD_SPAWNED, ACTIVE_MODE, DetectorMode,
    DOUBLE_TAP_DETECTOR, HOLD_DOWN_DETECTOR,
    HOLD_PRESS_COUNTER, HOLD_PROMOTED, IS_PROCESSING,
    MAX_HOLD_DURATION_MS, DOUBLE_TAP_WINDOW_MS,
};
use super::double_tap::DetectorState;
use super::hold_down::{HoldDownEvent, HoldState};

pub(super) fn spawn_listener_thread(app_handle: tauri::AppHandle) {
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

#[cfg(test)]
mod tests {
    use super::super::double_tap::DoubleTapDetector;
    use super::super::hold_down::{HoldDownDetector, HoldDownEvent};
    use super::super::double_tap::DetectorState;
    use super::super::DOUBLE_TAP_WINDOW_MS;
    use rdev::{EventType, Key};
    use std::thread::sleep;
    use std::time::Duration;

    fn press(key: Key) -> EventType {
        EventType::KeyPress(key)
    }

    fn release(key: Key) -> EventType {
        EventType::KeyRelease(key)
    }

    fn make_detector(key: Key) -> DoubleTapDetector {
        let mut d = DoubleTapDetector::new();
        d.set_target(Some(key));
        d
    }

    fn make_hold_detector(key: Key) -> HoldDownDetector {
        let mut d = HoldDownDetector::new();
        d.set_target(Some(key));
        d
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
