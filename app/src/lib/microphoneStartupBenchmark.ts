import { invoke } from '@tauri-apps/api/core';

export const MICROPHONE_STARTUP_BENCHMARK_CYCLES = 5;
export const MAX_SAVED_MICROPHONE_STARTUP_REPORTS = 10;

const REPORTS_KEY = 'murmur-microphone-startup-benchmark-reports';
const MAX_CYCLE_STARTUP_MS = 180_000;
const MAX_ATTEMPT_ACTIVE_MS = 180_000;
const MAX_ATTEMPT_STARTUP_MS = 60_000;
const MAX_ATTEMPT_BUDGET_MS = 60_000;

export type MicrophoneStartupBackend = 'auhal' | 'cpal';
export type MicrophoneStartupOutcome = 'ready' | 'failed' | 'cancelled';
export type MicrophoneStartupSetupTransition = 'entered' | 'completed';
export type MicrophoneStartupProgressPhase =
  | 'starting'
  | 'capturing'
  | 'stopping'
  | 'recovering'
  | 'waiting'
  | 'complete';
export type MicrophoneStartupFailureKind =
  | 'permission_denied'
  | 'device_unavailable'
  | 'host_unavailable'
  | 'invalid_input'
  | 'resource_exhausted'
  | 'stream_invalidated'
  | 'unsupported_config'
  | 'backend_error'
  | 'protocol_error'
  | 'first_buffer_timeout'
  | 'initialization_timeout'
  | 'permission_prompt_timeout'
  | 'termination_unconfirmed'
  | 'worker_panicked'
  | 'signature_invalid';
export type MicrophoneStartupFailurePhase =
  | 'device_enumeration'
  | 'stream_build'
  | 'first_buffer_wait'
  | 'runtime';

export interface MicrophoneStartupAttemptResult {
  resolutionPass: number;
  attemptIndex: number;
  backend: MicrophoneStartupBackend;
  outcome: MicrophoneStartupOutcome;
  attemptStartToFirstPcmMs: number | null;
  activeElapsedMs: number | null;
  failureKind: MicrophoneStartupFailureKind | null;
  failurePhase: MicrophoneStartupFailurePhase | null;
  lastSetupStep: string | null;
  lastSetupTransition: MicrophoneStartupSetupTransition | null;
  attemptBudgetMs: number;
}

export interface MicrophoneStartupBenchmarkProgress {
  schemaVersion: 1;
  runId: string;
  benchmarkRunId: number;
  completedCycles: number;
  totalCycles: number;
  currentCycle: number;
  phase: MicrophoneStartupProgressPhase;
  backend: MicrophoneStartupBackend | null;
  fallbackOccurred: boolean;
  lastSetupStep: string | null;
  lastSetupTransition: MicrophoneStartupSetupTransition | null;
}

export interface MicrophoneStartupCycleResult {
  cycle: number;
  outcome: MicrophoneStartupOutcome;
  cycleStartToFirstPcmMs: number | null;
  backend: MicrophoneStartupBackend | null;
  backendOrder: [MicrophoneStartupBackend, MicrophoneStartupBackend];
  backendOrderSource: 'default' | 'session_first_pcm_memo';
  fallbackOccurred: boolean;
  failureKind: MicrophoneStartupFailureKind | null;
  lastSetupStep: string | null;
  lastSetupTransition: MicrophoneStartupSetupTransition | null;
  attempts: MicrophoneStartupAttemptResult[];
}

export interface MicrophoneStartupBenchmarkReport {
  schemaVersion: 1;
  runId: string;
  benchmarkRunId: number;
  appVersion: string;
  platform: 'macos';
  deviceSelection: 'system_default' | 'pinned';
  requestedCycles: number;
  completedCycles: number;
  cancelled: boolean;
  startedAt: string;
  finishedAt: string;
  cycles: MicrophoneStartupCycleResult[];
}

