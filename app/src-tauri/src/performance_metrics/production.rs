//! Bounded, content-free evidence for local production-version comparisons.
use super::types::{MeasurementV1, RunOutcomeV1, UnavailableReasonV1};
use crate::audio::{AudioFailureKind, AudioStartupDiagnostic, HelperPreparationOutcome};
use crate::dictation_context::{DictationContextSnapshot, VocabularySource};
use murmur_capture_helper_protocol::{CaptureBackend, CaptureSetupStep};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionRunV1 {
    #[serde(deserialize_with = "version_one")]
    pub schema_version: u32,
    pub cohort: ProductionCohortV1,
    pub capture: ProductionCaptureV1,
}

fn version_one<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == 1 {
        Ok(version)
    } else {
        Err(serde::de::Error::custom("unsupported production schema"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicrophoneSelectionV1 {
    SystemDefault,
    Explicit,
    SmartAuto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicrophoneKindV1 {
    BuiltIn,
    External,
    Bluetooth,
    Virtual,
    Continuity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProductionArchitectureV1 {
    Aarch64,
    X86_64,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProductionBuildModeV1 {
    Development,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionCohortV1 {
    #[serde(deserialize_with = "machine_id")]
    pub machine_id: String,
    #[serde(deserialize_with = "os_version")]
    pub os_version: Option<String>,
    pub architecture: ProductionArchitectureV1,
    pub build_mode: ProductionBuildModeV1,
    pub microphone_selection: MicrophoneSelectionV1,
    pub microphone_kind: Option<MicrophoneKindV1>,
    #[serde(deserialize_with = "configuration_key")]
    pub configuration_key: Option<String>,
}

fn machine_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    uuid::Uuid::parse_str(&value)
        .ok()
        .filter(|id| id.to_string() == value)
        .map(|_| value)
        .ok_or_else(|| serde::de::Error::custom("invalid local machine identity"))
}
pub(crate) fn valid_os_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
fn os_version<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    if value.as_deref().is_none_or(valid_os_version) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("invalid OS version"))
    }
}
fn configuration_key<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    if value.as_ref().is_none_or(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("invalid configuration identity"))
    }
}

fn optional_duration<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<u64>::deserialize(d)?;
    if value.is_none_or(|value| value <= 9_007_199_254_740_991) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("invalid capture duration"))
    }
}

/// Optional on historical V1 rows; every new duration has an explicit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionGestureV1 {
    pub press_to_accepted_ms: MeasurementV1<u64>,
    pub double_tap_gap_ms: MeasurementV1<u64>,
    pub press_to_first_pcm_ms: MeasurementV1<u64>,
    pub accepted_to_first_pcm_ms: MeasurementV1<u64>,
    pub press_to_ready_ms: MeasurementV1<u64>,
    pub accepted_to_ready_ms: MeasurementV1<u64>,
}
impl ProductionGestureV1 {
    fn new(
        press: Option<u64>,
        gap: Option<u64>,
        expected: bool,
        double_tap_expected: bool,
    ) -> Self {
        let pending = if expected {
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples,
            }
        } else {
            MeasurementV1::NotApplicable
        };
        Self {
            press_to_accepted_ms: press
                .map(MeasurementV1::measured)
                .unwrap_or_else(|| pending.clone()),
            double_tap_gap_ms: gap.map(MeasurementV1::measured).unwrap_or_else(|| {
                if double_tap_expected {
                    MeasurementV1::Unavailable {
                        reason: UnavailableReasonV1::NoSamples,
                    }
                } else {
                    MeasurementV1::NotApplicable
                }
            }),
            press_to_first_pcm_ms: pending.clone(),
            accepted_to_first_pcm_ms: pending.clone(),
            press_to_ready_ms: pending.clone(),
            accepted_to_ready_ms: pending,
        }
    }
}

