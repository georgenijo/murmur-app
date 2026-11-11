//! Fn/Globe trigger: open the microphone on key-down, before the gesture is
//! confirmed, and keep or discard that audio by the gesture's outcome.
//!
//! Only `Key::Function` as the configured dictation key reaches this path.
//! A press opens a speculative capture only when it begins a fresh gesture
//! (the detectors were idle and now are not), no shortcut modifier is held,
//! and the press is not the single tap that stops a toggle recording. The
//! capture is bound to the confirmed gesture token when `hold-down-start` or
//! `double-tap-toggle` is emitted; otherwise it is discarded the moment the
//! detectors return to idle (released before the hold arms, a second tap that
//! never comes, Fn plus another key), or on Escape, a listener change,
//! processing, or disable. The audio supervisor's 1.5 s backstop
//! (`SPECULATIVE_CAPTURE_TTL`) bounds any path this module could miss.
//!
//! Lock order: the hold-promotion lock may be held while taking `TRACKER`,
//! and `TRACKER` may be held while taking a detector lock. Never take
//! `TRACKER` while holding a detector lock.
use super::{
    listener_context_matches, DetectorMode, DetectorState, HoldState, APP_DISABLED,
    DOUBLE_TAP_DETECTOR, HOLD_DOWN_DETECTOR, IS_PROCESSING,
};
use crate::audio_lifecycle::speculative::DiscardReason;
use crate::microphone_auto::SmartAutoRequest;
use crate::MutexExt;
use rdev::{EventType, Key};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Settled tracker transitions the caller must carry out, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open(u64),
    Discard(u64, DiscardReason),
    Bind { gesture_token: u64, capture_id: u64 },
}

/// At most one unconfirmed key-down capture exists at a time.
pub(super) struct Tracker {
    active: Option<u64>,
    next_id: u64,
}

impl Tracker {
    pub(super) const fn new() -> Self {
        Self {
            active: None,
            next_id: 0,
        }
    }

    pub(super) fn is_active(&self, id: u64) -> bool {
        self.active == Some(id)
    }

    fn begin(&mut self, actions: &mut Vec<Action>) {
        if let Some(old) = self.active.take() {
            actions.push(Action::Discard(old, DiscardReason::Superseded));
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.active = Some(self.next_id);
        actions.push(Action::Open(self.next_id));
    }

    /// The gesture was confirmed: hand the capture to its start request.
    pub(super) fn confirm(&mut self, gesture_token: u64) -> Option<Action> {
        self.active.take().map(|capture_id| Action::Bind {
            gesture_token,
            capture_id,
        })
    }

    pub(super) fn abandon(&mut self, reason: DiscardReason) -> Option<Action> {
        self.active.take().map(|id| Action::Discard(id, reason))
    }

    /// Discard an unconfirmed capture once the physical gesture has ended.
    pub(super) fn settle(&mut self, gesture_in_progress: bool) -> Option<Action> {
        if gesture_in_progress {
            None
        } else {
            self.abandon(DiscardReason::NotConfirmed)
        }
    }
}

/// Facts about one keyboard event, read around the detector update.
#[derive(Clone, Copy, Debug)]
pub(super) struct EventFacts<'a> {
    pub target: Option<Key>,
    pub event: &'a EventType,
    pub shortcut_modifiers_held: bool,
    /// Detectors were idle before this event.
    pub gesture_was_idle: bool,
    /// Detectors are mid-gesture after this event.
    pub gesture_in_progress: bool,
    /// A toggle recording is running, so this press can only stop it.
    pub toggle_recording: bool,
    /// Processing or disabled: no new dictation may start.
    pub blocked: bool,
}

/// Whether this event begins a gesture that may open key-down capture. Pure,
/// and false for every trigger other than Fn.
pub(super) fn opens_capture(facts: &EventFacts<'_>) -> bool {
    facts.target == Some(Key::Function)
        && matches!(facts.event, EventType::KeyPress(Key::Function))
        && !facts.shortcut_modifiers_held
        && facts.gesture_was_idle
        && facts.gesture_in_progress
        && !facts.toggle_recording
        && !facts.blocked
}

/// The tracker transitions for one keyboard event.
pub(super) fn decide(tracker: &mut Tracker, facts: &EventFacts<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    if facts.target != Some(Key::Function) {
        actions.extend(tracker.abandon(DiscardReason::ListenerChanged));
        return actions;
    }
    if opens_capture(facts) {
        tracker.begin(&mut actions);
    } else {
        actions.extend(tracker.settle(facts.gesture_in_progress));
    }
    actions
}

