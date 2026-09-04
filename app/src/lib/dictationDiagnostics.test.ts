import { describe, expect, it } from 'vitest';
import {
  isDictationCaptureArmStatusV1,
  isDictationCaptureSummaryV1,
  isDictationCaptureV1,
} from './dictationDiagnostics';

describe('private dictation capture boundary validation', () => {
  it('accepts only the three explicit arm states', () => {
    expect(isDictationCaptureArmStatusV1({ state: 'unarmed' })).toBe(true);
    expect(isDictationCaptureArmStatusV1({ state: 'armed', expiresAtMs: 42 })).toBe(true);
    expect(isDictationCaptureArmStatusV1({ state: 'capturing', recordingId: 7 })).toBe(true);
    expect(isDictationCaptureArmStatusV1({ state: 'armed', transcript: 'private' })).toBe(false);
  });

  it('rejects malformed list and content records', () => {
    expect(isDictationCaptureSummaryV1({
      captureId: 'capture',
      recordingId: 7,
      capturedAtMs: 10,
      expiresAtMs: 20,
      outcome: 'success',
      hasContent: true,
    })).toBe(true);
    expect(isDictationCaptureV1({
      schemaVersion: 1,
      captureId: 'capture',
      recordingId: 7,
      capturedAtMs: 10,
      expiresAtMs: 20,
      result: {
        kind: 'success',
        rawText: { text: 'private', truncated: false },
        finalText: { text: 'reviewed', truncated: false },
        modelId: 'test-model',
        totalMs: 42,
      },
    })).toBe(true);
    expect(isDictationCaptureV1({
      schemaVersion: 1,
      captureId: 'capture',
      result: { kind: 'success', rawText: 'private' },
    })).toBe(false);
  });
});
