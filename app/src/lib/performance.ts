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
  | 'selectedTextTransform'
  | 'voiceQuery';

export type RunCorrelationV1 =
  | { kind: 'dictation'; recordingId: number }
  | { kind: 'fileTranscription'; fileRunId: number }
  | { kind: 'selectedTextTransform'; transformPassId: number }
  | { kind: 'voiceQuery'; queryPassId: number };

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
  | 'spokenStructure'
  | 'spokenNumbers'
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
        | 'queryFailed'
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
  /** Voice Query child facts only. Never contains stderr text or provider detail. */
  queryProcess?: {
    exitCode: number | null;
    stderrPresent: boolean;
  };
}

export interface PerformanceRunListV1 {
  schemaVersion: 1;
  runs: PerformanceRunV1[];
}

export type PerformanceStoreErrorClassV1 =
  | 'busyLocked'
  | 'storageFull'
  | 'readOnly'
  | 'io'
  | 'corruptIntegrity'
  | 'schemaMigration'
  | 'invalidRecord'
  | 'unavailable';

export type PerformanceStoreOperationV1 =
  | 'initialize'
  | 'begin'
  | 'update'
  | 'complete'
  | 'read'
  | 'write'
  | 'clear';

export type PerformanceStoreRecommendedActionV1 =
  | 'none'
  | 'retry'
  | 'freeDisk'
  | 'checkPermissions'
  | 'reinitializeStore'
  | 'restartApp';

export interface PerformanceStoreFailureV1 {
  operation: PerformanceStoreOperationV1;
  errorClass: PerformanceStoreErrorClassV1;
  attemptCount: number;
  retryExhausted: boolean;
  atMs: number;
  recordingId?: number;
}

