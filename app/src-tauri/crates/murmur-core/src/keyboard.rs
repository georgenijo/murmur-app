//! Host-neutral gesture detectors. The native adapter converts rdev events once.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}
struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Alt,
    AltGr,
    BackQuote,
    BackSlash,
    Backspace,
    CapsLock,
    Comma,
    ControlLeft,
    ControlRight,
    Delete,
    Dot,
    DownArrow,
    End,
    Equal,
    Escape,
    F1,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F2,
    F20,
    F21,
    F22,
    F23,
    F24,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    Function,
    Home,
    Insert,
    IntlBackslash,
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Kp0,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
    KpDelete,
    KpDivide,
    KpMinus,
    KpMultiply,
    KpPlus,
    KpReturn,
    LeftArrow,
    LeftBracket,
    MetaLeft,
    MetaRight,
    Minus,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    PageDown,
    PageUp,
    Quote,
    Return,
    RightArrow,
    RightBracket,
    SemiColon,
    ShiftLeft,
    ShiftRight,
    Slash,
    Space,
    Tab,
    UpArrow,
    VolumeUp,
    Unknown(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Unknown(u8),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventType {
    KeyPress(Key),
    KeyRelease(Key),
    ButtonPress(Button),
    ButtonRelease(Button),
    Wheel { delta_x: i64, delta_y: i64 },
    MouseMove { x: f64, y: f64 },
}
#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: EventType,
    pub time: std::time::SystemTime,
    pub name: Option<String>,
}
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub token: u64,
    pub pressed: Instant,
    pub released: Option<Instant>,
}
impl Default for Candidate {
    fn default() -> Self {
        Self::new()
    }
}
impl Candidate {
    pub fn new() -> Self {
        Self::at(Instant::now())
    }
    fn at(pressed: Instant) -> Self {
        Self {
            token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
            pressed,
            released: None,
        }
    }
}

/// Max duration a single tap can be held before it's rejected
pub const MAX_HOLD_DURATION_MS: u128 = 200;

/// Max gap between first key-up and second key-down
pub const DOUBLE_TAP_WINDOW_MS: u128 = 400;

/// Cooldown after firing to prevent triple-tap spam
pub const COOLDOWN_MS: u128 = 50;

/// Cooldown after hold-down stop to prevent accidental re-trigger
pub const HOLD_DOWN_COOLDOWN_MS: u128 = 50;

/// Hold-down mode defers a modifier-only trigger's start by this long. Shift
/// is pressed for every capital letter: production false starts typed the
/// next key a p50 of ~81–98ms after the start was requested, so 150ms rejects
/// ordinary capitalization while staying below Both mode's 200ms promotion.
/// Capture startup (~220ms p50 to first PCM) still dominates start latency.
pub const MODIFIER_HOLD_ARMING_MS: u128 = 150;
const _: () = assert!(MODIFIER_HOLD_ARMING_MS < MAX_HOLD_DURATION_MS);

/// Report callback inactivity after this duration; it is not a health check.
pub const TAP_SILENCE_WARNING_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetectorState {
    Idle,
    WaitingFirstUp,
    WaitingSecondDown,
    WaitingSecondUp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RejectionReason {
    HeldTooLong,
    SecondTapExpired,
    ComboCancelled,
    SingleShortTapNoop,
    ProcessingSkipped,
}

impl RejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HeldTooLong => "held_too_long",
            Self::SecondTapExpired => "second_tap_expired",
            Self::ComboCancelled => "combo_cancelled",
            Self::SingleShortTapNoop => "single_short_tap_noop",
            Self::ProcessingSkipped => "processing_skipped",
        }
    }
}

