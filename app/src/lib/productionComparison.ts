import type {
  PerformanceRunV1,
  PerformanceStageV1,
  ProductionRunV1,
  RuntimeIdentityV1,
  SizeBucketV1,
} from './performance';
import { median } from './numeric';

export const PRODUCTION_COMPARISON_THRESHOLDS = {
  median: { absoluteMs: 20, relativePercent: 15, minimumSamples: 1 },
  p95: { absoluteMs: 30, relativePercent: 20, minimumSamples: 20 },
} as const;

type CaptureMetricKey = keyof Pick<ProductionRunV1['capture'],
    | 'helperResolveMs'
    | 'helperSignatureMs'
    | 'helperSpawnMs'
    | 'streamOpenMs'
    | 'firstCallbackWaitMs'
    | 'firstPcmMs'
    | 'readyMs'
    | 'stopToWorkerExitMs'>;
type StageMetricKey = Extract<PerformanceStageV1,
    | 'captureFinalization'
    | 'vad'
    | 'inferenceDecode'
    | 'transcriptTransform'
    | 'clipboardPaste'
    | 'totalProcessing'>;
export type ProductionMetricKey = CaptureMetricKey | StageMetricKey;

export type ProductionMetricDefinition = {
  key: CaptureMetricKey;
  label: string;
  source: 'capture';
} | {
  key: StageMetricKey;
  label: string;
  source: 'stage';
};

export const PRODUCTION_COMPARISON_METRICS: readonly ProductionMetricDefinition[] = [
  { key: 'helperResolveMs', label: 'Helper resolve', source: 'capture' },
  { key: 'helperSignatureMs', label: 'Helper signature', source: 'capture' },
  { key: 'helperSpawnMs', label: 'Helper spawn', source: 'capture' },
  { key: 'streamOpenMs', label: 'Stream open', source: 'capture' },
  { key: 'firstCallbackWaitMs', label: 'First callback wait', source: 'capture' },
  { key: 'firstPcmMs', label: 'First PCM', source: 'capture' },
  { key: 'readyMs', label: 'Capture ready', source: 'capture' },
  { key: 'stopToWorkerExitMs', label: 'Worker exit', source: 'capture' },
  { key: 'captureFinalization', label: 'Capture finalization', source: 'stage' },
  { key: 'vad', label: 'VAD', source: 'stage' },
  { key: 'inferenceDecode', label: 'Inference / decode', source: 'stage' },
  { key: 'transcriptTransform', label: 'Transcript transform', source: 'stage' },
  { key: 'clipboardPaste', label: 'Clipboard / paste', source: 'stage' },
  { key: 'totalProcessing', label: 'Total processing', source: 'stage' },
];

export type RecordingDurationBucket =
  | 'empty'
  | 'under5s'
  | '5to15s'
  | '15to60s'
  | 'over60s';

export type ProductionExclusionReason =
  | 'historicalMetadataMissing'
  | 'developmentBuild'
  | 'osVersionUnknown'
  | 'architectureUnknown'
  | 'microphoneKindUnknown'
  | 'configurationUnknown'
  | 'captureBackendUnknown'
  | 'runtimeUnknown'
  | 'audioDurationUnknown'
  | 'outputSizeUnknown';

export const PRODUCTION_EXCLUSION_LABELS: Record<ProductionExclusionReason, string> = {
  historicalMetadataMissing: 'Historical run without production cohort metadata',
  developmentBuild: 'Development build',
  osVersionUnknown: 'Operating-system version unavailable',
  architectureUnknown: 'Architecture unavailable',
  microphoneKindUnknown: 'Microphone class or transport unavailable',
  configurationUnknown: 'Supported configuration identity unavailable',
  captureBackendUnknown: 'Actual capture backend unavailable',
  runtimeUnknown: 'Runtime identity or warm state unavailable',
  audioDurationUnknown: 'Audio duration unavailable',
  outputSizeUnknown: 'Output-size bucket unavailable',
};
const productionExclusionReasons: readonly ProductionExclusionReason[] = [
  'historicalMetadataMissing',
  'developmentBuild',
  'osVersionUnknown',
  'architectureUnknown',
  'microphoneKindUnknown',
  'configurationUnknown',
  'captureBackendUnknown',
  'runtimeUnknown',
  'audioDurationUnknown',
  'outputSizeUnknown',
];

