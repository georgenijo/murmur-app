import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useSmartAutoMicrophoneStatus } from './useSmartAutoMicrophoneStatus';

const smartAuto = {
  approvedDeviceIds: ['usb'],
  preferredDeviceIds: ['usb'],
  allowContinuity: false,
};

function StatusProbe() {
  const { view } = useSmartAutoMicrophoneStatus(smartAuto);
  return <span>{view.kind === 'resolved'
    ? view.status.state === 'ready'
      ? `ready:${view.status.deviceId}`
      : `blocked:${view.status.retryAfterMs ?? 'none'}`
    : view.kind}</span>;
}

describe('useSmartAutoMicrophoneStatus', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    mocks.invoke.mockReset();
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
});
