import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  unlistens: new Map<string, ReturnType<typeof vi.fn>>(),
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    const unlisten = vi.fn();
    mocks.handlers.set(event, handler);
    mocks.unlistens.set(event, unlisten);
    return unlisten;
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import {
  useDeliveryRecoveryListeners,
  useDeliveryRecoveryNotice,
  type DeliveryRecoveryNotice,
  type PresentDeliveryRecoveryNotice,
} from './useDeliveryRecoveryListeners';

describe('useDeliveryRecoveryListeners', () => {
  let container: HTMLDivElement;
  let root: Root;
  let notice: DeliveryRecoveryNotice | null;
  let presentNotice: PresentDeliveryRecoveryNotice;

  beforeEach(async () => {
    vi.useFakeTimers();
    mocks.handlers.clear();
    mocks.unlistens.clear();
    notice = null;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      const owner = useDeliveryRecoveryNotice();
      notice = owner.notice;
      presentNotice = owner.presentNotice;
      useDeliveryRecoveryListeners(owner.presentNotice);
      return null;
    }

    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    container.remove();
  });

  it.each([
    ['auto_pasted', false],
    ['clipboard_only', true],
    ['empty', false],
    ['busy', true],
    ['failed', true],
  ])('maps %s feedback to the matching main-banner action', async (kind, retryable) => {
    await act(async () => {
      mocks.handlers.get('delivery-retry-feedback')?.({
        payload: { kind, message: `Result: ${kind}` },
      });
    });
    expect(notice).toEqual({ message: `Result: ${kind}`, retryable });

    await act(async () => vi.advanceTimersByTime(5000));
    expect(notice).toBeNull();
  });

  it('clears its timer and both listeners on unmount', async () => {
    await act(async () => {
      mocks.handlers.get('delivery-retry-feedback')?.({
        payload: { kind: 'failed', message: 'Try again.' },
      });
    });
    expect(vi.getTimerCount()).toBe(1);

    await act(async () => root.unmount());
    expect(vi.getTimerCount()).toBe(0);
    expect(mocks.unlistens.get('delivery-retry-feedback')).toHaveBeenCalledOnce();
    expect(mocks.unlistens.get('correction-start-failed')).toHaveBeenCalledOnce();
    root = createRoot(container);
  });

  it('does not let an older feedback timer clear a newer pending notice', async () => {
    await act(async () => {
      mocks.handlers.get('delivery-retry-feedback')?.({
        payload: { kind: 'failed', message: 'Older retry failed.' },
      });
    });
    await act(async () => presentNotice({
      message: 'Trying delivery again…',
      retryable: true,
    }));

    await act(async () => vi.advanceTimersByTime(5000));
    expect(notice).toEqual({
      message: 'Trying delivery again…',
      retryable: true,
    });
  });

  it('keeps a newer correction notice after prior retry feedback expires', async () => {
    await act(async () => {
      mocks.handlers.get('delivery-retry-feedback')?.({
        payload: { kind: 'clipboard_only', message: 'Copied.' },
      });
      mocks.handlers.get('correction-start-failed')?.({ payload: 'Correction unavailable.' });
    });

    await act(async () => vi.advanceTimersByTime(5000));
    expect(notice).toEqual({ message: 'Correction unavailable.', retryable: false });
  });
});
