import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

type Listener = (event: { payload: unknown }) => void | Promise<void>;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  cancelRecording: vi.fn(),
  listeners: new Map<string, Listener>(),
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return mocks.unlisten;
  }),
}));
vi.mock('../dictation', () => ({ cancelRecording: mocks.cancelRecording }));

import { useEscapeCancel } from './useEscapeCancel';

describe('useEscapeCancel', () => {
  let container: HTMLDivElement;
  let root: Root;
  let rootMounted: boolean;
  let consoleError: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.cancelRecording.mockReset();
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    rootMounted = true;
  });

  afterEach(async () => {
    if (rootMounted) await act(async () => root.unmount());
    container.remove();
    consoleError.mockRestore();
  });

  async function renderHook(status: DictationStatus = 'idle', enabled = true) {
    function Harness() {
      useEscapeCancel({ status, enabled });
      return null;
    }

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
  }

  async function pressEscape(payload: unknown = null) {
    await act(async () => {
      await mocks.listeners.get('escape-cancel')?.({ payload });
    });
  }

  it.each(['recording', 'processing'] as const)(
    'retains dictation cancellation while dictation is %s',
    async (status) => {
      mocks.cancelRecording.mockResolvedValue(undefined);
      await renderHook(status);

      await pressEscape();

      expect(mocks.invoke).not.toHaveBeenCalled();
      expect(mocks.cancelRecording).toHaveBeenCalledOnce();
    },
  );

  it('does nothing when dictation is idle', async () => {
    await renderHook('idle');

    await pressEscape();

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });

  it('does not register a listener while disabled', async () => {
    await renderHook('recording', false);
    expect(mocks.listeners.has('escape-cancel')).toBe(false);
  });

  it('ignores a queued Escape event after unmount', async () => {
    mocks.invoke.mockResolvedValue('thinking');
    await renderHook('idle');
    const listener = mocks.listeners.get('escape-cancel');

    await act(async () => root.unmount());
    rootMounted = false;
    await listener?.({ payload: null });

    expect(mocks.unlisten).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.cancelRecording).not.toHaveBeenCalled();
  });
});
