import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { canRetryDelivery, isDeliveryRetryResult } from '../deliveryRecovery';

export interface DeliveryRecoveryNotice {
  message: string;
  retryable: boolean;
}

export type PresentDeliveryRecoveryNotice = (
  notice: DeliveryRecoveryNotice,
  timeoutMs?: number,
) => void;

export function useDeliveryRecoveryNotice() {
  const [notice, setNotice] = useState<DeliveryRecoveryNotice | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const noticeIdentityRef = useRef(0);

  const clearNotice = useCallback(() => {
    noticeIdentityRef.current += 1;
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
    setNotice(null);
  }, []);

  const presentNotice = useCallback<PresentDeliveryRecoveryNotice>((next, timeoutMs) => {
    const identity = ++noticeIdentityRef.current;
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
    setNotice(next);
    if (timeoutMs === undefined) return;
    timeoutRef.current = setTimeout(() => {
      if (noticeIdentityRef.current === identity) setNotice(null);
      timeoutRef.current = null;
    }, timeoutMs);
  }, []);

  useEffect(() => () => {
    noticeIdentityRef.current += 1;
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    timeoutRef.current = null;
  }, []);

  return { notice, presentNotice, clearNotice };
}

/**
 * Owns the two passive delivery-recovery event listeners extracted from
 * `App.tsx`: a background retry's outcome (`delivery-retry-feedback`) and a
 * failed on-demand correction start (`correction-start-failed`). Both simply
 * surface a message via the shared banner setter. Behavior-preserving
 * extraction — same events, same cleanup ordering.
 */
export function useDeliveryRecoveryListeners(
  presentNotice: PresentDeliveryRecoveryNotice,
) {
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('delivery-retry-feedback', ({ payload }) => {
      if (disposed) return;
      if (!isDeliveryRetryResult(payload)) return;
      presentNotice({
        message: payload.message,
        retryable: canRetryDelivery(payload),
      }, 5000);
    }).then((fn) => {
      if (disposed) fn(); else unlisten = fn;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [presentNotice]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('correction-start-failed', ({ payload }) => {
      presentNotice({
        message: typeof payload === 'string'
          ? payload
          : 'Could not start correction. Try Correct last dictation from ⌘K.',
        retryable: false,
      });
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [presentNotice]);
}