export type CohortDimension =
  | 'machine'
  | 'operating system'
  | 'architecture'
  | 'build mode'
  | 'microphone selection'
  | 'microphone kind'
  | 'configuration'
  | 'runtime'
  | 'capture backend'
  | 'recording duration'
  | 'output size';

interface CohortIdentity {
  machineId: string;
  osVersion: string;
  architecture: ProductionRunV1['cohort']['architecture'];
  buildMode: 'release';
  microphoneSelection: ProductionRunV1['cohort']['microphoneSelection'];
  microphoneKind: NonNullable<ProductionRunV1['cohort']['microphoneKind']>;
  configurationKey: string;
  runtimes: string[];
  captureBackend: NonNullable<ProductionRunV1['capture']['backend']>;
  recordingDurationBucket: RecordingDurationBucket;
  outputSizeBucket: SizeBucketV1;
}

interface EligibleRun {
  run: PerformanceRunV1;
  identity: CohortIdentity;
  key: string;
}

export interface VersionSelectionOption {
  appVersion: string;
  runCount: number;
  latestStartedAtMs: number;
}

export interface ExclusionCount {
  reason: ProductionExclusionReason;
  label: string;
  baseline: number;
  candidate: number;
}

export interface MetricSampleSummary {
  measuredCount: number;
  missingCount: number;
  medianMs: number | null;
  p95Ms: number | null;
  rawMs: number[];
}

export type RegressionVerdict = 'regression' | 'withinThreshold' | 'unavailable';

export interface StatisticComparison {
  baselineMs: number | null;
  candidateMs: number | null;
  absoluteDeltaMs: number | null;
  percentageDelta: number | null;
  verdict: RegressionVerdict;
}

export interface ProductionMetricComparison {
  definition: ProductionMetricDefinition;
  baseline: MetricSampleSummary;
  candidate: MetricSampleSummary;
  median: StatisticComparison;
  p95: StatisticComparison;
  preliminary: boolean;
}

export interface ProductionCohortComparison {
  key: string;
  label: string;
  baselineRunCount: number;
  candidateRunCount: number;
  baselineSuccessCount: number;
  candidateSuccessCount: number;
  metrics: ProductionMetricComparison[];
}

export type VersionObservationKey =
  | 'success'
  | 'failure'
  | 'fallbackAttempted'
  | 'captureFailure'
  | 'zeroSampleSuccess'
  | 'workerInvariantViolation';

export interface VersionObservation {
  key: VersionObservationKey;
  label: string;
  baseline: number;
  candidate: number;
  baselineUnavailable: number;
  candidateUnavailable: number;
  newlyObserved: boolean;
  candidateObserved: boolean;
}

export interface ProductionVersionComparison {
  baselineVersion: string;
  candidateVersion: string;
  baselineDictationCount: number;
  candidateDictationCount: number;
  baselineEligibleCount: number;
  candidateEligibleCount: number;
  baselineUnmatchedCount: number;
  candidateUnmatchedCount: number;
  exclusions: ExclusionCount[];
  mismatchedDimensions: CohortDimension[];
  cohorts: ProductionCohortComparison[];
  observations: VersionObservation[];
}

export function productionVersionOptions(
  runs: readonly PerformanceRunV1[],
): VersionSelectionOption[] {
  const versions = new Map<string, VersionSelectionOption>();
  for (const run of runs) {
    if (run.kind !== 'dictation') continue;
    const current = versions.get(run.appVersion);
    versions.set(run.appVersion, {
      appVersion: run.appVersion,
      runCount: (current?.runCount ?? 0) + 1,
      latestStartedAtMs: Math.max(current?.latestStartedAtMs ?? 0, run.startedAtMs),
    });
  }
  return [...versions.values()].sort((left, right) =>
    right.latestStartedAtMs - left.latestStartedAtMs
      || right.appVersion.localeCompare(left.appVersion));
}

export function recordingDurationBucket(durationMs: number): RecordingDurationBucket {
  if (durationMs === 0) return 'empty';
  if (durationMs <= 5_000) return 'under5s';
  if (durationMs <= 15_000) return '5to15s';
  if (durationMs <= 60_000) return '15to60s';
  return 'over60s';
}