pub struct DoubleTapDetector {
    pub clock: Arc<dyn Clock>,
    pub candidate: Option<Candidate>,
    pub fired_candidate: Option<Candidate>,
    pub state: DetectorState,
    pub target_key: Option<Key>,
    pub recording: bool,
    pub state_entered_at: Instant,
    pub last_fired_at: Option<Instant>,
    pub last_rejection: Option<RejectionReason>,
    /// A first clean tap that may silently prepare a capture helper (#933).
    pub prepare_request: Option<Candidate>,
    /// The gesture token whose preparation is outstanding until this sequence
    /// fires or is abandoned.
    pub prepared_token: Option<u64>,
}

/// What the listener must do with silent first-tap preparation after one
/// detector event.
#[derive(Debug, Clone, Copy)]
pub enum PrepareSignal {
    None,
    Request(Candidate),
    Abandon,
}

impl DoubleTapDetector {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            candidate: None,
            fired_candidate: None,
            state: DetectorState::Idle,
            target_key: None,
            recording: false,
            state_entered_at: clock.now(),
            last_fired_at: None,
            last_rejection: None,
            prepare_request: None,
            prepared_token: None,
            clock,
        }
    }

    /// Drain this event's preparation transition. A fired double tap leaves
    /// the prepared helper for the confirmed start to adopt; any other exit
    /// from the second-tap phases abandons it.
    pub fn take_prepare_signal(&mut self, fired: bool) -> PrepareSignal {
        if let Some(candidate) = self.prepare_request.take() {
            self.prepared_token = Some(candidate.token);
            return PrepareSignal::Request(candidate);
        }
        let in_second_phase = matches!(
            self.state,
            DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
        );
        match self.prepared_token {
            Some(_) if fired => {
                self.prepared_token = None;
                PrepareSignal::None
            }
            Some(_) if !in_second_phase => {
                self.prepared_token = None;
                PrepareSignal::Abandon
            }
            _ => PrepareSignal::None,
        }
    }

    pub fn set_target(&mut self, key: Option<Key>) {
        self.target_key = key;
        self.reset();
    }

    pub fn reset(&mut self) {
        self.candidate = None;
        self.state = DetectorState::Idle;
        self.state_entered_at = self.clock.now();
    }

    pub fn preparation_candidate_matches(&self, token: u64) -> bool {
        self.prepared_token == Some(token)
            && self
                .candidate
                .is_some_and(|candidate| candidate.token == token)
            && matches!(
                self.state,
                DetectorState::WaitingSecondDown | DetectorState::WaitingSecondUp
            )
    }

    pub fn transition(&mut self, new_state: DetectorState) {
        self.state = new_state;
        self.state_entered_at = self.clock.now();
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.clock
            .now()
            .saturating_duration_since(self.state_entered_at)
            .as_millis()
    }

    pub fn in_cooldown(&self) -> bool {
        self.last_fired_at
            .map(|t| self.clock.now().saturating_duration_since(t).as_millis() < COOLDOWN_MS)
            .unwrap_or(false)
    }

    pub fn log_rejection(&mut self, reason: RejectionReason, event_type: &EventType) {
        self.last_rejection = Some(reason);
        tracing::info!(
            target: "keyboard",
            detector = "double_tap",
            reason = reason.as_str(),
            state = ?self.state,
            elapsed_ms = self.elapsed_ms() as u64,
            recording = self.recording,
            target_key = target_key_label(self.target_key),
            event_kind = event_kind(event_type),
            key_class = event_key_class(event_type),
            is_target_key = is_target_event(event_type, self.target_key),
            "keyboard detector rejected sequence"
        );
    }

    /// Process a keyboard event. Returns true if a double-tap was detected.
    pub fn handle_event(&mut self, event_type: &EventType) -> bool {
        self.handle_event_with_shortcut_modifiers(event_type, false)
    }

    pub fn handle_event_with_shortcut_modifiers(
        &mut self,
        event_type: &EventType,
        shortcut_modifiers_held: bool,
    ) -> bool {
        self.handle_event_with_context(event_type, shortcut_modifiers_held, false)
    }

    pub fn handle_event_with_context(
        &mut self,
        event_type: &EventType,
        shortcut_modifiers_held: bool,
        repeated_function_press: bool,
    ) -> bool {
        self.last_rejection = None;
        let target = match self.target_key {
            Some(k) => k,
            None => return false,
        };

        let function_chord_rejected = is_function_key(target)
            && match (self.state, event_type) {
                (DetectorState::Idle, EventType::KeyPress(key)) => {
                    is_target_key(*key, target)
                        && (shortcut_modifiers_held || repeated_function_press)
                }
                (DetectorState::WaitingFirstUp, EventType::KeyPress(_))
                | (DetectorState::WaitingSecondDown, EventType::KeyPress(_))
                | (DetectorState::WaitingSecondUp, EventType::KeyPress(_)) => {
                    shortcut_modifiers_held
                }
                _ => false,
            };
        if function_chord_rejected {
            self.log_rejection(RejectionReason::ComboCancelled, event_type);
            self.reset();
            return false;
        }

        if self.in_cooldown() {
            return false;
        }

        match self.state {
            DetectorState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_target_key(*key, target) {
                        self.candidate = Some(Candidate::at(self.clock.now()));
                        self.transition(DetectorState::WaitingFirstUp);
                    }
                }
                false
            }

            DetectorState::WaitingFirstUp => {
                match event_type {
                    EventType::KeyRelease(key) if is_target_key(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            if self.recording {
                                self.fired_candidate = None;
                                // Single tap to stop — fire immediately
                                self.last_fired_at = Some(self.clock.now());
                                self.reset();
                                return true;
                            }
                            if let Some(candidate) = self.candidate.as_mut() {
                                candidate.released = Some(self.clock.now());
                            }
                            self.prepare_request = self.candidate;
                            self.transition(DetectorState::WaitingSecondDown);
                        } else {
                            // Held too long — not a tap
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if is_target_key(*key, target) => {
                        // Key repeat event — ignore, stay in same state
                        // But check if we've been held too long
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(_)
                    | EventType::ButtonPress(_)
                    | EventType::Wheel { .. } => {
                        // Not a clean tap: a combo like Shift+A or Cmd+Shift+4,
                        // or a mouse button/wheel while the trigger is down.
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.reset();
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
                    EventType::KeyPress(key) if is_target_key(*key, target) => {
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
                    EventType::KeyRelease(key) if is_target_key(*key, target) => {
                        if self.elapsed_ms() <= MAX_HOLD_DURATION_MS {
                            self.fired_candidate = self.candidate;
                            // Double-tap detected!
                            self.last_fired_at = Some(self.clock.now());
                            self.reset();
                            return true;
                        } else {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(key) if is_target_key(*key, target) => {
                        // Key repeat — check timeout
                        if self.elapsed_ms() > MAX_HOLD_DURATION_MS {
                            self.log_rejection(RejectionReason::HeldTooLong, event_type);
                            self.reset();
                        }
                    }
                    EventType::KeyPress(_)
                    | EventType::ButtonPress(_)
                    | EventType::Wheel { .. } => {
                        // Combo, other modifier, or mouse input on second press
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.reset();
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

    pub fn take_rejection(&mut self) -> Option<RejectionReason> {
        self.last_rejection.take()
    }

    pub fn second_tap_wait_started_at(&self) -> Option<Instant> {
        (self.state == DetectorState::WaitingSecondDown).then_some(self.state_entered_at)
    }

    pub fn expire_second_tap_wait(&mut self, started_at: Instant) -> Option<u64> {
        if self.state != DetectorState::WaitingSecondDown
            || self.state_entered_at != started_at
            || self.elapsed_ms() <= DOUBLE_TAP_WINDOW_MS
        {
            return None;
        }

        let elapsed_ms = self.elapsed_ms() as u64;
        self.reset();
        self.prepared_token = None;
        Some(elapsed_ms)
    }
}

/// Whether a dictation trigger is a bare modifier that ordinary typing also
/// presses (Shift for capitals, Option/Control/Command for shortcuts).
pub fn is_modifier_only_hotkey(key: Key) -> bool {
    is_modifier(key) || key == Key::Function
}

/// Arming window for a dictation hold detector: modifier-only triggers wait
/// before committing a start; other triggers start immediately (0). Both mode
/// already waits `MAX_HOLD_DURATION_MS` before promoting a hold.
pub fn hold_arming_ms(mode: DetectorMode, target: Option<Key>) -> u128 {
    match (mode, target) {
        (_, None) => 0,
        (_, Some(key)) if !is_modifier_only_hotkey(key) => 0,
        (DetectorMode::HoldDown, Some(_)) => MODIFIER_HOLD_ARMING_MS,
        (DetectorMode::Both, Some(_)) => MAX_HOLD_DURATION_MS,
        (DetectorMode::DoubleTap, Some(_)) => 0,
    }
}

/// Check if a key is any modifier key
pub fn is_modifier(key: Key) -> bool {
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

pub fn is_function_key(key: Key) -> bool {
    matches!(
        key,
        Key::F1
            | Key::F2
            | Key::F3
            | Key::F4
            | Key::F5
            | Key::F6
            | Key::F7
            | Key::F8
            | Key::F9
            | Key::F10
            | Key::F11
            | Key::F12
            | Key::F13
            | Key::F14
            | Key::F15
            | Key::F16
            | Key::F17
            | Key::F18
            | Key::F19
            | Key::F20
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PressedFunctionKeys(u32);

impl PressedFunctionKeys {
    /// Record physical F-key state and report whether this KeyPress is an OS
    /// autorepeat. Releases always clear the bit, even while dictation is
    /// disabled or processing, so the next physical press starts fresh.
    pub fn update(&mut self, event_type: &EventType) -> bool {
        match event_type {
            EventType::KeyPress(key) => {
                let Some(bit) = function_key_bit(*key) else {
                    return false;
                };
                let repeated = self.0 & bit != 0;
                self.0 |= bit;
                repeated
            }
            EventType::KeyRelease(key) => {
                if let Some(bit) = function_key_bit(*key) {
                    self.0 &= !bit;
                }
                false
            }
            _ => false,
        }
    }
}

pub fn function_key_bit(key: Key) -> Option<u32> {
    match key {
        Key::F1 => Some(1 << 0),
        Key::F2 => Some(1 << 1),
        Key::F3 => Some(1 << 2),
        Key::F4 => Some(1 << 3),
        Key::F5 => Some(1 << 4),
        Key::F6 => Some(1 << 5),
        Key::F7 => Some(1 << 6),
        Key::F8 => Some(1 << 7),
        Key::F9 => Some(1 << 8),
        Key::F10 => Some(1 << 9),
        Key::F11 => Some(1 << 10),
        Key::F12 => Some(1 << 11),
        Key::F13 => Some(1 << 12),
        Key::F14 => Some(1 << 13),
        Key::F15 => Some(1 << 14),
        Key::F16 => Some(1 << 15),
        Key::F17 => Some(1 << 16),
        Key::F18 => Some(1 << 17),
        Key::F19 => Some(1 << 18),
        Key::F20 => Some(1 << 19),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutModifierState(u16);

impl ShortcutModifierState {
    pub fn update(&mut self, event_type: &EventType) {
        match event_type {
            EventType::KeyPress(key) => {
                if let Some(bit) = shortcut_modifier_bit(*key) {
                    self.0 |= bit;
                }
            }
            EventType::KeyRelease(key) => {
                if let Some(bit) = shortcut_modifier_bit(*key) {
                    self.0 &= !bit;
                }
            }
            _ => {}
        }
    }

    pub fn any(self) -> bool {
        self.0 != 0
    }
}

pub fn shortcut_modifier_bit(key: Key) -> Option<u16> {
    match key {
        Key::ShiftLeft => Some(1 << 0),
        Key::ShiftRight => Some(1 << 1),
        Key::Alt => Some(1 << 2),
        Key::AltGr => Some(1 << 3),
        Key::ControlLeft => Some(1 << 4),
        Key::ControlRight => Some(1 << 5),
        Key::MetaLeft => Some(1 << 6),
        Key::MetaRight => Some(1 << 7),
        _ => None,
    }
}

/// Match the configured physical trigger key, including function keys.
pub fn is_target_key(a: Key, b: Key) -> bool {
    a == b
}

// -- Hold-down detector --

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldDownEvent {
    None,
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HoldState {
    Idle,
    Held,
}

pub struct HoldDownDetector {
    pub clock: Arc<dyn Clock>,
    pub state: HoldState,
    pub target_key: Option<Key>,
    pub last_stopped_at: Option<Instant>,
    /// While a hold is younger than this, any other key press ends it. The
    /// listener commits the recording start only after the window (0 = off).
    pub arming_ms: u128,
    pub held_at: Instant,
}

impl HoldDownDetector {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            state: HoldState::Idle,
            target_key: None,
            last_stopped_at: None,
            arming_ms: 0,
            held_at: clock.now(),
            clock,
        }
    }

    pub fn is_arming(&self) -> bool {
        self.state == HoldState::Held
            && self
                .clock
                .now()
                .saturating_duration_since(self.held_at)
                .as_millis()
                < self.arming_ms
    }

    pub fn held_ms(&self) -> u64 {
        self.clock
            .now()
            .saturating_duration_since(self.held_at)
            .as_millis() as u64
    }

    /// Set the target key. Returns `true` if the detector was in `Held` state
    /// (i.e. the caller should emit a stop event to the frontend).
    pub fn set_target(&mut self, key: Option<Key>) -> bool {
        let was_held = self.state == HoldState::Held;
        if was_held {
            self.state = HoldState::Idle;
            self.last_stopped_at = Some(self.clock.now());
        }
        self.target_key = key;
        was_held
    }

    pub fn reset(&mut self) {
        self.state = HoldState::Idle;
    }

    pub fn in_cooldown(&self) -> bool {
        self.last_stopped_at
            .map(|t| {
                self.clock.now().saturating_duration_since(t).as_millis() < HOLD_DOWN_COOLDOWN_MS
            })
            .unwrap_or(false)
    }

    pub fn log_rejection(&self, reason: RejectionReason, event_type: &EventType) {
        tracing::info!(
            target: "keyboard",
            detector = "hold_down",
            reason = reason.as_str(),
            state = ?self.state,
            target_key = target_key_label(self.target_key),
            event_kind = event_kind(event_type),
            key_class = event_key_class(event_type),
            is_target_key = is_target_event(event_type, self.target_key),
            "keyboard detector rejected sequence"
        );
    }

    /// Process a keyboard event. Returns Start, Stop, or None.
    pub fn handle_event(&mut self, event_type: &EventType) -> HoldDownEvent {
        self.handle_event_with_shortcut_modifiers(event_type, false)
    }

    pub fn handle_event_with_shortcut_modifiers(
        &mut self,
        event_type: &EventType,
        shortcut_modifiers_held: bool,
    ) -> HoldDownEvent {
        self.handle_event_with_context(event_type, shortcut_modifiers_held, false)
    }

    pub fn handle_event_with_context(
        &mut self,
        event_type: &EventType,
        shortcut_modifiers_held: bool,
        repeated_function_press: bool,
    ) -> HoldDownEvent {
        let target = match self.target_key {
            Some(k) => k,
            None => return HoldDownEvent::None,
        };

        let function_chord_rejected = is_function_key(target)
            && match (self.state, event_type) {
                (HoldState::Idle, EventType::KeyPress(key)) => {
                    is_target_key(*key, target)
                        && (shortcut_modifiers_held || repeated_function_press)
                }
                (HoldState::Held, EventType::KeyPress(_)) => shortcut_modifiers_held,
                _ => false,
            };
        if function_chord_rejected {
            self.log_rejection(RejectionReason::ComboCancelled, event_type);
            return match self.state {
                HoldState::Idle => HoldDownEvent::None,
                HoldState::Held => {
                    self.state = HoldState::Idle;
                    self.last_stopped_at = Some(self.clock.now());
                    HoldDownEvent::Stop
                }
            };
        }

        match self.state {
            HoldState::Idle => {
                if let EventType::KeyPress(key) = event_type {
                    if is_target_key(*key, target) && !self.in_cooldown() {
                        self.state = HoldState::Held;
                        self.held_at = self.clock.now();
                        return HoldDownEvent::Start;
                    }
                }
                HoldDownEvent::None
            }

            HoldState::Held => {
                match event_type {
                    EventType::KeyRelease(key) if is_target_key(*key, target) => {
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(self.clock.now());
                        HoldDownEvent::Stop
                    }
                    EventType::KeyPress(key) if is_target_key(*key, target) => {
                        // Key repeat — ignore, stay held
                        HoldDownEvent::None
                    }
                    EventType::KeyPress(_)
                    | EventType::ButtonPress(_)
                    | EventType::Wheel { .. }
                        if self.is_arming() =>
                    {
                        // Shift+letter or Shift+Command: ordinary typing, not
                        // dictation. Ends the hold before the listener commits
                        // a start; the caller logs it without the key.
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(self.clock.now());
                        HoldDownEvent::Stop
                    }
                    EventType::KeyPress(key) if !is_modifier(*key) => {
                        // User is typing a combo like Shift+A — cancel hold
                        self.log_rejection(RejectionReason::ComboCancelled, event_type);
                        self.state = HoldState::Idle;
                        self.last_stopped_at = Some(self.clock.now());
                        HoldDownEvent::Stop
                    }
                    _ => HoldDownEvent::None,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetectorMode {
    DoubleTap,
    HoldDown,
    Both,
}

impl DetectorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DoubleTap => "double_tap",
            Self::HoldDown => "hold_down",
            Self::Both => "both",
        }
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

fn hotkey_id(key: Key) -> Option<&'static str> {
    match key {
        Key::ShiftLeft => Some("shift_l"),
        Key::ShiftRight => Some("shift_r"),
        Key::Alt => Some("alt_l"),
        Key::AltGr => Some("alt_r"),
        Key::ControlLeft => Some("ctrl_l"),
        Key::ControlRight => Some("ctrl_r"),
        Key::Function => Some("fn"),
        Key::F1 => Some("f1"),
        Key::F2 => Some("f2"),
        Key::F3 => Some("f3"),
        Key::F4 => Some("f4"),
        Key::F5 => Some("f5"),
        Key::F6 => Some("f6"),
        Key::F7 => Some("f7"),
        Key::F8 => Some("f8"),
        Key::F9 => Some("f9"),
        Key::F10 => Some("f10"),
        Key::F11 => Some("f11"),
        Key::F12 => Some("f12"),
        Key::F13 => Some("f13"),
        Key::F14 => Some("f14"),
        Key::F15 => Some("f15"),
        Key::F16 => Some("f16"),
        Key::F17 => Some("f17"),
        Key::F18 => Some("f18"),
        Key::F19 => Some("f19"),
        Key::F20 => Some("f20"),
        _ => None,
    }
}
fn event_key(event: &EventType) -> Option<Key> {
    match event {
        EventType::KeyPress(key) | EventType::KeyRelease(key) => Some(*key),
        _ => None,
    }
}
impl Default for DoubleTapDetector {
    fn default() -> Self {
        Self::new()
    }
}
impl Default for HoldDownDetector {
    fn default() -> Self {
        Self::new()
    }
}
