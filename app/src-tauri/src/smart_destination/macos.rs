use super::{
    candidate_action, candidate_is_capturable, candidate_is_preservable, claim_preparation,
    classify_foreground_focus, delivery_is_confirmed, editable_observation, generation_is_current,
    generation_is_latest, handoff_focus_allowed, hands_off, identity_relation,
    manual_failure_to_record, manual_retry_due, notification_invalidates, selection_allowed,
    switched_away_refusal_keeps_clipboard, t3_empty_composer_delivery_is_confirmed,
    verifies_after_post, CandidateAction, CandidateNotification, CapabilityStage, DeliveryRoute,
    EditableStatus, IdentityRelation, InsertOutcome, ManualAccessibilityAttempt, Observation,
    PrepareOutcome, Snapshot,
};
use crate::frontmost::{DeliveryTargetIdentity, DeliveryTargetSnapshot};
use objc2_app_kit::NSRunningApplication;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

type AXError = i32;
type AXUIElementRef = *const c_void;
type AXObserverRef = *const c_void;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CFIndex = isize;
type Boolean = u8;

const AX_SUCCESS: AXError = 0;
const AX_ERROR_FAILURE: AXError = -25200;
const AX_ERROR_ATTRIBUTE_UNSUPPORTED: AXError = -25205;
const AX_ERROR_NO_VALUE: AXError = -25212;
// Chromium can take more than 5 ms to return valid focused-window metadata.
// Keep each call bounded without routinely revoking a healthy destination;
// the separate operation budgets still bound each sampling/delivery pass.
const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.020;
const SAMPLE_AX_BUDGET: Duration = Duration::from_millis(50);
const CAPTURE_AX_BUDGET: Duration = Duration::from_millis(200);
const DELIVERY_AX_BUDGET: Duration = Duration::from_millis(500);
const POST_SETTLE: Duration = Duration::from_millis(40);
const UTF8_ENCODING: u32 = 0x0800_0100;
const AX_VALUE_CFRANGE_TYPE: u32 = 4;
const MAX_ANCESTOR_STEPS: usize = 40;
const T3_BUNDLE_ID: &str = "com.t3tools.t3code";

type AXObserverCallback = unsafe extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut Boolean,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> AXError;
    fn AXObserverCreate(
        application: i32,
        callback: AXObserverCallback,
        observer: *mut AXObserverRef,
    ) -> AXError;
    fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> AXError;
    fn AXObserverRemoveNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
    ) -> AXError;
    fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFRunLoopSourceRef;
    fn AXValueGetType(value: CFTypeRef) -> u32;
    fn AXValueGetValue(value: CFTypeRef, value_type: u32, value_ptr: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;

    fn CFRetain(value: CFTypeRef) -> CFTypeRef;
    fn CFRelease(value: CFTypeRef);
    fn CFEqual(left: CFTypeRef, right: CFTypeRef) -> Boolean;
    fn CFGetTypeID(value: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: CFTypeRef) -> Boolean;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CFTypeRef, number_type: u32, value: *mut c_void) -> bool;
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        string: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetLength(string: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
}

struct OwnedCf(CFTypeRef);

impl OwnedCf {
    unsafe fn from_create(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }

    fn as_type(&self) -> CFTypeRef {
        self.0
    }

    fn as_element(&self) -> AXUIElementRef {
        self.0
    }
}

impl Clone for OwnedCf {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.0);
        }
        Self(self.0)
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

fn same_ref(left: &OwnedCf, right: &OwnedCf) -> bool {
    unsafe { CFEqual(left.0, right.0) != 0 }
}

