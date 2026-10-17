import { describe, expect, it } from 'vitest';
import { makeRun, measured, notApplicable, unavailable } from '../components/log-viewer/testFixtures';
import {
  correlationFilterForRun,
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

  it('orders each run kind without inventing stage offsets', () => {
    expect(orderedStages(makeRun()).map(stage => stage.stage)).toEqual([
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
