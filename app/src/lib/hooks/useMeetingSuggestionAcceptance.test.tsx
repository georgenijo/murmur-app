import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listener: null as ((event: { payload: unknown }) => void) | null,
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name: string, listener: (event: { payload: unknown }) => void) => {
    mocks.listener = listener;
    return () => { mocks.listener = null; };
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { useMeetingSuggestionAcceptance } from './useMeetingSuggestionAcceptance';

function deferred<T>() {
  let resolve = (_value: T) => {};
  let reject = (_reason?: unknown) => {};
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe('useMeetingSuggestionAcceptance', () => {
  let container: HTMLDivElement;
  let root: Root;
  let start: ReturnType<typeof vi.fn<(token: string) => Promise<void>>>;
  let onError: ReturnType<typeof vi.fn<(message: string) => void>>;

  beforeEach(() => {
    mocks.listener = null;
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    start = vi.fn(async () => {});
    onError = vi.fn();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('starts once with the opaque token and ignores duplicate accepts in flight', async () => {
    const pending = deferred<void>();
    start.mockReturnValue(pending.promise);
    function Harness() {
      useMeetingSuggestionAcceptance(start, onError);
      return null;
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });

    await act(async () => {
      mocks.listener?.({ payload: { token: '33333333-3333-4333-8333-333333333333', title: 'must be ignored' } });
      mocks.listener?.({ payload: { token: '33333333-3333-4333-8333-333333333333', title: 'must be ignored' } });
    });
    expect(start).toHaveBeenCalledTimes(1);
    expect(start).toHaveBeenCalledWith('33333333-3333-4333-8333-333333333333');
    await act(async () => pending.resolve(undefined));
    expect(onError).not.toHaveBeenCalled();
  });

  it('shows the main window with a stable recovery message when accepted start fails', async () => {
    start.mockRejectedValue(new Error('private meeting title and backend detail'));
    function Harness() {
      useMeetingSuggestionAcceptance(start, onError);
      return null;
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });

    await act(async () => {
      mocks.listener?.({ payload: { token: '44444444-4444-4444-8444-444444444444' } });
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(onError).toHaveBeenCalledWith(
      'Notetaker could not start from that Calendar suggestion. Start it manually to continue.',
    );
    expect(JSON.stringify(onError.mock.calls)).not.toContain('private meeting title');
    expect(mocks.invoke).toHaveBeenCalledWith('show_main_window');
  });

  it('does not surface a late accepted-start failure after unmount', async () => {
    const pending = deferred<void>();
    start.mockReturnValue(pending.promise);
    function Harness() {
      useMeetingSuggestionAcceptance(start, onError);
      return null;
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
    await act(async () => mocks.listener?.({
      payload: { token: '99999999-9999-4999-8999-999999999999' },
    }));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      pending.reject(new Error('late private error'));
      await Promise.resolve();
    });

    expect(onError).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