export interface MicrophoneStartupSummary {
  readyCycles: number;
  failedCycles: number;
  medianMs: number | null;
  p95Ms: number | null;
  minimumMs: number | null;
  maximumMs: number | null;
  auhalWins: number;
  cpalWins: number;
  fallbackCycles: number;
  backendAttempts: Record<MicrophoneStartupBackend, {
    attempts: number;
    ready: number;
    failed: number;
    medianMs: number | null;
    p95Ms: number | null;
    maximumMs: number | null;
  }>;
}

const PROGRESS_KEYS = [
  'schemaVersion', 'runId', 'benchmarkRunId', 'completedCycles', 'totalCycles', 'currentCycle',
  'phase', 'backend', 'fallbackOccurred', 'lastSetupStep', 'lastSetupTransition',
] as const;
const REPORT_KEYS = [
  'schemaVersion', 'runId', 'benchmarkRunId', 'appVersion', 'platform', 'deviceSelection',
  'requestedCycles', 'completedCycles', 'cancelled', 'startedAt', 'finishedAt', 'cycles',
] as const;
const CYCLE_KEYS = [
  'cycle', 'outcome', 'cycleStartToFirstPcmMs', 'backend', 'backendOrder',
  'backendOrderSource', 'fallbackOccurred', 'failureKind', 'lastSetupStep',
  'lastSetupTransition', 'attempts',
] as const;
const ATTEMPT_KEYS = [
  'resolutionPass', 'attemptIndex', 'backend', 'outcome', 'attemptStartToFirstPcmMs',
  'activeElapsedMs', 'failureKind', 'failurePhase', 'lastSetupStep',
  'lastSetupTransition', 'attemptBudgetMs',
] as const;

const FAILURE_KINDS = new Set<MicrophoneStartupFailureKind>([
  'permission_denied', 'device_unavailable', 'host_unavailable', 'invalid_input',
  'resource_exhausted', 'stream_invalidated', 'unsupported_config', 'backend_error',
  'protocol_error', 'first_buffer_timeout', 'initialization_timeout',
  'permission_prompt_timeout', 'termination_unconfirmed', 'worker_panicked', 'signature_invalid',
]);
const FAILURE_PHASES = new Set<MicrophoneStartupFailurePhase>([
  'device_enumeration', 'stream_build', 'first_buffer_wait', 'runtime',
]);
const SETUP_STEPS = new Set([
  'device_resolution', 'audio_unit_creation', 'audio_unit_new', 'enable_input_io',
  'disable_output_io', 'set_current_device', 'format_configuration',
  'callback_installation', 'default_config', 'stream_build', 'stream_start',
  'awaiting_first_callback', 'system_tap_create', 'aggregate_device_create',
  'io_proc_create', 'io_proc_start',
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

function isCycleCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 1 && (value as number) <= 10;
}

function isCompletedCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= 10;
}

function isCurrentCycle(value: unknown, totalCycles: number): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= totalCycles;
}

function isRunId(value: unknown): value is string {
  return typeof value === 'string'
    && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value);
}

function isAppVersion(value: unknown): value is string {
  return typeof value === 'string'
    && value.length >= 1
    && value.length <= 64
    && /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(value);
}

function isPositiveSafeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
}

function isBackend(value: unknown): value is MicrophoneStartupBackend {
  return value === 'auhal' || value === 'cpal';
}

function isTransition(value: unknown): value is MicrophoneStartupSetupTransition {
  return value === 'entered' || value === 'completed';
}

function hasValidStepPair(step: unknown, transition: unknown): boolean {
  return (step === null && transition === null)
    || (typeof step === 'string' && SETUP_STEPS.has(step) && isTransition(transition));
}

function isFailureKind(value: unknown): value is MicrophoneStartupFailureKind {
  return typeof value === 'string' && FAILURE_KINDS.has(value as MicrophoneStartupFailureKind);
}

function isFailurePhase(value: unknown): value is MicrophoneStartupFailurePhase {
  return typeof value === 'string' && FAILURE_PHASES.has(value as MicrophoneStartupFailurePhase);
}