function runtimeIdentity(runtime: RuntimeIdentityV1): string {
  return [runtime.role, runtime.modelId, runtime.backend, runtime.accelerator, runtime.warmState]
    .join('\u0000');
}

function exclusionReasons(run: PerformanceRunV1): ProductionExclusionReason[] {
  const reasons: ProductionExclusionReason[] = [];
  const production = run.production;
  if (!production) return ['historicalMetadataMissing'];
  if (production.cohort.buildMode !== 'release') reasons.push('developmentBuild');
  if (production.cohort.osVersion === null) reasons.push('osVersionUnknown');
  if (production.cohort.architecture === 'other') reasons.push('architectureUnknown');
  if (production.cohort.microphoneKind === null || production.cohort.microphoneKind === 'external') {
    reasons.push('microphoneKindUnknown');
  }
  if (production.cohort.configurationKey === null) reasons.push('configurationUnknown');
  if (production.capture.backend === null) reasons.push('captureBackendUnknown');
  if (run.runtimes.length === 0
    || run.runtimes.some(runtime => runtime.warmState === 'unknown' || runtime.modelId.length === 0)) {
    reasons.push('runtimeUnknown');
  }
  const duration = run.input.audioDurationMs.status === 'measured'
    ? run.input.audioDurationMs.value
    : null;
  if (duration === null || !Number.isSafeInteger(duration) || duration < 0) {
    reasons.push('audioDurationUnknown');
  }
  if (run.input.outputSizeBucket.status !== 'measured') {
    reasons.push('outputSizeUnknown');
  }
  return reasons;
}

function eligibleRun(run: PerformanceRunV1): EligibleRun | null {
  if (exclusionReasons(run).length > 0 || !run.production) return null;
  const { cohort, capture } = run.production;
  if (cohort.osVersion === null
    || cohort.microphoneKind === null
    || cohort.configurationKey === null
    || capture.backend === null
    || run.input.audioDurationMs.status !== 'measured'
    || run.input.outputSizeBucket.status !== 'measured') {
    return null;
  }
  const identity: CohortIdentity = {
    machineId: cohort.machineId,
    osVersion: cohort.osVersion,
    architecture: cohort.architecture,
    buildMode: 'release',
    microphoneSelection: cohort.microphoneSelection,
    microphoneKind: cohort.microphoneKind,
    configurationKey: cohort.configurationKey,
    runtimes: run.runtimes.map(runtimeIdentity).sort(),
    captureBackend: capture.backend,
    recordingDurationBucket: recordingDurationBucket(run.input.audioDurationMs.value),
    outputSizeBucket: run.input.outputSizeBucket.value,
  };
  return { run, identity, key: JSON.stringify(identity) };
}

function nearestRankP95(values: readonly number[]): number | null {
  if (values.length < PRODUCTION_COMPARISON_THRESHOLDS.p95.minimumSamples) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.ceil(0.95 * sorted.length) - 1];
}

export function summarizeMetricSamples(
  values: readonly (number | null)[],
): MetricSampleSummary {
  const measured = values.filter((value): value is number =>
    value !== null && Number.isSafeInteger(value) && value >= 0);
  return {
    measuredCount: measured.length,
    missingCount: values.length - measured.length,
    medianMs: median(measured),
    p95Ms: nearestRankP95(measured),
    rawMs: [...measured],
  };
}

export function compareStatistic(
  baselineMs: number | null,
  candidateMs: number | null,
  threshold: { absoluteMs: number; relativePercent: number },
): StatisticComparison {
  if (baselineMs === null || candidateMs === null) {
    return {
      baselineMs,
      candidateMs,
      absoluteDeltaMs: null,
      percentageDelta: null,
      verdict: 'unavailable',
    };
  }
  const absoluteDeltaMs = candidateMs - baselineMs;
  const percentageDelta = baselineMs === 0 ? null : (absoluteDeltaMs / baselineMs) * 100;
  return {
    baselineMs,
    candidateMs,
    absoluteDeltaMs,
    percentageDelta,
    verdict: percentageDelta === null
      ? 'unavailable'
      : absoluteDeltaMs > threshold.absoluteMs
        && percentageDelta > threshold.relativePercent
        ? 'regression'
        : 'withinThreshold',
  };
}

