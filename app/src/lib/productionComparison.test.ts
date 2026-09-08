import { describe, expect, it } from 'vitest';
import { makeRun, measured, notApplicable } from '../components/log-viewer/testFixtures';
import type { PerformanceRunV1, ProductionRunV1, RuntimeIdentityV1 } from './performance';
import {
  compareProductionVersions,
  compareStatistic,
  PRODUCTION_COMPARISON_METRICS,
  PRODUCTION_COMPARISON_THRESHOLDS,
  recordingDurationBucket,
  summarizeMetricSamples,
} from './productionComparison';

const runtime: RuntimeIdentityV1 = {
  role: 'transcription',
  modelId: 'base.en',
  backend: 'whisper',
  accelerator: 'metalGpu',
  warmState: 'warm',
};

function production(
  cohort: Partial<ProductionRunV1['cohort']> = {},
  capture: Partial<ProductionRunV1['capture']> = {},
): ProductionRunV1 {
  return {
    schemaVersion: 1,
    cohort: {
      machineId: '123e4567-e89b-12d3-a456-426614174000',
      osVersion: '15.6.1',
      architecture: 'aarch64',
      buildMode: 'release',
      microphoneSelection: 'systemDefault',
      microphoneKind: 'builtIn',
      configurationKey: 'a'.repeat(64),
      ...cohort,
    },
    capture: {
      helperResolveMs: 4,
      helperSignatureMs: 5,
      helperSpawnMs: 10,
      streamOpenMs: 40,
      firstCallbackWaitMs: 90,
      firstPcmMs: 145,
      readyMs: 180,
      stopToWorkerExitMs: 20,
      backend: 'auhal',
      fallbackAttempted: false,
      fallbackSucceeded: null,
      failureKind: null,
      workerInvariantViolation: false,
      zeroSampleSuccess: false,
      ...capture,
    },
  };
}

function comparisonRun(
  appVersion: string,
  overrides: Partial<PerformanceRunV1> = {},
): PerformanceRunV1 {
  return makeRun({
    appVersion,
    runtimes: [runtime],
    production: production(),
    ...overrides,
  });
}