/// First-attempt silent first-tap helper preparation outcome (#933).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelperPreparationV1 {
    Adopted,
    NotPrepared,
    StillPreparing,
    IdentityMismatch,
    Expired,
}
impl From<HelperPreparationOutcome> for HelperPreparationV1 {
    fn from(outcome: HelperPreparationOutcome) -> Self {
        match outcome {
            HelperPreparationOutcome::Adopted => Self::Adopted,
            HelperPreparationOutcome::NotPrepared => Self::NotPrepared,
            HelperPreparationOutcome::StillPreparing => Self::StillPreparing,
            HelperPreparationOutcome::IdentityMismatch => Self::IdentityMismatch,
            HelperPreparationOutcome::Expired => Self::Expired,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionCaptureV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<ProductionGestureV1>,
    #[serde(deserialize_with = "optional_duration")]
    pub helper_resolve_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub helper_signature_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub helper_spawn_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub stream_open_ms: Option<u64>,
    // Split of the latest attempt's stream open (#933), absent on older rows.
    // `streamConfigureMs` sums every configuration step between unit/stream
    // creation and start; each value describes the latest attempt only.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_duration"
    )]
    pub device_resolution_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_duration"
    )]
    pub audio_unit_new_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_duration"
    )]
    pub audio_unit_initialize_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_duration"
    )]
    pub stream_configure_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_duration"
    )]
    pub stream_start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper_preparation: Option<HelperPreparationV1>,
    #[serde(deserialize_with = "optional_duration")]
    pub first_callback_wait_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub first_pcm_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub ready_ms: Option<u64>,
    #[serde(deserialize_with = "optional_duration")]
    pub stop_to_worker_exit_ms: Option<u64>,
    pub backend: Option<CaptureBackend>,
    pub fallback_attempted: Option<bool>,
    pub fallback_succeeded: Option<bool>,
    pub failure_kind: Option<AudioFailureKind>,
    pub worker_invariant_violation: Option<bool>,
    pub zero_sample_success: Option<bool>,
}

impl ProductionCaptureV1 {
    pub(crate) fn observe(&mut self, event: AudioStartupDiagnostic) {
        match event {
            AudioStartupDiagnostic::GestureAccepted {
                gesture_expected,
                double_tap_expected,
                press_to_accepted_ms,
                double_tap_gap_ms,
            } => {
                self.gesture = Some(ProductionGestureV1::new(
                    press_to_accepted_ms,
                    double_tap_gap_ms,
                    gesture_expected,
                    double_tap_expected,
                ));
            }
            AudioStartupDiagnostic::GesturePcm {
                press_to_first_pcm_ms,
                accepted_to_first_pcm_ms,
            } => {
                if let Some(g) = self.gesture.as_mut() {
                    g.press_to_first_pcm_ms = MeasurementV1::measured(press_to_first_pcm_ms);
                    g.accepted_to_first_pcm_ms = MeasurementV1::measured(accepted_to_first_pcm_ms);
                }
            }
            AudioStartupDiagnostic::GestureReady {
                press_to_ready_ms,
                accepted_to_ready_ms,
            } => {
                if let Some(g) = self.gesture.as_mut() {
                    g.press_to_ready_ms = MeasurementV1::measured(press_to_ready_ms);
                    g.accepted_to_ready_ms = MeasurementV1::measured(accepted_to_ready_ms);
                }
            }
            AudioStartupDiagnostic::BackendPlan { .. } => {
                self.fallback_attempted = Some(false);
            }
            AudioStartupDiagnostic::AttemptStarted { attempt_index, .. } => {
                if attempt_index > 1 {
                    self.fallback_attempted = Some(true);
                }
                // The split describes the latest attempt, like streamOpenMs.
                self.device_resolution_ms = None;
                self.audio_unit_new_ms = None;
                self.audio_unit_initialize_ms = None;
                self.stream_configure_ms = None;
                self.stream_start_ms = None;
            }
            AudioStartupDiagnostic::FirstPcm {
                backend,
                attempt_start_to_first_pcm_ms,
                ..
            } => {
                self.backend = Some(backend);
                self.first_pcm_ms = Some(attempt_start_to_first_pcm_ms);
                if self.fallback_attempted == Some(true) {
                    self.fallback_succeeded = Some(true);
                }
            }
            AudioStartupDiagnostic::CycleReady {
                cycle_start_to_first_pcm_ms,
            } => {
                self.ready_ms = Some(cycle_start_to_first_pcm_ms);
            }
            AudioStartupDiagnostic::AttemptFailed { failure_kind, .. } => {
                self.failure_kind = Some(failure_kind);
                if self.fallback_attempted == Some(true) {
                    self.fallback_succeeded = Some(false);
                }
                if failure_kind == AudioFailureKind::TerminationUnconfirmed {
                    self.worker_invariant_violation = Some(true);
                }
            }
            AudioStartupDiagnostic::HelperReady {
                resolve_ms,
                signature_ms,
                spawn_ms,
            } => {
                self.helper_resolve_ms = Some(
                    self.helper_resolve_ms
                        .unwrap_or(0)
                        .saturating_add(resolve_ms),
                );
                self.helper_signature_ms = Some(
                    self.helper_signature_ms
                        .unwrap_or(0)
                        .saturating_add(signature_ms),
                );
                self.helper_spawn_ms =
                    Some(self.helper_spawn_ms.unwrap_or(0).saturating_add(spawn_ms));
            }
            AudioStartupDiagnostic::WorkerStopped { elapsed_ms } => {
                self.stop_to_worker_exit_ms = Some(elapsed_ms);
                self.worker_invariant_violation.get_or_insert(false);
            }
            AudioStartupDiagnostic::WorkerInvariantViolation => {
                self.worker_invariant_violation = Some(true);
            }
            AudioStartupDiagnostic::PhaseDuration { phase, elapsed_ms } => match phase {
                crate::audio::AudioInitPhase::StreamBuild => {
                    self.stream_open_ms = Some(elapsed_ms);
                }
                crate::audio::AudioInitPhase::FirstBufferWait => {
                    self.first_callback_wait_ms = Some(elapsed_ms);
                }
                _ => {}
            },
            AudioStartupDiagnostic::SetupStep { .. }
            | AudioStartupDiagnostic::RecordingStopIssued => {}
            AudioStartupDiagnostic::SetupStepDuration { step, elapsed_ms } => match step {
                CaptureSetupStep::DeviceResolution => {
                    self.device_resolution_ms = Some(elapsed_ms);
                }
                CaptureSetupStep::AudioUnitNew => self.audio_unit_new_ms = Some(elapsed_ms),
                CaptureSetupStep::AudioUnitInit => self.audio_unit_initialize_ms = Some(elapsed_ms),
                CaptureSetupStep::EnableInputIo
                | CaptureSetupStep::DisableOutputIo
                | CaptureSetupStep::SetCurrentDevice
                | CaptureSetupStep::FormatConfiguration
                | CaptureSetupStep::CallbackInstallation
                | CaptureSetupStep::DefaultConfig
                | CaptureSetupStep::StreamBuild => {
                    self.stream_configure_ms = Some(
                        self.stream_configure_ms
                            .unwrap_or(0)
                            .saturating_add(elapsed_ms),
                    );
                }
                CaptureSetupStep::StreamStart => self.stream_start_ms = Some(elapsed_ms),
                // The legacy coarse bracket overlaps the split steps; the first
                // callback wait is already firstCallbackWaitMs.
                CaptureSetupStep::AudioUnitCreation
                | CaptureSetupStep::AwaitingFirstCallback
                | CaptureSetupStep::SystemTapCreate
                | CaptureSetupStep::AggregateDeviceCreate
                | CaptureSetupStep::IoProcCreate
                | CaptureSetupStep::IoProcStart => {}
            },
            AudioStartupDiagnostic::HelperPreparation { outcome } => {
                // Recorded once, for the dictation's first capture attempt.
                self.helper_preparation.get_or_insert(outcome.into());
            }
        }
    }
}

