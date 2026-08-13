import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => undefined),
  listeners: new Map<string, Listener>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));

import { useQueryFlow } from './useQueryFlow';

describe('useQueryFlow', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockClear();
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function renderFlow() {
    function Harness() {
      useQueryFlow({
        enabled: true,
        initialized: true,
        accessibilityGranted: true,
        queryHotkey: 'alt_r',
        microphone: 'system_default',
        command: {
          executable: '/usr/bin/printf',
          arguments: ['%s'],
          timeoutSeconds: 60,
          presetId: 'claude',
        },
      });
      return null;
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('arms the dedicated listener and carries one exact pass through start and stop', async () => {
    await renderFlow();
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_listener', { hotkey: 'alt_r' });

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 17, action: 'start' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('start_query_capture', {
      queryPassId: 17,
      deviceName: null,
      command: {
        executable: '/usr/bin/printf',
        arguments: ['%s'],
        timeoutSeconds: 60,
        // The preset rides along with the command so the backend knows whose
        // auth signatures and login to use when the run fails (#550).
        presetId: 'claude',
      },
    });

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 17, action: 'stop' } });
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('finish_query_capture', { queryPassId: 17 });
  });

  it('ignores malformed and stale stop events', async () => {
    await renderFlow();
    mocks.invoke.mockClear();

    await act(async () => {
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 0, action: 'start' } });
      mocks.listeners.get('query-toggle')?.({ payload: { queryPassId: 9, action: 'stop' } });
      await Promise.resolve();
    });

    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
