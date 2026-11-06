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
//!   Modifier triggers emit start only after an arming window in which no
//!   other key was pressed (`MODIFIER_HOLD_ARMING_MS`).
//!
//! Both modes reject trigger-key combos (for example Shift+A or F8+A).

use crate::MutexExt;
use murmur_core::keyboard::*;
use rdev::listen;
#[cfg(target_os = "macos")]
use rdev::set_is_main_thread;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

// -- Shared types --

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

fn current_gesture() -> Option<Candidate> {
    DOUBLE_TAP_DETECTOR
        .lock_or_recover()
        .as_ref()
        .and_then(|d| d.candidate)
}
fn emit_hold_start(handle: &tauri::AppHandle, candidate: Option<Candidate>) {
    let token = candidate.map(|c| crate::audio_lifecycle::gesture::emit(c, true));
    crate::dictation_control::send(
        handle,
        murmur_core::dictation::Input::HoldStart(token.clone()),
    );
    let _ = handle.emit(
        "hold-down-start",
        serde_json::json!({ "gestureToken": token }),
    );
}
fn emit_dictation_toggle(handle: &tauri::AppHandle) {
    let candidate = DOUBLE_TAP_DETECTOR
        .lock_or_recover()
        .as_mut()
        .and_then(|d| d.fired_candidate.take());
    let token = candidate.map(|c| crate::audio_lifecycle::gesture::emit(c, false));
    crate::dictation_control::send(handle, murmur_core::dictation::Input::Toggle(token.clone()));
    let _ = handle.emit(
        "double-tap-toggle",
        serde_json::json!({ "gestureToken": token }),
    );
}

fn invalidate_preparation(reason: crate::audio::prepare::InvalidateReason) {
    crate::audio::prepare::invalidate(reason);
}

/// The dictation listener generation a silent first-tap preparation is bound
/// to; a restart, key change, or stop makes older preparations unadoptable.
pub(crate) fn listener_generation() -> u64 {
    LISTENER_GENERATION.load(Ordering::SeqCst)
}

/// Act on a first clean tap (#933): silently prepare an idle capture helper,
/// or abandon one whose sequence ended without firing. Admission runs on the
/// preparation thread and only spawns when the shared dictation start would
/// be admitted, microphone access is already granted (never prompting), and
/// no capture or standby owns the microphone.
fn apply_prepare_signal(
    handle: &tauri::AppHandle,
    signal: PrepareSignal,
    mode: DetectorMode,
    listener_generation: u64,
) {
    match signal {
        PrepareSignal::None => {}
        PrepareSignal::Abandon => {
            invalidate_preparation(crate::audio::prepare::InvalidateReason::SequenceAbandoned);
        }
        PrepareSignal::Request(candidate) => {
            let revocation_generation = crate::audio::prepare::revocation_generation();
            let identity = crate::audio::prepare::PrepareIdentity {
                token: candidate.token,
                listener_generation,
                topology_generation: crate::audio_lifecycle::input_topology_generation(),
                configuration_generation: crate::audio::prepare::configuration_generation(),
            };
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let Some(state) = handle.try_state::<crate::State>() else {
                    return;
                };
                // Same transition as toolbar, meeting, and
                // preview claims. Keep it through the preparation slot claim.
                let Some(_transition) = crate::audio::prepare::admission_transition(
                    &state.app_state.recording_transition,
                    || {
                        listener_context_matches(mode, listener_generation)
                            && !IS_PROCESSING.load(Ordering::SeqCst)
                            && !APP_DISABLED.load(Ordering::SeqCst)
                            && crate::commands::permissions::check_microphone_permission_status()
                                == "granted"
                            && DOUBLE_TAP_DETECTOR
                                .lock_or_recover()
                                .as_ref()
                                .is_some_and(|d| d.preparation_candidate_matches(identity.token))
                            && crate::commands::recording::dictation_prepare_admitted(&state)
                            && identity.topology_generation
                                == crate::audio_lifecycle::input_topology_generation()
                            && identity.configuration_generation
                                == crate::audio::prepare::configuration_generation()
                    },
                )
                .await
                else {
                    return;
                };
                // Keep the detector identity stable through the slot claim;
                // a chord/expiry that already ran cannot be resurrected by a
                // queued async admission continuation.
                let detector = DOUBLE_TAP_DETECTOR.lock_or_recover();
                if detector
                    .as_ref()
                    .is_some_and(|d| d.preparation_candidate_matches(identity.token))
                {
                    crate::audio::prepare::request(identity, revocation_generation);
                }
            });
        }
    }
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

        let expired = {
            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
            det.as_mut().and_then(|d| {
                let token = d.prepared_token;
                d.expire_second_tap_wait(started_at)
                    .map(|elapsed| (elapsed, token))
            })
        };
        if let Some((_, Some(token))) = expired {
            crate::audio::prepare::invalidate_token(
                token,
                listener_generation,
                crate::audio::prepare::InvalidateReason::SecondTapExpired,
            );
        }
        let elapsed_ms = expired.map(|(elapsed, _)| elapsed);

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

/// Hotkey ids reserved for the dictation listener (`DictationKey` in
/// settings.ts). The transform hotkey commands (`start_transform_listener`,
/// `set_transform_key` in `commands/keyboard.rs`) reject these so the two key
/// sets stay disjoint at the Rust boundary too, not just in the TS type —
/// `hotkey_to_rdev_key` below accepts either set with no id-ownership check
/// of its own.
pub const DICTATION_KEY_IDS: &[&str] = &[
    "shift_l", "alt_l", "ctrl_r", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10",
    "f11", "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20",
];

/// Whether `hotkey` is one of the ids reserved for the dictation listener
/// (see `DICTATION_KEY_IDS`). Pure so it's unit-testable without a listener
/// or `tauri::AppHandle`.
pub fn is_dictation_key_id(hotkey: &str) -> bool {
    DICTATION_KEY_IDS.contains(&hotkey)
}

/// Map hotkey string from settings to rdev Key.
///
/// The dictation and Paste Last listeners share this mapping.
fn hotkey_to_rdev_key(hotkey: &str) -> Option<Key> {
    match hotkey {
        "shift_l" => Some(Key::ShiftLeft),
        "alt_l" => Some(Key::Alt),
        "ctrl_r" => Some(Key::ControlRight),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        "f13" => Some(Key::F13),
        "f14" => Some(Key::F14),
        "f15" => Some(Key::F15),
        "f16" => Some(Key::F16),
        "f17" => Some(Key::F17),
        "f18" => Some(Key::F18),
        "f19" => Some(Key::F19),
        "f20" => Some(Key::F20),
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

/// Inverse of `hotkey_to_rdev_key`: the settings id of a configurable hotkey.
fn hotkey_id(key: Key) -> Option<&'static str> {
    DICTATION_KEY_IDS
        .iter()
        .chain(["shift_r", "alt_r", "ctrl_l"].iter())
        .copied()
        .find(|id| hotkey_to_rdev_key(id) == Some(key))
}

/// Bounded, content-free class of a key for telemetry.
///
/// Telemetry never records the identity of a key other than the configured
/// hotkey: a key pressed during a hotkey hold is usually the next character
/// the user is typing (Shift+I), so its name would leak typed content.
fn key_class(key: Key) -> &'static str {
    match key {
        k if is_modifier(k) => "modifier",
        Key::CapsLock | Key::Function => "modifier",
        k if is_function_key(k) => "function",
        Key::F21 | Key::F22 | Key::F23 | Key::F24 => "function",
        Key::UpArrow
        | Key::DownArrow
        | Key::LeftArrow
        | Key::RightArrow
        | Key::Home
        | Key::End
        | Key::PageUp
        | Key::PageDown => "navigation",
        Key::Return
        | Key::KpReturn
        | Key::Tab
        | Key::Backspace
        | Key::Delete
        | Key::KpDelete
        | Key::Insert
        | Key::Escape => "editing",
        Key::Space
        | Key::KeyA
        | Key::KeyB
        | Key::KeyC
        | Key::KeyD
        | Key::KeyE
        | Key::KeyF
        | Key::KeyG
        | Key::KeyH
        | Key::KeyI
        | Key::KeyJ
        | Key::KeyK
        | Key::KeyL
        | Key::KeyM
        | Key::KeyN
        | Key::KeyO
        | Key::KeyP
        | Key::KeyQ
        | Key::KeyR
        | Key::KeyS
        | Key::KeyT
        | Key::KeyU
        | Key::KeyV
        | Key::KeyW
        | Key::KeyX
        | Key::KeyY
        | Key::KeyZ
        | Key::Num0
        | Key::Num1
        | Key::Num2
        | Key::Num3
        | Key::Num4
        | Key::Num5
        | Key::Num6
        | Key::Num7
        | Key::Num8
        | Key::Num9
        | Key::Kp0
        | Key::Kp1
        | Key::Kp2
        | Key::Kp3
        | Key::Kp4
        | Key::Kp5
        | Key::Kp6
        | Key::Kp7
        | Key::Kp8
        | Key::Kp9
        | Key::KpMinus
        | Key::KpPlus
        | Key::KpMultiply
        | Key::KpDivide
        | Key::BackQuote
        | Key::Minus
        | Key::Equal
        | Key::LeftBracket
        | Key::RightBracket
        | Key::SemiColon
        | Key::Quote
        | Key::BackSlash
        | Key::IntlBackslash
        | Key::Comma
        | Key::Dot
        | Key::Slash => "character",
        _ => "other",
    }
}

fn event_key_class(event_type: &EventType) -> &'static str {
    event_key(event_type).map_or("none", key_class)
}

/// Whether the event is a press or release of the configured target key.
fn is_target_event(event_type: &EventType, target: Option<Key>) -> bool {
    matches!((event_key(event_type), target), (Some(key), Some(target)) if is_target_key(key, target))
}