struct PendingProduction {
    run: ProductionRunV1,
    completed_run_id: Option<String>,
    recording_stop_issued: bool,
    context_key: Option<String>,
    backend_plan: Option<String>,
    samples_empty: Option<bool>,
}
impl PendingProduction {
    fn update_configuration(&mut self) {
        self.run.cohort.configuration_key = self
            .context_key
            .as_ref()
            .zip(self.backend_plan.as_ref())
            .map(|(context, plan)| {
                format!(
                    "{:x}",
                    Sha256::digest(format!("{context}|{plan}").as_bytes())
                )
            });
    }
}

#[derive(Default)]
pub(crate) struct ProductionAccumulator {
    pub machine_id: Option<String>,
    pub os_version: Option<String>,
    runs: BTreeMap<u64, PendingProduction>,
}
impl ProductionAccumulator {
    pub(crate) fn register(
        &mut self,
        recording_id: u64,
        selection: MicrophoneSelectionV1,
        kind: Option<MicrophoneKindV1>,
    ) {
        let Some(machine_id) = self.machine_id.clone() else {
            return;
        };
        // Bound unfinished observations independently of delivered rows so a
        // quick next recording cannot evict a queued teardown checkpoint.
        while self
            .runs
            .values()
            .filter(|run| run.completed_run_id.is_none())
            .count()
            >= 8
        {
            let Some(id) = self
                .runs
                .iter()
                .find_map(|(id, run)| run.completed_run_id.is_none().then_some(*id))
            else {
                break;
            };
            self.runs.remove(&id);
        }
        self.runs.insert(
            recording_id,
            PendingProduction {
                completed_run_id: None,
                recording_stop_issued: false,
                context_key: None,
                backend_plan: None,
                samples_empty: None,
                run: ProductionRunV1 {
                    schema_version: 1,
                    cohort: ProductionCohortV1 {
                        machine_id,
                        os_version: self.os_version.clone(),
                        architecture: match std::env::consts::ARCH {
                            "aarch64" => ProductionArchitectureV1::Aarch64,
                            "x86_64" => ProductionArchitectureV1::X86_64,
                            _ => ProductionArchitectureV1::Other,
                        },
                        build_mode: if cfg!(debug_assertions) {
                            ProductionBuildModeV1::Development
                        } else {
                            ProductionBuildModeV1::Release
                        },
                        microphone_selection: selection,
                        microphone_kind: kind,
                        configuration_key: None,
                    },
                    capture: ProductionCaptureV1::default(),
                },
            },
        );
    }
    pub(crate) fn context(&mut self, id: u64, context: &DictationContextSnapshot) {
        if let Some(run) = self.runs.get_mut(&id) {
            run.context_key = context_key(context);
            run.update_configuration();
        }
    }
    pub(crate) fn observe(&mut self, id: u64, event: AudioStartupDiagnostic) -> bool {
        if let Some(run) = self.runs.get_mut(&id) {
            if matches!(event, AudioStartupDiagnostic::RecordingStopIssued) {
                // A completed standby run cannot acquire a stop binding later.
                if run.completed_run_id.is_none() {
                    run.recording_stop_issued = true;
                }
                return false;
            }
            if run.completed_run_id.is_some() && !run.recording_stop_issued {
                return false;
            }
            if let AudioStartupDiagnostic::BackendPlan {
                primary,
                fallback,
                source,
            } = event
            {
                run.backend_plan = Some(format!("{primary:?}|{fallback:?}|{source:?}"));
                run.update_configuration();
            }
            let before = run.run.capture.clone();
            run.run.capture.observe(event);
            return before != run.run.capture
                && matches!(
                    event,
                    AudioStartupDiagnostic::GestureReady { .. }
                        | AudioStartupDiagnostic::CycleReady { .. }
                        | AudioStartupDiagnostic::WorkerStopped { .. }
                        | AudioStartupDiagnostic::WorkerInvariantViolation
                        | AudioStartupDiagnostic::AttemptFailed {
                            failure_kind: AudioFailureKind::TerminationUnconfirmed,
                            ..
                        }
                );
        }
        false
    }
    pub(crate) fn failure(&mut self, id: u64, kind: AudioFailureKind) -> bool {
        if let Some(run) = self.runs.get_mut(&id) {
            // InitializationFailed can be forwarded after standby delivery.
            // Only an in-flight run or its own bound cold Stop owns failures.
            if run.completed_run_id.is_some() && !run.recording_stop_issued {
                return false;
            }
            if run.run.capture.failure_kind == Some(kind) {
                return false;
            }
            run.run.capture.failure_kind = Some(kind);
            if kind == AudioFailureKind::TerminationUnconfirmed {
                run.run.capture.worker_invariant_violation = Some(true);
            }
            return true;
        }
        false
    }
    pub(crate) fn snapshot(&self, id: u64, outcome: &RunOutcomeV1) -> Option<ProductionRunV1> {
        let mut run = self.runs.get(&id)?.run.clone();
        if matches!(outcome, RunOutcomeV1::Success) {
            run.capture.zero_sample_success = self.runs.get(&id)?.samples_empty;
        }
        Some(run)
    }
    pub(crate) fn sample_count(&mut self, id: u64, count: usize) {
        if let Some(pending) = self.runs.get_mut(&id) {
            pending.samples_empty = Some(count == 0);
        }
    }

