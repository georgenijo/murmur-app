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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => { resolve = res; });
  return { promise, resolve };
}

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

  it('cancels exactly the started generation when the hold is released while start is pending', async () => {
    const start = deferred<{ type: string; state: string; recordingId: number }>();
    mocks.startRecording.mockReturnValueOnce(start.promise);
    mocks.cancelRecording.mockResolvedValueOnce(undefined);

    let starting!: Promise<void>;
    await act(async () => {
      starting = current.handleHoldStart();
    });
    expect(current.status).toBe('idle');

    // The key comes up ~10ms after the delayed hold start: the stop arrives
    // before the start IPC has claimed ownership.
    await act(async () => current.handleStop());
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
    expect(mocks.stopRecording).not.toHaveBeenCalled();

    await act(async () => {
      start.resolve({ type: 'recording_starting', state: 'starting', recordingId: 9 });
      await starting;
    });

    expect(mocks.cancelRecording).toHaveBeenCalledExactlyOnceWith(9);
    expect(mocks.stopRecording).not.toHaveBeenCalled();
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
    });
    expect(current.status).toBe('idle');
  });

  it('cancels a pending-start release even after readiness raced ahead of the response', async () => {
    const start = deferred<{ type: string; state: string; recordingId: number }>();
    mocks.startRecording.mockReturnValueOnce(start.promise);
    mocks.cancelRecording.mockResolvedValueOnce(undefined);

    let starting!: Promise<void>;
    await act(async () => {
      starting = current.handleHoldStart();
    });
    await act(async () => current.handleStop());
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'starting' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      start.resolve({ type: 'recording_starting', state: 'starting', recordingId: 4 });
      await starting;
    });

    expect(mocks.cancelRecording).toHaveBeenCalledExactlyOnceWith(4);
  });

  it('drops a pending-start release when the start is refused and does not leak it into the next start', async () => {
    const refused = deferred<{ type: string; state: string }>();
    mocks.startRecording.mockReturnValueOnce(refused.promise);

    let starting!: Promise<void>;
    await act(async () => {
      starting = current.handleHoldStart();
    });
    await act(async () => current.handleStop());
    await act(async () => {
      refused.resolve({ type: 'busy_transforming', state: 'idle' });
      await starting;
    });
    expect(mocks.cancelRecording).not.toHaveBeenCalled();

    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_starting',
      state: 'starting',
      recordingId: 10,
    });
    await act(async () => current.handleHoldStart());
    expect(current.status).toBe('starting');
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
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
    expect(mocks.updateStats).toHaveBeenCalledWith('one final transcript', 12, null);
    expect(current.transcription).toBe('one final transcript');
  });

  it('attributes stats to the recording-start Mode once, even for a repeated completion', async () => {
    const payload = {
      text: 'words completed in another Mode', duration: 8,
      recording: { recordingId: 81, modelId: 'parakeet-v3', modeId: 'frozen-mode', profileId: null, stages: [] },
    };
    await act(async () => {
      mocks.listeners.get('transcription-complete')?.({ payload });
      mocks.listeners.get('transcription-complete')?.({ payload });
    });
    expect(mocks.updateStats).toHaveBeenCalledOnce();
    expect(mocks.updateStats).toHaveBeenCalledWith(payload.text, 8, 'frozen-mode');
    expect(mocks.addEntry).toHaveBeenCalledOnce();
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

  it('keeps the smart destination scoped to the active recording generation', async () => {
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      mocks.listeners.get('dictation-generation-started')?.({
        payload: { recordingId: 10 },
      });
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 10, status: 'current', appName: 'T3 Code' },
      });
    });
    expect(current.smartDestinationStatus).toBe('current');
    expect(current.smartDestinationAppName).toBe('T3 Code');

    await act(async () => {
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 9, status: 'recent', appName: 'Old App' },
      });
    });
    expect(current.smartDestinationStatus).toBe('current');
    expect(current.smartDestinationAppName).toBe('T3 Code');

    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({
        payload: { recordingId: 11 },
      });
    });
    expect(current.smartDestinationStatus).toBeNull();
    expect(current.smartDestinationAppName).toBeNull();

    await act(async () => {
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 11, status: 'unavailable', appName: 'Ignored App' },
      });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'processing' });
    });
    expect(current.smartDestinationStatus).toBe('unavailable');
    expect(current.smartDestinationAppName).toBeNull();

    await act(async () => {
      mocks.listeners.get('recording-cancelled')?.({ payload: { recordingId: 11 } });
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 11, status: 'recent' },
      });
    });
    expect(current.smartDestinationStatus).toBeNull();
    expect(current.smartDestinationAppName).toBeNull();

    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({
        payload: { recordingId: 12 },
      });
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 12, status: 'recent' },
      });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      mocks.listeners.get('smart-destination-selected')?.({
        payload: { recordingId: 12, status: 'current' },
      });
    });
    expect(current.smartDestinationStatus).toBeNull();
    expect(current.smartDestinationAppName).toBeNull();
  });

  it('disposes the smart destination listeners on unmount', async () => {
    await act(async () => root.unmount());
    expect(mocks.listeners.has('smart-destination-selected')).toBe(false);
    expect(mocks.listeners.has('recording-cancelled')).toBe(false);
    root = createRoot(container);
  });

  it('clears the matching cleanup notice when recovery returns to idle', async () => {
    mocks.startRecording.mockResolvedValueOnce({
      type: 'audio_recovering',
      state: 'recovering',
    });

    await act(async () => current.handleStart());
    expect(current.error).toBe('Microphone cleanup is still in progress. Try again when Murmur is ready.');

    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
    });

    expect(current.status).toBe('idle');
    expect(current.error).toBe('');
  });

  it('dismisses the current notice while allowing a later distinct error', async () => {
    mocks.startRecording.mockResolvedValueOnce({
      type: 'busy_benchmarking',
      state: 'idle',
    });
    await act(async () => current.handleStart());
    expect(current.error).toBe('Wait for the benchmark to finish.');

    await act(async () => current.dismissError());
    expect(current.error).toBe('');

    await act(async () => {
      mocks.listeners.get('auto-paste-failed')?.({ payload: 'Murmur could not paste into this app.' });
    });
    expect(current.error).toBe('Murmur could not paste into this app.');
    expect(current.canRetryDelivery).toBe(false);
    await act(async () => {
      mocks.listeners.get('dictation-delivery-outcome')?.({
        payload: { recordingId: 1, outcome: 'clipboardOnly' },
      });
    });
    expect(current.canRetryDelivery).toBe(true);
  });

  it('does not offer Paste Last for an unconfirmed delivery failure', async () => {
    await act(async () => {
      mocks.listeners.get('auto-paste-failed')?.({ payload: 'Check the clipboard before pasting.' });
      mocks.listeners.get('dictation-delivery-outcome')?.({
        payload: { recordingId: 2, outcome: 'unconfirmed' },
      });
    });
    expect(current.error).toBe('Check the clipboard before pasting.');
    expect(current.canRetryDelivery).toBe(false);
  });

  it('does not let a stale start response resurrect an error after dismissal', async () => {
    const pending = deferred<{ type: string; state: string }>();
    mocks.startRecording.mockReturnValueOnce(pending.promise);

    let start!: Promise<void>;
    await act(async () => {
      start = current.handleStart();
      await Promise.resolve();
      current.dismissError();
    });
    await act(async () => {
      pending.resolve({ type: 'audio_recovering', state: 'recovering' });
      await start;
    });

    expect(current.error).toBe('');
  });

  it('keeps a newer distinct error when an older async response resolves', async () => {
    const pending = deferred<{ type: string; state: string }>();
    mocks.startRecording.mockReturnValueOnce(pending.promise);

    let start!: Promise<void>;
    await act(async () => {
      start = current.handleStart();
      await Promise.resolve();
      mocks.listeners.get('file-output-failed')?.({ payload: 'Saving the transcript failed.' });
    });
    expect(current.error).toBe('Saving the transcript failed.');

    await act(async () => {
      pending.resolve({ type: 'busy_benchmarking', state: 'idle' });
      await start;
    });
    expect(current.error).toBe('Saving the transcript failed.');
  });
});