function parseAttempt(
  value: unknown,
  backendOrder: [MicrophoneStartupBackend, MicrophoneStartupBackend],
): MicrophoneStartupAttemptResult | null {
  if (!isRecord(value) || !hasExactKeys(value, ATTEMPT_KEYS)
    || !Number.isSafeInteger(value.resolutionPass)
    || (value.resolutionPass as number) < 1
    || (value.resolutionPass as number) > 3
    || (value.attemptIndex !== 1 && value.attemptIndex !== 2)
    || !isBackend(value.backend)
    || value.backend !== backendOrder[(value.attemptIndex as number) - 1]
    || !['ready', 'failed', 'cancelled'].includes(value.outcome as string)
    || !isPositiveSafeInteger(value.attemptBudgetMs)
    || (value.attemptBudgetMs as number) > MAX_ATTEMPT_BUDGET_MS
    || !(value.failureKind === null || isFailureKind(value.failureKind))
    || !(value.failurePhase === null || isFailurePhase(value.failurePhase))
    || !hasValidStepPair(value.lastSetupStep, value.lastSetupTransition)) return null;

  const outcome = value.outcome as MicrophoneStartupOutcome;
  const latency = value.attemptStartToFirstPcmMs;
  if (outcome === 'ready') {
    if (typeof value.activeElapsedMs !== 'number' || !Number.isFinite(value.activeElapsedMs)
      || value.activeElapsedMs < 0 || value.activeElapsedMs > MAX_ATTEMPT_ACTIVE_MS
      || typeof latency !== 'number' || !Number.isFinite(latency) || latency < 0
      || latency > MAX_ATTEMPT_STARTUP_MS || latency > value.activeElapsedMs
      || value.failureKind !== null || value.failurePhase !== null) return null;
  } else if (outcome === 'failed') {
    if (latency !== null
      || typeof value.activeElapsedMs !== 'number' || !Number.isFinite(value.activeElapsedMs)
      || value.activeElapsedMs < 0 || value.activeElapsedMs > MAX_ATTEMPT_ACTIVE_MS
      || value.failureKind === null || value.failurePhase === null) return null;
  } else if (latency !== null || value.activeElapsedMs !== null
    || value.failureKind !== null || value.failurePhase !== null) return null;

  return {
    resolutionPass: value.resolutionPass as number,
    attemptIndex: value.attemptIndex as number,
    backend: value.backend,
    outcome,
    attemptStartToFirstPcmMs: latency as number | null,
    activeElapsedMs: value.activeElapsedMs as number | null,
    failureKind: value.failureKind as MicrophoneStartupFailureKind | null,
    failurePhase: value.failurePhase as MicrophoneStartupFailurePhase | null,
    lastSetupStep: value.lastSetupStep as string | null,
    lastSetupTransition: value.lastSetupTransition as MicrophoneStartupSetupTransition | null,
    attemptBudgetMs: value.attemptBudgetMs as number,
  };
}

