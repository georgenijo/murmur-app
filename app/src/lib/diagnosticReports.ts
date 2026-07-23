import type { BenchmarkReport } from './benchmark';

export const MAX_DIAGNOSTIC_REPORT_BYTES = 8 * 1024 * 1024;

export const DIAGNOSTIC_REPORT_LIMITS = {
  benchmarkModels: 64,
  benchmarkFixturesPerModel: 512,
  evaluationCases: 10_000,
  evaluationStagesPerCase: 128,
  stringsPerCollection: 512,
} as const;

export type DiagnosticReportSource = 'imported' | 'local';
export type DiagnosticReportKind = 'benchmark' | 'evaluation';

export type DiagnosticImportErrorCode =
  | 'empty'
  | 'oversized'
  | 'malformed_json'
  | 'unsupported_version'
  | 'schema_mismatch'
  | 'collection_limit';

export interface DiagnosticImportError {
  code: DiagnosticImportErrorCode;
  message: string;
}

export type DiagnosticImportResult =
  | { ok: true; report: NormalizedDiagnosticReport }
  | { ok: false; error: DiagnosticImportError };

export interface NormalizedBenchmarkFixture {
  fixtureId: string;
  audioSeconds: number;
  warmMedianMs: number;
  warmP95Ms: number;
  realtimeFactor: number;
  wordErrorRate: number;
  normalizedWordErrorRate: number;
  deliveredWordErrorRate: number;
  deliveredNormalizedWordErrorRate: number;
  deliveredTransformFailed: boolean;
}

export interface NormalizedBenchmarkModel {
  modelName: string;
  label: string;
  backend: string;
  accelerator: string;
  downloadSize: string | null;
  modelLoadMs: number | null;
  firstInferenceMs: number | null;
  warmMedianMs: number | null;
  warmP95Ms: number | null;
  realtimeFactor: number | null;
  wordErrorRate: number | null;
  normalizedWordErrorRate: number | null;
  deliveredWordErrorRate: number | null;
  deliveredNormalizedWordErrorRate: number | null;
  memoryDeltaMb: number;
  fixtures: NormalizedBenchmarkFixture[];
  succeeded: boolean;
}

export interface NormalizedBenchmarkReport {
  kind: 'benchmark';
  source: DiagnosticReportSource;
  schemaVersion: 'legacy' | 2;
  createdAt: string;
  appVersion: string;
  platform: string;
  preset: 'quick' | 'standard' | 'thorough';
  iterations: number;
  sharedInitMs: number;
  environment: {
    os: string;
    osVersion: string | null;
    architecture: string;
    hardwareModel: string | null;
    chip: string | null;
    memoryMb: number | null;
  } | null;
  corpus: {
    language: string;
    fixtureIds: string[];
    fixtureCount: number;
    referenceWords: number;
  } | null;
  configuration: {
    vadThreshold: number;
    executionPath: string;
    transcriptTransformProfile: string;
    percentileMethod: string;
    modelRunOrder: string[];
    sharedInitOrder: string[];
  } | null;
  models: NormalizedBenchmarkModel[];
  recommendations: {
    fastest: string | null;
    mostAccurate: string | null;
    balanced: string | null;
  };
  privacyWarnings: string[];
}

export interface NormalizedEvaluationCase {
  id: string;
  status: 'passed' | 'failed' | 'skipped';
  complete: boolean;
  fixtureOnly: boolean;
  model: {
    name: string;
    backend: string;
    accelerator: string;
  } | null;
  recognition: {
    rawWordErrors: number | null;
    normalizedWordErrors: number | null;
    referenceWords: number | null;
    normalizedReferenceWords: number | null;
    referenceCharacters: number | null;
    characterErrors: number | null;
    rawWer: number | null;
    normalizedWer: number | null;
    cer: number | null;
    boundedAlternativeMatch: boolean;
  };
  transformation: {
    exactMatch: boolean;
    commandExactMatch: boolean | null;
    noChangePreserved: boolean | null;
    stages: Array<{
      name: string;
      outcome: string;
      changed: boolean;
      durationUs: number;
      expectationMatch: boolean;
    }>;
  };
  delivery: {
    exactMatch: boolean;
    attempts: number;
    partialCount: number;
    firstPartialMs: number | null;
    firstPartialApplicability: string;
    finalOnly: boolean;
  };
  latency: {
    rawAsrMs: number;
    transformationMs: number;
    finalizationMs: number;
    deliveryMs: number;
    totalMs: number;
  };
  runtime: {
    incrementalCompletion: string;
    fallbackUsed: boolean;
    fallbackStages: string[];
    memoryBeforeMb: number | null;
    memoryAfterMb: number | null;
    memoryDeltaMb: number | null;
  };
}

