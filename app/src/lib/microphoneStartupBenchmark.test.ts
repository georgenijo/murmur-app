import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import {
  MAX_SAVED_MICROPHONE_STARTUP_REPORTS,
  addMicrophoneStartupReport,
  cancelMicrophoneStartupBenchmark,
  loadMicrophoneStartupReports,
  parseMicrophoneStartupBenchmarkProgress,
  parseMicrophoneStartupBenchmarkReport,
  runMicrophoneStartupBenchmark,
  saveMicrophoneStartupReport,
  saveMicrophoneStartupReports,
  summarizeMicrophoneStartupBenchmark,
  type MicrophoneStartupBenchmarkReport,
} from './microphoneStartupBenchmark';

const RUN_ID = '888d2ca8-7f8c-4f1c-bc52-a60ee41ff901';

function report(overrides: Partial<MicrophoneStartupBenchmarkReport> = {}): MicrophoneStartupBenchmarkReport {
  return {
    schemaVersion: 1,
    runId: RUN_ID,
    benchmarkRunId: 7,
    appVersion: '0.34.0',
    platform: 'macos',
    deviceSelection: 'pinned',
    requestedCycles: 2,
    completedCycles: 2,
    cancelled: false,
    startedAt: '2026-08-14T12:00:00.000Z',
    finishedAt: '2026-08-14T12:00:03.000Z',
    cycles: [
      {
        cycle: 1,
        outcome: 'ready',
        cycleStartToFirstPcmMs: 100,
        backend: 'auhal',
        backendOrder: ['auhal', 'cpal'],
        backendOrderSource: 'default',
        fallbackOccurred: false,
        failureKind: null,
        lastSetupStep: 'awaiting_first_callback',
        lastSetupTransition: 'completed',
        attempts: [{
          resolutionPass: 1,
          attemptIndex: 1,
          backend: 'auhal',
          outcome: 'ready',
          attemptStartToFirstPcmMs: 95,
          activeElapsedMs: 100,
          failureKind: null,
          failurePhase: null,
          lastSetupStep: 'awaiting_first_callback',
          lastSetupTransition: 'completed',
          attemptBudgetMs: 8_000,
        }],
      },
      {
        cycle: 2,
        outcome: 'ready',
        cycleStartToFirstPcmMs: 350,
        backend: 'cpal',
        backendOrder: ['auhal', 'cpal'],
        backendOrderSource: 'default',
        fallbackOccurred: true,
        failureKind: null,
        lastSetupStep: 'awaiting_first_callback',
        lastSetupTransition: 'completed',
        attempts: [
          {
            resolutionPass: 1,
            attemptIndex: 1,
            backend: 'auhal',
            outcome: 'failed',
            attemptStartToFirstPcmMs: null,
            activeElapsedMs: 200,
            failureKind: 'backend_error',
            failurePhase: 'stream_build',
            lastSetupStep: 'stream_build',
            lastSetupTransition: 'entered',
            attemptBudgetMs: 8_000,
          },
          {
            resolutionPass: 1,
            attemptIndex: 2,
            backend: 'cpal',
            outcome: 'ready',
            attemptStartToFirstPcmMs: 140,
            activeElapsedMs: 145,
            failureKind: null,
            failurePhase: null,
            lastSetupStep: 'awaiting_first_callback',
            lastSetupTransition: 'completed',
            attemptBudgetMs: 16_000,
          },
        ],
      },
    ],
    ...overrides,
  };
}

