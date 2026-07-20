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

import { useOverlayGeometry } from './useOverlayGeometry';

const notched: OverlayGeometry = {
  windowW: 305,
  collapsedH: 32,
  expandedH: 76,
  pillIdleW: 213,
  pillActiveW: 305,
  pillMarginIdle: 46,
  pillMarginActive: 0,
  dropdownH: 44,
};

const fallback: OverlayGeometry = {
  windowW: 200,
  collapsedH: 37,
  expandedH: 81,
  pillIdleW: 108,
  pillActiveW: 200,
  pillMarginIdle: 46,
  pillMarginActive: 0,
  dropdownH: 44,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('useOverlayGeometry', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: OverlayGeometry | null = null;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.invoke.mockResolvedValue(undefined);
    mocks.listener = null;
    current = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
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
});