export interface NormalizedEvaluationReport {
  kind: 'evaluation';
  source: DiagnosticReportSource;
  schemaVersion: 1;
  fixtureVersion: 1;
  createdAt: string;
  tier: 'deterministic' | 'hardware';
  environment: {
    appVersion: string;
    os: string;
    architecture: string;
    machineLabel: string;
    logicalCpus: number | null;
  };
  summary: {
    total: number;
    passed: number;
    failed: number;
    skipped: number;
    aggregateRawWer: number | null;
    aggregateNormalizedWer: number | null;
    aggregateCer: number | null;
    transformationMatchRate: number | null;
    commandExactMatchRate: number | null;
    noChangePreservationRate: number | null;
    deliveryMatchRate: number | null;
  };
  cases: NormalizedEvaluationCase[];
  privacyWarnings: string[];
}

export type NormalizedDiagnosticReport =
  | NormalizedBenchmarkReport
  | NormalizedEvaluationReport;

type JsonRecord = Record<string, unknown>;

interface ValidationContext {
  collectionExceeded: boolean;
}

const ERROR_MESSAGES: Record<DiagnosticImportErrorCode, string> = {
  empty: 'The selected diagnostic report is empty.',
  oversized: 'Diagnostic reports are limited to 8 MiB.',
  malformed_json: 'The selected file is not valid JSON.',
  unsupported_version: 'This diagnostic report version is not supported by this Murmur build.',
  schema_mismatch: 'The selected JSON is not a supported Murmur benchmark or evaluation report.',
  collection_limit: 'The selected report exceeds supported diagnostic collection limits.',
};

function failure(code: DiagnosticImportErrorCode): DiagnosticImportResult {
  return { ok: false, error: { code, message: ERROR_MESSAGES[code] } };
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: JsonRecord,
  required: readonly string[],
  optional: readonly string[] = [],
): boolean {
  const allowed = new Set([...required, ...optional]);
  return required.every((key) => Object.prototype.hasOwnProperty.call(value, key))
    && Object.keys(value).every((key) => allowed.has(key));
}

function isString(value: unknown): value is string {
  return typeof value === 'string';
}

function isNullableString(value: unknown): value is string | null {
  return value === null || isString(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNonNegativeNumber(value: unknown): value is number {
  return isFiniteNumber(value) && value >= 0;
}

function isInteger(value: unknown): value is number {
  return Number.isSafeInteger(value);
}

function isNonNegativeInteger(value: unknown): value is number {
  return isInteger(value) && (value as number) >= 0;
}

function isNullableNonNegativeInteger(value: unknown): value is number | null {
  return value === null || isNonNegativeInteger(value);
}

function isNullableNonNegativeNumber(value: unknown): value is number | null {
  return value === null || isNonNegativeNumber(value);
}

function isNullableInteger(value: unknown): value is number | null {
  return value === null || isInteger(value);
}

function isIsoDate(value: unknown): value is string {
  return isString(value) && value.length > 0 && Number.isFinite(Date.parse(value));
}

function boundedArray<T>(
  value: unknown,
  maximum: number,
  predicate: (entry: unknown, context: ValidationContext) => entry is T,
  context: ValidationContext,
): value is T[] {
  if (!Array.isArray(value)) return false;
  if (value.length > maximum) {
    context.collectionExceeded = true;
    return false;
  }
  return value.every((entry) => predicate(entry, context));
}

function boundedStrings(
  value: unknown,
  maximum: number,
  context: ValidationContext,
): value is string[] {
  return boundedArray(value, maximum, (entry): entry is string => isString(entry), context);
}

function hasUniqueStrings(values: string[]): boolean {
  return new Set(values).size === values.length;
}

const BENCHMARK_FIXTURE_KEYS = [
  'fixtureId', 'label', 'audioSeconds', 'warmMedianMs', 'warmP95Ms', 'realtimeFactor',
  'wordErrorRate', 'wordErrors', 'referenceWords', 'normalizedWordErrorRate',
  'normalizedWordErrors', 'normalizedReferenceWords', 'reference', 'transcript',
  'deliveredTranscript', 'deliveredWordErrorRate', 'deliveredWordErrors',
  'deliveredNormalizedWordErrorRate', 'deliveredNormalizedWordErrors',
  'deliveredTransformFailed',
] as const;

function isBenchmarkFixture(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, BENCHMARK_FIXTURE_KEYS)) return false;
  return isString(value.fixtureId)
    && isString(value.label)
    && isNonNegativeNumber(value.audioSeconds)
    && isNonNegativeNumber(value.warmMedianMs)
    && isNonNegativeNumber(value.warmP95Ms)
    && isNonNegativeNumber(value.realtimeFactor)
    && isNonNegativeNumber(value.wordErrorRate)
    && isNonNegativeInteger(value.wordErrors)
    && isNonNegativeInteger(value.referenceWords)
    && isNonNegativeNumber(value.normalizedWordErrorRate)
    && isNonNegativeInteger(value.normalizedWordErrors)
    && isNonNegativeInteger(value.normalizedReferenceWords)
    && isString(value.reference)
    && isString(value.transcript)
    && isString(value.deliveredTranscript)
    && isNonNegativeNumber(value.deliveredWordErrorRate)
    && isNonNegativeInteger(value.deliveredWordErrors)
    && isNonNegativeNumber(value.deliveredNormalizedWordErrorRate)
    && isNonNegativeInteger(value.deliveredNormalizedWordErrors)
    && typeof value.deliveredTransformFailed === 'boolean';
}

