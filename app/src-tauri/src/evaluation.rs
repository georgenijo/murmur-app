//! Local, fixture-driven evaluation for recognition, transformation, and delivery.
//!
//! The deterministic tier consumes curated text fixtures only. The opt-in
//! hardware tier reads explicitly referenced project WAV files and installed
//! models. Neither tier reads dictation history, app settings, the system
//! clipboard, microphone input, or frontmost-window state.

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::cli_command::{CliFormattingMode, CliLexicon};
use crate::correction::CorrectionMatcher;
use crate::knowledge_store::VoiceCommandKind;
use crate::transcript_transform::{
    transform_transcript_observed, StageReport, StageTextObserver, TranscriptContext,
    TranscriptSource, TranscriptStageConfig, TranscriptTransformResources,
};
use crate::voice_commands::{ResolvedVoiceCommand, VoiceCommandRuntime};

pub const FIXTURE_VERSION: u32 = 1;
pub const REPORT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EvaluationTier {
    Deterministic,
    Hardware,
}

impl EvaluationTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Hardware => "hardware",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub tier: EvaluationTier,
    pub fixtures_dir: PathBuf,
    pub workspace_root: PathBuf,
    pub machine_label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvaluationFixture {
    fixture_version: u32,
    id: String,
    tier: EvaluationTier,
    provenance: FixtureProvenance,
    #[serde(default)]
    requirements: FixtureRequirements,
    input: FixtureInput,
    #[serde(default)]
    context: FixtureContext,
    expected: FixtureExpected,
    timing: FixtureTiming,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FixtureDocument {
    One(EvaluationFixture),
    Many(Vec<EvaluationFixture>),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureProvenance {
    kind: String,
    source: String,
    contains_real_user_data: bool,
    deletion: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureRequirements {
    audio: Option<AudioRequirement>,
    model: Option<ModelRequirement>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioRequirement {
    path: String,
    sample_rate_hz: u32,
    channels: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelRequirement {
    name: String,
    backend: String,
    installed_only: bool,
    language: String,
    smart_punctuation: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureInput {
    raw_asr: Option<String>,
    #[serde(default)]
    reference_transcript: Option<String>,
    expected_raw_asr: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureContext {
    #[serde(default)]
    session_id: u64,
    #[serde(default)]
    bundle_id: Option<String>,
    #[serde(default)]
    matched_profile: Option<String>,
    #[serde(default)]
    cli_formatting_mode: FixtureCliMode,
    #[serde(default)]
    stages: FixtureStageConfig,
    #[serde(default)]
    resources: FixtureResources,
    #[serde(default = "default_fixed_now")]
    fixed_now: String,
    #[serde(default)]
    clipboard_text: Option<String>,
}

impl Default for FixtureContext {
    fn default() -> Self {
        Self {
            session_id: 0,
            bundle_id: None,
            matched_profile: None,
            cli_formatting_mode: FixtureCliMode::Auto,
            stages: FixtureStageConfig::default(),
            resources: FixtureResources::default(),
            fixed_now: default_fixed_now(),
            clipboard_text: None,
        }
    }
}

fn default_fixed_now() -> String {
    "2026-07-20T09:07:00-04:00".to_string()
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FixtureCliMode {
    Auto,
    Enabled,
    Disabled,
}

impl Default for FixtureCliMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl From<FixtureCliMode> for CliFormattingMode {
    fn from(value: FixtureCliMode) -> Self {
        match value {
            FixtureCliMode::Auto => Self::Auto,
            FixtureCliMode::Enabled => Self::Enabled,
            FixtureCliMode::Disabled => Self::Disabled,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct FixtureStageConfig {
    cleanup: bool,
    cleanup_remove_filler: bool,
    cleanup_capitalize: bool,
    voice_commands: bool,
    smart_correction: bool,
    smart_formatting: bool,
    ide_context: bool,
    cli_command: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct FixtureResources {
    vocabulary_terms: Vec<String>,
    vocabulary_aliases: Vec<FixturePair>,
    fuzzy_correction: bool,
    include_builtin_corrections: bool,
    voice_commands: Vec<FixtureVoiceCommand>,
    cli_prompt: Option<String>,
    ide_symbols: Vec<String>,
    ide_files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixturePair {
    spoken: String,
    written: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureVoiceCommand {
    id: String,
    phrase: String,
    command_type: VoiceCommandKind,
    content: String,
    allow_clipboard_read: bool,
    app_scoped: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureExpected {
    final_text: String,
    delivered_text: String,
    #[serde(default)]
    command_case: bool,
    #[serde(default)]
    no_change_preservation: bool,
    #[serde(default)]
    stages: Vec<ExpectedStage>,
    #[serde(default)]
    delivery: ExpectedDelivery,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedStage {
    name: String,
    outcome: String,
    changed: bool,
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedDelivery {
    attempts: usize,
    partial_count: usize,
    first_partial_ms: Option<u64>,
    final_only: bool,
}

impl Default for ExpectedDelivery {
    fn default() -> Self {
        Self {
            attempts: 1,
            partial_count: 0,
            first_partial_ms: None,
            final_only: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct FixtureTiming {
    raw_asr_ms: u64,
    transform_ms: u64,
    delivery_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationReport {
    pub report_version: u32,
    pub fixture_version: u32,
    pub generated_at: String,
    pub tier: EvaluationTier,
    pub privacy: ReportPrivacy,
    pub environment: ReportEnvironment,
    pub summary: ReportSummary,
    pub cases: Vec<CaseReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPrivacy {
    pub local_only: bool,
    pub history_ingestion: bool,
    pub network_used: bool,
    pub system_clipboard_used: bool,
    pub fixture_provenance_required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportEnvironment {
    pub app_version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub machine_label: String,
    pub logical_cpus: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub aggregate_raw_wer: Option<f64>,
    pub aggregate_normalized_wer: Option<f64>,
    pub aggregate_cer: Option<f64>,
    pub transformation_match_rate: Option<f64>,
    pub command_exact_match_rate: Option<f64>,
    pub no_change_preservation_rate: Option<f64>,
    pub delivery_match_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseReport {
    pub id: String,
    pub status: CaseStatus,
    pub failures: Vec<String>,
    pub provenance: CaseProvenance,
    pub context: CaseContext,
    pub model: Option<ModelMetadata>,
    pub recognition: RecognitionMetrics,
    pub transformation: TransformationMetrics,
    pub delivery: DeliveryMetrics,
    pub latency: LatencyMetrics,
    pub runtime: RuntimeMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaseStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseProvenance {
    pub kind: String,
    pub source: String,
    pub contains_real_user_data: bool,
    pub deletion: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseContext {
    pub bundle_id: Option<String>,
    pub matched_profile: Option<String>,
    pub fixture_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    pub name: String,
    pub backend: String,
    pub accelerator: String,
    pub audio_path: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionMetrics {
    pub expected_raw: Option<String>,
    pub actual_raw: Option<String>,
    pub raw_word_errors: Option<usize>,
    pub normalized_word_errors: Option<usize>,
    pub reference_words: Option<usize>,
    pub normalized_reference_words: Option<usize>,
    pub reference_characters: Option<usize>,
    pub character_errors: Option<usize>,
    pub raw_wer: Option<f64>,
    pub normalized_wer: Option<f64>,
    pub cer: Option<f64>,
    pub bounded_alternative_match: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TransformationMetrics {
    pub expected_final: String,
    pub actual_final: Option<String>,
    pub exact_match: bool,
    pub command_exact_match: Option<bool>,
    pub no_change_preserved: Option<bool>,
    pub stages: Vec<StageMetric>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageMetric {
    pub name: String,
    pub outcome: String,
    pub changed: bool,
    pub duration_us: u64,
    pub text: String,
    pub expectation_match: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryMetrics {
    pub expected: String,
    pub delivered: Option<String>,
    pub exact_match: bool,
    pub attempts: usize,
    pub partial_count: usize,
    pub first_partial_ms: Option<u64>,
    pub first_partial_applicability: &'static str,
    pub final_only: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LatencyMetrics {
    pub raw_asr_ms: u64,
    pub transformation_ms: u64,
    pub finalization_ms: u64,
    pub delivery_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetadata {
    pub incremental_completion: &'static str,
    pub fallback_used: bool,
    pub fallback_stages: Vec<String>,
    pub memory_before_mb: Option<u64>,
    pub memory_after_mb: Option<u64>,
    pub memory_delta_mb: Option<i64>,
}

#[derive(Clone)]
struct FixedVoiceCommandRuntime {
    now: DateTime<FixedOffset>,
    clipboard: Option<String>,
}

impl VoiceCommandRuntime for FixedVoiceCommandRuntime {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }

    fn clipboard_text(&self) -> Result<String, String> {
        self.clipboard
            .clone()
            .ok_or_else(|| "Fixture clipboard is unavailable.".to_string())
    }
}

#[derive(Default)]
struct InMemoryStageObserver {
    stages: Vec<(StageReport, String)>,
}

impl StageTextObserver for InMemoryStageObserver {
    fn observe(&mut self, report: &StageReport, text: &str) {
        self.stages.push((report.clone(), text.to_string()));
    }
}

#[derive(Default)]
struct InMemoryDeliverySink {
    values: Vec<String>,
}

impl InMemoryDeliverySink {
    fn deliver(&mut self, text: String) {
        self.values.push(text);
    }
}

#[derive(Default)]
struct FixedEvaluationClock {
    elapsed_ms: u64,
}

enum HardwareRecognition {
    Completed {
        raw_asr: String,
        model: ModelMetadata,
        elapsed_ms: u64,
    },
    Skipped {
        reason: String,
        model: ModelMetadata,
    },
    Failed {
        reason: String,
        model: ModelMetadata,
        elapsed_ms: u64,
    },
}

impl FixedEvaluationClock {
    fn advance(&mut self, amount_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(amount_ms);
    }
}

pub fn run(options: &RunOptions) -> Result<EvaluationReport, String> {
    let fixtures = load_fixtures(&options.fixtures_dir, options.tier)?;
    let mut cases = Vec::with_capacity(fixtures.len());
    for (path, fixture) in fixtures {
        cases.push(run_fixture(&path, fixture, options));
    }
    let summary = summarize(&cases);
    Ok(EvaluationReport {
        report_version: REPORT_VERSION,
        fixture_version: FIXTURE_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        tier: options.tier,
        privacy: ReportPrivacy {
            local_only: true,
            history_ingestion: false,
            network_used: false,
            system_clipboard_used: false,
            fixture_provenance_required: true,
        },
        environment: ReportEnvironment {
            app_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            machine_label: options.machine_label.clone(),
            logical_cpus: std::thread::available_parallelism().ok().map(usize::from),
        },
        summary,
        cases,
    })
}

pub fn write_report(report: &EvaluationReport, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create report directory: {error}"))?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("Could not serialize evaluation report: {error}"))?;
    fs::write(output, format!("{json}\n"))
        .map_err(|error| format!("Could not write {}: {error}", output.display()))
}

fn load_fixtures(
    fixtures_dir: &Path,
    tier: EvaluationTier,
) -> Result<Vec<(PathBuf, EvaluationFixture)>, String> {
    let mut paths = Vec::new();
    collect_json(fixtures_dir, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "No JSON fixtures found under {}",
            fixtures_dir.display()
        ));
    }
    let mut ids = HashSet::new();
    let mut fixtures = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let document: FixtureDocument = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid strict fixture {}: {error}", path.display()))?;
        let document_fixtures = match document {
            FixtureDocument::One(fixture) => vec![fixture],
            FixtureDocument::Many(fixtures) => fixtures,
        };
        if document_fixtures.is_empty() {
            return Err(format!(
                "Invalid strict fixture {}: fixture arrays must not be empty",
                path.display()
            ));
        }
        for fixture in document_fixtures {
            validate_fixture(&fixture, tier, &path)?;
            if !ids.insert(fixture.id.clone()) {
                return Err(format!("Duplicate fixture id '{}'", fixture.id));
            }
            fixtures.push((path.clone(), fixture));
        }
    }
    Ok(fixtures)
}

fn collect_json(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "Could not read fixture directory {}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not read fixture entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            output.push(path);
        }
    }
    Ok(())
}

fn validate_fixture(
    fixture: &EvaluationFixture,
    tier: EvaluationTier,
    path: &Path,
) -> Result<(), String> {
    let fail = |message: &str| format!("Fixture {}: {message}", path.display());
    if fixture.fixture_version != FIXTURE_VERSION {
        return Err(fail("unsupported fixtureVersion; expected 1"));
    }
    if fixture.tier != tier {
        return Err(fail("tier does not match the selected suite"));
    }
    if fixture.id.trim().is_empty() || fixture.input.expected_raw_asr.is_empty() {
        return Err(fail("id and expectedRawAsr must be non-empty"));
    }
    if fixture.provenance.contains_real_user_data {
        return Err(fail("real-user data is forbidden in evaluator fixtures"));
    }
    if fixture.provenance.source.trim().is_empty()
        || fixture.provenance.deletion.trim().is_empty()
        || fixture.provenance.kind.trim().is_empty()
    {
        return Err(fail("provenance kind, source, and deletion are required"));
    }
    if fixture.expected.delivery.partial_count != 0
        || fixture.expected.delivery.first_partial_ms.is_some()
        || !fixture.expected.delivery.final_only
        || fixture.expected.delivery.attempts != 1
    {
        return Err(fail(
            "Murmur is final-only: attempts must be 1, partialCount must be 0, firstPartialMs null, and finalOnly true",
        ));
    }
    DateTime::parse_from_rfc3339(&fixture.context.fixed_now)
        .map_err(|error| fail(&format!("fixedNow must be RFC 3339: {error}")))?;

    const STAGES: &[&str] = &[
        crate::transcript_transform::CLEANUP_STAGE,
        crate::transcript_transform::VOICE_COMMANDS_STAGE,
        crate::transcript_transform::SMART_CORRECTION_STAGE,
        crate::transcript_transform::SMART_FORMATTING_STAGE,
        crate::transcript_transform::IDE_CONTEXT_STAGE,
        crate::transcript_transform::CLI_COMMAND_STAGE,
    ];
    const OUTCOMES: &[&str] = &["applied", "skipped", "fallback", "failed"];
    let mut expected_stages = HashSet::new();
    for stage in &fixture.expected.stages {
        if !STAGES.contains(&stage.name.as_str()) {
            return Err(fail(&format!("unknown expected stage '{}'", stage.name)));
        }
        if !OUTCOMES.contains(&stage.outcome.as_str()) {
            return Err(fail(&format!(
                "unknown expected outcome '{}' for stage '{}'",
                stage.outcome, stage.name
            )));
        }
        if !expected_stages.insert(stage.name.as_str()) {
            return Err(fail(&format!("duplicate expected stage '{}'", stage.name)));
        }
    }

    let mut command_ids = HashSet::new();
    for command in &fixture.context.resources.voice_commands {
        if command.id.trim().is_empty() || command.phrase.trim().is_empty() {
            return Err(fail("voice-command id and phrase must be non-empty"));
        }
        if !command_ids.insert(command.id.as_str()) {
            return Err(fail(&format!(
                "duplicate voice-command id '{}'",
                command.id
            )));
        }
        if command.command_type == VoiceCommandKind::Snippet {
            crate::voice_commands::validate_snippet_template(
                &command.content,
                command.allow_clipboard_read,
            )
            .map_err(|error| fail(&format!("invalid voice-command snippet: {error}")))?;
        }
    }
    for file in &fixture.context.resources.ide_files {
        if !is_clean_relative_path(file) {
            return Err(fail("IDE fixture files must be clean root-relative paths"));
        }
    }
    match tier {
        EvaluationTier::Deterministic => {
            if fixture.input.raw_asr.is_none()
                || fixture.requirements.audio.is_some()
                || fixture.requirements.model.is_some()
            {
                return Err(fail(
                    "deterministic fixtures require rawAsr and forbid audio/model requirements",
                ));
            }
        }
        EvaluationTier::Hardware => {
            let Some(model) = fixture.requirements.model.as_ref() else {
                return Err(fail("hardware fixtures require model metadata"));
            };
            let Some(audio) = fixture.requirements.audio.as_ref() else {
                return Err(fail("hardware fixtures require audio metadata"));
            };
            if fixture.input.raw_asr.is_some() {
                return Err(fail(
                    "hardware fixtures require audio and must obtain rawAsr from the model",
                ));
            }
            if !model.installed_only {
                return Err(fail("hardware fixtures must set installedOnly true"));
            }
            let definition = crate::model_runtime::model_definition(&model.name)
                .map_err(|error| fail(&error))?;
            if model.backend != definition.backend.as_str() {
                return Err(fail(
                    "hardware model backend does not match the model catalog",
                ));
            }
            if model.language.trim().is_empty() {
                return Err(fail("hardware model language must be non-empty"));
            }
            if !is_clean_relative_path(&audio.path)
                || Path::new(&audio.path)
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("wav")
            {
                return Err(fail(
                    "hardware audio must be a clean workspace-relative WAV path",
                ));
            }
            if audio.sample_rate_hz == 0 || audio.channels == 0 {
                return Err(fail(
                    "hardware audio sample rate and channel count must be non-zero",
                ));
            }
        }
    }
    Ok(())
}

fn is_clean_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && !Path::new(value).is_absolute()
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn run_fixture(_path: &Path, fixture: EvaluationFixture, options: &RunOptions) -> CaseReport {
    let mut failures = Vec::new();
    let memory_before = (options.tier == EvaluationTier::Hardware)
        .then(current_memory_mb)
        .flatten();
    let (raw_asr, model, measured_raw_ms) = match options.tier {
        EvaluationTier::Deterministic => (
            fixture.input.raw_asr.clone().unwrap_or_default(),
            None,
            fixture.timing.raw_asr_ms,
        ),
        EvaluationTier::Hardware => match run_hardware_recognition(&fixture, options) {
            HardwareRecognition::Completed {
                raw_asr,
                model,
                elapsed_ms,
            } => (raw_asr, Some(model), elapsed_ms),
            HardwareRecognition::Skipped { reason, model } => {
                return unexecuted_case(
                    &fixture,
                    CaseStatus::Skipped,
                    reason,
                    Some(model),
                    0,
                    memory_before,
                )
            }
            HardwareRecognition::Failed {
                reason,
                model,
                elapsed_ms,
            } => {
                return unexecuted_case(
                    &fixture,
                    CaseStatus::Failed,
                    reason,
                    Some(model),
                    elapsed_ms,
                    memory_before,
                )
            }
        },
    };

    let recognition = score_recognition(
        fixture
            .input
            .reference_transcript
            .as_deref()
            .unwrap_or(&fixture.input.expected_raw_asr[0]),
        &fixture.input.expected_raw_asr,
        &raw_asr,
    );
    if !recognition.bounded_alternative_match {
        failures.push("raw ASR did not match a bounded expected alternative".to_string());
    }
    let transform_started = Instant::now();
    let (actual_final, stages, transform_error) = run_transform(&fixture, &raw_asr);
    let measured_transform_ms = if options.tier == EvaluationTier::Hardware {
        transform_started.elapsed().as_millis() as u64
    } else {
        fixture.timing.transform_ms
    };
    if let Some(error) = transform_error {
        failures.push(error);
    }

    let stage_metrics = score_stages(&fixture.expected.stages, stages, &mut failures);
    let exact_match = actual_final
        .as_ref()
        .is_some_and(|text| text == &fixture.expected.final_text);
    if !exact_match {
        failures.push("final transformed text mismatch".to_string());
    }
    let command_exact_match = fixture.expected.command_case.then_some(exact_match);
    let no_change_preserved = fixture
        .expected
        .no_change_preservation
        .then(|| actual_final.as_ref().is_some_and(|text| text == &raw_asr));
    if no_change_preserved == Some(false) {
        failures.push("ordinary prose changed".to_string());
    }

    let mut sink = InMemoryDeliverySink::default();
    if let Some(text) = actual_final.clone() {
        sink.deliver(text);
    }
    let delivered = sink.values.last().cloned();
    let delivery_match = delivered
        .as_ref()
        .is_some_and(|text| text == &fixture.expected.delivered_text);
    if !delivery_match || sink.values.len() != fixture.expected.delivery.attempts {
        failures.push("delivery expectation mismatch".to_string());
    }

    let mut clock = FixedEvaluationClock::default();
    clock.advance(measured_raw_ms);
    clock.advance(measured_transform_ms);
    let finalization_ms = clock.elapsed_ms;
    let delivery_ms = if options.tier == EvaluationTier::Hardware {
        0
    } else {
        fixture.timing.delivery_ms
    };
    clock.advance(delivery_ms);
    let fallback_stages = stage_metrics
        .iter()
        .filter(|stage| stage.outcome == "fallback")
        .map(|stage| stage.name.clone())
        .collect::<Vec<_>>();
    let memory_after = (options.tier == EvaluationTier::Hardware)
        .then(current_memory_mb)
        .flatten();

    CaseReport {
        id: fixture.id.clone(),
        status: if failures.is_empty() {
            CaseStatus::Passed
        } else {
            CaseStatus::Failed
        },
        failures,
        provenance: provenance_report(&fixture),
        context: context_report(&fixture),
        model,
        recognition,
        transformation: TransformationMetrics {
            expected_final: fixture.expected.final_text,
            actual_final,
            exact_match,
            command_exact_match,
            no_change_preserved,
            stages: stage_metrics,
        },
        delivery: DeliveryMetrics {
            expected: fixture.expected.delivered_text,
            delivered,
            exact_match: delivery_match,
            attempts: sink.values.len(),
            partial_count: 0,
            first_partial_ms: None,
            first_partial_applicability: "notApplicable",
            final_only: true,
        },
        latency: LatencyMetrics {
            raw_asr_ms: measured_raw_ms,
            transformation_ms: measured_transform_ms,
            finalization_ms,
            delivery_ms,
            total_ms: clock.elapsed_ms,
        },
        runtime: RuntimeMetadata {
            incremental_completion: "notApplicableFinalOnly",
            fallback_used: !fallback_stages.is_empty(),
            fallback_stages,
            memory_before_mb: memory_before,
            memory_after_mb: memory_after,
            memory_delta_mb: memory_before
                .zip(memory_after)
                .map(|(before, after)| after as i64 - before as i64),
        },
    }
}

fn run_hardware_recognition(
    fixture: &EvaluationFixture,
    options: &RunOptions,
) -> HardwareRecognition {
    let audio = fixture
        .requirements
        .audio
        .as_ref()
        .expect("validated audio");
    let model = fixture
        .requirements
        .model
        .as_ref()
        .expect("validated model");
    let model_metadata = ModelMetadata {
        name: model.name.clone(),
        backend: model.backend.clone(),
        accelerator: crate::model_runtime::model_definition(&model.name)
            .map(crate::model_runtime::model_accelerator)
            .unwrap_or("unknown")
            .to_string(),
        audio_path: audio.path.clone(),
        sample_rate_hz: audio.sample_rate_hz,
        channels: audio.channels,
    };
    let audio_path = options.workspace_root.join(&audio.path);
    let workspace = match options.workspace_root.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return HardwareRecognition::Failed {
                reason: format!("workspace root is unavailable: {error}"),
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
    };
    let audio_path = match audio_path.canonicalize() {
        Ok(path) if path.starts_with(&workspace) => path,
        Ok(_) => {
            return HardwareRecognition::Failed {
                reason: "audio path escapes the local workspace boundary".to_string(),
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
        Err(error) => {
            return HardwareRecognition::Failed {
                reason: format!("audio fixture is unavailable: {error}"),
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
    };
    if audio_path.extension().and_then(|value| value.to_str()) != Some("wav") {
        return HardwareRecognition::Failed {
            reason: "hardware audio fixture must be a WAV file".to_string(),
            model: model_metadata,
            elapsed_ms: 0,
        };
    }
    let wav_spec = match hound::WavReader::open(&audio_path) {
        Ok(reader) => reader.spec(),
        Err(error) => {
            return HardwareRecognition::Failed {
                reason: format!("could not inspect WAV fixture: {error}"),
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
    };
    if wav_spec.sample_rate != audio.sample_rate_hz || wav_spec.channels != audio.channels {
        return HardwareRecognition::Failed {
            reason: format!(
                "WAV metadata mismatch: expected {} Hz/{} channel(s), got {} Hz/{} channel(s)",
                audio.sample_rate_hz, audio.channels, wav_spec.sample_rate, wav_spec.channels
            ),
            model: model_metadata,
            elapsed_ms: 0,
        };
    }
    if !crate::model_runtime::model_installed(&model.name) {
        return HardwareRecognition::Skipped {
            reason: format!("required installed model '{}' is unavailable", model.name),
            model: model_metadata,
        };
    }
    let samples = match crate::audio_decode::decode_to_mono_16k(&audio_path.to_string_lossy()) {
        Ok(samples) => samples,
        Err(error) => {
            return HardwareRecognition::Failed {
                reason: error,
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
    };
    let mut backend = match crate::model_runtime::create_backend(&model.name) {
        Ok(backend) => backend,
        Err(error) => {
            return HardwareRecognition::Failed {
                reason: error,
                model: model_metadata,
                elapsed_ms: 0,
            }
        }
    };
    if backend.name() != model.backend {
        return HardwareRecognition::Failed {
            reason: format!(
                "fixture backend '{}' does not match runtime backend '{}'",
                model.backend,
                backend.name()
            ),
            model: model_metadata,
            elapsed_ms: 0,
        };
    }
    let started = Instant::now();
    if let Err(error) = backend.load_model(&model.name) {
        return HardwareRecognition::Failed {
            reason: error,
            model: model_metadata,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }
    let transcript = backend.transcribe(
        &samples,
        &model.language,
        fixture.context.resources.cli_prompt.as_deref(),
        model.smart_punctuation,
    );
    let elapsed_ms = started.elapsed().as_millis() as u64;
    backend.reset();
    match transcript {
        Ok(raw_asr) => HardwareRecognition::Completed {
            raw_asr,
            model: model_metadata,
            elapsed_ms,
        },
        Err(reason) => HardwareRecognition::Failed {
            reason,
            model: model_metadata,
            elapsed_ms,
        },
    }
}

fn run_transform(
    fixture: &EvaluationFixture,
    raw_asr: &str,
) -> (Option<String>, Vec<(StageReport, String)>, Option<String>) {
    let fixed_now = match DateTime::parse_from_rfc3339(&fixture.context.fixed_now) {
        Ok(value) => value,
        Err(error) => return (None, Vec::new(), Some(format!("invalid fixedNow: {error}"))),
    };
    let pairs = fixture
        .context
        .resources
        .vocabulary_aliases
        .iter()
        .map(|pair| (pair.spoken.clone(), pair.written.clone()))
        .collect::<Vec<_>>();
    let matcher = CorrectionMatcher::build(
        &fixture.context.resources.vocabulary_terms,
        &pairs,
        fixture.context.resources.fuzzy_correction,
        fixture.context.resources.include_builtin_corrections,
    );
    let voice_commands = fixture
        .context
        .resources
        .voice_commands
        .iter()
        .map(|command| ResolvedVoiceCommand {
            id: command.id.clone(),
            phrase: command.phrase.clone(),
            command_type: command.command_type,
            content: command.content.clone(),
            allow_clipboard_read: command.allow_clipboard_read,
            app_scoped: command.app_scoped,
        })
        .collect::<Vec<_>>();
    let resources = &fixture.context.resources;
    let context = TranscriptContext {
        session_id: fixture.context.session_id,
        source: TranscriptSource::Live,
        context_handle: fixture.context.matched_profile.clone(),
        cli_formatting_mode: fixture.context.cli_formatting_mode.into(),
        stages: TranscriptStageConfig {
            cleanup_enabled: fixture.context.stages.cleanup,
            cleanup_remove_filler: fixture.context.stages.cleanup_remove_filler,
            cleanup_capitalize: fixture.context.stages.cleanup_capitalize,
            voice_commands_enabled: fixture.context.stages.voice_commands,
            smart_correction_enabled: fixture.context.stages.smart_correction,
            smart_formatting_enabled: fixture.context.stages.smart_formatting,
            ide_context_enabled: fixture.context.stages.ide_context,
            cli_command_enabled: fixture.context.stages.cli_command,
        },
    };
    let mut observer = InMemoryStageObserver::default();
    let output = transform_transcript_observed(
        raw_asr.to_string(),
        &context,
        TranscriptTransformResources {
            custom_commands: Vec::new(),
            voice_commands,
            correction_matcher: (!matcher.is_empty()).then(|| Arc::new(matcher)),
            cli_lexicon: CliLexicon::from_context(resources.cli_prompt.as_deref(), &pairs),
            ide_context_index: fixture.context.stages.ide_context.then(|| {
                crate::ide_context::IdeContextIndex::from_eval_fixture(
                    &resources.ide_symbols,
                    &resources.ide_files,
                )
            }),
            voice_command_runtime: Some(Arc::new(FixedVoiceCommandRuntime {
                now: fixed_now,
                clipboard: fixture.context.clipboard_text.clone(),
            })),
        },
        &mut observer,
    );
    match output {
        Ok(output) => (Some(output.text), observer.stages, None),
        Err(error) => (None, observer.stages, Some(error.to_string())),
    }
}

fn score_recognition(
    reference: &str,
    bounded_expected: &[String],
    actual: &str,
) -> RecognitionMetrics {
    let (normalized_errors, normalized_words) =
        crate::benchmark::normalized_word_errors(reference, actual);
    let (raw_errors, raw_words) = crate::benchmark::word_errors(reference, actual);
    let (char_errors, reference_characters) = character_errors(reference, actual);
    RecognitionMetrics {
        expected_raw: Some(reference.to_string()),
        actual_raw: Some(actual.to_string()),
        raw_word_errors: Some(raw_errors),
        normalized_word_errors: Some(normalized_errors),
        reference_words: Some(raw_words),
        normalized_reference_words: Some(normalized_words),
        reference_characters: Some(reference_characters),
        character_errors: Some(char_errors),
        raw_wer: ratio(raw_errors, raw_words),
        normalized_wer: ratio(normalized_errors, normalized_words),
        cer: ratio(char_errors, reference_characters),
        bounded_alternative_match: bounded_expected.iter().any(|value| value == actual),
    }
}

fn character_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let reference = reference.to_lowercase().chars().collect::<Vec<_>>();
    let hypothesis = hypothesis.to_lowercase().chars().collect::<Vec<_>>();
    let mut previous = (0..=hypothesis.len()).collect::<Vec<_>>();
    for (row, expected) in reference.iter().enumerate() {
        let mut current = vec![row + 1; hypothesis.len() + 1];
        for (column, actual) in hypothesis.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(expected != actual));
        }
        previous = current;
    }
    (previous[hypothesis.len()], reference.len())
}

fn score_stages(
    expected: &[ExpectedStage],
    actual: Vec<(StageReport, String)>,
    failures: &mut Vec<String>,
) -> Vec<StageMetric> {
    let expected = expected
        .iter()
        .map(|stage| (stage.name.as_str(), stage))
        .collect::<HashMap<_, _>>();
    let mut observed = HashSet::new();
    let metrics = actual
        .into_iter()
        .map(|(report, text)| {
            observed.insert(report.stage);
            let outcome = report.outcome.as_str().to_string();
            let expectation_match = expected.get(report.stage).is_none_or(|expected| {
                expected.outcome == outcome
                    && expected.changed == report.changed
                    && expected.text.as_ref().is_none_or(|value| value == &text)
            });
            if !expectation_match {
                failures.push(format!("stage '{}' expectation mismatch", report.stage));
            }
            StageMetric {
                name: report.stage.to_string(),
                outcome,
                changed: report.changed,
                duration_us: report.duration_us,
                text,
                expectation_match,
            }
        })
        .collect::<Vec<_>>();
    for stage in expected.values() {
        if !observed.contains(stage.name.as_str()) {
            failures.push(format!("expected stage '{}' was not observed", stage.name));
        }
    }
    metrics
}

fn unexecuted_case(
    fixture: &EvaluationFixture,
    status: CaseStatus,
    reason: String,
    model: Option<ModelMetadata>,
    raw_asr_ms: u64,
    memory_before: Option<u64>,
) -> CaseReport {
    let memory_after = current_memory_mb();
    CaseReport {
        id: fixture.id.clone(),
        status,
        failures: vec![reason],
        provenance: provenance_report(fixture),
        context: context_report(fixture),
        model,
        recognition: RecognitionMetrics::default(),
        transformation: TransformationMetrics {
            expected_final: fixture.expected.final_text.clone(),
            ..TransformationMetrics::default()
        },
        delivery: DeliveryMetrics {
            expected: fixture.expected.delivered_text.clone(),
            first_partial_applicability: "notApplicable",
            final_only: true,
            ..DeliveryMetrics::default()
        },
        latency: LatencyMetrics {
            raw_asr_ms,
            finalization_ms: raw_asr_ms,
            total_ms: raw_asr_ms,
            ..LatencyMetrics::default()
        },
        runtime: RuntimeMetadata {
            incremental_completion: "notApplicableFinalOnly",
            memory_before_mb: memory_before,
            memory_after_mb: memory_after,
            memory_delta_mb: memory_before
                .zip(memory_after)
                .map(|(before, after)| after as i64 - before as i64),
            ..RuntimeMetadata::default()
        },
    }
}

fn provenance_report(fixture: &EvaluationFixture) -> CaseProvenance {
    CaseProvenance {
        kind: fixture.provenance.kind.clone(),
        source: fixture.provenance.source.clone(),
        contains_real_user_data: fixture.provenance.contains_real_user_data,
        deletion: fixture.provenance.deletion.clone(),
    }
}

fn context_report(fixture: &EvaluationFixture) -> CaseContext {
    CaseContext {
        bundle_id: fixture.context.bundle_id.clone(),
        matched_profile: fixture.context.matched_profile.clone(),
        fixture_only: true,
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then_some(numerator as f64 / denominator as f64)
}

fn current_memory_mb() -> Option<u64> {
    memory_stats::memory_stats().map(|stats| stats.physical_mem as u64 / 1_048_576)
}

fn summarize(cases: &[CaseReport]) -> ReportSummary {
    let mut summary = ReportSummary {
        total: cases.len(),
        passed: cases
            .iter()
            .filter(|case| case.status == CaseStatus::Passed)
            .count(),
        failed: cases
            .iter()
            .filter(|case| case.status == CaseStatus::Failed)
            .count(),
        skipped: cases
            .iter()
            .filter(|case| case.status == CaseStatus::Skipped)
            .count(),
        ..ReportSummary::default()
    };
    let scored = cases
        .iter()
        .filter(|case| case.status != CaseStatus::Skipped)
        .collect::<Vec<_>>();
    let raw_errors = scored
        .iter()
        .filter_map(|case| case.recognition.raw_word_errors)
        .sum::<usize>();
    let normalized_errors = scored
        .iter()
        .filter_map(|case| case.recognition.normalized_word_errors)
        .sum::<usize>();
    let reference_words = scored
        .iter()
        .filter_map(|case| case.recognition.reference_words)
        .sum::<usize>();
    let normalized_reference_words = scored
        .iter()
        .filter_map(|case| case.recognition.normalized_reference_words)
        .sum::<usize>();
    let character_errors = scored
        .iter()
        .filter_map(|case| case.recognition.character_errors)
        .sum::<usize>();
    let reference_characters = scored
        .iter()
        .filter_map(|case| case.recognition.reference_characters)
        .sum::<usize>();
    summary.aggregate_raw_wer = ratio(raw_errors, reference_words);
    summary.aggregate_normalized_wer = ratio(normalized_errors, normalized_reference_words);
    summary.aggregate_cer = ratio(character_errors, reference_characters);
    summary.transformation_match_rate = rate(&scored, |case| case.transformation.exact_match);
    summary.delivery_match_rate = rate(&scored, |case| case.delivery.exact_match);
    let command = scored
        .iter()
        .filter(|case| case.transformation.command_exact_match.is_some())
        .copied()
        .collect::<Vec<_>>();
    summary.command_exact_match_rate = rate(&command, |case| {
        case.transformation.command_exact_match == Some(true)
    });
    let no_change = scored
        .iter()
        .filter(|case| case.transformation.no_change_preserved.is_some())
        .copied()
        .collect::<Vec<_>>();
    summary.no_change_preservation_rate = rate(&no_change, |case| {
        case.transformation.no_change_preserved == Some(true)
    });
    summary
}

fn rate<T>(values: &[T], predicate: impl Fn(&T) -> bool) -> Option<f64> {
    (!values.is_empty()).then(|| {
        values.iter().filter(|value| predicate(value)).count() as f64 / values.len() as f64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_error_rate_counts_substitution() {
        assert_eq!(character_errors("Tauri", "Tori"), (2, 5));
    }

    #[test]
    fn deterministic_suite_is_fixture_driven_and_final_only() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("eval/fixtures/deterministic");
        let report = run(&RunOptions {
            tier: EvaluationTier::Deterministic,
            fixtures_dir: root,
            workspace_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
            machine_label: "test".to_string(),
        })
        .expect("deterministic fixtures should parse and execute");
        assert!(report.summary.total >= 10);
        assert_eq!(report.summary.failed, 0);
        assert_eq!(report.summary.skipped, 0);
        assert!(report.cases.iter().all(|case| {
            case.delivery.partial_count == 0
                && case.delivery.first_partial_ms.is_none()
                && case.delivery.first_partial_applicability == "notApplicable"
        }));
    }

    #[test]
    fn strict_schema_rejects_unknown_fields_and_malformed_expectations() {
        let fixture = r#"{
          "fixtureVersion": 1,
          "id": "bad",
          "tier": "deterministic",
          "unexpected": true
        }"#;
        assert!(serde_json::from_str::<EvaluationFixture>(fixture).is_err());

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("eval/fixtures/deterministic");
        let (path, fixture) = load_fixtures(&root, EvaluationTier::Deterministic)
            .unwrap()
            .remove(0);

        let mut real_user_fixture = fixture.clone();
        real_user_fixture.provenance.contains_real_user_data = true;
        assert!(
            validate_fixture(&real_user_fixture, EvaluationTier::Deterministic, &path)
                .unwrap_err()
                .contains("real-user data is forbidden")
        );

        let mut misspelled_stage = fixture.clone();
        misspelled_stage.expected.stages[0].name = "smart_formattting".to_string();
        assert!(
            validate_fixture(&misspelled_stage, EvaluationTier::Deterministic, &path)
                .unwrap_err()
                .contains("unknown expected stage")
        );

        let mut duplicate_stage = fixture.clone();
        duplicate_stage
            .expected
            .stages
            .push(duplicate_stage.expected.stages[0].clone());
        assert!(
            validate_fixture(&duplicate_stage, EvaluationTier::Deterministic, &path)
                .unwrap_err()
                .contains("duplicate expected stage")
        );

        let mut multiple_deliveries = fixture.clone();
        multiple_deliveries.expected.delivery.attempts = 2;
        assert!(
            validate_fixture(&multiple_deliveries, EvaluationTier::Deterministic, &path)
                .unwrap_err()
                .contains("attempts must be 1")
        );

        let empty_dir = tempfile::tempdir().unwrap();
        fs::write(empty_dir.path().join("empty.json"), "[]").unwrap();
        assert!(
            load_fixtures(empty_dir.path(), EvaluationTier::Deterministic)
                .unwrap_err()
                .contains("fixture arrays must not be empty")
        );
    }

    #[test]
    fn hardware_schema_and_runtime_errors_cannot_be_reported_as_skips() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("eval/fixtures/hardware");
        let (path, fixture) = load_fixtures(&root, EvaluationTier::Hardware)
            .unwrap()
            .remove(0);

        let mut unknown_model = fixture.clone();
        unknown_model.requirements.model.as_mut().unwrap().name = "unknown-model".to_string();
        assert!(
            validate_fixture(&unknown_model, EvaluationTier::Hardware, &path)
                .unwrap_err()
                .contains("Unknown transcription model")
        );

        let mut escaping_audio = fixture.clone();
        escaping_audio.requirements.audio.as_mut().unwrap().path = "../outside.wav".to_string();
        assert!(
            validate_fixture(&escaping_audio, EvaluationTier::Hardware, &path)
                .unwrap_err()
                .contains("clean workspace-relative WAV path")
        );

        let unavailable_workspace = tempfile::tempdir().unwrap().path().join("removed");
        let result = run_hardware_recognition(
            &fixture,
            &RunOptions {
                tier: EvaluationTier::Hardware,
                fixtures_dir: root,
                workspace_root: unavailable_workspace,
                machine_label: "test".to_string(),
            },
        );
        assert!(matches!(result, HardwareRecognition::Failed { .. }));
    }
}
