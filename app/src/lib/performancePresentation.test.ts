import { describe, expect, it } from 'vitest';
import { makeRun, measured, notApplicable, unavailable } from '../components/log-viewer/testFixtures';
import {
  correlationFilterForRun,
  formatMeasurement,
  orderedStages,
  rateForRun,
  resourceDelta,
} from './performancePresentation';

describe('performance presentation semantics', () => {
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
      'inferenceDecode',
      'transcriptTransform',
      'cleanup',
      'voiceCommands',
      'smartCorrection',
      'smartFormatting',
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
