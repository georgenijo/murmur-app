//! Native, content-blind tracking for smart dictation destinations.
//!
//! The sampler remembers only a positively focused editable AX element. It
//! never reads the element's value or selected text, never moves focus, and
//! never posts keyboard events. A captured destination is an opaque one-shot
//! capability: app/process/window/document/field/range identity is rechecked
//! at preparation and immediately before the single native Cmd+V post.

use crate::frontmost::DeliveryTargetSnapshot;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(any(target_os = "macos", test))]
const RECENT_TARGET_TTL: Duration = Duration::from_secs(120);

static ENABLED: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);
static SAMPLE_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureGate {
    Proceed,
    Disabled,
    Unavailable,
}

fn capture_gate(accepted_enabled: bool, currently_enabled: bool) -> CaptureGate {
    match (accepted_enabled, currently_enabled) {
        (false, _) => CaptureGate::Disabled,
        (true, false) => CaptureGate::Unavailable,
        (true, true) => CaptureGate::Proceed,
    }
}

/// Opaque smart-destination capability returned to one recording owner.
///
/// Deliberately has no `Debug` implementation: even though its native identity
/// is content-free, it is private routing state and must not drift into logs.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) enum Snapshot {
    #[default]
    Disabled,
    /// Fail closed: delivery stays clipboard-only (a secure field, supported
    /// media without a retained field, revocation during capture, or a
    /// media-handoff destination that could not be prepared).
    Unavailable,
    /// No field was positively proven at acceptance, but nothing positively
    /// ruled delivery out. Delivery uses the normal verified auto-paste
    /// policy, exactly as it would with Smart destination off.
    Unverified,
    Target {
        token: u64,
        revision: u64,
        identity: DeliveryTargetSnapshot,
        /// The field was in the app that was frontmost at recording start, so
        /// delivery needs no cross-app activation.
        current_app: bool,
    },
}

impl Snapshot {
    pub(crate) fn identity(&self) -> Option<&DeliveryTargetSnapshot> {
        match self {
            Self::Target { identity, .. } => Some(identity),
            Self::Disabled | Self::Unavailable | Self::Unverified => None,
        }
    }

    /// Whether delivery follows the normal auto-paste policy instead of the
    /// smart one-shot field capability.
    pub(crate) fn uses_normal_paste(&self) -> bool {
        matches!(self, Self::Disabled | Self::Unverified)
    }

    /// A current-app capture whose field could not be re-verified may use the
    /// normal verified paste into that same app, exactly as with the feature
    /// off. A cross-app (media handoff) capture never may: normal paste would
    /// target the media app instead.
    pub(crate) fn allows_same_app_fallback(&self) -> bool {
        matches!(
            self,
            Self::Target {
                current_app: true,
                ..
            }
        )
    }

    /// Content-free routing label for diagnostics.
    pub(crate) fn route_code(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Unavailable => "clipboard_only",
            Self::Unverified => "normal_paste",
            Self::Target { .. } => "smart_target",
        }
    }
}

/// Content-free result of the only native delivery this module performs.
///
/// `Posted` means the one allowed native key post occurred; like the existing
/// `AutoPasted` result it does not claim that target content was read or
/// confirmed. The token is retired before the post, so neither this module nor
/// its caller may automatically retry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertOutcome {
    Posted,
    Refused,
}

/// Result of the one allowed main-thread preparation attempt.
///
/// `WaitForActivation` means native activation was requested exactly once. The
/// caller must leave the main thread free, wait for activation to settle, then
/// re-check recording ownership before dispatching [`insert`] on the main
/// thread. `Ready` needs no activation wait. Every other result is terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrepareOutcome {
    Ready,
    WaitForActivation,
    Refused,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilityStage {
    Captured,
    Prepared,
}

#[cfg(any(target_os = "macos", test))]
fn claim_preparation(stage: &mut CapabilityStage) -> bool {
    if *stage != CapabilityStage::Captured {
        return false;
    }
    *stage = CapabilityStage::Prepared;
    true
}

/// Enable or disable tracking. Every transition advances the generation.
/// Disabling therefore revokes every outstanding token synchronously even
/// before the main-thread state gets its next opportunity to release AX refs.
pub(crate) fn set_enabled(enabled: bool) {
    let previous = ENABLED.swap(enabled, Ordering::AcqRel);
    if previous != enabled || !enabled {
        let generation = GENERATION.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        if !enabled {
            native::disable_if_main_thread(generation);
        }
    }
}

