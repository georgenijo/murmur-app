import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { VALID_STATUSES } from '../types';

type Listener = (event: { payload: unknown }) => void;
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn<(event: string, listener: Listener) => Promise<() => void>>(),
  unlisten: vi.fn(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../log', () => ({ flog: { info: vi.fn(), warn: vi.fn() } }));

import { useOverlayRecordingStatus } from './useOverlayRecordingStatus';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('overlay recording status after reload', () => {
  let container: HTMLDivElement;
  let root: Root;
  let listener: Listener;
  let current: ReturnType<typeof useOverlayRecordingStatus>;
  function Harness() {
    current = useOverlayRecordingStatus();
    return <span>{current.status}</span>;
  }
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.invoke.mockResolvedValue({ state: 'idle' });
    mocks.listen.mockImplementation(async (_event, handler) => {
      listener = handler;
      return mocks.unlisten;
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it.each(VALID_STATUSES)('restores %s without needing a new event', async (state) => {
    mocks.invoke.mockResolvedValue({ type: 'status', state });
    await act(async () => root.render(<Harness />));
    expect(container.textContent).toBe(state);
    expect(current.statusRef.current).toBe(state);
    expect(mocks.invoke).toHaveBeenCalledWith('get_status');
    expect(mocks.listen).toHaveBeenCalledWith('recording-status-changed', expect.any(Function));
  });

  it('waits for the subscription before reading native state', async () => {
    const subscription = deferred<() => void>();
    mocks.listen.mockReturnValue(subscription.promise);
    await act(async () => root.render(<Harness />));
    expect(mocks.invoke).not.toHaveBeenCalled();
    await act(async () => subscription.resolve(mocks.unlisten));
    expect(mocks.invoke).toHaveBeenCalledOnce();
  });

  it.each(['idle', 'processing'])('keeps a newer %s event when a recording snapshot arrives late', async (state) => {
    const snapshot = deferred<unknown>();
    mocks.invoke.mockReturnValue(snapshot.promise);
    await act(async () => root.render(<Harness />));
    await act(async () => {
      listener({ payload: state });
      expect(current.statusRef.current).toBe(state);
      snapshot.resolve({ state: 'recording' });
    });
    expect(container.textContent).toBe(state);
  });

  it('continues receiving transitions after restoring the recording', async () => {
    mocks.invoke.mockResolvedValue({ state: 'recording' });
    await act(async () => root.render(<Harness />));
    for (const state of ['processing', 'idle']) {
      await act(async () => listener({ payload: state }));
      expect(container.textContent).toBe(state);
    }
  });

  it('restores recording again when the webview remounts', async () => {
    mocks.invoke.mockResolvedValue({ state: 'recording' });
    await act(async () => root.render(<Harness />));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(container.textContent).toBe('recording');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });

  it('ignores malformed events without discarding a valid snapshot', async () => {
    const snapshot = deferred<unknown>();
    mocks.invoke.mockReturnValue(snapshot.promise);
    await act(async () => root.render(<Harness />));
    await act(async () => {
      listener({ payload: { state: 'idle' } });
      snapshot.resolve({ state: 'recording' });
    });
    expect(container.textContent).toBe('recording');
  });

  it('keeps listening when the snapshot fails', async () => {
    mocks.invoke.mockRejectedValue(new Error('IPC unavailable'));
    await act(async () => root.render(<Harness />));
    await act(async () => listener({ payload: 'recording' }));
    expect(container.textContent).toBe('recording');
  });

  it('cleans up a subscription that resolves after unmount', async () => {
    const subscription = deferred<() => void>();
    mocks.listen.mockReturnValue(subscription.promise);
    await act(async () => root.render(<Harness />));
    await act(async () => root.unmount());
    await act(async () => subscription.resolve(mocks.unlisten));
    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    root = createRoot(container);
  });

  it('ignores snapshots and queued events after unmount', async () => {
    const snapshot = deferred<unknown>();
    mocks.invoke.mockReturnValue(snapshot.promise);
    await act(async () => root.render(<Harness />));
    const statusRef = current.statusRef;
    await act(async () => root.unmount());
    await act(async () => {
      listener({ payload: 'recording' });
      snapshot.resolve({ state: 'processing' });
    });
    expect(statusRef.current).toBe('idle');
    expect(mocks.unlisten).toHaveBeenCalledOnce();
    root = createRoot(container);
  });
});
