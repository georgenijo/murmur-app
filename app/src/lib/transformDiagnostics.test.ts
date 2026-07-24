import { describe, expect, it } from 'vitest';
import { isDiagnosticCaptureV1, isTransformAttemptV1 } from './transformDiagnostics';

const phase = {
  phase: 'modelLoad',
  outcome: 'completed',
  durationMs: 42,
  errorCode: null,
};

describe('transform diagnostics validators', () => {
  it('accepts a content-free attempt record', () => {
    expect(isTransformAttemptV1({
      schemaVersion: 1,
      transformPassId: 9,
      startedAtMs: 100,
      finishedAtMs: 200,
      outcome: 'ready',
      selectionSource: 'accessibility',
      selectionResult: 'success',
      selectionSizeBucket: '17-64',
      rangeAvailable: true,
      boundsAvailable: true,
      instructionConfidenceBucket: 'unavailable',
      modelWarmState: 'cold',
      outputTokenCount: 12,
      finishReason: 'stop',
      processExitCode: null,
      processExitSignal: null,
      phases: [phase],
    })).toBe(true);
  });

  it('rejects malformed attempts and accepts explicit local capture content', () => {
    expect(isTransformAttemptV1({ schemaVersion: 1, transformPassId: '9' })).toBe(false);
    expect(isDiagnosticCaptureV1({
      schemaVersion: 1,
      captureId: '9-100',
      transformPassId: 9,
      capturedAtMs: 100,
      expiresAtMs: 200,
      outcome: 'ready',
      selection: 'private selected text',
      instruction: 'private instruction',
      output: 'private output',
      phases: [phase],
    })).toBe(true);
  });
});
