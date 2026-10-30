import { describe, expect, it } from 'vitest';
import { makeRun, measured, notApplicable, unavailable } from '../components/log-viewer/testFixtures';
import {
  correlationFilterForRun,
  dictationStagePercentiles,
  stageLabel,
  formatMeasurement,
  orderedStages,
  rateForRun,
  resourceDelta,
} from './performancePresentation';

describe('performance presentation semantics', () => {
  it('shows final inference contention separately from preparation without inventing legacy measurements', () => {
    const run = makeRun();
    run.stages = run.stages.map(timing => timing.stage === 'modelQueue'
      ? { ...timing, durationMs: measured(0) }
      : timing.stage === 'inferenceQueue'
        ? { ...timing, durationMs: measured(64) }
        : timing);
    expect(stageLabel('inferenceQueue', 'dictation')).toBe('Final inference queue');
    expect(orderedStages(run).find(timing => timing.stage === 'modelQueue')?.durationMs).toEqual(measured(0));
    expect(orderedStages(run).find(timing => timing.stage === 'inferenceQueue')?.durationMs).toEqual(measured(64));
    run.stages = run.stages.filter(timing => timing.stage !== 'inferenceQueue');
    expect(orderedStages(run).some(timing => timing.stage === 'inferenceQueue')).toBe(false);
  });

  it('keeps measured zero, unavailable, and not-applicable distinct', () => {
    expect(formatMeasurement(measured(0), value => `${value} ms`)).toEqual({
      text: '0 ms',
      status: 'measured',
    });
    expect(formatMeasurement(unavailable<number>(), value => `${value} ms`)).toMatchObject({
      text: 'Unavailable',
      status: 'unavailable',
    });
    expect(formatMeasurement(notApplicable<number>(), value => `${value} ms`)).toEqual({
      text: 'Not applicable',
      status: 'notApplicable',
    });
  });

  it('shows final inference queue for file transcription while retaining legacy queue rows', () => {
    const run = makeRun({ kind: 'fileTranscription', correlation: { kind: 'fileTranscription', fileRunId: 9 } });
    expect(orderedStages(run).map(timing => timing.stage)).toContain('inferenceQueue');
    run.stages = run.stages.filter(timing => timing.stage !== 'inferenceQueue');
    expect(orderedStages(run).map(timing => timing.stage)).toContain('modelQueue');
    expect(orderedStages(run).map(timing => timing.stage)).not.toContain('inferenceQueue');
  });

  it('summarizes release-to-paste p50/p90 only from measured successful dictations', () => {
    const withStage = (index: number, stage: 'releaseToPaste' | 'totalProcessing', value: number | null, overrides = {}) => {
      const run = makeRun({ runId: index.toString(16).padStart(32, '0'), ...overrides });
      run.stages = run.stages.map(timing => timing.stage === stage
        ? { ...timing, durationMs: value === null ? notApplicable() : measured(value) }
        : timing);
      return run;
    };
    const runs = [
      ...[120, 150, 170, 180, 200, 210, 230, 250, 300, 900].map((value, index) =>
        withStage(index + 1, 'releaseToPaste', value)),
      // A toggle-stopped dictation has no release; a failed run never pasted.
      withStage(20, 'releaseToPaste', null),
      withStage(21, 'releaseToPaste', 5, { outcome: { status: 'noSpeech' } }),
      withStage(22, 'releaseToPaste', 5, {
        kind: 'fileTranscription',
        correlation: { kind: 'fileTranscription', fileRunId: 3 },
      }),
    ];
    expect(dictationStagePercentiles(runs, 'releaseToPaste')).toEqual({
      p50Ms: 200,
      p90Ms: 300,
      sampleCount: 10,
    });
    expect(dictationStagePercentiles(runs, 'totalProcessing')).toEqual({
      p50Ms: 1_000,
      p90Ms: 1_000,
      sampleCount: 11,
    });
    expect(dictationStagePercentiles([makeRun()], 'releaseToPaste')).toBeNull();
    expect(stageLabel('releaseToStop', 'dictation')).toBe('Release to stop');
    expect(stageLabel('releaseToPaste', 'dictation')).toBe('Release to paste');
  });

  it('orders each run kind without inventing stage offsets', () => {
    expect(orderedStages(makeRun()).map(stage => stage.stage)).toEqual([
      'releaseToStop',
      'captureFinalization',
      'vad',
      'modelQueue',
      'modelLoad',
      'inferenceQueue',
      'inferenceDecode',
      'transcriptTransform',
      'cleanup',
      'voiceCommands',
      'smartCorrection',
      'smartFormatting',
      'spokenStructure',
      'spokenNumbers',
      'ideContext',
      'cliCommand',
      'fileOutput',
      'clipboardPaste',
      'totalProcessing',
      'releaseToPaste',
    ]);
    const transform = makeRun({
      kind: 'selectedTextTransform',
      correlation: { kind: 'selectedTextTransform', transformPassId: 42 },
    });
    expect(orderedStages(transform).map(stage => stage.stage)).toEqual([
      'selectedTextCapture',
      'instructionCapture',
      'instructionAsr',
      'sidecarSpawnLoad',
      'generation',
      'reviewReady',
    ]);
    const query = makeRun({
      kind: 'voiceQuery',
      correlation: { kind: 'voiceQuery', queryPassId: 43 },
    });
    expect(orderedStages(query).map(stage => stage.stage)).toEqual([
      'instructionCapture',
      'instructionAsr',
      'sidecarSpawnLoad',
      'generation',
      'totalProcessing',
    ]);
  });

  it('formats real-time factor and transform throughput only when measured', () => {
    expect(rateForRun(makeRun())).toMatchObject({
      label: 'Real-time factor',
      text: '0.50×',
      status: 'measured',
    });
    const transform = makeRun({
      kind: 'selectedTextTransform',
      correlation: { kind: 'selectedTextTransform', transformPassId: 42 },
      input: {
        audioDurationMs: notApplicable(),
        inputSizeBucket: measured('small'),
        outputSizeBucket: measured('small'),
        outputTokenCount: measured(25),
      },
      stages: makeRun().stages.map(stage => stage.stage === 'generation'
        ? { ...stage, durationMs: measured(500), outcome: 'completed' }
        : stage),
    });
    expect(rateForRun(transform)).toMatchObject({
      label: 'Throughput',
      text: '50.0 tok/s',
      status: 'measured',
    });
  });

  it('maps every run kind to its canonical structured Events filter', () => {
    expect(correlationFilterForRun(makeRun())).toEqual({
      field: 'recording_id',
      value: '17',
    });
    expect(correlationFilterForRun(makeRun({
      kind: 'fileTranscription',
      correlation: { kind: 'fileTranscription', fileRunId: 9 },
    }))).toEqual({
      field: 'file_run_id',
      value: '9',
    });
    expect(correlationFilterForRun(makeRun({
      kind: 'selectedTextTransform',
      correlation: { kind: 'selectedTextTransform', transformPassId: 42 },
    }))).toEqual({
      field: 'transform_pass_id',
      value: '42',
    });
    expect(correlationFilterForRun(makeRun({
      kind: 'voiceQuery',
      correlation: { kind: 'voiceQuery', queryPassId: 43 },
    }))).toEqual({
      field: 'query_pass_id',
      value: '43',
    });
  });

  it('formats resource deltas only from measured endpoints', () => {
    expect(resourceDelta(measured(100), measured(150))).toMatchObject({
      status: 'measured',
      text: '+0.0 MiB',
    });
    expect(resourceDelta(unavailable<number>(), measured(150))).toMatchObject({
      status: 'unavailable',
      text: 'Unavailable',
    });
  });
});
