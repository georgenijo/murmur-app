import { invoke } from '@tauri-apps/api/core';
import { getModelRuntimeCatalog } from './modelRuntime';

export type BenchmarkPreset = 'quick' | 'standard' | 'thorough';

export interface BenchmarkModel {
  modelName: string;
  label: string;
  backend: string;
  accelerator: string;
  size: string;
  supported: boolean;
  installed: boolean;
}

export interface BenchmarkProgress {
  completed: number;
  total: number;
  modelName: string;
  modelLabel: string;
  fixture: string | null;
  phase: 'priming' | 'loading' | 'warming' | 'measuring' | 'complete';
}

export interface BenchmarkActivity {
  benchmarkRunning: boolean;
  fileTranscribing: boolean;
}

export interface BenchmarkFixtureResult {
  fixtureId: string;
  label: string;
  audioSeconds: number;
  warmMedianMs: number;
  warmP95Ms: number;
  realtimeFactor: number;
  wordErrorRate: number;
  wordErrors: number;
  referenceWords: number;
  normalizedWordErrorRate: number;
  normalizedWordErrors: number;
  normalizedReferenceWords: number;
  reference: string;
  transcript: string;
  /** Text after the production transcript-transform pipeline ran on
   * `transcript` — what actually reaches the clipboard. The delivered* WER
   * fields score this against `reference`; raw/normalized above score the
   * decoder output. See issue #271. */
  deliveredTranscript: string;
  deliveredWordErrorRate: number;
  deliveredWordErrors: number;
  deliveredNormalizedWordErrorRate: number;
  deliveredNormalizedWordErrors: number;
  /** True when the transform errored and delivered* fell back to scoring the
   * untransformed `transcript`. */
  deliveredTransformFailed: boolean;
}

export interface BenchmarkModelResult {
  modelName: string;
  label: string;
  backend: string;
  accelerator: string;
  /** Catalog download size, separate from observed process memory. */
  downloadSize?: string;
  modelLoadMs: number | null;
  firstInferenceMs: number | null;
  warmMedianMs: number | null;
  warmP95Ms: number | null;
  realtimeFactor: number | null;
  wordErrorRate: number | null;
  normalizedWordErrorRate: number | null;
  /** Corpus WER of the delivered text (post transcript-transform pipeline),
   * raw and normalized — the metric reflecting clipboard output rather than
   * raw decoder output. See issue #271. */
  deliveredWordErrorRate: number | null;
  deliveredNormalizedWordErrorRate: number | null;
  /** Process-RSS delta for this model's run. Models are benchmarked
   * sequentially in one process, so allocator retention from an earlier
   * model can inflate a later model's baseline — treat as a rough signal,
   * not an isolated per-model measurement. */
  memoryDeltaMb: number;
  fixtures: BenchmarkFixtureResult[];
  error: string | null;
}

export interface BenchmarkReport {
  /** Version 2 adds explicit environment, corpus, and execution metadata. */
  reportVersion?: number;
  createdAt: string;
  appVersion: string;
  platform: string;
  preset: BenchmarkPreset;
  iterations: number;
  /** Duration (ms) of the untimed warm-up pass run once before any
   * per-model timing, absorbing one-time shared backend init (Metal shader
   * compilation, ANE compile cache, etc). Represents real first-launch
   * latency but is not a per-model attribute. */
  sharedInitMs: number;
  environment?: {
    os: string;
    osVersion: string | null;
    architecture: string;
    hardwareModel: string | null;
    chip: string | null;
    memoryMb: number | null;
  };
  corpus?: {
    language: string;
    fixtureIds: string[];
    fixtureCount: number;
    referenceWords: number;
    provenance: string;
    limitation: string;
  };
  configuration?: {
    vadThreshold: number;
    executionPath: string;
    transcriptTransformProfile: string;
    percentileMethod: string;
    modelRunOrder: string[];
    sharedInitOrder: string[];
  };
  results: BenchmarkModelResult[];
  recommendations: {
    fastest: string | null;
    mostAccurate: string | null;
    balanced: string | null;
  };
}

