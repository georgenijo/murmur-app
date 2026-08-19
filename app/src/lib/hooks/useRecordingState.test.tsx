import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    startRecording: vi.fn(),
    stopRecording: vi.fn(),
    cancelRecording: vi.fn(),
    listeners,
    listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    }),
    addEntry: vi.fn(),
    updateStats: vi.fn(),
  };
});

vi.mock('../dictation', () => ({
  startRecording: mocks.startRecording,
  stopRecording: mocks.stopRecording,
  cancelRecording: mocks.cancelRecording,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}));

vi.mock('../stats', () => ({
  updateStats: mocks.updateStats,
}));

vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { useRecordingState } from './useRecordingState';

type RecordingState = ReturnType<typeof useRecordingState>;

describe('useRecordingState transition ordering', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: RecordingState;

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      current = useRecordingState({
        addEntry: mocks.addEntry,
        microphone: 'system_default',
      });
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    container.remove();
  });

  it('cancels promptly while microphone initialization is still starting', async () => {
    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_starting',
      state: 'starting',
    });
    mocks.cancelRecording.mockResolvedValueOnce(undefined);

    await act(async () => current.handleStart());
    expect(current.status).toBe('starting');
    await act(async () => current.handleStop());

    expect(mocks.startRecording).toHaveBeenCalledOnce();
    expect(mocks.stopRecording).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
    });
    expect(current.status).toBe('idle');
  });

  it('surfaces a reason for every busy refusal instead of failing silently', async () => {
    const refusals = [
      'busy_benchmarking',
      'busy_transcribing_file',
      'busy_meeting',
      'busy_querying',
      'busy_transforming',
      'busy_recording_corpus',
    ];

    for (const type of refusals) {
      mocks.startRecording.mockResolvedValueOnce({ type, state: 'idle' });

      await act(async () => current.handleStart());

      expect(current.status).toBe('idle');
      expect(current.error, `${type} must explain itself`).not.toBe('');
    }
  });

  it('does not let a start response overwrite an earlier readiness event', async () => {
    mocks.startRecording.mockImplementationOnce(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'starting' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      return {
        type: 'recording_starting',
        state: 'starting',
      };
    });

    await act(async () => current.handleStart());

    expect(current.status).toBe('recording');
  });

  it('does not let a transform-owned supervisor response overwrite dictation idle', async () => {
    for (const type of ['audio_recovering', 'already_starting']) {
      mocks.startRecording.mockImplementationOnce(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'starting' });
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
        return {
          type,
          state: type === 'audio_recovering' ? 'recovering' : 'starting',
        };
      });

      await act(async () => current.handleStart());
      expect(current.status).toBe('idle');
    }
  });

  it('starts duration at readiness rather than start acceptance', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-28T16:00:00Z'));
    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_starting',
      state: 'starting',
    });

    await act(async () => current.handleStart());
    await act(async () => {
      vi.advanceTimersByTime(10_000);
    });
    expect(current.recordingDuration).toBe(0);

    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
    });
    await act(async () => {
      vi.advanceTimersByTime(2_100);
    });
    expect(current.recordingDuration).toBe(2);
  });

  it('records history and stats exactly once from the final completion event', async () => {
    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_starting',
      state: 'starting',
    });
    mocks.stopRecording.mockImplementationOnce(async () => {
      mocks.listeners.get('transcription-complete')?.({
        payload: {
          text: 'one final transcript',
          rawText: 'one final transcript',
          duration: 12,
          teachingContext: { appBundleId: 'com.example.Editor', appLabel: 'Editor' },
          recording: {
            recordingId: 7,
            modelId: 'parakeet-v3',
            modeId: null,
            profileId: 'Editor',
            stages: [],
          },
        },
      });
      return {
        type: 'transcription',
        state: 'idle',
        text: 'one final transcript',
      };
    });

    await act(async () => current.handleStart());
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
    });
    await act(async () => current.handleStop());

    expect(mocks.addEntry).toHaveBeenCalledTimes(1);
    expect(mocks.addEntry).toHaveBeenCalledWith(
      'one final transcript',
      12,
      'recording',
      undefined,
      { appBundleId: 'com.example.Editor', appLabel: 'Editor' },
      undefined,
      {
        rawText: 'one final transcript',
        recording: {
          recordingId: 7,
          modelId: 'parakeet-v3',
          modeId: null,
          profileId: 'Editor',
          stages: [],
        },
      },
    );
    expect(mocks.updateStats).toHaveBeenCalledTimes(1);
    expect(mocks.updateStats).toHaveBeenCalledWith('one final transcript', 12);
    expect(current.transcription).toBe('one final transcript');
  });

  it('clears an older failure for a new generation and ignores delayed stale presentations', async () => {
    const initializationFailure = (recordingId: number) => ({
      recordingId,
      error: 'Microphone capture failed.',
      errorKind: 'backend_error',
      statusCode: 'microphone_initialization_failed',
      actionCode: 'retry',
    });

    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({
        payload: { recordingId: 2 },
      });
      mocks.listeners.get('recording-initialization-failed')?.({
        payload: initializationFailure(2),
      });
    });
    expect(current.error).toBe('Microphone capture failed.');

    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({
        payload: { recordingId: 3 },
      });
    });
    expect(current.error).toBe('');

    await act(async () => {
      mocks.listeners.get('recording-initialization-failed')?.({
        payload: initializationFailure(2),
      });
      mocks.listeners.get('recording-recovery-stalled')?.({
        payload: {
          recordingId: 2,
          statusCode: 'microphone_cleanup_stalled',
          actionCode: 'restart_app',
        },
      });
      mocks.listeners.get('recording-interrupted')?.({
        payload: {
          recordingId: 2,
          autoTranscribe: false,
          statusCode: 'microphone_interrupted',
          actionCode: 'retry',
        },
      });
    });
    expect(current.error).toBe('');
  });
});