fn same_optional_ref(left: Option<&OwnedCf>, right: Option<&OwnedCf>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_ref(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

struct ObserverRegistration {
    observer: OwnedCf,
    source: CFRunLoopSourceRef,
    app: OwnedCf,
    element: OwnedCf,
    signals: Arc<ObserverSignals>,
    // AX retains the callback pointer; each context must stay at a stable address.
    #[allow(clippy::vec_box)]
    _contexts: Vec<Box<ObserverContext>>,
}

impl Drop for ObserverRegistration {
    fn drop(&mut self) {
        unsafe {
            for notification in [
                "AXValueChanged",
                "AXSelectedTextChanged",
                "AXUIElementDestroyed",
            ] {
                if let Some(notification) = cfstring(notification) {
                    let _ = AXObserverRemoveNotification(
                        self.observer.0,
                        self.element.as_element(),
                        notification.0,
                    );
                }
            }
            if let Some(notification) = cfstring("AXFocusedUIElementChanged") {
                let _ = AXObserverRemoveNotification(
                    self.observer.0,
                    self.app.as_element(),
                    notification.0,
                );
            }
            if !self.source.is_null() {
                CFRunLoopRemoveSource(CFRunLoopGetMain(), self.source, kCFRunLoopCommonModes);
            }
        }
    }
}

struct Candidate {
    identity: DeliveryTargetSnapshot,
    launch_token_bits: u64,
    app: OwnedCf,
    window: OwnedCf,
    field: OwnedCf,
    document: Option<OwnedCf>,
    observed_at: Instant,
    revision: u64,
    registration_epoch: u64,
    handled_hard_epoch: u64,
    handled_selection_epoch: u64,
    handled_focus_epoch: u64,
    valid: bool,
    _observer: ObserverRegistration,
}

struct Frozen {
    generation: u64,
    revision: u64,
    expected_foreground: DeliveryTargetSnapshot,
    destination_identity: DeliveryTargetSnapshot,
    destination_launch_token_bits: u64,
    expected_foreground_launch_token_bits: u64,
    app: OwnedCf,
    window: OwnedCf,
    field: OwnedCf,
    document: Option<OwnedCf>,
    range: OwnedCf,
    route: DeliveryRoute,
    stage: CapabilityStage,
    prepared_character_count: Option<usize>,
    /// The application the user switched to while speaking, for a
    /// current-app destination handed back to at delivery. Set at prepare;
    /// focus returns there afterwards, as it does to the media app.
    switched_away_from: Option<(DeliveryTargetSnapshot, u64)>,
}

impl Frozen {
    fn hands_off(&self) -> bool {
        hands_off(self.route, self.switched_away_from.is_some())
    }

    /// Where focus returns after a handoff: the media app, or the application
    /// the user switched to.
    fn return_target(&self) -> (&DeliveryTargetSnapshot, u64) {
        match &self.switched_away_from {
            Some((snapshot, launch_token_bits)) => (snapshot, *launch_token_bits),
            None => (
                &self.expected_foreground,
                self.expected_foreground_launch_token_bits,
            ),
        }
    }
}

struct ManualOverride {
    identity: DeliveryTargetSnapshot,
    launch_token_bits: u64,
    app: OwnedCf,
}

impl Drop for ManualOverride {
    fn drop(&mut self) {
        if !process_is_running(&self.identity, self.launch_token_bits) {
            return;
        }
        let Some(current) = copy_bool_attribute(self.app.as_element(), "AXManualAccessibility")
        else {
            return;
        };
        if current {
            let _ = set_attribute(self.app.as_element(), "AXManualAccessibility", unsafe {
                kCFBooleanFalse
            });
        }
    }
}

#[derive(Default)]
struct State {
    generation: u64,
    next_token: u64,
    next_revision: u64,
    next_registration_epoch: u64,
    candidate: Option<Candidate>,
    frozen: HashMap<u64, Frozen>,
    manual_override: Option<ManualOverride>,
    manual_attempted: Option<(DeliveryTargetSnapshot, Instant)>,
}

impl State {
    fn sync_generation(&mut self, generation: u64) {
        if self.generation == generation {
            return;
        }
        self.candidate = None;
        self.frozen.clear();
        self.manual_override = None;
        self.manual_attempted = None;
        self.generation = generation;
    }

    fn clear_candidate(&mut self) {
        self.candidate = None;
        self.frozen.clear();
    }

    fn invalidate_candidate(&mut self) {
        self.next_revision = self.next_revision.wrapping_add(1).max(1);
        self.frozen.clear();
        if let Some(candidate) = self.candidate.as_mut() {
            candidate.valid = false;
            candidate.revision = self.next_revision;
        }
    }

    fn observe(&mut self, sample: EditableSample) {
        let unchanged = self.candidate.as_ref().is_some_and(|candidate| {
            candidate.valid
                && same_process(&candidate.identity, &sample.identity)
                && same_ref(&candidate.window, &sample.window)
                && same_ref(&candidate.field, &sample.field)
                && same_optional_ref(candidate.document.as_ref(), sample.document.as_ref())
        });
        if unchanged {
            if let Some(candidate) = self.candidate.as_mut() {
                candidate.identity = sample.identity;
                candidate.launch_token_bits = sample.launch_token_bits;
                candidate.observed_at = Instant::now();
            }
            return;
        }

        self.next_revision = self.next_revision.wrapping_add(1).max(1);
        self.frozen.clear();
        self.next_registration_epoch = self.next_registration_epoch.wrapping_add(1).max(1);
        let registration_epoch = self.next_registration_epoch;
        let Some(observer) = register_candidate_observer(
            complete_identity(&sample.identity)
                .map(|identity| identity.process_id)
                .unwrap_or_default(),
            &sample.app,
            &sample.field,
            registration_epoch,
        ) else {
            self.candidate = None;
            return;
        };
        self.candidate = Some(Candidate {
            identity: sample.identity,
            launch_token_bits: sample.launch_token_bits,
            app: sample.app,
            window: sample.window,
            field: sample.field,
            document: sample.document,
            observed_at: Instant::now(),
            revision: self.next_revision,
            registration_epoch,
            handled_hard_epoch: 0,
            handled_selection_epoch: 0,
            handled_focus_epoch: 0,
            valid: true,
            _observer: observer,
        });
    }

    fn reconcile_observer_epochs(&mut self) {
        let Some(candidate) = self.candidate.as_mut() else {
            return;
        };
        let signals = &candidate._observer.signals;
        let registration_mismatch = signals.registration_epoch != candidate.registration_epoch;
        let hard_epoch = signals.hard_epoch.load(Ordering::Acquire);
        let selection_epoch = signals.selection_epoch.load(Ordering::Acquire);
        let focus_epoch = signals.focus_epoch.load(Ordering::Acquire);
        let hard_pending = hard_epoch > candidate.handled_hard_epoch;
        let selection_pending = selection_epoch > candidate.handled_selection_epoch;
        let focus_pending = focus_epoch > candidate.handled_focus_epoch;
        candidate.handled_hard_epoch = hard_epoch;
        candidate.handled_selection_epoch = selection_epoch;
        candidate.handled_focus_epoch = focus_epoch;

        if registration_mismatch
            || hard_pending
                && notification_invalidates(CandidateNotification::ValueOrDestroyed, false)
            || selection_pending
                && notification_invalidates(CandidateNotification::Selection, false)
        {
            self.invalidate_candidate();
            return;
        }
        if !focus_pending {
            return;
        }

        let retained = self.candidate.as_ref().is_some_and(|candidate| {
            !focus_transition_invalidates(candidate_focus_transition(candidate))
        });
        if notification_invalidates(CandidateNotification::Focus, retained) {
            self.invalidate_candidate();
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static AX_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

struct EditableSample {
    identity: DeliveryTargetSnapshot,
    launch_token_bits: u64,
    app: OwnedCf,
    window: OwnedCf,
    field: OwnedCf,
    document: Option<OwnedCf>,
}

enum CurrentObservation {
    Editable(EditableSample),
    /// Positively editable, but the window or document ancestor could not be
    /// captured, so it cannot be a destination. It still revokes the old one.
    EditableUnverified,
    SupportedMedia,
    SelfOverlay,
    Secure,
    Other,
    Unavailable,
}

impl CurrentObservation {
    fn policy(&self) -> Observation {
        match self {
            Self::Editable(_) => Observation::Editable,
            Self::EditableUnverified => Observation::EditableUnverified,
            Self::SupportedMedia => Observation::SupportedMedia,
            Self::SelfOverlay => Observation::SelfOverlay,
            Self::Secure => Observation::Secure,
            Self::Other => Observation::Other,
            Self::Unavailable => Observation::Unavailable,
        }
    }
}

pub(super) fn sample(generation: u64) {
    if !is_main_thread() {
        return;
    }
    with_ax_budget(SAMPLE_AX_BUDGET, || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            // A queued sampler from an older enablement generation must never
            // roll back newer native state.
            if !generation_is_latest(generation) {
                return;
            }
            state.sync_generation(generation);
            if !generation_is_current(generation) {
                state.clear_candidate();
                state.manual_override = None;
                return;
            }

            state.reconcile_observer_epochs();
            let observation = inspect_current(&mut state);
            apply_observation(&mut state, observation);
        });
    });
}

pub(super) fn disable_if_main_thread(generation: u64) {
    if !is_main_thread() {
        return;
    }
    with_ax_budget(SAMPLE_AX_BUDGET, || {
        STATE.with(|state| {
            if let Ok(mut state) = state.try_borrow_mut() {
                state.sync_generation(generation);
            }
        });
    });
}

/// Smart destination is additive: anything other than a secure field, a media
/// control without a retained field, or revocation returns `Unverified`, which
/// delivers through the normal verified auto-paste policy exactly as with the
/// feature off (that policy does its own self, process, and stop-anchor checks).
/// `fallback_vetoed` is set as soon as such refusal evidence (or the media
/// route) is found, before observer cleanup that could outlive the timeout.
pub(super) fn capture(
    expected: &DeliveryTargetSnapshot,
    generation: u64,
    fallback_vetoed: &AtomicBool,
) -> Snapshot {
    let veto = || fallback_vetoed.store(true, Ordering::Release);
    if !is_main_thread() || !generation_is_current(generation) {
        veto();
        return Snapshot::Unavailable;
    }
    with_ax_budget(CAPTURE_AX_BUDGET, || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.sync_generation(generation);
            state.reconcile_observer_epochs();

            let current = crate::frontmost::capture_delivery_target_snapshot();
            // Focus moving to another app (or Murmur) is not a reason to be
            // stricter than normal paste, which re-verifies its own anchor.
            if identity_relation(&current, expected) != IdentityRelation::Same {
                return Snapshot::Unverified;
            }

            let observation = inspect_current_from_snapshot(&mut state, current);
            let route = match &observation {
                CurrentObservation::Editable(_) => DeliveryRoute::CurrentEditable,
                CurrentObservation::SupportedMedia => {
                    // An unfinished media capture must never fall back to
                    // normal paste, which would target the media app.
                    veto();
                    DeliveryRoute::RecentMedia
                }
                CurrentObservation::Secure => {
                    veto();
                    apply_observation(&mut state, observation);
                    return Snapshot::Unavailable;
                }
                CurrentObservation::EditableUnverified
                | CurrentObservation::SelfOverlay
                | CurrentObservation::Other
                | CurrentObservation::Unavailable => {
                    apply_observation(&mut state, observation);
                    return Snapshot::Unverified;
                }
            };
            apply_observation(&mut state, observation);

            freeze_candidate(&mut state, expected.clone(), generation, route).unwrap_or(match route
            {
                // The field could not be frozen (budget, observer, caret,
                // or a focus change): use normal paste into this app.
                DeliveryRoute::CurrentEditable => Snapshot::Unverified,
                // No retained field behind the media control.
                DeliveryRoute::RecentMedia => Snapshot::Unavailable,
            })
        })
    })
}