function metricValue(run: PerformanceRunV1, metric: ProductionMetricDefinition): number | null {
  if (metric.source === 'capture') {
    const production = run.production;
    if (!production) return null;
    const value = production.capture[metric.key];
    return typeof value === 'number' ? value : null;
  }
  const timing = run.stages.find(stage => stage.stage === metric.key);
  return timing?.durationMs.status === 'measured' ? timing.durationMs.value : null;
}

function metricComparison(
  definition: ProductionMetricDefinition,
  baselineRuns: readonly PerformanceRunV1[],
  candidateRuns: readonly PerformanceRunV1[],
): ProductionMetricComparison {
  const baseline = summarizeMetricSamples(baselineRuns.map(run => metricValue(run, definition)));
  const candidate = summarizeMetricSamples(candidateRuns.map(run => metricValue(run, definition)));
  return {
    definition,
    baseline,
    candidate,
    median: compareStatistic(
      baseline.medianMs,
      candidate.medianMs,
      PRODUCTION_COMPARISON_THRESHOLDS.median,
    ),
    p95: compareStatistic(
      baseline.p95Ms,
      candidate.p95Ms,
      PRODUCTION_COMPARISON_THRESHOLDS.p95,
    ),
    preliminary: baseline.measuredCount < PRODUCTION_COMPARISON_THRESHOLDS.p95.minimumSamples
      || candidate.measuredCount < PRODUCTION_COMPARISON_THRESHOLDS.p95.minimumSamples,
  };
}

function cohortLabel(identity: CohortIdentity): string {
  const runtime = identity.runtimes
    .map(value => value.split('\u0000').join(' · '))
    .join('; ');
  return [
    `installation ${identity.machineId.slice(0, 8)}`,
    `macOS ${identity.osVersion} ${identity.architecture}`,
    identity.buildMode,
    `${identity.microphoneSelection} ${identity.microphoneKind}`,
    identity.captureBackend.toUpperCase(),
    identity.recordingDurationBucket,
    `${identity.outputSizeBucket} output`,
    `config ${identity.configurationKey.slice(0, 8)}`,
    runtime,
  ].join(' · ');
}

const cohortDimensions: readonly [keyof CohortIdentity, CohortDimension][] = [
  ['machineId', 'machine'],
  ['osVersion', 'operating system'],
  ['architecture', 'architecture'],
  ['buildMode', 'build mode'],
  ['microphoneSelection', 'microphone selection'],
  ['microphoneKind', 'microphone kind'],
  ['configurationKey', 'configuration'],
  ['runtimes', 'runtime'],
  ['captureBackend', 'capture backend'],
  ['recordingDurationBucket', 'recording duration'],
  ['outputSizeBucket', 'output size'],
];

function dimensionDifferences(
  baseline: readonly EligibleRun[],
  candidate: readonly EligibleRun[],
): CohortDimension[] {
  const differences = new Set<CohortDimension>();
  for (const left of baseline) {
    for (const right of candidate) {
      for (const [key, label] of cohortDimensions) {
        if (JSON.stringify(left.identity[key]) !== JSON.stringify(right.identity[key])) {
          differences.add(label);
        }
      }
    }
  }
  return [...differences];
}

function observationValue(
  run: PerformanceRunV1,
  key: VersionObservationKey,
): true | false | null {
  if (key === 'success') return run.outcome.status === 'success';
  if (key === 'failure') {
    return run.outcome.status === 'failed'
      || run.outcome.status === 'timedOut'
      || run.outcome.status === 'interrupted';
  }
  const capture = run.production?.capture;
  if (!capture) return null;
  if (key === 'fallbackAttempted') return capture.fallbackAttempted;
  if (key === 'captureFailure') return capture.failureKind === null ? null : true;
  if (key === 'zeroSampleSuccess') return capture.zeroSampleSuccess;
  return capture.workerInvariantViolation;
}

const observationLabels: Record<VersionObservationKey, string> = {
  success: 'Successful runs',
  failure: 'Failed, timed out, or interrupted',
  fallbackAttempted: 'Capture fallback attempted',
  captureFailure: 'Capture failure recorded',
  zeroSampleSuccess: 'Zero-sample success',
  workerInvariantViolation: 'Worker invariant violation',
};