function parseCycle(value: unknown, expectedCycle: number): MicrophoneStartupCycleResult | null {
  if (!isRecord(value) || !hasExactKeys(value, CYCLE_KEYS)
    || value.cycle !== expectedCycle
    || !['ready', 'failed', 'cancelled'].includes(value.outcome as string)
    || !(value.backend === null || isBackend(value.backend))
    || !Array.isArray(value.backendOrder)
    || value.backendOrder.length !== 2
    || !value.backendOrder.every(isBackend)
    || value.backendOrder[0] === value.backendOrder[1]
    || (value.backendOrderSource !== 'default'
      && value.backendOrderSource !== 'session_first_pcm_memo')
    || typeof value.fallbackOccurred !== 'boolean'
    || !(value.failureKind === null || isFailureKind(value.failureKind))
    || !hasValidStepPair(value.lastSetupStep, value.lastSetupTransition)
    || !Array.isArray(value.attempts)
    || value.attempts.length > 6) return null;

  const outcome = value.outcome as MicrophoneStartupOutcome;
  const latency = value.cycleStartToFirstPcmMs;
  if (outcome === 'ready') {
    if (typeof latency !== 'number' || !Number.isFinite(latency) || latency < 0
      || latency > MAX_CYCLE_STARTUP_MS || value.backend === null || value.failureKind !== null) return null;
  } else if (latency !== null) {
    return null;
  }
  if ((outcome === 'failed') !== (value.failureKind !== null)) return null;
  if (outcome !== 'ready' && value.backend !== null) return null;

  const backendOrder = value.backendOrder as [MicrophoneStartupBackend, MicrophoneStartupBackend];
  const attempts: MicrophoneStartupAttemptResult[] = [];
  const attemptKeys = new Set<string>();
  let priorOrder = 0;
  for (const attemptValue of value.attempts) {
    const attempt = parseAttempt(attemptValue, backendOrder);
    if (!attempt) return null;
    const key = `${attempt.resolutionPass}:${attempt.attemptIndex}`;
    const order = (attempt.resolutionPass - 1) * 2 + attempt.attemptIndex;
    if (attemptKeys.has(key) || order <= priorOrder) return null;
    attemptKeys.add(key);
    priorOrder = order;
    attempts.push(attempt);
  }
  const readyAttempts = attempts.filter((attempt) => attempt.outcome === 'ready');
  if (outcome === 'ready' && (readyAttempts.length !== 1 || readyAttempts[0].backend !== value.backend)) return null;
  if (outcome === 'ready'
    && (latency as number) < (readyAttempts[0].attemptStartToFirstPcmMs as number)) return null;
  if (outcome !== 'ready' && readyAttempts.length !== 0) return null;
  if (value.fallbackOccurred !== attempts.some((attempt) => attempt.attemptIndex === 2)) return null;
  const lastAttempt = attempts[attempts.length - 1];
  if ((lastAttempt?.lastSetupStep ?? null) !== value.lastSetupStep
    || (lastAttempt?.lastSetupTransition ?? null) !== value.lastSetupTransition) return null;

  return {
    cycle: expectedCycle,
    outcome,
    cycleStartToFirstPcmMs: latency as number | null,
    backend: value.backend as MicrophoneStartupBackend | null,
    backendOrder: value.backendOrder as [MicrophoneStartupBackend, MicrophoneStartupBackend],
    backendOrderSource: value.backendOrderSource,
    fallbackOccurred: value.fallbackOccurred,
    failureKind: value.failureKind as MicrophoneStartupFailureKind | null,
    lastSetupStep: value.lastSetupStep as string | null,
    lastSetupTransition: value.lastSetupTransition as MicrophoneStartupSetupTransition | null,
    attempts,
  };
}

/** Strict validation for the main-window progress event boundary. */
export function parseMicrophoneStartupBenchmarkProgress(
  value: unknown,
): MicrophoneStartupBenchmarkProgress | null {
  if (!isRecord(value) || !hasExactKeys(value, PROGRESS_KEYS)
    || value.schemaVersion !== 1
    || !isRunId(value.runId)
    || !isPositiveSafeInteger(value.benchmarkRunId)
    || !isCompletedCount(value.completedCycles)
    || !isCycleCount(value.totalCycles)
    || !isCurrentCycle(value.currentCycle, value.totalCycles as number)
    || (value.completedCycles as number) > (value.totalCycles as number)
    || !['starting', 'capturing', 'stopping', 'recovering', 'waiting', 'complete'].includes(value.phase as string)
    || !(value.backend === null || isBackend(value.backend))
    || typeof value.fallbackOccurred !== 'boolean'
    || !hasValidStepPair(value.lastSetupStep, value.lastSetupTransition)) return null;

  if (value.phase === 'capturing' && value.currentCycle === 0) return null;
  return value as unknown as MicrophoneStartupBenchmarkProgress;
}

