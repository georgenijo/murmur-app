import { describe, expect, it } from 'vitest';
import {
  isMeasurementV1,
  isPerformanceRunV1,
  isResourceSampleV1,
  measuredValue,
  PERFORMANCE_STAGES_V1,
  type MeasurementV1,
} from './performance';

describe('performance contracts', () => {
  it('keeps measured zero distinct from unavailable and not-applicable', () => {
    const measured: MeasurementV1<number> = { status: 'measured', value: 0 };
    expect(isMeasurementV1(measured, (value): value is number => typeof value === 'number')).toBe(true);
    expect(measuredValue(measured)).toBe(0);
    expect(measuredValue({ status: 'notApplicable' })).toBeNull();
    expect(measuredValue({ status: 'unavailable', reason: 'noSamples' })).toBeNull();
  });

  it('rejects unknown resource schema versions and zero sentinels', () => {
    const sample = {
      schemaVersion: 1,
      observedAtMs: 1,
      host: { cpuPercent: { status: 'unavailable', reason: 'sampleFailed' } },
      mainProcess: {
        cpuPercent: { status: 'measured', value: 0 },
        rssBytes: { status: 'measured', value: 0 },
        rustHeapBytes: { status: 'notApplicable' },
        ffiNativeHeapBytes: { status: 'unavailable', reason: 'unsupportedPlatform' },
      },
      sidecarProcess: {
        cpuPercent: { status: 'unavailable', reason: 'dependencyPending' },
        rssBytes: { status: 'unavailable', reason: 'dependencyPending' },
      },
    };
    expect(isResourceSampleV1(sample)).toBe(true);
    expect(isResourceSampleV1({ ...sample, schemaVersion: 2 })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      host: { cpuPercent: 0 },
    })).toBe(false);
  });

  it('requires correlation to match the run kind', () => {
    const unavailable = { status: 'unavailable', reason: 'noSamples' };
    const notApplicable = { status: 'notApplicable' };
    const range = {
      start: unavailable,
      average: unavailable,
      peak: unavailable,
      end: unavailable,
    };
    const base = {
      schemaVersion: 1,
      runId: '0123456789abcdef0123456789abcdef',
      kind: 'dictation',
      startedAtMs: 1,
      finishedAtMs: 2,
      appVersion: '1.0.0',
      correlation: { kind: 'dictation', recordingId: 4 },
      outcome: { status: 'success' },
      runtimes: [],
      stages: PERFORMANCE_STAGES_V1.map(stage => ({
        stage,
        durationMs: notApplicable,
        outcome: 'skipped',
      })),
      input: {
        audioDurationMs: { status: 'measured', value: 100 },
        inputSizeBucket: notApplicable,
        outputSizeBucket: { status: 'measured', value: 'small' },
        outputTokenCount: notApplicable,
      },
      resources: {
        sampleCount: 0,
        host: { cpuPercent: range },
        mainProcess: {
          cpuPercent: range,
          rssBytes: range,
          rustHeapBytes: range,
          ffiNativeHeapBytes: range,
        },
        sidecarProcess: {
          cpuPercent: {
            start: notApplicable,
            average: notApplicable,
            peak: notApplicable,
            end: notApplicable,
          },
          rssBytes: {
            start: notApplicable,
            average: notApplicable,
            peak: notApplicable,
            end: notApplicable,
          },
        },
      },
      followUps: [],
    };
    expect(isPerformanceRunV1(base)).toBe(true);
    expect(isPerformanceRunV1({
      ...base,
      correlation: { kind: 'fileTranscription', fileRunId: 4 },
    })).toBe(false);
  });
});
