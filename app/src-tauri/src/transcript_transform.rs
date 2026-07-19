//! Ordered, backend-neutral transcript transformations.
//!
//! ASR backends produce text; this module applies the deterministic text stages
//! that follow recognition. Delivery concerns (file persistence, clipboard,
//! paste, history, and stats) deliberately remain outside this module.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use crate::cleanup::CleanupOptions;
use crate::correction::CorrectionMatcher;

pub(crate) const CLEANUP_STAGE: &str = "cleanup";
pub(crate) const VOICE_COMMANDS_STAGE: &str = "voice_commands";
pub(crate) const SMART_CORRECTION_STAGE: &str = "smart_correction";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranscriptSource {
    Live,
    File,
}

impl TranscriptSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::File => "file",
        }
    }
}

/// Privacy-safe switches that select which stages run for one transcript.
///
/// Custom replacements and correction vocabulary are intentionally not stored
/// here because their values can contain user-authored text. Those resources
/// are captured by their typed stage implementations and are never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TranscriptStageConfig {
    pub cleanup_enabled: bool,
    pub cleanup_remove_filler: bool,
    pub cleanup_capitalize: bool,
    pub voice_commands_enabled: bool,
    pub smart_correction_enabled: bool,
}

impl TranscriptStageConfig {
    pub(crate) fn verbatim() -> Self {
        Self {
            cleanup_enabled: false,
            cleanup_remove_filler: false,
            cleanup_capitalize: false,
            voice_commands_enabled: false,
            smart_correction_enabled: false,
        }
    }
}

/// Immutable metadata and stage selection for one transformation pass.
///
/// `context_handle` is deliberately opaque. Issue #245 can populate it with a
/// resolved per-app snapshot identifier without this module knowing how app
/// profiles are selected or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptContext {
    pub session_id: u64,
    pub source: TranscriptSource,
    pub context_handle: Option<String>,
    pub model: String,
    pub language: String,
    pub stages: TranscriptStageConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageFailurePolicy {
    Required,
    OptionalFallback,
}

impl StageFailurePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::OptionalFallback => "optional_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageOutcome {
    Applied,
    Skipped,
    Fallback,
    Failed,
}

impl StageOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Skipped => "skipped",
            Self::Fallback => "fallback",
            Self::Failed => "failed",
        }
    }

    fn code(self) -> u64 {
        match self {
            Self::Applied => 1,
            Self::Skipped => 2,
            Self::Fallback => 3,
            Self::Failed => 4,
        }
    }
}

/// Privacy-safe execution metadata. It never carries transcript text, custom
/// replacement values, correction vocabulary, or stage error details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StageReport {
    pub stage: &'static str,
    pub duration_us: u64,
    pub changed: bool,
    pub outcome: StageOutcome,
    pub failure_policy: StageFailurePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StageError {
    code: &'static str,
}

impl StageError {
    #[cfg(test)]
    fn new(code: &'static str) -> Self {
        Self { code }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptPipelineError {
    pub stage: &'static str,
    pub code: &'static str,
}

impl fmt::Display for TranscriptPipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "required transcript stage '{}' failed ({})",
            self.stage, self.code
        )
    }
}

impl std::error::Error for TranscriptPipelineError {}

/// Internal typed composition seam. This is not a public or third-party plugin
/// API; the application owns every stage and its ordering.
trait TranscriptTransform: Send + Sync {
    fn name(&self) -> &'static str;
    fn failure_policy(&self) -> StageFailurePolicy;
    fn enabled(&self, context: &TranscriptContext) -> bool;
    fn transform(&self, text: &str, context: &TranscriptContext) -> Result<String, StageError>;
}

pub(crate) struct TranscriptPipelineOutput {
    pub text: String,
    pub stages: Vec<StageReport>,
}

impl TranscriptPipelineOutput {
    pub(crate) fn stage_duration_ms(&self, stage: &str) -> u64 {
        self.stages
            .iter()
            .find(|report| report.stage == stage)
            .map_or(0, |report| report.duration_us / 1_000)
    }
}

struct TranscriptPipeline {
    stages: Vec<Box<dyn TranscriptTransform>>,
}

impl TranscriptPipeline {
    fn new(stages: Vec<Box<dyn TranscriptTransform>>) -> Self {
        Self { stages }
    }

    fn standard(resources: TranscriptTransformResources) -> Self {
        Self::new(vec![
            Box::new(CleanupStage),
            Box::new(VoiceCommandsStage {
                custom_commands: resources.custom_commands,
            }),
            Box::new(SmartCorrectionStage {
                matcher: resources.correction_matcher,
            }),
        ])
    }