/// The configured hotkey's settings id. Never the raw identity of another key.
fn target_key_label(target: Option<Key>) -> &'static str {
    match target {
        None => "none",
        Some(key) => hotkey_id(key).unwrap_or("other"),
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

/// The configured dictation hotkey, from whichever detector is installed.
fn dictation_target_key() -> Option<Key> {
    let hold_target = {
        let det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
        det.as_ref().and_then(|d| d.target_key)
    };
    hold_target.or_else(|| {
        let det = DOUBLE_TAP_DETECTOR.lock_or_recover();
        det.as_ref().and_then(|d| d.target_key)
    })
}

fn trace_raw_callback(event: &Event, mode: DetectorMode) {
    let (double_tap_state, hold_state) = detector_state_snapshot();
    let target = dictation_target_key();
    // Never record `event.name` or the raw key: they are what the user typed.
    tracing::trace!(
        target: "keyboard",
        event_kind = event_kind(&event.event_type),
        key_class = event_key_class(&event.event_type),
        is_target_key = is_target_event(&event.event_type, target),
        target_key = target_key_label(target),
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
    let target = dictation_target_key();
    tracing::info!(
        target: "keyboard",
        reason = reason.as_str(),
        mode = ?mode,
        event_kind = event_kind(event_type),
        key_class = event_key_class(event_type),
        is_target_key = is_target_event(event_type, target),
        target_key = target_key_label(target),
        double_tap_state = ?double_tap_state,
        hold_state = ?hold_state,
        "keyboard event rejected"
    );
}

/// Arming facts read under the hold-detector lock alongside its result.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HoldArming {
    window_ms: u128,
    held_ms: u64,
}

impl HoldArming {
    const OFF: Self = Self {
        window_ms: 0,
        held_ms: 0,
    };

    fn of(detector: &HoldDownDetector) -> Self {
        Self {
            window_ms: detector.arming_ms,
            held_ms: detector.held_ms(),
        }
    }

    /// The hold's duration when `event_type` (which ended an unpromoted hold)
    /// was another key press rather than the trigger's own release.
    fn cancelled_by(self, event_type: &EventType) -> Option<u64> {
        (self.window_ms > 0 && is_unclean_tap_input(event_type)).then_some(self.held_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HoldDownSignal {
    Start,
    Stop,
    /// Begin the arming timer for this press.
    Arm {
        press_id: u64,
        window_ms: u128,
    },
    ArmingCancelled {
        reason: &'static str,
        held_ms: u64,
    },
}

/// Hold-Down mode's response to a detector result. With an arming window the
/// start is deferred to a timer and an unpromoted hold ends silently, so a
/// Shift+letter or short Shift tap never reaches `hold-down-start`.
fn dispatch_hold_down(
    promotion: &HoldPromotion,
    result: HoldDownEvent,
    arming: HoldArming,
    event_type: &EventType,
    mut signal: impl FnMut(HoldDownSignal),
) {
    match result {
        HoldDownEvent::Start if arming.window_ms > 0 => signal(HoldDownSignal::Arm {
            press_id: promotion.begin(),
            window_ms: arming.window_ms,
        }),
        HoldDownEvent::Start => signal(HoldDownSignal::Start),
        HoldDownEvent::Stop if arming.window_ms > 0 => {
            let started = promotion.finish(|promoted| {
                if promoted.is_some() {
                    signal(HoldDownSignal::Stop);
                }
                promoted.is_some()
            });
            if !started {
                let reason = match event_type {
                    EventType::KeyRelease(_) => "released",
                    _ => "other_key",
                };
                signal(HoldDownSignal::ArmingCancelled {
                    reason,
                    held_ms: arming.held_ms,
                });
            }
        }
        HoldDownEvent::Stop => signal(HoldDownSignal::Stop),
        HoldDownEvent::None => {}
    }
}

/// Both mode: any key press that ended a modifier hold's arming window resets
/// the double-tap sequence directly instead of being fed to it. A typed key
/// would reach the same Idle state through `ComboCancelled`, which logs the
/// key. A second modifier would leave the sequence in `WaitingFirstUp`, so the
/// next typed key would be logged there instead.
fn arming_combo_resets_double_tap(
    was_arming: bool,
    hold_result: HoldDownEvent,
    event_type: &EventType,
) -> bool {
    was_arming && hold_result == HoldDownEvent::Stop && is_unclean_tap_input(event_type)
}

/// Input that makes a held trigger key not a clean tap or arming hold: any
/// other key (modifiers included), a mouse button, or the wheel. Pointer
/// movement alone is clean.
fn is_unclean_tap_input(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::KeyPress(_) | EventType::ButtonPress(_) | EventType::Wheel { .. }
    )
}

/// One content-free record per modifier hold that ended before its recording
/// start was committed. Never includes which key ended it.
fn log_hotkey_arming_cancelled(mode: DetectorMode, reason: &'static str, held_ms: u64) {
    tracing::info!(
        target: "keyboard",
        event_code = "keyboard.hotkey_arming_cancelled",
        mode = mode.as_str(),
        reason,
        held_ms,
        "modifier hotkey hold ended before recording start"
    );
}

// -- Both-mode arbitration state --

use hold_promotion::HoldPromotion;
use murmur_core::hold_promotion;

static HOLD_PROMOTION: HoldPromotion = HoldPromotion::new();

/// Whether a promoted hold ending with `event_type` stops the recording. A
/// hold promoted over a running recording did not start it: the held key is
/// most likely a modifier of another shortcut (Cmd+Shift+4), so the combo key
/// ends the hold without stopping. Releasing the key on its own still stops,
/// as before.
fn promoted_hold_stops_recording(over_recording: bool, event_type: &EventType) -> bool {
    !(over_recording && matches!(event_type, EventType::KeyPress(_)))
}

/// Only a key release is a release; a chord press that ends the hold also
/// publishes `hold-down-stop` but must not enter the release metric.
fn is_hold_release(event_type: &EventType) -> bool {
    matches!(event_type, EventType::KeyRelease(_))
}

/// Stamp the native release behind a `hold-down-stop` for the live recording
/// generation, so the stop can time the hotkey-to-invoke hop (#826).
fn stamp_hold_release(handle: &tauri::AppHandle, event_type: &EventType) {
    let released_at = Instant::now();
    if !is_hold_release(event_type) {
        return;
    }
    if let Some(state) = handle.try_state::<crate::State>() {
        crate::hotkey_release::record(
            state.app_state.recording_id.load(Ordering::SeqCst),
            released_at,
        );
    }
}

/// A dictation is in flight that the current hold did not start.
fn dictation_in_flight(handle: &tauri::AppHandle) -> bool {
    handle.try_state::<crate::State>().is_some_and(|state| {
        matches!(
            state.app_state.dictation.lock_or_recover().status,
            crate::state::DictationStatus::Starting
                | crate::state::DictationStatus::Recording
                | crate::state::DictationStatus::Recovering
        )
    })
}
/// Commit deferred hold `press_id` after `delay_ms` unless the press ended or the
/// listener context changed. Both mode promotes over a running recording as a
/// physical-hold mark; Hold-Down mode always publishes `hold-down-start`.
fn schedule_hold_promotion(
    app_handle: tauri::AppHandle,
    mode: DetectorMode,
    listener_generation: u64,
    press_id: u64,
    delay_ms: u128,
) {
    let candidate = if mode == DetectorMode::Both {
        current_gesture()
    } else {
        Some(Candidate::new())
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        // Never hold the promotion lock while acquiring the backend dictation
        // lock: backend transitions invalidate pending timers too.
        let over_recording = mode == DetectorMode::Both && dictation_in_flight(&app_handle);
        HOLD_PROMOTION.promote(
            press_id,
            over_recording,
            || {
                listener_context_matches(mode, listener_generation)
                    && !(mode == DetectorMode::Both && IS_PROCESSING.load(Ordering::SeqCst))
                    && !APP_DISABLED.load(Ordering::SeqCst)
                    && HOLD_DOWN_DETECTOR
                        .lock_or_recover()
                        .as_ref()
                        .is_some_and(|d| d.state == HoldState::Held)
            },
            || {
                if over_recording {
                    tracing::info!(target: "keyboard", "BOTH -> emit hold-over-recording-start");
                    crate::dictation_control::send(
                        &app_handle,
                        murmur_core::dictation::Input::HoldOverStart,
                    );
                    let _ = app_handle.emit("hold-over-recording-start", ());
                } else {
                    tracing::info!(
                        target: "keyboard",
                        mode = mode.as_str(),
                        "hold timer promoted to hold-down-start"
                    );
                    emit_hold_start(&app_handle, candidate);
                }
            },
        );
    });
}

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
        crate::audio_lifecycle::gesture::clear_pending();
        invalidate_preparation(crate::audio::prepare::InvalidateReason::Processing);
        // Entering processing: invalidate any pending hold-promotion timer
        // so it can't fire hold-down-start during active processing.
        HOLD_PROMOTION.invalidate();
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
        HOLD_PROMOTION.invalidate();
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
    crate::audio_lifecycle::gesture::clear_pending();
    let was_disabled = APP_DISABLED.swap(disabled, Ordering::SeqCst);
    if disabled {
        invalidate_preparation(crate::audio::prepare::InvalidateReason::Disabled);
    }
    if !was_disabled && disabled {
        // Transitioning false → true: clean up any partial detector state
        HOLD_PROMOTION.invalidate();
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

static PASTE_LAST_ACTIVE: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteLastShortcut {
    CommandShiftV,
    CommandOptionV,
    CommandControlV,
}

impl PasteLastShortcut {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "command_shift_v" => Some(Self::CommandShiftV),
            "command_option_v" => Some(Self::CommandOptionV),
            "command_control_v" => Some(Self::CommandControlV),
            _ => None,
        }
    }
}

pub fn is_paste_last_shortcut_id(id: &str) -> bool {
    PasteLastShortcut::from_id(id).is_some()
}

static PASTE_LAST_SHORTCUT: Mutex<Option<PasteLastShortcut>> = Mutex::new(None);
static PASTE_LAST_META_DOWN: AtomicBool = AtomicBool::new(false);
static PASTE_LAST_SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static PASTE_LAST_OPTION_DOWN: AtomicBool = AtomicBool::new(false);
static PASTE_LAST_CONTROL_DOWN: AtomicBool = AtomicBool::new(false);
static PASTE_LAST_LATCHED: AtomicBool = AtomicBool::new(false);

fn reset_paste_last_chord_state() {
    PASTE_LAST_META_DOWN.store(false, Ordering::SeqCst);
    PASTE_LAST_SHIFT_DOWN.store(false, Ordering::SeqCst);
    PASTE_LAST_OPTION_DOWN.store(false, Ordering::SeqCst);
    PASTE_LAST_CONTROL_DOWN.store(false, Ordering::SeqCst);
    PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
}

fn handle_paste_last_chord(event_type: &EventType) -> bool {
    match event_type {
        EventType::KeyPress(Key::MetaLeft | Key::MetaRight) => {
            PASTE_LAST_META_DOWN.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::MetaLeft | Key::MetaRight) => {
            PASTE_LAST_META_DOWN.store(false, Ordering::SeqCst);
            PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
        }
        EventType::KeyPress(Key::ShiftLeft | Key::ShiftRight) => {
            PASTE_LAST_SHIFT_DOWN.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::ShiftLeft | Key::ShiftRight) => {
            PASTE_LAST_SHIFT_DOWN.store(false, Ordering::SeqCst);
            PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
        }
        EventType::KeyPress(Key::Alt | Key::AltGr) => {
            PASTE_LAST_OPTION_DOWN.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::Alt | Key::AltGr) => {
            PASTE_LAST_OPTION_DOWN.store(false, Ordering::SeqCst);
            PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
        }
        EventType::KeyPress(Key::ControlLeft | Key::ControlRight) => {
            PASTE_LAST_CONTROL_DOWN.store(true, Ordering::SeqCst);
        }
        EventType::KeyRelease(Key::ControlLeft | Key::ControlRight) => {
            PASTE_LAST_CONTROL_DOWN.store(false, Ordering::SeqCst);
            PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
        }
        EventType::KeyPress(Key::KeyV)
            if PASTE_LAST_META_DOWN.load(Ordering::SeqCst)
                && PASTE_LAST_SHORTCUT.lock_or_recover().is_some_and(
                    |shortcut| match shortcut {
                        PasteLastShortcut::CommandShiftV => {
                            PASTE_LAST_SHIFT_DOWN.load(Ordering::SeqCst)
                        }
                        PasteLastShortcut::CommandOptionV => {
                            PASTE_LAST_OPTION_DOWN.load(Ordering::SeqCst)
                        }
                        PasteLastShortcut::CommandControlV => {
                            PASTE_LAST_CONTROL_DOWN.load(Ordering::SeqCst)
                        }
                    },
                )
                && !PASTE_LAST_LATCHED.swap(true, Ordering::SeqCst) =>
        {
            return true;
        }
        EventType::KeyRelease(Key::KeyV) => {
            PASTE_LAST_LATCHED.store(false, Ordering::SeqCst);
        }
        _ => {}
    }
    false
}

/// Registration must re-arm detectors after an rdev error, even when the
/// frontend confirms the same shortcut as before.
pub(crate) fn dictation_listener_active() -> bool {
    LISTENER_ACTIVE.load(Ordering::SeqCst)
}

/// Start the keyboard listener. Spawns the rdev listener thread if not already running.
/// If already running, just updates the target key, mode, and re-enables.
///
/// `mode` should be `"double_tap"`, `"hold_down"`, or `"both"`.
pub fn start_listener(app_handle: tauri::AppHandle, hotkey: &str, mode: &str) {
    crate::audio_lifecycle::gesture::clear_pending();
    LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);
    invalidate_preparation(crate::audio::prepare::InvalidateReason::ListenerChanged);
    HOLD_PROMOTION.invalidate();
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
                    d.arming_ms = hold_arming_ms(detector_mode, target);
                }
                None => {
                    let mut d = HoldDownDetector::new();
                    let _ = d.set_target(target);
                    d.arming_ms = hold_arming_ms(detector_mode, target);
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
                        d.arming_ms = hold_arming_ms(detector_mode, target);
                    }
                    None => {
                        let mut d = HoldDownDetector::new();
                        let _ = d.set_target(target);
                        d.arming_ms = hold_arming_ms(detector_mode, target);
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
pub(crate) fn ensure_listener_thread_spawned(app_handle: tauri::AppHandle) {
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
            tracing::info!(
                target: "keyboard",
                event_code = "keyboard.listener_started",
                "rdev listener thread started"
            );

            crate::launch::milestone("system.launch_listener_armed");
            let mut shortcut_modifiers = ShortcutModifierState::default();
            let mut pressed_function_keys = PressedFunctionKeys::default();
            let callback = move |raw: rdev::Event| {
                let event = Event {
                    event_type: native_event(raw.event_type),
                    time: raw.time,
                    name: None,
                };
                // Track these even while every feature is idle. If a user
                // holds Command before enabling a function-key trigger, the
                // next F-key press must still be recognized as a chord. Fn is
                // intentionally absent because Apple keyboards may require it
                // to produce F1–F12 events.
                shortcut_modifiers.update(&event.event_type);
                let repeated_function_press = pressed_function_keys.update(&event.event_type);

                // Dictation and Paste Last independently keep this thread active.
                if !LISTENER_ACTIVE.load(Ordering::SeqCst)
                    && !PASTE_LAST_ACTIVE.load(Ordering::SeqCst)
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

                if PASTE_LAST_ACTIVE.load(Ordering::SeqCst)
                    && handle_paste_last_chord(&event.event_type)
                {
                    crate::dictation_control::send(
                        &handle,
                        murmur_core::dictation::Input::PasteLast,
                    );
                }

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
                    // Invalidate pending hold-promotion timers
                    HOLD_PROMOTION.invalidate();
                    invalidate_preparation(crate::audio::prepare::InvalidateReason::Escape);

                    tracing::info!(
                        target: "keyboard",
                        event_code = "keyboard.escape_cancel",
                        "escape-cancel emitted"
                    );
                    crate::dictation_control::send(&handle, murmur_core::dictation::Input::Escape);
                    let _ = handle.emit("escape-cancel", ());
                    return;
                }

                if APP_DISABLED.load(Ordering::SeqCst) {
                    return;
                }

                if !LISTENER_ACTIVE.load(Ordering::SeqCst) {
                    return;
                }

                match mode {
                    DetectorMode::DoubleTap => {
                        let (fired, rejection, wait_started_at, prepare) = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
                            if let Some(d) = det.as_mut() {
                                let previous_wait = d.second_tap_wait_started_at();
                                let fired = d.handle_event_with_context(
                                    &event.event_type,
                                    shortcut_modifiers.any(),
                                    repeated_function_press,
                                );
                                let wait_started_at = d
                                    .second_tap_wait_started_at()
                                    .filter(|started_at| Some(*started_at) != previous_wait);
                                (
                                    fired,
                                    d.take_rejection(),
                                    wait_started_at,
                                    d.take_prepare_signal(fired),
                                )
                            } else {
                                (false, None, None, PrepareSignal::None)
                            }
                        };
                        apply_prepare_signal(&handle, prepare, mode, listener_generation);
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
                            emit_dictation_toggle(&handle);
                        }
                    }
                    DetectorMode::HoldDown => {
                        let (result, arming) = {
                            let mut det =
                                HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                let result = d.handle_event_with_context(
                                    &event.event_type,
                                    shortcut_modifiers.any(),
                                    repeated_function_press,
                                );
                                (result, HoldArming::of(d))
                            } else {
                                (HoldDownEvent::None, HoldArming::OFF)
                            }
                        };
                        dispatch_hold_down(
                            &HOLD_PROMOTION,
                            result,
                            arming,
                            &event.event_type,
                            |signal| match signal {
                                HoldDownSignal::Start => {
                                    emit_hold_start(&handle, Some(Candidate::new()));
                                }
                                HoldDownSignal::Stop => {
                                    stamp_hold_release(&handle, &event.event_type);
                                    crate::dictation_control::send(
                                        &handle,
                                        murmur_core::dictation::Input::HoldStop,
                                    );
                                    let _ = handle.emit("hold-down-stop", ());
                                }
                                HoldDownSignal::Arm {
                                    press_id,
                                    window_ms,
                                } => {
                                    schedule_hold_promotion(
                                        handle.clone(),
                                        mode,
                                        listener_generation,
                                        press_id,
                                        window_ms,
                                    );
                                }
                                HoldDownSignal::ArmingCancelled { reason, held_ms } => {
                                    log_hotkey_arming_cancelled(mode, reason, held_ms);
                                }
                            },
                        );
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
                        let (hold_result, arming, was_arming) = if !dtap_second_phase {
                            let mut det =
                                HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                            if let Some(d) = det.as_mut() {
                                let was_arming = d.is_arming();
                                let result = d.handle_event_with_context(
                                    &event.event_type,
                                    shortcut_modifiers.any(),
                                    repeated_function_press,
                                );
                                (result, HoldArming::of(d), was_arming)
                            } else {
                                (HoldDownEvent::None, HoldArming::OFF, false)
                            }
                        } else {
                            (HoldDownEvent::None, HoldArming::OFF, false)
                        };

                        // Always feed double-tap
                        let (dtap_fired, rejection, wait_started_at, prepare) = {
                            let mut det = DOUBLE_TAP_DETECTOR.lock_or_recover();
                            if let Some(d) = det.as_mut() {
                                if arming_combo_resets_double_tap(
                                    was_arming,
                                    hold_result,
                                    &event.event_type,
                                ) {
                                    d.reset();
                                    (false, None, None, d.take_prepare_signal(false))
                                } else {
                                    let previous_wait = d.second_tap_wait_started_at();
                                    let fired = d.handle_event_with_context(
                                        &event.event_type,
                                        shortcut_modifiers.any(),
                                        repeated_function_press,
                                    );
                                    let wait_started_at = d
                                        .second_tap_wait_started_at()
                                        .filter(|started_at| Some(*started_at) != previous_wait);
                                    (
                                        fired,
                                        d.take_rejection(),
                                        wait_started_at,
                                        d.take_prepare_signal(fired),
                                    )
                                }
                            } else {
                                (false, None, None, PrepareSignal::None)
                            }
                        };
                        apply_prepare_signal(&handle, prepare, mode, listener_generation);
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
                                // Don't emit hold-down-start yet — the timer
                                // promotes after MAX_HOLD_DURATION_MS.
                                schedule_hold_promotion(
                                    handle.clone(),
                                    mode,
                                    listener_generation,
                                    HOLD_PROMOTION.begin(),
                                    MAX_HOLD_DURATION_MS,
                                );
                            }
                            HoldDownEvent::Stop => {
                                let arming_cancelled = arming.cancelled_by(&event.event_type);
                                let handled = HOLD_PROMOTION.finish(|promoted| {
                                    if let Some(over_recording) = promoted {
                                        if !promoted_hold_stops_recording(over_recording, &event.event_type) {
                                            tracing::info!(target: "keyboard", "BOTH -> emit hold-over-recording-end (combo; recording continues)");
                                            crate::dictation_control::send(&handle, murmur_core::dictation::Input::HoldOverEnd);
let _ = handle.emit("hold-over-recording-end", ());
                                        } else {
                                            tracing::info!(target: "keyboard", "BOTH -> emit hold-down-stop (promoted hold)");
                                            stamp_hold_release(&handle, &event.event_type);
                                            crate::dictation_control::send(&handle, murmur_core::dictation::Input::HoldStop);
let _ = handle.emit("hold-down-stop", ());
                                        }
                                        true
                                    } else if dtap_fired {
                                        tracing::info!(target: "keyboard", "BOTH -> emit double-tap-toggle");
                                        emit_dictation_toggle(&handle);
                                        true
                                    } else {
                                        false
                                    }
                                });
                                if let (false, Some(held_ms)) = (handled, arming_cancelled) {
                                    log_hotkey_arming_cancelled(mode, "other_key", held_ms);
                                } else if !handled {
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
                                    emit_dictation_toggle(&handle);
                                }
                            }
                        }
                    }
                }
            };

            if let Err(e) = listen(callback) {
                tracing::error!(
                    target: "keyboard",
                    event_code = "keyboard.listener_failed",
                    "rdev listener error: {:?}",
                    e
                );
                LISTENER_THREAD_SPAWNED.store(false, Ordering::SeqCst);
                LISTENER_ACTIVE.store(false, Ordering::SeqCst);
                let _ = error_handle.emit("keyboard-listener-error", format!("{:?}", e));
                let retry_handle = error_handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    crate::dictation_control::refresh(&retry_handle).await;
                });
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
                    // Info, not warn: silence means "nobody touched the keyboard",
                    // which is the normal state of an idle machine, not a fault. As
                    // a warning this was 94% of all production warning volume and
                    // buried the real ones (empty transcripts, AX focus failures, a
                    // diagnostics-store failure). Info keeps the event in
                    // events.jsonl — the subscriber filters at `info`, so debug
                    // would drop it and orphan its `canonical_event_code` entry.
                    tracing::info!(
                        target: "keyboard",
                        event_code = "keyboard.listener_silent",
                        silent_for_ms = silent_for_ms,
                        threshold_ms = TAP_SILENCE_WARNING_MS,
                        "keyboard callbacks idle — inactivity alone does not establish listener failure"
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

fn configure_paste_last_shortcut(shortcut: Option<&str>) -> Result<(), String> {
    // Resolve first: a bad replacement must not tear down the working target.
    let shortcut = shortcut
        .map(|id| {
            PasteLastShortcut::from_id(id).ok_or_else(|| {
                "Unsupported Paste Last shortcut. Choose one from Settings.".to_string()
            })
        })
        .transpose()?;
    reset_paste_last_chord_state();
    *PASTE_LAST_SHORTCUT.lock_or_recover() = shortcut;
    PASTE_LAST_ACTIVE.store(shortcut.is_some(), Ordering::SeqCst);
    Ok(())
}

pub fn start_paste_last_listener(
    app_handle: tauri::AppHandle,
    shortcut: &str,
) -> Result<(), String> {
    configure_paste_last_shortcut(Some(shortcut))?;
    ensure_listener_thread_spawned(app_handle);
    Ok(())
}

pub fn stop_paste_last_listener() {
    configure_paste_last_shortcut(None).expect("disabling Paste Last is infallible");
}

/// Stop processing keyboard events (the thread stays alive but idle).
pub fn stop_listener() {
    crate::audio_lifecycle::gesture::clear_pending();
    LISTENER_ACTIVE.store(false, Ordering::SeqCst);
    LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);
    invalidate_preparation(crate::audio::prepare::InvalidateReason::ListenerChanged);

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
    HOLD_PROMOTION.invalidate();
}

