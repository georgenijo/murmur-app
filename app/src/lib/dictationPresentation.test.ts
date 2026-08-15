import { describe, expect, it } from 'vitest';
import {
  cleanupStalledPresentationFromPayload,
  dictationPresentationFromPayload,
  initializationPresentationFromPayload,
  interruptedPresentationFromPayload,
} from './dictationPresentation';

describe('dictation presentation payload contract', () => {
  it('accepts every bounded status/action pair', () => {
    for (const [statusCode, actionCode] of [
      ['microphone_cleanup_in_progress', 'wait'],
      ['microphone_initialization_failed', 'retry'],
      ['microphone_initialization_failed', 'open_microphone_settings'],
      ['microphone_initialization_failed', 'choose_microphone'],
      ['microphone_cleanup_stalled', 'restart_app'],
      ['microphone_interrupted', 'retry'],
      ['microphone_interrupted', 'wait_for_partial_transcription'],
    ]) {
      expect(dictationPresentationFromPayload({ recordingId: 7, statusCode, actionCode }))
        .toEqual({ recordingId: 7, statusCode, actionCode });
    }
  });

  it('rejects unknown, contradictory, and unowned payloads', () => {
    for (const payload of [
      null,
      { recordingId: 0, statusCode: 'microphone_interrupted', actionCode: 'retry' },
      { recordingId: 7, statusCode: 'microphone_interrupted', actionCode: 'restart_app' },
      { recordingId: 7, statusCode: 'private', actionCode: 'retry' },
    ]) {
      expect(dictationPresentationFromPayload(payload)).toBeNull();
    }
  });

  it('ties initialization actions to the bounded failure kind', () => {
    expect(initializationPresentationFromPayload({
      recordingId: 1,
      errorKind: 'permission_denied',
      statusCode: 'microphone_initialization_failed',
      actionCode: 'open_microphone_settings',
    })?.actionCode).toBe('open_microphone_settings');
    expect(initializationPresentationFromPayload({
      recordingId: 2,
      errorKind: 'device_unavailable',
      statusCode: 'microphone_initialization_failed',
      actionCode: 'choose_microphone',
    })?.actionCode).toBe('choose_microphone');
    expect(initializationPresentationFromPayload({
      recordingId: 3,
      errorKind: 'backend_error',
      statusCode: 'microphone_initialization_failed',
      actionCode: 'retry',
    })?.actionCode).toBe('retry');
    expect(initializationPresentationFromPayload({
      recordingId: 4,
      errorKind: 'permission_denied',
      statusCode: 'microphone_initialization_failed',
      actionCode: 'retry',
    })).toBeNull();
  });

  it('ties interruption actions to actual partial-transcription behavior', () => {
    expect(interruptedPresentationFromPayload({
      recordingId: 5,
      autoTranscribe: true,
      statusCode: 'microphone_interrupted',
      actionCode: 'wait_for_partial_transcription',
    })?.actionCode).toBe('wait_for_partial_transcription');
    expect(interruptedPresentationFromPayload({
      recordingId: 5,
      autoTranscribe: false,
      statusCode: 'microphone_interrupted',
      actionCode: 'retry',
    })?.actionCode).toBe('retry');
    expect(interruptedPresentationFromPayload({
      recordingId: 5,
      autoTranscribe: true,
      statusCode: 'microphone_interrupted',
      actionCode: 'retry',
    })).toBeNull();
  });

  it('accepts only the restart action for stalled cleanup', () => {
    expect(cleanupStalledPresentationFromPayload({
      recordingId: 6,
      statusCode: 'microphone_cleanup_stalled',
      actionCode: 'restart_app',
    })?.recordingId).toBe(6);
  });
});