describe('production version comparison', () => {
  it('uses the exact recording-duration boundaries', () => {
    expect([0, 1, 5_000, 5_001, 15_000, 15_001, 60_000, 60_001]
      .map(recordingDurationBucket))
      .toEqual([
        'empty', 'under5s', 'under5s', '5to15s',
        '5to15s', '15to60s', '15to60s', 'over60s',
      ]);
  });

  it('matches exact cohorts with runtime identity independent of list order', () => {
    const generation: RuntimeIdentityV1 = {
      role: 'generation',
      modelId: 'rewrite.gguf',
      backend: 'llamaCpp',
      accelerator: 'metalGpu',
      warmState: 'coldLoaded',
    };
    const baseline = comparisonRun('1.0.0', { runtimes: [runtime, generation] });
    const candidate = comparisonRun('1.1.0', { runtimes: [generation, runtime] });
    const result = compareProductionVersions([baseline, candidate], '1.0.0', '1.1.0');

    expect(result.cohorts).toHaveLength(1);
    expect(result.baselineUnmatchedCount).toBe(0);
    expect(result.candidateUnmatchedCount).toBe(0);
    expect(result.cohorts[0].metrics).toHaveLength(PRODUCTION_COMPARISON_METRICS.length);
  });

  it('keeps multiple compatible cohorts separate', () => {
    const baselineBuiltIn = comparisonRun('1.0.0');
    const candidateBuiltIn = comparisonRun('1.1.0');
    const baselineBluetooth = comparisonRun('1.0.0', {
      production: production({ microphoneKind: 'bluetooth' }),
    });
    const candidateBluetooth = comparisonRun('1.1.0', {
      production: production({ microphoneKind: 'bluetooth' }),
    });

    const result = compareProductionVersions(
      [baselineBuiltIn, candidateBuiltIn, baselineBluetooth, candidateBluetooth],
      '1.0.0',
      '1.1.0',
    );
    expect(result.cohorts).toHaveLength(2);
    expect(result.cohorts.map(cohort => [cohort.baselineRunCount, cohort.candidateRunCount]))
      .toEqual([[1, 1], [1, 1]]);
  });

  it.each([
    ['machine', { production: production({ machineId: '223e4567-e89b-12d3-a456-426614174000' }) }],
    ['operating system', { production: production({ osVersion: '15.7' }) }],
    ['architecture', { production: production({ architecture: 'x86_64' }) }],
    ['microphone selection', { production: production({ microphoneSelection: 'smartAuto' }) }],
    ['microphone kind', { production: production({ microphoneKind: 'bluetooth' }) }],
    ['configuration', { production: production({ configurationKey: 'b'.repeat(64) }) }],
    ['runtime', { runtimes: [{ ...runtime, warmState: 'coldLoaded' }] }],
    ['capture backend', { production: production({}, { backend: 'cpal' }) }],
    ['recording duration', { input: { ...makeRun().input, audioDurationMs: measured(8_000) } }],
    ['output size', { input: { ...makeRun().input, outputSizeBucket: measured('large') } }],
  ] satisfies Array<[string, Partial<PerformanceRunV1>]>)('keeps a %s mismatch out of a pooled verdict', (dimension, override) => {
    const result = compareProductionVersions([
      comparisonRun('1.0.0'),
      comparisonRun('1.1.0', override),
    ], '1.0.0', '1.1.0');

    expect(result.cohorts).toHaveLength(0);
    expect(result.baselineUnmatchedCount).toBe(1);
    expect(result.candidateUnmatchedCount).toBe(1);
    expect(result.mismatchedDimensions).toContain(dimension);
  });

  it('excludes unknown cohort facts and development runs with explicit reasons', () => {
    const runs = [
      comparisonRun('1.0.0', { production: undefined }),
      comparisonRun('1.0.0', { production: production({ buildMode: 'development' }) }),
      comparisonRun('1.0.0', { production: production({ osVersion: null }) }),
      comparisonRun('1.0.0', { production: production({ architecture: 'other' }) }),
      comparisonRun('1.0.0', { production: production({ microphoneKind: null }) }),
      comparisonRun('1.0.0', { production: production({ configurationKey: null }) }),
      comparisonRun('1.0.0', { production: production({}, { backend: null }) }),
      comparisonRun('1.0.0', { runtimes: [{ ...runtime, warmState: 'unknown' }] }),
      comparisonRun('1.0.0', {
        input: { ...makeRun().input, audioDurationMs: notApplicable() },
      }),
      comparisonRun('1.0.0', {
        input: { ...makeRun().input, outputSizeBucket: notApplicable() },
      }),
      comparisonRun('1.1.0'),
    ];
    const result = compareProductionVersions(runs, '1.0.0', '1.1.0');

    expect(result.baselineEligibleCount).toBe(0);
    expect(result.exclusions.map(exclusion => exclusion.reason)).toEqual([
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
    ]);
  });

  it('computes odd and even medians, excludes missing values, and gates nearest-rank p95 at 20', () => {
    expect(summarizeMetricSamples([1, 7, 3]).medianMs).toBe(3);
    expect(summarizeMetricSamples([1, 3, 7, 9]).medianMs).toBe(5);
    expect(summarizeMetricSamples([0, null, 10])).toMatchObject({
      measuredCount: 2,
      missingCount: 1,
      medianMs: 5,
      p95Ms: null,
    });
    expect(summarizeMetricSamples(Array.from({ length: 19 }, (_, index) => index + 1)).p95Ms)
      .toBeNull();
    expect(summarizeMetricSamples(Array.from({ length: 20 }, (_, index) => index + 1)).p95Ms)
      .toBe(19);
  });

  it('applies strict AND thresholds and does not invent a percentage from zero', () => {
    const threshold = PRODUCTION_COMPARISON_THRESHOLDS.median;
    expect(compareStatistic(100, 121, threshold).verdict).toBe('regression');
    expect(compareStatistic(100, 120, threshold).verdict).toBe('withinThreshold');
    expect(compareStatistic(100, 115, threshold).verdict).toBe('withinThreshold');
    expect(compareStatistic(1_000, 1_021, threshold).verdict).toBe('withinThreshold');
    expect(compareStatistic(0, 100, threshold)).toMatchObject({
      absoluteDeltaMs: 100,
      percentageDelta: null,
      verdict: 'unavailable',
    });
    expect(compareStatistic(null, 100, threshold).verdict).toBe('unavailable');
    const p95 = PRODUCTION_COMPARISON_THRESHOLDS.p95;
    expect(compareStatistic(100, 131, p95).verdict).toBe('regression');
    expect(compareStatistic(100, 130, p95).verdict).toBe('withinThreshold');
    expect(compareStatistic(150, 181, p95).verdict).toBe('regression');
    expect(compareStatistic(150, 180, p95).verdict).toBe('withinThreshold');
  });

  it('uses success-only metric samples and keeps missing measurements separate', () => {
    const baselineSuccess = comparisonRun('1.0.0');
    const baselineFailure = comparisonRun('1.0.0', {
      outcome: { status: 'failed', stage: 'inferenceDecode', errorCode: 'inferenceFailed' },
      production: production({}, { readyMs: 999 }),
    });
    const candidate = comparisonRun('1.1.0', {
      production: production({}, { readyMs: null, helperResolveMs: 0 }),
    });
    const result = compareProductionVersions(
      [baselineSuccess, baselineFailure, candidate],
      '1.0.0',
      '1.1.0',
    );
    const ready = result.cohorts[0].metrics.find(metric => metric.definition.key === 'readyMs');
    const resolve = result.cohorts[0].metrics.find(metric => metric.definition.key === 'helperResolveMs');

    expect(ready?.baseline.rawMs).toEqual([180]);
    expect(ready?.candidate).toMatchObject({ measuredCount: 0, missingCount: 1 });
    expect(resolve?.candidate.rawMs).toEqual([0]);
  });

  it('shows one-run anomalies across mixed versions without a compatible latency cohort', () => {
    const baseline = comparisonRun('1.0.0');
    const candidate = comparisonRun('1.1.0', {
      outcome: { status: 'timedOut', stage: 'captureFinalization' },
      production: production(
        { machineId: '223e4567-e89b-12d3-a456-426614174000' },
        {
          fallbackAttempted: true,
          fallbackSucceeded: false,
          failureKind: 'first_buffer_timeout',
          workerInvariantViolation: true,
          zeroSampleSuccess: true,
        },
      ),
    });
    const irrelevant = comparisonRun('2.0.0');
    const result = compareProductionVersions([baseline, candidate, irrelevant], '1.0.0', '1.1.0');

    expect(result.cohorts).toHaveLength(0);
    expect(result.observations).toEqual(expect.arrayContaining([
      expect.objectContaining({ key: 'failure', candidate: 1, newlyObserved: true }),
      expect.objectContaining({ key: 'fallbackAttempted', candidate: 1, newlyObserved: true }),
      expect.objectContaining({ key: 'captureFailure', candidate: 1, newlyObserved: false, candidateObserved: true }),
      expect.objectContaining({ key: 'zeroSampleSuccess', candidate: 1, newlyObserved: true }),
      expect.objectContaining({ key: 'workerInvariantViolation', candidate: 1, newlyObserved: true }),
    ]));
  });

  it('refuses legacy undifferentiated external microphone cohorts', () => {
    const result = compareProductionVersions([
      comparisonRun('1.0.0', { production: production({ microphoneKind: 'external' }) }),
      comparisonRun('1.1.0', { production: production({ microphoneKind: 'external' }) }),
    ], '1.0.0', '1.1.0');
    expect(result.cohorts).toHaveLength(0);
    expect(result.exclusions).toEqual(expect.arrayContaining([
      expect.objectContaining({ reason: 'microphoneKindUnknown' }),
    ]));
  });

  it('requires known negative baseline evidence before calling an anomaly new', () => {
    const known = comparisonRun('1.0.0');
    const historical = comparisonRun('1.0.0', { production: undefined });
    const unknown = comparisonRun('1.0.0', {
      production: production({}, { fallbackAttempted: null }),
    });
    const candidate = comparisonRun('1.1.0', {
      production: production({}, { fallbackAttempted: true }),
    });
    for (const baseline of [[], [historical], [unknown], [known, historical], [known, unknown]]) {
      const result = compareProductionVersions([...baseline, candidate], '1.0.0', '1.1.0');
      expect(result.observations.find(observation => observation.key === 'fallbackAttempted'))
        .toMatchObject({ newlyObserved: false, candidateObserved: true });
    }
    const result = compareProductionVersions([known, candidate], '1.0.0', '1.1.0');
    expect(result.observations.find(observation => observation.key === 'fallbackAttempted'))
      .toMatchObject({ newlyObserved: true, candidateObserved: true, baselineUnavailable: 0 });
  });

});