/** Strict validation for the Rust command result and persisted report boundary. */
export function parseMicrophoneStartupBenchmarkReport(
  value: unknown,
): MicrophoneStartupBenchmarkReport | null {
  if (!isRecord(value) || !hasExactKeys(value, REPORT_KEYS)
    || value.schemaVersion !== 1
    || !isRunId(value.runId)
    || !isPositiveSafeInteger(value.benchmarkRunId)
    || !isAppVersion(value.appVersion)
    || value.platform !== 'macos'
    || (value.deviceSelection !== 'system_default' && value.deviceSelection !== 'pinned')
    || !isCycleCount(value.requestedCycles)
    || !isCompletedCount(value.completedCycles)
    || (value.completedCycles as number) > (value.requestedCycles as number)
    || typeof value.cancelled !== 'boolean'
    || typeof value.startedAt !== 'string'
    || typeof value.finishedAt !== 'string'
    || value.startedAt.length > 64
    || value.finishedAt.length > 64
    || !Number.isFinite(Date.parse(value.startedAt))
    || !Number.isFinite(Date.parse(value.finishedAt))
    || Date.parse(value.finishedAt) < Date.parse(value.startedAt)
    || !Array.isArray(value.cycles)
    || value.cycles.length !== value.completedCycles) return null;

  const cycles: MicrophoneStartupCycleResult[] = [];
  for (const [index, cycle] of value.cycles.entries()) {
    const parsed = parseCycle(cycle, index + 1);
    if (!parsed) return null;
    cycles.push(parsed);
  }
  const firstCycle = cycles[0];
  if (firstCycle && cycles.some((cycle) => (
    cycle.backendOrder[0] !== firstCycle.backendOrder[0]
    || cycle.backendOrder[1] !== firstCycle.backendOrder[1]
    || cycle.backendOrderSource !== firstCycle.backendOrderSource
  ))) return null;
  if (!value.cancelled && value.completedCycles !== value.requestedCycles) return null;
  if (!value.cancelled && cycles.some((cycle) => cycle.outcome === 'cancelled')) return null;
  if (value.cancelled && value.completedCycles === value.requestedCycles
    && cycles[cycles.length - 1]?.outcome !== 'cancelled') return null;

  return {
    schemaVersion: 1,
    runId: value.runId,
    benchmarkRunId: value.benchmarkRunId,
    appVersion: value.appVersion,
    platform: 'macos',
    deviceSelection: value.deviceSelection,
    requestedCycles: value.requestedCycles as number,
    completedCycles: value.completedCycles as number,
    cancelled: value.cancelled,
    startedAt: value.startedAt,
    finishedAt: value.finishedAt,
    cycles,
  };
}

export function summarizeMicrophoneStartupBenchmark(
  report: MicrophoneStartupBenchmarkReport,
): MicrophoneStartupSummary {
  const ready = report.cycles.filter((cycle) => cycle.outcome === 'ready');
  const latencies = ready
    .map((cycle) => cycle.cycleStartToFirstPcmMs as number)
    .sort((left, right) => left - right);
  const middle = Math.floor(latencies.length / 2);
  const medianMs = latencies.length === 0
    ? null
    : latencies.length % 2 === 0
      ? (latencies[middle - 1] + latencies[middle]) / 2
      : latencies[middle];
  const p95Index = Math.max(0, Math.ceil(latencies.length * 0.95) - 1);
  const backendSummary = (backend: MicrophoneStartupBackend) => {
    const attempts = report.cycles.flatMap((cycle) => cycle.attempts)
      .filter((attempt) => attempt.backend === backend);
    const successfulLatencies = attempts
      .filter((attempt) => attempt.outcome === 'ready')
      .map((attempt) => attempt.attemptStartToFirstPcmMs as number)
      .sort((left, right) => left - right);
    const successfulMiddle = Math.floor(successfulLatencies.length / 2);
    const successfulMedian = successfulLatencies.length === 0
      ? null
      : successfulLatencies.length % 2 === 0
        ? (successfulLatencies[successfulMiddle - 1] + successfulLatencies[successfulMiddle]) / 2
        : successfulLatencies[successfulMiddle];
    return {
      attempts: attempts.length,
      ready: successfulLatencies.length,
      failed: attempts.filter((attempt) => attempt.outcome === 'failed').length,
      medianMs: successfulMedian,
      p95Ms: successfulLatencies.length > 0
        ? successfulLatencies[Math.max(0, Math.ceil(successfulLatencies.length * 0.95) - 1)]
        : null,
      maximumMs: successfulLatencies.length > 0
        ? successfulLatencies[successfulLatencies.length - 1]
        : null,
    };
  };
  return {
    readyCycles: ready.length,
    failedCycles: report.cycles.filter((cycle) => cycle.outcome === 'failed').length,
    medianMs,
    p95Ms: latencies.length > 0 ? latencies[p95Index] : null,
    minimumMs: latencies[0] ?? null,
    maximumMs: latencies.length > 0 ? latencies[latencies.length - 1] : null,
    auhalWins: ready.filter((cycle) => cycle.backend === 'auhal').length,
    cpalWins: ready.filter((cycle) => cycle.backend === 'cpal').length,
    fallbackCycles: report.cycles.filter((cycle) => cycle.fallbackOccurred).length,
    backendAttempts: {
      auhal: backendSummary('auhal'),
      cpal: backendSummary('cpal'),
    },
  };
}