/// Whether the dictation detectors for `mode` are mid-gesture.
pub(super) fn detectors_in_progress(
    mode: DetectorMode,
    double_tap: Option<DetectorState>,
    hold: Option<HoldState>,
) -> bool {
    let tap = double_tap.is_some_and(|state| state != DetectorState::Idle);
    let held = hold == Some(HoldState::Held);
    match mode {
        DetectorMode::DoubleTap => tap,
        DetectorMode::HoldDown => held,
        DetectorMode::Both => tap || held,
    }
}

fn live_in_progress(mode: DetectorMode) -> bool {
    let tap = DOUBLE_TAP_DETECTOR
        .lock_or_recover()
        .as_ref()
        .map(|d| d.state);
    let hold = HOLD_DOWN_DETECTOR
        .lock_or_recover()
        .as_ref()
        .map(|d| d.state);
    detectors_in_progress(mode, tap, hold)
}

fn toggle_recording() -> bool {
    DOUBLE_TAP_DETECTOR
        .lock_or_recover()
        .as_ref()
        .is_some_and(|d| d.recording)
}

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());
static FN_TRIGGER: AtomicBool = AtomicBool::new(false);

/// The microphone the next dictation will request, mirrored from settings so
/// key-down capture opens the same input the confirmed start asks for. Never
/// logged. `None` until the frontend reports it: no speculation until then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CaptureInput {
    Device(Option<String>),
    SmartAuto(SmartAutoRequest),
}

static INPUT: Mutex<Option<CaptureInput>> = Mutex::new(None);

pub(crate) fn set_capture_input(input: CaptureInput) {
    *INPUT.lock_or_recover() = Some(input);
}

pub(super) fn trigger_active() -> bool {
    FN_TRIGGER.load(Ordering::SeqCst)
}

/// Called whenever the dictation listener's key or registration changes.
pub(super) fn listener_changed(target: Option<Key>, active: bool) {
    FN_TRIGGER.store(active && target == Some(Key::Function), Ordering::SeqCst);
    abandon(DiscardReason::ListenerChanged);
}

pub(super) fn abandon(reason: DiscardReason) {
    let mut tracker = TRACKER.lock_or_recover();
    if let Some(action) = tracker.abandon(reason) {
        run(None, action, DetectorMode::DoubleTap, 0);
    }
}

/// Bind the unconfirmed capture to the gesture token the listener is about to
/// emit. Must run before the event reaches the webview.
pub(super) fn confirm(gesture_token: u64) {
    if !trigger_active() {
        return;
    }
    let mut tracker = TRACKER.lock_or_recover();
    if let Some(action) = tracker.confirm(gesture_token) {
        run(None, action, DetectorMode::DoubleTap, 0);
    }
}

/// Re-evaluate after a detector changed off the listener thread (second-tap
/// expiry).
pub(super) fn settle(mode: DetectorMode) {
    if !trigger_active() {
        return;
    }
    let mut tracker = TRACKER.lock_or_recover();
    let in_progress = live_in_progress(mode);
    if let Some(action) = tracker.settle(in_progress) {
        run(None, action, mode, 0);
    }
}

/// Snapshot taken by the listener before feeding the detectors.
pub(super) fn gesture_idle_before(mode: DetectorMode) -> bool {
    !live_in_progress(mode)
}

/// Apply one keyboard event after the detectors have consumed it.
pub(super) fn after_event(
    handle: &tauri::AppHandle,
    mode: DetectorMode,
    listener_generation: u64,
    target: Option<Key>,
    event: &EventType,
    shortcut_modifiers_held: bool,
    gesture_was_idle: bool,
) {
    let mut tracker = TRACKER.lock_or_recover();
    let facts = EventFacts {
        target,
        event,
        shortcut_modifiers_held,
        gesture_was_idle,
        gesture_in_progress: live_in_progress(mode),
        toggle_recording: toggle_recording(),
        blocked: IS_PROCESSING.load(Ordering::SeqCst) || APP_DISABLED.load(Ordering::SeqCst),
    };
    for action in decide(&mut tracker, &facts) {
        run(Some(handle), action, mode, listener_generation);
    }
}

