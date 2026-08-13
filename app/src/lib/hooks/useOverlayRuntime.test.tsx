import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

type EventListener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, EventListener>(),
  unlistens: new Map<string, ReturnType<typeof vi.fn>>(),
  listen: vi.fn((event: string, handler: EventListener) => {
    const unlisten = vi.fn();
    mocks.handlers.set(event, handler);
    mocks.unlistens.set(event, unlisten);
    return Promise.resolve(unlisten);
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));
vi.mock('../settings', () => ({
  loadSettings: () => ({ hotkeyMissFeedback: false }),
}));

import {
  CLIPBOARD_ONLY_FLASH_MS,
  type OverlayRuntime,
  useOverlayRuntime,
} from './useOverlayRuntime';

function Harness({ status = 'idle' }: { status?: DictationStatus }) {
  const statusRef = useRef<DictationStatus>(status);
  const hotkeyMissFeedbackRef = useRef(false);
  current = useOverlayRuntime({
    status,
    statusRef,
    disabled: false,
    setDisabled: vi.fn(),
    showHotkeyMiss: false,
    setShowHotkeyMiss: vi.fn(),
    hotkeyMissFeedbackRef,
  });
  return null;
}

let current: OverlayRuntime | null = null;

describe('useOverlayRuntime clipboard-only cue', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    mocks.handlers.clear();
    mocks.unlistens.clear();
    current = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  async function emitClipboardOnly() {
    await act(async () => {
      mocks.handlers.get('auto-paste-failed')?.({
        payload: 'App focus changed. Text is in your clipboard; paste it when ready.',
      });
    });
  }

  it('shows the cue for a bounded five seconds', async () => {
    expect(current?.showClipboardOnly).toBe(false);

    await emitClipboardOnly();
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1));
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(1));
    expect(current?.showClipboardOnly).toBe(false);
  });

  it('restarts the full timeout when another failure arrives', async () => {
    await emitClipboardOnly();
    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1000));

    await emitClipboardOnly();
    await act(async () => vi.advanceTimersByTime(1000));
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1000));
    expect(current?.showClipboardOnly).toBe(false);
  });

  it('unsubscribes and clears the pending timeout on unmount', async () => {
    await emitClipboardOnly();
    expect(vi.getTimerCount()).toBe(1);

    await act(async () => root.unmount());
    expect(mocks.unlistens.get('auto-paste-failed')).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    root = createRoot(container);
  });
});