export async function runMicrophoneStartupBenchmark(
  runId: string,
  deviceId: string,
  cycles = MICROPHONE_STARTUP_BENCHMARK_CYCLES,
): Promise<MicrophoneStartupBenchmarkReport> {
  const value: unknown = await invoke('run_microphone_startup_benchmark', {
    request: { runId, deviceId, cycles },
  });
  const parsed = parseMicrophoneStartupBenchmarkReport(value);
  if (!parsed || parsed.runId !== runId) {
    throw new Error('Murmur returned an invalid microphone startup report.');
  }
  return parsed;
}

export async function cancelMicrophoneStartupBenchmark(runId: string): Promise<boolean> {
  const value: unknown = await invoke('cancel_microphone_startup_benchmark', { runId });
  if (typeof value !== 'boolean') throw new Error('Murmur returned an invalid cancellation response.');
  return value;
}

export function addMicrophoneStartupReport(
  reports: MicrophoneStartupBenchmarkReport[],
  next: MicrophoneStartupBenchmarkReport,
): MicrophoneStartupBenchmarkReport[] {
  return [next, ...reports.filter((report) => report.startedAt !== next.startedAt)]
    .sort((left, right) => Date.parse(right.startedAt) - Date.parse(left.startedAt))
    .slice(0, MAX_SAVED_MICROPHONE_STARTUP_REPORTS);
}

export function saveMicrophoneStartupReports(
  reports: MicrophoneStartupBenchmarkReport[],
): MicrophoneStartupBenchmarkReport[] {
  const normalized = reports
    .map(parseMicrophoneStartupBenchmarkReport)
    .filter((report): report is MicrophoneStartupBenchmarkReport => report !== null)
    .sort((left, right) => Date.parse(right.startedAt) - Date.parse(left.startedAt))
    .slice(0, MAX_SAVED_MICROPHONE_STARTUP_REPORTS);
  localStorage.setItem(REPORTS_KEY, JSON.stringify(normalized));
  return normalized;
}

export function loadMicrophoneStartupReports(): MicrophoneStartupBenchmarkReport[] {
  try {
    const saved = localStorage.getItem(REPORTS_KEY);
    if (!saved) return [];
    const parsed: unknown = JSON.parse(saved);
    return Array.isArray(parsed) ? saveMicrophoneStartupReports(parsed) : [];
  } catch {
    return [];
  }
}

export function clearMicrophoneStartupReports(): void {
  localStorage.removeItem(REPORTS_KEY);
}

export function saveMicrophoneStartupReport(
  report: MicrophoneStartupBenchmarkReport,
  outputDir: string,
): Promise<string> {
  return invoke('save_microphone_startup_benchmark_report', {
    report,
    outputDir,
  });
}
