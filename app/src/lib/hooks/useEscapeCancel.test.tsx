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

  async function pressEscape(payload: unknown = { queryPassId: null }) {
    await act(async () => {
      await mocks.listeners.get('escape-cancel')?.({ payload });
    });
  }

  it('routes a correlated Escape to only that exact query pass', async () => {
    mocks.invoke.mockResolvedValue(undefined);
    await renderHook('processing');

    await pressEscape({ queryPassId: 41 });

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', {
      queryPassId: 41,
    });
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it('routes a correlated Escape to only that exact query pass', async () => {
    mocks.invoke.mockResolvedValue(undefined);
    await renderHook('idle');

    await pressEscape({ queryPassId: 42 });

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', {
      queryPassId: 42,
    });
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it.each(['recording', 'processing'] as const)(
    'retains dictation cancellation for a null query target while dictation is %s',
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
        payload: { queryPassId: 51 },
      }));
      await Promise.resolve();
      await mocks.listeners.get('escape-cancel')?.({
        payload: { queryPassId: 51 },
      });
    });

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', {
      queryPassId: 51,
    });

    cancellation.resolve();
    await act(async () => first);
  });

  it('does not let an in-flight stale pass suppress cancellation of a newer pass', async () => {
    const firstCancellation = deferred<void>();
    mocks.invoke.mockImplementation(
      (_command: string, args?: { queryPassId?: number }) => (
        args?.queryPassId === 61 ? firstCancellation.promise : Promise.resolve()
      ),
    );
    await renderHook('idle');

    let first!: Promise<void>;
    await act(async () => {
      first = Promise.resolve(mocks.listeners.get('escape-cancel')?.({
        payload: { queryPassId: 61 },
      }));
      await Promise.resolve();
      await mocks.listeners.get('escape-cancel')?.({
        payload: { queryPassId: 62 },
      });
    });

    expect(mocks.invoke.mock.calls).toEqual([
      ['cancel_query', { queryPassId: 61 }],
      ['cancel_query', { queryPassId: 62 }],
    ]);

    firstCancellation.resolve();
    await act(async () => first);
  });

  it('bounds distinct in-flight cancellation targets and releases capacity on settle', async () => {
    const cancellations = Array.from({ length: 9 }, () => deferred<void>());
    mocks.invoke.mockImplementation(
      (_command: string, args?: { queryPassId?: number }) => (
        cancellations[(args?.queryPassId ?? 1) - 1].promise
      ),
    );
    await renderHook('idle');

    const pending: Promise<void>[] = [];
    await act(async () => {
      for (let queryPassId = 1; queryPassId <= 9; queryPassId += 1) {
        pending.push(Promise.resolve(mocks.listeners.get('escape-cancel')?.({
          payload: { queryPassId },
        })));
      }
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(8);

    cancellations[0].resolve();
    await act(async () => pending[0]);
    await act(async () => {
      pending.push(Promise.resolve(mocks.listeners.get('escape-cancel')?.({
        payload: { queryPassId: 9 },
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
    { queryPassId: undefined },
    { queryPassId: 0 },
    { queryPassId: -1 },
    { queryPassId: 1.5 },
    { queryPassId: Number.MAX_SAFE_INTEGER + 1 },
    { queryPassId: '41' },
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
    await listener?.({ payload: { queryPassId: 71 } });

    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });
});