describe('microphone startup benchmark boundary', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  it('accepts an exact bounded report and derives cycle and per-backend distributions', () => {
    const parsed = parseMicrophoneStartupBenchmarkReport(report());
    expect(parsed).not.toBeNull();
    expect(summarizeMicrophoneStartupBenchmark(parsed!)).toEqual({
      readyCycles: 2,
      failedCycles: 0,
      medianMs: 225,
      p95Ms: 350,
      minimumMs: 100,
      maximumMs: 350,
      auhalWins: 1,
      cpalWins: 1,
      fallbackCycles: 1,
      backendAttempts: {
        auhal: { attempts: 2, ready: 1, failed: 1, medianMs: 95, p95Ms: 95, maximumMs: 95 },
        cpal: { attempts: 1, ready: 1, failed: 0, medianMs: 140, p95Ms: 140, maximumMs: 140 },
      },
    });
  });

  it('rejects unknown keys, unrecognized constants, contradictory timing, and bad attempt order', () => {
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), extra: true })).toBeNull();

    const unknownFailure = structuredClone(report());
    unknownFailure.cycles[1].attempts[0].failureKind = 'raw_coreaudio_error' as never;
    expect(parseMicrophoneStartupBenchmarkReport(unknownFailure)).toBeNull();

    const impossibleTiming = structuredClone(report());
    impossibleTiming.cycles[0].attempts[0].attemptStartToFirstPcmMs = 101;
    expect(parseMicrophoneStartupBenchmarkReport(impossibleTiming)).toBeNull();

    const impossibleCycleTiming = structuredClone(report());
    impossibleCycleTiming.cycles[0].cycleStartToFirstPcmMs = 94;
    expect(parseMicrophoneStartupBenchmarkReport(impossibleCycleTiming)).toBeNull();

    const duplicateAttempt = structuredClone(report());
    duplicateAttempt.cycles[1].attempts.push(structuredClone(duplicateAttempt.cycles[1].attempts[0]));
    expect(parseMicrophoneStartupBenchmarkReport(duplicateAttempt)).toBeNull();

    const wrongBackend = structuredClone(report());
    wrongBackend.cycles[1].attempts[0].backend = 'cpal';
    expect(parseMicrophoneStartupBenchmarkReport(wrongBackend)).toBeNull();

    const driftedPlan = structuredClone(report());
    driftedPlan.cycles[1].backendOrderSource = 'session_first_pcm_memo';
    expect(parseMicrophoneStartupBenchmarkReport(driftedPlan)).toBeNull();

    const contradictoryFallback = structuredClone(report());
    contradictoryFallback.cycles[1].fallbackOccurred = false;
    expect(parseMicrophoneStartupBenchmarkReport(contradictoryFallback)).toBeNull();

    const cancelledAttempt = structuredClone(report());
    cancelledAttempt.requestedCycles = 3;
    cancelledAttempt.completedCycles = 2;
    cancelledAttempt.cancelled = true;
    cancelledAttempt.cycles[1] = {
      ...cancelledAttempt.cycles[1],
      outcome: 'cancelled',
      cycleStartToFirstPcmMs: null,
      backend: null,
      failureKind: null,
      attempts: [{
        ...cancelledAttempt.cycles[1].attempts[0],
        outcome: 'cancelled',
        activeElapsedMs: null,
        failureKind: null,
        failurePhase: null,
      }],
      fallbackOccurred: false,
      lastSetupStep: 'stream_build',
      lastSetupTransition: 'entered',
    };
    expect(parseMicrophoneStartupBenchmarkReport(cancelledAttempt)).not.toBeNull();

    const fullCancelled = structuredClone(cancelledAttempt);
    fullCancelled.requestedCycles = 2;
    expect(parseMicrophoneStartupBenchmarkReport(fullCancelled)).not.toBeNull();
    fullCancelled.cancelled = false;
    expect(parseMicrophoneStartupBenchmarkReport(fullCancelled)).toBeNull();

    const contradictoryFullCancellation = report({ cancelled: true });
    expect(parseMicrophoneStartupBenchmarkReport(contradictoryFullCancellation)).toBeNull();

    cancelledAttempt.cycles[1].attempts[0].activeElapsedMs = 1;
    expect(parseMicrophoneStartupBenchmarkReport(cancelledAttempt)).toBeNull();
  });

  it('accepts correlated partial completion progress and rejects stale-shaped payloads', () => {
    const progress = {
      schemaVersion: 1,
      runId: RUN_ID,
      benchmarkRunId: 7,
      completedCycles: 2,
      totalCycles: 5,
      currentCycle: 3,
      phase: 'complete',
      backend: null,
      fallbackOccurred: false,
      lastSetupStep: null,
      lastSetupTransition: null,
    };
    expect(parseMicrophoneStartupBenchmarkProgress(progress)).toEqual(progress);
    expect(parseMicrophoneStartupBenchmarkProgress({ ...progress, runId: '' })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkProgress({ ...progress, lastSetupStep: 'private_device_name' })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkProgress({ ...progress, unexpected: true })).toBeNull();
  });

  it('retains only valid newest reports within the bounded local dashboard', () => {
    const reports = Array.from({ length: MAX_SAVED_MICROPHONE_STARTUP_REPORTS + 3 }, (_, index) => report({
      runId: `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
      benchmarkRunId: index + 1,
      startedAt: `2026-08-14T12:${String(index).padStart(2, '0')}:00.000Z`,
      finishedAt: `2026-08-14T12:${String(index).padStart(2, '0')}:03.000Z`,
    }));
    const saved = saveMicrophoneStartupReports(reports);
    expect(saved).toHaveLength(MAX_SAVED_MICROPHONE_STARTUP_REPORTS);
    expect(saved[0].runId).toBe('00000000-0000-4000-8000-000000000012');
    expect(loadMicrophoneStartupReports()).toEqual(saved);

    expect(addMicrophoneStartupReport(saved, saved[0])).toHaveLength(MAX_SAVED_MICROPHONE_STARTUP_REPORTS);
  });

  it('uses exact run correlation for run and cancellation commands', async () => {
    mocks.invoke.mockResolvedValueOnce(report());
    await expect(runMicrophoneStartupBenchmark(RUN_ID, 'device-id', 2)).resolves.toEqual(report());
    expect(mocks.invoke).toHaveBeenLastCalledWith('run_microphone_startup_benchmark', {
      request: { runId: RUN_ID, deviceId: 'device-id', cycles: 2 },
    });

    mocks.invoke.mockResolvedValueOnce(true);
    await expect(cancelMicrophoneStartupBenchmark(RUN_ID)).resolves.toBe(true);
    expect(mocks.invoke).toHaveBeenLastCalledWith('cancel_microphone_startup_benchmark', { runId: RUN_ID });

    mocks.invoke.mockResolvedValueOnce(report({ runId: '11111111-1111-4111-8111-111111111111' }));
    await expect(runMicrophoneStartupBenchmark(RUN_ID, 'device-id', 2)).rejects.toThrow('invalid microphone startup report');

    mocks.invoke.mockResolvedValueOnce(`/tmp/murmur-microphone-startup-20260814T120000.000Z-${RUN_ID}.json`);
    await expect(saveMicrophoneStartupReport(report(), '/tmp/reports'))
      .resolves.toBe(`/tmp/murmur-microphone-startup-20260814T120000.000Z-${RUN_ID}.json`);
    expect(mocks.invoke).toHaveBeenLastCalledWith('save_microphone_startup_benchmark_report', {
      report: report(),
      outputDir: '/tmp/reports',
    });
  });

  it('rejects free-form metadata at the persistence boundary', () => {
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), platform: 'macos / private-host' })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), appVersion: '/Users/me/build' })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), runId: 'private-user-label' })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), startedAt: `${'2'.repeat(80)}Z` })).toBeNull();
    expect(parseMicrophoneStartupBenchmarkReport({ ...report(), appVersion: '1.2.3-rc.1+build.7' })).not.toBeNull();
  });

  it('matches the Rust SemVer and RFC 3339 provenance contract', () => {
    expect(parseMicrophoneStartupBenchmarkReport(report({
      appVersion: '1.2.3-rc.1+build.01',
      startedAt: '2026-08-14t12:00:00.123456789123z',
      finishedAt: '2026-08-14 12:00:01+00:00',
    }))).not.toBeNull();

    for (const appVersion of ['01.2.3', '1.2.3-01', '1.2.3-a..b', '1.2.3+', '1.2.3+a..b']) {
      expect(parseMicrophoneStartupBenchmarkReport(report({ appVersion })), appVersion).toBeNull();
    }
    for (const startedAt of [
      '2026-08-14',
      '2026-08-14T12:00:00',
      '2026-02-30T12:00:00Z',
      '2026-08-14T12:00:00+24:00',
    ]) {
      expect(parseMicrophoneStartupBenchmarkReport(report({ startedAt })), startedAt).toBeNull();
    }
  });

  it('rejects fractional values for every u64-derived timing field', () => {
    const fractionalCycle = structuredClone(report());
    fractionalCycle.cycles[0].cycleStartToFirstPcmMs = 100.5;
    expect(parseMicrophoneStartupBenchmarkReport(fractionalCycle)).toBeNull();

    const fractionalStartup = structuredClone(report());
    fractionalStartup.cycles[0].attempts[0].attemptStartToFirstPcmMs = 94.5;
    expect(parseMicrophoneStartupBenchmarkReport(fractionalStartup)).toBeNull();

    const fractionalElapsed = structuredClone(report());
    fractionalElapsed.cycles[0].attempts[0].activeElapsedMs = 100.5;
    expect(parseMicrophoneStartupBenchmarkReport(fractionalElapsed)).toBeNull();

    const fractionalBudget = structuredClone(report());
    fractionalBudget.cycles[0].attempts[0].attemptBudgetMs = 8_000.5;
    expect(parseMicrophoneStartupBenchmarkReport(fractionalBudget)).toBeNull();
  });
});
