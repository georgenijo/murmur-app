import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type UnavailableReasonV1 =
  | 'unsupportedPlatform'
  | 'sampleFailed'
  | 'noSamples'
  | 'dependencyPending';

export type MeasurementV1<T> =
  | { status: 'measured'; value: T }
  | { status: 'notApplicable' }
  | { status: 'unavailable'; reason: UnavailableReasonV1 };

export type PerformanceRunKindV1 =
  | 'dictation'
  | 'fileTranscription'
  | 'selectedTextTransform';

export type RunCorrelationV1 =
  | { kind: 'dictation'; recordingId: number }
  | { kind: 'fileTranscription'; fileRunId: number }
  | { kind: 'selectedTextTransform'; transformPassId: number };

export type PerformanceStageV1 =
  | 'captureFinalization'
  | 'fileDecode'
  | 'vad'
  | 'modelQueue'
  | 'modelLoad'
  | 'inferenceDecode'
  | 'transcriptTransform'
  | 'cleanup'
  | 'voiceCommands'
  | 'smartCorrection'
  | 'smartFormatting'
  | 'ideContext'
  | 'cliCommand'
  | 'fileOutput'
  | 'clipboardPaste'
  | 'fileReturn'
  | 'totalProcessing'
  | 'selectedTextCapture'
  | 'instructionCapture'
  | 'instructionAsr'
  | 'sidecarSpawnLoad'
  | 'generation'
  | 'reviewReady'
  | 'apply'
  | 'undo';

export type StageOutcomeV1 = 'completed' | 'skipped' | 'fallback' | 'failed';

export interface StageTimingV1 {
  stage: PerformanceStageV1;
  durationMs: MeasurementV1<number>;
  outcome: StageOutcomeV1;
}

export interface RuntimeIdentityV1 {
  role: 'transcription' | 'instructionAsr' | 'generation';
  modelId: string;
  backend: 'whisper' | 'parakeet' | 'coreml' | 'llamaCpp';
  accelerator: 'cpu' | 'metalGpu' | 'appleNeuralEngine' | 'platformFallback';
  warmState: 'warm' | 'coldLoaded' | 'unknown';
}

export type SizeBucketV1 =
  | 'empty'
  | 'tiny'
  | 'small'
  | 'medium'
  | 'large'
  | 'extraLarge';

export interface ContentFreeInputSummaryV1 {
  audioDurationMs: MeasurementV1<number>;
  inputSizeBucket: MeasurementV1<SizeBucketV1>;
  outputSizeBucket: MeasurementV1<SizeBucketV1>;
  outputTokenCount: MeasurementV1<number>;
}

export interface ResourceRangeV1<T> {
  start: MeasurementV1<T>;
  average: MeasurementV1<T>;
  peak: MeasurementV1<T>;
  end: MeasurementV1<T>;
}

export interface ResourceSampleV1 {
  schemaVersion: 1;
  observedAtMs: number;
  host: {
    cpuPercent: MeasurementV1<number>;
  };
  mainProcess: {
    cpuPercent: MeasurementV1<number>;
    rssBytes: MeasurementV1<number>;
    rustHeapBytes: MeasurementV1<number>;
    ffiNativeHeapBytes: MeasurementV1<number>;
  };
  sidecarProcess: {
    cpuPercent: MeasurementV1<number>;
    rssBytes: MeasurementV1<number>;
  };
}

export interface ResourceSummaryV1 {
  sampleCount: number;
  host: { cpuPercent: ResourceRangeV1<number> };
  mainProcess: {
    cpuPercent: ResourceRangeV1<number>;
    rssBytes: ResourceRangeV1<number>;
    rustHeapBytes: ResourceRangeV1<number>;
    ffiNativeHeapBytes: ResourceRangeV1<number>;
  };
  sidecarProcess: {
    cpuPercent: ResourceRangeV1<number>;
    rssBytes: ResourceRangeV1<number>;
  };
}