pub(super) fn prepare(snapshot: &Snapshot, allow_activation: bool) -> PrepareOutcome {
    if !is_main_thread() {
        return PrepareOutcome::Refused;
    }
    let Snapshot::Target {
        token,
        revision,
        identity,
        ..
    } = snapshot
    else {
        return PrepareOutcome::Refused;
    };

    with_ax_budget(DELIVERY_AX_BUDGET, || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.reconcile_observer_epochs();
            let current = crate::frontmost::capture_delivery_target_snapshot();
            // A current-app destination whose application is no longer in
            // front: the user switched away while speaking.
            let switched_away = allow_activation
                && state.frozen.get(token).is_some_and(|frozen| {
                    frozen.route == DeliveryRoute::CurrentEditable
                        && !same_process(&current, &frozen.expected_foreground)
                });
            let switched_away_from = if switched_away {
                running_process_launch_bits(&current).map(|bits| (current.clone(), bits))
            } else {
                None
            };
            // One focus observation of the app now in front. Only an app that
            // positively holds no text input may hand the paste back to the
            // retained field; any other app is the user's new destination,
            // unless its focused field is positively secure.
            let foreground_focus = switched_away.then(|| foreground_focus_observation(&current));
            let handoff_focus = foreground_focus.is_some_and(handoff_focus_allowed);
            let refusal_keeps_clipboard =
                foreground_focus.is_some_and(switched_away_refusal_keeps_clipboard);
            let valid = state.frozen.get(token).is_some_and(|frozen| {
                frozen.revision == *revision
                    && frozen.destination_identity == *identity
                    && frozen.stage == CapabilityStage::Captured
                    && (allow_activation || frozen.route == DeliveryRoute::CurrentEditable)
                    && generation_is_current(frozen.generation)
                    && if switched_away {
                        switched_away_from.is_some()
                            && handoff_focus
                            && validate_switched_away_for_prepare(&state, frozen)
                    } else {
                        validate_frozen_for_prepare(&state, frozen)
                    }
            });
            if !valid {
                state.frozen.remove(token);
                // Nothing was activated. When the app in front may hold a
                // text input, the caller's normal verified paste goes to the
                // app the user stopped in, exactly as with the feature off. A
                // positively secure field there stays clipboard-only even if
                // the later injector probe cannot see it.
                return refused(refusal_keeps_clipboard);
            }

            // The capability becomes one-shot before activation is attempted.
            // A repeated prepare can neither activate again nor later insert.
            let route = {
                let frozen = state
                    .frozen
                    .get_mut(token)
                    .expect("validated frozen destination is present");
                if !claim_preparation(&mut frozen.stage) {
                    state.frozen.remove(token);
                    return refused(switched_away);
                }
                frozen.prepared_character_count =
                    copy_number_attribute(frozen.field.as_element(), "AXNumberOfCharacters");
                frozen.switched_away_from = switched_away_from;
                frozen.route
            };

            // The validation above blocks on Accessibility. The user may have
            // moved on again, possibly into a text input: the application in
            // front must still be the one that was judged.
            if switched_away
                && !same_process(
                    &crate::frontmost::capture_delivery_target_snapshot(),
                    &current,
                )
            {
                state.frozen.remove(token);
                return PrepareOutcome::RefusedClipboardOnly;
            }

            match route {
                DeliveryRoute::CurrentEditable if !switched_away => PrepareOutcome::Ready,
                DeliveryRoute::CurrentEditable | DeliveryRoute::RecentMedia => {
                    let activated = state
                        .frozen
                        .get(token)
                        .is_some_and(request_recent_destination_activation);
                    if activated {
                        PrepareOutcome::WaitForActivation
                    } else {
                        state.frozen.remove(token);
                        refused(switched_away)
                    }
                }
            }
        })
    })
}

pub(super) fn abort(snapshot: &Snapshot, pasteboard_change: Option<i64>) {
    if !is_main_thread() {
        return;
    }
    let Snapshot::Target {
        token,
        revision,
        identity,
        ..
    } = snapshot
    else {
        return;
    };

    with_ax_budget(DELIVERY_AX_BUDGET, || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.reconcile_observer_epochs();
            let Some(frozen) = state.frozen.remove(token) else {
                return;
            };
            state.frozen.clear();
            if frozen.revision != *revision
                || frozen.destination_identity != *identity
                || frozen.stage != CapabilityStage::Prepared
                || !generation_is_current(frozen.generation)
            {
                return;
            }
            if let (Some(expected_range), Some(pasteboard_change)) =
                (decode_range(frozen.range.as_type()), pasteboard_change)
            {
                maybe_restore_media_before_post(
                    &frozen,
                    expected_range,
                    frozen.prepared_character_count,
                    pasteboard_change,
                );
            }
            state.invalidate_candidate();
        });
    });
}

pub(super) fn insert(snapshot: &Snapshot, text: &str, pasteboard_change: i64) -> InsertOutcome {
    if !is_main_thread() {
        return InsertOutcome::Refused;
    }
    let Snapshot::Target {
        token,
        revision,
        identity,
        ..
    } = snapshot
    else {
        return InsertOutcome::Refused;
    };

    with_ax_budget(DELIVERY_AX_BUDGET, || {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.reconcile_observer_epochs();
            let valid = state.frozen.get(token).is_some_and(|frozen| {
                frozen.revision == *revision
                    && frozen.destination_identity == *identity
                    && frozen.stage == CapabilityStage::Prepared
                    && generation_is_current(frozen.generation)
                    && validate_frozen_candidate(&state, frozen)
            });
            if !valid {
                state.frozen.remove(token);
                return InsertOutcome::Refused;
            }

            // Retire before validation or posting. There is exactly one final
            // delivery attempt for a prepared recording token and no retry.
            let Some(frozen) = state.frozen.remove(token) else {
                return InsertOutcome::Refused;
            };
            state.frozen.clear();

            let Some(expected_range) = decode_range(frozen.range.as_type()) else {
                state.invalidate_candidate();
                return InsertOutcome::Refused;
            };
            let inserted_length = text.encode_utf16().count();
            if inserted_length == 0 {
                state.invalidate_candidate();
                return InsertOutcome::Refused;
            }
            let character_count_before =
                copy_number_attribute(frozen.field.as_element(), "AXNumberOfCharacters");

            let ready_to_post = destination_is_active(&frozen)
                && validate_exact_field(&frozen, expected_range)
                && (!frozen.hands_off()
                    || frozen.prepared_character_count == character_count_before)
                && crate::injector::pasteboard_change_count() == Some(pasteboard_change);
            if !ready_to_post {
                maybe_restore_media_before_post(
                    &frozen,
                    expected_range,
                    frozen.prepared_character_count,
                    pasteboard_change,
                );
                state.invalidate_candidate();
                return InsertOutcome::Refused;
            }

            if crate::injector::simulate_smart_paste_native().is_err() {
                maybe_restore_media_before_post(
                    &frozen,
                    expected_range,
                    frozen.prepared_character_count,
                    pasteboard_change,
                );
                state.invalidate_candidate();
                return InsertOutcome::Refused;
            }

            // The read-back below only gates returning focus after a handoff.
            // Delivery into the current app has nothing to decide, so it skips
            // the settle.
            if verifies_after_post(frozen.route, frozen.switched_away_from.is_some()) {
                let _ = sleep_within_budget(POST_SETTLE);
                let range_after = copy_attribute(frozen.field.as_element(), "AXSelectedTextRange")
                    .ok()
                    .and_then(|value| decode_range(value.as_type()));
                let character_count_after =
                    copy_number_attribute(frozen.field.as_element(), "AXNumberOfCharacters");
                let confirmed = delivery_is_confirmed(
                    expected_range,
                    inserted_length,
                    range_after,
                    character_count_before,
                    character_count_after,
                ) || complete_identity(&frozen.destination_identity)
                    .is_some_and(|identity| known_t3_bundle_id(&identity.bundle_id))
                    && t3_empty_composer_delivery_is_confirmed(
                        expected_range,
                        inserted_length,
                        range_after,
                        character_count_before,
                        character_count_after,
                    );
                if confirmed {
                    maybe_restore_media_after_post(
                        &frozen,
                        range_after.expect("confirmed range is present"),
                        character_count_after.expect("confirmed count is present"),
                        pasteboard_change,
                    );
                }
            }
            state.invalidate_candidate();
            InsertOutcome::Posted
        })
    })
}