/// Runs under `TRACKER`, so an open and a later discard of the same capture
/// reach the supervisor in order.
fn run(
    handle: Option<&tauri::AppHandle>,
    action: Action,
    mode: DetectorMode,
    listener_generation: u64,
) {
    match action {
        Action::Open(id) => {
            if let Some(handle) = handle {
                spawn_open(handle.clone(), id, mode, listener_generation);
            }
        }
        Action::Discard(id, reason) => {
            crate::audio_lifecycle::discard_speculative_capture(id, reason);
        }
        Action::Bind {
            gesture_token,
            capture_id,
        } => crate::audio_lifecycle::speculative::bind(gesture_token, capture_id),
    }
}

/// Admission mirrors silent first-tap preparation: the shared recording
/// transition, an already granted microphone (never prompting), and the
/// dictation start guards. The supervisor additionally refuses while any
/// capture, standby, or teardown owns the microphone.
fn spawn_open(handle: tauri::AppHandle, id: u64, mode: DetectorMode, listener_generation: u64) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        let Some(state) = handle.try_state::<crate::State>() else {
            return;
        };
        let Some(input) = INPUT.lock_or_recover().clone() else {
            log_refused(id, "input_unconfigured");
            return;
        };
        let Some(_transition) = crate::audio::prepare::admission_transition(
            &state.app_state.recording_transition,
            || {
                listener_context_matches(mode, listener_generation)
                    && !IS_PROCESSING.load(Ordering::SeqCst)
                    && !APP_DISABLED.load(Ordering::SeqCst)
                    && TRACKER.lock_or_recover().is_active(id)
                    && crate::commands::permissions::check_microphone_permission_status()
                        == "granted"
                    && crate::commands::recording::dictation_prepare_admitted(&state)
            },
        )
        .await
        else {
            log_refused(id, "not_admitted");
            return;
        };
        let device_id = match input {
            CaptureInput::Device(device) => device,
            CaptureInput::SmartAuto(request) => {
                match crate::audio_inventory::resolve_smart_auto_preview(&request) {
                    Ok(selection) => Some(selection.device_id),
                    Err(_) => {
                        log_refused(id, "smart_auto_unavailable");
                        return;
                    }
                }
            }
        };
        let admitted = {
            let tracker = TRACKER.lock_or_recover();
            if !tracker.is_active(id) {
                return;
            }
            crate::audio_lifecycle::request_speculative_capture(id, device_id, Some(handle.clone()))
        };
        // Hold the transition until the supervisor has published ownership,
        // so no other capture claim can interleave with this one.
        let _ = crate::audio_lifecycle::await_speculative_admission(admitted);
    });
}

fn log_refused(id: u64, reason: &'static str) {
    tracing::info!(
        target: "keyboard",
        event_code = "keyboard.fn_capture_skipped",
        capture_id = id,
        reason,
        "Fn key-down capture not opened; the confirmed start takes the ordinary path"
    );
}

#[cfg(test)]
pub(super) fn reset_for_test() {
    *TRACKER.lock_or_recover() = Tracker::new();
}

#[cfg(test)]
mod tests {
    use super::super::{
        arming_combo_resets_double_tap, hold_arming_ms, hotkey_to_rdev_key, DoubleTapDetector,
        HoldDownDetector, HoldDownEvent, DICTATION_KEY_IDS, DOUBLE_TAP_WINDOW_MS,
    };
    use super::*;
    use std::time::{Duration, Instant};

    /// Drives the real detectors the way the listener callback does, then
    /// applies `decide`, recording every tracker action. Promotion and
    /// second-tap expiry are the timer threads' effects, invoked explicitly.
    struct Sim {
        mode: DetectorMode,
        target: Key,
        tap: Option<DoubleTapDetector>,
        hold: Option<HoldDownDetector>,
        tracker: Tracker,
        actions: Vec<Action>,
        hold_token: u64,
    }

    impl Sim {
        fn new(mode: DetectorMode, target: Key) -> Self {
            let tap = (mode != DetectorMode::HoldDown).then(|| {
                let mut d = DoubleTapDetector::new();
                d.set_target(Some(target));
                d
            });
            let hold = (mode != DetectorMode::DoubleTap).then(|| {
                let mut d = HoldDownDetector::new();
                d.set_target(Some(target));
                d.arming_ms = hold_arming_ms(mode, Some(target));
                d
            });
            Self {
                mode,
                target,
                tap,
                hold,
                tracker: Tracker::new(),
                actions: Vec::new(),
                hold_token: 10_000,
            }
        }