/// Start the single bounded sampler. Repeated calls are harmless.
pub(crate) fn start(app: tauri::AppHandle) {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        let mut last_dispatched_generation = 0;
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let generation = GENERATION.load(Ordering::Acquire);
            let enabled = ENABLED.load(Ordering::Acquire);
            if (!enabled && generation == last_dispatched_generation)
                || SAMPLE_PENDING
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                continue;
            }

            let dispatched = app.run_on_main_thread(move || {
                native::sample(generation);
                SAMPLE_PENDING.store(false, Ordering::Release);
            });
            if dispatched.is_err() {
                SAMPLE_PENDING.store(false, Ordering::Release);
            } else {
                last_dispatched_generation = generation;
            }
        }
    });
}

/// Freeze the exact current input, or the recent input while the expected
/// foreground app is positively identified as media.
pub async fn capture(
    app: &tauri::AppHandle,
    expected: &DeliveryTargetSnapshot,
    enabled: bool,
) -> Snapshot {
    match capture_gate(enabled, ENABLED.load(Ordering::Acquire)) {
        CaptureGate::Proceed => {}
        CaptureGate::Disabled => return Snapshot::Disabled,
        CaptureGate::Unavailable => return Snapshot::Unavailable,
    }
    if let Some(snapshot) = start_without_external_app(expected) {
        return snapshot;
    }

    let generation = GENERATION.load(Ordering::Acquire);
    let expected = expected.clone();
    // Native capture sets this the moment it has positive evidence against
    // falling back (a secure field or the media route),
    // before any cleanup that could outlive the timeout.
    let fallback_vetoed = std::sync::Arc::new(AtomicBool::new(false));
    let native_veto = fallback_vetoed.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if app
        .run_on_main_thread(move || {
            let snapshot = native::capture(&expected, generation, &native_veto);
            let _ = sender.send(snapshot);
        })
        .is_err()
    {
        return unproven_capture(generation, &fallback_vetoed);
    }

    match tokio::time::timeout(CAPTURE_TIMEOUT, receiver).await {
        Ok(Ok(Snapshot::Unverified)) | Ok(Err(_)) | Err(_) => {
            unproven_capture(generation, &fallback_vetoed)
        }
        Ok(Ok(snapshot)) => snapshot,
    }
}

/// No complete external app at start (a lookup gap, Murmur itself in front
/// after a relaunch, or a mismatch): normal delivery applies its own start and
/// stop anchor checks, exactly as with the feature off.
fn start_without_external_app(expected: &DeliveryTargetSnapshot) -> Option<Snapshot> {
    match expected {
        DeliveryTargetSnapshot::Complete(_) => None,
        DeliveryTargetSnapshot::Incomplete
        | DeliveryTargetSnapshot::SelfTarget
        | DeliveryTargetSnapshot::Mismatch(_) => Some(Snapshot::Unverified),
    }
}

fn unproven_capture(captured_generation: u64, fallback_vetoed: &AtomicBool) -> Snapshot {
    unproven_capture_policy(
        fallback_vetoed.load(Ordering::Acquire),
        ENABLED.load(Ordering::Acquire),
        GENERATION.load(Ordering::Acquire),
        captured_generation,
    )
}