    pub(crate) fn active_snapshot(&self, id: u64) -> Option<ProductionRunV1> {
        let pending = self.runs.get(&id)?;
        pending
            .completed_run_id
            .is_none()
            .then(|| pending.run.clone())
    }
    pub(crate) fn persistence_snapshot(
        &self,
        id: u64,
    ) -> Option<(ProductionRunV1, Option<String>)> {
        let pending = self.runs.get(&id)?;
        // Recheck after the async continuation acquires the operation lock:
        // an in-flight checkpoint can become a completed standby checkpoint.
        if pending.completed_run_id.is_some() && !pending.recording_stop_issued {
            return None;
        }
        Some((pending.run.clone(), pending.completed_run_id.clone()))
    }
    pub(crate) fn completed(&mut self, id: u64, run: &super::PerformanceRunV1) {
        if let Some(pending) = self.runs.get_mut(&id) {
            // Freeze the delivered outcome's derived fields (including real
            // zero-sample evidence), and bind every late write to this UUID.
            if let Some(production) = &run.production {
                pending.run.capture.zero_sample_success = production.capture.zero_sample_success;
            }
            pending.completed_run_id = Some(run.run_id.clone());
        }
        while self
            .runs
            .values()
            .filter(|run| run.completed_run_id.is_some())
            .count()
            > super::repository::MAX_COMPLETED_RUNS
        {
            let Some(id) = self
                .runs
                .iter()
                .find_map(|(id, run)| run.completed_run_id.is_some().then_some(*id))
            else {
                break;
            };
            self.runs.remove(&id);
        }
    }
    pub(crate) fn clear(&mut self) {
        self.runs.clear();
    }
}