const BENCHMARK_MODEL_KEYS = [
  'modelName', 'label', 'backend', 'accelerator', 'modelLoadMs', 'firstInferenceMs',
  'warmMedianMs', 'warmP95Ms', 'realtimeFactor', 'wordErrorRate',
  'normalizedWordErrorRate', 'deliveredWordErrorRate',
  'deliveredNormalizedWordErrorRate', 'memoryDeltaMb', 'fixtures', 'error',
] as const;

function isBenchmarkModel(value: unknown, context: ValidationContext): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, BENCHMARK_MODEL_KEYS, ['downloadSize'])) {
    return false;
  }
  if (!boundedArray(
    value.fixtures,
    DIAGNOSTIC_REPORT_LIMITS.benchmarkFixturesPerModel,
    (entry): entry is JsonRecord => isBenchmarkFixture(entry),
    context,
  )) {
    return false;
  }
  const fixtureIds = value.fixtures.map((fixture) => fixture.fixtureId as string);
  const structurallyValid = isString(value.modelName)
    && isString(value.label)
    && isString(value.backend)
    && isString(value.accelerator)
    && (value.downloadSize === undefined || isString(value.downloadSize))
    && isNullableNonNegativeNumber(value.modelLoadMs)
    && isNullableNonNegativeNumber(value.firstInferenceMs)
    && isNullableNonNegativeNumber(value.warmMedianMs)
    && isNullableNonNegativeNumber(value.warmP95Ms)
    && isNullableNonNegativeNumber(value.realtimeFactor)
    && isNullableNonNegativeNumber(value.wordErrorRate)
    && isNullableNonNegativeNumber(value.normalizedWordErrorRate)
    && isNullableNonNegativeNumber(value.deliveredWordErrorRate)
    && isNullableNonNegativeNumber(value.deliveredNormalizedWordErrorRate)
    && isNonNegativeNumber(value.memoryDeltaMb)
    && isNullableString(value.error)
    && hasUniqueStrings(fixtureIds);
  if (!structurallyValid) return false;
  return value.error !== null || hasCompleteBenchmarkMeasurements(value);
}

function hasCompleteBenchmarkMeasurements(value: JsonRecord): boolean {
  return isNonNegativeNumber(value.modelLoadMs)
    && isNonNegativeNumber(value.firstInferenceMs)
    && isNonNegativeNumber(value.warmMedianMs)
    && isNonNegativeNumber(value.warmP95Ms)
    && isNonNegativeNumber(value.realtimeFactor)
    && isNonNegativeNumber(value.wordErrorRate)
    && isNonNegativeNumber(value.normalizedWordErrorRate)
    && isNonNegativeNumber(value.deliveredWordErrorRate)
    && isNonNegativeNumber(value.deliveredNormalizedWordErrorRate)
    && Array.isArray(value.fixtures)
    && value.fixtures.length > 0;
}

function isBenchmarkEnvironment(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'os', 'osVersion', 'architecture', 'hardwareModel', 'chip', 'memoryMb',
  ])) return false;
  return isString(value.os)
    && isNullableString(value.osVersion)
    && isString(value.architecture)
    && isNullableString(value.hardwareModel)
    && isNullableString(value.chip)
    && isNullableNonNegativeInteger(value.memoryMb);
}

function isBenchmarkCorpus(value: unknown, context: ValidationContext): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'language', 'fixtureIds', 'fixtureCount', 'referenceWords', 'provenance', 'limitation',
  ])) return false;
  if (!boundedStrings(
    value.fixtureIds,
    DIAGNOSTIC_REPORT_LIMITS.stringsPerCollection,
    context,
  )) return false;
  return isString(value.language)
    && hasUniqueStrings(value.fixtureIds)
    && isNonNegativeInteger(value.fixtureCount)
    && value.fixtureCount === value.fixtureIds.length
    && isNonNegativeInteger(value.referenceWords)
    && isString(value.provenance)
    && isString(value.limitation);
}

function isBenchmarkConfiguration(value: unknown, context: ValidationContext): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'vadThreshold', 'executionPath', 'transcriptTransformProfile', 'percentileMethod',
    'modelRunOrder', 'sharedInitOrder',
  ])) return false;
  return isNonNegativeNumber(value.vadThreshold)
    && isString(value.executionPath)
    && isString(value.transcriptTransformProfile)
    && isString(value.percentileMethod)
    && boundedStrings(value.modelRunOrder, DIAGNOSTIC_REPORT_LIMITS.stringsPerCollection, context)
    && boundedStrings(value.sharedInitOrder, DIAGNOSTIC_REPORT_LIMITS.stringsPerCollection, context);
}

