import { describe, expect, it } from 'vitest';
import {
  PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
  classifyPartialTranscriptEvent,
  createPartialTranscriptState,
  isActiveRecordingSessionSnapshot,
  isRecordingIdPayload,
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
    expect(second.acceptedEventCount).toBe(2);
    expect(second.rejectedEventCount).toBe(0);
  });

  it('rejects stale recording IDs and out-of-order chunks', () => {
    const current = withPartial(started(8), 8, 'current words', 2);
    const staleSession = withPartial(current, 7, 'stale words', 3);
    const staleChunk = withPartial(current, 8, 'older chunk', 1);

    expect(staleSession.text).toBe(current.text);
    expect(staleSession.lastEventDecision).toBe('recording_id_mismatch');
    expect(staleSession.rejectedEventCount).toBe(1);
    expect(staleChunk.text).toBe(current.text);
    expect(staleChunk.lastEventDecision).toBe('out_of_order');
    expect(staleChunk.rejectedEventCount).toBe(1);
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

  it('ignores duplicate or stale session-start events', () => {
    const current = withPartial(started(8), 8, 'current words', 1);
    const duplicate = partialTranscriptReducer(current, {
      type: 'sessionStarted',
      payload: session(8),
    });
    const stale = partialTranscriptReducer(current, {
      type: 'sessionStarted',
      payload: session(7),
    });

    expect(duplicate).toBe(current);
    expect(stale).toBe(current);
    expect(stale.text).toBe('current words');
  });

  it('remembers the latest generation after clear and rejects a late start', () => {
    const current = withPartial(started(8), 8, 'current words', 1);
    const cleared = partialTranscriptReducer(current, {
      type: 'sessionCleared',
      payload: { ...session(8), reason: 'finalized' },
    });
    const lateStart = partialTranscriptReducer(cleared, {
      type: 'sessionStarted',
      payload: session(7),
    });

    expect(cleared.activeRecordingId).toBeNull();
    expect(cleared.latestRecordingId).toBe(8);
    expect(lateStart).toBe(cleared);
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
    expect(ignored.text).toBe('');
    expect(ignored.lastEventDecision).toBe('disabled');
    expect(ignored.rejectedEventCount).toBe(1);
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

  it('distinguishes an update received before any active session', () => {
    const state = createPartialTranscriptState(true, 'base.en');
    const payload = {
      ...session(7),
      text: 'not adopted',
      chunkIndex: 1,
      processedAudioMs: 10_000,
    };

    expect(classifyPartialTranscriptEvent(state, payload)).toBe('no_active_session');
    const rejected = partialTranscriptReducer(state, { type: 'partialReceived', payload });
    expect(rejected.text).toBe('');
    expect(rejected.rejectedEventCount).toBe(1);
  });
});

describe('active session readiness snapshot', () => {
  it('accepts only active recording or processing snapshots', () => {
    expect(isActiveRecordingSessionSnapshot({ recordingId: 12, status: 'recording' })).toBe(true);
    expect(isActiveRecordingSessionSnapshot({ recordingId: 12, status: 'processing' })).toBe(true);
    expect(isActiveRecordingSessionSnapshot({ recordingId: 12, status: 'idle' })).toBe(false);
    expect(isActiveRecordingSessionSnapshot({ recordingId: 0, status: 'recording' })).toBe(false);
  });

  it('validates session-scoped fallback lifecycle payloads', () => {
    expect(isRecordingIdPayload({ recordingId: 9 })).toBe(true);
    expect(isRecordingIdPayload({ recordingId: 0 })).toBe(false);
    expect(isRecordingIdPayload(undefined)).toBe(false);
  });
});
