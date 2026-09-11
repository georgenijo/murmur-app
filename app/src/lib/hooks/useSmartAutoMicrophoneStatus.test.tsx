import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: new Map<string, () => void>(),
  listen: vi.fn(async (event: string, handler: () => void) => {
    mocks.handlers.set(event, handler);
    return () => { mocks.handlers.delete(event); };
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useSmartAutoMicrophoneStatus } from './useSmartAutoMicrophoneStatus';

const smartAuto = {
  approvedDeviceIds: ['usb'],
  preferredDeviceIds: ['usb'],
  allowContinuity: false,
  requireRecentSignal: true,
};

function StatusProbe() {
  const { view } = useSmartAutoMicrophoneStatus(smartAuto);
  if (view.kind !== 'resolved') return <span>{view.kind}</span>;
  switch (view.status.state) {
    case 'ready': return <span>{`ready:${view.status.deviceId}`}</span>;
    case 'blocked': return <span>{`blocked:${view.status.retryAfterMs ?? 'none'}`}</span>;
    case 'probing': return <span>{`probing:${view.status.deviceId}:${view.status.phase}`}</span>;
  }
}

describe('useSmartAutoMicrophoneStatus', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    mocks.invoke.mockReset();
    mocks.handlers.clear();
    mocks.listen.mockClear();
    mocks.listen.mockImplementation(async () => () => {});
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    container.remove();
  });

  it('keeps availability ready without a signal-expiry retry loop', async () => {
    mocks.invoke.mockResolvedValue({ state: 'ready', deviceId: 'usb', reason: 'preferred_approved', validForMs: null });
    await act(async () => { root.render(<StatusProbe />); });
    expect(container.textContent).toBe('ready:usb');
    await act(async () => { await vi.advanceTimersByTimeAsync(180_000); });
    expect(container.textContent).toBe('ready:usb');
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it('subtracts command latency and rereads once to discover the next fresh candidate', async () => {
    mocks.invoke.mockImplementationOnce(() => new Promise((resolve) => {
      setTimeout(() => resolve({
        state: 'ready',
        deviceId: 'a',
        reason: 'current_verified',
        validForMs: 1_000,
      }), 400);
    })).mockResolvedValueOnce({
      state: 'ready',
      deviceId: 'b',
      reason: 'preferred_approved',
      validForMs: 5_000,
    });

    await act(async () => root.render(<StatusProbe />));
    expect(container.textContent).toBe('loading');

    await act(async () => vi.advanceTimersByTimeAsync(400));
    expect(container.textContent).toBe('ready:a');

    await act(async () => vi.advanceTimersByTimeAsync(599));
    expect(container.textContent).toBe('ready:a');
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(container.textContent).toBe('ready:b');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('rereads once when a switch cooldown ends', async () => {
    mocks.invoke
      .mockResolvedValueOnce({ state: 'blocked', message: 'Switch cooldown.', retryAfterMs: 1_000 })
      .mockResolvedValueOnce({ state: 'ready', deviceId: 'usb', reason: 'current_verified', validForMs: 5_000 });

    await act(async () => root.render(<StatusProbe />));
    expect(container.textContent).toBe('blocked:1000');

    await act(async () => vi.advanceTimersByTimeAsync(999));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(container.textContent).toBe('ready:usb');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it('does not schedule retries for an untimed blocked status', async () => {
    mocks.invoke.mockResolvedValue({ state: 'blocked', message: 'Verification required.', retryAfterMs: null });

    await act(async () => root.render(<StatusProbe />));
    expect(container.textContent).toBe('blocked:none');
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it('allows one immediate reread when command latency consumes the cooldown', async () => {
    mocks.invoke.mockImplementationOnce(() => new Promise((resolve) => {
      setTimeout(() => resolve({
        state: 'blocked', message: 'Switch cooldown.', retryAfterMs: 1_000,
      }), 1_200);
    })).mockImplementationOnce(() => new Promise((resolve) => {
      setTimeout(() => resolve({
        state: 'blocked', message: 'Switch cooldown.', retryAfterMs: 500,
      }), 600);
    })).mockResolvedValueOnce({
      state: 'ready', deviceId: 'usb', reason: 'current_verified', validForMs: 5_000,
    });

    await act(async () => root.render(<StatusProbe />));
    await act(async () => vi.advanceTimersByTimeAsync(1_200));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(600));
    expect(container.textContent).toBe('blocked:500');
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(499));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(container.textContent).toBe('ready:usb');
    expect(mocks.invoke).toHaveBeenCalledTimes(3);
  });

  it('installs invalidation listeners before requesting the initial snapshot', async () => {
    const installListeners: Array<(stop: () => void) => void> = [];
    mocks.listen.mockImplementation(() => new Promise((resolve) => installListeners.push(resolve)));
    mocks.invoke.mockResolvedValue({ state: 'blocked', message: 'Verification required.', retryAfterMs: null });

    await act(async () => root.render(<StatusProbe />));
    expect(mocks.listen).toHaveBeenCalledTimes(3);
    expect(mocks.invoke).not.toHaveBeenCalled();

    await act(async () => {
      for (const install of installListeners) install(() => {});
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe('blocked:none');
  });

  it('drops an older probing snapshot after a newer status refresh wins', async () => {
    mocks.listen.mockImplementation(async (event: string, handler: () => void) => {
      mocks.handlers.set(event, handler);
      return () => { mocks.handlers.delete(event); };
    });
    mocks.invoke.mockResolvedValueOnce({ state: 'blocked', message: 'Waiting.', retryAfterMs: null });
    await act(async () => root.render(<StatusProbe />));

    let finishOld: ((value: unknown) => void) | null = null;
    mocks.invoke.mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }));
    await act(async () => mocks.handlers.get('smart-auto-microphone-changed')?.());
    mocks.invoke.mockResolvedValueOnce({
      state: 'ready', deviceId: 'new', reason: 'preferred_approved', validForMs: 5_000,
    });
    await act(async () => {
      mocks.handlers.get('smart-auto-microphone-changed')?.();
      await Promise.resolve();
    });
    expect(container.textContent).toBe('ready:new');

    await act(async () => finishOld?.({
      state: 'probing', deviceId: 'old', phase: 'verifying', remainingMs: 4_000,
    }));
    expect(container.textContent).toBe('ready:new');
  });

  it('keeps teardown visible after the audio budget expires without polling', async () => {
    mocks.invoke.mockResolvedValue({
      state: 'probing', deviceId: 'usb', phase: 'stopping', remainingMs: 0,
    });
    await act(async () => root.render(<StatusProbe />));
    expect(container.textContent).toBe('probing:usb:stopping');
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe('probing:usb:stopping');
  });
});