/// A capture that proved nothing, or could not finish, must not make delivery
/// stricter than normal auto-paste, unless native capture already found
/// refusal evidence or Smart destination was revoked while it was pending.
fn unproven_capture_policy(
    fallback_vetoed: bool,
    enabled_now: bool,
    generation_now: u64,
    captured_generation: u64,
) -> Snapshot {
    if !fallback_vetoed && enabled_now && generation_now == captured_generation {
        Snapshot::Unverified
    } else {
        Snapshot::Unavailable
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityRelation {
    Same,
    /// A transient lookup gap: nothing contradicts the expected process.
    Unknown,
    /// Self, mismatched, or a different complete process: positive evidence.
    Different,
}

#[cfg(any(target_os = "macos", test))]
fn identity_relation(
    current: &DeliveryTargetSnapshot,
    expected: &DeliveryTargetSnapshot,
) -> IdentityRelation {
    match (current, expected) {
        (DeliveryTargetSnapshot::Complete(current), DeliveryTargetSnapshot::Complete(expected))
            if current.bundle_id == expected.bundle_id
                && current.process_id == expected.process_id
                && current.process_instance == expected.process_instance =>
        {
            IdentityRelation::Same
        }
        (DeliveryTargetSnapshot::Incomplete, _) => IdentityRelation::Unknown,
        _ => IdentityRelation::Different,
    }
}

/// Validate and prepare one smart-destination token on the native main thread.
///
/// This is the only stage that checks the original foreground/media target.
/// It performs no clipboard write or key post and never waits for activation.
pub(crate) fn prepare(snapshot: &Snapshot) -> PrepareOutcome {
    native::prepare(snapshot, true)
}

/// Explicit retries require a currently focused field and never activate a
/// remembered background destination.
pub(crate) fn prepare_focused(snapshot: &Snapshot) -> PrepareOutcome {
    native::prepare(snapshot, false)
}

/// Retire a prepared token without posting and, when native proof shows the
/// activated destination is still untouched, return focus to the media app.
/// Passing `None` only retires the token, without changing focus. This is
/// intentionally synchronous and main-thread only.
pub(crate) fn abort(snapshot: &Snapshot, pasteboard_change: Option<i64>) {
    native::abort(snapshot, pasteboard_change);
}

/// Perform at most one native Cmd+V post for a prepared token. The pasteboard
/// change count is rechecked inside the exact-field transport immediately
/// before the key post. This is intentionally synchronous and may only be
/// called from the native main thread.
pub(crate) fn insert(snapshot: &Snapshot, text: &str, pasteboard_change: i64) -> InsertOutcome {
    native::insert(snapshot, text, pasteboard_change)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryRoute {
    CurrentEditable,
    RecentMedia,
}

#[cfg(any(target_os = "macos", test))]
fn selection_allowed(route: DeliveryRoute, selection_length: usize) -> bool {
    route == DeliveryRoute::CurrentEditable || selection_length == 0
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateNotification {
    ValueOrDestroyed,
    Selection,
    Focus,
}

#[cfg(any(target_os = "macos", test))]
fn notification_invalidates(
    notification: CandidateNotification,
    retained_identity_proved: bool,
) -> bool {
    match notification {
        CandidateNotification::ValueOrDestroyed => true,
        CandidateNotification::Selection => true,
        CandidateNotification::Focus => !retained_identity_proved,
    }
}

#[cfg(any(target_os = "macos", test))]
fn delivery_is_confirmed(
    before_range: (usize, usize),
    inserted_length: usize,
    after_range: Option<(usize, usize)>,
    count_before: Option<usize>,
    count_after: Option<usize>,
) -> bool {
    let Some(expected_caret) = before_range.0.checked_add(inserted_length) else {
        return false;
    };
    if after_range != Some((expected_caret, 0)) {
        return false;
    }

    let (Some(before), Some(after)) = (count_before, count_after) else {
        return false;
    };
    before
        .checked_sub(before_range.1)
        .and_then(|without_selection| without_selection.checked_add(inserted_length))
        == Some(after)
}

#[cfg(any(target_os = "macos", test))]
fn t3_empty_composer_delivery_is_confirmed(
    before_range: (usize, usize),
    inserted_length: usize,
    after_range: Option<(usize, usize)>,
    count_before: Option<usize>,
    count_after: Option<usize>,
) -> bool {
    let Some(expected_count) = 1_usize.checked_add(inserted_length) else {
        return false;
    };
    before_range == (0, 0)
        && count_before == Some(1)
        && count_after == Some(expected_count)
        && after_range == Some((expected_count, 0))
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observation {
    Editable,
    SupportedMedia,
    SelfOverlay,
    Secure,
    Other,
    Unavailable,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateAction {
    Replace,
    Preserve,
    Clear,
}

#[cfg(any(target_os = "macos", test))]
/// Only a positively focused editable field replaces the retained one, and
/// only a positively secure field forgets it. Everything else (a media control,
/// Murmur's overlay, ordinary page focus, or a failed/timed-out lookup) is not
/// evidence that the user moved to another text input, so one such sample must
/// not discard the destination. The retained field is still bounded by its
/// recency window, by its own observer invalidation, and by exact-field
/// validation at capture and delivery.
fn candidate_action(observation: Observation, has_recent_candidate: bool) -> CandidateAction {
    match observation {
        Observation::Editable => CandidateAction::Replace,
        Observation::SupportedMedia
        | Observation::SelfOverlay
        | Observation::Other
        | Observation::Unavailable
            if has_recent_candidate =>
        {
            CandidateAction::Preserve
        }
        Observation::SupportedMedia
        | Observation::SelfOverlay
        | Observation::Other
        | Observation::Unavailable
        | Observation::Secure => CandidateAction::Clear,
    }
}

/// Whether the media app's current focus still permits a media handoff that
/// was decided at recording start. Only a different editable field or a secure
/// field the user genuinely moved to revokes it; the media control no longer
/// needing to hold focus (controls auto-hide, pages re-focus) does not.
#[cfg(any(target_os = "macos", test))]
fn handoff_focus_allowed(observation: Observation) -> bool {
    !matches!(observation, Observation::Editable | Observation::Secure)
}

/// How long after a failed `AXManualAccessibility` attempt the same process is
/// retried. A permanent latch would leave a T3 window without an exposed
/// accessibility tree untracked until Smart destination is toggled.
#[cfg(any(target_os = "macos", test))]
const MANUAL_ACCESSIBILITY_RETRY: Duration = Duration::from_secs(5);

#[cfg(any(target_os = "macos", test))]
fn manual_retry_due(since_attempt: Duration) -> bool {
    since_attempt >= MANUAL_ACCESSIBILITY_RETRY
}

#[cfg(any(target_os = "macos", test))]
fn generation_is_current(token_generation: u64) -> bool {
    ENABLED.load(Ordering::Acquire) && generation_is_latest(token_generation)
}

#[cfg(any(target_os = "macos", test))]
fn generation_is_latest(token_generation: u64) -> bool {
    token_generation == GENERATION.load(Ordering::Acquire)
}

#[cfg(any(target_os = "macos", test))]
fn is_recent(age: Duration) -> bool {
    age <= RECENT_TARGET_TTL
}

#[cfg(any(target_os = "macos", test))]
fn candidate_is_preservable(valid: bool, age: Duration, has_frozen: bool) -> bool {
    valid && (has_frozen || is_recent(age))
}

#[cfg(any(target_os = "macos", test))]
fn candidate_is_capturable(valid: bool, age: Duration) -> bool {
    valid && is_recent(age)
}

#[cfg(target_os = "macos")]
#[path = "smart_destination/macos.rs"]
mod native;

#[cfg(not(target_os = "macos"))]
mod native {
    use super::{DeliveryTargetSnapshot, InsertOutcome, PrepareOutcome, Snapshot};

    pub(super) fn sample(_generation: u64) {}

    pub(super) fn disable_if_main_thread(_generation: u64) {}

    pub(super) fn capture(
        _expected: &DeliveryTargetSnapshot,
        _generation: u64,
        _fallback_vetoed: &std::sync::atomic::AtomicBool,
    ) -> Snapshot {
        Snapshot::Unavailable
    }

    pub(super) fn prepare(_snapshot: &Snapshot, _allow_activation: bool) -> PrepareOutcome {
        PrepareOutcome::Refused
    }

    pub(super) fn abort(_snapshot: &Snapshot, _pasteboard_change: Option<i64>) {}

    pub(super) fn insert(
        _snapshot: &Snapshot,
        _text: &str,
        _pasteboard_change: i64,
    ) -> InsertOutcome {
        InsertOutcome::Refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_defaults_disabled_and_hides_missing_identity() {
        let snapshot = Snapshot::default();
        assert!(matches!(snapshot, Snapshot::Disabled));
        assert!(snapshot.identity().is_none());
        assert!(Snapshot::Unavailable.identity().is_none());
        assert!(Snapshot::Unverified.identity().is_none());
    }

    #[test]
    fn unproven_capture_falls_back_only_without_veto_while_enabled_and_current() {
        assert!(matches!(
            unproven_capture_policy(false, true, 7, 7),
            Snapshot::Unverified
        ));
        // Secure-field or media-route evidence found before a timeout.
        assert!(matches!(
            unproven_capture_policy(true, true, 7, 7),
            Snapshot::Unavailable
        ));
        // Revoked while pending, including a capture that completed Unverified.
        assert!(matches!(
            unproven_capture_policy(false, false, 7, 7),
            Snapshot::Unavailable
        ));
        assert!(matches!(
            unproven_capture_policy(false, true, 8, 7),
            Snapshot::Unavailable
        ));
    }

    #[test]
    fn only_an_incomplete_sample_is_an_unknown_identity() {
        use crate::frontmost::{
            AppTransitionSnapshot, DeliveryTargetIdentity, ProcessInstanceToken,
        };
        let target = |pid: i32, instance: u64| {
            DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
                bundle_id: "com.example.Editor".into(),
                process_id: pid,
                process_instance: ProcessInstanceToken::for_test(instance),
                window_token: None,
                transitions: AppTransitionSnapshot {
                    activation_generation: 0,
                    space_generation: 0,
                },
            })
        };
        let expected = target(42, 1);
        assert_eq!(
            identity_relation(&target(42, 1), &expected),
            IdentityRelation::Same
        );
        assert_eq!(
            identity_relation(&DeliveryTargetSnapshot::Incomplete, &expected),
            IdentityRelation::Unknown
        );
        assert_eq!(
            identity_relation(&target(43, 1), &expected),
            IdentityRelation::Different
        );
        assert_eq!(
            identity_relation(&target(42, 2), &expected),
            IdentityRelation::Different
        );
        assert_eq!(
            identity_relation(&DeliveryTargetSnapshot::SelfTarget, &expected),
            IdentityRelation::Different
        );
    }

    #[test]
    fn only_unproven_and_disabled_snapshots_use_normal_paste() {
        assert!(Snapshot::Disabled.uses_normal_paste());
        assert!(Snapshot::Unverified.uses_normal_paste());
        assert!(!Snapshot::Unavailable.uses_normal_paste());
        assert!(!Snapshot::Target {
            token: 1,
            revision: 1,
            identity: DeliveryTargetSnapshot::Incomplete,
            current_app: true,
        }
        .uses_normal_paste());
    }

    #[test]
    fn starting_without_an_external_app_uses_normal_paste() {
        // Murmur frontmost right after an update relaunch must not make the
        // recording clipboard-only.
        for start in [
            DeliveryTargetSnapshot::SelfTarget,
            DeliveryTargetSnapshot::Incomplete,
        ] {
            assert!(matches!(
                start_without_external_app(&start),
                Some(Snapshot::Unverified)
            ));
        }
    }

    #[test]
    fn only_current_app_captures_may_fall_back_to_normal_paste() {
        let target = |current_app| Snapshot::Target {
            token: 1,
            revision: 1,
            identity: DeliveryTargetSnapshot::Incomplete,
            current_app,
        };
        assert!(target(true).allows_same_app_fallback());
        assert!(!target(false).allows_same_app_fallback());
        assert!(!Snapshot::Unavailable.allows_same_app_fallback());
        assert!(!Snapshot::Unverified.allows_same_app_fallback());
        assert!(!Snapshot::Disabled.allows_same_app_fallback());
    }

    #[test]
    fn capture_revocation_never_downgrades_an_accepted_smart_recording() {
        assert_eq!(capture_gate(false, false), CaptureGate::Disabled);
        assert_eq!(capture_gate(false, true), CaptureGate::Disabled);
        assert_eq!(capture_gate(true, false), CaptureGate::Unavailable);
        assert_eq!(capture_gate(true, true), CaptureGate::Proceed);
    }

    #[test]
    fn policy_replaces_only_with_a_proven_editable_field() {
        assert_eq!(
            candidate_action(Observation::Editable, false),
            CandidateAction::Replace
        );
        assert_eq!(
            candidate_action(Observation::Editable, true),
            CandidateAction::Replace
        );
        // A secure field always forgets the retained destination.
        assert_eq!(
            candidate_action(Observation::Secure, true),
            CandidateAction::Clear
        );
        assert_eq!(
            candidate_action(Observation::Secure, false),
            CandidateAction::Clear
        );
    }

    #[test]
    fn a_single_non_editable_sample_preserves_a_recent_target_without_refreshing_it() {
        // Ordinary page focus or a failed lookup (for example Brave reporting
        // web-area focus after the media click) must not forget the T3 field.
        for observation in [
            Observation::SupportedMedia,
            Observation::SelfOverlay,
            Observation::Other,
            Observation::Unavailable,
        ] {
            assert_eq!(
                candidate_action(observation, true),
                CandidateAction::Preserve
            );
            assert_eq!(candidate_action(observation, false), CandidateAction::Clear);
        }
    }

    #[test]
    fn media_handoff_survives_focus_drift_but_not_a_new_field_or_secure_field() {
        for observation in [
            Observation::SupportedMedia,
            Observation::Other,
            Observation::Unavailable,
            Observation::SelfOverlay,
        ] {
            assert!(handoff_focus_allowed(observation));
        }
        assert!(!handoff_focus_allowed(Observation::Editable));
        assert!(!handoff_focus_allowed(Observation::Secure));
    }

    #[test]
    fn manual_accessibility_is_retried_after_the_backoff_not_latched_forever() {
        assert!(!manual_retry_due(Duration::ZERO));
        assert!(!manual_retry_due(
            MANUAL_ACCESSIBILITY_RETRY - Duration::from_nanos(1)
        ));
        assert!(manual_retry_due(MANUAL_ACCESSIBILITY_RETRY));
    }

    #[test]
    fn recent_target_expiry_is_inclusive_and_bounded() {
        assert!(is_recent(Duration::from_secs(119)));
        assert!(is_recent(Duration::from_secs(120)));
        assert!(!is_recent(
            Duration::from_secs(120) + Duration::from_nanos(1)
        ));
    }

    #[test]
    fn accepted_recording_preserves_frozen_candidate_past_history_expiration() {
        let expired = RECENT_TARGET_TTL + Duration::from_nanos(1);
        assert!(!candidate_is_preservable(true, expired, false));
        assert!(candidate_is_preservable(true, expired, true));
        assert!(!candidate_is_capturable(true, expired));
        assert!(!candidate_is_preservable(false, Duration::ZERO, true));
    }

    #[test]
    fn preparation_is_a_one_shot_state_transition() {
        let mut stage = CapabilityStage::Captured;
        assert!(claim_preparation(&mut stage));
        assert_eq!(stage, CapabilityStage::Prepared);
        assert!(!claim_preparation(&mut stage));
    }

    #[test]
    fn generation_change_revokes_tokens_even_if_reenabled() {
        set_enabled(false);
        set_enabled(true);
        let generation = GENERATION.load(Ordering::Acquire);
        assert!(generation_is_current(generation));

        set_enabled(false);
        assert!(!generation_is_current(generation));
        set_enabled(true);
        assert!(!generation_is_current(generation));
        assert!(!generation_is_latest(generation));

        set_enabled(false);
    }

    #[test]
    fn insert_outcomes_are_distinct_and_content_free() {
        assert_ne!(InsertOutcome::Posted, InsertOutcome::Refused);
        assert_ne!(PrepareOutcome::Ready, PrepareOutcome::WaitForActivation);
        assert_ne!(PrepareOutcome::WaitForActivation, PrepareOutcome::Refused);
    }

    #[test]
    fn recent_media_requires_a_collapsed_selection_but_current_editors_allow_replacement() {
        assert!(selection_allowed(DeliveryRoute::RecentMedia, 0));
        assert!(!selection_allowed(DeliveryRoute::RecentMedia, 1));
        assert!(selection_allowed(DeliveryRoute::CurrentEditable, 0));
        assert!(selection_allowed(DeliveryRoute::CurrentEditable, 12));
    }

    #[test]
    fn observer_policy_never_ignores_value_or_destroy_and_freezes_selection() {
        assert!(notification_invalidates(
            CandidateNotification::ValueOrDestroyed,
            true,
        ));
        assert!(notification_invalidates(
            CandidateNotification::Selection,
            false,
        ));
        assert!(notification_invalidates(
            CandidateNotification::Selection,
            true,
        ));
        assert!(!notification_invalidates(
            CandidateNotification::Focus,
            true,
        ));
        assert!(notification_invalidates(
            CandidateNotification::Focus,
            false,
        ));
    }

    #[test]
    fn delivery_confirmation_requires_both_caret_and_character_count_advancement() {
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
        assert!(!delivery_is_confirmed(
            (7, 4),
            2,
            Some((7, 4)),
            Some(20),
            Some(18),
        ));
        assert!(!delivery_is_confirmed(
            (7, 4),
            2,
            Some((9, 0)),
            None,
            Some(18),
        ));
    }

    #[test]
    fn t3_empty_composer_confirmation_is_narrowly_scoped_to_its_placeholder_shape() {
        assert!(t3_empty_composer_delivery_is_confirmed(
            (0, 0),
            20,
            Some((21, 0)),
            Some(1),
            Some(21),
        ));
        assert!(!t3_empty_composer_delivery_is_confirmed(
            (1, 0),
            20,
            Some((21, 0)),
            Some(1),
            Some(21),
        ));
        assert!(!t3_empty_composer_delivery_is_confirmed(
            (0, 0),
            20,
            Some((20, 0)),
            Some(1),
            Some(21),
        ));
    }
}
