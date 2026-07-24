import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, Listener>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { useTransformFlow } from './useTransformFlow';

describe('useTransformFlow Escape recovery', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('resets a held pass on Escape, ignores its stale release, and starts the next pass', async () => {
    function Harness() {
      useTransformFlow({
        enabled: true,
        initialized: true,
        accessibilityGranted: true,
        transformHoldKey: 'alt_r',
        microphone: 'system_default',
      });
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.listeners.get('transform-key-pressed')).toBeDefined();
    expect(mocks.listeners.get('transform-key-released')).toBeDefined();
    expect(mocks.listeners.get('escape-cancel')).toBeDefined();

    await act(async () => {
      mocks.listeners.get('transform-key-pressed')?.({
        payload: { transformPassId: 7 },
      });
      mocks.listeners.get('escape-cancel')?.({ payload: null });
      mocks.listeners.get('transform-key-released')?.({
        payload: { transformPassId: 7 },
      });
      mocks.listeners.get('transform-key-pressed')?.({
        payload: { transformPassId: 8 },
      });
      await Promise.resolve();
    });

    const flowCalls = mocks.invoke.mock.calls.filter(([command]) => (
      command === 'start_transform_capture'
      || command === 'finish_transform_instruction'
      || command === 'cancel_transform'
    ));
    expect(flowCalls).toEqual([
      [
        'start_transform_capture',
        { deviceName: null, transformPassId: 7 },
      ],
      [
        'start_transform_capture',
        { deviceName: null, transformPassId: 8 },
      ],
    ]);
  });
});
