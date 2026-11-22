import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    startRecording: vi.fn(),
    stopRecording: vi.fn(),
    cancelRecording: vi.fn(),
    syncFnCaptureInput: vi.fn(async () => undefined),
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
  syncFnCaptureInput: mocks.syncFnCaptureInput,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}));

vi.mock('../stats', () => ({
  updateStats: mocks.updateStats,
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

  it('mirrors the start request microphone for Fn key-down capture once per change', async () => {
    expect(mocks.syncFnCaptureInput).toHaveBeenCalledWith('system_default', null);
    mocks.syncFnCaptureInput.mockClear();
    const probeContainer = document.createElement('div');
    const probeRoot = createRoot(probeContainer);
    function Probe({ microphone }: { microphone: string }) {
      useRecordingState({ addEntry: mocks.addEntry, microphone });
      return null;
    }
    await act(async () => probeRoot.render(<Probe microphone="system_default" />));
    await act(async () => probeRoot.render(<Probe microphone="system_default" />));
    expect(mocks.syncFnCaptureInput).toHaveBeenCalledOnce();
    await act(async () => probeRoot.render(<Probe microphone="usb-mic" />));
    expect(mocks.syncFnCaptureInput).toHaveBeenCalledTimes(2);
    expect(mocks.syncFnCaptureInput).toHaveBeenLastCalledWith('usb-mic', null);
    await act(async () => probeRoot.unmount());
  });

  // The property harness (`dictation-core/src/property`, D-S2b-13 (c)) relies on this: a
  // stop in `Recovering` is not a step a user can take, so a capture nobody took is not
  // recoverable by one.
  it('issues no stop or cancel IPC while the status is recovering', async () => {
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
    });
    expect(current.status).toBe('recovering');

    await act(async () => current.handleStop());

    expect(mocks.stopRecording).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
    expect(current.status).toBe('recovering');
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

  it('does not let a late start response restore a generation already cancelled elsewhere', async () => {
    const start = deferred<{ type: string; state: string; recordingId: number }>();
    mocks.startRecording.mockReturnValueOnce(start.promise);

    let starting!: Promise<void>;
    await act(async () => {
      starting = current.handleHoldStart();
    });
    await act(async () => current.handleStop());
    // An external cancellation (Escape, overlay) ends generation 9 before
    // the start response arrives.
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 9 } });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'starting' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
    });
    await act(async () => {
      start.resolve({ type: 'recording_starting', state: 'starting', recordingId: 9 });
      await starting;
    });

    expect(current.status).toBe('idle');
    expect(mocks.cancelRecording).not.toHaveBeenCalled();

    // The next generation's response is still applied.
    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_starting',
      state: 'starting',
      recordingId: 10,
    });
    await act(async () => current.handleHoldStart());
    expect(current.status).toBe('starting');
  });

  it('does not reuse a rejected keyboard token on the next toolbar start', async () => {
    mocks.startRecording.mockResolvedValueOnce({ type: 'busy_transcribing_file', state: 'idle' });
    await act(async () => current.toggleRecording('keyboard-42'));
    expect(mocks.startRecording).toHaveBeenNthCalledWith(1, 'system_default', 'toggle', null, 'keyboard-42');
    expect(current.status).toBe('idle');
    mocks.startRecording.mockResolvedValueOnce({ type: 'recording_starting', state: 'starting', recordingId: 43 });
    await act(async () => current.handleStart());
    expect(mocks.startRecording).toHaveBeenNthCalledWith(2, 'system_default', 'toggle', null, undefined);
  });

  it('a toolbar start racing a queued keyboard callback never acquires its token', async () => {
    const toolbar = deferred<{ type: string; state: string; recordingId: number }>();
    mocks.startRecording.mockReturnValueOnce(toolbar.promise);
    let toolbarStart!: Promise<void>;
    let keyboardStart!: Promise<void>;
    await act(async () => {
      toolbarStart = current.handleStart();
      keyboardStart = current.toggleRecording('queued-keyboard-44');
    });
    expect(mocks.startRecording).toHaveBeenCalledExactlyOnceWith('system_default', 'toggle', null, undefined);
    await act(async () => {
      toolbar.resolve({ type: 'recording_starting', state: 'starting', recordingId: 44 });
      await Promise.all([toolbarStart, keyboardStart]);
    });
    expect(mocks.startRecording).toHaveBeenCalledOnce();
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

  describe('the agent command key', () => {
    const started = { type: 'recording_starting', state: 'starting', recordingId: 9 };

    afterEach(() => {
      mocks.cancelRecording.mockReset();
    });

    async function holdAgentKey() {
      mocks.startRecording.mockResolvedValueOnce(started);
      await act(async () => current.handleAgentCommandStart());
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      });
    }

    it('starts a recording for the agent, and its release sends it', async () => {
      mocks.stopRecording.mockResolvedValueOnce({ type: 'transcription', state: 'idle', text: '' });
      await holdAgentKey();
      expect(mocks.startRecording).toHaveBeenCalledExactlyOnceWith(
        'system_default', 'agent_command', null, undefined,
      );

      await act(async () => current.handleAgentCommandStop());
      expect(mocks.stopRecording).toHaveBeenCalledExactlyOnceWith('hold_release');
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
    });

    it('keeps the spoken command out of history and stats', async () => {
      mocks.stopRecording.mockImplementationOnce(async () => {
        mocks.listeners.get('transcription-complete')?.({
          payload: { recordingId: 9, purpose: 'agent_command', text: '', duration: 3 },
        });
        return { type: 'transcription', state: 'idle', text: '' };
      });
      await holdAgentKey();
      await act(async () => current.handleAgentCommandStop());

      expect(mocks.addEntry).not.toHaveBeenCalled();
      expect(mocks.updateStats).not.toHaveBeenCalled();
      expect(current.transcription).toBe('');
      expect(current.status).toBe('idle');
    });

    it('is not ended by the dictation key release', async () => {
      await holdAgentKey();
      await act(async () => current.handleHoldStop());
      expect(mocks.stopRecording).not.toHaveBeenCalled();
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
      expect(current.status).toBe('recording');
    });

    it('never starts over a recording, and its release leaves that recording alone', async () => {
      mocks.startRecording.mockResolvedValueOnce(started);
      await act(async () => current.handleStart());
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      });
      mocks.startRecording.mockClear();

      await act(async () => current.handleAgentCommandStart());
      await act(async () => current.handleAgentCommandStop());
      await act(async () => current.handleAgentCommandCancel());

      expect(mocks.startRecording).not.toHaveBeenCalled();
      expect(mocks.stopRecording).not.toHaveBeenCalled();
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
      expect(current.status).toBe('recording');
    });

    it('discards the recording when the hold turns out to be a shortcut', async () => {
      mocks.cancelRecording.mockResolvedValueOnce(undefined);
      await holdAgentKey();
      await act(async () => current.handleAgentCommandCancel());
      expect(mocks.cancelRecording).toHaveBeenCalledOnce();
      expect(mocks.stopRecording).not.toHaveBeenCalled();

      // The key comes up afterwards: nothing is left for it to send.
      await act(async () => current.handleAgentCommandStop());
      expect(mocks.stopRecording).not.toHaveBeenCalled();
    });

    it('cancels the generation a still-pending start creates', async () => {
      const start = deferred<typeof started>();
      mocks.startRecording.mockReturnValueOnce(start.promise);
      mocks.cancelRecording.mockResolvedValueOnce(undefined);
      let starting!: Promise<void>;
      await act(async () => {
        starting = current.handleAgentCommandStart();
      });
      await act(async () => current.handleAgentCommandCancel());
      expect(mocks.cancelRecording).not.toHaveBeenCalled();

      await act(async () => {
        start.resolve(started);
        await starting;
      });
      expect(mocks.cancelRecording).toHaveBeenCalledExactlyOnceWith(9);
      expect(mocks.stopRecording).not.toHaveBeenCalled();
    });

    it('gives the dictation key its release back once the agent recording has ended', async () => {
      await holdAgentKey();
      // Escape, or a failure, ended the recording without the key coming up.
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      });
      mocks.startRecording.mockResolvedValueOnce(started);
      mocks.stopRecording.mockResolvedValueOnce({ type: 'transcription', state: 'idle' });
      await act(async () => current.handleHoldStart());
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      });
      await act(async () => current.handleHoldStop());
      expect(mocks.stopRecording).toHaveBeenCalledOnce();
    });

    it('releases its ownership the moment the agent recording ends, before a dictation start', async () => {
      await holdAgentKey();
      // Escape ends the agent recording and the dictation key goes down before
      // React has rendered the idle status.
      const start = deferred<typeof started>();
      mocks.startRecording.mockReturnValueOnce(start.promise);
      mocks.stopRecording.mockResolvedValueOnce({ type: 'transcription', state: 'idle' });
      let starting!: Promise<void>;
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
        starting = current.handleHoldStart();
      });
      await act(async () => {
        start.resolve(started);
        await starting;
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      });
      expect(mocks.startRecording).toHaveBeenLastCalledWith('system_default', 'hold', null, undefined);

      await act(async () => current.handleHoldStop());
      expect(mocks.stopRecording).toHaveBeenCalledOnce();
    });

    it('keeps a newer agent hold when the previous recording reports idle late', async () => {
      mocks.stopRecording.mockResolvedValueOnce({ type: 'transcription', state: 'idle', text: '' });
      await holdAgentKey();
      // The stop response moves the status to idle before the idle event.
      await act(async () => current.handleAgentCommandStop());
      expect(current.status).toBe('idle');

      mocks.startRecording.mockResolvedValueOnce({ ...started, recordingId: 10 });
      const start = deferred<void>();
      await act(async () => {
        void current.handleAgentCommandStart().then(start.resolve);
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
        await start.promise;
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
      });

      await act(async () => current.handleHoldStop());
      expect(mocks.stopRecording).toHaveBeenCalledOnce();
      expect(current.status).toBe('recording');
    });

    it('retries a discard the native side refused', async () => {
      mocks.cancelRecording
        .mockRejectedValueOnce(new Error('capture cancel timed out'))
        .mockResolvedValueOnce(undefined);
      await holdAgentKey();
      await act(async () => current.handleAgentCommandCancel());

      expect(mocks.cancelRecording.mock.calls).toEqual([[9], [9]]);
      expect(mocks.stopRecording).not.toHaveBeenCalled();
      expect(current.error).toBe('');
    });

    it('never sends a command whose discard failed, and says it is still recording', async () => {
      mocks.cancelRecording.mockRejectedValue(new Error('capture cancel timed out'));
      await holdAgentKey();
      await act(async () => current.handleAgentCommandCancel());

      // Capture is still running: the hold stays responsible and says so.
      expect(mocks.cancelRecording).toHaveBeenCalledTimes(2);
      expect(current.status).toBe('recording');
      expect(current.error).toMatch(/still recording/);

      // No release or stop may turn the cancelled command into a sent one.
      await act(async () => current.handleHoldStop());
      expect(mocks.cancelRecording).toHaveBeenCalledTimes(2);
      await act(async () => current.handleAgentCommandStop());
      await act(async () => current.toggleRecording());
      await act(async () => current.handleStop());
      expect(mocks.stopRecording).not.toHaveBeenCalled();

      // Each of those retried the discard; one that works ends the ownership.
      mocks.cancelRecording.mockReset();
      mocks.cancelRecording.mockResolvedValueOnce(undefined);
      await act(async () => current.handleAgentCommandStop());
      expect(mocks.cancelRecording).toHaveBeenCalledExactlyOnceWith(9);
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      });
      await act(async () => current.handleAgentCommandStop());
      expect(mocks.stopRecording).not.toHaveBeenCalled();
      expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    });

    it.each(['starting', 'recording'] as const)(
      'waits for the recording id when the %s event arrives before the start response',
      async (lifecycle) => {
        const start = deferred<typeof started>();
        mocks.startRecording.mockReturnValueOnce(start.promise);
        mocks.cancelRecording.mockResolvedValue(undefined);
        let starting!: Promise<void>;
        await act(async () => {
          starting = current.handleAgentCommandStart();
        });
        // The lifecycle event overtakes the start response.
        await act(async () => {
          mocks.listeners.get('recording-status-changed')?.({ payload: lifecycle });
        });
        expect(current.status).toBe(lifecycle);

        // Neither the cancel nor the key coming up may cancel unscoped: that
        // call could land on a newer recording.
        await act(async () => current.handleAgentCommandCancel());
        await act(async () => current.handleAgentCommandStop());
        expect(mocks.cancelRecording).not.toHaveBeenCalled();

        await act(async () => {
          start.resolve(started);
          await starting;
        });
        expect(mocks.cancelRecording).toHaveBeenCalledExactlyOnceWith(9);
        expect(mocks.stopRecording).not.toHaveBeenCalled();
      },
    );

    it('cancels nothing when the cancelled recording ended before its start response', async () => {
      const start = deferred<typeof started>();
      mocks.startRecording.mockReturnValueOnce(start.promise);
      let starting!: Promise<void>;
      await act(async () => {
        starting = current.handleAgentCommandStart();
      });
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'starting' });
      });
      await act(async () => current.handleAgentCommandCancel());
      await act(async () => {
        mocks.listeners.get('recording-status-changed')?.({ payload: 'recovering' });
        mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      });
      await act(async () => {
        start.resolve(started);
        await starting;
      });
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
      expect(mocks.stopRecording).not.toHaveBeenCalled();
    });

    it('discards a command cancelled before its start was accepted, even when the first try fails', async () => {
      const start = deferred<typeof started>();
      mocks.startRecording.mockReturnValueOnce(start.promise);
      mocks.cancelRecording
        .mockRejectedValueOnce(new Error('capture cancel timed out'))
        .mockResolvedValueOnce(undefined);
      let starting!: Promise<void>;
      await act(async () => {
        starting = current.handleAgentCommandStart();
      });
      await act(async () => current.handleAgentCommandCancel());
      await act(async () => {
        start.resolve(started);
        await starting;
      });

      expect(mocks.cancelRecording.mock.calls).toEqual([[9], [9]]);
      expect(mocks.stopRecording).not.toHaveBeenCalled();
    });
  });

  it('passes an explicit toggle trigger only for a hotkey toggle stop', async () => {
    mocks.stopRecording.mockResolvedValueOnce({ type: 'transcription', state: 'idle' })
      .mockResolvedValueOnce({ type: 'transcription', state: 'idle' });
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
    });
    await act(async () => current.toggleRecording());
    expect(mocks.stopRecording).toHaveBeenLastCalledWith('toggle');
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'recording' });
    });
    await act(async () => current.handleStop());
    expect(mocks.stopRecording).toHaveBeenLastCalledWith('other');
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
          recording: {
            recordingId: 7,
            modelId: 'parakeet-v3',
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
      undefined,
      {
        rawText: 'one final transcript',
        recording: {
          recordingId: 7,
          modelId: 'parakeet-v3',
          profileId: 'Editor',
          stages: [],
        },
      },
    );
    expect(mocks.updateStats).toHaveBeenCalledTimes(1);
    expect(mocks.updateStats).toHaveBeenCalledWith('one final transcript', 12);
    expect(current.transcription).toBe('one final transcript');
  });

  it('counts stats once, even for a repeated completion', async () => {
    const payload = {
      text: 'words completed twice', duration: 8,
      recording: { recordingId: 81, modelId: 'parakeet-v3', profileId: null, stages: [] },
    };
    await act(async () => {
      mocks.listeners.get('transcription-complete')?.({ payload });
      mocks.listeners.get('transcription-complete')?.({ payload });
    });
    expect(mocks.updateStats).toHaveBeenCalledOnce();
    expect(mocks.updateStats).toHaveBeenCalledWith(payload.text, 8);
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

  it('keeps teardown guidance after final delivery and clears it independently of the result', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 20 } });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'processing' });
      mocks.listeners.get('transcription-complete')?.({ payload: { text: 'Synthetic final result.', duration: 1 } });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      mocks.listeners.get('recording-recovery-stalled')?.({ payload: {
        recordingId: 20, statusCode: 'microphone_cleanup_stalled', actionCode: 'restart_app',
      } });
    });
    expect(current.transcription).toBe('Synthetic final result.');
    expect(current.error).toContain('Restarting Murmur');
    await act(async () => {
      mocks.listeners.get('recording-status-changed')?.({ payload: 'idle' });
      mocks.listeners.get('recording-cleanup-finished')?.({ payload: { recordingId: 19 } });
    });
    expect(current.error).toContain('Restarting Murmur');
    await act(async () => {
      mocks.listeners.get('recording-cleanup-finished')?.({ payload: { recordingId: 20 } });
    });
    expect(current.error).toBe('');
    expect(current.transcription).toBe('Synthetic final result.');
    expect(current.status).toBe('idle');
  });

  it('cleanup completion preserves Processing and newer delivery errors', async () => {
    await act(async () => {
      mocks.listeners.get('dictation-generation-started')?.({ payload: { recordingId: 21 } });
      mocks.listeners.get('recording-status-changed')?.({ payload: 'processing' });
      mocks.listeners.get('recording-recovery-stalled')?.({ payload: {
        recordingId: 21, statusCode: 'microphone_cleanup_stalled', actionCode: 'restart_app',
      } });
      mocks.listeners.get('auto-paste-failed')?.({ payload: 'Synthetic paste failure.' });
      mocks.listeners.get('recording-cleanup-finished')?.({ payload: { recordingId: 21 } });
    });
    expect(current.status).toBe('processing');
    expect(current.error).toBe('Synthetic paste failure.');
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

  // A start refused during the retiring capture's cleanup leaves the app Idle and
  // publishes no status, so nothing takes the notice away through the status path.
  describe('the notice for a start refused during cleanup', () => {
    const refusedStart = async () => {
      mocks.startRecording.mockResolvedValueOnce({ type: 'audio_recovering', state: 'recovering' });
      await act(async () => current.handleStart());
      expect(current.status).toBe('idle');
      expect(current.error).toBe('Microphone cleanup is still in progress. Try again when Murmur is ready.');
    };

    it('clears when the recording\'s cleanup finishes, with no status change', async () => {
      await refusedStart();
      await act(async () => {
        mocks.listeners.get('recording-cleanup-finished')?.({ payload: { recordingId: 7 } });
      });
      expect(current.error).toBe('');
      expect(current.status).toBe('idle');
    });

    it('expires on its own when no signal follows the owner clearing', async () => {
      vi.useFakeTimers();
      await refusedStart();
      await act(async () => { vi.advanceTimersByTime(4_900); });
      expect(current.error).toContain('Microphone cleanup is still in progress');
      await act(async () => { vi.advanceTimersByTime(200); });
      expect(current.error).toBe('');
      expect(current.status).toBe('idle');
    });

    it('does not expire a newer distinct error', async () => {
      vi.useFakeTimers();
      await refusedStart();
      await act(async () => { vi.advanceTimersByTime(3_000); });
      await act(async () => {
        mocks.listeners.get('file-output-failed')?.({ payload: 'Saving the transcript failed.' });
      });
      // The refusal's expiry fires at 5 s and must leave the newer notice alone.
      await act(async () => { vi.advanceTimersByTime(3_000); });
      expect(current.error).toBe('Saving the transcript failed.');
    });
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

  describe('audio level while the window is hidden', () => {
    let visibility: PropertyDescriptor | undefined;
    const setVisibility = (state: 'visible' | 'hidden') => {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
    };
    beforeEach(() => { visibility = Object.getOwnPropertyDescriptor(document, 'visibilityState'); });
    afterEach(() => {
      if (visibility) Object.defineProperty(document, 'visibilityState', visibility);
      else delete (document as unknown as Record<string, unknown>).visibilityState;
    });

    it('updates the level while visible', async () => {
      setVisibility('visible');
      await act(async () => mocks.listeners.get('audio-level')?.({ payload: 0.04 }));
      expect(current.audioLevel).toBe(0.04);
    });

    it('does not re-render App-level state for audio-level events while hidden', async () => {
      setVisibility('visible');
      await act(async () => mocks.listeners.get('audio-level')?.({ payload: 0.04 }));
      let renders = 0;
      const countingContainer = document.createElement('div');
      const countingRoot = createRoot(countingContainer);
      function Counter() {
        useRecordingState({ addEntry: mocks.addEntry, microphone: 'system_default' });
        renders += 1;
        return null;
      }
      await act(async () => countingRoot.render(<Counter />));
      const baseline = renders;
      setVisibility('hidden');
      await act(async () => {
        for (let i = 0; i < 20; i += 1) mocks.listeners.get('audio-level')?.({ payload: 0.01 * i });
      });
      expect(renders).toBe(baseline);
      setVisibility('visible');
      await act(async () => mocks.listeners.get('audio-level')?.({ payload: 0.09 }));
      expect(renders).toBe(baseline + 1);
      await act(async () => countingRoot.unmount());
    });
  });
});
