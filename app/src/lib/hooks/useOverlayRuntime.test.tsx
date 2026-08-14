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

  async function emitDelivery(recordingId: number, outcome = 'clipboardOnly') {
    await act(async () => {
      mocks.handlers.get('dictation-delivery-outcome')?.({
        payload: { recordingId, outcome },
      });
    });
  }

  async function emitGeneration(recordingId: number) {
    await act(async () => {
      mocks.handlers.get('dictation-generation-started')?.({
        payload: { recordingId },
      });
    });
  }

  it('shows the cue for a bounded five seconds', async () => {
    expect(current?.showClipboardOnly).toBe(false);

    await emitDelivery(1);
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1));
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(1));
    expect(current?.showClipboardOnly).toBe(false);
  });

  it('restarts the full timeout for a newer clipboard-only delivery', async () => {
    await emitDelivery(1);
    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1000));

    await emitDelivery(2);
    await act(async () => vi.advanceTimersByTime(1000));
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1000));
    expect(current?.showClipboardOnly).toBe(false);
  });

  it('does not extend the cue for a duplicate delivery event', async () => {
    await emitDelivery(4);
    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS - 1000));

    await emitDelivery(4);
    await act(async () => vi.advanceTimersByTime(1000));
    expect(current?.showClipboardOnly).toBe(false);
  });

  it('ignores stale and malformed delivery events', async () => {
    await emitDelivery(8);
    await act(async () => vi.advanceTimersByTime(CLIPBOARD_ONLY_FLASH_MS));

    await emitDelivery(7);
    expect(current?.showClipboardOnly).toBe(false);

    await act(async () => {
      mocks.handlers.get('dictation-delivery-outcome')?.({ payload: { recordingId: 9 } });
    });
    expect(current?.showClipboardOnly).toBe(false);
  });

  it.each(['autoPastePosted', 'clipboardWriteFailed', 'unconfirmed'])(
    'does not claim clipboard readiness for %s',
    async (outcome) => {
      await emitDelivery(10, outcome);
      expect(current?.showClipboardOnly).toBe(false);
      expect(vi.getTimerCount()).toBe(0);
    },
  );

  it('clears an older cue when a newer automatic paste completes', async () => {
    await emitDelivery(11);
    expect(current?.showClipboardOnly).toBe(true);

    await emitDelivery(12, 'autoPastePosted');
    expect(current?.showClipboardOnly).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('clears the cue when a newer recording lifecycle starts', async () => {
    await emitDelivery(15);
    expect(current?.showClipboardOnly).toBe(true);

    await act(async () => root.render(<Harness status="starting" />));
    expect(current?.showClipboardOnly).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('rejects an older outcome that arrives after a newer generation starts', async () => {
    await emitGeneration(31);
    await emitDelivery(30);
    expect(current?.showClipboardOnly).toBe(false);
    expect(vi.getTimerCount()).toBe(0);

    await emitDelivery(31);
    expect(current?.showClipboardOnly).toBe(true);
  });

  it('keeps a newer delivery cue when an older generation event arrives late', async () => {
    await emitDelivery(41);
    await emitGeneration(40);
    expect(current?.showClipboardOnly).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it('keeps a delivery cue when its generation event arrives late', async () => {
    await emitDelivery(42);
    await emitGeneration(42);
    expect(current?.showClipboardOnly).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it('unsubscribes and clears the pending timeout on unmount', async () => {
    await emitDelivery(20);
    expect(vi.getTimerCount()).toBe(1);

    await act(async () => root.unmount());
    expect(mocks.unlistens.get('dictation-delivery-outcome')).toHaveBeenCalledOnce();
    expect(mocks.unlistens.get('dictation-generation-started')).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    root = createRoot(container);
  });
});
