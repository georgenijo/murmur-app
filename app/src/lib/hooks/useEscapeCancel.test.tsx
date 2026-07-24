import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

type Listener = (event: { payload: unknown }) => void | Promise<void>;

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

  async function pressEscape(payload: unknown = { transformPassId: null }) {
    await act(async () => {
      await mocks.listeners.get('escape-cancel')?.({ payload });
    });
  }

  it('routes a correlated Escape to only that exact transform pass', async () => {
    mocks.invoke.mockResolvedValue(undefined);
    await renderHook('processing');

    await pressEscape({ transformPassId: 41 });

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_transform', {
      transformPassId: 41,
    });
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it.each(['recording', 'processing'] as const)(
    'retains dictation cancellation for a null transform target while dictation is %s',
    async (status) => {
      mocks.cancelRecording.mockResolvedValue(undefined);
      await renderHook(status);

      await pressEscape();

      expect(mocks.invoke).not.toHaveBeenCalled();
      expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    },
  );

  it('does nothing when both pipelines are idle', async () => {
    await renderHook('idle');

    await pressEscape();

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it('coalesces repeated Escape events for the same pass while cancellation is in flight', async () => {
    const cancellation = deferred<void>();
    mocks.invoke.mockReturnValue(cancellation.promise);
    await renderHook('idle');

    let first!: Promise<void>;
    await act(async () => {
      first = Promise.resolve(mocks.listeners.get('escape-cancel')?.({
        payload: { transformPassId: 51 },
      }));
      await Promise.resolve();
      await mocks.listeners.get('escape-cancel')?.({
        payload: { transformPassId: 51 },
      });
    });

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_transform', {
      transformPassId: 51,
    });

    cancellation.resolve();
    await act(async () => first);
  });

  it('does not let an in-flight stale pass suppress cancellation of a newer pass', async () => {
    const firstCancellation = deferred<void>();
    mocks.invoke.mockImplementation(
      (_command: string, args?: { transformPassId?: number }) => (
        args?.transformPassId === 61 ? firstCancellation.promise : Promise.resolve()
      ),
    );
    await renderHook('idle');

    let first!: Promise<void>;
    await act(async () => {
      first = Promise.resolve(mocks.listeners.get('escape-cancel')?.({
        payload: { transformPassId: 61 },
      }));
      await Promise.resolve();
      await mocks.listeners.get('escape-cancel')?.({
        payload: { transformPassId: 62 },
      });
    });

    expect(mocks.invoke.mock.calls).toEqual([
      ['cancel_transform', { transformPassId: 61 }],
      ['cancel_transform', { transformPassId: 62 }],
    ]);

    firstCancellation.resolve();
    await act(async () => first);
  });

  it('bounds distinct in-flight cancellation targets and releases capacity on settle', async () => {
    const cancellations = Array.from({ length: 9 }, () => deferred<void>());
    mocks.invoke.mockImplementation(
      (_command: string, args?: { transformPassId?: number }) => (
        cancellations[(args?.transformPassId ?? 1) - 1].promise
      ),
    );
    await renderHook('idle');

    const pending: Promise<void>[] = [];
    await act(async () => {
      for (let transformPassId = 1; transformPassId <= 9; transformPassId += 1) {
        pending.push(Promise.resolve(mocks.listeners.get('escape-cancel')?.({
          payload: { transformPassId },
        })));
      }
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(8);

    cancellations[0].resolve();
    await act(async () => pending[0]);
    await act(async () => {
      pending.push(Promise.resolve(mocks.listeners.get('escape-cancel')?.({
        payload: { transformPassId: 9 },
      })));
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(9);

    for (const cancellation of cancellations) cancellation.resolve();
    await act(async () => Promise.all(pending));
  });

  it.each([
    null,
    {},
    { transformPassId: undefined },
    { transformPassId: 0 },
    { transformPassId: -1 },
    { transformPassId: 1.5 },
    { transformPassId: Number.MAX_SAFE_INTEGER + 1 },
    { transformPassId: '41' },
  ])('fails closed for malformed payload %#', async (payload) => {
    await renderHook('recording');

    await pressEscape(payload);

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
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
    await listener?.({ payload: { transformPassId: 71 } });

    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });
});
