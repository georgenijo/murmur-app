import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type EventCallback = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, EventCallback>(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: mocks.invoke,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: EventCallback) => {
    mocks.listeners.set(name, callback);
    return () => {
      if (mocks.listeners.get(name) === callback) mocks.listeners.delete(name);
    };
  }),
}));

vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import {
  PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
  usePartialTranscript,
} from './usePartialTranscript';

type HookState = ReturnType<typeof usePartialTranscript>;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('usePartialTranscript readiness ordering', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current!: HookState;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function mount() {
    function Harness() {
      current = usePartialTranscript(true, 'small.en');
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  async function emit(name: string, payload: unknown) {
    const callback = mocks.listeners.get(name);
    expect(callback, `${name} listener should be registered`).toBeDefined();
    await act(async () => {
      callback?.({ payload });
      await Promise.resolve();
    });
  }

  it('recovers an active session when no newer lifecycle event arrives', async () => {
    mocks.invoke.mockResolvedValueOnce({ recordingId: 12, status: 'recording' });
    await mount();

    expect(current.status).toBe('recording');
    expect(current.activeRecordingId).toBe(12);

    await emit('partial-transcript', {
      contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
      recordingId: 12,
      text: 'visible after readiness recovery',
      chunkIndex: 1,
      processedAudioMs: 10_000,
    });
    expect(current.text).toBe('visible after readiness recovery');
  });

  it('does not let a late recording snapshot overwrite processing status', async () => {
    const snapshot = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(snapshot.promise);
    await mount();

    await emit('recording-status-changed', 'processing');
    await act(async () => {
      snapshot.resolve({ recordingId: 12, status: 'recording' });
      await snapshot.promise;
      await Promise.resolve();
    });

    expect(current.status).toBe('processing');
    expect(current.activeRecordingId).toBeNull();
  });

  it.each([
    ['partial-transcript-cleared', {
      contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
      recordingId: 12,
      reason: 'fallback',
    }],
    ['recording-cancelled', { recordingId: 12 }],
  ] as const)('does not reactivate after a late snapshot following %s', async (name, payload) => {
    const snapshot = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(snapshot.promise);
    await mount();

    await emit(name, payload);
    await act(async () => {
      snapshot.resolve({ recordingId: 12, status: 'recording' });
      await snapshot.promise;
      await Promise.resolve();
    });

    expect(current.status).toBe('idle');
    expect(current.activeRecordingId).toBeNull();
    expect(current.text).toBe('');
  });
});