        fn in_progress(&self) -> bool {
            detectors_in_progress(
                self.mode,
                self.tap.as_ref().map(|d| d.state),
                self.hold.as_ref().map(|d| d.state),
            )
        }

        fn confirm(&mut self, token: u64) {
            self.actions.extend(self.tracker.confirm(token));
        }

        fn event_with(&mut self, event: EventType, modifiers: bool) -> Vec<Action> {
            let start = self.actions.len();
            let was_idle = !self.in_progress();
            let mut fired_token = None;
            match self.mode {
                DetectorMode::DoubleTap => {
                    let d = self.tap.as_mut().unwrap();
                    if d.handle_event_with_shortcut_modifiers(&event, modifiers) {
                        fired_token = d.fired_candidate.take().map(|c| c.token);
                    }
                }
                DetectorMode::HoldDown => {
                    self.hold
                        .as_mut()
                        .unwrap()
                        .handle_event_with_shortcut_modifiers(&event, modifiers);
                }
                DetectorMode::Both => {
                    let tap = self.tap.as_mut().unwrap();
                    let second_phase = matches!(
                        tap.state,
                        DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
                    ) && tap.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS;
                    let (hold_result, was_arming) = if second_phase {
                        (HoldDownEvent::None, false)
                    } else {
                        let hold = self.hold.as_mut().unwrap();
                        let was_arming = hold.is_arming();
                        (
                            hold.handle_event_with_shortcut_modifiers(&event, modifiers),
                            was_arming,
                        )
                    };
                    if arming_combo_resets_double_tap(was_arming, hold_result, &event) {
                        tap.reset();
                    } else if tap.handle_event_with_shortcut_modifiers(&event, modifiers) {
                        fired_token = tap.fired_candidate.take().map(|c| c.token);
                    }
                }
            }
            if let Some(token) = fired_token {
                self.confirm(token);
            }
            let facts = EventFacts {
                target: Some(self.target),
                event: &event,
                shortcut_modifiers_held: modifiers,
                gesture_was_idle: was_idle,
                gesture_in_progress: self.in_progress(),
                toggle_recording: self.tap.as_ref().is_some_and(|d| d.recording),
                blocked: false,
            };
            let actions = decide(&mut self.tracker, &facts);
            self.actions.extend(actions);
            self.actions[start..].to_vec()
        }

        fn press(&mut self) -> Vec<Action> {
            self.event_with(EventType::KeyPress(self.target), false)
        }

        fn release(&mut self) -> Vec<Action> {
            self.event_with(EventType::KeyRelease(self.target), false)
        }

        /// The hold timer elapsed with the key still down.
        fn promote(&mut self) -> Vec<Action> {
            let start = self.actions.len();
            let hold = self.hold.as_mut().unwrap();
            hold.held_at = Instant::now() - Duration::from_millis(hold.arming_ms as u64 + 1);
            if hold.state == HoldState::Held {
                let token = match self.mode {
                    DetectorMode::Both => self.tap.as_ref().unwrap().candidate.unwrap().token,
                    _ => {
                        self.hold_token += 1;
                        self.hold_token
                    }
                };
                self.confirm(token);
            }
            self.actions[start..].to_vec()
        }

        /// The second-tap expiry thread ran after the window.
        fn expire(&mut self) -> Vec<Action> {
            let start = self.actions.len();
            let tap = self.tap.as_mut().unwrap();
            let started = tap
                .second_tap_wait_started_at()
                .expect("waiting for second tap");
            tap.state_entered_at = started - Duration::from_millis(DOUBLE_TAP_WINDOW_MS as u64 + 1);
            let started = tap.state_entered_at;
            assert!(tap.expire_second_tap_wait(started).is_some());
            let in_progress = self.in_progress();
            self.actions.extend(self.tracker.settle(in_progress));
            self.actions[start..].to_vec()
        }

        fn opened(&self) -> usize {
            self.actions
                .iter()
                .filter(|a| matches!(a, Action::Open(_)))
                .count()
        }
    }

    fn bound(actions: &[Action], capture_id: u64) -> bool {
        matches!(actions, [Action::Bind { capture_id: id, .. }] if *id == capture_id)
    }

