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
  return <span>{view.kind === 'resolved' ? view.status.state : view.kind}</span>;
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

  it('subtracts command latency and expires once without polling', async () => {
    mocks.invoke.mockImplementation(() => new Promise((resolve) => {
      setTimeout(() => resolve({
        state: 'ready',
        deviceId: 'usb',
        reason: 'current_verified',
        validForMs: 1_000,
      }), 400);
    }));

    await act(async () => root.render(<StatusProbe />));
    expect(container.textContent).toBe('loading');

    await act(async () => vi.advanceTimersByTimeAsync(400));
    expect(container.textContent).toBe('ready');

    await act(async () => vi.advanceTimersByTimeAsync(599));
    expect(container.textContent).toBe('ready');
    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(container.textContent).toBe('expired');
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it('installs invalidation listeners before requesting the initial snapshot', async () => {
    const installListeners: Array<(stop: () => void) => void> = [];
    mocks.listen.mockImplementation(() => new Promise((resolve) => installListeners.push(resolve)));
    mocks.invoke.mockResolvedValue({ state: 'blocked', message: 'Verification required.' });

    await act(async () => root.render(<StatusProbe />));
    expect(mocks.listen).toHaveBeenCalledTimes(3);
    expect(mocks.invoke).not.toHaveBeenCalled();

    await act(async () => {
      for (const install of installListeners) install(() => {});
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe('blocked');
  });
});
