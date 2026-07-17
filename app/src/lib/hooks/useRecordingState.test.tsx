import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  startRecording: vi.fn(),
  stopRecording: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));

vi.mock('../dictation', () => ({
  startRecording: mocks.startRecording,
  stopRecording: mocks.stopRecording,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}));

vi.mock('../stats', () => ({
  updateStats: vi.fn(),
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
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      current = useRecordingState({
        addEntry: vi.fn(),
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
});