function isRecommendations(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, ['fastest', 'mostAccurate', 'balanced'])) {
    return false;
  }
  return isNullableString(value.fastest)
    && isNullableString(value.mostAccurate)
    && isNullableString(value.balanced);
}

const BENCHMARK_BASE_KEYS = [
  'createdAt', 'appVersion', 'platform', 'preset', 'iterations', 'sharedInitMs',
  'results', 'recommendations',
] as const;

function isBenchmarkReport(
  value: JsonRecord,
  context: ValidationContext,
): value is JsonRecord {
  const version = value.reportVersion;
  const isLegacy = version === undefined;
  const required = isLegacy
    ? BENCHMARK_BASE_KEYS
    : [...BENCHMARK_BASE_KEYS, 'reportVersion', 'environment', 'corpus', 'configuration'];
  if (!hasExactKeys(value, required)) return false;
  if (!boundedArray(
    value.results,
    DIAGNOSTIC_REPORT_LIMITS.benchmarkModels,
    isBenchmarkModel,
    context,
  )) return false;
  const identities = value.results.map((model) =>
    `${String(model.modelName)}\u0000${String(model.backend)}\u0000${String(model.accelerator)}`);
  return (isLegacy || version === 2)
    && isIsoDate(value.createdAt)
    && isString(value.appVersion)
    && isString(value.platform)
    && (value.preset === 'quick' || value.preset === 'standard' || value.preset === 'thorough')
    && isNonNegativeInteger(value.iterations)
    && value.iterations > 0
    && isNonNegativeNumber(value.sharedInitMs)
    && isRecommendations(value.recommendations)
    && hasUniqueStrings(identities)
    && (isLegacy || (
      isBenchmarkEnvironment(value.environment)
      && isBenchmarkCorpus(value.corpus, context)
      && isBenchmarkConfiguration(value.configuration, context)
    ));
}

function isEvaluationPrivacy(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'localOnly', 'historyIngestion', 'networkUsed', 'systemClipboardUsed',
    'fixtureProvenanceRequired',
  ])) return false;
  return value.localOnly === true
    && value.historyIngestion === false
    && value.networkUsed === false
    && value.systemClipboardUsed === false
    && value.fixtureProvenanceRequired === true;
}

function isEvaluationEnvironment(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'appVersion', 'os', 'arch', 'machineLabel', 'logicalCpus',
  ])) return false;
  return isString(value.appVersion)
    && isString(value.os)
    && isString(value.arch)
    && isString(value.machineLabel)
    && isNullableNonNegativeInteger(value.logicalCpus);
}

const EVALUATION_SUMMARY_KEYS = [
  'total', 'passed', 'failed', 'skipped', 'aggregateRawWer', 'aggregateNormalizedWer',
  'aggregateCer', 'transformationMatchRate', 'commandExactMatchRate',
  'noChangePreservationRate', 'deliveryMatchRate',
] as const;

function isEvaluationSummary(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, EVALUATION_SUMMARY_KEYS)) return false;
  return isNonNegativeInteger(value.total)
    && isNonNegativeInteger(value.passed)
    && isNonNegativeInteger(value.failed)
    && isNonNegativeInteger(value.skipped)
    && value.total === value.passed + value.failed + value.skipped
    && isNullableNonNegativeNumber(value.aggregateRawWer)
    && isNullableNonNegativeNumber(value.aggregateNormalizedWer)
    && isNullableNonNegativeNumber(value.aggregateCer)
    && isNullableNonNegativeNumber(value.transformationMatchRate)
    && isNullableNonNegativeNumber(value.commandExactMatchRate)
    && isNullableNonNegativeNumber(value.noChangePreservationRate)
    && isNullableNonNegativeNumber(value.deliveryMatchRate);
}

function isCaseProvenance(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'kind', 'source', 'containsRealUserData', 'deletion',
  ])) return false;
  return isString(value.kind)
    && isString(value.source)
    && value.containsRealUserData === false
    && isString(value.deletion);
}

function isCaseContext(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'bundleId', 'matchedProfile', 'fixtureOnly',
  ])) return false;
  return isNullableString(value.bundleId)
    && isNullableString(value.matchedProfile)
    && typeof value.fixtureOnly === 'boolean';
}

function isEvaluationModel(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'name', 'backend', 'accelerator', 'audioPath', 'sampleRateHz', 'channels',
  ])) return false;
  return isString(value.name)
    && isString(value.backend)
    && isString(value.accelerator)
    && isString(value.audioPath)
    && isNonNegativeInteger(value.sampleRateHz)
    && value.sampleRateHz > 0
    && isNonNegativeInteger(value.channels)
    && value.channels > 0;
}