fn apply_observation(state: &mut State, observation: CurrentObservation) {
    let has_recent = state.candidate.as_ref().is_some_and(|candidate| {
        candidate_is_preservable(
            candidate.valid,
            candidate.observed_at.elapsed(),
            !state.frozen.is_empty(),
        )
    });
    match candidate_action(observation.policy(), has_recent) {
        CandidateAction::Replace => {
            if let CurrentObservation::Editable(sample) = observation {
                state.observe(sample);
            }
        }
        CandidateAction::Preserve => {}
        CandidateAction::Clear => state.clear_candidate(),
    }
}

fn freeze_candidate(
    state: &mut State,
    expected: DeliveryTargetSnapshot,
    generation: u64,
    route: DeliveryRoute,
) -> Option<Snapshot> {
    let candidate = state.candidate.as_ref()?;
    let candidate_is_valid = match route {
        DeliveryRoute::CurrentEditable => validate_candidate_focused(candidate),
        DeliveryRoute::RecentMedia => validate_retained_candidate(candidate),
    };
    if !candidate_is_capturable(candidate.valid, candidate.observed_at.elapsed())
        || !candidate_is_valid
    {
        state.clear_candidate();
        return None;
    }

    let range = copy_attribute(candidate.field.as_element(), "AXSelectedTextRange").ok()?;
    let decoded_range = decode_range(range.as_type())?;
    if !selection_allowed(route, decoded_range.1) {
        return None;
    }
    let expected_foreground_launch_token_bits = running_process_launch_bits(&expected)?;

    state.next_token = state.next_token.wrapping_add(1).max(1);
    let token = state.next_token;
    let revision = candidate.revision;
    let destination_identity = candidate.identity.clone();
    let frozen = Frozen {
        generation,
        revision,
        expected_foreground: expected.clone(),
        destination_identity: destination_identity.clone(),
        destination_launch_token_bits: candidate.launch_token_bits,
        expected_foreground_launch_token_bits,
        app: candidate.app.clone(),
        window: candidate.window.clone(),
        field: candidate.field.clone(),
        document: candidate.document.clone(),
        range,
        route,
        stage: CapabilityStage::Captured,
        prepared_character_count: None,
        switched_away_from: None,
    };
    // One live capability is sufficient for the single recording owner and
    // bounds retained native references even if a caller abandons a snapshot.
    state.frozen.clear();
    state.frozen.insert(token, frozen);
    Some(Snapshot::Target {
        token,
        revision,
        identity: destination_identity,
        current_app: route == DeliveryRoute::CurrentEditable,
    })
}

fn validate_frozen_for_prepare(state: &State, frozen: &Frozen) -> bool {
    if !same_process(
        &crate::frontmost::capture_delivery_target_snapshot(),
        &frozen.expected_foreground,
    ) || !process_is_running(
        &frozen.expected_foreground,
        frozen.expected_foreground_launch_token_bits,
    ) {
        return false;
    }
    match frozen.route {
        DeliveryRoute::CurrentEditable => {}
        // The media handoff was decided when recording started. The control
        // no longer holding focus is not a revocation; only the user having
        // moved into another editable or a secure field is.
        DeliveryRoute::RecentMedia => {
            if !handoff_focus_allowed(foreground_focus_observation(&frozen.expected_foreground)) {
                return false;
            }
        }
    }

    if !validate_frozen_candidate(state, frozen) {
        return false;
    }

    let Some(expected_range) = decode_range(frozen.range.as_type()) else {
        return false;
    };
    match frozen.route {
        DeliveryRoute::CurrentEditable => validate_exact_field(frozen, expected_range),
        DeliveryRoute::RecentMedia => validate_retained_field(frozen, expected_range),
    }
}

/// A handoff preparation that does not complete keeps the transcript on the
/// clipboard: the app in front holds no text input to paste into, and an
/// activation may already have moved focus, so the normal paste fallback is no
/// longer bound to a known field.
fn refused(switched_away: bool) -> PrepareOutcome {
    if switched_away {
        PrepareOutcome::RefusedClipboardOnly
    } else {
        PrepareOutcome::Refused
    }
}

/// A current-app destination after the user switched apps while speaking.
/// Exactly what a media handoff requires: the caller has already observed that
/// the application now in front positively holds no text input, and the
/// retained field must still be the captured one with a collapsed caret where
/// it was. A background selection is never replaced.
fn validate_switched_away_for_prepare(state: &State, frozen: &Frozen) -> bool {
    if !validate_frozen_candidate(state, frozen) {
        return false;
    }
    decode_range(frozen.range.as_type()).is_some_and(|expected_range| {
        selection_allowed(DeliveryRoute::RecentMedia, expected_range.1)
            && validate_retained_field(frozen, expected_range)
    })
}

fn validate_frozen_candidate(state: &State, frozen: &Frozen) -> bool {
    state.candidate.as_ref().is_some_and(|candidate| {
        candidate.valid
            && candidate.revision == frozen.revision
            && same_process(&candidate.identity, &frozen.destination_identity)
            && same_ref(&candidate.window, &frozen.window)
            && same_ref(&candidate.field, &frozen.field)
            && same_optional_ref(candidate.document.as_ref(), frozen.document.as_ref())
            && process_is_running(
                &frozen.destination_identity,
                frozen.destination_launch_token_bits,
            )
    })
}

fn validate_exact_field(frozen: &Frozen, expected_range: (usize, usize)) -> bool {
    let Ok(current_window) = copy_element_attribute(frozen.app.as_element(), "AXFocusedWindow")
    else {
        return false;
    };
    let Ok(current_field) = copy_element_attribute(frozen.app.as_element(), "AXFocusedUIElement")
    else {
        return false;
    };
    if !same_ref(&current_window, &frozen.window)
        || !same_ref(&current_field, &frozen.field)
        || editable_status(current_field.as_element()) != EditableStatus::Editable
    {
        return false;
    }

    let current_document = retained_document_ancestor(&current_field);
    if !same_optional_ref(current_document.as_ref(), frozen.document.as_ref()) {
        return false;
    }
    copy_attribute(current_field.as_element(), "AXSelectedTextRange")
        .ok()
        .and_then(|value| decode_range(value.as_type()))
        == Some(expected_range)
}

fn destination_is_active(frozen: &Frozen) -> bool {
    running_application(
        &frozen.destination_identity,
        frozen.destination_launch_token_bits,
    )
    .is_some_and(|application| application.isActive())
}

fn request_recent_destination_activation(frozen: &Frozen) -> bool {
    request_application_activation(
        &frozen.destination_identity,
        frozen.destination_launch_token_bits,
        &frozen.app,
    )
}

fn request_application_activation(
    identity: &DeliveryTargetSnapshot,
    launch_token_bits: u64,
    app: &OwnedCf,
) -> bool {
    // AppKit may refuse cooperative activation from background Murmur. Use
    // the granted Accessibility capability to activate this exact application,
    // without changing the focused field or selection. The later main-thread
    // insertion turn must still prove that the original field gained focus.
    attribute_settable(app.as_element(), "AXFrontmost")
        && process_is_running(identity, launch_token_bits)
        && set_attribute(app.as_element(), "AXFrontmost", unsafe { kCFBooleanTrue }) == AX_SUCCESS
}

fn maybe_restore_media_before_post(
    frozen: &Frozen,
    expected_range: (usize, usize),
    expected_count: Option<usize>,
    pasteboard_change: i64,
) {
    let Some(expected_count) = expected_count else {
        return;
    };
    maybe_restore_media(frozen, expected_range, expected_count, pasteboard_change);
}