export interface PerformanceStoreHealthV1 {
  schemaVersion: 1;
  status: 'available' | 'unavailable';
  skippedRunCount: number;
  lastFailure?: PerformanceStoreFailureV1;
  recommendedAction: PerformanceStoreRecommendedActionV1;
  lastRecovery?: {
    action: 'quarantinedAndReinitialized';
    atMs: number;
  };
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
  'smartCorrection', 'smartFormatting', 'spokenStructure', 'spokenNumbers',
  'ideContext', 'cliCommand',
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
  'queryFailed', 'internalEarlyExit', 'interruptedByRestart',
]);
const performanceStoreErrorClasses = new Set<PerformanceStoreErrorClassV1>([
  'busyLocked', 'storageFull', 'readOnly', 'io', 'corruptIntegrity',
  'schemaMigration', 'invalidRecord', 'unavailable',
]);
const performanceStoreOperations = new Set<PerformanceStoreOperationV1>([
  'initialize', 'begin', 'update', 'complete', 'read', 'write', 'clear',
]);
const performanceStoreRecommendedActions = new Set<PerformanceStoreRecommendedActionV1>([
  'none', 'retry', 'freeDisk', 'checkPermissions', 'reinitializeStore', 'restartApp',
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  return actual.length === expected.length
    && actual.every((key, index) => key === expected[index]);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isPositiveSafeInteger(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value > 0;
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === 'number'
    && Number.isSafeInteger(value)
    && value >= 0;
}

export function isPerformanceStoreHealthV1(
  value: unknown,
): value is PerformanceStoreHealthV1 {
  if (!isRecord(value)) return false;
  const expectedKeys = [
    'schemaVersion', 'status', 'skippedRunCount', 'recommendedAction',
  ];
  if ('lastFailure' in value) expectedKeys.push('lastFailure');
  if ('lastRecovery' in value) expectedKeys.push('lastRecovery');
  if (!hasExactKeys(value, expectedKeys)
    || value.schemaVersion !== 1
    || (value.status !== 'available' && value.status !== 'unavailable')
    || !isNonNegativeSafeInteger(value.skippedRunCount)
    || typeof value.recommendedAction !== 'string'
    || !performanceStoreRecommendedActions.has(
      value.recommendedAction as PerformanceStoreRecommendedActionV1,
    )) {
    return false;
  }
  if ('lastFailure' in value) {
    if (!isRecord(value.lastFailure)) return false;
    const failureKeys = [
      'operation', 'errorClass', 'attemptCount', 'retryExhausted', 'atMs',
    ];
    if ('recordingId' in value.lastFailure) failureKeys.push('recordingId');
    if (!hasExactKeys(value.lastFailure, failureKeys)
      || typeof value.lastFailure.operation !== 'string'
      || !performanceStoreOperations.has(
        value.lastFailure.operation as PerformanceStoreOperationV1,
      )
      || typeof value.lastFailure.errorClass !== 'string'
      || !performanceStoreErrorClasses.has(
        value.lastFailure.errorClass as PerformanceStoreErrorClassV1,
      )
      || !isPositiveSafeInteger(value.lastFailure.attemptCount)
      || value.lastFailure.attemptCount > 3
      || typeof value.lastFailure.retryExhausted !== 'boolean'
      || value.lastFailure.retryExhausted !== (
        value.lastFailure.errorClass === 'busyLocked'
        && value.lastFailure.attemptCount === 3
      )
      || !isNonNegativeSafeInteger(value.lastFailure.atMs)
      || ('recordingId' in value.lastFailure
        && !isPositiveSafeInteger(value.lastFailure.recordingId))) {
      return false;
    }
  }
  if ('lastRecovery' in value) {
    if (!isRecord(value.lastRecovery)
      || !hasExactKeys(value.lastRecovery, ['action', 'atMs'])
      || value.lastRecovery.action !== 'quarantinedAndReinitialized'
      || !isNonNegativeSafeInteger(value.lastRecovery.atMs)) {
      return false;
    }
  }
  return true;
}

export function isMeasurementV1<T>(
  value: unknown,
  isValue: (candidate: unknown) => candidate is T,
): value is MeasurementV1<T> {
  if (!isRecord(value) || typeof value.status !== 'string') return false;
  if (value.status === 'measured') {
    return hasExactKeys(value, ['status', 'value']) && isValue(value.value);
  }
  if (value.status === 'notApplicable') return hasExactKeys(value, ['status']);
  return value.status === 'unavailable'
    && hasExactKeys(value, ['status', 'reason'])
    && typeof value.reason === 'string'
    && unavailableReasons.has(value.reason as UnavailableReasonV1);
}

export function measuredValue<T>(measurement: MeasurementV1<T>): T | null {
  return measurement.status === 'measured' ? measurement.value : null;
}

export function isResourceSampleV1(value: unknown): value is ResourceSampleV1 {
  if (!isRecord(value)
    || !hasExactKeys(value, ['schemaVersion', 'observedAtMs', 'host', 'mainProcess', 'sidecarProcess'])
    || value.schemaVersion !== 1
    || !isFiniteNumber(value.observedAtMs)
    || !isRecord(value.host)
    || !hasExactKeys(value.host, ['cpuPercent'])
    || !isRecord(value.mainProcess)
    || !hasExactKeys(value.mainProcess, [
      'cpuPercent', 'rssBytes', 'rustHeapBytes', 'ffiNativeHeapBytes',
    ])
    || !isRecord(value.sidecarProcess)
    || !hasExactKeys(value.sidecarProcess, ['cpuPercent', 'rssBytes'])) {
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
    && hasExactKeys(value, ['stage', 'durationMs', 'outcome'])
    && isStage(value.stage)
    && isMeasurementV1(value.durationMs, isFiniteNumber)
    && typeof value.outcome === 'string'
    && stageOutcomes.has(value.outcome as StageOutcomeV1);
}

function isRuntime(value: unknown): value is RuntimeIdentityV1 {
  return isRecord(value)
    && hasExactKeys(value, ['role', 'modelId', 'backend', 'accelerator', 'warmState'])
    && ['transcription', 'instructionAsr', 'generation'].includes(String(value.role))
    && typeof value.modelId === 'string'
    && ['whisper', 'parakeet', 'coreml', 'llamaCpp'].includes(String(value.backend))
    && ['cpu', 'metalGpu', 'appleNeuralEngine', 'platformFallback'].includes(String(value.accelerator))
    && ['warm', 'coldLoaded', 'unknown'].includes(String(value.warmState));
}

function isInputSummary(value: unknown): value is ContentFreeInputSummaryV1 {
  return isRecord(value)
    && hasExactKeys(value, [
      'audioDurationMs', 'inputSizeBucket', 'outputSizeBucket', 'outputTokenCount',
    ])
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
    && hasExactKeys(value, ['start', 'average', 'peak', 'end'])
    && isMeasurementV1(value.start, isFiniteNumber)
    && isMeasurementV1(value.average, isFiniteNumber)
    && isMeasurementV1(value.peak, isFiniteNumber)
    && isMeasurementV1(value.end, isFiniteNumber);
}

function isResourceSummary(value: unknown): value is ResourceSummaryV1 {
  return isRecord(value)
    && hasExactKeys(value, ['sampleCount', 'host', 'mainProcess', 'sidecarProcess'])
    && isFiniteNumber(value.sampleCount)
    && isRecord(value.host)
    && hasExactKeys(value.host, ['cpuPercent'])
    && isResourceRange(value.host.cpuPercent)
    && isRecord(value.mainProcess)
    && hasExactKeys(value.mainProcess, [
      'cpuPercent', 'rssBytes', 'rustHeapBytes', 'ffiNativeHeapBytes',
    ])
    && isResourceRange(value.mainProcess.cpuPercent)
    && isResourceRange(value.mainProcess.rssBytes)
    && isResourceRange(value.mainProcess.rustHeapBytes)
    && isResourceRange(value.mainProcess.ffiNativeHeapBytes)
    && isRecord(value.sidecarProcess)
    && hasExactKeys(value.sidecarProcess, ['cpuPercent', 'rssBytes'])
    && isResourceRange(value.sidecarProcess.cpuPercent)
    && isResourceRange(value.sidecarProcess.rssBytes);
}

function isRunOutcome(value: unknown): value is RunOutcomeV1 {
  if (!isRecord(value) || typeof value.status !== 'string') return false;
  if (value.status === 'success' || value.status === 'noSpeech') {
    return hasExactKeys(value, ['status']);
  }
  if (value.status === 'cancelled' || value.status === 'timedOut') {
    return hasExactKeys(value, ['status', 'stage']) && isStage(value.stage);
  }
  return (value.status === 'failed' || value.status === 'interrupted')
    && hasExactKeys(value, ['status', 'stage', 'errorCode'])
    && isStage(value.stage)
    && typeof value.errorCode === 'string'
    && stableErrors.has(value.errorCode);
}

function isFollowUp(value: unknown): boolean {
  return isRecord(value)
    && hasExactKeys(value, ['kind', 'atMs', 'durationMs', 'outcome'])
    && (value.kind === 'apply' || value.kind === 'undo')
    && isFiniteNumber(value.atMs)
    && isMeasurementV1(value.durationMs, isFiniteNumber)
    && typeof value.outcome === 'string'
    && stageOutcomes.has(value.outcome as StageOutcomeV1);
}

function isQueryProcess(value: unknown): boolean {
  return isRecord(value)
    && hasExactKeys(value, ['exitCode', 'stderrPresent'])
    && (value.exitCode === null || (
      typeof value.exitCode === 'number'
      && Number.isSafeInteger(value.exitCode)
      && value.exitCode >= -2_147_483_648
      && value.exitCode <= 2_147_483_647
    ))
    && typeof value.stderrPresent === 'boolean';
}

function isRunCorrelation(value: unknown, kind: PerformanceRunKindV1): value is RunCorrelationV1 {
  if (!isRecord(value) || value.kind !== kind) return false;
  switch (kind) {
    case 'dictation':
      return hasExactKeys(value, ['kind', 'recordingId'])
        && isFiniteNumber(value.recordingId);
    case 'fileTranscription':
      return hasExactKeys(value, ['kind', 'fileRunId'])
        && isFiniteNumber(value.fileRunId);
    case 'selectedTextTransform':
      return hasExactKeys(value, ['kind', 'transformPassId'])
        && isFiniteNumber(value.transformPassId);
    case 'voiceQuery':
      return hasExactKeys(value, ['kind', 'queryPassId'])
        && isPositiveSafeInteger(value.queryPassId);
  }
}

export function isPerformanceRunV1(value: unknown): value is PerformanceRunV1 {
  if (!isRecord(value)) return false;
  const expectedKeys = [
    'schemaVersion', 'runId', 'kind', 'startedAtMs', 'finishedAtMs', 'appVersion',
    'correlation', 'outcome', 'runtimes', 'stages', 'input', 'resources', 'followUps',
  ];
  if ('queryProcess' in value) expectedKeys.push('queryProcess');
  if (!hasExactKeys(value, expectedKeys)
    || value.schemaVersion !== 1
    || typeof value.runId !== 'string'
    || !/^[a-f0-9]{32}$/.test(value.runId)
    || !['dictation', 'fileTranscription', 'selectedTextTransform', 'voiceQuery'].includes(String(value.kind))
    || !isFiniteNumber(value.startedAtMs)
    || !isFiniteNumber(value.finishedAtMs)
    || typeof value.appVersion !== 'string'
    || !isRunOutcome(value.outcome)
    || !Array.isArray(value.runtimes)
    || !Array.isArray(value.stages)
    || !isInputSummary(value.input)
    || !isResourceSummary(value.resources)
    || !Array.isArray(value.followUps)
    || !value.runtimes.every(isRuntime)
    || !value.stages.every(isStageTiming)
    || !value.followUps.every(isFollowUp)
    || ('queryProcess' in value && !isQueryProcess(value.queryProcess))) {
    return false;
  }
  const stages = new Set(value.stages.map(stage => isRecord(stage) ? stage.stage : undefined));
  if (stages.size !== PERFORMANCE_STAGES_V1.length
    || !PERFORMANCE_STAGES_V1.every(stage => stages.has(stage))) {
    return false;
  }
  return isRunCorrelation(value.correlation, value.kind as PerformanceRunKindV1)
    && (value.kind === 'voiceQuery' || !('queryProcess' in value));
}

export async function listPerformanceRuns(limit = 50): Promise<PerformanceRunListV1> {
  const value = await invoke<unknown>('list_performance_runs', { limit });
  if (!isRecord(value)
    || !hasExactKeys(value, ['schemaVersion', 'runs'])
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

export async function getPerformanceStoreHealth(): Promise<PerformanceStoreHealthV1> {
  const value = await invoke<unknown>('get_performance_store_health');
  if (!isPerformanceStoreHealthV1(value)) {
    throw new Error('Murmur returned an unsupported diagnostics-health schema.');
  }
  return value;
}

export async function recoverPerformanceStore(
  allowReinitialize: boolean,
): Promise<PerformanceStoreHealthV1> {
  const value = await invoke<unknown>('recover_performance_store', { allowReinitialize });
  if (!isPerformanceStoreHealthV1(value)) {
    throw new Error('Murmur returned an unsupported diagnostics-health schema.');
  }
  return value;
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

export function onPerformanceDiagnosticsCleared(
  callback: () => void,
): Promise<UnlistenFn> {
  return listen('performance-diagnostics-cleared', callback);
}