function isRecognition(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'expectedRaw', 'actualRaw', 'rawWordErrors', 'normalizedWordErrors',
    'referenceWords', 'normalizedReferenceWords', 'referenceCharacters',
    'characterErrors', 'rawWer', 'normalizedWer', 'cer', 'boundedAlternativeMatch',
  ])) return false;
  return isNullableString(value.expectedRaw)
    && isNullableString(value.actualRaw)
    && isNullableNonNegativeInteger(value.rawWordErrors)
    && isNullableNonNegativeInteger(value.normalizedWordErrors)
    && isNullableNonNegativeInteger(value.referenceWords)
    && isNullableNonNegativeInteger(value.normalizedReferenceWords)
    && isNullableNonNegativeInteger(value.referenceCharacters)
    && isNullableNonNegativeInteger(value.characterErrors)
    && isNullableNonNegativeNumber(value.rawWer)
    && isNullableNonNegativeNumber(value.normalizedWer)
    && isNullableNonNegativeNumber(value.cer)
    && typeof value.boundedAlternativeMatch === 'boolean';
}

function isStage(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'name', 'outcome', 'changed', 'durationUs', 'text', 'expectationMatch',
  ])) return false;
  return isString(value.name)
    && isString(value.outcome)
    && typeof value.changed === 'boolean'
    && isNonNegativeInteger(value.durationUs)
    && isString(value.text)
    && typeof value.expectationMatch === 'boolean';
}

function isTransformation(value: unknown, context: ValidationContext): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'expectedFinal', 'actualFinal', 'exactMatch', 'commandExactMatch',
    'noChangePreserved', 'stages',
  ])) return false;
  return isString(value.expectedFinal)
    && isNullableString(value.actualFinal)
    && typeof value.exactMatch === 'boolean'
    && (value.commandExactMatch === null || typeof value.commandExactMatch === 'boolean')
    && (value.noChangePreserved === null || typeof value.noChangePreserved === 'boolean')
    && boundedArray(
      value.stages,
      DIAGNOSTIC_REPORT_LIMITS.evaluationStagesPerCase,
      (entry): entry is JsonRecord => isStage(entry),
      context,
    );
}

function isDelivery(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'expected', 'delivered', 'exactMatch', 'attempts', 'partialCount', 'firstPartialMs',
    'firstPartialApplicability', 'finalOnly',
  ])) return false;
  return isString(value.expected)
    && isNullableString(value.delivered)
    && typeof value.exactMatch === 'boolean'
    && isNonNegativeInteger(value.attempts)
    && isNonNegativeInteger(value.partialCount)
    && isNullableNonNegativeInteger(value.firstPartialMs)
    && isString(value.firstPartialApplicability)
    && typeof value.finalOnly === 'boolean';
}

function isLatency(value: unknown): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'rawAsrMs', 'transformationMs', 'finalizationMs', 'deliveryMs', 'totalMs',
  ])) return false;
  return isNonNegativeInteger(value.rawAsrMs)
    && isNonNegativeInteger(value.transformationMs)
    && isNonNegativeInteger(value.finalizationMs)
    && isNonNegativeInteger(value.deliveryMs)
    && isNonNegativeInteger(value.totalMs);
}

function isRuntime(value: unknown, context: ValidationContext): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'incrementalCompletion', 'fallbackUsed', 'fallbackStages', 'memoryBeforeMb',
    'memoryAfterMb', 'memoryDeltaMb',
  ])) return false;
  return isString(value.incrementalCompletion)
    && typeof value.fallbackUsed === 'boolean'
    && boundedStrings(value.fallbackStages, DIAGNOSTIC_REPORT_LIMITS.stringsPerCollection, context)
    && isNullableNonNegativeInteger(value.memoryBeforeMb)
    && isNullableNonNegativeInteger(value.memoryAfterMb)
    && isNullableInteger(value.memoryDeltaMb);
}

function isEvaluationCase(
  value: unknown,
  context: ValidationContext,
): value is JsonRecord {
  if (!isRecord(value) || !hasExactKeys(value, [
    'id', 'status', 'failures', 'provenance', 'context', 'model', 'recognition',
    'transformation', 'delivery', 'latency', 'runtime',
  ])) return false;
  const structurallyValid = isString(value.id)
    && (value.status === 'passed' || value.status === 'failed' || value.status === 'skipped')
    && boundedStrings(value.failures, DIAGNOSTIC_REPORT_LIMITS.stringsPerCollection, context)
    && isCaseProvenance(value.provenance)
    && isCaseContext(value.context)
    && (value.model === null || isEvaluationModel(value.model))
    && isRecognition(value.recognition)
    && isTransformation(value.transformation, context)
    && isDelivery(value.delivery)
    && isLatency(value.latency)
    && isRuntime(value.runtime, context);
  if (!structurallyValid) return false;
  return hasConsistentEvaluationOutcome(value);
}

