import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { DeliveryRetryResult } from '../deliveryRecovery';

/**
 * Owns the two passive delivery-recovery event listeners extracted from
 * `App.tsx`: a background retry's outcome (`delivery-retry-feedback`) and a
 * failed on-demand correction start (`correction-start-failed`). Both simply
 * surface a message via the shared banner setter. Behavior-preserving
 * extraction — same events, same cleanup ordering.
 */
export function useDeliveryRecoveryListeners(setMessage: (message: string) => void) {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<DeliveryRetryResult>('delivery-retry-feedback', ({ payload }) => {
      setMessage(payload.message);
      window.setTimeout(() => setMessage(''), 5000);
    }).then((fn) => { unlisten = fn; }).catch(() => {});
    return () => unlisten?.();
  }, [setMessage]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>('correction-start-failed', ({ payload }) => {
      setMessage(typeof payload === 'string' ? payload : 'Could not start correction. Try Correct last dictation from ⌘K.');
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, [setMessage]);
}
