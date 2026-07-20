import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OverlayGeometry } from '../overlayGeometry';

type GeometryListener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listener: null as GeometryListener | null,
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_event: string, listener: GeometryListener) => {
    mocks.listener = listener;
    return mocks.unlisten;
  }),
}));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { useOverlayGeometry } from './useOverlayGeometry';

const notched: OverlayGeometry = {
  windowW: 257,
  collapsedH: 32,
  expandedH: 76,
  pillIdleW: 257,
  pillActiveW: 257,
  pillMarginIdle: 0,
  pillMarginActive: 0,
  dropdownH: 44,
  wingW: 36,
};

const fallback: OverlayGeometry = {
  windowW: 152,
  collapsedH: 37,
  expandedH: 81,
  pillIdleW: 152,
  pillActiveW: 152,
  pillMarginIdle: 0,
  pillMarginActive: 0,
  dropdownH: 44,
  wingW: 36,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe('useOverlayGeometry', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: OverlayGeometry | null = null;

  function geometryCallCount() {
    return mocks.invoke.mock.calls.filter((call) => call[0] === 'get_overlay_geometry').length;
  }

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    mocks.listener = null;
    mocks.unlisten.mockReset();
    current = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('does not let the initial fetch overwrite a newer display-change event', async () => {
    const initialFetch = deferred<OverlayGeometry>();
    mocks.invoke.mockReturnValueOnce(initialFetch.promise);

    function Harness() {
      current = useOverlayGeometry();
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.listener).not.toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith('get_overlay_geometry');

    await act(async () => {
      mocks.listener?.({ payload: fallback });
    });
    expect(current!).toEqual(fallback);

    await act(async () => {
      initialFetch.resolve(notched);
      await initialFetch.promise;
    });
    expect(current!).toEqual(fallback);
  });

  it('accepts the initial Rust geometry and unregisters on unmount', async () => {
    mocks.invoke.mockResolvedValueOnce(notched);

    function Harness() {
      current = useOverlayGeometry();
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(current!).toEqual(notched);

    await act(async () => root.unmount());
    expect(mocks.unlisten).toHaveBeenCalledOnce();
    root = createRoot(container);
  });

  it('retries a transient initial fetch failure after the configured backoff', async () => {
    const firstFetch = deferred<OverlayGeometry>();
    mocks.invoke
      .mockReturnValueOnce(firstFetch.promise)
      .mockResolvedValueOnce(notched);

    function Harness() {
      current = useOverlayGeometry();
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(geometryCallCount()).toBe(1);
    firstFetch.reject(new Error('backend not ready'));
    await Promise.resolve();
    await Promise.resolve();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 300));
    });
    expect(geometryCallCount()).toBe(2);
    expect(current!).toEqual(notched);
  });

  it('cancels a pending retry when a newer display geometry event arrives', async () => {
    const firstFetch = deferred<OverlayGeometry>();
    mocks.invoke.mockReturnValueOnce(firstFetch.promise);

    function Harness() {
      current = useOverlayGeometry();
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    firstFetch.reject(new Error('backend not ready'));
    await Promise.resolve();
    await Promise.resolve();
    expect(geometryCallCount()).toBe(1);
    await act(async () => { mocks.listener?.({ payload: fallback }); });
    expect(current!).toEqual(fallback);

    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 300)); });
    expect(geometryCallCount()).toBe(1);
    expect(current!).toEqual(fallback);
  });
});