function hasCompleteRecognition(value: JsonRecord): boolean {
  return isString(value.expectedRaw)
    && isString(value.actualRaw)
    && isNonNegativeInteger(value.rawWordErrors)
    && isNonNegativeInteger(value.normalizedWordErrors)
    && isNonNegativeInteger(value.referenceWords)
    && value.referenceWords > 0
    && isNonNegativeInteger(value.normalizedReferenceWords)
    && value.normalizedReferenceWords > 0
    && isNonNegativeInteger(value.referenceCharacters)
    && value.referenceCharacters > 0
    && isNonNegativeInteger(value.characterErrors)
    && isNonNegativeNumber(value.rawWer)
    && isNonNegativeNumber(value.normalizedWer)
    && isNonNegativeNumber(value.cer)
    && value.boundedAlternativeMatch === true;
}

function hasConsistentEvaluationOutcome(value: JsonRecord): boolean {
  const failures = value.failures as string[];
  if (value.status !== 'passed') return failures.length > 0;
  const transformation = value.transformation as JsonRecord;
  const delivery = value.delivery as JsonRecord;
  return failures.length === 0
    && hasCompleteRecognition(value.recognition as JsonRecord)
    && isString(transformation.actualFinal)
    && transformation.exactMatch === true
    && transformation.commandExactMatch !== false
    && transformation.noChangePreserved !== false
    && (transformation.stages as JsonRecord[])
      .every((stage) => stage.expectationMatch === true)
    && isString(delivery.delivered)
    && delivery.exactMatch === true
    && isNonNegativeInteger(delivery.attempts)
    && delivery.attempts > 0;
}

function isEvaluationReport(
  value: JsonRecord,
  context: ValidationContext,
): value is JsonRecord {
  if (!hasExactKeys(value, [
    'reportVersion', 'fixtureVersion', 'generatedAt', 'tier', 'privacy',
    'environment', 'summary', 'cases',
  ])) return false;
  if (!boundedArray(
    value.cases,
    DIAGNOSTIC_REPORT_LIMITS.evaluationCases,
    isEvaluationCase,
    context,
  )) return false;
  if (!isEvaluationSummary(value.summary)) return false;
  const caseIds = value.cases.map((entry) => entry.id as string);
  const tierModelsAreConsistent = value.cases.every((entry) =>
    value.tier === 'deterministic' ? entry.model === null : entry.model !== null);
  const summary = value.summary as JsonRecord;
  const passedSummaryIsComplete = summary.passed === 0 || (
    isNonNegativeNumber(summary.aggregateRawWer)
    && isNonNegativeNumber(summary.aggregateNormalizedWer)
    && isNonNegativeNumber(summary.aggregateCer)
    && isNonNegativeNumber(summary.transformationMatchRate)
    && isNonNegativeNumber(summary.deliveryMatchRate)
  );
  return value.reportVersion === 1
    && value.fixtureVersion === 1
    && isIsoDate(value.generatedAt)
    && (value.tier === 'deterministic' || value.tier === 'hardware')
    && isEvaluationPrivacy(value.privacy)
    && isEvaluationEnvironment(value.environment)
    && value.summary.total === value.cases.length
    && tierModelsAreConsistent
    && passedSummaryIsComplete
    && hasUniqueStrings(caseIds);
}