fn context_key(context: &DictationContextSnapshot) -> Option<String> {
    let t = &context.transformations;
    // Custom content cannot become a diagnostics fingerprint. Without its exact
    // configuration identity, those runs are explicitly ineligible for a verdict.
    if context
        .transcription
        .prompt
        .as_ref()
        .is_some_and(|prompt| !prompt.is_empty())
        || context.vocabulary.source != VocabularySource::None
        || !t.voice_commands.is_empty()
        || t.correction_matcher
            .as_ref()
            .is_some_and(|matcher| !matcher.has_only_contextual_rules())
        || t.ide_context_enabled
        || context.delivery.save_audio
        || context.delivery.save_transcript
        || context.delivery.mirror_to_notchpill
    {
        return None;
    }
    let language = &context.transcription.language;
    if language.len() > 16
        || !language
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'-')
    {
        return None;
    }
    let safe = format!(
        "production-config-v1|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{}|{}|{:?}|{}|{}|{}|{}|{}|{}|{:?}|{}|{}",
        language,
        context.transcription.vad_sensitivity,
        context.transcription.smart_punctuation,
        t.cleanup_enabled,
        t.cleanup_remove_filler,
        t.cleanup_capitalize,
        t.correction_enabled,
        t.cli_formatting_enabled,
        t.cli_formatting_mode,
        t.smart_formatting_enabled,
        t.spoken_numbers_enabled,
        t.spoken_structure_policy,
        context.delivery.auto_paste,
        context.delivery.paste_delay_ms,
        context.delivery.save_transcript,
        context.delivery.save_audio,
        context.delivery.mirror_to_notchpill,
        context.enabled_command_groups.built_in_voice_commands,
        context.writing_style,
        context.enabled_command_groups.custom_voice_commands,
        context.context_capture.local_project_index
    );
    Some(format!("{:x}", Sha256::digest(safe.as_bytes())))
}

