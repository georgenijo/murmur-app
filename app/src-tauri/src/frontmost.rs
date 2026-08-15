//! Frontmost-app detection used by per-app dictation profiles.
//!
//! The primary query uses `NSWorkspace` directly. Transient unavailable/empty
//! results are retried briefly before the existing System Events AppleScript is
//! used once as a bounded compatibility fallback. The first successful sample
//! is returned to the caller and becomes part of its immutable recording
//! context; failures remain global-only and deny app-specific context reads.

#![cfg_attr(feature = "internal-benchmark", allow(dead_code))]

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const MAX_RUNNING_APPLICATIONS: usize = 64;
const MAX_DELIVERY_VERIFICATION_ATTEMPTS: usize = 3;
const DELIVERY_VERIFICATION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(10);

static ACTIVATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static SPACE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Privacy-bounded data exposed to the Settings picker. Process identifiers,
/// paths, launch arguments, window titles, and document state never cross the
/// command boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningApplication {
    pub bundle_id: String,
    pub name: String,
}

/// Content-free identity frozen into a live recording. The process identifier
/// never crosses the Tauri command boundary or enters history/telemetry; it is
/// used only to keep a delayed auto-paste bound to its original application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmostAppIdentity {
    pub bundle_id: Option<String>,
    pub process_id: Option<i32>,
}

/// Opaque app-launch identity. The value is derived from LaunchServices'
/// launch date and is never logged or exposed over the Tauri boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProcessInstanceToken(u64);

/// Opaque Accessibility identity for the focused window. This is diagnostic
/// evidence only: moving between windows in the same process never blocks a
/// paste.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowToken(u64);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppTransitionSnapshot {
    pub activation_generation: u64,
    pub space_generation: u64,
}

/// Complete native identity frozen at the accepted recording transition.
/// Absence of any required field is represented by no identity rather than a
/// partial value that a later paste might accidentally trust.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DeliveryTargetIdentity {
    pub bundle_id: String,
    pub process_id: i32,
    pub process_instance: ProcessInstanceToken,
    pub window_token: Option<WindowToken>,
    pub transitions: AppTransitionSnapshot,
}

/// Result of one native frontmost-application sample. Keeping self and partial
/// identities distinct prevents an unbundled Murmur process from being treated
/// as a transient external lookup failure.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum DeliveryTargetSnapshot {
    Complete(DeliveryTargetIdentity),
    SelfTarget,
    Incomplete,
    Mismatch(DeliveryTargetMismatch),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryTargetMismatchKind {
    DifferentApplication,
    DifferentProcess,
    PartialIdentityMismatch,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DeliveryTargetMismatch {
    kind: DeliveryTargetMismatchKind,
    same_application: bool,
    same_process: bool,
    transitions: AppTransitionSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationOutcome {
    Verified,
    DifferentApplication,
    DifferentProcess,
    ProcessRelaunched,
    PartialIdentityMismatch,
    LookupUnavailable,
    StartIdentityIncomplete,
    StartTargetIsSelf,
    StaleOwner,
}

impl VerificationOutcome {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::DifferentApplication => "different_application",
            Self::DifferentProcess => "different_process",
            Self::ProcessRelaunched => "process_relaunched",
            Self::PartialIdentityMismatch => "partial_identity_mismatch",
            Self::LookupUnavailable => "lookup_unavailable",
            Self::StartIdentityIncomplete => "start_identity_incomplete",
            Self::StartTargetIsSelf => "start_target_is_self",
            Self::StaleOwner => "stale_owner",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationSource {
    Native,
    None,
}

impl VerificationSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowRelation {
    Unknown,
    Same,
    Different,
}

impl WindowRelation {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Same => "same",
            Self::Different => "different",
        }
    }
}

/// Content-free evidence returned for every delivery verification decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerificationEvidence {
    pub outcome: VerificationOutcome,
    pub source: VerificationSource,
    pub retry_count: u64,
    pub elapsed_ms: u64,
    pub same_application: bool,
    pub same_process: bool,
    pub same_process_instance: bool,
    pub window_relation: WindowRelation,
    pub activation_changed: bool,
    pub space_changed: bool,
    pub current_is_self: bool,
    pub ownership_current: bool,
}

impl VerificationEvidence {
    pub(crate) const fn verified(self) -> bool {
        matches!(self.outcome, VerificationOutcome::Verified)
    }
}

/// Ephemeral frontmost-app metadata used only by an opted-in Voice Query pass.
/// These strings are prompt content: callers must not log, trace, or persist
/// them, and the type intentionally has no `Debug` implementation.
#[derive(Clone)]
pub struct QueryAppMetadata {
    pub application_name: Option<String>,
    pub window_title: Option<String>,
}

#[derive(Debug)]
struct RunningApplicationCandidate {
    bundle_id: Option<String>,
    name: Option<String>,
    regular: bool,
    current_process: bool,
}

fn bounded_running_applications(
    candidates: impl IntoIterator<Item = RunningApplicationCandidate>,
) -> Vec<RunningApplication> {
    let mut applications = candidates
        .into_iter()
        .filter(|candidate| candidate.regular && !candidate.current_process)
        .filter_map(|candidate| {
            let bundle_id = candidate.bundle_id?.trim().to_string();
            if bundle_id.is_empty() {
                return None;
            }
            let name = candidate.name.unwrap_or_default().trim().to_string();
            Some(RunningApplication {
                name: if name.is_empty() {
                    bundle_id.clone()
                } else {
                    name
                },
                bundle_id,
            })
        })
        .collect::<Vec<_>>();
    applications.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| {
                left.bundle_id
                    .to_lowercase()
                    .cmp(&right.bundle_id.to_lowercase())
            })
    });
    let mut seen = std::collections::HashSet::new();
    applications.retain(|application| seen.insert(application.bundle_id.to_lowercase()));
    applications.truncate(MAX_RUNNING_APPLICATIONS);
    applications
}

/// Return a bounded, ephemeral list for Settings. The caller owns the only
/// copy; this module does not cache or log app names or bundle identifiers.
#[tauri::command]
#[cfg(target_os = "macos")]
pub fn list_running_applications() -> Vec<RunningApplication> {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};

    let current_pid = std::process::id() as i32;
    let candidates = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .map(|application| RunningApplicationCandidate {
            bundle_id: application
                .bundleIdentifier()
                .map(|value| value.to_string()),
            name: application.localizedName().map(|value| value.to_string()),
            regular: application.activationPolicy() == NSApplicationActivationPolicy::Regular,
            current_process: application.processIdentifier() == current_pid,
        })
        .collect::<Vec<_>>();
    bounded_running_applications(candidates)
}

/// Unsupported test targets retain the command surface without probing
/// platform process state.
#[tauri::command]
#[cfg(not(target_os = "macos"))]
pub fn list_running_applications() -> Vec<RunningApplication> {
    Vec::new()
}

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

#[cfg(any(target_os = "macos", test))]
const MAX_NATIVE_ATTEMPTS: usize = 3;
#[cfg(any(target_os = "macos", test))]
const NATIVE_RETRY_DELAY: Duration = Duration::from_millis(10);

#[cfg(any(target_os = "macos", test))]
type QueryResult = Result<Option<String>, ()>;

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectionSource {
    None,
    Native,
    Osascript,
}