    fn run(
        &self,
        mut text: String,
        context: &TranscriptContext,
    ) -> Result<TranscriptPipelineOutput, TranscriptPipelineError> {
        let mut reports = Vec::with_capacity(self.stages.len());

        for stage in &self.stages {
            let started = Instant::now();
            let policy = stage.failure_policy();

            if !stage.enabled(context) {
                let report = StageReport {
                    stage: stage.name(),
                    duration_us: started.elapsed().as_micros() as u64,
                    changed: false,
                    outcome: StageOutcome::Skipped,
                    failure_policy: policy,
                };
                log_stage(context, &report);
                reports.push(report);
                continue;
            }

            match stage.transform(&text, context) {
                Ok(transformed) => {
                    let changed = transformed != text;
                    text = transformed;
                    let report = StageReport {
                        stage: stage.name(),
                        duration_us: started.elapsed().as_micros() as u64,
                        changed,
                        outcome: StageOutcome::Applied,
                        failure_policy: policy,
                    };
                    log_stage(context, &report);
                    reports.push(report);
                }
                Err(_error) if policy == StageFailurePolicy::OptionalFallback => {
                    let report = StageReport {
                        stage: stage.name(),
                        duration_us: started.elapsed().as_micros() as u64,
                        changed: false,
                        outcome: StageOutcome::Fallback,
                        failure_policy: policy,
                    };
                    log_stage(context, &report);
                    reports.push(report);
                }
                Err(error) => {
                    let report = StageReport {
                        stage: stage.name(),
                        duration_us: started.elapsed().as_micros() as u64,
                        changed: false,
                        outcome: StageOutcome::Failed,
                        failure_policy: policy,
                    };
                    log_stage(context, &report);
                    return Err(TranscriptPipelineError {
                        stage: stage.name(),
                        code: error.code,
                    });
                }
            }
        }

        Ok(TranscriptPipelineOutput {
            text,
            stages: reports,
        })
    }
}

fn log_stage(context: &TranscriptContext, report: &StageReport) {
    tracing::info!(
        target: "pipeline",
        session_id = context.session_id,
        source = context.source.as_str(),
        source_file = context.source == TranscriptSource::File,
        context_handle_present = context.context_handle.is_some(),
        model = context.model.as_str(),
        language = context.language.as_str(),
        stage = report.stage,
        duration_us = report.duration_us,
        changed = report.changed,
        outcome = report.outcome.as_str(),
        outcome_code = report.outcome.code(),
        failure_policy = report.failure_policy.as_str(),
        required = report.failure_policy == StageFailurePolicy::Required,
        "transcript_transform_stage: {}",
        report.stage
    );
}

pub(crate) struct TranscriptTransformResources {
    pub custom_commands: Vec<(String, String)>,
    pub correction_matcher: Option<Arc<CorrectionMatcher>>,
}

impl TranscriptTransformResources {
    pub(crate) fn empty() -> Self {
        Self {
            custom_commands: Vec::new(),
            correction_matcher: None,
        }
    }
}

/// The single authoritative entry point for post-recognition text transforms.
pub(crate) fn transform_transcript(
    text: String,
    context: &TranscriptContext,
    resources: TranscriptTransformResources,
) -> Result<TranscriptPipelineOutput, TranscriptPipelineError> {
    TranscriptPipeline::standard(resources).run(text, context)
}

struct CleanupStage;

impl TranscriptTransform for CleanupStage {
    fn name(&self) -> &'static str {
        CLEANUP_STAGE
    }

    fn failure_policy(&self) -> StageFailurePolicy {
        StageFailurePolicy::Required
    }

    fn enabled(&self, context: &TranscriptContext) -> bool {
        context.stages.cleanup_enabled
    }

    fn transform(&self, text: &str, context: &TranscriptContext) -> Result<String, StageError> {
        if text.trim().is_empty() {
            return Ok(text.to_string());
        }
        Ok(crate::cleanup::clean_transcript(
            text,
            CleanupOptions {
                remove_filler: context.stages.cleanup_remove_filler,
                capitalize: context.stages.cleanup_capitalize,
            },
        ))
    }
}

struct VoiceCommandsStage {
    custom_commands: Vec<(String, String)>,
}

impl TranscriptTransform for VoiceCommandsStage {
    fn name(&self) -> &'static str {
        VOICE_COMMANDS_STAGE
    }

    fn failure_policy(&self) -> StageFailurePolicy {
        StageFailurePolicy::Required
    }

    fn enabled(&self, context: &TranscriptContext) -> bool {
        context.stages.voice_commands_enabled
    }

    fn transform(&self, text: &str, _context: &TranscriptContext) -> Result<String, StageError> {
        Ok(crate::voice_commands::apply_voice_commands_with_custom(
            text,
            true,
            &self.custom_commands,
        ))
    }
}