function normalizeBenchmark(
  value: JsonRecord,
  source: DiagnosticReportSource,
): NormalizedBenchmarkReport {
  const versioned = value.reportVersion === 2;
  const environment = versioned ? value.environment as JsonRecord : null;
  const corpus = versioned ? value.corpus as JsonRecord : null;
  const configuration = versioned ? value.configuration as JsonRecord : null;
  const recommendations = value.recommendations as JsonRecord;
  return {
    kind: 'benchmark',
    source,
    schemaVersion: versioned ? 2 : 'legacy',
    createdAt: value.createdAt as string,
    appVersion: value.appVersion as string,
    platform: value.platform as string,
    preset: value.preset as NormalizedBenchmarkReport['preset'],
    iterations: value.iterations as number,
    sharedInitMs: value.sharedInitMs as number,
    environment: environment ? {
      os: environment.os as string,
      osVersion: environment.osVersion as string | null,
      architecture: environment.architecture as string,
      hardwareModel: environment.hardwareModel as string | null,
      chip: environment.chip as string | null,
      memoryMb: environment.memoryMb as number | null,
    } : null,
    corpus: corpus ? {
      language: corpus.language as string,
      fixtureIds: [...corpus.fixtureIds as string[]],
      fixtureCount: corpus.fixtureCount as number,
      referenceWords: corpus.referenceWords as number,
    } : null,
    configuration: configuration ? {
      vadThreshold: configuration.vadThreshold as number,
      executionPath: configuration.executionPath as string,
      transcriptTransformProfile: configuration.transcriptTransformProfile as string,
      percentileMethod: configuration.percentileMethod as string,
      modelRunOrder: [...configuration.modelRunOrder as string[]],
      sharedInitOrder: [...configuration.sharedInitOrder as string[]],
    } : null,
    models: (value.results as JsonRecord[]).map((model) => ({
      modelName: model.modelName as string,
      label: model.label as string,
      backend: model.backend as string,
      accelerator: model.accelerator as string,
      downloadSize: model.downloadSize as string | undefined ?? null,
      modelLoadMs: model.modelLoadMs as number | null,
      firstInferenceMs: model.firstInferenceMs as number | null,
      warmMedianMs: model.warmMedianMs as number | null,
      warmP95Ms: model.warmP95Ms as number | null,
      realtimeFactor: model.realtimeFactor as number | null,
      wordErrorRate: model.wordErrorRate as number | null,
      normalizedWordErrorRate: model.normalizedWordErrorRate as number | null,
      deliveredWordErrorRate: model.deliveredWordErrorRate as number | null,
      deliveredNormalizedWordErrorRate:
        model.deliveredNormalizedWordErrorRate as number | null,
      memoryDeltaMb: model.memoryDeltaMb as number,
      fixtures: (model.fixtures as JsonRecord[]).map((fixture) => ({
        fixtureId: fixture.fixtureId as string,
        audioSeconds: fixture.audioSeconds as number,
        warmMedianMs: fixture.warmMedianMs as number,
        warmP95Ms: fixture.warmP95Ms as number,
        realtimeFactor: fixture.realtimeFactor as number,
        wordErrorRate: fixture.wordErrorRate as number,
        normalizedWordErrorRate: fixture.normalizedWordErrorRate as number,
        deliveredWordErrorRate: fixture.deliveredWordErrorRate as number,
        deliveredNormalizedWordErrorRate:
          fixture.deliveredNormalizedWordErrorRate as number,
        deliveredTransformFailed: fixture.deliveredTransformFailed as boolean,
      })),
      succeeded: model.error === null && hasCompleteBenchmarkMeasurements(model),
    })),
    recommendations: {
      fastest: recommendations.fastest as string | null,
      mostAccurate: recommendations.mostAccurate as string | null,
      balanced: recommendations.balanced as string | null,
    },
    privacyWarnings: versioned
      ? []
      : ['Legacy benchmark reports omit environment, corpus, and execution configuration metadata.'],
  };
}

function normalizeEvaluation(
  value: JsonRecord,
  source: DiagnosticReportSource,
): NormalizedEvaluationReport {
  const environment = value.environment as JsonRecord;
  const summary = value.summary as JsonRecord;
  return {
    kind: 'evaluation',
    source,
    schemaVersion: 1,
    fixtureVersion: 1,
    createdAt: value.generatedAt as string,
    tier: value.tier as NormalizedEvaluationReport['tier'],
    environment: {
      appVersion: environment.appVersion as string,
      os: environment.os as string,
      architecture: environment.arch as string,
      machineLabel: environment.machineLabel as string,
      logicalCpus: environment.logicalCpus as number | null,
    },
    summary: {
      total: summary.total as number,
      passed: summary.passed as number,
      failed: summary.failed as number,
      skipped: summary.skipped as number,
      aggregateRawWer: summary.aggregateRawWer as number | null,
      aggregateNormalizedWer: summary.aggregateNormalizedWer as number | null,
      aggregateCer: summary.aggregateCer as number | null,
      transformationMatchRate: summary.transformationMatchRate as number | null,
      commandExactMatchRate: summary.commandExactMatchRate as number | null,
      noChangePreservationRate: summary.noChangePreservationRate as number | null,
      deliveryMatchRate: summary.deliveryMatchRate as number | null,
    },
    cases: (value.cases as JsonRecord[]).map((entry) => {
      const context = entry.context as JsonRecord;
      const model = entry.model as JsonRecord | null;
      const recognition = entry.recognition as JsonRecord;
      const transformation = entry.transformation as JsonRecord;
      const delivery = entry.delivery as JsonRecord;
      const latency = entry.latency as JsonRecord;
      const runtime = entry.runtime as JsonRecord;
      return {
        id: entry.id as string,
        status: entry.status as NormalizedEvaluationCase['status'],
        complete: entry.status === 'passed' && hasConsistentEvaluationOutcome(entry),
        fixtureOnly: context.fixtureOnly as boolean,
        model: model ? {
          name: model.name as string,
          backend: model.backend as string,
          accelerator: model.accelerator as string,
        } : null,
        recognition: {
          rawWordErrors: recognition.rawWordErrors as number | null,
          normalizedWordErrors: recognition.normalizedWordErrors as number | null,
          referenceWords: recognition.referenceWords as number | null,
          normalizedReferenceWords: recognition.normalizedReferenceWords as number | null,
          referenceCharacters: recognition.referenceCharacters as number | null,
          characterErrors: recognition.characterErrors as number | null,
          rawWer: recognition.rawWer as number | null,
          normalizedWer: recognition.normalizedWer as number | null,
          cer: recognition.cer as number | null,
          boundedAlternativeMatch: recognition.boundedAlternativeMatch as boolean,
        },
        transformation: {
          exactMatch: transformation.exactMatch as boolean,
          commandExactMatch: transformation.commandExactMatch as boolean | null,
          noChangePreserved: transformation.noChangePreserved as boolean | null,
          stages: (transformation.stages as JsonRecord[]).map((stage) => ({
            name: stage.name as string,
            outcome: stage.outcome as string,
            changed: stage.changed as boolean,
            durationUs: stage.durationUs as number,
            expectationMatch: stage.expectationMatch as boolean,
          })),
        },
        delivery: {
          exactMatch: delivery.exactMatch as boolean,
          attempts: delivery.attempts as number,
          partialCount: delivery.partialCount as number,
          firstPartialMs: delivery.firstPartialMs as number | null,
          firstPartialApplicability: delivery.firstPartialApplicability as string,
          finalOnly: delivery.finalOnly as boolean,
        },
        latency: {
          rawAsrMs: latency.rawAsrMs as number,
          transformationMs: latency.transformationMs as number,
          finalizationMs: latency.finalizationMs as number,
          deliveryMs: latency.deliveryMs as number,
          totalMs: latency.totalMs as number,
        },
        runtime: {
          incrementalCompletion: runtime.incrementalCompletion as string,
          fallbackUsed: runtime.fallbackUsed as boolean,
          fallbackStages: [...runtime.fallbackStages as string[]],
          memoryBeforeMb: runtime.memoryBeforeMb as number | null,
          memoryAfterMb: runtime.memoryAfterMb as number | null,
          memoryDeltaMb: runtime.memoryDeltaMb as number | null,
        },
      };
    }),
    privacyWarnings: [
      'Evaluation reports may contain curated fixture transcripts and per-stage text; imported data stays in this session only.',
    ],
  };
}

