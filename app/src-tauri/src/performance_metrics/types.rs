use serde::{Deserialize, Serialize};

pub const PERFORMANCE_RUN_SCHEMA_VERSION: u32 = 1;
pub const RESOURCE_SAMPLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnavailableReasonV1 {
    UnsupportedPlatform,
    SampleFailed,
    NoSamples,
    DependencyPending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum MeasurementV1<T> {
    Measured { value: T },
    NotApplicable,
    Unavailable { reason: UnavailableReasonV1 },
}

impl<T> MeasurementV1<T> {
    pub fn measured(value: T) -> Self {
        Self::Measured { value }
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Measured { value } => Some(value),
            Self::NotApplicable | Self::Unavailable { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PerformanceRunKindV1 {
    Dictation,
    FileTranscription,
    SelectedTextTransform,
}

impl PerformanceRunKindV1 {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dictation => "dictation",
            Self::FileTranscription => "fileTranscription",
            Self::SelectedTextTransform => "selectedTextTransform",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunCorrelationV1 {
    Dictation { recording_id: u64 },
    FileTranscription { file_run_id: u64 },
    SelectedTextTransform { transform_pass_id: u64 },
}

impl RunCorrelationV1 {
    pub(crate) fn storage_parts(&self) -> (&'static str, u64) {
        match self {
            Self::Dictation { recording_id } => ("dictation", *recording_id),
            Self::FileTranscription { file_run_id } => ("fileTranscription", *file_run_id),
            Self::SelectedTextTransform { transform_pass_id } => {
                ("selectedTextTransform", *transform_pass_id)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PerformanceStageV1 {
    CaptureFinalization,
    FileDecode,
    Vad,
    ModelQueue,
    ModelLoad,
    InferenceDecode,
    TranscriptTransform,
    Cleanup,
    VoiceCommands,
    SmartCorrection,
    SmartFormatting,
    IdeContext,
    CliCommand,
    FileOutput,
    ClipboardPaste,
    FileReturn,
    TotalProcessing,
    SelectedTextCapture,
    InstructionCapture,
    InstructionAsr,
    SidecarSpawnLoad,
    Generation,
    ReviewReady,
    Apply,
    Undo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageOutcomeV1 {
    Completed,
    Skipped,
    Fallback,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageTimingV1 {
    pub stage: PerformanceStageV1,
    pub duration_ms: MeasurementV1<u64>,
    pub outcome: StageOutcomeV1,
}

impl StageTimingV1 {
    pub fn measured(stage: PerformanceStageV1, duration_ms: u64) -> Self {
        Self {
            stage,
            duration_ms: MeasurementV1::measured(duration_ms),
            outcome: StageOutcomeV1::Completed,
        }
    }

    pub fn not_applicable(stage: PerformanceStageV1) -> Self {
        Self {
            stage,
            duration_ms: MeasurementV1::NotApplicable,
            outcome: StageOutcomeV1::Skipped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeRoleV1 {
    Transcription,
    InstructionAsr,
    Generation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeBackendV1 {
    Whisper,
    Parakeet,
    Coreml,
    LlamaCpp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcceleratorV1 {
    Cpu,
    MetalGpu,
    AppleNeuralEngine,
    PlatformFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelWarmStateV1 {
    Warm,
    ColdLoaded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeIdentityV1 {
    pub role: RuntimeRoleV1,
    pub model_id: String,
    pub backend: RuntimeBackendV1,
    pub accelerator: AcceleratorV1,
    pub warm_state: ModelWarmStateV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SizeBucketV1 {
    Empty,
    Tiny,
    Small,
    Medium,
    Large,
    ExtraLarge,
}

pub fn size_bucket(bytes: usize) -> SizeBucketV1 {
    match bytes {
        0 => SizeBucketV1::Empty,
        1..=64 => SizeBucketV1::Tiny,
        65..=512 => SizeBucketV1::Small,
        513..=4_096 => SizeBucketV1::Medium,
        4_097..=32_768 => SizeBucketV1::Large,
        _ => SizeBucketV1::ExtraLarge,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentFreeInputSummaryV1 {
    pub audio_duration_ms: MeasurementV1<u64>,
    pub input_size_bucket: MeasurementV1<SizeBucketV1>,
    pub output_size_bucket: MeasurementV1<SizeBucketV1>,
    pub output_token_count: MeasurementV1<u64>,
}

impl ContentFreeInputSummaryV1 {
    pub fn audio(audio_duration_ms: u64) -> Self {
        Self {
            audio_duration_ms: MeasurementV1::measured(audio_duration_ms),
            input_size_bucket: MeasurementV1::NotApplicable,
            output_size_bucket: MeasurementV1::NotApplicable,
            output_token_count: MeasurementV1::NotApplicable,
        }
    }

    pub fn audio_with_output(audio_duration_ms: u64, output_bytes: usize) -> Self {
        Self {
            output_size_bucket: MeasurementV1::measured(size_bucket(output_bytes)),
            ..Self::audio(audio_duration_ms)
        }
    }
}

impl Default for ContentFreeInputSummaryV1 {
    fn default() -> Self {
        Self {
            audio_duration_ms: MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::NoSamples,
            },
            input_size_bucket: MeasurementV1::NotApplicable,
            output_size_bucket: MeasurementV1::NotApplicable,
            output_token_count: MeasurementV1::NotApplicable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceRangeV1<T> {
    pub start: MeasurementV1<T>,
    pub average: MeasurementV1<T>,
    pub peak: MeasurementV1<T>,
    pub end: MeasurementV1<T>,
}

impl<T> ResourceRangeV1<T> {
    pub fn unavailable(reason: UnavailableReasonV1) -> Self {
        Self {
            start: MeasurementV1::Unavailable { reason },
            average: MeasurementV1::Unavailable { reason },
            peak: MeasurementV1::Unavailable { reason },
            end: MeasurementV1::Unavailable { reason },
        }
    }

    pub fn not_applicable() -> Self {
        Self {
            start: MeasurementV1::NotApplicable,
            average: MeasurementV1::NotApplicable,
            peak: MeasurementV1::NotApplicable,
            end: MeasurementV1::NotApplicable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResourceSampleV1 {
    pub cpu_percent: MeasurementV1<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResourceSampleV1 {
    pub cpu_percent: MeasurementV1<f32>,
    pub rss_bytes: MeasurementV1<u64>,
    pub rust_heap_bytes: MeasurementV1<u64>,
    pub ffi_native_heap_bytes: MeasurementV1<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarResourceSampleV1 {
    pub cpu_percent: MeasurementV1<f32>,
    pub rss_bytes: MeasurementV1<u64>,
}

impl SidecarResourceSampleV1 {
    pub fn unavailable(reason: UnavailableReasonV1) -> Self {
        Self {
            cpu_percent: MeasurementV1::Unavailable { reason },
            rss_bytes: MeasurementV1::Unavailable { reason },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSampleV1 {
    pub schema_version: u32,
    pub observed_at_ms: i64,
    pub host: HostResourceSampleV1,
    pub main_process: ProcessResourceSampleV1,
    pub sidecar_process: SidecarResourceSampleV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostResourceSummaryV1 {
    pub cpu_percent: ResourceRangeV1<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResourceSummaryV1 {
    pub cpu_percent: ResourceRangeV1<f32>,
    pub rss_bytes: ResourceRangeV1<u64>,
    pub rust_heap_bytes: ResourceRangeV1<u64>,
    pub ffi_native_heap_bytes: ResourceRangeV1<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarResourceSummaryV1 {
    pub cpu_percent: ResourceRangeV1<f32>,
    pub rss_bytes: ResourceRangeV1<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSummaryV1 {
    pub sample_count: u32,
    pub host: HostResourceSummaryV1,
    pub main_process: ProcessResourceSummaryV1,
    pub sidecar_process: SidecarResourceSummaryV1,
}

impl ResourceSummaryV1 {
    pub fn unavailable_for(kind: PerformanceRunKindV1) -> Self {
        let no_samples = UnavailableReasonV1::NoSamples;
        let sidecar = if kind == PerformanceRunKindV1::SelectedTextTransform {
            SidecarResourceSummaryV1 {
                cpu_percent: ResourceRangeV1::unavailable(no_samples),
                rss_bytes: ResourceRangeV1::unavailable(no_samples),
            }
        } else {
            SidecarResourceSummaryV1 {
                cpu_percent: ResourceRangeV1::not_applicable(),
                rss_bytes: ResourceRangeV1::not_applicable(),
            }
        };
        Self {
            sample_count: 0,
            host: HostResourceSummaryV1 {
                cpu_percent: ResourceRangeV1::unavailable(no_samples),
            },
            main_process: ProcessResourceSummaryV1 {
                cpu_percent: ResourceRangeV1::unavailable(no_samples),
                rss_bytes: ResourceRangeV1::unavailable(no_samples),
                rust_heap_bytes: ResourceRangeV1::unavailable(no_samples),
                ffi_native_heap_bytes: ResourceRangeV1::unavailable(no_samples),
            },
            sidecar_process: sidecar,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StableRunErrorV1 {
    AudioCaptureFailed,
    DecodeFailed,
    VadFailed,
    ModelFailed,
    InferenceFailed,
    TransformStageFailed,
    DeliveryFailed,
    InternalEarlyExit,
    InterruptedByRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunOutcomeV1 {
    Success,
    NoSpeech,
    Cancelled {
        stage: PerformanceStageV1,
    },
    TimedOut {
        stage: PerformanceStageV1,
    },
    Failed {
        stage: PerformanceStageV1,
        error_code: StableRunErrorV1,
    },
    Interrupted {
        stage: PerformanceStageV1,
        error_code: StableRunErrorV1,
    },
}

impl RunOutcomeV1 {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::NoSpeech => "noSpeech",
            Self::Cancelled { .. } => "cancelled",
            Self::TimedOut { .. } => "timedOut",
            Self::Failed { .. } => "failed",
            Self::Interrupted { .. } => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformFollowUpKindV1 {
    Apply,
    Undo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformFollowUpV1 {
    pub kind: TransformFollowUpKindV1,
    pub at_ms: i64,
    pub duration_ms: MeasurementV1<u64>,
    pub outcome: StageOutcomeV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRunV1 {
    pub schema_version: u32,
    pub run_id: String,
    pub kind: PerformanceRunKindV1,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub app_version: String,
    pub correlation: RunCorrelationV1,
    pub outcome: RunOutcomeV1,
    pub runtimes: Vec<RuntimeIdentityV1>,
    pub stages: Vec<StageTimingV1>,
    pub input: ContentFreeInputSummaryV1,
    pub resources: ResourceSummaryV1,
    pub follow_ups: Vec<TransformFollowUpV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveRunV1 {
    pub run_id: String,
    pub kind: PerformanceRunKindV1,
    pub started_at_ms: i64,
    pub app_version: String,
    pub correlation: RunCorrelationV1,
    pub current_stage: PerformanceStageV1,
    pub runtimes: Vec<RuntimeIdentityV1>,
    pub stages: Vec<StageTimingV1>,
    pub input: ContentFreeInputSummaryV1,
    pub clear_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRunListV1 {
    pub schema_version: u32,
    pub runs: Vec<PerformanceRunV1>,
}