fn maybe_restore_media_after_post(
    frozen: &Frozen,
    expected_range: (usize, usize),
    expected_count: usize,
    pasteboard_change: i64,
) {
    maybe_restore_media(frozen, expected_range, expected_count, pasteboard_change);
}

fn maybe_restore_media(
    frozen: &Frozen,
    expected_range: (usize, usize),
    expected_count: usize,
    pasteboard_change: i64,
) {
    let (return_to, return_launch_token_bits) = frozen.return_target();
    if !frozen.hands_off()
        || crate::injector::pasteboard_change_count() != Some(pasteboard_change)
        || !destination_is_active(frozen)
        || !validate_exact_field(frozen, expected_range)
        || copy_number_attribute(frozen.field.as_element(), "AXNumberOfCharacters")
            != Some(expected_count)
        || !process_is_running(return_to, return_launch_token_bits)
    {
        return;
    }
    if let Some(app) =
        complete_identity(return_to).and_then(|identity| create_application(identity.process_id))
    {
        let _ = request_application_activation(return_to, return_launch_token_bits, &app);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CandidateFocusTransition {
    ExactField,
    BackgroundBlur,
    DifferentField,
    Unavailable,
}

fn focus_transition_invalidates(transition: CandidateFocusTransition) -> bool {
    matches!(
        transition,
        CandidateFocusTransition::DifferentField | CandidateFocusTransition::Unavailable
    )
}

fn candidate_focus_transition(candidate: &Candidate) -> CandidateFocusTransition {
    if !process_is_running(&candidate.identity, candidate.launch_token_bits) {
        return CandidateFocusTransition::Unavailable;
    }
    match copy_element_attribute(candidate.app.as_element(), "AXFocusedUIElement") {
        Ok(field)
            if same_ref(&field, &candidate.field) && validate_candidate_focused(candidate) =>
        {
            CandidateFocusTransition::ExactField
        }
        Ok(_) => CandidateFocusTransition::DifferentField,
        Err(AX_ERROR_NO_VALUE) if validate_retained_candidate(candidate) => {
            CandidateFocusTransition::BackgroundBlur
        }
        Err(_) => CandidateFocusTransition::Unavailable,
    }
}

fn validate_candidate_focused(candidate: &Candidate) -> bool {
    if !validate_retained_candidate(candidate) {
        return false;
    }
    let Ok(window) = copy_element_attribute(candidate.app.as_element(), "AXFocusedWindow") else {
        return false;
    };
    let Ok(field) = copy_element_attribute(candidate.app.as_element(), "AXFocusedUIElement") else {
        return false;
    };
    same_ref(&window, &candidate.window) && same_ref(&field, &candidate.field)
}

fn validate_retained_candidate(candidate: &Candidate) -> bool {
    process_is_running(&candidate.identity, candidate.launch_token_bits)
        && retained_field_identity_matches(
            &candidate.field,
            &candidate.window,
            candidate.document.as_ref(),
        )
}

fn validate_retained_field(frozen: &Frozen, expected_range: (usize, usize)) -> bool {
    retained_field_identity_matches(&frozen.field, &frozen.window, frozen.document.as_ref())
        && copy_attribute(frozen.field.as_element(), "AXSelectedTextRange")
            .ok()
            .and_then(|range| decode_range(range.as_type()))
            == Some(expected_range)
}

fn retained_field_identity_matches(
    field: &OwnedCf,
    expected_window: &OwnedCf,
    expected_document: Option<&OwnedCf>,
) -> bool {
    if editable_status(field.as_element()) != EditableStatus::Editable {
        return false;
    }
    let Ok(window) = copy_element_attribute(field.as_element(), "AXWindow") else {
        return false;
    };
    same_ref(&window, expected_window)
        && same_optional_ref(
            retained_document_ancestor(field).as_ref(),
            expected_document,
        )
}

fn inspect_current(state: &mut State) -> CurrentObservation {
    let snapshot = crate::frontmost::capture_delivery_target_snapshot();
    inspect_current_from_snapshot(state, snapshot)
}

fn inspect_current_from_snapshot(
    state: &mut State,
    snapshot: DeliveryTargetSnapshot,
) -> CurrentObservation {
    let identity = match &snapshot {
        DeliveryTargetSnapshot::Complete(identity) => identity,
        DeliveryTargetSnapshot::SelfTarget => return CurrentObservation::SelfOverlay,
        DeliveryTargetSnapshot::Incomplete | DeliveryTargetSnapshot::Mismatch(_) => {
            return CurrentObservation::Unavailable
        }
    };

    let Some(app) = create_application(identity.process_id) else {
        return CurrentObservation::Unavailable;
    };
    let Some(launch_token_bits) = running_process_launch_bits(&snapshot) else {
        return CurrentObservation::Unavailable;
    };
    let mut window = copy_element_attribute(app.as_element(), "AXFocusedWindow").ok();
    let mut field = copy_element_attribute(app.as_element(), "AXFocusedUIElement").ok();
    let mut status = field
        .as_ref()
        .map(|field| editable_status(field.as_element()))
        .unwrap_or(EditableStatus::Unavailable);

    if status != EditableStatus::Editable
        && status != EditableStatus::Secure
        && !crate::browser_site::allowed_browser(&identity.bundle_id)
        && enable_manual_accessibility_if_needed(state, &snapshot, &app)
    {
        window = copy_element_attribute(app.as_element(), "AXFocusedWindow").ok();
        field = copy_element_attribute(app.as_element(), "AXFocusedUIElement").ok();
        status = field
            .as_ref()
            .map(|field| editable_status(field.as_element()))
            .unwrap_or(EditableStatus::Unavailable);
    }

    match status {
        EditableStatus::Secure => CurrentObservation::Secure,
        EditableStatus::Editable => {
            let document = field.as_ref().and_then(retained_document_ancestor);
            let requires_document = crate::browser_site::allowed_browser(&identity.bundle_id)
                || manual_accessibility_applies(state, &snapshot, &app);
            let (Some(window), Some(field)) = (window, field) else {
                return CurrentObservation::EditableUnverified;
            };
            if editable_observation(true, requires_document, document.is_some())
                != Observation::Editable
            {
                return CurrentObservation::EditableUnverified;
            }
            if !same_process(
                &crate::frontmost::capture_delivery_target_snapshot(),
                &snapshot,
            ) {
                return CurrentObservation::Unavailable;
            }
            CurrentObservation::Editable(EditableSample {
                identity: snapshot,
                launch_token_bits,
                app,
                window,
                field,
                document,
            })
        }
        EditableStatus::NotEditable | EditableStatus::Unavailable => {
            if current_is_supported_media_with_field(&snapshot, field.as_ref()) {
                CurrentObservation::SupportedMedia
            } else if status == EditableStatus::Unavailable {
                CurrentObservation::Unavailable
            } else {
                CurrentObservation::Other
            }
        }
    }
}

fn editable_status(element: AXUIElementRef) -> EditableStatus {
    if let Err(status) = classify_text_role(copy_string_attribute_status(element, "AXRole"), || {
        copy_string_attribute_status(element, "AXSubrole")
    }) {
        return status;
    }
    if !copy_bool_attribute(element, "AXEnabled").is_some_and(|enabled| enabled)
        || !attribute_settable(element, "AXSelectedText")
    {
        return EditableStatus::Unavailable;
    }
    EditableStatus::Editable
}

/// Classify role evidence. `Err` is final; `Ok` means a text role that still
/// needs the enabled/settable checks. The subrole is queried only after the
/// role, so positive secure evidence wins before another fallible query could
/// downgrade it to `Unavailable`, which delivery treats as "nothing proven".
fn classify_text_role(
    role: Result<String, AXError>,
    subrole: impl FnOnce() -> Result<String, AXError>,
) -> Result<(), EditableStatus> {
    let role = match role {
        Ok(role) => role,
        Err(status) if benign_absence(status) => return Err(EditableStatus::NotEditable),
        Err(_) => return Err(EditableStatus::Unavailable),
    };
    if role.trim() == "AXSecureTextField" {
        return Err(EditableStatus::Secure);
    }
    let subrole = match subrole() {
        Ok(subrole) => Some(subrole),
        Err(status) if benign_absence(status) => None,
        Err(_) => return Err(EditableStatus::Unavailable),
    };
    if subrole
        .as_deref()
        .is_some_and(|subrole| subrole.trim() == "AXSecureTextField")
    {
        return Err(EditableStatus::Secure);
    }
    if !matches!(role.trim(), "AXTextArea" | "AXTextField") {
        return Err(EditableStatus::NotEditable);
    }
    Ok(())
}

/// Classify the expected foreground app's focus without any side effects.
/// Only positive evidence authorizes delivery; see `classify_foreground_focus`.
fn foreground_focus_observation(expected: &DeliveryTargetSnapshot) -> Observation {
    let Some(identity) = complete_identity(expected) else {
        return Observation::Unavailable;
    };
    let Some(app) = create_application(identity.process_id) else {
        return Observation::Unavailable;
    };
    let field = copy_element_attribute(app.as_element(), "AXFocusedUIElement").ok();
    let status = field
        .as_ref()
        .map(|field| editable_status(field.as_element()));
    classify_foreground_focus(status, || {
        current_is_supported_media_with_field(expected, field.as_ref())
    })
}

fn current_is_supported_media_with_field(
    expected: &DeliveryTargetSnapshot,
    field: Option<&OwnedCf>,
) -> bool {
    if complete_identity(expected).is_none() {
        return false;
    }
    field.is_some_and(|field| positively_identified_media_control(field.as_element()))
}

fn positively_identified_media_control(element: AXUIElementRef) -> bool {
    let Ok(role) = copy_string_attribute_status(element, "AXRole") else {
        return false;
    };
    let role = role.trim();
    if media_semantics_match(role, "") {
        return true;
    }
    if !matches!(role, "AXButton" | "AXSlider") {
        return false;
    }
    for attribute in ["AXIdentifier", "AXDescription", "AXHelp"] {
        let Ok(value) = copy_string_attribute_status(element, attribute) else {
            continue;
        };
        if value.len() > 128 {
            continue;
        }
        let normalized = normalize_marker(&value);
        if media_semantics_match(role, &normalized) {
            return true;
        }
    }
    false
}

fn media_semantics_match(role: &str, normalized_marker: &str) -> bool {
    match role {
        "AXVideo" => true,
        "AXButton" => matches!(
            normalized_marker,
            "play"
                | "pause"
                | "play button"
                | "pause button"
                | "play video"
                | "pause video"
                | "media play"
                | "media pause"
        ),
        "AXSlider" => {
            normalized_marker.contains("playback")
                || normalized_marker.contains("seek")
                || normalized_marker == "media scrubber"
                || normalized_marker == "video progress"
        }
        _ => false,
    }
}

fn normalize_marker(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn enable_manual_accessibility_if_needed(
    state: &mut State,
    identity: &DeliveryTargetSnapshot,
    app: &OwnedCf,
) -> bool {
    if !complete_identity(identity).is_some_and(|identity| known_t3_bundle_id(&identity.bundle_id))
    {
        return false;
    }
    if state
        .manual_override
        .as_ref()
        .is_some_and(|owned| same_process(&owned.identity, identity))
    {
        return true;
    }
    if state
        .manual_attempted
        .as_ref()
        .is_some_and(|(attempted, at)| {
            same_process(attempted, identity) && !manual_retry_due(at.elapsed())
        })
    {
        return false;
    }
    // Backoff applies to calls that reached the native API, even if they ran
    // past the sampling deadline; a call skipped for an exhausted budget says
    // nothing about the app and is retried on the next sample.
    let record_failure = |state: &mut State, attempt: ManualAccessibilityAttempt| {
        if let Some(at) = manual_failure_to_record(attempt, Instant::now()) {
            state.manual_attempted = Some((identity.clone(), at));
        }
    };
    let (current, read_dispatched) =
        copy_bool_attribute_dispatch(app.as_element(), "AXManualAccessibility");
    match current {
        Some(true) => true,
        Some(false) => {
            let Some(launch_token_bits) = running_process_launch_bits(identity) else {
                return false;
            };
            match set_attribute_dispatch(app.as_element(), "AXManualAccessibility", unsafe {
                kCFBooleanTrue
            }) {
                Some(AX_SUCCESS) => {
                    state.manual_override = Some(ManualOverride {
                        identity: identity.clone(),
                        launch_token_bits,
                        app: app.clone(),
                    });
                    true
                }
                Some(_) => {
                    record_failure(state, ManualAccessibilityAttempt::DispatchedFailure);
                    false
                }
                None => {
                    record_failure(state, ManualAccessibilityAttempt::NotDispatched);
                    false
                }
            }
        }
        None => {
            record_failure(
                state,
                if read_dispatched {
                    ManualAccessibilityAttempt::DispatchedFailure
                } else {
                    ManualAccessibilityAttempt::NotDispatched
                },
            );
            false
        }
    }
}

fn manual_accessibility_applies(
    state: &State,
    identity: &DeliveryTargetSnapshot,
    app: &OwnedCf,
) -> bool {
    complete_identity(identity).is_some_and(|identity| known_t3_bundle_id(&identity.bundle_id))
        && (state
            .manual_override
            .as_ref()
            .is_some_and(|owned| same_process(&owned.identity, identity))
            || copy_bool_attribute(app.as_element(), "AXManualAccessibility") == Some(true))
}

fn known_t3_bundle_id(bundle_id: &str) -> bool {
    bundle_id == T3_BUNDLE_ID
}

fn retained_document_ancestor(field: &OwnedCf) -> Option<OwnedCf> {
    let mut current = field.clone();
    for _ in 0..MAX_ANCESTOR_STEPS {
        let parent = copy_element_attribute(current.as_element(), "AXParent").ok()?;
        let role = copy_string_attribute_status(parent.as_element(), "AXRole").ok()?;
        if role.trim() == "AXWebArea" {
            return Some(parent);
        }
        current = parent;
    }
    None
}

#[derive(Clone, Copy)]
enum ObserverEvent {
    HardInvalidate,
    SelectionChanged,
    RevalidateFocus,
}

struct ObserverSignals {
    registration_epoch: u64,
    hard_epoch: AtomicU64,
    selection_epoch: AtomicU64,
    focus_epoch: AtomicU64,
}

struct ObserverContext {
    registration_epoch: u64,
    event: ObserverEvent,
    signals: Arc<ObserverSignals>,
}

unsafe extern "C" fn observer_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    _notification: CFStringRef,
    refcon: *mut c_void,
) {
    let Some(context) = (unsafe { (refcon as *const ObserverContext).as_ref() }) else {
        return;
    };
    debug_assert_eq!(
        context.registration_epoch,
        context.signals.registration_epoch
    );
    match context.event {
        ObserverEvent::HardInvalidate => {
            context.signals.hard_epoch.fetch_add(1, Ordering::AcqRel);
        }
        ObserverEvent::SelectionChanged => {
            context
                .signals
                .selection_epoch
                .fetch_add(1, Ordering::AcqRel);
        }
        ObserverEvent::RevalidateFocus => {
            context.signals.focus_epoch.fetch_add(1, Ordering::AcqRel);
        }
    }
}

fn register_candidate_observer(
    pid: i32,
    app: &OwnedCf,
    field: &OwnedCf,
    registration_epoch: u64,
) -> Option<ObserverRegistration> {
    if pid <= 0 || !ax_budget_open() {
        return None;
    }
    let mut observer = std::ptr::null();
    if unsafe { AXObserverCreate(pid, observer_callback, &mut observer) } != AX_SUCCESS {
        return None;
    }
    let observer = unsafe { OwnedCf::from_create(observer)? };
    let signals = Arc::new(ObserverSignals {
        registration_epoch,
        hard_epoch: AtomicU64::new(0),
        selection_epoch: AtomicU64::new(0),
        focus_epoch: AtomicU64::new(0),
    });
    let mut registration = ObserverRegistration {
        source: unsafe { AXObserverGetRunLoopSource(observer.0) },
        observer,
        app: app.clone(),
        element: field.clone(),
        signals: signals.clone(),
        _contexts: Vec::new(),
    };
    if registration.source.is_null() {
        return None;
    }
    for notification in ["AXValueChanged", "AXUIElementDestroyed"] {
        let notification = cfstring(notification)?;
        let context = Box::new(ObserverContext {
            registration_epoch,
            event: ObserverEvent::HardInvalidate,
            signals: signals.clone(),
        });
        if unsafe {
            AXObserverAddNotification(
                registration.observer.0,
                field.as_element(),
                notification.0,
                (&*context as *const ObserverContext).cast_mut().cast(),
            )
        } != AX_SUCCESS
        {
            return None;
        }
        registration._contexts.push(context);
    }
    let selection_notification = cfstring("AXSelectedTextChanged")?;
    let selection_context = Box::new(ObserverContext {
        registration_epoch,
        event: ObserverEvent::SelectionChanged,
        signals: signals.clone(),
    });
    if unsafe {
        AXObserverAddNotification(
            registration.observer.0,
            field.as_element(),
            selection_notification.0,
            (&*selection_context as *const ObserverContext)
                .cast_mut()
                .cast(),
        )
    } != AX_SUCCESS
    {
        return None;
    }
    registration._contexts.push(selection_context);
    let focus_notification = cfstring("AXFocusedUIElementChanged")?;
    let focus_context = Box::new(ObserverContext {
        registration_epoch,
        event: ObserverEvent::RevalidateFocus,
        signals,
    });
    if unsafe {
        AXObserverAddNotification(
            registration.observer.0,
            app.as_element(),
            focus_notification.0,
            (&*focus_context as *const ObserverContext)
                .cast_mut()
                .cast(),
        )
    } != AX_SUCCESS
    {
        return None;
    }
    registration._contexts.push(focus_context);
    unsafe {
        CFRunLoopAddSource(
            CFRunLoopGetMain(),
            registration.source,
            kCFRunLoopCommonModes,
        );
    }
    Some(registration)
}

fn complete_identity(snapshot: &DeliveryTargetSnapshot) -> Option<&DeliveryTargetIdentity> {
    match snapshot {
        DeliveryTargetSnapshot::Complete(identity) => Some(identity),
        DeliveryTargetSnapshot::SelfTarget
        | DeliveryTargetSnapshot::Incomplete
        | DeliveryTargetSnapshot::Mismatch(_) => None,
    }
}

fn same_process(left: &DeliveryTargetSnapshot, right: &DeliveryTargetSnapshot) -> bool {
    match (complete_identity(left), complete_identity(right)) {
        (Some(left), Some(right)) => {
            left.bundle_id == right.bundle_id
                && left.process_id == right.process_id
                && left.process_instance == right.process_instance
        }
        _ => false,
    }
}

fn running_process_launch_bits(expected: &DeliveryTargetSnapshot) -> Option<u64> {
    let expected = complete_identity(expected)?;
    let application =
        NSRunningApplication::runningApplicationWithProcessIdentifier(expected.process_id)?;
    if application.processIdentifier() != expected.process_id
        || application
            .bundleIdentifier()
            .map(|bundle| bundle.to_string())
            .as_deref()
            != Some(expected.bundle_id.as_str())
    {
        return None;
    }
    let launch_date = application.launchDate()?;
    let interval = launch_date.timeIntervalSinceReferenceDate();
    if !interval.is_finite() || interval <= 0.0 {
        return None;
    }
    Some(interval.to_bits())
}

fn running_application(
    expected: &DeliveryTargetSnapshot,
    launch_token_bits: u64,
) -> Option<objc2::rc::Retained<NSRunningApplication>> {
    let expected = complete_identity(expected)?;
    let application =
        NSRunningApplication::runningApplicationWithProcessIdentifier(expected.process_id)?;
    if application.processIdentifier() != expected.process_id
        || application
            .bundleIdentifier()
            .map(|bundle| bundle.to_string())
            .as_deref()
            != Some(expected.bundle_id.as_str())
        || application
            .launchDate()
            .map(|date| date.timeIntervalSinceReferenceDate().to_bits())
            != Some(launch_token_bits)
    {
        return None;
    }
    Some(application)
}

fn process_is_running(expected: &DeliveryTargetSnapshot, launch_token_bits: u64) -> bool {
    running_application(expected, launch_token_bits).is_some()
}

fn create_application(pid: i32) -> Option<OwnedCf> {
    if pid <= 0 || !ax_budget_open() {
        return None;
    }
    let app = unsafe { OwnedCf::from_create(AXUIElementCreateApplication(pid))? };
    (unsafe { AXUIElementSetMessagingTimeout(app.as_element(), AX_QUERY_TIMEOUT_SECONDS) }
        == AX_SUCCESS)
        .then_some(app)
}

fn cfstring(value: &str) -> Option<OwnedCf> {
    let value = CString::new(value).ok()?;
    unsafe {
        OwnedCf::from_create(CFStringCreateWithCString(
            std::ptr::null(),
            value.as_ptr(),
            UTF8_ENCODING,
        ))
    }
}

fn copy_attribute_raw(element: AXUIElementRef, name: &str) -> Result<OwnedCf, AXError> {
    copy_attribute_dispatch(element, name).0
}

/// Also reports whether the native call was dispatched (`false`: skipped
/// before the call because the budget was exhausted or setup failed).
fn copy_attribute_dispatch(
    element: AXUIElementRef,
    name: &str,
) -> (Result<OwnedCf, AXError>, bool) {
    if !ax_budget_open() {
        return (Err(AX_ERROR_FAILURE), false);
    }
    let Some(attribute) = cfstring(name) else {
        return (Err(AX_ERROR_FAILURE), false);
    };
    let mut value = std::ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element, attribute.0, &mut value) };
    if status != AX_SUCCESS || value.is_null() {
        if !value.is_null() {
            unsafe {
                CFRelease(value);
            }
        }
        return (
            Err(if status == AX_SUCCESS {
                AX_ERROR_NO_VALUE
            } else {
                status
            }),
            true,
        );
    }
    (
        unsafe { OwnedCf::from_create(value).ok_or(AX_ERROR_FAILURE) },
        true,
    )
}