/// Update the target key without stopping/restarting the listener.
/// Returns `true` if a hold-down stop event should be emitted (key changed while held).
pub fn set_target_key(hotkey: &str) -> bool {
    crate::audio_lifecycle::gesture::clear_pending();
    LISTENER_GENERATION.fetch_add(1, Ordering::SeqCst);
    invalidate_preparation(crate::audio::prepare::InvalidateReason::ListenerChanged);
    HOLD_PROMOTION.invalidate();
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
                d.arming_ms = hold_arming_ms(mode, target);
                d.set_target(target)
            } else {
                false
            }
        }
        DetectorMode::Both => {
            let was_held = {
                let mut det = HOLD_DOWN_DETECTOR.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(d) = det.as_mut() {
                    d.arming_ms = hold_arming_ms(mode, target);
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

fn native_key(key: rdev::Key) -> Key {
    match key {
        rdev::Key::Alt => Key::Alt,
        rdev::Key::AltGr => Key::AltGr,
        rdev::Key::BackQuote => Key::BackQuote,
        rdev::Key::BackSlash => Key::BackSlash,
        rdev::Key::Backspace => Key::Backspace,
        rdev::Key::CapsLock => Key::CapsLock,
        rdev::Key::Comma => Key::Comma,
        rdev::Key::ControlLeft => Key::ControlLeft,
        rdev::Key::ControlRight => Key::ControlRight,
        rdev::Key::Delete => Key::Delete,
        rdev::Key::Dot => Key::Dot,
        rdev::Key::DownArrow => Key::DownArrow,
        rdev::Key::End => Key::End,
        rdev::Key::Equal => Key::Equal,
        rdev::Key::Escape => Key::Escape,
        rdev::Key::F1 => Key::F1,
        rdev::Key::F10 => Key::F10,
        rdev::Key::F11 => Key::F11,
        rdev::Key::F12 => Key::F12,
        rdev::Key::F13 => Key::F13,
        rdev::Key::F14 => Key::F14,
        rdev::Key::F15 => Key::F15,
        rdev::Key::F16 => Key::F16,
        rdev::Key::F17 => Key::F17,
        rdev::Key::F18 => Key::F18,
        rdev::Key::F19 => Key::F19,
        rdev::Key::F2 => Key::F2,
        rdev::Key::F20 => Key::F20,
        rdev::Key::F21 => Key::F21,
        rdev::Key::F22 => Key::F22,
        rdev::Key::F23 => Key::F23,
        rdev::Key::F24 => Key::F24,
        rdev::Key::F3 => Key::F3,
        rdev::Key::F4 => Key::F4,
        rdev::Key::F5 => Key::F5,
        rdev::Key::F6 => Key::F6,
        rdev::Key::F7 => Key::F7,
        rdev::Key::F8 => Key::F8,
        rdev::Key::F9 => Key::F9,
        rdev::Key::Function => Key::Function,
        rdev::Key::Home => Key::Home,
        rdev::Key::Insert => Key::Insert,
        rdev::Key::IntlBackslash => Key::IntlBackslash,
        rdev::Key::KeyA => Key::KeyA,
        rdev::Key::KeyB => Key::KeyB,
        rdev::Key::KeyC => Key::KeyC,
        rdev::Key::KeyD => Key::KeyD,
        rdev::Key::KeyE => Key::KeyE,
        rdev::Key::KeyF => Key::KeyF,
        rdev::Key::KeyG => Key::KeyG,
        rdev::Key::KeyH => Key::KeyH,
        rdev::Key::KeyI => Key::KeyI,
        rdev::Key::KeyJ => Key::KeyJ,
        rdev::Key::KeyK => Key::KeyK,
        rdev::Key::KeyL => Key::KeyL,
        rdev::Key::KeyM => Key::KeyM,
        rdev::Key::KeyN => Key::KeyN,
        rdev::Key::KeyO => Key::KeyO,
        rdev::Key::KeyP => Key::KeyP,
        rdev::Key::KeyQ => Key::KeyQ,
        rdev::Key::KeyR => Key::KeyR,
        rdev::Key::KeyS => Key::KeyS,
        rdev::Key::KeyT => Key::KeyT,
        rdev::Key::KeyU => Key::KeyU,
        rdev::Key::KeyV => Key::KeyV,
        rdev::Key::KeyW => Key::KeyW,
        rdev::Key::KeyX => Key::KeyX,
        rdev::Key::KeyY => Key::KeyY,
        rdev::Key::KeyZ => Key::KeyZ,
        rdev::Key::Kp0 => Key::Kp0,
        rdev::Key::Kp1 => Key::Kp1,
        rdev::Key::Kp2 => Key::Kp2,
        rdev::Key::Kp3 => Key::Kp3,
        rdev::Key::Kp4 => Key::Kp4,
        rdev::Key::Kp5 => Key::Kp5,
        rdev::Key::Kp6 => Key::Kp6,
        rdev::Key::Kp7 => Key::Kp7,
        rdev::Key::Kp8 => Key::Kp8,
        rdev::Key::Kp9 => Key::Kp9,
        rdev::Key::KpDelete => Key::KpDelete,
        rdev::Key::KpDivide => Key::KpDivide,
        rdev::Key::KpMinus => Key::KpMinus,
        rdev::Key::KpMultiply => Key::KpMultiply,
        rdev::Key::KpPlus => Key::KpPlus,
        rdev::Key::KpReturn => Key::KpReturn,
        rdev::Key::LeftArrow => Key::LeftArrow,
        rdev::Key::LeftBracket => Key::LeftBracket,
        rdev::Key::MetaLeft => Key::MetaLeft,
        rdev::Key::MetaRight => Key::MetaRight,
        rdev::Key::Minus => Key::Minus,
        rdev::Key::Num0 => Key::Num0,
        rdev::Key::Num1 => Key::Num1,
        rdev::Key::Num2 => Key::Num2,
        rdev::Key::Num3 => Key::Num3,
        rdev::Key::Num4 => Key::Num4,
        rdev::Key::Num5 => Key::Num5,
        rdev::Key::Num6 => Key::Num6,
        rdev::Key::Num7 => Key::Num7,
        rdev::Key::Num8 => Key::Num8,
        rdev::Key::Num9 => Key::Num9,
        rdev::Key::PageDown => Key::PageDown,
        rdev::Key::PageUp => Key::PageUp,
        rdev::Key::Quote => Key::Quote,
        rdev::Key::Return => Key::Return,
        rdev::Key::RightArrow => Key::RightArrow,
        rdev::Key::RightBracket => Key::RightBracket,
        rdev::Key::SemiColon => Key::SemiColon,
        rdev::Key::ShiftLeft => Key::ShiftLeft,
        rdev::Key::ShiftRight => Key::ShiftRight,
        rdev::Key::Slash => Key::Slash,
        rdev::Key::Space => Key::Space,
        rdev::Key::Tab => Key::Tab,
        rdev::Key::UpArrow => Key::UpArrow,
        rdev::Key::VolumeUp => Key::VolumeUp,
        rdev::Key::Unknown(code) => Key::Unknown(code),
        _ => Key::Unknown(u32::MAX),
    }
}
fn native_button(button: rdev::Button) -> Button {
    match button {
        rdev::Button::Left => Button::Left,
        rdev::Button::Right => Button::Right,
        rdev::Button::Middle => Button::Middle,
        rdev::Button::Unknown(n) => Button::Unknown(n),
    }
}
fn native_event(event: rdev::EventType) -> EventType {
    match event {
        rdev::EventType::KeyPress(k) => EventType::KeyPress(native_key(k)),
        rdev::EventType::KeyRelease(k) => EventType::KeyRelease(native_key(k)),
        rdev::EventType::ButtonPress(b) => EventType::ButtonPress(native_button(b)),
        rdev::EventType::ButtonRelease(b) => EventType::ButtonRelease(native_button(b)),
        rdev::EventType::Wheel { delta_x, delta_y } => EventType::Wheel { delta_x, delta_y },
        rdev::EventType::MouseMove { x, y } => EventType::MouseMove { x, y },
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
    fn paste_last_chord_uses_configured_binding_and_latches_key_repeat() {
        stop_paste_last_listener();
        *PASTE_LAST_SHORTCUT.lock_or_recover() = Some(PasteLastShortcut::CommandShiftV);
        assert!(!handle_paste_last_chord(&press(Key::MetaLeft)));
        assert!(!handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&release(Key::KeyV)));
        assert!(!handle_paste_last_chord(&press(Key::ShiftLeft)));
        assert!(handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&release(Key::KeyV)));
        assert!(handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&release(Key::MetaLeft)));
        assert!(!handle_paste_last_chord(&release(Key::ShiftLeft)));
    }

    #[test]
    fn paste_last_reconfiguration_unregisters_the_previous_chord() {
        stop_paste_last_listener();
        *PASTE_LAST_SHORTCUT.lock_or_recover() = Some(PasteLastShortcut::CommandShiftV);
        assert!(!handle_paste_last_chord(&press(Key::MetaLeft)));
        assert!(!handle_paste_last_chord(&press(Key::ShiftLeft)));
        assert!(handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&release(Key::KeyV)));

        reset_paste_last_chord_state();
        *PASTE_LAST_SHORTCUT.lock_or_recover() = Some(PasteLastShortcut::CommandOptionV);
        assert!(!handle_paste_last_chord(&press(Key::MetaLeft)));
        assert!(!handle_paste_last_chord(&press(Key::ShiftLeft)));
        assert!(!handle_paste_last_chord(&press(Key::KeyV)));
        assert!(!handle_paste_last_chord(&release(Key::KeyV)));
        assert!(!handle_paste_last_chord(&press(Key::Alt)));
        assert!(handle_paste_last_chord(&press(Key::KeyV)));

        stop_paste_last_listener();
        assert_eq!(*PASTE_LAST_SHORTCUT.lock_or_recover(), None);
        assert!(!handle_paste_last_chord(&release(Key::KeyV)));
    }

    #[test]
    fn paste_last_shortcut_ids_are_allow_listed() {
        assert_eq!(
            PasteLastShortcut::from_id("command_shift_v"),
            Some(PasteLastShortcut::CommandShiftV)
        );
        assert_eq!(
            PasteLastShortcut::from_id("command_control_v"),
            Some(PasteLastShortcut::CommandControlV)
        );
        assert_eq!(PasteLastShortcut::from_id("shift_l"), None);
        assert_eq!(PasteLastShortcut::from_id("command_shift_p"), None);
        assert_eq!(PasteLastShortcut::from_id(""), None);
    }

    #[test]
    fn paste_last_invalid_reconfiguration_preserves_the_working_binding() {
        stop_paste_last_listener();
        configure_paste_last_shortcut(Some("command_control_v")).unwrap();
        assert!(PASTE_LAST_ACTIVE.load(Ordering::SeqCst));
        assert_eq!(
            *PASTE_LAST_SHORTCUT.lock_or_recover(),
            Some(PasteLastShortcut::CommandControlV)
        );

        assert!(configure_paste_last_shortcut(Some("command_shift_p")).is_err());
        assert!(PASTE_LAST_ACTIVE.load(Ordering::SeqCst));
        assert_eq!(
            *PASTE_LAST_SHORTCUT.lock_or_recover(),
            Some(PasteLastShortcut::CommandControlV)
        );

        configure_paste_last_shortcut(None).unwrap();
        assert!(!PASTE_LAST_ACTIVE.load(Ordering::SeqCst));
        assert_eq!(*PASTE_LAST_SHORTCUT.lock_or_recover(), None);
    }

    #[test]
    fn basic_double_tap_fires() {
        let mut d = make_detector(Key::ShiftLeft);

        // First tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);
        let first_press = d.candidate.unwrap();

        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondDown);

        // Second tap: press then release quickly
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::WaitingSecondUp);

        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert_eq!(d.state, DetectorState::Idle);
        let fired = d.fired_candidate.unwrap();
        assert_eq!(fired.token, first_press.token);
        assert_eq!(fired.pressed, first_press.pressed);
        assert!(fired.released.is_some());
        assert!(d.candidate.is_none());
    }

    fn assert_tap_cancelled_by(during_first: bool, interrupt: EventType) {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        if !during_first {
            d.handle_event(&release(Key::ShiftLeft));
            d.handle_event(&press(Key::ShiftLeft));
        }
        assert!(!d.handle_event(&interrupt));
        assert_eq!(d.state, DetectorState::Idle, "{interrupt:?}");
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn other_modifier_mouse_button_or_wheel_during_either_press_is_not_a_clean_tap() {
        for during_first in [true, false] {
            for interrupt in [
                press(Key::MetaLeft),
                press(Key::Alt),
                press(Key::ShiftRight),
                EventType::ButtonPress(Button::Left),
                EventType::Wheel {
                    delta_x: 0,
                    delta_y: 1,
                },
            ] {
                assert_tap_cancelled_by(during_first, interrupt);
            }
        }
    }

    #[test]
    fn pointer_movement_does_not_break_a_clean_double_tap() {
        let mut d = make_detector(Key::ShiftLeft);
        let moved = EventType::MouseMove { x: 1.0, y: 2.0 };
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&moved);
        d.handle_event(&release(Key::ShiftLeft));
        d.handle_event(&moved);
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&moved);
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn command_shift_four_during_recording_does_not_stop_it() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;
        for event in [
            press(Key::MetaLeft),
            press(Key::ShiftLeft),
            press(Key::Num4),
            release(Key::Num4),
            release(Key::ShiftLeft),
            release(Key::MetaLeft),
        ] {
            assert!(!d.handle_event(&event), "{event:?}");
        }
        for event in [
            press(Key::ShiftLeft),
            press(Key::MetaLeft),
            press(Key::Num4),
            release(Key::Num4),
            release(Key::MetaLeft),
            release(Key::ShiftLeft),
        ] {
            assert!(!d.handle_event(&event), "{event:?}");
        }
    }

    #[test]
    fn first_clean_release_requests_preparation_and_fire_keeps_it_for_adoption() {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        assert!(matches!(d.take_prepare_signal(false), PrepareSignal::None));
        let token = d.candidate.unwrap().token;
        d.handle_event(&release(Key::ShiftLeft));
        assert!(matches!(
            d.take_prepare_signal(false),
            PrepareSignal::Request(candidate) if candidate.token == token
        ));
        d.handle_event(&press(Key::ShiftLeft));
        assert!(matches!(d.take_prepare_signal(false), PrepareSignal::None));
        let fired = d.handle_event(&release(Key::ShiftLeft));
        assert!(fired);
        assert!(matches!(d.take_prepare_signal(fired), PrepareSignal::None));
        assert_eq!(d.fired_candidate.unwrap().token, token);
        assert!(d.prepared_token.is_none());
    }

    #[test]
    fn abandoned_sequence_signals_preparation_teardown() {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert!(matches!(
            d.take_prepare_signal(false),
            PrepareSignal::Request(_)
        ));
        d.handle_event(&press(Key::KeyA));
        assert!(matches!(
            d.take_prepare_signal(false),
            PrepareSignal::Abandon
        ));
        assert!(matches!(d.take_prepare_signal(false), PrepareSignal::None));
    }

    #[test]
    fn lifecycle_reset_rejects_signal_resumed_before_its_generation_snapshot() {
        for revoke_and_resume in [
            (|| {
                set_app_disabled(true);
                set_app_disabled(false);
            }) as fn(),
            || {
                set_processing(true);
                set_processing(false);
            },
        ] {
            let mut detector = make_detector(Key::ShiftLeft);
            detector.handle_event(&press(Key::ShiftLeft));
            detector.handle_event(&release(Key::ShiftLeft));
            let PrepareSignal::Request(candidate) = detector.take_prepare_signal(false) else {
                panic!("first clean tap must request preparation");
            };
            let saved = DOUBLE_TAP_DETECTOR.lock_or_recover().replace(detector);
            // The signal exists, but apply_prepare_signal has not sampled
            // revocation yet. A complete disable/enable cycle happens first.
            revoke_and_resume();
            let _fresh_generation = crate::audio::prepare::revocation_generation();
            let mut detector = DOUBLE_TAP_DETECTOR.lock_or_recover();
            assert!(!detector
                .as_ref()
                .unwrap()
                .preparation_candidate_matches(candidate.token));
            *detector = saved;
        }
    }

    #[test]
    fn single_tap_stop_while_recording_never_prepares() {
        let mut d = make_detector(Key::ShiftLeft);
        d.recording = true;
        d.handle_event(&press(Key::ShiftLeft));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
        assert!(matches!(d.take_prepare_signal(true), PrepareSignal::None));
    }

    #[test]
    fn second_tap_expiry_clears_preparation_ownership() {
        let mut d = make_detector(Key::ShiftLeft);
        d.handle_event(&press(Key::ShiftLeft));
        d.handle_event(&release(Key::ShiftLeft));
        assert!(matches!(
            d.take_prepare_signal(false),
            PrepareSignal::Request(_)
        ));
        let started = d.second_tap_wait_started_at().unwrap();
        d.state_entered_at = started - Duration::from_millis(DOUBLE_TAP_WINDOW_MS as u64 + 1);
        assert!(d.expire_second_tap_wait(d.state_entered_at).is_some());
        assert!(d.prepared_token.is_none());
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
        assert_eq!(
            DICTATION_KEY_IDS,
            [
                "shift_l", "alt_l", "ctrl_r", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9",
                "f10", "f11", "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20",
            ]
        );
        assert_eq!(hotkey_to_rdev_key("shift_l"), Some(Key::ShiftLeft));
        assert_eq!(hotkey_to_rdev_key("alt_l"), Some(Key::Alt));
        assert_eq!(hotkey_to_rdev_key("ctrl_r"), Some(Key::ControlRight));
        let function_keys = [
            Key::F1,
            Key::F2,
            Key::F3,
            Key::F4,
            Key::F5,
            Key::F6,
            Key::F7,
            Key::F8,
            Key::F9,
            Key::F10,
            Key::F11,
            Key::F12,
            Key::F13,
            Key::F14,
            Key::F15,
            Key::F16,
            Key::F17,
            Key::F18,
            Key::F19,
            Key::F20,
        ];
        for (index, key) in function_keys.into_iter().enumerate() {
            assert_eq!(hotkey_to_rdev_key(&format!("f{}", index + 1)), Some(key));
        }
        assert_eq!(hotkey_to_rdev_key("unknown"), None);
    }

    #[test]
    fn transform_hotkey_string_mapping() {
        // TransformKey ids (settings.ts) — distinct from the dictation
        // DictationKey ids above, so both hotkeys can be configured at once
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
    fn only_a_key_release_stop_is_stamped_as_a_hotkey_release() {
        // Hold-Down mode and Both mode's promoted hold stamp through the same
        // predicate: a chord press that ends the hold is a stop, not a release.
        let mut d = make_hold_detector(Key::ShiftLeft);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        let chord = press(Key::KeyA);
        assert_eq!(d.handle_event(&chord), HoldDownEvent::Stop);
        assert!(!is_hold_release(&chord));
        assert!(promoted_hold_stops_recording(false, &chord));

        let mut d = make_hold_detector(Key::ShiftLeft);
        assert_eq!(d.handle_event(&press(Key::ShiftLeft)), HoldDownEvent::Start);
        let released = release(Key::ShiftLeft);
        assert_eq!(d.handle_event(&released), HoldDownEvent::Stop);
        assert!(is_hold_release(&released));
        assert!(promoted_hold_stops_recording(true, &released));
    }

    #[test]
    fn function_key_hold_handles_repeats_combo_cancellation_and_stale_release() {
        let mut d = make_hold_detector(Key::F8);

        assert_eq!(d.handle_event(&press(Key::F8)), HoldDownEvent::Start);
        assert_eq!(d.handle_event(&press(Key::F8)), HoldDownEvent::None);
        assert_eq!(d.state, HoldState::Held);

        assert_eq!(d.handle_event(&press(Key::KeyA)), HoldDownEvent::Stop);
        assert_eq!(d.state, HoldState::Idle);
        assert_eq!(
            d.handle_event_with_context(&press(Key::F8), false, true),
            HoldDownEvent::None,
            "an autorepeat before physical release stays suppressed"
        );
        assert_eq!(d.handle_event(&release(Key::F8)), HoldDownEvent::None);
        d.last_stopped_at = None;
        assert_eq!(d.handle_event(&press(Key::F8)), HoldDownEvent::Start);
    }

    #[test]
    fn shortcut_modifiers_block_function_key_starts_but_allow_fn() {
        for modifier in [
            Key::ShiftLeft,
            Key::ShiftRight,
            Key::Alt,
            Key::AltGr,
            Key::ControlLeft,
            Key::ControlRight,
            Key::MetaLeft,
            Key::MetaRight,
        ] {
            let mut modifiers = ShortcutModifierState::default();
            modifiers.update(&press(modifier));
            assert!(modifiers.any(), "{modifier:?}");

            let mut hold = make_hold_detector(Key::F8);
            assert_eq!(
                hold.handle_event_with_context(&press(Key::F8), modifiers.any(), false),
                HoldDownEvent::None,
                "{modifier:?}+F8 must not start a hold"
            );
            let mut double_tap = make_detector(Key::F8);
            assert!(
                !double_tap.handle_event_with_shortcut_modifiers(&press(Key::F8), modifiers.any(),)
            );
            assert_eq!(double_tap.state, DetectorState::Idle, "{modifier:?}+F8");
            double_tap.recording = true;
            assert!(
                !double_tap.handle_event_with_shortcut_modifiers(&press(Key::F8), modifiers.any(),)
            );
            assert_eq!(
                double_tap.state,
                DetectorState::Idle,
                "{modifier:?}+F8 must not stop a recording"
            );

            modifiers.update(&release(modifier));
            assert!(!modifiers.any(), "{modifier:?}");
            assert_eq!(
                hold.handle_event_with_context(&press(Key::F8), modifiers.any(), true),
                HoldDownEvent::None,
                "releasing {modifier:?} must not reinterpret F8 autorepeat"
            );
            assert!(!double_tap.handle_event_with_context(&press(Key::F8), modifiers.any(), true,));
            assert_eq!(double_tap.state, DetectorState::Idle);
            assert_eq!(hold.handle_event(&release(Key::F8)), HoldDownEvent::None);
            assert!(!double_tap.handle_event(&release(Key::F8)));
            assert_eq!(hold.handle_event(&press(Key::F8)), HoldDownEvent::Start);
        }

        let mut modifiers = ShortcutModifierState::default();
        modifiers.update(&press(Key::Function));
        assert!(
            !modifiers.any(),
            "Fn must remain available for Apple function rows"
        );
        let mut hold = make_hold_detector(Key::F8);
        assert_eq!(
            hold.handle_event_with_shortcut_modifiers(&press(Key::F8), modifiers.any()),
            HoldDownEvent::Start
        );
    }

    #[test]
    fn shortcut_modifier_tracking_keeps_sides_independent_and_modifier_triggers_working() {
        let mut modifiers = ShortcutModifierState::default();
        modifiers.update(&press(Key::ShiftLeft));
        modifiers.update(&press(Key::ShiftRight));
        modifiers.update(&release(Key::ShiftLeft));
        assert!(modifiers.any(), "right Shift is still held");

        let mut hold = make_hold_detector(Key::ShiftRight);
        assert_eq!(
            hold.handle_event_with_shortcut_modifiers(&press(Key::ShiftRight), modifiers.any()),
            HoldDownEvent::Start
        );
        modifiers.update(&release(Key::ShiftRight));
        assert!(!modifiers.any());
    }

    #[test]
    fn shortcut_modifier_during_function_hold_stops_once_and_clears_state() {
        let mut hold = make_hold_detector(Key::F8);
        assert_eq!(hold.handle_event(&press(Key::F8)), HoldDownEvent::Start);
        assert_eq!(
            hold.handle_event_with_shortcut_modifiers(&press(Key::MetaLeft), true),
            HoldDownEvent::Stop
        );
        assert_eq!(hold.state, HoldState::Idle);
        assert_eq!(
            hold.handle_event_with_context(&press(Key::F8), false, true),
            HoldDownEvent::None,
            "Command release followed by F8 autorepeat stays suppressed"
        );
        assert_eq!(hold.handle_event(&release(Key::F8)), HoldDownEvent::None);
        hold.last_stopped_at = None;
        assert_eq!(hold.handle_event(&press(Key::F8)), HoldDownEvent::Start);
    }

    #[test]
    fn shortcut_modifier_cancels_each_function_double_tap_phase() {
        let mut first_down = make_detector(Key::F8);
        first_down.handle_event(&press(Key::F8));
        assert_eq!(first_down.state, DetectorState::WaitingFirstUp);
        assert!(!first_down.handle_event_with_shortcut_modifiers(&press(Key::MetaLeft), true,));
        assert_eq!(first_down.state, DetectorState::Idle);
        assert!(!first_down.handle_event_with_context(&press(Key::F8), false, true));
        assert_eq!(first_down.state, DetectorState::Idle);
        assert!(!first_down.handle_event(&release(Key::F8)));
        assert!(!first_down.handle_event(&press(Key::F8)));
        assert_eq!(first_down.state, DetectorState::WaitingFirstUp);

        let mut between_taps = make_detector(Key::F8);
        between_taps.handle_event(&press(Key::F8));
        between_taps.handle_event(&release(Key::F8));
        assert_eq!(between_taps.state, DetectorState::WaitingSecondDown);
        assert!(!between_taps.handle_event_with_shortcut_modifiers(&press(Key::ControlLeft), true,));
        assert_eq!(between_taps.state, DetectorState::Idle);

        let mut second_down = make_detector(Key::F8);
        second_down.handle_event(&press(Key::F8));
        second_down.handle_event(&release(Key::F8));
        second_down.handle_event(&press(Key::F8));
        assert_eq!(second_down.state, DetectorState::WaitingSecondUp);
        assert!(!second_down.handle_event_with_shortcut_modifiers(&press(Key::Alt), true,));
        assert_eq!(second_down.state, DetectorState::Idle);
        assert!(!second_down.handle_event_with_context(&press(Key::F8), false, true));
        assert_eq!(second_down.state, DetectorState::Idle);
        assert!(!second_down.handle_event(&release(Key::F8)));
        assert!(!second_down.handle_event(&press(Key::F8)));
        assert_eq!(second_down.state, DetectorState::WaitingFirstUp);

        let mut stopping = make_detector(Key::F8);
        stopping.recording = true;
        stopping.handle_event(&press(Key::F8));
        assert!(!stopping.handle_event_with_shortcut_modifiers(&press(Key::ShiftLeft), true,));
        assert!(!stopping.handle_event(&release(Key::F8)));
        assert_eq!(stopping.state, DetectorState::Idle);
    }

    #[test]
    fn changing_function_key_target_accepts_the_new_physical_key() {
        let mut hold = make_hold_detector(Key::F8);
        assert_eq!(
            hold.handle_event_with_shortcut_modifiers(&press(Key::F8), true),
            HoldDownEvent::None
        );
        assert!(!hold.set_target(Some(Key::F9)));
        assert_eq!(hold.handle_event(&press(Key::F9)), HoldDownEvent::Start);

        let mut double_tap = make_detector(Key::F8);
        assert!(!double_tap.handle_event_with_shortcut_modifiers(&press(Key::F8), true));
        double_tap.set_target(Some(Key::F9));
        assert!(!double_tap.handle_event(&press(Key::F9)));
        assert_eq!(double_tap.state, DetectorState::WaitingFirstUp);
    }

    #[test]
    fn physical_function_key_state_marks_repeats_until_the_matching_release() {
        let mut pressed = PressedFunctionKeys::default();
        assert!(!pressed.update(&press(Key::F8)));
        assert!(pressed.update(&press(Key::F8)));
        assert!(!pressed.update(&release(Key::F9)));
        assert!(pressed.update(&press(Key::F8)));
        assert!(!pressed.update(&release(Key::F8)));
        assert!(!pressed.update(&press(Key::F8)));
        assert!(!pressed.update(&press(Key::Function)));
    }

    #[test]
    fn function_key_double_tap_handles_repeats_and_combo_cancellation() {
        let mut d = make_detector(Key::F12);

        assert!(!d.handle_event(&press(Key::F12)));
        assert!(!d.handle_event(&press(Key::F12)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);
        assert!(!d.handle_event(&release(Key::F12)));
        assert!(!d.handle_event(&press(Key::KeyA)));
        assert_eq!(d.state, DetectorState::Idle);

        assert!(!d.handle_event(&press(Key::F12)));
        assert!(!d.handle_event(&release(Key::F12)));
        assert!(!d.handle_event(&press(Key::F12)));
        assert!(d.handle_event(&release(Key::F12)));
    }

    #[test]
    fn timed_out_function_key_repeat_stays_suppressed_until_release() {
        let mut d = make_detector(Key::F12);
        assert!(!d.handle_event(&press(Key::F12)));
        d.state_entered_at = Instant::now() - Duration::from_millis(250);

        assert!(!d.handle_event_with_context(&press(Key::F12), false, true));
        assert_eq!(d.state, DetectorState::Idle);
        assert!(!d.handle_event_with_context(&press(Key::F12), false, true));
        assert_eq!(d.state, DetectorState::Idle);
        assert!(!d.handle_event(&release(Key::F12)));
        assert!(!d.handle_event(&press(Key::F12)));
        assert_eq!(d.state, DetectorState::WaitingFirstUp);
    }

    #[test]
    fn resetting_function_key_detectors_cancels_partial_sequences() {
        let mut hold = make_hold_detector(Key::F20);
        let mut double_tap = make_detector(Key::F20);

        assert_eq!(hold.handle_event(&press(Key::F20)), HoldDownEvent::Start);
        assert!(!double_tap.handle_event(&press(Key::F20)));
        hold.reset();
        double_tap.reset();

        assert_eq!(
            hold.handle_event_with_context(&press(Key::F20), false, true),
            HoldDownEvent::None
        );
        assert!(!double_tap.handle_event_with_context(&press(Key::F20), false, true));
        assert_eq!(hold.handle_event(&release(Key::F20)), HoldDownEvent::None);
        assert!(!double_tap.handle_event(&release(Key::F20)));
        assert_eq!(hold.state, HoldState::Idle);
        assert_eq!(double_tap.state, DetectorState::Idle);
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
        HoldOverRecordingEnd,
        DoubleTapToggle,
    }

    /// Simulate the Both-mode deferred-hold arbitration logic.
    /// `promoted` simulates whether the timer thread promoted the press
    /// to a real hold.
    fn both_handle_event(
        hold: &mut HoldDownDetector,
        dtap: &mut DoubleTapDetector,
        event_type: &EventType,
        promoted: bool,
    ) -> Vec<BothEmit> {
        both_handle_event_over(hold, dtap, event_type, promoted, false)
    }

    /// `over_recording` simulates a hold the timer promoted while a recording
    /// it did not start was already running.
    fn both_handle_event_over(
        hold: &mut HoldDownDetector,
        dtap: &mut DoubleTapDetector,
        event_type: &EventType,
        promoted: bool,
        over_recording: bool,
    ) -> Vec<BothEmit> {
        // Check dtap phase BEFORE feeding — also verify the window hasn't expired
        let dtap_second_phase = matches!(
            dtap.state,
            DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
        ) && dtap.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS;

        // Only feed hold-down when NOT in second phase
        let (hold_result, was_arming) = if !dtap_second_phase {
            let was_arming = hold.is_arming();
            (hold.handle_event(event_type), was_arming)
        } else {
            (HoldDownEvent::None, false)
        };

        // Always feed double-tap, except the typed key that ended an arming
        // window (production resets it directly).
        let dtap_fired = if arming_combo_resets_double_tap(was_arming, hold_result, event_type) {
            dtap.reset();
            false
        } else {
            dtap.handle_event(event_type)
        };
        let mut emitted = Vec::new();

        match hold_result {
            HoldDownEvent::Start => {
                // In real code: spawns a timer thread, no synchronous emission
            }
            HoldDownEvent::Stop => {
                if promoted && !promoted_hold_stops_recording(over_recording, event_type) {
                    // A combo key during a recording the hold did not start.
                    emitted.push(BothEmit::HoldOverRecordingEnd);
                } else if promoted {
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
    fn function_key_works_in_both_mode_for_hold_and_double_tap() {
        let mut hold = make_hold_detector(Key::F6);
        let mut double_tap = make_detector(Key::F6);

        assert!(both_handle_event(&mut hold, &mut double_tap, &press(Key::F6), false).is_empty());
        assert_eq!(
            both_handle_event(&mut hold, &mut double_tap, &release(Key::F6), true),
            vec![BothEmit::HoldStop]
        );

        let mut hold = make_hold_detector(Key::F6);
        let mut double_tap = make_detector(Key::F6);
        assert!(both_handle_event(&mut hold, &mut double_tap, &press(Key::F6), false).is_empty());
        assert!(both_handle_event(&mut hold, &mut double_tap, &release(Key::F6), false).is_empty());
        assert!(both_handle_event(&mut hold, &mut double_tap, &press(Key::F6), false).is_empty());
        assert_eq!(
            both_handle_event(&mut hold, &mut double_tap, &release(Key::F6), false),
            vec![BothEmit::DoubleTapToggle]
        );
    }

    #[test]
    fn shortcut_modifier_cancels_both_mode_function_press_before_promotion() {
        let mut hold = make_hold_detector(Key::F6);
        let mut double_tap = make_detector(Key::F6);
        assert_eq!(hold.handle_event(&press(Key::F6)), HoldDownEvent::Start);
        assert!(!double_tap.handle_event(&press(Key::F6)));

        assert_eq!(
            hold.handle_event_with_shortcut_modifiers(&press(Key::MetaLeft), true),
            HoldDownEvent::Stop
        );
        assert!(!double_tap.handle_event_with_shortcut_modifiers(&press(Key::MetaLeft), true,));
        assert_eq!(hold.state, HoldState::Idle);
        assert_eq!(double_tap.state, DetectorState::Idle);
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
    fn both_combo_during_toggle_recording_keeps_recording() {
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);

        // Cmd+Shift+4 while a double-tap recording runs: Shift is held past
        // promotion (over the running recording), then the 4 ends the hold.
        // Recording continues; only the physical-hold mark is lifted.
        let e = both_handle_event_over(&mut hold, &mut dtap, &press(Key::ShiftLeft), false, false);
        assert_eq!(e, vec![]);
        let e = both_handle_event_over(&mut hold, &mut dtap, &press(Key::Num4), true, true);
        assert_eq!(e, vec![BothEmit::HoldOverRecordingEnd]);

        // Releasing the trigger alone during that recording still stops it.
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event_over(&mut hold, &mut dtap, &press(Key::ShiftLeft), false, false);
        let e = both_handle_event_over(&mut hold, &mut dtap, &release(Key::ShiftLeft), true, true);
        assert_eq!(e, vec![BothEmit::HoldStop]);

        // A hold that started its own recording still stops on a combo.
        let mut hold = make_hold_detector(Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event_over(&mut hold, &mut dtap, &press(Key::ShiftLeft), false, false);
        let e = both_handle_event_over(&mut hold, &mut dtap, &press(Key::Num4), true, false);
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

    // -- Modifier arming window --

    fn make_armed_hold_detector(mode: DetectorMode, key: Key) -> HoldDownDetector {
        let mut d = make_hold_detector(key);
        d.arming_ms = hold_arming_ms(mode, Some(key));
        d
    }

    /// Feed one event through the Hold-Down detector and the production
    /// dispatch, recording every signal the listener would act on.
    fn hold_down_step(
        hold: &mut HoldDownDetector,
        promotion: &HoldPromotion,
        event_type: &EventType,
        signals: &mut Vec<HoldDownSignal>,
    ) {
        let result = hold.handle_event(event_type);
        let arming = HoldArming::of(hold);
        dispatch_hold_down(promotion, result, arming, event_type, |s| signals.push(s));
    }

    /// Run the arming timer exactly as `schedule_hold_promotion` would once
    /// its window closes. Returns whether it published `hold-down-start`.
    fn fire_arming_timer(
        hold: &HoldDownDetector,
        promotion: &HoldPromotion,
        signals: &mut Vec<HoldDownSignal>,
    ) -> bool {
        let press_id = signals.iter().rev().find_map(|s| match s {
            HoldDownSignal::Arm { press_id, .. } => Some(*press_id),
            _ => None,
        });
        let Some(press_id) = press_id else {
            return false;
        };
        promotion.promote(
            press_id,
            false,
            || hold.state == HoldState::Held,
            || signals.push(HoldDownSignal::Start),
        )
    }

    #[test]
    fn arming_window_applies_only_to_modifier_hold_triggers() {
        for key in [Key::ShiftLeft, Key::Alt, Key::ControlRight, Key::Function] {
            assert_eq!(
                hold_arming_ms(DetectorMode::HoldDown, Some(key)),
                MODIFIER_HOLD_ARMING_MS
            );
            assert_eq!(
                hold_arming_ms(DetectorMode::Both, Some(key)),
                MAX_HOLD_DURATION_MS
            );
            assert_eq!(hold_arming_ms(DetectorMode::DoubleTap, Some(key)), 0);
        }
        for mode in [DetectorMode::HoldDown, DetectorMode::Both] {
            assert_eq!(hold_arming_ms(mode, Some(Key::F8)), 0);
            assert_eq!(hold_arming_ms(mode, None), 0);
        }
    }

    #[test]
    fn hold_down_shift_letter_never_starts_recording() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();

        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        assert!(matches!(
            signals.as_slice(),
            [HoldDownSignal::Arm {
                window_ms: MODIFIER_HOLD_ARMING_MS,
                ..
            }]
        ));
        hold_down_step(&mut hold, &promotion, &press(Key::KeyA), &mut signals);
        hold_down_step(&mut hold, &promotion, &release(Key::KeyA), &mut signals);
        hold_down_step(
            &mut hold,
            &promotion,
            &release(Key::ShiftLeft),
            &mut signals,
        );
        assert_eq!(hold.state, HoldState::Idle);

        // The timer that would have spawned capture finds nothing to start.
        assert!(!fire_arming_timer(&hold, &promotion, &mut signals));
        // One content-free cancellation; no start, no stop.
        assert!(matches!(
            &signals[1..],
            [HoldDownSignal::ArmingCancelled {
                reason: "other_key",
                ..
            }]
        ));
    }

    #[test]
    fn hold_down_shift_with_another_modifier_never_starts_recording() {
        // Shift, then Command within the window (Shift+Command+Z): a shortcut.
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        hold_down_step(&mut hold, &promotion, &press(Key::MetaLeft), &mut signals);
        assert_eq!(hold.state, HoldState::Idle);
        assert!(!fire_arming_timer(&hold, &promotion, &mut signals));
        assert!(matches!(
            signals.last(),
            Some(HoldDownSignal::ArmingCancelled {
                reason: "other_key",
                ..
            })
        ));
        assert!(!signals.contains(&HoldDownSignal::Start));
    }

    #[test]
    fn hold_down_mouse_click_or_wheel_while_arming_never_starts_recording() {
        for interrupt in [
            EventType::ButtonPress(Button::Left),
            EventType::Wheel {
                delta_x: 0,
                delta_y: -1,
            },
        ] {
            let promotion = HoldPromotion::new();
            let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
            let mut signals = Vec::new();
            hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
            hold_down_step(&mut hold, &promotion, &interrupt, &mut signals);
            assert_eq!(hold.state, HoldState::Idle);
            assert!(!fire_arming_timer(&hold, &promotion, &mut signals));
            assert!(!signals.contains(&HoldDownSignal::Start));
        }
    }

    #[test]
    fn hold_down_short_shift_tap_never_starts_recording() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        hold_down_step(
            &mut hold,
            &promotion,
            &release(Key::ShiftLeft),
            &mut signals,
        );
        assert!(!fire_arming_timer(&hold, &promotion, &mut signals));
        assert!(matches!(
            signals.last(),
            Some(HoldDownSignal::ArmingCancelled {
                reason: "released",
                ..
            })
        ));
        assert!(!signals.contains(&HoldDownSignal::Start));
        assert!(!signals.contains(&HoldDownSignal::Stop));
    }

    #[test]
    fn hold_down_shift_held_alone_past_window_starts_and_stops() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        assert!(hold.is_arming());
        sleep(Duration::from_millis(MODIFIER_HOLD_ARMING_MS as u64 + 10));
        assert!(!hold.is_arming());
        assert!(fire_arming_timer(&hold, &promotion, &mut signals));

        // After the window, modifiers keep today's rule (ignored while held).
        hold_down_step(&mut hold, &promotion, &press(Key::MetaLeft), &mut signals);
        assert_eq!(hold.state, HoldState::Held);
        hold_down_step(&mut hold, &promotion, &release(Key::MetaLeft), &mut signals);
        hold_down_step(
            &mut hold,
            &promotion,
            &release(Key::ShiftLeft),
            &mut signals,
        );
        assert!(matches!(
            signals.as_slice(),
            [
                HoldDownSignal::Arm { .. },
                HoldDownSignal::Start,
                HoldDownSignal::Stop
            ]
        ));
    }

    #[test]
    fn hold_down_combo_after_window_still_stops_the_started_recording() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        sleep(Duration::from_millis(MODIFIER_HOLD_ARMING_MS as u64 + 10));
        assert!(fire_arming_timer(&hold, &promotion, &mut signals));
        hold_down_step(&mut hold, &promotion, &press(Key::KeyA), &mut signals);
        assert_eq!(hold.state, HoldState::Idle);
        assert_eq!(signals[1..], [HoldDownSignal::Start, HoldDownSignal::Stop]);
    }

    #[test]
    fn hold_down_function_key_trigger_starts_immediately() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::F8);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::F8), &mut signals);
        assert_eq!(signals, [HoldDownSignal::Start]);
        hold_down_step(&mut hold, &promotion, &press(Key::KeyA), &mut signals);
        assert_eq!(signals, [HoldDownSignal::Start, HoldDownSignal::Stop]);

        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::F8);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::F8), &mut signals);
        hold_down_step(&mut hold, &promotion, &release(Key::F8), &mut signals);
        assert_eq!(signals, [HoldDownSignal::Start, HoldDownSignal::Stop]);
    }

    #[test]
    fn hold_down_new_press_after_cancel_arms_a_fresh_window() {
        let promotion = HoldPromotion::new();
        let mut hold = make_armed_hold_detector(DetectorMode::HoldDown, Key::ShiftLeft);
        let mut signals = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut signals);
        hold_down_step(&mut hold, &promotion, &press(Key::KeyA), &mut signals);
        hold_down_step(
            &mut hold,
            &promotion,
            &release(Key::ShiftLeft),
            &mut signals,
        );
        let stale = match signals[0] {
            HoldDownSignal::Arm { press_id, .. } => press_id,
            other => panic!("expected arm, got {other:?}"),
        };
        sleep(Duration::from_millis(HOLD_DOWN_COOLDOWN_MS as u64 + 10));

        let mut fresh = Vec::new();
        hold_down_step(&mut hold, &promotion, &press(Key::ShiftLeft), &mut fresh);
        // The earlier press's timer cannot start the new hold.
        assert!(!promotion.promote(stale, false, || true, || panic!("stale start")));
        assert!(fire_arming_timer(&hold, &promotion, &mut fresh));
    }

    #[test]
    fn both_shift_letter_before_promotion_emits_nothing() {
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false).is_empty());
        let arming = HoldArming::of(&hold);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::KeyA), false).is_empty());
        assert_eq!(hold.state, HoldState::Idle);
        assert!(arming.cancelled_by(&press(Key::KeyA)).is_some());
        assert!(arming.cancelled_by(&release(Key::ShiftLeft)).is_none());
        assert!(
            both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false).is_empty()
        );

        // Shift then Command inside the promotion window also ends the hold.
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::MetaLeft), false).is_empty());
        assert_eq!(hold.state, HoldState::Idle);
    }

    #[test]
    fn both_arming_combo_resets_double_tap_without_feeding_the_typed_key() {
        assert!(arming_combo_resets_double_tap(
            true,
            HoldDownEvent::Stop,
            &press(Key::KeyA)
        ));
        // A second modifier also cancels arming and resets the sequence.
        // Releases and post-window combos are still fed.
        assert!(arming_combo_resets_double_tap(
            true,
            HoldDownEvent::Stop,
            &press(Key::MetaLeft)
        ));
        assert!(!arming_combo_resets_double_tap(
            true,
            HoldDownEvent::Stop,
            &release(Key::ShiftLeft)
        ));
        assert!(!arming_combo_resets_double_tap(
            false,
            HoldDownEvent::Stop,
            &press(Key::KeyA)
        ));
        assert!(!arming_combo_resets_double_tap(
            true,
            HoldDownEvent::None,
            &press(Key::KeyA)
        ));

        // The reset reaches the state ComboCancelled would have: Shift+A then
        // a quick Shift release is not a first tap.
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::KeyA), false).is_empty());
        assert_eq!(dtap.state, DetectorState::Idle);
        assert_eq!(dtap.last_rejection, None);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert_eq!(dtap.state, DetectorState::Idle);

        // Shift, then Command inside the window, then a typed key: the
        // sequence is already Idle, so the typed key reaches no rejection.
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::MetaLeft), false).is_empty());
        assert_eq!(dtap.state, DetectorState::Idle);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::KeyA), false).is_empty());
        assert_eq!(dtap.state, DetectorState::Idle);
        assert_eq!(dtap.last_rejection, None);
    }

    #[test]
    fn both_double_tap_shift_still_toggles_with_arming() {
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false);
        assert!(both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false).is_empty());
        assert_eq!(
            both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false),
            vec![BothEmit::DoubleTapToggle]
        );

        // Single tap still stops a toggle-started recording.
        let mut hold = make_armed_hold_detector(DetectorMode::Both, Key::ShiftLeft);
        let mut dtap = make_detector(Key::ShiftLeft);
        dtap.recording = true;
        both_handle_event(&mut hold, &mut dtap, &press(Key::ShiftLeft), false);
        assert_eq!(
            both_handle_event(&mut hold, &mut dtap, &release(Key::ShiftLeft), false),
            vec![BothEmit::DoubleTapToggle]
        );
    }

    #[test]
    fn double_tap_mode_shift_is_unaffected_by_arming() {
        let mut d = make_detector(Key::ShiftLeft);
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(!d.handle_event(&release(Key::ShiftLeft)));
        assert!(!d.handle_event(&press(Key::ShiftLeft)));
        assert!(d.handle_event(&release(Key::ShiftLeft)));
    }

    #[test]
    fn arming_cancel_event_code_is_allowlisted() {
        assert_eq!(
            crate::telemetry::canonical_event_code("keyboard.hotkey_arming_cancelled"),
            Some("keyboard.hotkey_arming_cancelled")
        );
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

    #[test]
    fn dictation_key_ids_are_rejected_for_transform() {
        assert!(is_dictation_key_id("shift_l"));
        assert!(is_dictation_key_id("alt_l"));
        assert!(is_dictation_key_id("ctrl_r"));
        assert!(is_dictation_key_id("f1"));
        assert!(is_dictation_key_id("f10"));
        assert!(is_dictation_key_id("f20"));
        // The transform key set must remain distinct.
        assert!(!is_dictation_key_id("shift_r"));
        assert!(!is_dictation_key_id("alt_r"));
        assert!(!is_dictation_key_id("ctrl_l"));
        assert!(!is_dictation_key_id("not_a_real_key"));
        assert!(!is_dictation_key_id(""));
    }

    /// Collect every tracing field emitted by `f` as `name=value` lines.
    fn capture_keyboard_fields(f: impl FnOnce()) -> Vec<String> {
        use std::sync::Arc;
        use tracing_subscriber::layer::{Context, SubscriberExt};
        use tracing_subscriber::{Layer, Registry};

        #[derive(Clone, Default)]
        struct CaptureLayer(Arc<Mutex<Vec<String>>>);

        impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
            fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
                struct Visitor(Vec<String>);
                impl tracing::field::Visit for Visitor {
                    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                        self.0.push(format!("{}={value}", field.name()));
                    }
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        self.0.push(format!("{}={value:?}", field.name()));
                    }
                }
                let mut visitor = Visitor(Vec::new());
                event.record(&mut visitor);
                self.0.lock_or_recover().extend(visitor.0);
            }
        }

        let layer = CaptureLayer::default();
        let captured = Arc::clone(&layer.0);
        tracing::subscriber::with_default(Registry::default().with(layer), f);
        let fields = captured.lock_or_recover().clone();
        fields
    }

    /// Typed keys whose identity must never reach telemetry.
    const TYPED_KEYS: &[(Key, &str)] = &[
        (Key::KeyI, "KeyI"),
        (Key::KeyA, "KeyA"),
        (Key::Num4, "Num4"),
        (Key::Slash, "Slash"),
        (Key::SemiColon, "SemiColon"),
    ];

    fn assert_no_typed_key_identity(fields: &[String]) {
        let joined = fields.join("\n");
        for (_, name) in TYPED_KEYS {
            assert!(!joined.contains(name), "{name} leaked: {joined}");
        }
        for marker in [
            "KeyPress(",
            "KeyRelease(",
            "event_type=",
            "event_name=",
            "key=Some(",
        ] {
            assert!(!joined.contains(marker), "{marker} leaked: {joined}");
        }
    }

    #[test]
    fn key_class_is_bounded_and_content_free() {
        for (key, _) in TYPED_KEYS {
            assert_eq!(key_class(*key), "character");
        }
        assert_eq!(key_class(Key::Space), "character");
        assert_eq!(key_class(Key::Kp7), "character");
        assert_eq!(key_class(Key::ShiftLeft), "modifier");
        assert_eq!(key_class(Key::MetaRight), "modifier");
        assert_eq!(key_class(Key::F8), "function");
        assert_eq!(key_class(Key::LeftArrow), "navigation");
        assert_eq!(key_class(Key::Return), "editing");
        assert_eq!(key_class(Key::VolumeUp), "other");
        assert_eq!(key_class(Key::Unknown(63)), "other");
        assert_eq!(
            event_key_class(&EventType::Wheel {
                delta_x: 0,
                delta_y: 1
            }),
            "none"
        );

        assert_eq!(target_key_label(Some(Key::ShiftLeft)), "shift_l");
        assert_eq!(target_key_label(Some(Key::F8)), "f8");
        assert_eq!(target_key_label(Some(Key::ShiftRight)), "shift_r");
        assert_eq!(target_key_label(Some(Key::KeyI)), "other");
        assert_eq!(target_key_label(None), "none");
    }

    #[test]
    fn hold_combo_cancel_telemetry_records_only_the_key_class() {
        for (key, _) in TYPED_KEYS {
            let fields = capture_keyboard_fields(|| {
                let mut det = HoldDownDetector::new();
                det.set_target(Some(Key::ShiftLeft));
                assert_eq!(
                    det.handle_event(&press(Key::ShiftLeft)),
                    HoldDownEvent::Start
                );
                assert_eq!(det.handle_event(&press(*key)), HoldDownEvent::Stop);
            });
            assert_no_typed_key_identity(&fields);
            assert!(
                fields.contains(&"reason=combo_cancelled".to_string()),
                "{fields:?}"
            );
            assert!(
                fields.contains(&"key_class=character".to_string()),
                "{fields:?}"
            );
            assert!(
                fields.contains(&"event_kind=key_press".to_string()),
                "{fields:?}"
            );
            assert!(
                fields.contains(&"is_target_key=false".to_string()),
                "{fields:?}"
            );
            assert!(
                fields.contains(&"target_key=shift_l".to_string()),
                "{fields:?}"
            );
        }
    }

    #[test]
    fn double_tap_combo_cancel_telemetry_records_only_the_key_class() {
        for (key, _) in TYPED_KEYS {
            // Combo during the first tap.
            let first = capture_keyboard_fields(|| {
                let mut det = make_detector(Key::ShiftLeft);
                det.handle_event(&press(Key::ShiftLeft));
                assert!(!det.handle_event(&press(*key)));
            });
            // Another key while waiting for the second tap.
            let second = capture_keyboard_fields(|| {
                let mut det = make_detector(Key::ShiftLeft);
                det.handle_event(&press(Key::ShiftLeft));
                det.handle_event(&release(Key::ShiftLeft));
                assert!(!det.handle_event(&press(*key)));
            });
            for fields in [first, second] {
                assert_no_typed_key_identity(&fields);
                assert!(
                    fields.contains(&"reason=combo_cancelled".to_string()),
                    "{fields:?}"
                );
                assert!(
                    fields.contains(&"detector=double_tap".to_string()),
                    "{fields:?}"
                );
                assert!(
                    fields.contains(&"key_class=character".to_string()),
                    "{fields:?}"
                );
                assert!(
                    fields.contains(&"is_target_key=false".to_string()),
                    "{fields:?}"
                );
                assert!(
                    fields.contains(&"target_key=shift_l".to_string()),
                    "{fields:?}"
                );
            }
        }
    }

    #[test]
    fn single_tap_and_raw_callback_telemetry_records_only_the_key_class() {
        let previous = HOLD_DOWN_DETECTOR.lock_or_recover().replace({
            let mut det = HoldDownDetector::new();
            det.set_target(Some(Key::ShiftLeft));
            det
        });
        let fields = capture_keyboard_fields(|| {
            log_rejection(
                RejectionReason::SingleShortTapNoop,
                DetectorMode::Both,
                &release(Key::ShiftLeft),
            );
            log_rejection(
                RejectionReason::ProcessingSkipped,
                DetectorMode::Both,
                &press(Key::Slash),
            );
            for (key, name) in TYPED_KEYS {
                trace_raw_callback(
                    &Event {
                        time: std::time::SystemTime::now(),
                        name: Some(name.to_lowercase()),
                        event_type: press(*key),
                    },
                    DetectorMode::Both,
                );
            }
        });
        *HOLD_DOWN_DETECTOR.lock_or_recover() = previous;

        assert_no_typed_key_identity(&fields);
        assert!(!fields.iter().any(|f| f.contains("\"keyi\"")), "{fields:?}");
        assert!(
            fields.contains(&"reason=single_short_tap_noop".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"reason=processing_skipped".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"key_class=modifier".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"key_class=character".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"event_kind=key_release".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"is_target_key=true".to_string()),
            "{fields:?}"
        );
        assert!(
            fields.contains(&"target_key=shift_l".to_string()),
            "{fields:?}"
        );
    }
}