#[cfg(any(target_os = "macos", test))]
impl DetectionSource {
    const fn code(self) -> u64 {
        match self {
            Self::None => 0,
            Self::Native => 1,
            Self::Osascript => 2,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
struct DetectionResult {
    bundle_id: Option<String>,
    source: DetectionSource,
    retry_count: usize,
}

#[cfg(any(target_os = "macos", test))]
impl DetectionResult {
    fn outcome_code(&self) -> u64 {
        u64::from(self.bundle_id.is_some())
    }
}

#[cfg(any(target_os = "macos", test))]
fn normalized_bundle_id(result: QueryResult) -> Option<String> {
    result.ok().flatten().and_then(|bundle_id| {
        let bundle_id = bundle_id.trim();
        (!bundle_id.is_empty() && !bundle_id.eq_ignore_ascii_case("missing value"))
            .then(|| bundle_id.to_string())
    })
}

#[cfg(any(target_os = "macos", test))]
fn detect_with<N, F, S>(mut native: N, mut fallback: F, mut sleep: S) -> DetectionResult
where
    N: FnMut() -> QueryResult,
    F: FnMut() -> QueryResult,
    S: FnMut(Duration),
{
    for attempt in 0..MAX_NATIVE_ATTEMPTS {
        if let Some(bundle_id) = normalized_bundle_id(native()) {
            return DetectionResult {
                bundle_id: Some(bundle_id),
                source: DetectionSource::Native,
                retry_count: attempt,
            };
        }

        if attempt + 1 < MAX_NATIVE_ATTEMPTS {
            sleep(NATIVE_RETRY_DELAY);
        }
    }

    let retry_count = MAX_NATIVE_ATTEMPTS.saturating_sub(1);
    if let Some(bundle_id) = normalized_bundle_id(fallback()) {
        DetectionResult {
            bundle_id: Some(bundle_id),
            source: DetectionSource::Osascript,
            retry_count,
        }
    } else {
        DetectionResult {
            bundle_id: None,
            source: DetectionSource::None,
            retry_count,
        }
    }
}

#[cfg(target_os = "macos")]
fn native_frontmost_bundle_id() -> QueryResult {
    use objc2_app_kit::NSWorkspace;

    let app = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or(())?;
    Ok(app.bundleIdentifier().map(|value| value.to_string()))
}

#[cfg(target_os = "macos")]
fn osascript_frontmost_bundle_id() -> QueryResult {
    let output = crate::injector::run_osascript_with_timeout(
        r#"tell application "System Events" to get bundle identifier of first process whose frontmost is true"#,
    )
    .map_err(|_| ())?;

    if !output.status.success() {
        return Err(());
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn app_transition_snapshot() -> AppTransitionSnapshot {
    AppTransitionSnapshot {
        activation_generation: ACTIVATION_GENERATION.load(Ordering::SeqCst),
        space_generation: SPACE_GENERATION.load(Ordering::SeqCst),
    }
}

/// Register app-lifetime, content-free transition counters. Observer callbacks
/// deliberately do no native lookup and capture no application identity.
#[cfg(target_os = "macos")]
pub(crate) fn register_delivery_transition_observers() {
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
        NSWorkspaceDidActivateApplicationNotification,
    };
    use objc2_foundation::NSNotification;

    fn register(
        center: &objc2_foundation::NSNotificationCenter,
        name: &objc2_foundation::NSNotificationName,
        generation: &'static AtomicU64,
    ) {
        let block =
            block2::RcBlock::new(move |_notification: std::ptr::NonNull<NSNotification>| {
                generation.fetch_add(1, Ordering::SeqCst);
            });
        unsafe {
            let observer =
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
            std::mem::forget(observer);
        }
    }

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    register(
        &center,
        unsafe { NSWorkspaceDidActivateApplicationNotification },
        &ACTIVATION_GENERATION,
    );
    register(
        &center,
        unsafe { NSWorkspaceActiveSpaceDidChangeNotification },
        &SPACE_GENERATION,
    );
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn register_delivery_transition_observers() {}

/// Capture exactly one native frontmost-application sample for delivery. The
/// compatibility bundle detector is intentionally not used: auto-paste needs a
/// complete bundle/PID/process-instance tuple from the same application
/// object. If activation or Space changes during that sample, fail closed.
#[cfg(target_os = "macos")]
pub(crate) fn capture_delivery_target_snapshot() -> DeliveryTargetSnapshot {
    use objc2_app_kit::NSWorkspace;

    let before = app_transition_snapshot();
    let Some(application) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let process_id = application.processIdentifier();
    if process_id == std::process::id() as i32 {
        return DeliveryTargetSnapshot::SelfTarget;
    }
    if process_id <= 0 {
        return DeliveryTargetSnapshot::Incomplete;
    }
    let Some(bundle_id) = application
        .bundleIdentifier()
        .map(|value| value.to_string())
    else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let bundle_id = bundle_id.trim();
    if bundle_id.is_empty() || bundle_id.eq_ignore_ascii_case("missing value") {
        return DeliveryTargetSnapshot::Incomplete;
    }
    let Some(launch_date) = application.launchDate() else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let launch_interval = launch_date.timeIntervalSinceReferenceDate();
    if !launch_interval.is_finite() || launch_interval <= 0.0 {
        return DeliveryTargetSnapshot::Incomplete;
    }
    let process_instance = ProcessInstanceToken(launch_interval.to_bits());
    let window_token = ax_window_token(process_id);
    let after = app_transition_snapshot();
    if before != after {
        return DeliveryTargetSnapshot::Incomplete;
    }

    DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
        bundle_id: bundle_id.to_string(),
        process_id,
        process_instance,
        window_token,
        transitions: after,
    })
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn capture_delivery_target_snapshot() -> DeliveryTargetSnapshot {
    DeliveryTargetSnapshot::Incomplete
}

fn normalized_native_bundle_id(bundle_id: Option<String>) -> Option<String> {
    bundle_id.and_then(|bundle_id| {
        let bundle_id = bundle_id.trim();
        (!bundle_id.is_empty() && !bundle_id.eq_ignore_ascii_case("missing value"))
            .then(|| bundle_id.to_string())
    })
}

fn delivery_mismatch_from_partial_identity(
    expected: &DeliveryTargetIdentity,
    process_id: i32,
    bundle_id: Option<&str>,
    process_instance: Option<ProcessInstanceToken>,
) -> Option<DeliveryTargetSnapshot> {
    let same_process = process_id == expected.process_id;
    let same_application = bundle_id.is_some_and(|bundle_id| bundle_id == expected.bundle_id);
    let kind = if bundle_id.is_some_and(|bundle_id| bundle_id != expected.bundle_id) {
        DeliveryTargetMismatchKind::DifferentApplication
    } else if !same_process && same_application {
        DeliveryTargetMismatchKind::DifferentProcess
    } else if !same_process
        || (bundle_id.is_none()
            && process_instance.is_some_and(|token| token != expected.process_instance))
    {
        DeliveryTargetMismatchKind::PartialIdentityMismatch
    } else {
        return None;
    };
    Some(DeliveryTargetSnapshot::Mismatch(DeliveryTargetMismatch {
        kind,
        same_application,
        same_process,
        transitions: app_transition_snapshot(),
    }))
}

/// First delivery-time sample. Unlike accepted-start capture, transition churn
/// is diagnostic here: preserve the complete tuple and latest generations, then
/// let the tuple-only final sample decide whether identity stayed stable across
/// the optional AX window lookup.
#[cfg(target_os = "macos")]
fn capture_delivery_verification_first_snapshot(
    expected: &DeliveryTargetIdentity,
) -> DeliveryTargetSnapshot {
    use objc2_app_kit::NSWorkspace;

    let Some(application) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let process_id = application.processIdentifier();
    if process_id == std::process::id() as i32 {
        return DeliveryTargetSnapshot::SelfTarget;
    }
    if process_id <= 0 {
        return DeliveryTargetSnapshot::Incomplete;
    }
    let bundle_id = normalized_native_bundle_id(
        application
            .bundleIdentifier()
            .map(|value| value.to_string()),
    );
    let process_instance = application.launchDate().and_then(|launch_date| {
        let interval = launch_date.timeIntervalSinceReferenceDate();
        (interval.is_finite() && interval > 0.0).then(|| ProcessInstanceToken(interval.to_bits()))
    });
    if let Some(mismatch) = delivery_mismatch_from_partial_identity(
        expected,
        process_id,
        bundle_id.as_deref(),
        process_instance,
    ) {
        return mismatch;
    }
    let Some(bundle_id) = bundle_id else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let Some(process_instance) = process_instance else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
        bundle_id,
        process_id,
        process_instance,
        window_token: ax_window_token(process_id),
        transitions: app_transition_snapshot(),
    })
}

#[cfg(not(target_os = "macos"))]
fn capture_delivery_verification_first_snapshot(
    _expected: &DeliveryTargetIdentity,
) -> DeliveryTargetSnapshot {
    DeliveryTargetSnapshot::Incomplete
}

/// Capture only the native process tuple, without an AX window lookup. Used as
/// the final half of delivery verification so a focus change during the first
/// sample's optional AX query cannot be hidden by delayed workspace observers.
#[cfg(target_os = "macos")]
fn capture_delivery_process_snapshot(expected: &DeliveryTargetIdentity) -> DeliveryTargetSnapshot {
    use objc2_app_kit::NSWorkspace;

    let Some(application) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let process_id = application.processIdentifier();
    if process_id == std::process::id() as i32 {
        return DeliveryTargetSnapshot::SelfTarget;
    }
    if process_id <= 0 {
        return DeliveryTargetSnapshot::Incomplete;
    }
    let bundle_id = normalized_native_bundle_id(
        application
            .bundleIdentifier()
            .map(|value| value.to_string()),
    );
    let process_instance = application.launchDate().and_then(|launch_date| {
        let interval = launch_date.timeIntervalSinceReferenceDate();
        (interval.is_finite() && interval > 0.0).then(|| ProcessInstanceToken(interval.to_bits()))
    });
    if let Some(mismatch) = delivery_mismatch_from_partial_identity(
        expected,
        process_id,
        bundle_id.as_deref(),
        process_instance,
    ) {
        return mismatch;
    }
    let Some(bundle_id) = bundle_id else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    let Some(process_instance) = process_instance else {
        return DeliveryTargetSnapshot::Incomplete;
    };
    DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
        bundle_id,
        process_id,
        process_instance,
        window_token: None,
        transitions: app_transition_snapshot(),
    })
}