export const MAX_SAVED_BENCHMARK_REPORTS = 10;
const REPORT_KEY = 'murmur-benchmark-report';
const REPORTS_KEY = 'murmur-benchmark-reports';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || isNumber(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isFixtureResult(value: unknown): value is BenchmarkFixtureResult {
  if (!isRecord(value)) return false;
  return typeof value.fixtureId === 'string'
    && typeof value.label === 'string'
    && isNumber(value.audioSeconds)
    && isNumber(value.warmMedianMs)
    && isNumber(value.warmP95Ms)
    && isNumber(value.realtimeFactor)
    && isNumber(value.wordErrorRate)
    && isNumber(value.wordErrors)
    && isNumber(value.referenceWords)
    && isNumber(value.normalizedWordErrorRate)
    && isNumber(value.normalizedWordErrors)
    && isNumber(value.normalizedReferenceWords)
    && typeof value.reference === 'string'
    && typeof value.transcript === 'string'
    && typeof value.deliveredTranscript === 'string'
    && isNumber(value.deliveredWordErrorRate)
    && isNumber(value.deliveredWordErrors)
    && isNumber(value.deliveredNormalizedWordErrorRate)
    && isNumber(value.deliveredNormalizedWordErrors)
    && typeof value.deliveredTransformFailed === 'boolean';
}

function isModelResult(value: unknown): value is BenchmarkModelResult {
  if (!isRecord(value)) return false;
  return typeof value.modelName === 'string'
    && typeof value.label === 'string'
    && typeof value.backend === 'string'
    && typeof value.accelerator === 'string'
    && (value.downloadSize === undefined || typeof value.downloadSize === 'string')
    && isNullableNumber(value.modelLoadMs)
    && isNullableNumber(value.firstInferenceMs)
    && isNullableNumber(value.warmMedianMs)
    && isNullableNumber(value.warmP95Ms)
    && isNullableNumber(value.realtimeFactor)
    && isNullableNumber(value.wordErrorRate)
    && isNullableNumber(value.normalizedWordErrorRate)
    && isNullableNumber(value.deliveredWordErrorRate)
    && isNullableNumber(value.deliveredNormalizedWordErrorRate)
    && isNumber(value.memoryDeltaMb)
    && Array.isArray(value.fixtures)
    && value.fixtures.every(isFixtureResult)
    && isNullableString(value.error);
}

function isEnvironment(value: unknown): boolean {
  return isRecord(value)
    && typeof value.os === 'string'
    && isNullableString(value.osVersion)
    && typeof value.architecture === 'string'
    && isNullableString(value.hardwareModel)
    && isNullableString(value.chip)
    && isNullableNumber(value.memoryMb);
}

function isCorpus(value: unknown): boolean {
  return isRecord(value)
    && typeof value.language === 'string'
    && Array.isArray(value.fixtureIds)
    && value.fixtureIds.every((fixture) => typeof fixture === 'string')
    && isNumber(value.fixtureCount)
    && isNumber(value.referenceWords)
    && typeof value.provenance === 'string'
    && typeof value.limitation === 'string';
}

function isConfiguration(value: unknown): boolean {
  return isRecord(value)
    && isNumber(value.vadThreshold)
    && typeof value.executionPath === 'string'
    && typeof value.transcriptTransformProfile === 'string'
    && typeof value.percentileMethod === 'string'
    && Array.isArray(value.modelRunOrder)
    && value.modelRunOrder.every((model) => typeof model === 'string')
    && Array.isArray(value.sharedInitOrder)
    && value.sharedInitOrder.every((model) => typeof model === 'string');
}

function isBenchmarkReport(value: unknown): value is BenchmarkReport {
  if (!isRecord(value) || !isRecord(value.recommendations)) return false;
  const metadataIsCompatible = value.reportVersion === undefined
    ? value.environment === undefined && value.corpus === undefined && value.configuration === undefined
    : value.reportVersion === 2
      && isEnvironment(value.environment)
      && isCorpus(value.corpus)
      && isConfiguration(value.configuration);
  return typeof value.createdAt === 'string'
    && Number.isFinite(Date.parse(value.createdAt))
    && typeof value.appVersion === 'string'
    && typeof value.platform === 'string'
    && (value.preset === 'quick' || value.preset === 'standard' || value.preset === 'thorough')
    && isNumber(value.iterations)
    && isNumber(value.sharedInitMs)
    && metadataIsCompatible
    && Array.isArray(value.results)
    && value.results.every(isModelResult)
    && isNullableString(value.recommendations.fastest)
    && isNullableString(value.recommendations.mostAccurate)
    && isNullableString(value.recommendations.balanced);
}

export function addBenchmarkReport(
  reports: BenchmarkReport[],
  next: BenchmarkReport,
): BenchmarkReport[] {
  return [
    next,
    ...reports.filter((report) => report.createdAt !== next.createdAt),
  ]
    .sort((left, right) => Date.parse(right.createdAt) - Date.parse(left.createdAt))
    .slice(0, MAX_SAVED_BENCHMARK_REPORTS);
}

export function saveBenchmarkReports(reports: BenchmarkReport[]): BenchmarkReport[] {
  const normalized = reports
    .filter(isBenchmarkReport)
    .sort((left, right) => Date.parse(right.createdAt) - Date.parse(left.createdAt))
    .slice(0, MAX_SAVED_BENCHMARK_REPORTS);
  localStorage.setItem(REPORTS_KEY, JSON.stringify(normalized));
  localStorage.removeItem(REPORT_KEY);
  return normalized;
}

export function loadBenchmarkReports(): BenchmarkReport[] {
  try {
    const saved = localStorage.getItem(REPORTS_KEY);
    if (saved) {
      const parsed: unknown = JSON.parse(saved);
      return Array.isArray(parsed) ? saveBenchmarkReports(parsed.filter(isBenchmarkReport)) : [];
    }

    const legacy = localStorage.getItem(REPORT_KEY);
    if (!legacy) return [];
    const parsed: unknown = JSON.parse(legacy);
    return isBenchmarkReport(parsed) ? saveBenchmarkReports([parsed]) : [];
  } catch {
    return [];
  }
}

export function clearBenchmarkReports(): void {
  localStorage.removeItem(REPORTS_KEY);
  localStorage.removeItem(REPORT_KEY);
}

export async function getBenchmarkModels(): Promise<BenchmarkModel[]> {
  const catalog = await getModelRuntimeCatalog();
  return catalog.map((model) => ({
    modelName: model.modelName,
    label: model.label,
    backend: model.backend,
    accelerator: model.accelerator,
    size: model.size,
    supported: model.supported,
    installed: model.installState === 'installed',
  }));
}

export function getBenchmarkActivity(): Promise<BenchmarkActivity> {
  return invoke('get_benchmark_activity');
}

export function runBenchmark(modelNames: string[], preset: BenchmarkPreset): Promise<BenchmarkReport> {
  return invoke('run_benchmark', { request: { modelNames, preset } });
}

export function cancelBenchmark(): Promise<boolean> {
  return invoke('cancel_benchmark');
}
