//! Test-only fault injection for native acceptance of the System Audio relaunch
//! and echo-reduction rows. Both faults enter through the real service paths:
//! the access probe, the capture start, and the generation-scoped echo update.
//!
//! Every hook is gated twice. The `native-host-qa` feature must be compiled in
//! and the process must be the exact QA bundle. Any other build or identity never
//! reads the environment, so these functions are inert in production.
use super::*;

/// The only bundle identity that may arm a hook, shared with the updater fixture.
const QA_BUNDLE: &str = "com.localdictation.gpui-host-qa110";
/// `denied` makes macOS list Murmur as allowed while Core Audio refuses the tap.
const TAP_ENV: &str = "MURMUR_QA_SYSTEM_AUDIO_TAP";
/// Comma-separated `recovering` and `bypassed` echo-reduction events.
const ECHO_ENV: &str = "MURMUR_QA_MEETING_ECHO";
const STEP: Duration = Duration::from_secs(3);
const RECORDING_WAIT: Duration = Duration::from_secs(120);

fn armed_for(feature: bool, identifier: Option<&str>) -> bool {
    feature && identifier == Some(QA_BUNDLE)
}

pub(crate) fn armed() -> bool {
    armed_for(
        cfg!(feature = "native-host-qa"),
        crate::identity::initialized_identifier(),
    )
}

pub(crate) fn setting(name: &str) -> Option<String> {
    if armed() {
        std::env::var(name).ok()
    } else {
        None
    }
}

fn tap_refused_for(value: Option<&str>) -> bool {
    value == Some("denied")
}

/// True when the QA bundle asked for an authorized-but-refused System Audio tap.
pub(super) fn tap_refused() -> bool {
    tap_refused_for(setting(TAP_ENV).as_deref())
}

/// The probe result a refused tap produces. `resolve_access` then derives
/// `needs_relaunch` from the authorized TCC state exactly as it does for the
/// real probe.
pub(super) fn refused_tap() -> SystemAudioAccess {
    SystemAudioAccess {
        permission: SystemAudioPermissionState::Denied,
        ..SystemAudioAccess::default()
    }
}

fn echo_script_for(
    value: Option<&str>,
    requested: EchoCancellationMode,
) -> Vec<EchoCancellationStatus> {
    if !matches!(requested, EchoCancellationMode::Enabled) {
        return Vec::new();
    }
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|step| match step.trim() {
            "recovering" => Some(EchoCancellationStatus::Recovering {
                reason: EchoCancellationBypassReason::ProcessorFailed,
                episode: 1,
                attempt: 1,
                max_attempts: MAX_ECHO_CANCELLATION_RECOVERY_ATTEMPTS,
            }),
            "bypassed" => Some(EchoCancellationStatus::Bypassed {
                reason: EchoCancellationBypassReason::ProcessorFailed,
            }),
            _ => None,
        })
        .collect()
}

/// Replays the requested echo events once this generation is recording. Each
/// event goes through `update_echo_cancellation`, which drops it if the meeting
/// has ended or a newer generation owns the coordinator.
pub(super) fn inject_echo(
    coordinator: &MeetingCoordinator,
    app: &crate::host::Context,
    config: &MeetingCaptureConfig,
) {
    let script = echo_script_for(setting(ECHO_ENV).as_deref(), config.echo_cancellation);
    if script.is_empty() {
        return;
    }
    let coordinator = coordinator.clone();
    let app = app.clone();
    let generation = config.generation;
    let _ = thread::Builder::new()
        .name(format!("murmur-meeting-qa-echo-{generation}"))
        .spawn(move || {
            let deadline = Instant::now() + RECORDING_WAIT;
            loop {
                let status = coordinator.status();
                if status.generation != generation || Instant::now() >= deadline {
                    return;
                }
                match status.phase {
                    MeetingRuntimePhase::Recording => break,
                    MeetingRuntimePhase::Starting => thread::sleep(Duration::from_millis(100)),
                    _ => return,
                }
            }
            for step in script {
                thread::sleep(STEP);
                coordinator.update_echo_cancellation(&app, generation, step);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_arm_only_for_the_qa_feature_and_exact_bundle() {
        assert!(armed_for(true, Some(QA_BUNDLE)));
        assert!(!armed_for(false, Some(QA_BUNDLE)));
        assert!(!armed_for(true, Some(crate::identity::PRODUCTION)));
        assert!(!armed_for(true, None));
    }

    #[test]
    fn refused_tap_resolves_to_a_relaunch_receipt_through_the_real_access_path() {
        assert!(tap_refused_for(Some("denied")));
        assert!(!tap_refused_for(Some("granted")));
        assert!(!tap_refused_for(None));
        let access = resolve_access(true, refused_tap());
        assert_eq!(access.permission, SystemAudioPermissionState::Denied);
        assert!(!access.capture_ready);
        assert!(access.needs_relaunch);
    }

    #[test]
    fn echo_script_needs_enabled_echo_reduction_and_known_events() {
        let steps = echo_script_for(
            Some("recovering, bypassed,nonsense"),
            EchoCancellationMode::Enabled,
        );
        assert_eq!(steps.len(), 2);
        assert!(matches!(
            steps[0],
            EchoCancellationStatus::Recovering { attempt: 1, .. }
        ));
        assert!(matches!(steps[1], EchoCancellationStatus::Bypassed { .. }));
        assert!(echo_script_for(Some("bypassed"), EchoCancellationMode::Disabled).is_empty());
        assert!(echo_script_for(None, EchoCancellationMode::Enabled).is_empty());
    }
}
