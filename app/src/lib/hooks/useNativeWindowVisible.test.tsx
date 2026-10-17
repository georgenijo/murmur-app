import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  isVisible: vi.fn(),
  isMinimized: vi.fn(),
  handlers: new Map<string, (event: { payload: boolean }) => void>(),
  stop: vi.fn(),
  registration: null as Promise<void> | null,
}));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: mocks.isTauri }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => {
    const subscribe = (name: string) => async (handler: (event: { payload: boolean }) => void) => {
      await mocks.registration;
      mocks.handlers.set(name, handler);
      return () => { mocks.stop(name); mocks.handlers.delete(name); };
    };
    return {
      isVisible: mocks.isVisible,
      isMinimized: mocks.isMinimized,
      onFocusChanged: subscribe('focus'),
      onResized: subscribe('resize'),
      listen: (name: string, handler: (event: { payload: boolean }) => void) => {
        expect(name).toBe('tauri://close-requested');
        return subscribe('close')(handler);
      },
    };
  },
}));

import { useNativeWindowVisible } from './useNativeWindowVisible';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

describe('native window activity', () => {
  let container: HTMLDivElement;
  let root: Root;
  const observed: boolean[] = [];
  function Probe() {
    const visible = useNativeWindowVisible();
    observed.push(visible);
    return <output>{String(visible)}</output>;
  }
  const emit = async (name: string, payload = false) => {
    await act(async () => { mocks.handlers.get(name)?.({ payload }); });
  };
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.isTauri.mockReturnValue(true);
    mocks.isVisible.mockReset().mockResolvedValue(false);
    mocks.isMinimized.mockReset().mockResolvedValue(false);
    mocks.handlers.clear();
    mocks.stop.mockClear();
    mocks.registration = null;
    observed.length = 0;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    container.remove();
  });

  it('never activates on hidden startup and does no recurring hidden-window polling', async () => {
    await act(async () => root.render(<Probe />));
    expect(observed).not.toContain(true);
    await act(async () => { vi.advanceTimersByTime(200); });
    mocks.isVisible.mockClear();
    mocks.isMinimized.mockClear();
    await act(async () => { vi.advanceTimersByTime(60_000); });
    expect(mocks.isVisible).not.toHaveBeenCalled();
    expect(mocks.isMinimized).not.toHaveBeenCalled();
  });

  it('preserves activity when unfocused, pauses on close-to-hide and resumes on focused reopen', async () => {
    mocks.isVisible.mockResolvedValue(true);
    await act(async () => root.render(<Probe />));
    expect(container.textContent).toBe('true');
    await emit('focus', false);
    expect(container.textContent).toBe('true');
    await emit('close');
    expect(container.textContent).toBe('false');
    await emit('resize');
    expect(container.textContent).toBe('false');
    await emit('focus', true);
    expect(container.textContent).toBe('true');
  });

  it('checks minimized state after the native transition settles and restores on focus', async () => {
    mocks.isVisible.mockResolvedValue(true);
    await act(async () => root.render(<Probe />));
    await emit('focus', false);
    expect(container.textContent).toBe('true');
    mocks.isMinimized.mockResolvedValue(true);
    await act(async () => { vi.advanceTimersByTime(150); });
    expect(container.textContent).toBe('false');
    mocks.isMinimized.mockResolvedValue(false);
    await emit('focus', true);
    expect(container.textContent).toBe('true');
  });

  it('uses document visibility notifications to recheck native state without a focus change', async () => {
    mocks.isVisible.mockResolvedValue(true);
    await act(async () => root.render(<Probe />));
    mocks.isMinimized.mockResolvedValue(true);
    await act(async () => { document.dispatchEvent(new Event('visibilitychange')); });
    expect(container.textContent).toBe('false');
    mocks.isMinimized.mockResolvedValue(false);
    await act(async () => { document.dispatchEvent(new Event('visibilitychange')); });
    expect(container.textContent).toBe('true');
  });

  it('rejects a stale visible hydration response after close', async () => {
    const pending = deferred<boolean>();
    mocks.isVisible.mockReturnValueOnce(pending.promise);
    await act(async () => root.render(<Probe />));
    await emit('close');
    await act(async () => pending.resolve(true));
    expect(observed).not.toContain(true);
  });

  it('does not let an older native read overwrite a newer minimize result', async () => {
    const pending = deferred<boolean>();
    mocks.isVisible.mockReturnValueOnce(pending.promise).mockResolvedValue(true);
    await act(async () => root.render(<Probe />));
    mocks.isMinimized.mockResolvedValue(true);
    await emit('resize');
    await act(async () => pending.resolve(true));
    expect(observed).not.toContain(true);
  });

  it('waits for native listener registration and disposes late registrations after unmount', async () => {
    const registration = deferred<void>();
    mocks.registration = registration.promise;
    mocks.isVisible.mockResolvedValue(true);
    await act(async () => root.render(<Probe />));
    expect(mocks.isVisible).not.toHaveBeenCalled();
    await act(async () => root.render(null));
    await act(async () => registration.resolve());
    expect(mocks.stop).toHaveBeenCalledTimes(3);
    expect(mocks.isVisible).not.toHaveBeenCalled();
  });
});