#[cfg(not(target_os = "macos"))]
fn capture_delivery_process_snapshot(_expected: &DeliveryTargetIdentity) -> DeliveryTargetSnapshot {
    DeliveryTargetSnapshot::Incomplete
}

fn same_delivery_process_tuple(
    first: &DeliveryTargetIdentity,
    final_sample: &DeliveryTargetIdentity,
) -> bool {
    first.bundle_id == final_sample.bundle_id
        && first.process_id == final_sample.process_id
        && first.process_instance == final_sample.process_instance
}

fn capture_current_delivery_target_with<F, S>(
    expected: &DeliveryTargetIdentity,
    mut first_capture: F,
    mut final_capture: S,
) -> DeliveryTargetSnapshot
where
    F: FnMut() -> DeliveryTargetSnapshot,
    S: FnMut() -> DeliveryTargetSnapshot,
{
    let first = first_capture();
    let DeliveryTargetSnapshot::Complete(mut first_identity) = first else {
        return first;
    };
    // A positively observed mismatch is terminal. Never let a later sample of
    // the expected target erase evidence that focus had already moved away.
    if !same_delivery_process_tuple(expected, &first_identity) {
        return DeliveryTargetSnapshot::Complete(first_identity);
    }
    match final_capture() {
        DeliveryTargetSnapshot::Complete(final_identity)
            if same_delivery_process_tuple(&first_identity, &final_identity) =>
        {
            first_identity.transitions = final_identity.transitions;
            DeliveryTargetSnapshot::Complete(first_identity)
        }
        final_sample => final_sample,
    }
}

/// Delivery-time native sample with a final tuple re-check after optional AX
/// window evidence. Start capture intentionally does not use this: its contract
/// is exactly one retained NSRunningApplication sample at acceptance.
fn capture_current_delivery_target_snapshot(
    expected: &DeliveryTargetIdentity,
) -> DeliveryTargetSnapshot {
    capture_current_delivery_target_with(
        expected,
        || capture_delivery_verification_first_snapshot(expected),
        || capture_delivery_process_snapshot(expected),
    )
}

/// Resolve profile identity from the same accepted native sample that owns
/// delivery. Live dictation must never combine profile/context from a later
/// frontmost app with an earlier immutable paste target.
pub(crate) fn profile_identity_from_delivery_target(
    target: &DeliveryTargetSnapshot,
) -> FrontmostAppIdentity {
    match target {
        DeliveryTargetSnapshot::Complete(identity) => FrontmostAppIdentity {
            bundle_id: Some(identity.bundle_id.clone()),
            process_id: Some(identity.process_id),
        },
        DeliveryTargetSnapshot::SelfTarget
        | DeliveryTargetSnapshot::Incomplete
        | DeliveryTargetSnapshot::Mismatch(_) => FrontmostAppIdentity {
            bundle_id: None,
            process_id: None,
        },
    }
}

fn mismatch_verification_evidence(
    expected: &DeliveryTargetIdentity,
    mismatch: &DeliveryTargetMismatch,
    retry_count: u64,
) -> VerificationEvidence {
    let outcome = match mismatch.kind {
        DeliveryTargetMismatchKind::DifferentApplication => {
            VerificationOutcome::DifferentApplication
        }
        DeliveryTargetMismatchKind::DifferentProcess => VerificationOutcome::DifferentProcess,
        DeliveryTargetMismatchKind::PartialIdentityMismatch => {
            VerificationOutcome::PartialIdentityMismatch
        }
    };
    VerificationEvidence {
        outcome,
        source: VerificationSource::Native,
        retry_count,
        elapsed_ms: 0,
        same_application: mismatch.same_application,
        same_process: mismatch.same_process,
        same_process_instance: false,
        window_relation: WindowRelation::Unknown,
        activation_changed: expected.transitions.activation_generation
            != mismatch.transitions.activation_generation,
        space_changed: expected.transitions.space_generation
            != mismatch.transitions.space_generation,
        current_is_self: false,
        ownership_current: true,
    }
}

fn unavailable_verification_evidence(
    outcome: VerificationOutcome,
    ownership_current: bool,
) -> VerificationEvidence {
    VerificationEvidence {
        outcome,
        source: VerificationSource::None,
        retry_count: 0,
        elapsed_ms: 0,
        same_application: false,
        same_process: false,
        same_process_instance: false,
        window_relation: WindowRelation::Unknown,
        activation_changed: false,
        space_changed: false,
        current_is_self: false,
        ownership_current,
    }
}

fn classify_delivery_target(
    expected: &DeliveryTargetIdentity,
    current: Option<&DeliveryTargetIdentity>,
    self_process_id: i32,
    ownership_current: bool,
) -> VerificationEvidence {
    if !ownership_current {
        return unavailable_verification_evidence(VerificationOutcome::StaleOwner, false);
    }
    let Some(current) = current else {
        return unavailable_verification_evidence(VerificationOutcome::LookupUnavailable, true);
    };

    let same_application = expected.bundle_id == current.bundle_id;
    let same_process = expected.process_id == current.process_id;
    let same_process_instance =
        same_application && same_process && expected.process_instance == current.process_instance;
    let window_relation = match (expected.window_token, current.window_token) {
        (Some(expected), Some(current)) if expected == current => WindowRelation::Same,
        (Some(_), Some(_)) => WindowRelation::Different,
        _ => WindowRelation::Unknown,
    };
    let outcome = if !same_application {
        VerificationOutcome::DifferentApplication
    } else if !same_process {
        VerificationOutcome::DifferentProcess
    } else if !same_process_instance {
        VerificationOutcome::ProcessRelaunched
    } else {
        VerificationOutcome::Verified
    };
    VerificationEvidence {
        outcome,
        source: VerificationSource::Native,
        retry_count: 0,
        elapsed_ms: 0,
        same_application,
        same_process,
        same_process_instance,
        window_relation,
        activation_changed: expected.transitions.activation_generation
            != current.transitions.activation_generation,
        space_changed: expected.transitions.space_generation
            != current.transitions.space_generation,
        current_is_self: current.process_id == self_process_id,
        ownership_current,
    }
}