fn copy_attribute(element: AXUIElementRef, name: &str) -> Result<OwnedCf, AXError> {
    copy_attribute_raw(element, name)
}

fn copy_element_attribute(element: AXUIElementRef, name: &str) -> Result<OwnedCf, AXError> {
    let value = copy_attribute_raw(element, name)?;
    if unsafe { AXUIElementSetMessagingTimeout(value.as_element(), AX_QUERY_TIMEOUT_SECONDS) }
        != AX_SUCCESS
    {
        return Err(AX_ERROR_FAILURE);
    }
    Ok(value)
}

fn copy_string_attribute_status(element: AXUIElementRef, name: &str) -> Result<String, AXError> {
    let value = copy_attribute_raw(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFStringGetTypeID() } {
        return Err(AX_ERROR_FAILURE);
    }
    let length = unsafe { CFStringGetLength(value.0) };
    let maximum = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
    if !(0..=4096).contains(&maximum) {
        return Err(AX_ERROR_FAILURE);
    }
    let mut buffer = vec![0 as c_char; maximum as usize + 1];
    if !unsafe {
        CFStringGetCString(
            value.0,
            buffer.as_mut_ptr(),
            buffer.len() as CFIndex,
            UTF8_ENCODING,
        )
    } {
        return Err(AX_ERROR_FAILURE);
    }
    Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

fn copy_bool_attribute(element: AXUIElementRef, name: &str) -> Option<bool> {
    copy_bool_attribute_dispatch(element, name).0
}

fn copy_bool_attribute_dispatch(element: AXUIElementRef, name: &str) -> (Option<bool>, bool) {
    let (value, dispatched) = copy_attribute_dispatch(element, name);
    let Ok(value) = value else {
        return (None, dispatched);
    };
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFBooleanGetTypeID() } {
        return (None, dispatched);
    }
    (Some(unsafe { CFBooleanGetValue(value.0) != 0 }), dispatched)
}

