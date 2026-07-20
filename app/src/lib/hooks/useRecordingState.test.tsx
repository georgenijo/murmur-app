import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    startRecording: vi.fn(),
    stopRecording: vi.fn(),
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
  const promise = new Promise<T>((done) => { resolve = done; });
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
    container.remove();
  });

  it('waits for in-flight recorder startup before invoking stop', async () => {
    const startup = deferred<{ type: string; state: string }>();
    mocks.startRecording.mockReturnValueOnce(startup.promise);
    mocks.stopRecording.mockResolvedValueOnce({
      type: 'transcription',
      state: 'idle',
      text: '',
    });

    let startPromise!: Promise<void>;
    await act(async () => {
      startPromise = current.handleStart();
      await Promise.resolve();
    });

    let stopPromise!: Promise<void>;
    await act(async () => {
      stopPromise = current.handleStop();
      await Promise.resolve();
    });

    expect(mocks.startRecording).toHaveBeenCalledOnce();
    expect(mocks.stopRecording).not.toHaveBeenCalled();

    startup.resolve({ type: 'recording_started', state: 'recording' });
    await act(async () => {
      await Promise.all([startPromise, stopPromise]);
    });

    expect(mocks.stopRecording).toHaveBeenCalledOnce();
    expect(current.status).toBe('idle');
  });

  it('records history and stats exactly once from the final completion event', async () => {
    mocks.startRecording.mockResolvedValueOnce({
      type: 'recording_started',
      state: 'recording',
    });
    mocks.stopRecording.mockImplementationOnce(async () => {
      mocks.listeners.get('transcription-complete')?.({
        payload: {
          text: 'one final transcript',
          duration: 12,
          teachingContext: { appBundleId: 'com.example.Editor', appLabel: 'Editor' },
        },
      });
      return {
        type: 'transcription',
        state: 'idle',
        text: 'one final transcript',
      };
    });

    await act(async () => current.handleStart());
    await act(async () => current.handleStop());

    expect(mocks.addEntry).toHaveBeenCalledTimes(1);
    expect(mocks.addEntry).toHaveBeenCalledWith(
      'one final transcript',
      12,
      'recording',
      undefined,
      { appBundleId: 'com.example.Editor', appLabel: 'Editor' },
    );
    expect(mocks.updateStats).toHaveBeenCalledTimes(1);
    expect(mocks.updateStats).toHaveBeenCalledWith('one final transcript', 12);
    expect(current.transcription).toBe('one final transcript');
  });
});