function reportFamily(value: JsonRecord): DiagnosticReportKind | null {
  if (Object.prototype.hasOwnProperty.call(value, 'createdAt')
    || Object.prototype.hasOwnProperty.call(value, 'results')
    || Object.prototype.hasOwnProperty.call(value, 'recommendations')) {
    return 'benchmark';
  }
  if (Object.prototype.hasOwnProperty.call(value, 'generatedAt')
    || Object.prototype.hasOwnProperty.call(value, 'fixtureVersion')
    || Object.prototype.hasOwnProperty.call(value, 'cases')) {
    return 'evaluation';
  }
  return null;
}

function hasUnsupportedVersion(value: JsonRecord, family: DiagnosticReportKind): boolean {
  if (family === 'benchmark') {
    return value.reportVersion !== undefined && value.reportVersion !== 2;
  }
  return (value.reportVersion !== undefined && value.reportVersion !== 1)
    || (value.fixtureVersion !== undefined && value.fixtureVersion !== 1);
}

export function parseDiagnosticReportJson(
  contents: string,
  sourceBytes: number,
): DiagnosticImportResult {
  if (!Number.isSafeInteger(sourceBytes) || sourceBytes < 0) {
    return failure('schema_mismatch');
  }
  if (sourceBytes > MAX_DIAGNOSTIC_REPORT_BYTES) return failure('oversized');
  if (contents.trim().length === 0) return failure('empty');
  if (new TextEncoder().encode(contents).byteLength > MAX_DIAGNOSTIC_REPORT_BYTES) {
    return failure('oversized');
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(contents);
  } catch {
    return failure('malformed_json');
  }
  if (!isRecord(parsed)) return failure('schema_mismatch');

  const family = reportFamily(parsed);
  if (!family) return failure('schema_mismatch');
  if (hasUnsupportedVersion(parsed, family)) return failure('unsupported_version');

  const context: ValidationContext = { collectionExceeded: false };
  const valid = family === 'benchmark'
    ? isBenchmarkReport(parsed, context)
    : isEvaluationReport(parsed, context);
  if (!valid) {
    return failure(context.collectionExceeded ? 'collection_limit' : 'schema_mismatch');
  }
  return {
    ok: true,
    report: family === 'benchmark'
      ? normalizeBenchmark(parsed, 'imported')
      : normalizeEvaluation(parsed, 'imported'),
  };
}

export function normalizeLocalBenchmarkReport(
  report: BenchmarkReport,
): DiagnosticImportResult {
  const parsed = report as unknown;
  if (!isRecord(parsed)) return failure('schema_mismatch');
  const context: ValidationContext = { collectionExceeded: false };
  if (!isBenchmarkReport(parsed, context)) {
    return failure(context.collectionExceeded ? 'collection_limit' : 'schema_mismatch');
  }
  return { ok: true, report: normalizeBenchmark(parsed, 'local') };
}