fn copy_number_attribute(element: AXUIElementRef, name: &str) -> Option<usize> {
    let value = copy_attribute_raw(element, name).ok()?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFNumberGetTypeID() } {
        return None;
    }
    let mut number = 0_i64;
    // kCFNumberSInt64Type
    if !unsafe { CFNumberGetValue(value.0, 4, &mut number as *mut i64 as *mut c_void) }
        || number < 0
    {
        return None;
    }
    usize::try_from(number).ok()
}

fn attribute_settable(element: AXUIElementRef, name: &str) -> bool {
    if !ax_budget_open() {
        return false;
    }
    let Some(attribute) = cfstring(name) else {
        return false;
    };
    let mut settable = 0;
    (unsafe { AXUIElementIsAttributeSettable(element, attribute.0, &mut settable) }) == AX_SUCCESS
        && settable != 0
}

fn set_attribute(element: AXUIElementRef, name: &str, value: CFTypeRef) -> AXError {
    set_attribute_dispatch(element, name, value).unwrap_or(AX_ERROR_FAILURE)
}

/// `None` when the call was skipped before reaching the native API.
fn set_attribute_dispatch(
    element: AXUIElementRef,
    name: &str,
    value: CFTypeRef,
) -> Option<AXError> {
    if !ax_budget_open() {
        return None;
    }
    let attribute = cfstring(name)?;
    Some(unsafe { AXUIElementSetAttributeValue(element, attribute.0, value) })
}