pub(crate) fn local_machine_id(root: &Path) -> Option<String> {
    use std::io::{Read, Write};
    let path = root.join("production-machine-id");
    std::fs::create_dir_all(root).ok()?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let id = uuid::Uuid::new_v4().to_string();
            file.write_all(id.as_bytes()).ok()?;
            Some(id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if path.symlink_metadata().ok()?.file_type().is_symlink() {
                return None;
            }
            let mut text = String::new();
            std::fs::File::open(path)
                .ok()?
                .take(37)
                .read_to_string(&mut text)
                .ok()?;
            uuid::Uuid::parse_str(&text)
                .ok()
                .filter(|id| id.to_string() == text)
                .map(|_| text)
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioBackendOrderSource, AudioInitPhase};
    use crate::dictation_context::{resolve, ResolverInputs, SessionOverrides};

    fn accumulator() -> ProductionAccumulator {
        ProductionAccumulator {
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
            os_version: Some("26.0".to_string()),
            ..ProductionAccumulator::default()
        }
    }
    fn context() -> DictationContextSnapshot {
        resolve(ResolverInputs {
            bundle_id: None,
            site_mode_id: None,
            global: &crate::state::DictationState::default(),
            prompt: None,
            correction_matcher: None,
            ide_context_index: None,
            vocabulary_version: 1,
            voice_commands: Some(Vec::new()),
            session_overrides: SessionOverrides::default(),
        })
    }
    fn plan() -> AudioStartupDiagnostic {
        AudioStartupDiagnostic::BackendPlan {
            primary: CaptureBackend::Auhal,
            fallback: CaptureBackend::Cpal,
            source: AudioBackendOrderSource::Default,
        }
    }

    #[test]
    fn compatible_identity_requires_context_and_observed_plan_in_either_order() {
        let mut first = accumulator();
        first.register(
            1,
            MicrophoneSelectionV1::Explicit,
            Some(MicrophoneKindV1::BuiltIn),
        );
        first.context(1, &context());
        assert!(first
            .active_snapshot(1)
            .unwrap()
            .cohort
            .configuration_key
            .is_none());
        first.observe(1, plan());
        let key = first
            .active_snapshot(1)
            .unwrap()
            .cohort
            .configuration_key
            .unwrap();
        let mut second = accumulator();
        second.register(
            2,
            MicrophoneSelectionV1::Explicit,
            Some(MicrophoneKindV1::BuiltIn),
        );
        second.observe(2, plan());
        second.context(2, &context());
        assert_eq!(
            second.active_snapshot(2).unwrap().cohort.configuration_key,
            Some(key.clone())
        );
        let mut changed = context();
        changed.delivery.paste_delay_ms += 1;
        second.context(2, &changed);
        assert_ne!(
            second.active_snapshot(2).unwrap().cohort.configuration_key,
            Some(key)
        );
    }

    #[test]
    fn custom_content_never_becomes_a_configuration_fingerprint() {
        let mut context = context();
        assert!(context_key(&context).is_some());
        context.transformations.correction_matcher = Some(std::sync::Arc::new(
            crate::correction::CorrectionMatcher::build(&[], &[], true, false),
        ));
        assert!(
            context_key(&context).is_some(),
            "built-in contextual rules are content-free"
        );
        context.transformations.correction_matcher = Some(std::sync::Arc::new(
            crate::correction::CorrectionMatcher::build(
                &["PrivateProjectSentinel".into()],
                &[],
                true,
                false,
            ),
        ));
        assert!(context_key(&context).is_none());
        context.transformations.correction_matcher = None;
        context.transcription.prompt = Some("private prompt sentinel".into());
        assert!(context_key(&context).is_none());
    }

    #[test]
    fn fallback_errors_survive_success_and_missing_values_never_become_zero() {
        let mut state = accumulator();
        state.register(4, MicrophoneSelectionV1::SystemDefault, None);
        let initial = state.active_snapshot(4).unwrap();
        assert_eq!(initial.capture.ready_ms, None);
        assert_eq!(initial.capture.fallback_attempted, None);
        state.observe(4, plan());
        state.observe(
            4,
            AudioStartupDiagnostic::AttemptFailed {
                backend: CaptureBackend::Auhal,
                resolution_pass: 1,
                attempt_index: 1,
                active_elapsed_ms: 2000,
                failure_kind: AudioFailureKind::FirstBufferTimeout,
                failure_phase: AudioInitPhase::FirstBufferWait,
            },
        );
        state.observe(
            4,
            AudioStartupDiagnostic::AttemptStarted {
                backend: CaptureBackend::Cpal,
                resolution_pass: 1,
                attempt_index: 2,
                attempt_budget_ms: 1000,
            },
        );
        state.observe(
            4,
            AudioStartupDiagnostic::FirstPcm {
                backend: CaptureBackend::Cpal,
                resolution_pass: 1,
                attempt_index: 2,
                attempt_start_to_first_pcm_ms: 14,
                active_elapsed_ms: 20,
            },
        );
        let snapshot = state.snapshot(4, &RunOutcomeV1::Success).unwrap();
        assert_eq!(snapshot.capture.fallback_attempted, Some(true));
        assert_eq!(snapshot.capture.fallback_succeeded, Some(true));
        assert_eq!(
            snapshot.capture.failure_kind,
            Some(AudioFailureKind::FirstBufferTimeout)
        );
        assert_eq!(snapshot.capture.zero_sample_success, None);
        state.sample_count(4, 1);
        assert_eq!(
            state
                .snapshot(4, &RunOutcomeV1::Success)
                .unwrap()
                .capture
                .zero_sample_success,
            Some(false)
        );
        state.sample_count(4, 0);
        assert_eq!(
            state
                .snapshot(4, &RunOutcomeV1::Success)
                .unwrap()
                .capture
                .zero_sample_success,
            Some(true)
        );
        assert_eq!(
            state
                .snapshot(4, &RunOutcomeV1::NoSpeech)
                .unwrap()
                .capture
                .zero_sample_success,
            None
        );
    }

    #[test]
    fn observations_are_bounded_exact_owner_and_cannot_revive_after_clear() {
        let mut state = accumulator();
        for id in 1..=9 {
            state.register(id, MicrophoneSelectionV1::SystemDefault, None);
        }
        assert!(state.active_snapshot(1).is_none());
        assert_eq!(state.runs.len(), 8);
        assert!(!state.observe(99, AudioStartupDiagnostic::WorkerInvariantViolation));
        assert!(state.observe(9, AudioStartupDiagnostic::WorkerInvariantViolation));
        assert!(
            !state.observe(9, AudioStartupDiagnostic::WorkerInvariantViolation),
            "duplicate observations do not queue persistence"
        );
        state.observe(9, AudioStartupDiagnostic::WorkerStopped { elapsed_ms: 3 });
        assert_eq!(
            state
                .active_snapshot(9)
                .unwrap()
                .capture
                .worker_invariant_violation,
            Some(true)
        );
        state.clear();
        state.observe(9, plan());
        state.context(9, &context());
        assert!(state.active_snapshot(9).is_none());
    }

    #[test]
    fn nested_contract_rejects_future_versions_content_and_invalid_measurements() {
        let mut state = accumulator();
        state.register(1, MicrophoneSelectionV1::SystemDefault, None);
        let valid = serde_json::to_value(state.active_snapshot(1).unwrap()).unwrap();
        assert!(serde_json::from_value::<ProductionRunV1>(valid.clone()).is_ok());
        for (field, value) in [
            ("schemaVersion", serde_json::json!(2)),
            ("transcript", serde_json::json!("private")),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(serde_json::from_value::<ProductionRunV1>(bad).is_err());
        }
        let mut bad = valid.clone();
        bad["cohort"]["machineId"] = serde_json::json!("private-hostname");
        assert!(serde_json::from_value::<ProductionRunV1>(bad).is_err());
        let mut bad = valid.clone();
        bad["capture"]["helperSpawnMs"] = serde_json::json!(u64::MAX);
        assert!(serde_json::from_value::<ProductionRunV1>(bad).is_err());
        let mut bad = valid;
        bad["capture"]["stderr"] = serde_json::json!("private");
        assert!(serde_json::from_value::<ProductionRunV1>(bad).is_err());
    }

    #[test]
    fn local_identity_survives_restart_without_hardware_or_host_identifiers() {
        let dir = tempfile::tempdir().unwrap();
        let first = local_machine_id(dir.path()).unwrap();
        assert_eq!(local_machine_id(dir.path()), Some(first.clone()));
        assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 4);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("production-machine-id"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[cfg(test)]
mod gesture_contract_tests {
    use super::*;
    #[test]
    fn zero_is_measured_and_missing_samples_are_unavailable() {
        let mut capture = ProductionCaptureV1::default();
        capture.observe(AudioStartupDiagnostic::GestureAccepted {
            press_to_accepted_ms: Some(0),
            double_tap_gap_ms: None,
            gesture_expected: true,
            double_tap_expected: false,
        });
        let g = capture.gesture.as_ref().unwrap();
        assert_eq!(g.press_to_accepted_ms, MeasurementV1::measured(0));
        assert_eq!(g.double_tap_gap_ms, MeasurementV1::NotApplicable);
        assert_eq!(
            g.press_to_first_pcm_ms,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples
            }
        );
        capture.observe(AudioStartupDiagnostic::GesturePcm {
            press_to_first_pcm_ms: 163,
            accepted_to_first_pcm_ms: 163,
        });
        capture.observe(AudioStartupDiagnostic::GestureReady {
            press_to_ready_ms: 201,
            accepted_to_ready_ms: 201,
        });
        let roundtrip: ProductionCaptureV1 =
            serde_json::from_str(&serde_json::to_string(&capture).unwrap()).unwrap();
        assert_eq!(
            roundtrip.gesture.unwrap().press_to_ready_ms,
            MeasurementV1::measured(201)
        );
    }
    #[test]
    fn historical_capture_without_gesture_is_readable() {
        let capture = ProductionCaptureV1::default();
        let value = serde_json::to_value(&capture).unwrap();
        assert!(value.get("gesture").is_none());
        assert_eq!(
            serde_json::from_value::<ProductionCaptureV1>(value).unwrap(),
            capture
        );
    }
    #[test]
    fn missing_keyboard_stamp_is_unavailable_not_a_toolbar_measurement() {
        let g = ProductionGestureV1::new(None, None, true, true);
        assert_eq!(
            g.double_tap_gap_ms,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples
            }
        );
        assert_eq!(
            g.press_to_accepted_ms,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples
            }
        );
        assert_eq!(
            g.press_to_ready_ms,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples
            }
        );
    }
    #[test]
    fn non_keyboard_start_is_not_applicable() {
        let g = ProductionGestureV1::new(None, None, false, false);
        assert_eq!(g.press_to_first_pcm_ms, MeasurementV1::NotApplicable);
        assert_eq!(g.press_to_ready_ms, MeasurementV1::NotApplicable);
    }
}

