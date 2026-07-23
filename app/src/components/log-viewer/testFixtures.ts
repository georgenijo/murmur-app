import {
  PERFORMANCE_STAGES_V1,
  type MeasurementV1,
  type PerformanceRunV1,
  type ResourceRangeV1,
  type ResourceSampleV1,
} from '../../lib/performance';

export const measured = <T>(value: T): MeasurementV1<T> => ({ status: 'measured', value });
export const unavailable = <T>(): MeasurementV1<T> => ({
  status: 'unavailable',
  reason: 'noSamples',
});
export const notApplicable = <T>(): MeasurementV1<T> => ({ status: 'notApplicable' });

const numberRange = (
  start: MeasurementV1<number> = measured(100),
  average: MeasurementV1<number> = measured(120),
  peak: MeasurementV1<number> = measured(150),
  end: MeasurementV1<number> = measured(130),
): ResourceRangeV1<number> => ({ start, average, peak, end });

export function makeRun(overrides: Partial<PerformanceRunV1> = {}): PerformanceRunV1 {
  return {
    schemaVersion: 1,
    runId: '0123456789abcdef0123456789abcdef',
    kind: 'dictation',
    startedAtMs: Date.UTC(2026, 6, 23, 12),
    finishedAtMs: Date.UTC(2026, 6, 23, 12, 0, 2),
    appVersion: '0.20.2',
    correlation: { kind: 'dictation', recordingId: 17 },
    outcome: { status: 'success' },
    runtimes: [{
      role: 'transcription',
      modelId: 'base.en',
      backend: 'whisper',
      accelerator: 'metalGpu',
      warmState: 'warm',
    }],
    stages: PERFORMANCE_STAGES_V1.map(stage => ({
      stage,
      durationMs: stage === 'totalProcessing'
        ? measured(1_000)
        : stage === 'vad'
          ? measured(0)
          : notApplicable(),
      outcome: stage === 'totalProcessing' || stage === 'vad' ? 'completed' : 'skipped',
    })),
    input: {
      audioDurationMs: measured(2_000),
      inputSizeBucket: notApplicable(),
      outputSizeBucket: measured('small'),
      outputTokenCount: notApplicable(),
    },
    resources: {
      sampleCount: 3,
      host: { cpuPercent: numberRange(measured(22), measured(31), measured(48), measured(26)) },
      mainProcess: {
        cpuPercent: numberRange(measured(5), measured(22), measured(77), measured(8)),
        rssBytes: numberRange(
          measured(100 * 1_048_576),
          measured(120 * 1_048_576),
          measured(150 * 1_048_576),
          measured(130 * 1_048_576),
        ),
        rustHeapBytes: numberRange(measured(10), measured(20), measured(30), measured(15)),
        ffiNativeHeapBytes: numberRange(measured(40), measured(50), measured(60), measured(45)),
      },
      sidecarProcess: {
        cpuPercent: numberRange(notApplicable(), notApplicable(), notApplicable(), notApplicable()),
        rssBytes: numberRange(notApplicable(), notApplicable(), notApplicable(), notApplicable()),
      },
    },
    followUps: [],
    ...overrides,
  };
}

export function makeResourceSample(
  observedAtMs: number,
  overrides: Partial<ResourceSampleV1> = {},
): ResourceSampleV1 {
  return {
    schemaVersion: 1,
    observedAtMs,
    host: { cpuPercent: measured(35) },
    mainProcess: {
      cpuPercent: measured(120),
      rssBytes: measured(420 * 1_048_576),
      rustHeapBytes: measured(42 * 1_048_576),
      ffiNativeHeapBytes: measured(180 * 1_048_576),
    },
    sidecarProcess: {
      cpuPercent: unavailable(),
      rssBytes: unavailable(),
    },
    ...overrides,
  };
}