#[repr(C)]
struct CFRange {
    location: CFIndex,
    length: CFIndex,
}

fn decode_range(value: CFTypeRef) -> Option<(usize, usize)> {
    if unsafe { AXValueGetType(value) } != AX_VALUE_CFRANGE_TYPE {
        return None;
    }
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    let decoded = unsafe {
        AXValueGetValue(
            value,
            AX_VALUE_CFRANGE_TYPE,
            &mut range as *mut CFRange as *mut c_void,
        )
    };
    if !decoded || range.location < 0 || range.length < 0 {
        return None;
    }
    Some((range.location as usize, range.length as usize))
}

fn benign_absence(status: AXError) -> bool {
    matches!(status, AX_ERROR_NO_VALUE | AX_ERROR_ATTRIBUTE_UNSUPPORTED)
}

fn with_ax_budget<T>(budget: Duration, operation: impl FnOnce() -> T) -> T {
    AX_DEADLINE.with(|deadline| {
        let previous = deadline.replace(Some(Instant::now() + budget));
        let result = operation();
        deadline.set(previous);
        result
    })
}

fn ax_budget_open() -> bool {
    AX_DEADLINE.with(|deadline| {
        deadline
            .get()
            .is_none_or(|deadline| Instant::now() < deadline)
    })
}

fn sleep_within_budget(duration: Duration) -> bool {
    let allowed = AX_DEADLINE.with(|deadline| {
        deadline
            .get()
            .is_none_or(|deadline| Instant::now() + duration <= deadline)
    });
    if !allowed {
        return false;
    }
    thread::sleep(duration);
    ax_budget_open()
}

fn is_main_thread() -> bool {
    unsafe { libc::pthread_main_np() != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_role_wins_even_when_the_subrole_query_fails() {
        assert!(matches!(
            classify_text_role(Ok("AXSecureTextField".into()), || Err(AX_ERROR_FAILURE)),
            Err(EditableStatus::Secure)
        ));
        assert!(matches!(
            classify_text_role(Ok("AXTextField".into()), || Ok("AXSecureTextField".into())),
            Err(EditableStatus::Secure)
        ));
        assert!(matches!(
            classify_text_role(Ok("AXTextField".into()), || Err(AX_ERROR_FAILURE)),
            Err(EditableStatus::Unavailable)
        ));
        assert!(matches!(
            classify_text_role(Ok("AXTextArea".into()), || Err(AX_ERROR_NO_VALUE)),
            Ok(())
        ));
        assert!(matches!(
            classify_text_role(Ok("AXWebArea".into()), || Err(AX_ERROR_NO_VALUE)),
            Err(EditableStatus::NotEditable)
        ));
    }

    #[test]
    fn media_marker_normalization_is_bounded_to_explicit_markers() {
        assert_eq!(normalize_marker(" Pause-video "), "pause video");
        assert_eq!(normalize_marker("playback_position"), "playback position");
        assert_ne!(normalize_marker("Submit"), "play");
        assert_ne!(normalize_marker("Volume"), "video progress");
    }

    #[test]
    fn media_semantics_reject_account_form_controls_and_generic_actions() {
        assert!(media_semantics_match("AXVideo", ""));
        assert!(media_semantics_match("AXButton", "pause video"));
        assert!(media_semantics_match("AXSlider", "video progress"));
        assert!(!media_semantics_match("AXTextField", "play"));
        assert!(!media_semantics_match("AXButton", "submit"));
        assert!(!media_semantics_match("AXSlider", "account volume"));
    }

    #[test]
    fn manual_accessibility_is_restricted_to_the_confirmed_t3_bundle() {
        assert!(known_t3_bundle_id("com.t3tools.t3code"));
        assert!(!known_t3_bundle_id("com.t3tools.t3code.dev.local"));
        assert!(!known_t3_bundle_id("com.example.t3code"));
    }

    #[test]
    fn a_background_blur_is_preserved_but_a_different_field_revokes() {
        assert!(!focus_transition_invalidates(
            CandidateFocusTransition::ExactField
        ));
        assert!(!focus_transition_invalidates(
            CandidateFocusTransition::BackgroundBlur
        ));
        assert!(focus_transition_invalidates(
            CandidateFocusTransition::DifferentField
        ));
        assert!(focus_transition_invalidates(
            CandidateFocusTransition::Unavailable
        ));
    }

    #[test]
    fn observer_epochs_count_repeated_events_and_are_registration_local() {
        let first = ObserverSignals {
            registration_epoch: 7,
            hard_epoch: AtomicU64::new(0),
            selection_epoch: AtomicU64::new(0),
            focus_epoch: AtomicU64::new(0),
        };
        let second = ObserverSignals {
            registration_epoch: 8,
            hard_epoch: AtomicU64::new(0),
            selection_epoch: AtomicU64::new(0),
            focus_epoch: AtomicU64::new(0),
        };
        first.hard_epoch.fetch_add(1, Ordering::AcqRel);
        first.hard_epoch.fetch_add(1, Ordering::AcqRel);
        assert_eq!(first.hard_epoch.load(Ordering::Acquire), 2);
        assert_eq!(second.hard_epoch.load(Ordering::Acquire), 0);
    }

    #[test]
    fn mutation_confirmation_rejects_success_without_structural_advancement() {
        assert!(!delivery_is_confirmed(
            (12, 0),
            5,
            Some((12, 0)),
            Some(20),
            Some(20),
        ));
        assert!(delivery_is_confirmed(
            (12, 0),
            5,
            Some((17, 0)),
            Some(20),
            Some(25),
        ));
        assert!(!delivery_is_confirmed(
            (12, 0),
            5,
            Some((17, 0)),
            None,
            None,
        ));
    }

    #[test]
    fn replacement_confirmation_accounts_for_replaced_range() {
        assert!(delivery_is_confirmed(
            (7, 4),
            2,
            Some((9, 0)),
            Some(20),
            Some(18),
        ));
        assert!(!delivery_is_confirmed(
            (7, 4),
            2,
            Some((9, 0)),
            Some(20),
            Some(20),
        ));
    }
}