#[cfg(test)]
mod stream_open_split_tests {
    use super::*;

    fn step(step: CaptureSetupStep, elapsed_ms: u64) -> AudioStartupDiagnostic {
        AudioStartupDiagnostic::SetupStepDuration { step, elapsed_ms }
    }
    fn attempt(attempt_index: u8) -> AudioStartupDiagnostic {
        AudioStartupDiagnostic::AttemptStarted {
            backend: CaptureBackend::Auhal,
            resolution_pass: 1,
            attempt_index,
            attempt_budget_ms: 1000,
        }
    }

    #[test]
    fn setup_steps_split_stream_open_for_the_latest_attempt() {
        let mut capture = ProductionCaptureV1::default();
        capture.observe(attempt(1));
        for (s, ms) in [
            (CaptureSetupStep::DeviceResolution, 2),
            (CaptureSetupStep::AudioUnitNew, 116),
            (CaptureSetupStep::AudioUnitInit, 4),
            (CaptureSetupStep::EnableInputIo, 1),
            (CaptureSetupStep::DisableOutputIo, 1),
            (CaptureSetupStep::SetCurrentDevice, 9),
            (CaptureSetupStep::FormatConfiguration, 3),
            (CaptureSetupStep::CallbackInstallation, 1),
            (CaptureSetupStep::StreamStart, 30),
            (CaptureSetupStep::AwaitingFirstCallback, 13),
            (CaptureSetupStep::AudioUnitCreation, 500),
        ] {
            capture.observe(step(s, ms));
        }
        assert_eq!(capture.device_resolution_ms, Some(2));
        assert_eq!(capture.audio_unit_new_ms, Some(116));
        assert_eq!(capture.audio_unit_initialize_ms, Some(4));
        let roundtrip: ProductionCaptureV1 =
            serde_json::from_value(serde_json::to_value(&capture).unwrap()).unwrap();
        assert_eq!(roundtrip.audio_unit_initialize_ms, Some(4));
        assert_eq!(capture.stream_configure_ms, Some(15));
        assert_eq!(capture.stream_start_ms, Some(30));

        // A fallback CPAL attempt replaces the split rather than adding to it.
        capture.observe(attempt(2));
        assert_eq!(capture.stream_configure_ms, None);
        for (s, ms) in [
            (CaptureSetupStep::DeviceResolution, 0),
            (CaptureSetupStep::DefaultConfig, 4),
            (CaptureSetupStep::StreamBuild, 6),
            (CaptureSetupStep::StreamStart, 7),
        ] {
            capture.observe(step(s, ms));
        }
        assert_eq!(capture.device_resolution_ms, Some(0), "zero is measured");
        assert_eq!(capture.audio_unit_new_ms, None);
        assert_eq!(capture.audio_unit_initialize_ms, None);
        assert_eq!(capture.stream_configure_ms, Some(10));
        assert_eq!(capture.stream_start_ms, Some(7));
    }

