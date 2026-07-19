import { describe, expect, it } from 'vitest';
import {
  PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
  createPartialTranscriptState,
  partialTranscriptReducer,
  type PartialTranscriptClearReason,
  type PartialTranscriptState,
} from './usePartialTranscript';

const session = (recordingId: number) => ({
  contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
  recordingId,
});

function withPartial(
  state: PartialTranscriptState,
  recordingId: number,
  text: string,
  chunkIndex: number,
): PartialTranscriptState {
  return partialTranscriptReducer(state, {
    type: 'partialReceived',
    payload: {
      ...session(recordingId),
      text,
      chunkIndex,
      processedAudioMs: chunkIndex * 8_000 + 2_000,
    },
  });
}

function started(recordingId = 7, enabled = true): PartialTranscriptState {
  return partialTranscriptReducer(createPartialTranscriptState(enabled, 'base.en'), {
    type: 'sessionStarted',
    payload: session(recordingId),
  });
}

describe('partialTranscriptReducer', () => {
  it('adopts cumulative updates in chunk order', () => {
    const first = withPartial(started(), 7, 'one reliable chunk', 1);
    const second = withPartial(first, 7, 'one reliable chunk followed by another', 2);

    expect(first.text).toBe('one reliable chunk');
    expect(second.text).toBe('one reliable chunk followed by another');
    expect(second.chunkIndex).toBe(2);
    expect(second.processedAudioMs).toBe(18_000);
  });

  it('rejects stale recording IDs and out-of-order chunks', () => {
    const current = withPartial(started(8), 8, 'current words', 2);
    const staleSession = withPartial(current, 7, 'stale words', 3);
    const staleChunk = withPartial(current, 8, 'older chunk', 1);

    expect(staleSession).toBe(current);
    expect(staleChunk).toBe(current);
  });

  it.each<PartialTranscriptClearReason>([
    'cancelled',
    'fallback',
    'finalized',
    'error',
  ])('clears the matching session for %s', (reason) => {
    const current = withPartial(started(), 7, 'provisional words', 1);
    const cleared = partialTranscriptReducer(current, {
      type: 'sessionCleared',
      payload: { ...session(7), reason },
    });

    expect(cleared.activeRecordingId).toBeNull();
    expect(cleared.text).toBe('');
  });

  it('ignores a stale clear after a newer recording starts', () => {
    const old = withPartial(started(7), 7, 'old words', 1);
    const newer = partialTranscriptReducer(old, {
      type: 'sessionStarted',
      payload: session(8),
    });
    const staleClear = partialTranscriptReducer(newer, {
      type: 'sessionCleared',
      payload: { ...session(7), reason: 'cancelled' },
    });

    expect(staleClear.activeRecordingId).toBe(8);
    expect(staleClear).toBe(newer);
  });

  it('clears on the generic final/cancel status fallback', () => {
    const current = withPartial(started(), 7, 'provisional words', 1);
    const cleared = partialTranscriptReducer(current, { type: 'clearActive' });

    expect(cleared.activeRecordingId).toBeNull();
    expect(cleared.text).toBe('');
  });

  it('clears and rejects updates while disabled', () => {
    const current = withPartial(started(), 7, 'sensitive words', 1);
    const disabled = partialTranscriptReducer(current, {
      type: 'settingsChanged',
      enabled: false,
      model: 'base.en',
    });
    const ignored = withPartial(disabled, 7, 'more sensitive words', 2);

    expect(disabled.text).toBe('');
    expect(disabled.activeRecordingId).toBe(7);
    expect(ignored).toBe(disabled);
  });

  it('clears and invalidates the session when the model changes', () => {
    const current = withPartial(started(), 7, 'model-bound words', 1);
    const changed = partialTranscriptReducer(current, {
      type: 'settingsChanged',
      enabled: true,
      model: 'small.en',
    });

    expect(changed.activeRecordingId).toBeNull();
    expect(changed.text).toBe('');
    expect(changed.model).toBe('small.en');
  });
});