fn verify_delivery_target_with<Q, S>(
    expected: &DeliveryTargetSnapshot,
    self_process_id: i32,
    ownership_current: bool,
    mut query: Q,
    mut sleep: S,
) -> VerificationEvidence
where
    Q: FnMut() -> DeliveryTargetSnapshot,
    S: FnMut(std::time::Duration),
{
    if !ownership_current {
        return unavailable_verification_evidence(VerificationOutcome::StaleOwner, false);
    }
    let expected = match expected {
        DeliveryTargetSnapshot::Complete(expected) => expected,
        DeliveryTargetSnapshot::SelfTarget => {
            let mut evidence =
                unavailable_verification_evidence(VerificationOutcome::StartTargetIsSelf, true);
            evidence.source = VerificationSource::Native;
            evidence.current_is_self = true;
            return evidence;
        }
        DeliveryTargetSnapshot::Incomplete => {
            return unavailable_verification_evidence(
                VerificationOutcome::StartIdentityIncomplete,
                true,
            );
        }
        DeliveryTargetSnapshot::Mismatch(_) => {
            return unavailable_verification_evidence(
                VerificationOutcome::StartIdentityIncomplete,
                true,
            );
        }
    };

    for attempt in 0..MAX_DELIVERY_VERIFICATION_ATTEMPTS {
        match query() {
            DeliveryTargetSnapshot::Complete(current) => {
                let mut evidence =
                    classify_delivery_target(expected, Some(&current), self_process_id, true);
                evidence.retry_count = attempt as u64;
                return evidence;
            }
            DeliveryTargetSnapshot::SelfTarget => {
                let mut evidence = unavailable_verification_evidence(
                    VerificationOutcome::DifferentApplication,
                    true,
                );
                evidence.source = VerificationSource::Native;
                evidence.retry_count = attempt as u64;
                evidence.current_is_self = true;
                return evidence;
            }
            DeliveryTargetSnapshot::Incomplete => {}
            DeliveryTargetSnapshot::Mismatch(mismatch) => {
                return mismatch_verification_evidence(expected, &mismatch, attempt as u64);
            }
        }
        if attempt + 1 < MAX_DELIVERY_VERIFICATION_ATTEMPTS {
            sleep(DELIVERY_VERIFICATION_RETRY_DELAY);
        }
    }

    let mut evidence = unavailable_verification_evidence(
        VerificationOutcome::LookupUnavailable,
        ownership_current,
    );
    evidence.retry_count = MAX_DELIVERY_VERIFICATION_ATTEMPTS.saturating_sub(1) as u64;
    evidence
}

/// Re-verify the exact native delivery identity. Only transient native lookup
/// failure is retried; a positively observed mismatch is terminal.
pub(crate) fn verify_delivery_target(
    expected: &DeliveryTargetSnapshot,
    ownership_current: bool,
) -> VerificationEvidence {
    let started = Instant::now();
    let query_expected = match expected {
        DeliveryTargetSnapshot::Complete(identity) => Some(identity.clone()),
        DeliveryTargetSnapshot::SelfTarget
        | DeliveryTargetSnapshot::Incomplete
        | DeliveryTargetSnapshot::Mismatch(_) => None,
    };
    let mut evidence = verify_delivery_target_with(
        expected,
        std::process::id() as i32,
        ownership_current,
        || {
            query_expected
                .as_ref()
                .map(capture_current_delivery_target_snapshot)
                .unwrap_or(DeliveryTargetSnapshot::Incomplete)
        },
        std::thread::sleep,
    );
    evidence.elapsed_ms = (started.elapsed().as_millis() as u64).min(1_000);
    evidence
}

/// Return the bundle identifier of the first frontmost macOS app observed by
/// the bounded detector. Returns `None` on total failure so the caller resolves
/// a global-only dictation context.
#[cfg(target_os = "macos")]
pub fn frontmost_bundle_id() -> Option<String> {
    let started = std::time::Instant::now();
    let result = detect_with(
        native_frontmost_bundle_id,
        osascript_frontmost_bundle_id,
        std::thread::sleep,
    );
    tracing::info!(
        target: "pipeline",
        outcome_code = result.outcome_code(),
        retry_count = result.retry_count as u64,
        source_code = result.source.code(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "frontmost app detection completed"
    );
    result.bundle_id
}

/// Capture the frontmost bundle and PID as one privacy-bounded recording input.
/// The bounded bundle detector retains its compatibility fallback. A PID is
/// accepted only when a second native sample still reports the same bundle,
/// preventing a focus transition from pairing identities from different apps.
#[cfg(target_os = "macos")]
pub fn frontmost_app_identity() -> FrontmostAppIdentity {
    use objc2_app_kit::NSWorkspace;

    let bundle_id = frontmost_bundle_id();
    let process_id = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|application| {
            let native_bundle = application
                .bundleIdentifier()
                .map(|value| value.to_string());
            (native_bundle == bundle_id).then(|| application.processIdentifier())
        });
    FrontmostAppIdentity {
        bundle_id,
        process_id,
    }
}

/// Freeze Voice Query's app identity from one native `NSWorkspace` sample.
/// Unlike dictation's compatibility detector, this path never invokes
/// AppleScript or any other child process: unavailable native state simply
/// yields an empty identity and therefore no query context.
#[cfg(target_os = "macos")]
pub fn query_frontmost_app_identity() -> FrontmostAppIdentity {
    use objc2_app_kit::NSWorkspace;

    let Some(application) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return FrontmostAppIdentity {
            bundle_id: None,
            process_id: None,
        };
    };
    query_identity(
        application.processIdentifier(),
        application
            .bundleIdentifier()
            .map(|value| value.to_string()),
    )
}

#[cfg(any(target_os = "macos", test))]
fn query_identity(process_id: i32, bundle_id: Option<String>) -> FrontmostAppIdentity {
    // Bundle ID is required to apply the user's per-app deny profile. If it is
    // unavailable, fail closed instead of using a PID-only identity that could
    // bypass an exclusion.
    let bundle_id = bundle_id.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    });
    match bundle_id {
        Some(bundle_id) => FrontmostAppIdentity {
            bundle_id: Some(bundle_id),
            process_id: Some(process_id),
        },
        None => FrontmostAppIdentity {
            bundle_id: None,
            process_id: None,
        },
    }
}

/// Query context requires the exact native PID and bundle pair. Partial
/// identities are deliberately never accepted because they cannot prove that
/// per-app privacy exclusions were resolved for the same application.
pub(crate) fn query_identity_matches(
    expected: &FrontmostAppIdentity,
    actual: &FrontmostAppIdentity,
) -> bool {
    matches!(
        (
            expected.process_id,
            expected.bundle_id.as_deref(),
            actual.process_id,
            actual.bundle_id.as_deref(),
        ),
        (Some(expected_pid), Some(expected_bundle), Some(actual_pid), Some(actual_bundle))
            if expected_pid == actual_pid && expected_bundle == actual_bundle
    )
}