function observations(
  baseline: readonly PerformanceRunV1[],
  candidate: readonly PerformanceRunV1[],
): VersionObservation[] {
  const keys: readonly VersionObservationKey[] = [
    'success',
    'failure',
    'fallbackAttempted',
    'captureFailure',
    'zeroSampleSuccess',
    'workerInvariantViolation',
  ];
  return keys.map(key => {
    const baselineValues = baseline.map(run => observationValue(run, key));
    const candidateValues = candidate.map(run => observationValue(run, key));
    const baselineCount = baselineValues.filter(value => value === true).length;
    const candidateCount = candidateValues.filter(value => value === true).length;
    const anomaly = key !== 'success';
    return {
      key,
      label: observationLabels[key],
      baseline: baselineCount,
      candidate: candidateCount,
      baselineUnavailable: baselineValues.filter(value => value === null).length,
      candidateUnavailable: candidateValues.filter(value => value === null).length,
      newlyObserved: anomaly && baselineValues.length > 0
        && baselineValues.every(value => value === false) && candidateCount > 0,
      candidateObserved: anomaly && candidateCount > 0,
    };
  });
}

function groupByCohortKey(entries: readonly EligibleRun[]): Map<string, EligibleRun[]> {
  const groups = new Map<string, EligibleRun[]>();
  for (const entry of entries) {
    const group = groups.get(entry.key);
    if (group) group.push(entry);
    else groups.set(entry.key, [entry]);
  }
  return groups;
}

export function compareProductionVersions(
  runs: readonly PerformanceRunV1[],
  baselineVersion: string,
  candidateVersion: string,
): ProductionVersionComparison {
  const baselineRuns = runs.filter(run =>
    run.kind === 'dictation' && run.appVersion === baselineVersion);
  const candidateRuns = runs.filter(run =>
    run.kind === 'dictation' && run.appVersion === candidateVersion);
  const baselineEligible = baselineRuns.flatMap(run => {
    const eligible = eligibleRun(run);
    return eligible ? [eligible] : [];
  });
  const candidateEligible = candidateRuns.flatMap(run => {
    const eligible = eligibleRun(run);
    return eligible ? [eligible] : [];
  });
  const baselineByKey = groupByCohortKey(baselineEligible);
  const candidateByKey = groupByCohortKey(candidateEligible);
  const matchedKeys = [...baselineByKey.keys()].filter(key => candidateByKey.has(key)).sort();
  const cohorts = matchedKeys.map(key => {
    const left = baselineByKey.get(key) ?? [];
    const right = candidateByKey.get(key) ?? [];
    const baselineSuccess = left.map(entry => entry.run)
      .filter(run => run.outcome.status === 'success');
    const candidateSuccess = right.map(entry => entry.run)
      .filter(run => run.outcome.status === 'success');
    return {
      key,
      label: cohortLabel(left[0].identity),
      baselineRunCount: left.length,
      candidateRunCount: right.length,
      baselineSuccessCount: baselineSuccess.length,
      candidateSuccessCount: candidateSuccess.length,
      metrics: PRODUCTION_COMPARISON_METRICS.map(definition =>
        metricComparison(definition, baselineSuccess, candidateSuccess)),
    };
  });
  const exclusions = productionExclusionReasons.flatMap(reason => {
    const baseline = baselineRuns.filter(run => exclusionReasons(run).includes(reason)).length;
    const candidate = candidateRuns.filter(run => exclusionReasons(run).includes(reason)).length;
    return baseline > 0 || candidate > 0
      ? [{ reason, label: PRODUCTION_EXCLUSION_LABELS[reason], baseline, candidate }]
      : [];
  });
  const matchedKeySet = new Set(matchedKeys);
  const baselineUnmatched = baselineEligible.filter(entry => !matchedKeySet.has(entry.key));
  const candidateUnmatched = candidateEligible.filter(entry => !matchedKeySet.has(entry.key));
  return {
    baselineVersion,
    candidateVersion,
    baselineDictationCount: baselineRuns.length,
    candidateDictationCount: candidateRuns.length,
    baselineEligibleCount: baselineEligible.length,
    candidateEligibleCount: candidateEligible.length,
    baselineUnmatchedCount: baselineUnmatched.length,
    candidateUnmatchedCount: candidateUnmatched.length,
    exclusions,
    mismatchedDimensions: dimensionDifferences(baselineUnmatched, candidateUnmatched),
    cohorts,
    observations: observations(baselineRuns, candidateRuns),
  };
}