export type RunOutcomeV1 =
  | { status: 'success' }
  | { status: 'noSpeech' }
  | { status: 'cancelled'; stage: PerformanceStageV1 }
  | { status: 'timedOut'; stage: PerformanceStageV1 }
  | {
      status: 'failed' | 'interrupted';
      stage: PerformanceStageV1;
      errorCode:
        | 'audioCaptureFailed'
        | 'decodeFailed'
        | 'vadFailed'
        | 'modelFailed'
        | 'inferenceFailed'
        | 'transformStageFailed'
        | 'deliveryFailed'
        | 'internalEarlyExit'
        | 'interruptedByRestart';
    };

export interface PerformanceRunV1 {
  schemaVersion: 1;
  runId: string;
  kind: PerformanceRunKindV1;
  startedAtMs: number;
  finishedAtMs: number;
  appVersion: string;
  correlation: RunCorrelationV1;
  outcome: RunOutcomeV1;
  runtimes: RuntimeIdentityV1[];
  stages: StageTimingV1[];
  input: ContentFreeInputSummaryV1;
  resources: ResourceSummaryV1;
  followUps: Array<{
    kind: 'apply' | 'undo';
    atMs: number;
    durationMs: MeasurementV1<number>;
    outcome: StageOutcomeV1;
  }>;
}

export interface PerformanceRunListV1 {
  schemaVersion: 1;
  runs: PerformanceRunV1[];
}