#[cfg(target_os = "macos")]
pub(crate) async fn query_identity_is_current(
    app_handle: &tauri::AppHandle,
    expected: &FrontmostAppIdentity,
) -> bool {
    let expected = expected.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    if app_handle
        .run_on_main_thread(move || {
            let current = query_frontmost_app_identity();
            let _ = tx.send(query_identity_matches(&expected, &current));
        })
        .is_err()
    {
        return false;
    }
    rx.await.unwrap_or(false)
}

/// Read the app name and focused-window title for the exact app identity that
/// was frozen at query start. The native sample is rejected if focus moved in
/// the meantime, preventing context from two different apps being combined.
/// Window titles are read directly through Accessibility; no shell,
/// AppleScript, screen capture, or OCR path is involved.
#[cfg(target_os = "macos")]
pub async fn query_app_metadata(
    app_handle: &tauri::AppHandle,
    expected: &FrontmostAppIdentity,
) -> Option<QueryAppMetadata> {
    if expected.process_id.is_none() && expected.bundle_id.is_none() {
        return None;
    }
    let expected = expected.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app_handle
        .run_on_main_thread(move || {
            let _ = tx.send(native_query_metadata(&expected));
        })
        .ok()?;
    rx.await.ok().flatten()
}

#[cfg(target_os = "macos")]
fn native_query_metadata(expected: &FrontmostAppIdentity) -> Option<QueryAppMetadata> {
    use objc2_app_kit::NSWorkspace;

    let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let process_id = application.processIdentifier();
    let bundle_id = application
        .bundleIdentifier()
        .map(|value| value.to_string());
    let actual = query_identity(process_id, bundle_id.clone());
    if !query_identity_matches(expected, &actual) {
        return None;
    }
    let application_name = application
        .localizedName()
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| bundle_id.filter(|value| !value.trim().is_empty()));
    Some(QueryAppMetadata {
        application_name,
        window_title: ax_window_title(process_id),
    })
}

#[cfg(target_os = "macos")]
fn ax_window_token(process_id: i32) -> Option<WindowToken> {
    use std::ffi::{c_char, c_void, CString};

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFHash(value: CFTypeRef) -> usize;
        fn CFRelease(value: CFTypeRef);
    }

    struct CFGuard(CFTypeRef);
    impl Drop for CFGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    const AX_SUCCESS: i32 = 0;
    const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.025;
    const UTF8_ENCODING: u32 = 0x0800_0100;

    let application = unsafe { AXUIElementCreateApplication(process_id) };
    if application.is_null() {
        return None;
    }
    let application = CFGuard(application);
    if unsafe { AXUIElementSetMessagingTimeout(application.0, AX_QUERY_TIMEOUT_SECONDS) }
        != AX_SUCCESS
    {
        return None;
    }
    let attribute_name = CString::new("AXFocusedWindow").ok()?;
    let attribute = unsafe {
        CFStringCreateWithCString(std::ptr::null(), attribute_name.as_ptr(), UTF8_ENCODING)
    };
    if attribute.is_null() {
        return None;
    }
    let attribute = CFGuard(attribute);
    let mut window: CFTypeRef = std::ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(application.0, attribute.0, &mut window) };
    if status != AX_SUCCESS || window.is_null() {
        if !window.is_null() {
            unsafe { CFRelease(window) };
        }
        return None;
    }
    let window = CFGuard(window);
    Some(WindowToken(unsafe { CFHash(window.0) } as u64))
}

