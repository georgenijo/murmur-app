import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

type Listener = () => void | Promise<void>;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  cancelRecording: vi.fn(),
  listeners: new Map<string, Listener>(),
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return mocks.unlisten;
  }),
}));
vi.mock('../dictation', () => ({ cancelRecording: mocks.cancelRecording }));

import { useEscapeCancel } from './useEscapeCancel';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('useEscapeCancel', () => {
  let container: HTMLDivElement;
  let root: Root;
  let rootMounted: boolean;
  let consoleError: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.cancelRecording.mockReset();
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    rootMounted = true;
  });

  afterEach(async () => {
    if (rootMounted) await act(async () => root.unmount());
    container.remove();
    consoleError.mockRestore();
  });

  async function renderHook(status: DictationStatus = 'idle', enabled = true) {
    function Harness() {
      useEscapeCancel({ status, enabled });
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
  }

  async function pressEscape() {
    await act(async () => {
      await mocks.listeners.get('escape-cancel')?.();
    });
  }

  it.each(['capturing', 'listening', 'thinking']) (
    'routes Escape during %s to transform cancellation only',
    async (transformStatus) => {
      mocks.invoke.mockImplementation(async (command: string) => {
        if (command === 'transform_status') return transformStatus;
        return undefined;
      });
      await renderHook('idle');

      await pressEscape();

      expect(mocks.invoke).toHaveBeenNthCalledWith(1, 'transform_status');
      expect(mocks.invoke).toHaveBeenNthCalledWith(2, 'cancel_transform');
      expect(mocks.invoke).toHaveBeenCalledTimes(2);
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
    },
  );

  it.each(['review_pending', 'applying'])(
    'leaves %s out of the global Escape cancellation path',
    async (transformStatus) => {
      mocks.invoke.mockResolvedValue(transformStatus);
      await renderHook('processing');

      await pressEscape();

      expect(mocks.invoke).toHaveBeenCalledOnce();
      expect(mocks.invoke).toHaveBeenCalledWith('transform_status');
      expect(mocks.cancelRecording).not.toHaveBeenCalled();
    },
  );

  it.each(['recording', 'processing'] as const)(
    'retains dictation cancellation while dictation is %s and transform is idle',
    async (status) => {
      mocks.invoke.mockResolvedValue('idle');
      mocks.cancelRecording.mockResolvedValue(undefined);
      await renderHook(status);

      await pressEscape();

      expect(mocks.invoke).toHaveBeenCalledOnce();
      expect(mocks.invoke).toHaveBeenCalledWith('transform_status');
      expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    },
  );

  it('does nothing when both pipelines are idle', async () => {
    mocks.invoke.mockResolvedValue('idle');
    await renderHook('idle');

    await pressEscape();

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it('coalesces repeated Escape events while cancellation is in flight', async () => {
    const cancellation = deferred<void>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'transform_status') return Promise.resolve('thinking');
      if (command === 'cancel_transform') return cancellation.promise;
      return Promise.resolve(undefined);
    });
    await renderHook('idle');

    let first!: Promise<void>;
    await act(async () => {
      first = Promise.resolve(mocks.listeners.get('escape-cancel')?.());
      await Promise.resolve();
      await mocks.listeners.get('escape-cancel')?.();
    });

    expect(mocks.invoke.mock.calls.filter(([command]) => command === 'cancel_transform'))
      .toHaveLength(1);

    cancellation.resolve();
    await act(async () => first);
  });

  it('falls back to dictation cancellation when transform status cannot be read', async () => {
    mocks.invoke.mockRejectedValueOnce(new Error('unavailable'));
    mocks.cancelRecording.mockResolvedValue(undefined);
    await renderHook('recording');

    await pressEscape();

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    expect(consoleError).toHaveBeenCalledWith(
      'transform_status failed during Escape cancellation:',
      expect.any(Error),
    );
  });

  it('does not register a listener while disabled', async () => {
    await renderHook('recording', false);
    expect(mocks.listeners.has('escape-cancel')).toBe(false);
  });

  it('ignores a queued Escape event after unmount', async () => {
    mocks.invoke.mockResolvedValue('thinking');
    await renderHook('idle');
    const listener = mocks.listeners.get('escape-cancel');

    await act(async () => root.unmount());
    rootMounted = false;
    await listener?.();

    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });
});