    #[test]
    fn first_attempt_preparation_outcome_is_kept() {
        let mut capture = ProductionCaptureV1::default();
        capture.observe(AudioStartupDiagnostic::HelperPreparation {
            outcome: HelperPreparationOutcome::Adopted,
        });
        capture.observe(AudioStartupDiagnostic::HelperPreparation {
            outcome: HelperPreparationOutcome::NotPrepared,
        });
        assert_eq!(
            capture.helper_preparation,
            Some(HelperPreparationV1::Adopted)
        );
        let value = serde_json::to_value(&capture).unwrap();
        assert_eq!(value["helperPreparation"], serde_json::json!("adopted"));
        for (outcome, text) in [
            (HelperPreparationOutcome::NotPrepared, "notPrepared"),
            (HelperPreparationOutcome::StillPreparing, "stillPreparing"),
            (
                HelperPreparationOutcome::IdentityMismatch,
                "identityMismatch",
            ),
            (HelperPreparationOutcome::Expired, "expired"),
        ] {
            assert_eq!(
                serde_json::to_value(HelperPreparationV1::from(outcome)).unwrap(),
                serde_json::json!(text)
            );
        }
    }

    #[test]
    fn historical_rows_without_split_fields_parse_and_new_fields_validate() {
        let historical = serde_json::json!({
            "helperResolveMs": 1,
            "helperSignatureMs": 15,
            "helperSpawnMs": 6,
            "streamOpenMs": 165,
            "firstCallbackWaitMs": 13,
            "firstPcmMs": 190,
            "readyMs": 218,
            "stopToWorkerExitMs": null,
            "backend": null,
            "fallbackAttempted": false,
            "fallbackSucceeded": null,
            "failureKind": null,
            "workerInvariantViolation": null,
            "zeroSampleSuccess": null
        });
        let parsed: ProductionCaptureV1 = serde_json::from_value(historical.clone()).unwrap();
        assert_eq!(parsed.stream_open_ms, Some(165));
        assert_eq!(parsed.audio_unit_new_ms, None);
        assert_eq!(parsed.audio_unit_initialize_ms, None);
        assert_eq!(parsed.helper_preparation, None);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), historical);

        let mut bad = historical.clone();
        bad["audioUnitNewMs"] = serde_json::json!(u64::MAX);
        assert!(serde_json::from_value::<ProductionCaptureV1>(bad).is_err());
        let mut bad = historical.clone();
        bad["audioUnitInitializeMs"] = serde_json::json!(u64::MAX);
        assert!(serde_json::from_value::<ProductionCaptureV1>(bad).is_err());
        let mut bad = historical;
        bad["helperPreparation"] = serde_json::json!("private");
        assert!(serde_json::from_value::<ProductionCaptureV1>(bad).is_err());
    }
}