#[cfg(target_os = "macos")]
fn ax_window_title(process_id: i32) -> Option<String> {
    use std::ffi::{c_char, c_void, CStr, CString};

    type AXUIElementRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFTypeRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn CFStringCreateWithCString(
            allocator: CFTypeRef,
            string: *const c_char,
            encoding: u32,
        ) -> CFTypeRef;
        fn CFStringGetLength(string: CFTypeRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            string: CFTypeRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> bool;
        fn CFRelease(value: CFTypeRef);
    }

    struct CFGuard(CFTypeRef);
    impl Drop for CFGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    const AX_SUCCESS: i32 = 0;
    const AX_QUERY_TIMEOUT_SECONDS: f32 = 0.025;
    const UTF8_ENCODING: u32 = 0x0800_0100;

    fn attribute(name: &str) -> Option<CFGuard> {
        let name = CString::new(name).ok()?;
        let value =
            unsafe { CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8_ENCODING) };
        (!value.is_null()).then(|| CFGuard(value))
    }

    fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFGuard> {
        let attribute = attribute(name)?;
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, attribute.0, &mut value) };
        if status != AX_SUCCESS || value.is_null() {
            if !value.is_null() {
                unsafe { CFRelease(value) };
            }
            return None;
        }
        Some(CFGuard(value))
    }

    fn string(value: CFTypeRef) -> Option<String> {
        let length = unsafe { CFStringGetLength(value) };
        let maximum = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
        if maximum < 0 {
            return None;
        }
        let mut buffer = vec![0 as c_char; maximum.saturating_add(1) as usize];
        let converted = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                UTF8_ENCODING,
            )
        };
        converted.then(|| {
            unsafe { CStr::from_ptr(buffer.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
    }

    let application = unsafe { AXUIElementCreateApplication(process_id) };
    if application.is_null() {
        return None;
    }
    let application = CFGuard(application);
    if unsafe { AXUIElementSetMessagingTimeout(application.0, AX_QUERY_TIMEOUT_SECONDS) }
        != AX_SUCCESS
    {
        return None;
    }
    let window = copy_attribute(application.0, "AXFocusedWindow")?;
    if unsafe { AXUIElementSetMessagingTimeout(window.0, AX_QUERY_TIMEOUT_SECONDS) } != AX_SUCCESS {
        return None;
    }
    string(copy_attribute(window.0, "AXTitle")?.0).filter(|title| !title.trim().is_empty())
}

/// Non-macOS platforms have no frontmost-app concept here; profiles are a no-op.
#[cfg(not(target_os = "macos"))]
pub fn frontmost_bundle_id() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_app_identity() -> FrontmostAppIdentity {
    FrontmostAppIdentity {
        bundle_id: None,
        process_id: None,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn query_frontmost_app_identity() -> FrontmostAppIdentity {
    FrontmostAppIdentity {
        bundle_id: None,
        process_id: None,
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn query_identity_is_current(
    _app_handle: &tauri::AppHandle,
    _expected: &FrontmostAppIdentity,
) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub async fn query_app_metadata(
    _app_handle: &tauri::AppHandle,
    _expected: &FrontmostAppIdentity,
) -> Option<QueryAppMetadata> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    fn candidate(id: &str, name: &str) -> RunningApplicationCandidate {
        RunningApplicationCandidate {
            bundle_id: Some(id.to_string()),
            name: Some(name.to_string()),
            regular: true,
            current_process: false,
        }
    }

    fn delivery_target(
        bundle_id: &str,
        process_id: i32,
        process_instance: u64,
        window_token: Option<u64>,
    ) -> DeliveryTargetIdentity {
        DeliveryTargetIdentity {
            bundle_id: bundle_id.to_string(),
            process_id,
            process_instance: ProcessInstanceToken(process_instance),
            window_token: window_token.map(WindowToken),
            transitions: AppTransitionSnapshot {
                activation_generation: 10,
                space_generation: 20,
            },
        }
    }

    #[test]
    fn running_app_picker_is_sorted_deduplicated_and_private_by_default() {
        let mut candidates = vec![
            candidate("com.example.zulu", "Zulu"),
            candidate("com.example.alpha", "Alpha"),
        ];
        candidates.push(candidate("COM.EXAMPLE.03", "Duplicate"));
        candidates.push(candidate("com.example.03", "Elsewhere in sort order"));
        candidates.push(RunningApplicationCandidate {
            bundle_id: Some("com.example.menu".into()),
            name: Some("Menu helper".into()),
            regular: false,
            current_process: false,
        });
        candidates.push(RunningApplicationCandidate {
            bundle_id: Some("com.example.murmur".into()),
            name: Some("Murmur".into()),
            regular: true,
            current_process: true,
        });

        let applications = bounded_running_applications(candidates);

        assert_eq!(applications.len(), 3);
        assert_eq!(applications[0].name, "Alpha");
        assert_eq!(
            applications
                .iter()
                .filter(|app| app.bundle_id.eq_ignore_ascii_case("com.example.03"))
                .count(),
            1
        );
        assert!(applications
            .iter()
            .all(|app| app.bundle_id != "com.example.menu"));
        assert!(applications
            .iter()
            .all(|app| app.bundle_id != "com.example.murmur"));
    }

    #[test]
    fn running_app_picker_is_bounded() {
        let candidates = (0..80)
            .map(|index| {
                candidate(
                    &format!("com.example.{index:02}"),
                    &format!("App {index:02}"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bounded_running_applications(candidates).len(),
            MAX_RUNNING_APPLICATIONS
        );
    }

    #[test]
    fn running_app_payload_contains_only_picker_fields() {
        let payload = serde_json::to_value(RunningApplication {
            bundle_id: "com.apple.Terminal".into(),
            name: "Terminal".into(),
        })
        .expect("serialize picker payload");

        assert_eq!(payload.as_object().expect("object").len(), 2);
        assert_eq!(payload["bundleId"], "com.apple.Terminal");
        assert_eq!(payload["name"], "Terminal");
    }

    #[test]
    fn immediate_native_success_skips_retry_and_fallback() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Ok(Some(" com.apple.Terminal ".to_string()))
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some("fallback".to_string()))
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.source, DetectionSource::Native);
        assert_eq!(result.retry_count, 0);
        assert_eq!(native_calls.get(), 1);
        assert_eq!(fallback_calls.get(), 0);
        assert_eq!(sleep_calls.get(), 0);
    }

    #[test]
    fn transient_native_failures_retry_until_success() {
        let mut native_results = VecDeque::from([
            Err(()),
            Ok(Some("  ".to_string())),
            Ok(Some("com.todesktop.cursor".to_string())),
        ]);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Err(())
            },
            |delay| {
                assert_eq!(delay, NATIVE_RETRY_DELAY);
                sleep_calls.set(sleep_calls.get() + 1);
            },
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.todesktop.cursor"));
        assert_eq!(result.source, DetectionSource::Native);
        assert_eq!(result.retry_count, 2);
        assert_eq!(fallback_calls.get(), 0);
        assert_eq!(sleep_calls.get(), 2);
    }

    #[test]
    fn fallback_succeeds_after_native_attempts_are_exhausted() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Err(())
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some("com.apple.Safari".to_string()))
            },
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Safari"));
        assert_eq!(result.source, DetectionSource::Osascript);
        assert_eq!(result.retry_count, 2);
        assert_eq!(native_calls.get(), MAX_NATIVE_ATTEMPTS);
        assert_eq!(fallback_calls.get(), 1);
    }

    #[test]
    fn total_failure_is_bounded_and_deny_by_default() {
        let native_calls = Cell::new(0);
        let fallback_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);

        let result = detect_with(
            || {
                native_calls.set(native_calls.get() + 1);
                Err(())
            },
            || {
                fallback_calls.set(fallback_calls.get() + 1);
                Ok(Some(String::new()))
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(result.bundle_id, None);
        assert_eq!(result.source, DetectionSource::None);
        assert_eq!(result.outcome_code(), 0);
        assert_eq!(result.retry_count, 2);
        assert_eq!(native_calls.get(), MAX_NATIVE_ATTEMPTS);
        assert_eq!(fallback_calls.get(), 1);
        assert_eq!(sleep_calls.get(), MAX_NATIVE_ATTEMPTS - 1);
    }

    #[test]
    fn compatibility_detector_rejects_osascript_missing_value_literal() {
        let result = detect_with(
            || Err(()),
            || Ok(Some("  MiSsInG VaLuE\n".to_string())),
            |_| {},
        );

        assert_eq!(result.bundle_id, None);
        assert_eq!(result.source, DetectionSource::None);
    }

    #[test]
    fn app_change_during_retry_uses_first_successful_sample() {
        let mut native_results = VecDeque::from([
            Err(()),
            Ok(Some("com.apple.Terminal".to_string())),
            Ok(Some("com.apple.Safari".to_string())),
        ]);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || Err(()),
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.retry_count, 1);
        assert_eq!(
            native_results.len(),
            1,
            "later focus changes are not sampled"
        );
    }

    #[test]
    fn first_success_is_immutable_even_if_the_app_would_change() {
        let mut native_results = VecDeque::from([
            Ok(Some("com.apple.Terminal".to_string())),
            Ok(Some("com.apple.Safari".to_string())),
        ]);

        let result = detect_with(
            || native_results.pop_front().expect("bounded native attempt"),
            || Err(()),
            |_| {},
        );

        assert_eq!(result.bundle_id.as_deref(), Some("com.apple.Terminal"));
        assert_eq!(result.retry_count, 0);
        assert_eq!(native_results.len(), 1, "detector must not re-read focus");
    }

    #[test]
    fn query_metadata_requires_the_frozen_app_identity() {
        let with_pid = FrontmostAppIdentity {
            bundle_id: Some("com.example.Editor".to_string()),
            process_id: Some(42),
        };
        let same = FrontmostAppIdentity {
            bundle_id: Some("com.example.Editor".to_string()),
            process_id: Some(42),
        };
        assert!(query_identity_matches(&with_pid, &same));
        let bundle_only = FrontmostAppIdentity {
            bundle_id: Some("com.example.Editor".to_string()),
            process_id: None,
        };
        assert!(!query_identity_matches(&with_pid, &bundle_only));
        let different_bundle = FrontmostAppIdentity {
            bundle_id: Some("com.example.Other".to_string()),
            process_id: Some(42),
        };
        assert!(!query_identity_matches(&with_pid, &different_bundle));
    }

    #[test]
    fn query_identity_requires_a_bundle_id_for_profile_exclusions() {
        assert_eq!(
            query_identity(42, Some(" com.example.Editor ".to_string())),
            FrontmostAppIdentity {
                bundle_id: Some("com.example.Editor".to_string()),
                process_id: Some(42),
            }
        );
        assert_eq!(
            query_identity(42, None),
            FrontmostAppIdentity {
                bundle_id: None,
                process_id: None,
            }
        );
    }

    #[test]
    fn delivery_verification_allows_same_process_in_a_different_window() {
        let expected = delivery_target("com.example.Editor", 41, 100, Some(7));
        let mut current = delivery_target("com.example.Editor", 41, 100, Some(8));
        current.transitions.activation_generation = 11;
        current.transitions.space_generation = 21;

        let evidence = classify_delivery_target(&expected, Some(&current), 999, true);

        assert_eq!(evidence.outcome, VerificationOutcome::Verified);
        assert_eq!(evidence.window_relation, WindowRelation::Different);
        assert!(evidence.activation_changed);
        assert!(evidence.space_changed);
        assert!(evidence.verified());
    }

    #[test]
    fn current_capture_rechecks_tuple_after_window_lookup() {
        let expected = delivery_target("com.example.Editor", 41, 100, Some(7));
        let first = DeliveryTargetSnapshot::Complete(expected.clone());
        let mut final_same_identity = delivery_target("com.example.Editor", 41, 100, None);
        final_same_identity.transitions = AppTransitionSnapshot {
            activation_generation: 12,
            space_generation: 22,
        };
        let final_same = DeliveryTargetSnapshot::Complete(final_same_identity);
        let result = capture_current_delivery_target_with(
            &expected,
            || first.clone(),
            || final_same.clone(),
        );
        match result {
            DeliveryTargetSnapshot::Complete(identity) => {
                assert!(identity.window_token == Some(WindowToken(7)));
                assert_eq!(identity.transitions.activation_generation, 12);
                assert_eq!(identity.transitions.space_generation, 22);
            }
            _ => panic!("unchanged tuple must retain first sample's window evidence"),
        }

        let final_changed =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Browser", 42, 200, None));
        let final_calls = Cell::new(0);
        let result = capture_current_delivery_target_with(
            &expected,
            || first.clone(),
            || {
                final_calls.set(final_calls.get() + 1);
                final_changed.clone()
            },
        );
        match result {
            DeliveryTargetSnapshot::Complete(identity) => {
                assert_eq!(identity.bundle_id, "com.example.Browser");
                assert_eq!(identity.process_id, 42);
                assert!(identity.window_token.is_none());
            }
            _ => panic!("changed tuple must return the final native sample"),
        }
        assert_eq!(final_calls.get(), 1);
    }

    #[test]
    fn current_capture_returns_self_or_incomplete_without_second_query() {
        let expected = delivery_target("com.example.Editor", 41, 100, None);
        let result = capture_current_delivery_target_with(
            &expected,
            || DeliveryTargetSnapshot::SelfTarget,
            || panic!("self sample must be immediate"),
        );
        assert!(matches!(result, DeliveryTargetSnapshot::SelfTarget));

        let result = capture_current_delivery_target_with(
            &expected,
            || DeliveryTargetSnapshot::Incomplete,
            || panic!("incomplete first sample is retried by outer verifier"),
        );
        assert!(matches!(result, DeliveryTargetSnapshot::Incomplete));
    }

    #[test]
    fn current_capture_never_adopts_expected_target_after_explicit_mismatch() {
        let expected = delivery_target("com.example.Editor", 41, 100, None);
        let first_changed =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Browser", 42, 200, None));
        let final_calls = Cell::new(0);
        let result = capture_current_delivery_target_with(
            &expected,
            || first_changed.clone(),
            || {
                final_calls.set(final_calls.get() + 1);
                DeliveryTargetSnapshot::Complete(expected.clone())
            },
        );

        match result {
            DeliveryTargetSnapshot::Complete(identity) => {
                assert_eq!(identity.bundle_id, "com.example.Browser");
                assert_eq!(identity.process_id, 42);
            }
            _ => panic!("first explicit mismatch must remain terminal"),
        }
        assert_eq!(final_calls.get(), 0);
    }

    #[test]
    fn partial_pid_mismatch_is_terminal_before_or_after_tuple_recheck() {
        let expected_identity = delivery_target("com.example.Editor", 41, 100, None);
        let expected = DeliveryTargetSnapshot::Complete(expected_identity.clone());
        let partial_mismatch =
            delivery_mismatch_from_partial_identity(&expected_identity, 42, None, None)
                .expect("known PID mismatch must be terminal");

        let final_calls = Cell::new(0);
        let first_result = capture_current_delivery_target_with(
            &expected_identity,
            || partial_mismatch.clone(),
            || {
                final_calls.set(final_calls.get() + 1);
                DeliveryTargetSnapshot::Complete(expected_identity.clone())
            },
        );
        assert!(matches!(first_result, DeliveryTargetSnapshot::Mismatch(_)));
        assert_eq!(
            final_calls.get(),
            0,
            "partial B -> A must not take a second native sample"
        );

        let query_calls = Cell::new(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                match query_calls.get() {
                    1 => partial_mismatch.clone(),
                    _ => DeliveryTargetSnapshot::Complete(expected_identity.clone()),
                }
            },
            |_| panic!("partial mismatch must not retry"),
        );
        assert_eq!(
            evidence.outcome,
            VerificationOutcome::PartialIdentityMismatch
        );
        assert_eq!(evidence.source, VerificationSource::Native);
        assert!(!evidence.same_application);
        assert!(!evidence.same_process);
        assert!(!evidence.same_process_instance);
        assert_eq!(evidence.window_relation, WindowRelation::Unknown);
        assert_eq!(query_calls.get(), 1, "B partial -> A must not authorize");

        let first_expected = DeliveryTargetSnapshot::Complete(expected_identity.clone());
        let final_calls = Cell::new(0);
        let result = capture_current_delivery_target_with(
            &expected_identity,
            || first_expected.clone(),
            || {
                final_calls.set(final_calls.get() + 1);
                partial_mismatch.clone()
            },
        );
        assert!(matches!(result, DeliveryTargetSnapshot::Mismatch(_)));
        assert_eq!(final_calls.get(), 1, "A -> partial B must return mismatch");
    }

    #[test]
    fn partial_identity_classifier_uses_only_proven_equality() {
        let expected = delivery_target("com.example.Editor", 41, 100, None);

        let different_app = delivery_mismatch_from_partial_identity(
            &expected,
            41,
            Some("com.example.Browser"),
            None,
        )
        .expect("bundle mismatch");
        match different_app {
            DeliveryTargetSnapshot::Mismatch(mismatch) => {
                assert!(matches!(
                    mismatch.kind,
                    DeliveryTargetMismatchKind::DifferentApplication
                ));
                assert!(!mismatch.same_application);
                assert!(mismatch.same_process);
            }
            _ => panic!("known bundle mismatch must be terminal"),
        }

        let different_process = delivery_mismatch_from_partial_identity(
            &expected,
            42,
            Some("com.example.Editor"),
            None,
        )
        .expect("process mismatch");
        match different_process {
            DeliveryTargetSnapshot::Mismatch(mismatch) => {
                assert!(matches!(
                    mismatch.kind,
                    DeliveryTargetMismatchKind::DifferentProcess
                ));
                assert!(mismatch.same_application);
                assert!(!mismatch.same_process);
            }
            _ => panic!("known same-app PID mismatch must be terminal"),
        }

        let relaunched_partial = delivery_mismatch_from_partial_identity(
            &expected,
            41,
            None,
            Some(ProcessInstanceToken(200)),
        )
        .expect("known launch-token mismatch must be terminal");
        match relaunched_partial {
            DeliveryTargetSnapshot::Mismatch(mismatch) => {
                assert!(matches!(
                    mismatch.kind,
                    DeliveryTargetMismatchKind::PartialIdentityMismatch
                ));
                assert!(!mismatch.same_application);
                assert!(mismatch.same_process);
            }
            _ => panic!("incomplete app metadata must retain the proven token mismatch"),
        }

        assert!(delivery_mismatch_from_partial_identity(
            &expected,
            41,
            None,
            Some(ProcessInstanceToken(100)),
        )
        .is_none());
    }

    #[test]
    fn partial_launch_token_mismatch_is_terminal_without_retry() {
        let expected_identity = delivery_target("com.example.Editor", 41, 100, None);
        let expected = DeliveryTargetSnapshot::Complete(expected_identity.clone());
        let partial_mismatch = delivery_mismatch_from_partial_identity(
            &expected_identity,
            41,
            None,
            Some(ProcessInstanceToken(200)),
        )
        .expect("known launch-token mismatch must be terminal");
        let final_calls = Cell::new(0);
        let first_result = capture_current_delivery_target_with(
            &expected_identity,
            || partial_mismatch.clone(),
            || {
                final_calls.set(final_calls.get() + 1);
                DeliveryTargetSnapshot::Complete(expected_identity.clone())
            },
        );
        assert!(matches!(first_result, DeliveryTargetSnapshot::Mismatch(_)));
        assert_eq!(final_calls.get(), 0);

        let query_calls = Cell::new(0);

        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                partial_mismatch.clone()
            },
            |_| panic!("partial launch-token mismatch must not retry"),
        );

        assert_eq!(
            evidence.outcome,
            VerificationOutcome::PartialIdentityMismatch
        );
        assert_eq!(evidence.source, VerificationSource::Native);
        assert!(!evidence.same_application);
        assert!(evidence.same_process);
        assert!(!evidence.same_process_instance);
        assert_eq!(evidence.window_relation, WindowRelation::Unknown);
        assert_eq!(query_calls.get(), 1);
    }

    #[test]
    fn live_profile_identity_uses_only_the_accepted_delivery_target() {
        let complete = DeliveryTargetSnapshot::Complete(delivery_target(
            "com.example.Editor",
            41,
            100,
            Some(7),
        ));
        assert_eq!(
            profile_identity_from_delivery_target(&complete),
            FrontmostAppIdentity {
                bundle_id: Some("com.example.Editor".to_string()),
                process_id: Some(41),
            }
        );
        for unavailable in [
            DeliveryTargetSnapshot::Incomplete,
            DeliveryTargetSnapshot::SelfTarget,
            DeliveryTargetSnapshot::Mismatch(DeliveryTargetMismatch {
                kind: DeliveryTargetMismatchKind::PartialIdentityMismatch,
                same_application: false,
                same_process: false,
                transitions: AppTransitionSnapshot {
                    activation_generation: 0,
                    space_generation: 0,
                },
            }),
        ] {
            assert_eq!(
                profile_identity_from_delivery_target(&unavailable),
                FrontmostAppIdentity {
                    bundle_id: None,
                    process_id: None,
                }
            );
        }
    }

    #[test]
    fn delivery_verification_distinguishes_app_process_and_relaunch() {
        let expected = delivery_target("com.example.Editor", 41, 100, Some(7));

        let different_app = delivery_target("com.example.Browser", 42, 200, Some(8));
        let evidence = classify_delivery_target(&expected, Some(&different_app), 999, true);
        assert_eq!(evidence.outcome, VerificationOutcome::DifferentApplication);
        assert!(!evidence.same_application);

        let same_pid_token_different_app = delivery_target("com.example.Browser", 41, 100, Some(8));
        let evidence =
            classify_delivery_target(&expected, Some(&same_pid_token_different_app), 999, true);
        assert_eq!(evidence.outcome, VerificationOutcome::DifferentApplication);
        assert!(evidence.same_process);
        assert!(!evidence.same_process_instance);

        let different_process = delivery_target("com.example.Editor", 42, 200, Some(8));
        let evidence = classify_delivery_target(&expected, Some(&different_process), 999, true);
        assert_eq!(evidence.outcome, VerificationOutcome::DifferentProcess);
        assert!(evidence.same_application);
        assert!(!evidence.same_process);
        assert!(!evidence.same_process_instance);

        let different_process_same_launch = delivery_target("com.example.Editor", 42, 100, Some(8));
        let evidence =
            classify_delivery_target(&expected, Some(&different_process_same_launch), 999, true);
        assert_eq!(evidence.outcome, VerificationOutcome::DifferentProcess);
        assert!(!evidence.same_process_instance);

        let relaunched = delivery_target("com.example.Editor", 41, 200, Some(8));
        let evidence = classify_delivery_target(&expected, Some(&relaunched), 999, true);
        assert_eq!(evidence.outcome, VerificationOutcome::ProcessRelaunched);
        assert!(evidence.same_process);
        assert!(!evidence.same_process_instance);
    }

    #[test]
    fn delivery_verification_fails_closed_for_incomplete_self_and_stale_start() {
        let expected = delivery_target("com.example.Murmur", 41, 100, None);

        let incomplete = verify_delivery_target_with(
            &DeliveryTargetSnapshot::Incomplete,
            999,
            true,
            || panic!("incomplete start must not query current identity"),
            |_| panic!("incomplete start must not sleep"),
        );
        assert_eq!(
            incomplete.outcome,
            VerificationOutcome::StartIdentityIncomplete
        );
        let self_target = verify_delivery_target_with(
            &DeliveryTargetSnapshot::SelfTarget,
            41,
            true,
            || panic!("self start must not query current identity"),
            |_| panic!("self start must not sleep"),
        );
        assert_eq!(self_target.outcome, VerificationOutcome::StartTargetIsSelf);
        assert_eq!(self_target.source, VerificationSource::Native);
        assert!(self_target.current_is_self);
        let stale = verify_delivery_target_with(
            &DeliveryTargetSnapshot::Complete(expected),
            999,
            false,
            || panic!("stale owner must not query current identity"),
            |_| panic!("stale owner must not sleep"),
        );
        assert_eq!(stale.outcome, VerificationOutcome::StaleOwner);
        assert!(!stale.ownership_current);
    }

    #[test]
    fn current_self_target_is_an_immediate_native_mismatch() {
        let expected =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Editor", 41, 100, None));
        let query_calls = Cell::new(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                DeliveryTargetSnapshot::SelfTarget
            },
            |_| panic!("self target must not retry"),
        );

        assert_eq!(evidence.outcome, VerificationOutcome::DifferentApplication);
        assert_eq!(evidence.source, VerificationSource::Native);
        assert!(evidence.current_is_self);
        assert_eq!(query_calls.get(), 1);
    }

    #[test]
    fn delivery_verification_retries_only_unavailable_lookups() {
        let expected =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Editor", 41, 100, None));
        let current = expected.clone();
        let query_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                if query_calls.get() == 3 {
                    current.clone()
                } else {
                    DeliveryTargetSnapshot::Incomplete
                }
            },
            |delay| {
                assert_eq!(delay, DELIVERY_VERIFICATION_RETRY_DELAY);
                sleep_calls.set(sleep_calls.get() + 1);
            },
        );
        assert_eq!(evidence.outcome, VerificationOutcome::Verified);
        assert_eq!(evidence.retry_count, 2);
        assert_eq!(query_calls.get(), 3);
        assert_eq!(sleep_calls.get(), 2);

        let mismatch =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Other", 42, 200, None));
        query_calls.set(0);
        sleep_calls.set(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                mismatch.clone()
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );
        assert_eq!(evidence.outcome, VerificationOutcome::DifferentApplication);
        assert_eq!(query_calls.get(), 1);
        assert_eq!(sleep_calls.get(), 0);
    }

    #[test]
    fn delivery_verification_stops_after_retry_finds_explicit_mismatch() {
        let expected =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Editor", 41, 100, None));
        let mismatch =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Other", 42, 200, None));
        let query_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                match query_calls.get() {
                    1 => DeliveryTargetSnapshot::Incomplete,
                    2 => mismatch.clone(),
                    _ => panic!("explicit mismatch must stop further queries"),
                }
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(evidence.outcome, VerificationOutcome::DifferentApplication);
        assert_eq!(evidence.retry_count, 1);
        assert_eq!(query_calls.get(), 2);
        assert_eq!(sleep_calls.get(), 1);
    }

    #[test]
    fn unavailable_delivery_lookup_is_bounded() {
        let expected =
            DeliveryTargetSnapshot::Complete(delivery_target("com.example.Editor", 41, 100, None));
        let query_calls = Cell::new(0);
        let sleep_calls = Cell::new(0);
        let evidence = verify_delivery_target_with(
            &expected,
            999,
            true,
            || {
                query_calls.set(query_calls.get() + 1);
                DeliveryTargetSnapshot::Incomplete
            },
            |_| sleep_calls.set(sleep_calls.get() + 1),
        );

        assert_eq!(evidence.outcome, VerificationOutcome::LookupUnavailable);
        assert_eq!(evidence.source, VerificationSource::None);
        assert_eq!(evidence.retry_count, 2);
        assert_eq!(query_calls.get(), 3);
        assert_eq!(sleep_calls.get(), 2);
    }
}