    #[test]
    fn confirmed_hold_binds_the_key_down_capture() {
        for mode in [DetectorMode::HoldDown, DetectorMode::Both] {
            let mut sim = Sim::new(mode, Key::Function);
            assert_eq!(sim.press(), vec![Action::Open(1)], "{mode:?}");
            assert!(bound(&sim.promote(), 1), "{mode:?}");
            assert_eq!(sim.release(), vec![], "{mode:?}");
            assert!(!sim.actions.iter().any(|a| matches!(a, Action::Discard(..))));
        }
    }

    #[test]
    fn confirmed_double_tap_keeps_one_capture_open_across_both_taps() {
        for mode in [DetectorMode::DoubleTap, DetectorMode::Both] {
            let mut sim = Sim::new(mode, Key::Function);
            assert_eq!(sim.press(), vec![Action::Open(1)], "{mode:?}");
            assert_eq!(sim.release(), vec![], "{mode:?}");
            assert_eq!(sim.press(), vec![], "{mode:?}: second press reuses capture");
            assert!(bound(&sim.release(), 1), "{mode:?}");
            assert_eq!(sim.opened(), 1);
        }
    }

    #[test]
    fn stray_taps_discard_without_confirmation() {
        // Hold: released inside the arming window.
        let mut sim = Sim::new(DetectorMode::HoldDown, Key::Function);
        sim.press();
        assert_eq!(
            sim.release(),
            vec![Action::Discard(1, DiscardReason::NotConfirmed)]
        );
        // Double-tap and Both: the second tap never comes.
        for mode in [DetectorMode::DoubleTap, DetectorMode::Both] {
            let mut sim = Sim::new(mode, Key::Function);
            sim.press();
            assert_eq!(sim.release(), vec![], "{mode:?}: waits for second tap");
            assert_eq!(
                sim.expire(),
                vec![Action::Discard(1, DiscardReason::NotConfirmed)],
                "{mode:?}"
            );
        }
    }

    #[test]
    fn fn_plus_another_key_discards_immediately() {
        for mode in [
            DetectorMode::DoubleTap,
            DetectorMode::HoldDown,
            DetectorMode::Both,
        ] {
            for other in [Key::UpArrow, Key::KeyE, Key::ShiftLeft] {
                let mut sim = Sim::new(mode, Key::Function);
                sim.press();
                assert_eq!(
                    sim.event_with(EventType::KeyPress(other), false),
                    vec![Action::Discard(1, DiscardReason::NotConfirmed)],
                    "{mode:?} {other:?}"
                );
            }
        }
        // A combo that starts on the second tap also ends the gesture.
        let mut sim = Sim::new(DetectorMode::DoubleTap, Key::Function);
        sim.press();
        sim.release();
        sim.press();
        assert_eq!(
            sim.event_with(EventType::KeyPress(Key::KeyE), false),
            vec![Action::Discard(1, DiscardReason::NotConfirmed)]
        );
    }

    #[test]
    fn modifier_chords_and_toggle_stop_taps_never_open() {
        for mode in [
            DetectorMode::DoubleTap,
            DetectorMode::HoldDown,
            DetectorMode::Both,
        ] {
            let mut sim = Sim::new(mode, Key::Function);
            sim.event_with(EventType::KeyPress(Key::Function), true);
            assert_eq!(sim.opened(), 0, "{mode:?}: Cmd+Fn style chord");
        }
        for mode in [DetectorMode::DoubleTap, DetectorMode::Both] {
            let mut sim = Sim::new(mode, Key::Function);
            sim.tap.as_mut().unwrap().recording = true;
            assert_eq!(sim.press(), vec![], "{mode:?}: stop tap");
            sim.release();
            assert_eq!(sim.opened(), 0);
        }
        let mut sim = Sim::new(DetectorMode::HoldDown, Key::Function);
        let facts = EventFacts {
            target: Some(Key::Function),
            event: &EventType::KeyPress(Key::Function),
            shortcut_modifiers_held: false,
            gesture_was_idle: true,
            gesture_in_progress: true,
            toggle_recording: false,
            blocked: true,
        };
        assert!(
            decide(&mut sim.tracker, &facts).is_empty(),
            "processing or disabled"
        );
    }

