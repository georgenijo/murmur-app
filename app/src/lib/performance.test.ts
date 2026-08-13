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
    expect(isResourceSampleV1({ ...sample, question: 'private' })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      host: { ...sample.host, context: 'private' },
    })).toBe(false);
    expect(isResourceSampleV1({
      ...sample,
      mainProcess: {
        ...sample.mainProcess,
        rssBytes: { status: 'measured', value: 0, detail: 'private' },
      },
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

    const voiceQuery = {
      ...base,
      kind: 'voiceQuery',
      correlation: { kind: 'voiceQuery', queryPassId: 9 },
      queryProcess: { exitCode: 0, stderrPresent: false },
    };
    expect(isPerformanceRunV1(voiceQuery)).toBe(true);
    const { queryProcess: _legacyMissingProcess, ...legacyVoiceQuery } = voiceQuery;
    expect(isPerformanceRunV1(legacyVoiceQuery)).toBe(true);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      correlation: { kind: 'selectedTextTransform', transformPassId: 9 },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      queryProcess: { exitCode: 0, stderrPresent: true, stderr: 'secret' },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      queryProcess: { exitCode: '0', stderrPresent: true },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...voiceQuery,
      kind: 'dictation',
      correlation: { kind: 'dictation', recordingId: 9 },
    })).toBe(false);
  });

  it('rejects undeclared content recursively at every performance-run boundary', () => {
    const unavailable = { status: 'unavailable', reason: 'noSamples' };
    const notApplicable = { status: 'notApplicable' };
    const range = {
      start: unavailable,
      average: unavailable,
      peak: unavailable,
      end: unavailable,
    };
    const run = {
      schemaVersion: 1,
      runId: '0123456789abcdef0123456789abcdef',
      kind: 'voiceQuery',
      startedAtMs: 1,
      finishedAtMs: 2,
      appVersion: '1.0.0',
      correlation: { kind: 'voiceQuery', queryPassId: 9 },
      outcome: { status: 'success' },
      runtimes: [{
        role: 'transcription',
        modelId: 'parakeet-v3',
        backend: 'parakeet',
        accelerator: 'appleNeuralEngine',
        warmState: 'warm',
      }],
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
        sidecarProcess: { cpuPercent: range, rssBytes: range },
      },
      followUps: [{
        kind: 'apply',
        atMs: 2,
        durationMs: notApplicable,
        outcome: 'skipped',
      }],
      queryProcess: { exitCode: 0, stderrPresent: true },
    };
    expect(isPerformanceRunV1(run)).toBe(true);

    const privateValue = 'must not cross the diagnostics boundary';
    expect(isPerformanceRunV1({ ...run, question: privateValue })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      correlation: { ...run.correlation, answer: privateValue },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      outcome: { status: 'success', context: privateValue },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      runtimes: [{ ...run.runtimes[0], stderr: privateValue }],
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      stages: run.stages.map((stage, index) => (
        index === 0 ? { ...stage, detail: privateValue } : stage
      )),
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      input: {
        ...run.input,
        audioDurationMs: { status: 'measured', value: 100, question: privateValue },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      resources: {
        ...run.resources,
        host: { ...run.resources.host, context: privateValue },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      resources: {
        ...run.resources,
        mainProcess: {
          ...run.resources.mainProcess,
          rssBytes: { ...run.resources.mainProcess.rssBytes, stderr: privateValue },
        },
      },
    })).toBe(false);
    expect(isPerformanceRunV1({
      ...run,
      followUps: [{ ...run.followUps[0], answer: privateValue }],
    })).toBe(false);
  });
});
