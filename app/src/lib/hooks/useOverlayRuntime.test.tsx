import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';

type EventListener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, EventListener>(),
  unlistens: new Map<string, ReturnType<typeof vi.fn>>(),
  listenFailures: new Set<string>(),
  deferredListeners: new Set<string>(),
  deferredResolves: new Map<string, () => void>(),
  warn: vi.fn(),
  listen: vi.fn((event: string, handler: EventListener) => {
    if (mocks.listenFailures.has(event)) {
      return Promise.reject(new Error(`${event} unavailable`));
    }
    const unlisten = vi.fn();
    mocks.handlers.set(event, handler);
    mocks.unlistens.set(event, unlisten);
    if (mocks.deferredListeners.has(event)) {
      return new Promise<() => void>((resolve) => {
        mocks.deferredResolves.set(event, () => resolve(unlisten));
      });
    }
    return Promise.resolve(unlisten);
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: mocks.warn, error: vi.fn() },
}));
vi.mock('../settings', () => ({
  loadSettings: () => ({ hotkeyMissFeedback: false }),
}));

import {
  CLIPBOARD_ONLY_FLASH_MS,
  MICROPHONE_FAILURE_FLASH_MS,
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

describe('useOverlayRuntime transient cues', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    mocks.handlers.clear();
    mocks.unlistens.clear();
    mocks.listenFailures.clear();
    mocks.deferredListeners.clear();
    mocks.deferredResolves.clear();
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

  async function emitInitializationFailure(errorKind: unknown, recordingId = 1) {
    await act(async () => {
      mocks.handlers.get('recording-initialization-failed')?.({
        payload: { recordingId, errorKind },
      });
    });
  }

  it('shows a typed device-unavailable cue for a bounded five seconds', async () => {
    expect(current?.showMicrophoneFailure).toBeNull();

    await emitInitializationFailure('device_unavailable');
    expect(current?.showMicrophoneFailure).toBe('deviceUnavailable');

    await act(async () => vi.advanceTimersByTime(MICROPHONE_FAILURE_FLASH_MS - 1));
    expect(current?.showMicrophoneFailure).toBe('deviceUnavailable');

    await act(async () => vi.advanceTimersByTime(1));
    expect(current?.showMicrophoneFailure).toBeNull();
  });

  it('keeps unknown and interrupted failures generic and ignores malformed payloads', async () => {
    await emitInitializationFailure('permission_denied');
    expect(current?.showMicrophoneFailure).toBe('generic');

    await emitGeneration(2);
    await act(async () => {
      mocks.handlers.get('recording-initialization-failed')?.({ payload: null });
    });
    expect(current?.showMicrophoneFailure).toBeNull();

    await act(async () => {
      mocks.handlers.get('recording-interrupted')?.({ payload: { recordingId: 2 } });
    });
    expect(current?.showMicrophoneFailure).toBe('generic');
  });

  it('restarts the full failure timeout and adopts the latest typed cue', async () => {
    await emitInitializationFailure('device_unavailable');
    await act(async () => vi.advanceTimersByTime(MICROPHONE_FAILURE_FLASH_MS - 1000));

    await emitInitializationFailure('backend_error');
    expect(current?.showMicrophoneFailure).toBe('generic');

    await act(async () => vi.advanceTimersByTime(1000));
    expect(current?.showMicrophoneFailure).toBe('generic');

    await act(async () => vi.advanceTimersByTime(MICROPHONE_FAILURE_FLASH_MS - 1000));
    expect(current?.showMicrophoneFailure).toBeNull();
  });

  it('clears an older failure cue when a newer recording generation starts', async () => {
    await emitInitializationFailure('device_unavailable');
    expect(current?.showMicrophoneFailure).toBe('deviceUnavailable');

    await emitGeneration(2);
    expect(current?.showMicrophoneFailure).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('rejects delayed initialization and interruption failures from an older generation', async () => {
    await emitGeneration(3);
    await emitInitializationFailure('device_unavailable', 2);
    await act(async () => {
      mocks.handlers.get('recording-interrupted')?.({ payload: { recordingId: 2 } });
    });
    expect(current?.showMicrophoneFailure).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('uses a newer delivery as an ownership floor for delayed microphone failures', async () => {
    await emitDelivery(4);
    await emitInitializationFailure('device_unavailable', 3);
    expect(current?.showMicrophoneFailure).toBeNull();
    expect(current?.showClipboardOnly).toBe(true);
  });

  it('clears an older microphone failure when a newer delivery arrives first', async () => {
    await emitInitializationFailure('device_unavailable', 8);
    expect(current?.showMicrophoneFailure).toBe('deviceUnavailable');

    await emitDelivery(9);
    await emitGeneration(9);
    expect(current?.showMicrophoneFailure).toBeNull();
    expect(current?.showClipboardOnly).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
  });

  it('clears microphone failure even when the clipboard delivery listener is unavailable', async () => {
    await act(async () => root.unmount());
    mocks.handlers.clear();
    mocks.unlistens.clear();
    mocks.listenFailures.add('dictation-delivery-outcome');
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });

    await emitInitializationFailure('device_unavailable', 5);
    expect(current?.showMicrophoneFailure).toBe('deviceUnavailable');
    await emitGeneration(6);
    expect(current?.showMicrophoneFailure).toBeNull();
  });

  it('ignores a failure callback after cleanup while listener registration is pending', async () => {
    await act(async () => root.unmount());
    mocks.handlers.clear();
    mocks.unlistens.clear();
    mocks.deferredListeners.add('recording-initialization-failed');
    root = createRoot(container);
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
    const delayedHandler = mocks.handlers.get('recording-initialization-failed');

    await act(async () => root.unmount());
    await act(async () => {
      delayedHandler?.({ payload: { recordingId: 7, errorKind: 'device_unavailable' } });
    });
    expect(vi.getTimerCount()).toBe(0);
    await act(async () => {
      mocks.deferredResolves.get('recording-initialization-failed')?.();
      await Promise.resolve();
    });
    expect(mocks.unlistens.get('recording-initialization-failed')).toHaveBeenCalledOnce();
    root = createRoot(container);
  });

  it('clears the failure timer and both listeners on unmount', async () => {
    await emitInitializationFailure('device_unavailable');
    expect(vi.getTimerCount()).toBe(1);

    await act(async () => root.unmount());
    expect(mocks.unlistens.get('recording-initialization-failed')).toHaveBeenCalledOnce();
    expect(mocks.unlistens.get('recording-interrupted')).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    root = createRoot(container);
  });

  it('cleans up the surviving listener when one failure listener is unavailable', async () => {
    await act(async () => root.unmount());
    mocks.handlers.clear();
    mocks.unlistens.clear();
    mocks.warn.mockClear();
    mocks.listenFailures.add('recording-initialization-failed');
    root = createRoot(container);

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(current?.showMicrophoneFailure).toBeNull();
    expect(mocks.warn).toHaveBeenCalledWith(
      'overlay',
      'recording-initialization-failed listener unavailable',
      { error: 'Error: recording-initialization-failed unavailable' },
    );

    await act(async () => root.unmount());
    expect(mocks.unlistens.get('recording-interrupted')).toHaveBeenCalledOnce();
    root = createRoot(container);
  });

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

  it.each([
    'dictation-delivery-outcome',
    'dictation-generation-started',
  ])('fails closed when the %s listener is unavailable', async (eventName) => {
    await act(async () => root.unmount());
    mocks.handlers.clear();
    mocks.unlistens.clear();
    mocks.warn.mockClear();
    mocks.listenFailures.add(eventName);
    root = createRoot(container);

    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
    await emitDelivery(50);

    expect(current?.showClipboardOnly).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    expect(mocks.warn).toHaveBeenCalledWith(
      'overlay',
      `${eventName} listener unavailable`,
      { error: `Error: ${eventName} unavailable` },
    );
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