struct SmartCorrectionStage {
    matcher: Option<Arc<CorrectionMatcher>>,
}

impl TranscriptTransform for SmartCorrectionStage {
    fn name(&self) -> &'static str {
        SMART_CORRECTION_STAGE
    }

    fn failure_policy(&self) -> StageFailurePolicy {
        // Correction is an enhancement. If a future matcher implementation can
        // fail, users should still receive the preceding deterministic output.
        StageFailurePolicy::OptionalFallback
    }

    fn enabled(&self, context: &TranscriptContext) -> bool {
        context.stages.smart_correction_enabled
    }

    fn transform(&self, text: &str, _context: &TranscriptContext) -> Result<String, StageError> {
        if text.trim().is_empty() {
            return Ok(text.to_string());
        }
        match &self.matcher {
            Some(matcher) if !matcher.is_empty() => Ok(matcher.apply(text)),
            _ => Ok(text.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_context(stages: TranscriptStageConfig) -> TranscriptContext {
        TranscriptContext {
            session_id: 7,
            source: TranscriptSource::Live,
            context_handle: None,
            model: "test-model".to_string(),
            language: "en".to_string(),
            stages,
        }
    }

    fn all_stages() -> TranscriptStageConfig {
        TranscriptStageConfig {
            cleanup_enabled: true,
            cleanup_remove_filler: true,
            cleanup_capitalize: true,
            voice_commands_enabled: true,
            smart_correction_enabled: true,
        }
    }

    fn correction_matcher() -> Arc<CorrectionMatcher> {
        Arc::new(CorrectionMatcher::build(
            &["useEffect".to_string()],
            &[],
            false,
            false,
        ))
    }

    fn resources(with_matcher: bool) -> TranscriptTransformResources {
        TranscriptTransformResources {
            custom_commands: vec![("my email".to_string(), "test@example.com".to_string())],
            correction_matcher: with_matcher.then(correction_matcher),
        }
    }

    fn legacy_transform(
        text: &str,
        stages: TranscriptStageConfig,
        resources: &TranscriptTransformResources,
    ) -> String {
        let text = if stages.cleanup_enabled && !text.trim().is_empty() {
            crate::cleanup::clean_transcript(
                text,
                CleanupOptions {
                    remove_filler: stages.cleanup_remove_filler,
                    capitalize: stages.cleanup_capitalize,
                },
            )
        } else {
            text.to_string()
        };
        let text = crate::voice_commands::apply_voice_commands_with_custom(
            &text,
            stages.voice_commands_enabled,
            &resources.custom_commands,
        );
        if stages.smart_correction_enabled && !text.trim().is_empty() {
            match &resources.correction_matcher {
                Some(matcher) if !matcher.is_empty() => matcher.apply(&text),
                _ => text,
            }
        } else {
            text
        }
    }

    #[test]
    fn representative_live_fixtures_match_legacy_output_byte_for_byte() {
        let fixture_configs = [
            all_stages(),
            TranscriptStageConfig {
                smart_correction_enabled: false,
                ..all_stages()
            },
            TranscriptStageConfig {
                cleanup_enabled: false,
                ..all_stages()
            },
            TranscriptStageConfig {
                voice_commands_enabled: false,
                ..all_stages()
            },
        ];
        let fixtures = [
            "um hello new line use effect my email",
            "um the the cat , uh world .",
            "done period new line my email",
            "  preserve   raw spacing  ",
            "",
        ];

        for stages in fixture_configs {
            for fixture in fixtures {
                let expected_resources = resources(true);
                let expected = legacy_transform(fixture, stages, &expected_resources);
                let actual = transform_transcript(
                    fixture.to_string(),
                    &live_context(stages),
                    resources(true),
                )
                .unwrap();
                assert_eq!(
                    actual.text.as_bytes(),
                    expected.as_bytes(),
                    "fixture: {fixture:?}"
                );
            }
        }
    }

    #[test]
    fn standard_pipeline_reports_cleanup_then_commands_then_correction() {
        let output = transform_transcript(
            "raw".to_string(),
            &live_context(all_stages()),
            resources(true),
        )
        .unwrap();
        assert_eq!(
            output
                .stages
                .iter()
                .map(|report| report.stage)
                .collect::<Vec<_>>(),
            vec![CLEANUP_STAGE, VOICE_COMMANDS_STAGE, SMART_CORRECTION_STAGE]
        );
    }

    #[test]
    fn cleanup_stage_can_be_enabled_independently() {
        let stages = TranscriptStageConfig {
            cleanup_enabled: true,
            cleanup_remove_filler: true,
            cleanup_capitalize: true,
            voice_commands_enabled: false,
            smart_correction_enabled: false,
        };
        let output = transform_transcript(
            "um the the cat , world .".to_string(),
            &live_context(stages),
            TranscriptTransformResources::empty(),
        )
        .unwrap();
        assert_eq!(output.text, "The cat, world.");
    }

    #[test]
    fn voice_commands_stage_can_be_enabled_independently() {
        let stages = TranscriptStageConfig {
            cleanup_enabled: false,
            cleanup_remove_filler: false,
            cleanup_capitalize: false,
            voice_commands_enabled: true,
            smart_correction_enabled: false,
        };
        let output = transform_transcript(
            "hello new line my email".to_string(),
            &live_context(stages),
            resources(false),
        )
        .unwrap();
        assert_eq!(output.text, "hello\ntest@example.com");
    }

    #[test]
    fn smart_correction_stage_can_be_enabled_independently() {
        let stages = TranscriptStageConfig {
            cleanup_enabled: false,
            cleanup_remove_filler: false,
            cleanup_capitalize: false,
            voice_commands_enabled: false,
            smart_correction_enabled: true,
        };
        let output = transform_transcript(
            "use effect".to_string(),
            &live_context(stages),
            resources(true),
        )
        .unwrap();
        assert_eq!(output.text, "useEffect");
    }

    #[test]
    fn file_context_preserves_raw_text_and_reports_skipped_stages() {
        let context = TranscriptContext {
            session_id: 11,
            source: TranscriptSource::File,
            context_handle: None,
            model: "test-model".to_string(),
            language: "auto".to_string(),
            stages: TranscriptStageConfig::verbatim(),
        };
        let raw = "um hello new line use effect   ";
        let output = transform_transcript(raw.to_string(), &context, resources(true)).unwrap();
        assert_eq!(output.text.as_bytes(), raw.as_bytes());
        assert_eq!(output.stages.len(), 3);
        assert!(output
            .stages
            .iter()
            .all(|stage| stage.outcome == StageOutcome::Skipped));
    }

    struct AppendStage {
        name: &'static str,
        suffix: &'static str,
    }

    impl TranscriptTransform for AppendStage {
        fn name(&self) -> &'static str {
            self.name
        }

        fn failure_policy(&self) -> StageFailurePolicy {
            StageFailurePolicy::Required
        }

        fn enabled(&self, _context: &TranscriptContext) -> bool {
            true
        }

        fn transform(
            &self,
            text: &str,
            _context: &TranscriptContext,
        ) -> Result<String, StageError> {
            Ok(format!("{text}{}", self.suffix))
        }
    }

    #[test]
    fn internal_test_stage_composes_in_order_without_recording_orchestration() {
        let pipeline = TranscriptPipeline::new(vec![
            Box::new(AppendStage {
                name: "first",
                suffix: "-a",
            }),
            Box::new(AppendStage {
                name: "second",
                suffix: "-b",
            }),
        ]);
        let output = pipeline
            .run("raw".to_string(), &live_context(all_stages()))
            .unwrap();
        assert_eq!(output.text, "raw-a-b");
        assert_eq!(
            output.stages.iter().map(|s| s.stage).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    struct FailingStage {
        policy: StageFailurePolicy,
    }

    impl TranscriptTransform for FailingStage {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn failure_policy(&self) -> StageFailurePolicy {
            self.policy
        }

        fn enabled(&self, _context: &TranscriptContext) -> bool {
            true
        }

        fn transform(
            &self,
            _text: &str,
            _context: &TranscriptContext,
        ) -> Result<String, StageError> {
            Err(StageError::new("fixture_failure"))
        }
    }

    #[test]
    fn required_stage_failure_stops_the_pipeline() {
        let pipeline = TranscriptPipeline::new(vec![Box::new(FailingStage {
            policy: StageFailurePolicy::Required,
        })]);
        let error = match pipeline.run("raw".to_string(), &live_context(all_stages())) {
            Ok(_) => panic!("required stage failure unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.stage, "failing");
        assert_eq!(error.code, "fixture_failure");
    }

    #[test]
    fn optional_stage_failure_falls_back_to_previous_text_and_continues() {
        let pipeline = TranscriptPipeline::new(vec![
            Box::new(FailingStage {
                policy: StageFailurePolicy::OptionalFallback,
            }),
            Box::new(AppendStage {
                name: "after",
                suffix: "-kept",
            }),
        ]);
        let output = pipeline
            .run("raw".to_string(), &live_context(all_stages()))
            .unwrap();
        assert_eq!(output.text, "raw-kept");
        assert_eq!(output.stages[0].outcome, StageOutcome::Fallback);
        assert!(!output.stages[0].changed);
    }
}