    #[test]
    fn escape_and_trigger_change_mid_gesture_discard() {
        let mut sim = Sim::new(DetectorMode::Both, Key::Function);
        sim.press();
        assert_eq!(
            sim.tracker.abandon(DiscardReason::Escape),
            Some(Action::Discard(1, DiscardReason::Escape))
        );
        assert_eq!(sim.release(), vec![]);

        let mut sim = Sim::new(DetectorMode::DoubleTap, Key::Function);
        sim.press();
        let facts = EventFacts {
            target: Some(Key::ShiftLeft),
            event: &EventType::KeyRelease(Key::Function),
            shortcut_modifiers_held: false,
            gesture_was_idle: false,
            gesture_in_progress: false,
            toggle_recording: false,
            blocked: false,
        };
        assert_eq!(
            decide(&mut sim.tracker, &facts),
            vec![Action::Discard(1, DiscardReason::ListenerChanged)]
        );
    }

    #[test]
    fn listener_hooks_track_the_trigger_and_abandon_the_capture() {
        reset_for_test();
        listener_changed(Some(Key::Function), true);
        assert!(trigger_active());
        TRACKER.lock_or_recover().begin(&mut Vec::new());
        abandon(DiscardReason::Escape);
        assert!(TRACKER.lock_or_recover().active.is_none());

        TRACKER.lock_or_recover().begin(&mut Vec::new());
        listener_changed(Some(Key::ShiftLeft), true);
        assert!(!trigger_active());
        assert!(TRACKER.lock_or_recover().active.is_none());
        // Confirmation is ignored once Fn is no longer the trigger.
        TRACKER.lock_or_recover().begin(&mut Vec::new());
        confirm(5);
        assert!(TRACKER.lock_or_recover().active.is_some());
        listener_changed(None, false);
        assert!(TRACKER.lock_or_recover().active.is_none());
        reset_for_test();
    }

    #[test]
    fn a_new_press_supersedes_an_unsettled_capture() {
        let mut tracker = Tracker::new();
        let mut actions = Vec::new();
        tracker.begin(&mut actions);
        tracker.begin(&mut actions);
        assert_eq!(
            actions,
            vec![
                Action::Open(1),
                Action::Discard(1, DiscardReason::Superseded),
                Action::Open(2),
            ]
        );
        assert_eq!(
            tracker.confirm(9),
            Some(Action::Bind {
                gesture_token: 9,
                capture_id: 2
            })
        );
        assert_eq!(tracker.confirm(9), None);
        assert_eq!(tracker.settle(false), None);
    }

    #[test]
    fn no_other_dictation_key_opens_capture_before_a_confirmed_start() {
        let mut checked = 0;
        for id in DICTATION_KEY_IDS.iter().filter(|id| **id != "fn") {
            let key = hotkey_to_rdev_key(id).unwrap();
            for mode in [
                DetectorMode::DoubleTap,
                DetectorMode::HoldDown,
                DetectorMode::Both,
            ] {
                let mut sim = Sim::new(mode, key);
                sim.press();
                if mode != DetectorMode::DoubleTap {
                    sim.promote();
                }
                sim.release();
                sim.press();
                sim.release();
                sim.press();
                sim.event_with(EventType::KeyPress(Key::KeyE), false);
                sim.event_with(EventType::KeyPress(key), true);
                assert_eq!(sim.opened(), 0, "{id} {mode:?}");
                assert!(sim.actions.is_empty(), "{id} {mode:?}: {:?}", sim.actions);
                checked += 1;
            }
            // Even with every other fact favourable, the target gates it.
            assert!(!opens_capture(&EventFacts {
                target: Some(key),
                event: &EventType::KeyPress(key),
                shortcut_modifiers_held: false,
                gesture_was_idle: true,
                gesture_in_progress: true,
                toggle_recording: false,
                blocked: false,
            }));
        }
        assert_eq!(checked, (DICTATION_KEY_IDS.len() - 1) * 3);
    }

    #[test]
    fn in_progress_follows_the_mode_detectors() {
        use DetectorMode::*;
        let waiting = Some(DetectorState::WaitingSecondDown);
        let held = Some(HoldState::Held);
        assert!(detectors_in_progress(DoubleTap, waiting, None));
        assert!(!detectors_in_progress(
            DoubleTap,
            Some(DetectorState::Idle),
            held
        ));
        assert!(detectors_in_progress(HoldDown, None, held));
        assert!(!detectors_in_progress(
            HoldDown,
            waiting,
            Some(HoldState::Idle)
        ));
        assert!(detectors_in_progress(Both, waiting, Some(HoldState::Idle)));
        assert!(detectors_in_progress(Both, Some(DetectorState::Idle), held));
        assert!(!detectors_in_progress(Both, None, None));
    }
}