const unavailableReasons = new Set<UnavailableReasonV1>([
  'unsupportedPlatform',
  'sampleFailed',
  'noSamples',
  'dependencyPending',
]);
export const PERFORMANCE_STAGES_V1: readonly PerformanceStageV1[] = [
  'captureFinalization', 'fileDecode', 'vad', 'modelQueue', 'modelLoad',
  'inferenceDecode', 'transcriptTransform', 'cleanup', 'voiceCommands',
  'smartCorrection', 'smartFormatting', 'ideContext', 'cliCommand',
  'fileOutput', 'clipboardPaste', 'fileReturn', 'totalProcessing',
  'selectedTextCapture', 'instructionCapture', 'instructionAsr',
  'sidecarSpawnLoad', 'generation', 'reviewReady', 'apply', 'undo',
] as const;
const performanceStages = new Set<PerformanceStageV1>(PERFORMANCE_STAGES_V1);
const sizeBuckets = new Set<SizeBucketV1>([
  'empty', 'tiny', 'small', 'medium', 'large', 'extraLarge',
]);
const stageOutcomes = new Set<StageOutcomeV1>([
  'completed', 'skipped', 'fallback', 'failed',
]);
const stableErrors = new Set([
  'audioCaptureFailed', 'decodeFailed', 'vadFailed', 'modelFailed',
  'inferenceFailed', 'transformStageFailed', 'deliveryFailed',
  'internalEarlyExit', 'interruptedByRestart',
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function isMeasurementV1<T>(
  value: unknown,
  isValue: (candidate: unknown) => candidate is T,
): value is MeasurementV1<T> {
  if (!isRecord(value) || typeof value.status !== 'string') return false;
  if (value.status === 'measured') return isValue(value.value);
  if (value.status === 'notApplicable') return !('value' in value);
  return value.status === 'unavailable'
    && typeof value.reason === 'string'
    && unavailableReasons.has(value.reason as UnavailableReasonV1);
}

export function measuredValue<T>(measurement: MeasurementV1<T>): T | null {
  return measurement.status === 'measured' ? measurement.value : null;
}

export function isResourceSampleV1(value: unknown): value is ResourceSampleV1 {
  if (!isRecord(value)
    || value.schemaVersion !== 1
    || !isFiniteNumber(value.observedAtMs)
    || !isRecord(value.host)
    || !isRecord(value.mainProcess)
    || !isRecord(value.sidecarProcess)) {
    return false;
  }
  const numberMeasurement = (candidate: unknown): candidate is MeasurementV1<number> =>
    isMeasurementV1(candidate, isFiniteNumber);
  return numberMeasurement(value.host.cpuPercent)
    && numberMeasurement(value.mainProcess.cpuPercent)
    && numberMeasurement(value.mainProcess.rssBytes)
    && numberMeasurement(value.mainProcess.rustHeapBytes)
    && numberMeasurement(value.mainProcess.ffiNativeHeapBytes)
    && numberMeasurement(value.sidecarProcess.cpuPercent)
    && numberMeasurement(value.sidecarProcess.rssBytes);
}

function isStage(value: unknown): value is PerformanceStageV1 {
  return typeof value === 'string'
    && performanceStages.has(value as PerformanceStageV1);
}

function isStageTiming(value: unknown): value is StageTimingV1 {
  return isRecord(value)
    && isStage(value.stage)
    && isMeasurementV1(value.durationMs, isFiniteNumber)
    && typeof value.outcome === 'string'
    && stageOutcomes.has(value.outcome as StageOutcomeV1);
}

function isRuntime(value: unknown): value is RuntimeIdentityV1 {
  return isRecord(value)
    && ['transcription', 'instructionAsr', 'generation'].includes(String(value.role))
    && typeof value.modelId === 'string'
    && ['whisper', 'parakeet', 'coreml', 'llamaCpp'].includes(String(value.backend))
    && ['cpu', 'metalGpu', 'appleNeuralEngine', 'platformFallback'].includes(String(value.accelerator))
    && ['warm', 'coldLoaded', 'unknown'].includes(String(value.warmState));
}

function isInputSummary(value: unknown): value is ContentFreeInputSummaryV1 {
  return isRecord(value)
    && isMeasurementV1(value.audioDurationMs, isFiniteNumber)
    && isMeasurementV1(
      value.inputSizeBucket,
      (candidate): candidate is SizeBucketV1 =>
        typeof candidate === 'string' && sizeBuckets.has(candidate as SizeBucketV1),
    )
    && isMeasurementV1(
      value.outputSizeBucket,
      (candidate): candidate is SizeBucketV1 =>
        typeof candidate === 'string' && sizeBuckets.has(candidate as SizeBucketV1),
    )
    && isMeasurementV1(value.outputTokenCount, isFiniteNumber);
}

function isResourceRange(value: unknown): value is ResourceRangeV1<number> {
  return isRecord(value)
    && isMeasurementV1(value.start, isFiniteNumber)
    && isMeasurementV1(value.average, isFiniteNumber)
    && isMeasurementV1(value.peak, isFiniteNumber)
    && isMeasurementV1(value.end, isFiniteNumber);
}

function isResourceSummary(value: unknown): value is ResourceSummaryV1 {
  return isRecord(value)
    && isFiniteNumber(value.sampleCount)
    && isRecord(value.host)
    && isResourceRange(value.host.cpuPercent)
    && isRecord(value.mainProcess)
    && isResourceRange(value.mainProcess.cpuPercent)
    && isResourceRange(value.mainProcess.rssBytes)
    && isResourceRange(value.mainProcess.rustHeapBytes)
    && isResourceRange(value.mainProcess.ffiNativeHeapBytes)
    && isRecord(value.sidecarProcess)
    && isResourceRange(value.sidecarProcess.cpuPercent)
    && isResourceRange(value.sidecarProcess.rssBytes);
}

function isRunOutcome(value: unknown): value is RunOutcomeV1 {
  if (!isRecord(value) || typeof value.status !== 'string') return false;
  if (value.status === 'success' || value.status === 'noSpeech') return true;
  if (value.status === 'cancelled' || value.status === 'timedOut') {
    return isStage(value.stage);
  }
  return (value.status === 'failed' || value.status === 'interrupted')
    && isStage(value.stage)
    && typeof value.errorCode === 'string'
    && stableErrors.has(value.errorCode);
}

function isFollowUp(value: unknown): boolean {
  return isRecord(value)
    && (value.kind === 'apply' || value.kind === 'undo')
    && isFiniteNumber(value.atMs)
    && isMeasurementV1(value.durationMs, isFiniteNumber)
    && typeof value.outcome === 'string'
    && stageOutcomes.has(value.outcome as StageOutcomeV1);
}

export function isPerformanceRunV1(value: unknown): value is PerformanceRunV1 {
  if (!isRecord(value)
    || value.schemaVersion !== 1
    || typeof value.runId !== 'string'
    || !/^[a-f0-9]{32}$/.test(value.runId)
    || !['dictation', 'fileTranscription', 'selectedTextTransform'].includes(String(value.kind))
    || !isFiniteNumber(value.startedAtMs)
    || !isFiniteNumber(value.finishedAtMs)
    || typeof value.appVersion !== 'string'
    || !isRecord(value.correlation)
    || !isRunOutcome(value.outcome)
    || !Array.isArray(value.runtimes)
    || !Array.isArray(value.stages)
    || !isInputSummary(value.input)
    || !isResourceSummary(value.resources)
    || !Array.isArray(value.followUps)
    || !value.runtimes.every(isRuntime)
    || !value.stages.every(isStageTiming)
    || !value.followUps.every(isFollowUp)) {
    return false;
  }
  const stages = new Set(value.stages.map(stage => isRecord(stage) ? stage.stage : undefined));
  if (stages.size !== PERFORMANCE_STAGES_V1.length
    || !PERFORMANCE_STAGES_V1.every(stage => stages.has(stage))) {
    return false;
  }
  const correlationMatches =
    (value.kind === 'dictation'
      && value.correlation.kind === 'dictation'
      && isFiniteNumber(value.correlation.recordingId))
    || (value.kind === 'fileTranscription'
      && value.correlation.kind === 'fileTranscription'
      && isFiniteNumber(value.correlation.fileRunId))
    || (value.kind === 'selectedTextTransform'
      && value.correlation.kind === 'selectedTextTransform'
      && isFiniteNumber(value.correlation.transformPassId));
  return correlationMatches;
}

export async function listPerformanceRuns(limit = 50): Promise<PerformanceRunListV1> {
  const value = await invoke<unknown>('list_performance_runs', { limit });
  if (!isRecord(value)
    || value.schemaVersion !== 1
    || !Array.isArray(value.runs)
    || !value.runs.every(isPerformanceRunV1)) {
    throw new Error('Murmur returned an unsupported performance-run schema.');
  }
  return value as unknown as PerformanceRunListV1;
}

export async function getPerformanceRun(runId: string): Promise<PerformanceRunV1 | null> {
  const value = await invoke<unknown>('get_performance_run', { runId });
  if (value === null) return null;
  if (!isPerformanceRunV1(value)) {
    throw new Error('Murmur returned an unsupported performance-run schema.');
  }
  return value;
}

export async function getPerformanceResourceWindow(): Promise<ResourceSampleV1[]> {
  const value = await invoke<unknown>('get_performance_resource_window');
  if (!Array.isArray(value) || !value.every(isResourceSampleV1)) {
    throw new Error('Murmur returned an unsupported resource-sample schema.');
  }
  return value;
}

export async function clearPerformanceDiagnostics(): Promise<void> {
  await invoke('clear_performance_diagnostics');
}

export function onPerformanceRunCompleted(
  callback: (run: PerformanceRunV1) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('performance-run-completed', (event) => {
    if (isPerformanceRunV1(event.payload)) callback(event.payload);
  });
}

export function onPerformanceResourceSample(
  callback: (sample: ResourceSampleV1) => void,
): Promise<UnlistenFn> {
  return listen<unknown>('performance-resource-sample', (event) => {
    if (isResourceSampleV1(event.payload)) callback(event.payload);
  });
}
